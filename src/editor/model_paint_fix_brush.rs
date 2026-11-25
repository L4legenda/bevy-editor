//! The GPU keeps the correction texture between strokes. Readbacks publish undo
//! snapshots in submission order without delaying the next stroke on that texture.
use super::*;
use crate::editor::model_paint_data::map_fixes::{BakeMap, Fixes, Plane};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const MAX_READBACKS: usize = 3;
type Archive = Result<Option<Arc<Plane>>, String>;

#[derive(Default)]
pub(super) struct Runtime {
    target: Option<Target>,
    cancel_drawing: bool,
    retired: Option<u64>,
}

struct Target {
    key: Key,
    map: BakeMap,
    source: Arc<bake::Maps>,
    expected: Arc<Fixes>,
    mapping: Arc<Mapping>,
    preparing: Option<Task<Prepared>>,
    preparing_trace: Option<Task<Arc<surface::SurfaceTrace>>>,
    gpu: Option<Live>,
    strokes: VecDeque<Stroke>,
    readbacks: VecDeque<Readback>,
}

struct Stroke {
    view: gpu::View,
    brush: Brush,
    path: stroke::Path,
    stamps: VecDeque<Stamp>,
    finished: bool,
    serial: Option<u64>,
}

struct Readback {
    serial: u64,
    archive: Option<Task<Archive>>,
}

struct Prepared {
    scene: Arc<gpu::Scene>,
    surface: Arc<Surface>,
    base: Vec<u8>,
    preview: Vec<u8>,
}

struct Live {
    id: u64,
    trace: Option<Arc<surface::SurfaceTrace>>,
}

impl Runtime {
    pub fn busy(&self, model: usize) -> bool {
        self.target
            .as_ref()
            .is_some_and(|target| target.key.0 == model && target.busy())
    }

    pub fn drawing(&self) -> bool {
        self.target
            .as_ref()
            .is_some_and(|target| target.strokes.back().is_some_and(|stroke| !stroke.finished))
    }

    pub fn finish(&mut self) {
        if let Some(stroke) = self
            .target
            .as_mut()
            .and_then(|target| target.strokes.back_mut())
        {
            stroke.finished = true;
        }
    }

    pub fn finish_unless(&mut self, key: Key, map: Option<BakeMap>) {
        if self
            .target
            .as_ref()
            .is_some_and(|target| target.key != key || Some(target.map) != map)
        {
            self.finish();
        }
    }

    pub fn request_cancel(&mut self) {
        self.cancel_drawing = self.drawing();
    }

    pub fn move_to(&mut self, point: Vec2, pressure: f32) {
        if let Some(stroke) = self
            .target
            .as_mut()
            .and_then(|target| target.strokes.back_mut())
            && !stroke.finished
            && stroke.view.camera.clip.contains(point)
        {
            stroke.stamps.extend(stroke.path.advance(point, pressure));
        }
    }

    pub fn begin(
        &mut self,
        key: Key,
        map: BakeMap,
        source: Arc<bake::Maps>,
        mapping: Arc<Mapping>,
        original: Arc<Fixes>,
        brush: Brush,
        path: stroke::Path,
    ) -> bool {
        let compatible = self.target.as_ref().is_some_and(|target| {
            target.key == key
                && target.map == map
                && target.source.same_background(&source)
                && Arc::ptr_eq(&target.expected, &original)
                && target.mapping.visibility.revision == mapping.visibility.revision
        });
        if !compatible {
            if self.target.as_ref().is_some_and(Target::busy) {
                return false;
            }
            let plane = original
                .iter()
                .find_map(|(kind, plane)| (*kind == map).then(|| plane.clone()));
            if brush.erase && plane.is_none() {
                return false;
            }
            self.retired = self
                .target
                .take()
                .and_then(|target| target.gpu.map(|gpu| gpu.id));
            self.target = Some(Target::new(
                key,
                map,
                source,
                mapping.clone(),
                original,
                plane,
            ));
        }
        self.finish();
        self.target.as_mut().unwrap().strokes.push_back(Stroke {
            view: gpu::View {
                camera: mapping.camera,
                uv: mapping.uv,
            },
            brush,
            stamps: VecDeque::from([path.first()]),
            path,
            finished: false,
            serial: None,
        });
        true
    }

    pub fn reply(&mut self, id: u64, serial: u64, pixels: Result<Vec<u8>, String>) {
        let Some(target) = self
            .target
            .as_mut()
            .filter(|target| target.gpu.as_ref().is_some_and(|gpu| gpu.id == id))
        else {
            return;
        };
        let Some(readback) = target
            .readbacks
            .iter_mut()
            .find(|readback| readback.serial == serial)
        else {
            return;
        };
        let size = target.source.size;
        readback.archive =
            Some(AsyncComputeTaskPool::get().spawn(async move { decode_snapshot(size, pixels?) }));
    }

