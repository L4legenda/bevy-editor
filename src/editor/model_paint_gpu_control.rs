//! Main-world stroke ordering. GPU snapshots enter the CPU document only at boundaries.
use super::*;
use std::collections::{BTreeMap, VecDeque};
#[derive(Default)]
pub(super) struct Controller {
    id: u64,
    key: Option<(u64, usize, usize)>,
    scene: Option<(u64, Arc<gpu::Scene>)>,
    prepare: Option<(u64, Task<Arc<gpu::Scene>>)>,
    pending: Option<Brush>,
    active: Option<u64>,
    inflight: VecDeque<StrokeRecord>,
    results: BTreeMap<u64, Result<Vec<u8>, String>>,
    error: Option<String>,
}
struct StrokeRecord {
    id: u64,
    channel: usize,
    selected: usize,
}
impl Controller {
    pub fn owns(&self, id: u64) -> bool {
        self.id == id
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.active.is_some() || !self.inflight.is_empty()
    }
    pub fn reply(&mut self, id: u64, stroke: u64, result: Result<Vec<u8>, String>) {
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
    pub fn archive(&mut self, work: Work) -> Result<Work, Task<Batch>> {
        let Some(stroke) = self.inflight.front() else {
            return Ok(work);
        };
        let Some(result) = self.results.remove(&stroke.id) else {
            return Ok(work);
        };
        let stroke = self.inflight.pop_front().unwrap();
        Err(AsyncComputeTaskPool::get().spawn(async move {
            let mut work = work;
            let mut history = Vec::new();
            let mut maps = None;
            if let Ok(pixels) = result {
                let layer = &work.surface.stack.layers[stroke.selected];
                let changed = layer.channels[stroke.channel].as_ref().map_or_else(
                    || pixels.chunks_exact(4).any(|p| p[3] != 0),
                    |old| old.as_slice() != pixels.as_slice(),
                );
                if changed {
                    history.push(work.surface.stack.clone());
                    work.surface.stack.layers[stroke.selected].channels[stroke.channel] =
                        Some(Arc::new(pixels));
                    if stroke.channel == 4 || stroke.channel == 7 {
                        let mut updates = Updates {
                            textures: std::array::from_fn(|_| None),
                            transparent: None,
                            reflectance: None,
                        };
                        let pixels = work.surface.composite(stroke.channel);
                        if stroke.channel == 4 {
                            updates.transparent = Some(pixels.chunks_exact(4).any(|p| p[0] < 255));
                        } else {
                            updates.reflectance = Some(
                                pixels
                                    .chunks_exact(4)
                                    .map(|p| p[0] as f32 / 255.)
                                    .sum::<f32>()
                                    / (work.surface.size * work.surface.size) as f32,
                            );
                        }
                        maps = Some(updates);
                    }
                }
            }
            Batch {
                work,
                maps,
                history,
            }
        }))
    }
    fn context(
        &mut self,
        work: Option<&Work>,
        brush: Brush,
        map: &Arc<Mapping>,
        images: &[Handle<Image>; 9],
        queue: &mut gpu::Queue,
    ) -> bool {
        let selected =
            work.map_or_else(|| self.key.map_or(0, |k| k.2), |w| w.surface.stack.selected);
        let key = (map.revision, brush.channel, selected);
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
            .is_some_and(|(revision, _)| *revision == map.revision)
        {
            if !self
                .prepare
                .as_ref()
                .is_some_and(|(revision, _)| *revision == map.revision)
            {
                let objects = map.objects.clone();
                self.prepare = Some((
                    map.revision,
                    AsyncComputeTaskPool::get()
                        .spawn(async move { Arc::new(gpu::Scene::prepare(&objects)) }),
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
            images: std::array::from_fn(|i| images[i].id()),
            scene: self.scene.as_ref().unwrap().1.clone(),
            object: map.object as u32 + 1,
            slot: map.slot as u32,
        });
        true
    }
    pub fn pump(
        &mut self,
        work: Option<&Work>,
        events: &mut VecDeque<Event>,
        images: &[Handle<Image>; 9],
        queue: &mut gpu::Queue,
    ) {
        let mut budget = 256;
        while let Some(event) = events.pop_front() {
            match event {
                Event::Begin(brush) => self.pending = Some(brush),
                Event::Points(mut points, map) => {
                    if let Some(brush) = self.pending {
                        if !self.context(work, brush, &map, images, queue) {
                            events.push_front(Event::Points(points, map));
                            break;
                        }
                        let stroke = queue.id();
                        queue.commands.push(gpu::Command::Begin {
                            id: self.id,
                            stroke,
                            brush,
                            view: gpu::View {
                                camera: map.camera,
                                uv: map.uv,
                            },
                        });
                        self.inflight.push_back(StrokeRecord {
                            id: stroke,
                            channel: brush.channel,
                            selected: self.key.unwrap().2,
                        });
                        self.active = Some(stroke);
                        self.pending = None;
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
                        });
                        if budget == 0 {
                            break;
                        }
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
