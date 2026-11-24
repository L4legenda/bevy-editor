//! Main-world stroke ordering. GPU snapshots enter the CPU document only at boundaries.
use super::*;
use std::collections::{BTreeMap, VecDeque};
#[derive(Default)]
pub(super) struct Controller {
    id: u64,
    key: Option<(GeometryVersion, u16, usize)>,
    scene: Option<(GeometryVersion, Arc<gpu::Scene>)>,
    prepare: Option<(GeometryVersion, Task<Arc<gpu::Scene>>)>,
    trace: Option<(GeometryVersion, Arc<surface::SurfaceTrace>)>,
    prepare_trace: Option<(GeometryVersion, bool, Task<Arc<surface::SurfaceTrace>>)>,
    region: Option<Task<Vec<std::ops::Range<u32>>>>,
    pending: Option<Brush>,
    active: Option<u64>,
    inflight: VecDeque<StrokeRecord>,
    results: BTreeMap<u64, Result<Vec<Vec<u8>>, String>>,
    error: Option<String>,
}
struct StrokeRecord {
    id: u64,
    channels: u16,
    selected: usize,
}
impl Controller {
    pub fn owns(&self, id: u64) -> bool {
        self.id == id
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.active.is_some() || !self.inflight.is_empty()
    }
    pub fn affects_opacity(&self) -> bool {
        let opacity = (1 << 4) | (1 << data::MASK_TARGET);
        self.inflight
            .iter()
            .any(|stroke| stroke.channels & opacity != 0)
    }
    pub fn reply(&mut self, id: u64, stroke: u64, result: Result<Vec<Vec<u8>>, String>) {
        if self.id == id {
            if let Err(error) = &result {
                self.error = Some(error.clone());
            }
            self.results.insert(stroke, result);
        }
    }
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
    pub fn invalidate(&mut self, queue: &mut gpu::Queue) {
        if self.id != 0 {
            queue.commands.push(gpu::Command::Drop(self.id));
        }
        *self = Self::default();
    }
    pub fn archive(&mut self, work: Work, queue: &mut gpu::Queue) -> Result<Work, Task<Batch>> {
        let Some(stroke) = self.inflight.front() else {
            return Ok(work);
        };
        let Some(result) = self.results.remove(&stroke.id) else {
            return Ok(work);
        };
        let stroke = self.inflight.pop_front().unwrap();
        let bytes = work.surface.size as usize * work.surface.size as usize * 4;
        let channels = match result {
            Ok(channels)
                if channels.len() == stroke.channels.count_ones() as usize
                    && channels.iter().all(|pixels| pixels.len() == bytes) =>
            {
                channels
            }
            _ => {
                let error = self
                    .error
                    .take()
                    .unwrap_or_else(|| "Повреждён снимок мазка".into());
                self.invalidate(queue);
                self.error = Some(error);
                return Err(AsyncComputeTaskPool::get().spawn(async move { compose(work) }));
            }
        };
        Err(AsyncComputeTaskPool::get().spawn(async move {
            let mut work = work;
            let before = work.surface.stack.clone();
            let layer = &mut work.surface.stack.layers[stroke.selected];
            let mut changed = false;
            let mut opacity_changed = false;
            for (channel, pixels) in channels_in(stroke.channels).zip(channels) {
                if channel == data::MASK_TARGET {
                    if let Some(mask) = layer
                        .mask
                        .as_ref()
                        .filter(|old| old.pixels.as_slice() != pixels.as_slice())
                    {
                        layer.mask = Some(Arc::new(mask.with_pixels(Arc::new(pixels))));
                        changed = true;
                        opacity_changed = true;
                    }
                } else if layer.channels[channel].as_ref().map_or_else(
                    || pixels.chunks_exact(4).any(|p| p[3] != 0),
                    |old| old.as_slice() != pixels.as_slice(),
                ) {
                    layer.channels[channel] = Some(Arc::new(pixels));
                    changed = true;
                    opacity_changed |= channel == 4;
                }
            }
            Batch {
                maps: opacity_changed.then(|| Updates {
                    textures: std::array::from_fn(|_| None),
                    transparent: Some(
                        work.surface
                            .composite(4)
                            .chunks_exact(4)
                            .any(|p| p[0] < 255),
                    ),
                }),
                work,
                history: if changed { vec![before] } else { Vec::new() },
            }
        }))
    }
    fn context(
        &mut self,
        work: Option<&Work>,
        brush: Brush,
        map: &Arc<Mapping>,
        images: &[Handle<Image>; composite::IMAGE_COUNT],
        queue: &mut gpu::Queue,
    ) -> bool {
        let selected =
            work.map_or_else(|| self.key.map_or(0, |k| k.2), |w| w.surface.stack.selected);
        if self.inflight.len() >= 3 {
            return false;
        }
        let version = map.version();
        let key = (version, brush.channels(), selected);
        if self.key == Some(key) {
            return true;
        }
        if !self.inflight.is_empty() || work.is_none() {
            return false;
        }
        if let Some((revision, job)) = self.prepare.as_mut() {
            if let Some(scene) = block_on(poll_once(job)) {
                self.scene = Some((*revision, scene));
                self.prepare = None;
            }
        }
        if !self
            .scene
            .as_ref()
            .is_some_and(|(revision, _)| *revision == version)
        {
            if !self
                .prepare
                .as_ref()
                .is_some_and(|(revision, _)| *revision == version)
            {
                let objects = map.objects.clone();
                let visibility = map.visibility.clone();
                self.prepare = Some((
                    version,
                    AsyncComputeTaskPool::get()
                        .spawn(async move { Arc::new(gpu::Scene::prepare(&objects, &visibility)) }),
                ));
            }
            return false;
        }
        if self.id != 0 {
            queue.commands.push(gpu::Command::Drop(self.id));
        }
        self.id = queue.id();
        self.key = Some(key);
        queue.commands.push(gpu::Command::Setup {
            id: self.id,
            surface: Arc::new(work.unwrap().surface.clone()),
            channel: brush.channel,
            channels: brush.channels(),
            images: std::array::from_fn(|i| images[i].id()),
            scene: self.scene.as_ref().unwrap().1.clone(),
            object: map.object as u32 + 1,
            slot: map.slot as u32,
            fix: None,
        });
        true
    }
    pub(super) fn trace(
        &mut self,
        revision: u64,
        visibility: &sets::SetVisibility,
        objects: &[Object],
        fill: bool,
    ) -> Option<Arc<surface::SurfaceTrace>> {
        let revision = (revision, visibility.revision);
        if let Some((revision, _, job)) = self.prepare_trace.as_mut()
            && let Some(trace) = block_on(poll_once(job))
        {
            self.trace = Some((*revision, trace));
            self.prepare_trace = None;
        }
        if let Some((current, trace)) = &self.trace
            && *current == revision
            && (!fill || trace.supports_fill())
        {
            return Some(trace.clone());
        }
        if !self
            .prepare_trace
            .as_ref()
            .is_some_and(|(current, preparing_fill, _)| {
                *current == revision && (!fill || *preparing_fill)
            })
        {
            let objects = objects.to_vec();
            let visibility = visibility.clone();
            self.prepare_trace = Some((
                revision,
                fill,
                AsyncComputeTaskPool::get().spawn(async move {
                    Arc::new(surface::SurfaceTrace::build(&objects, &visibility, fill))
                }),
            ));
        }
        None
    }
    fn begin(
        &mut self,
        brush: Brush,
        map: &Mapping,
        trace: Option<Arc<surface::SurfaceTrace>>,
        queue: &mut gpu::Queue,
    ) {
        let stroke = queue.id();
        queue.commands.push(gpu::Command::Begin {
            id: self.id,
            stroke,
            brush,
            trace,
            view: gpu::View {
                camera: map.camera,
                uv: map.uv,
            },
        });
        self.inflight.push_back(StrokeRecord {
            id: stroke,
            channels: brush.channels(),
            selected: self.key.unwrap().2,
        });
        self.active = Some(stroke);
        self.pending = None;
    }
    pub fn pump(
        &mut self,
        work: Option<&Work>,
        events: &mut VecDeque<Event>,
        images: &[Handle<Image>; composite::IMAGE_COUNT],
        queue: &mut gpu::Queue,
    ) {
        let mut budget = 256;
        while let Some(event) = events.pop_front() {
            match event {
                Event::Begin(brush) => self.pending = Some(brush),
                Event::Points(mut points, map) => {
                    if let Some(brush) = self.pending {
                        let needs_trace = brush.fill || brush.projection != BrushProjection::Screen;
                        let trace = needs_trace
                            .then(|| {
                                self.trace(map.revision, &map.visibility, &map.objects, brush.fill)
                            })
                            .flatten();
                        if !self.context(work, brush, &map, images, queue)
                            || (needs_trace && trace.is_none())
                        {
                            events.push_front(Event::Points(points, map));
                            break;
                        }
                        self.begin(brush, &map, trace, queue);
                    }
                    if self.active.is_some() {
                        if points.len() > budget {
                            let tail = points.split_off(budget);
                            events.push_front(Event::Points(tail, map));
                        }
                        budget -= points.len();
                        queue.commands.push(gpu::Command::Points {
                            id: self.id,
                            points,
                            ranges: None,
                        });
                        if budget == 0 {
                            break;
                        }
                    }
                }
                Event::Region(region, map) => {
                    let Some(brush) = self.pending else { continue };
                    let Some(trace) = self.trace(map.revision, &map.visibility, &map.objects, true)
                    else {
                        events.push_front(Event::Region(region, map));
                        break;
                    };
                    if !self.context(work, brush, &map, images, queue) {
                        events.push_front(Event::Region(region, map));
                        break;
                    }
                    let job = self.region.get_or_insert_with(|| {
                        let trace = trace.clone();
                        let region = region.clone();
                        let target = surface::FillTarget {
                            view: gpu::View {
                                camera: map.camera,
                                uv: map.uv,
                            },
                            object: map.object,
                            slot: map.slot,
                            mode: brush.fill_mode,
                            angle: brush.fill_angle,
                        };
                        AsyncComputeTaskPool::get()
                            .spawn(async move { trace.fill_region(target, &region) })
                    });
                    let Some(ranges) = block_on(poll_once(job)) else {
                        events.push_front(Event::Region(region, map));
                        break;
                    };
                    self.region = None;
                    if !ranges.is_empty() {
                        self.begin(brush, &map, Some(trace), queue);
                        queue.commands.push(gpu::Command::Points {
                            id: self.id,
                            points: Vec::new(),
                            ranges: Some(ranges),
                        });
                    }
                }
                Event::End => {
                    if let Some(stroke) = self.active.take() {
                        queue.commands.push(gpu::Command::End {
                            id: self.id,
                            stroke,
                        });
                    }
                    self.pending = None;
                }
            }
        }
    }
}