    pub fn update(
        &mut self,
        documents: &mut HashMap<usize, Document>,
        baked: &mut bake::State,
        images: &mut Assets<Image>,
        queue: &mut gpu::Queue,
        status: &mut String,
    ) {
        if let Some(id) = self.retired.take() {
            queue.commands.push(gpu::Command::Drop(id));
            baked.fix_preview = None;
        }
        let Some(target) = &mut self.target else {
            return;
        };
        if std::mem::take(&mut self.cancel_drawing) {
            target.cancel_stroke(queue);
        }
        let (model, object, slot) = target.key;
        let Some(doc) = documents.get_mut(&model) else {
            self.discard(queue, baked);
            return;
        };
        let valid = baked
            .maps(model, doc, (object, slot), target.source.size)
            .is_some_and(|maps| maps.same_background(&target.source))
            && doc
                .objects
                .iter()
                .find(|item| item.id == object)
                .and_then(|item| item.materials.get(slot))
                .is_some_and(|material| Arc::ptr_eq(&material.bake_fixes, &target.expected));
        if !valid {
            self.discard(queue, baked);
            return;
        }
        while let Some(result) = target
            .readbacks
            .front_mut()
            .and_then(|readback| readback.archive.as_mut())
            .and_then(|task| block_on(poll_once(task)))
        {
            target.readbacks.pop_front();
            match result {
                Ok(plane) => target.commit(plane, doc, baked),
                Err(error) => {
                    *status = format!("Не удалось исправить карту · {error}");
                    self.discard(queue, baked);
                    return;
                }
            }
        }
        target.pump(baked, images, queue);
    }

    fn discard(&mut self, queue: &mut gpu::Queue, baked: &mut bake::State) {
        if let Some(target) = self.target.take() {
            if let Some(live) = target.gpu {
                queue.commands.push(gpu::Command::Drop(live.id));
            }
            baked.fix_preview = None;
        }
    }
}

impl Target {
    fn new(
        key: Key,
        map: BakeMap,
        source: Arc<bake::Maps>,
        mapping: Arc<Mapping>,
        expected: Arc<Fixes>,
        plane: Option<Arc<Plane>>,
    ) -> Self {
        let mesh = mapping.clone();
        let background = source.clone();
        let preparing = AsyncComputeTaskPool::get().spawn(async move {
            let size = background.size;
            let scene = Arc::new(gpu::Scene::prepare(&mesh.objects, &mesh.visibility));
            let mut layer = Layer::new(String::new(), LayerKind::Paint);
            if let Some(plane) = plane {
                let pixels = (0..size as usize * size as usize)
                    .flat_map(|index| {
                        let (value, coverage) = plane.sample(index, size);
                        let gray = if coverage > 0. {
                            (value / coverage).round() as u8
                        } else {
                            0
                        };
                        [gray, gray, gray, (coverage * 255.).round() as u8]
                    })
                    .collect();
                layer.channels[1] = Some(Arc::new(pixels));
            }
            // Fix contexts use the selected layer buffer directly, without a material stack.
            let empty = Arc::new(vec![0; 4]);
            let surface = Arc::new(Surface {
                size,
                base: std::array::from_fn(|_| empty.clone()),
                stack: Stack {
                    layers: vec![layer],
                    selected: 0,
                },
                material: serde_json::Value::Null,
                baked: None,
            });
            Prepared {
                scene,
                surface,
                base: background.rgba_before_fixes(map),
                preview: background.rgba(map),
            }
        });
        Self {
            key,
            map,
            source,
            expected,
            mapping,
            preparing: Some(preparing),
            preparing_trace: None,
            gpu: None,
            strokes: VecDeque::new(),
            readbacks: VecDeque::new(),
        }
    }

    fn busy(&self) -> bool {
        !self.strokes.is_empty() || !self.readbacks.is_empty()
    }

    fn cancel_stroke(&mut self, queue: &mut gpu::Queue) {
        if self.strokes.back().is_some_and(|stroke| !stroke.finished)
            && let Some(stroke) = self.strokes.pop_back()
            && stroke.serial.is_some()
            && let Some(live) = &self.gpu
        {
            queue.commands.push(gpu::Command::CancelFix(live.id));
        }
    }

    fn commit(&mut self, plane: Option<Arc<Plane>>, doc: &mut Document, baked: &mut bake::State) {
        let (_, object, slot) = self.key;
        let before = doc.snapshot();
        let material = &mut doc
            .objects
            .iter_mut()
            .find(|item| item.id == object)
            .unwrap()
            .materials[slot];
        let previous = material
            .bake_fixes
            .iter()
            .find_map(|(map, plane)| (*map == self.map).then_some(plane));
        if previous != plane.as_ref() {
            let fixes = Arc::make_mut(&mut material.bake_fixes);
            fixes.retain(|(map, _)| *map != self.map);
            if let Some(plane) = plane {
                fixes.push((self.map, plane));
            }
            self.expected = material.bake_fixes.clone();
            if let Some(preview) = baked.fix_preview.as_mut() {
                preview.fixes = self.expected.clone();
            }
            doc.checkpoint(before);
        }
    }

    fn pump(
        &mut self,
        baked: &mut bake::State,
        images: &mut Assets<Image>,
        queue: &mut gpu::Queue,
    ) {
        if self.strokes.is_empty() {
            return;
        }
        if let Some(task) = &mut self.preparing {
            let Some(ready) = block_on(poll_once(task)) else {
                return;
            };
            self.preparing = None;
            let (model, object, slot) = self.key;
            let size = self.source.size;
            let image = images.add(Image::new(
                Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                ready.preview,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            ));
            let id = queue.id();
            queue.commands.push(gpu::Command::Setup {
                id,
                surface: ready.surface,
                channel: 1,
                channels: 1 << 1,
                images: [image.id(); composite::IMAGE_COUNT],
                scene: ready.scene,
                object: object as u32 + 1,
                slot: slot as u32,
                relief_factor: 4.,
                baked_preview: None,
                fix: Some((ready.base, self.map == BakeMap::ProjectedNormal)),
            });
            self.gpu = Some(Live { id, trace: None });
            baked.fix_preview = Some(bake::FixPreview {
                model,
                binding: (object, slot),
                map: self.map,
                image,
                source: self.source.clone(),
                fixes: self.expected.clone(),
            });
        }
        let live = self.gpu.as_mut().unwrap();
        let mut budget = 256;
        while let Some(stroke) = self.strokes.front_mut() {
            if self.readbacks.len() >= MAX_READBACKS {
                break;
            }
            if stroke.serial.is_none() {
                let needs_trace = stroke.brush.projection != BrushProjection::Screen;
                if needs_trace && live.trace.is_none() {
                    let task = self.preparing_trace.get_or_insert_with(|| {
                        let mesh = self.mapping.clone();
                        AsyncComputeTaskPool::get().spawn(async move {
                            Arc::new(surface::SurfaceTrace::build(
                                &mesh.objects,
                                &mesh.visibility,
                                false,
                            ))
                        })
                    });
                    let Some(trace) = block_on(poll_once(task)) else {
                        break;
                    };
                    self.preparing_trace = None;
                    live.trace = Some(trace);
                }
                let serial = queue.id();
                queue.commands.push(gpu::Command::Begin {
                    id: live.id,
                    stroke: serial,
                    brush: stroke.brush,
                    view: stroke.view,
                    trace: needs_trace.then(|| live.trace.clone()).flatten(),
                });
                stroke.serial = Some(serial);
            }
            let count = stroke.stamps.len().min(budget);
            if count > 0 {
                budget -= count;
                queue.commands.push(gpu::Command::Points {
                    id: live.id,
                    points: stroke.stamps.drain(..count).collect(),
                    ranges: None,
                });
            }
            if !stroke.finished || !stroke.stamps.is_empty() {
                break;
            }
            let serial = self.strokes.pop_front().unwrap().serial.unwrap();
            queue.commands.push(gpu::Command::End {
                id: live.id,
                stroke: serial,
            });
            self.readbacks.push_back(Readback {
                serial,
                archive: None,
            });
            if budget == 0 {
                break;
            }
        }
    }
}

fn decode_snapshot(size: u32, pixels: Vec<u8>) -> Archive {
    if pixels.len() != size as usize * size as usize * 4 {
        return Err("Некорректный размер исправленной карты".into());
    }
    let mut visible = false;
    let pixels = pixels
        .chunks_exact(4)
        .flat_map(|rgba| {
            let alpha = rgba[3];
            visible |= alpha != 0;
            [
                ((u16::from(rgba[0]) * u16::from(alpha) + 127) / 255) as u8,
                alpha,
            ]
        })
        .collect();
    Ok(visible.then(|| Arc::new(Plane { size, pixels })))
}
