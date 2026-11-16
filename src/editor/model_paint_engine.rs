//! Surface painting runs on the compute pool. The UI only queues pointer samples
//! and uploads completed textures. No scene materials are modified in place.
use super::*;
use crate::editor::{
    model_document::{Document, Object},
    model_paint_data as data,
};
use bevy::{
    asset::RenderAssetUsages,
    input_focus::InputFocus,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    ui_widgets::TextInput,
};
use std::{collections::VecDeque, sync::Arc};

type Key = (usize, usize, usize);
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct TextureCanvas;
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct BrushCursor;
#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct Runtime {
    lives: HashMap<Key, Live>,
    gesture: Option<(Key, bool, Vec2)>,
    mapping: Option<Arc<Mapping>>,
    map_cache: Option<((Key, u64, Transform, Rect, bool), Arc<Mapping>)>,
    deferred_save: HashMap<usize, bool>,
}
struct Live {
    work: Option<Work>,
    task: Option<Task<Batch>>,
    queue: VecDeque<Event>,
    gpu: Option<Gpu>,
    published: Option<Arc<Surface>>,
    origin: crate::editor::model_document::MaterialSlot,
    preview_needed: bool,
    ui_stack: Stack,
    controller: gpu_control::Controller,
}
struct Work {
    surface: Surface,
    #[cfg(test)]
    stroke: Option<Stroke>,
}
impl Work {
    fn new(surface: Surface) -> Self {
        Self {
            surface,
            #[cfg(test)]
            stroke: None,
        }
    }
}
struct Batch {
    work: Work,
    maps: Option<Updates>,
    history: Vec<Stack>,
}
struct Gpu {
    material: Handle<StandardMaterial>,
    images: [Handle<Image>; 9],
    preview: [Handle<Image>; 8],
}
#[cfg(test)]
struct Stroke {
    brush: Brush,
    original: Option<Arc<Vec<u8>>>,
    coverage: Vec<f32>,
    changed: bool,
    visibility: Vec<u64>,
}
#[derive(Clone, Copy)]
struct Brush {
    channel: usize,
    color: [u8; 3],
    radius: f32,
    hardness: f32,
    flow: f32,
    opacity: f32,
    erase: bool,
    fill: bool,
}
enum Event {
    Begin(Brush),
    Points(Vec<Vec2>, Arc<Mapping>),
    End,
}
#[path = "model_paint_projection.rs"]
mod projection;
use projection::*;
#[path = "model_paint_composite.rs"]
mod composite;
#[cfg(test)]
use composite::Dirty;
use composite::Updates;
#[path = "model_paint_gpu.rs"]
pub(in crate::editor::ui::model_viewer) mod gpu;
#[path = "model_paint_gpu_control.rs"]
mod gpu_control;
#[cfg(test)]
fn paint_points(work: &mut Work, points: &[Vec2], map: &Mapping) -> Option<URect> {
    let stroke = work.stroke.as_mut()?;
    let brush = stroke.brush;
    let layer = work
        .surface
        .stack
        .layers
        .get_mut(work.surface.stack.selected)
        .filter(|l| l.kind == LayerKind::Paint && l.visible)?;
    let geometry = map.geometry();
    let samples = points
        .iter()
        .filter_map(|&p| {
            let face = if map.uv {
                if brush.fill {
                    let uv = (p - map.camera.rect.min) / map.camera.rect.size();
                    Some(
                        geometry
                            .triangles
                            .iter()
                            .find(|t| weights(uv, t.uv).is_some_and(|b| b.min_element() >= 0.))?
                            .face,
                    )
                } else {
                    None
                }
            } else {
                let (o, d) = map.camera.ray(p);
                let (id, face, _) = geometry.bvh.cast(o, d, None)?;
                if id != map.object {
                    return None;
                }
                Some(face)
            };
            Some((p, face))
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return None;
    }
    let mut bounds = Rect::from_center_size(samples[0].0, Vec2::splat(brush.radius * 2.));
    for (p, _) in &samples {
        bounds.min = bounds.min.min(*p - Vec2::splat(brush.radius));
        bounds.max = bounds.max.max(*p + Vec2::splat(brush.radius));
    }
    let candidates = if brush.fill {
        (0..geometry.triangles.len())
            .filter(|&i| {
                samples
                    .iter()
                    .any(|(_, face)| *face == Some(geometry.triangles[i].face))
            })
            .collect()
    } else {
        geometry.candidates(bounds)
    };
    let size = work.surface.size as usize;
    let bytes =
        layer.channels[brush.channel].get_or_insert_with(|| Arc::new(vec![0; size * size * 4]));
    let mut dirty: Option<URect> = None;
    for index in candidates {
        let triangle = &geometry.triangles[index];
        let Some(screen) = triangle.screen else {
            continue;
        };
        let smin = screen[0].min(screen[1]).min(screen[2]);
        let smax = screen[0].max(screen[1]).max(screen[2]);
        if !brush.fill
            && (smin.x > bounds.max.x
                || smax.x < bounds.min.x
                || smin.y > bounds.max.y
                || smax.y < bounds.min.y)
        {
            continue;
        }
        let mut umin = triangle.uv[0].min(triangle.uv[1]).min(triangle.uv[2]);
        let mut umax = triangle.uv[0].max(triangle.uv[1]).max(triangle.uv[2]);
        if !brush.fill {
            let corners = [
                bounds.min,
                Vec2::new(bounds.max.x, bounds.min.y),
                bounds.max,
                Vec2::new(bounds.min.x, bounds.max.y),
            ];
            let depth = triangle
                .points
                .map(|p| -map.camera.inverse.transform_point3(p).z);
            let projected = corners.map(|p| {
                weights(p, screen).and_then(|mut b| {
                    if !map.uv && !map.camera.ortho {
                        b /= Vec3::from_array(depth);
                        let sum = b.element_sum();
                        if sum.abs() < 1.0e-8 {
                            return None;
                        }
                        b /= sum;
                    }
                    Some(triangle.uv[0] * b.x + triangle.uv[1] * b.y + triangle.uv[2] * b.z)
                })
            });
            if let [Some(a), Some(b), Some(c), Some(d)] = projected {
                umin = umin.max(a.min(b).min(c).min(d));
                umax = umax.min(a.max(b).max(c).max(d));
            }
        }
        let lo = (umin.clamp(Vec2::ZERO, Vec2::ONE) * size as f32)
            .floor()
            .as_uvec2();
        let hi = (umax.clamp(Vec2::ZERO, Vec2::ONE) * size as f32)
            .ceil()
            .as_uvec2()
            .min(UVec2::splat(size as u32));
        for y in lo.y..hi.y {
            for x in lo.x..hi.x {
                let i = y as usize * size + x as usize;
                let previous = stroke.coverage[i];
                if previous >= 1. {
                    continue;
                }
                let uv = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
                let Some(b) = weights(uv, triangle.uv).filter(|b| b.min_element() >= -1.0e-5)
                else {
                    continue;
                };
                let world = mix3(triangle.points, b);
                let projected = if map.uv {
                    map.camera.rect.min + uv * map.camera.rect.size()
                } else {
                    let Some(p) = map.camera.project(world) else {
                        continue;
                    };
                    p
                };
                let mut keep = 1.;
                for (point, face) in &samples {
                    let fall = if brush.fill {
                        if *face == Some(triangle.face) { 1. } else { 0. }
                    } else {
                        data::coverage(projected.distance(*point) / brush.radius, brush.hardness)
                    };
                    keep *= 1. - if brush.fill { fall } else { fall * brush.flow };
                }
                if keep >= 1. {
                    continue;
                }
                if !map.uv {
                    let tag = (index as u64 + 1) << 1;
                    let cached = stroke.visibility[i];
                    let visible = if cached >> 1 == tag >> 1 {
                        cached & 1 != 0
                    } else {
                        let (o, d) = map.camera.ray(projected);
                        let depth = (world - o).dot(d);
                        let visible = geometry.bvh.cast(o, d, None).is_some_and(|(id, _, near)| {
                            id == map.object && depth <= near + map.camera.epsilon
                        });
                        stroke.visibility[i] = tag | u64::from(visible);
                        visible
                    };
                    if !visible {
                        continue;
                    }
                }
                let mask = 1. - (1. - previous) * keep;
                if mask <= previous {
                    continue;
                }
                stroke.coverage[i] = mask;
                let base = stroke
                    .original
                    .as_ref()
                    .map_or([0; 4], |p| p[i * 4..i * 4 + 4].try_into().unwrap());
                let rgba = data::over(base, brush.color, mask * brush.opacity, brush.erase);
                if bytes[i * 4..i * 4 + 4] != rgba {
                    Arc::make_mut(bytes)[i * 4..i * 4 + 4].copy_from_slice(&rgba);
                    let point = UVec2::new(x, y);
                    dirty = Some(match dirty {
                        None => URect {
                            min: point,
                            max: point + UVec2::ONE,
                        },
                        Some(r) => URect {
                            min: r.min.min(point),
                            max: r.max.max(point + UVec2::ONE),
                        },
                    });
                }
            }
        }
    }
    stroke.changed |= dirty.is_some();
    dirty
}
#[cfg(test)]
fn batch(mut work: Work, events: Vec<Event>, force: bool) -> Batch {
    let mut dirty: Dirty = std::array::from_fn(|_| {
        force.then_some(URect {
            min: UVec2::ZERO,
            max: UVec2::splat(work.surface.size),
        })
    });
    let mut history = Vec::new();
    let mut events = events.into_iter().peekable();
    while let Some(event) = events.next() {
        match event {
            Event::Begin(brush) => {
                work.stroke = Some(Stroke {
                    brush,
                    original: work
                        .surface
                        .stack
                        .layers
                        .get(work.surface.stack.selected)
                        .and_then(|l| l.channels[brush.channel].clone()),
                    coverage: vec![0.; (work.surface.size * work.surface.size) as usize],
                    visibility: vec![0; (work.surface.size * work.surface.size) as usize],
                    changed: false,
                })
            }
            Event::Points(mut points, map) => {
                while matches!(events.peek(),Some(Event::Points(_,next)) if Arc::ptr_eq(&map,next))
                {
                    if let Some(Event::Points(next, _)) = events.next() {
                        points.extend(next);
                    }
                }
                let first = work.stroke.as_ref().is_some_and(|s| !s.changed);
                let before = first.then(|| work.surface.stack.clone());
                let channel = work.stroke.as_ref().map_or(0, |s| s.brush.channel);
                if let Some(rect) = paint_points(&mut work, &points, &map) {
                    if first {
                        history.push(before.unwrap());
                    }
                    dirty[channel] = composite::union(dirty[channel], Some(rect));
                }
            }
            Event::End => {
                if let Some(stroke) = work.stroke.take() {
                    if !stroke.changed {
                        if let Some(layer) = work
                            .surface
                            .stack
                            .layers
                            .get_mut(work.surface.stack.selected)
                        {
                            layer.channels[stroke.brush.channel] = stroke.original;
                        }
                    }
                }
            }
        }
    }
    let maps = dirty
        .iter()
        .any(Option::is_some)
        .then(|| composite::prepare(&work.surface, dirty));
    Batch {
        work,
        maps,
        history,
    }
}
#[cfg(test)]
fn take_events(queue: &mut VecDeque<Event>) -> Vec<Event> {
    // Keep feedback bounded while preserving every sample and stroke boundary.
    let mut remaining = 32;
    let mut events = Vec::new();
    while let Some(event) = queue.pop_front() {
        match event {
            Event::Points(mut points, map) if points.len() > remaining => {
                let tail = points.split_off(remaining);
                queue.push_front(Event::Points(tail, map.clone()));
                if !points.is_empty() {
                    events.push(Event::Points(points, map));
                }
                break;
            }
            Event::Points(points, map) => {
                remaining -= points.len();
                events.push(Event::Points(points, map));
            }
            event => events.push(event),
        }
        if remaining == 0 {
            break;
        }
    }
    events
}
fn compose(work: Work) -> Batch {
    let rect = URect {
        min: UVec2::ZERO,
        max: UVec2::splat(work.surface.size),
    };
    let maps = Some(composite::prepare(&work.surface, [Some(rect); 8]));
    Batch {
        work,
        maps,
        history: Vec::new(),
    }
}

fn image_pixels(
    images: &Assets<Image>,
    handle: Option<&Handle<Image>>,
    size: u32,
    default: [u8; 4],
) -> Vec<u8> {
    handle
        .and_then(|h| images.get(h))
        .and_then(|i| i.clone().try_into_dynamic().ok())
        .map(|i| {
            image::imageops::resize(
                &i.to_rgba8(),
                size,
                size,
                image::imageops::FilterType::Triangle,
            )
            .into_raw()
        })
        .unwrap_or_else(|| default.repeat((size * size) as usize))
}
fn initial(
    doc: &Document,
    object: &Object,
    slot: usize,
    images: &Assets<Image>,
    materials: &Assets<StandardMaterial>,
) -> Surface {
    if let Some(surface) = object.materials.get(slot).and_then(|m| m.paint.as_ref()) {
        return surface.as_ref().clone();
    }
    let m = object
        .materials
        .get(slot)
        .and_then(|s| materials.get(&s.material))
        .cloned()
        .unwrap_or_default();
    let size = 1024;
    let factor = m.base_color.to_srgba().to_f32_array();
    let mut color = image_pixels(images, m.base_color_texture.as_ref(), size, [255; 4]);
    let mut opacity = vec![255; size as usize * size as usize * 4];
    for (p, a) in color.chunks_exact_mut(4).zip(opacity.chunks_exact_mut(4)) {
        for c in 0..4 {
            p[c] = (p[c] as f32 * factor[c]).round() as u8;
        }
        a[0] = p[3];
        a[1] = p[3];
        a[2] = p[3];
        p[3] = 255;
    }
    let rm = image_pixels(
        images,
        m.metallic_roughness_texture.as_ref(),
        size,
        [255; 4],
    );
    let mut rough = rm.clone();
    let mut metal = rm;
    for p in rough.chunks_exact_mut(4) {
        let v = (p[1] as f32 * m.perceptual_roughness).round() as u8;
        *p.get_mut(0).unwrap() = v;
        p[1] = v;
        p[2] = v;
        p[3] = 255;
    }
    for p in metal.chunks_exact_mut(4) {
        let v = (p[2] as f32 * m.metallic).round() as u8;
        p[0] = v;
        p[1] = v;
        p[2] = v;
        p[3] = 255;
    }
    let mut emission = image_pixels(images, m.emissive_texture.as_ref(), size, [255; 4]);
    let ef = m.emissive.to_f32_array();
    for p in emission.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = (p[c] as f32 * ef[c]).clamp(0., 255.).round() as u8;
        }
        p[3] = 255;
    }
    let normal = image_pixels(
        images,
        m.normal_map_texture.as_ref(),
        size,
        [128, 128, 255, 255],
    );
    let material = object
        .materials
        .get(slot)
        .and_then(|m| m.source["material"].as_u64())
        .and_then(|i| doc.source.json["materials"].get(i as usize))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"doubleSided":true}));
    Surface {
        size,
        stack: Stack::default(),
        material,
        base: [
            Arc::new(color),
            Arc::new(rough),
            Arc::new(metal),
            data::solid(size, [128, 128, 128, 255]),
            Arc::new(opacity),
            Arc::new(emission),
            Arc::new(normal),
            data::solid(size, [(m.reflectance * 255.).round() as u8; 4]),
        ],
    }
}
fn upload(
    live: &mut Live,
    mut updates: Updates,
    slot: &mut crate::editor::model_document::MaterialSlot,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    uploads: &mut crate::editor::image_texture::TextureUploads,
) {
    let fresh = live.gpu.is_none();
    if fresh {
        let textures: [Handle<Image>; 9] = std::array::from_fn(|i| {
            let patch = updates.textures[i]
                .take()
                .expect("first paint upload initializes every texture");
            images.add(Image::from_dynamic(
                image::DynamicImage::ImageRgba8(
                    image::RgbaImage::from_raw(patch.rect.width(), patch.rect.height(), patch.rgba)
                        .unwrap(),
                ),
                i == 0 || i == 3,
                RenderAssetUsages::default(),
            ))
        });
        let material = materials.add(materials.get(&slot.material).cloned().unwrap_or_default());
        let preview = std::array::from_fn(|c| textures[composite::PREVIEW[c]].clone());
        live.gpu = Some(Gpu {
            material,
            images: textures,
            preview,
        });
    } else {
        let gpu = live.gpu.as_ref().unwrap();
        for (i, patch) in updates.textures.into_iter().enumerate() {
            if let Some(p) = patch {
                uploads.patch(gpu.images[i].id(), p.rect.min, p.rect.size(), p.rgba);
            }
        }
    }
    let gpu = live.gpu.as_ref().unwrap();
    if fresh || updates.transparent.is_some() || updates.reflectance.is_some() {
        if let Some(mut m) = materials.get_mut(&gpu.material) {
            if fresh {
                m.base_color = Color::WHITE;
                m.base_color_texture = Some(gpu.images[0].clone());
                m.metallic = 1.;
                m.perceptual_roughness = 1.;
                m.metallic_roughness_texture = Some(gpu.images[1].clone());
                m.normal_map_texture = Some(gpu.images[2].clone());
                m.flip_normal_map_y = false;
                m.emissive = LinearRgba::WHITE;
                m.emissive_texture = Some(gpu.images[3].clone());
                m.base_color_channel = bevy::mesh::UvChannel::Uv0;
                m.normal_map_channel = bevy::mesh::UvChannel::Uv0;
                m.metallic_roughness_channel = bevy::mesh::UvChannel::Uv0;
                m.emissive_channel = bevy::mesh::UvChannel::Uv0;
            }
            if let Some(alpha) = updates.transparent {
                m.alpha_mode = if alpha {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                };
            }
            if let Some(spec) = updates.reflectance {
                m.reflectance = spec;
            }
        }
    }
    slot.material = gpu.material.clone();
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut runtime: ResMut<Runtime>,
    mut paint: ResMut<PaintState>,
    mut state: ResMut<editing::State>,
    mut editor: ResMut<Editor>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut uploads: ResMut<crate::editor::image_texture::TextureUploads>,
    mut gpu_queue: ResMut<gpu::Queue>,
    mut messages: ParamSet<(MessageReader<Action>, MessageWriter<Action>)>,
) {
    let replies = std::mem::take(&mut *gpu_queue.replies.lock().unwrap());
    for gpu::Reply::Layer { id, stroke, pixels } in replies {
        if let Some(live) = runtime.lives.values_mut().find(|l| l.controller.owns(id)) {
            live.controller.reply(id, stroke, pixels);
        }
    }
    for action in messages.p0().read() {
        let close = matches!(action, Action::SaveAndContinue);
        if close
            || matches!(
                action,
                Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save))
            )
        {
            if let Some(index) = editor
                .viewing_model
                .filter(|i| state.documents.get(i).is_some_and(|d| d.paint_busy))
            {
                runtime.deferred_save.insert(index, close);
            }
        }
    }
    runtime.lives.retain(|(index, _, _), live| {
        let keep = state.documents.contains_key(index);
        if !keep {
            live.controller.invalidate(&mut gpu_queue);
        }
        keep
    });
    // Bind the UI to the selected material's own stack, including imported layers.
    if let Some(index) = editor.viewing_model {
        if let Some(doc) = state
            .documents
            .get(&index)
            .filter(|d| d.mode == Mode::Paint)
        {
            let session = paint.sessions.entry(index).or_default();
            let target = doc
                .objects
                .iter()
                .flat_map(|o| (0..o.materials.len()).map(move |s| (o, s)))
                .nth(session.set);
            if let Some((object, slot)) = target {
                let key = (index, object.id, slot);
                if session.binding != Some((object.id, slot)) {
                    if let Some(old) = session.binding {
                        session.histories.insert(
                            old,
                            (
                                std::mem::take(&mut session.undo),
                                std::mem::take(&mut session.redo),
                            ),
                        );
                    }
                    session.binding = Some((object.id, slot));
                    let history = session
                        .histories
                        .remove(&(object.id, slot))
                        .unwrap_or_default();
                    session.undo = history.0;
                    session.redo = history.1;
                    session.stack = object.materials[slot]
                        .paint
                        .as_ref()
                        .map_or_else(Stack::default, |s| s.stack.clone());
                    session.revision += 1;
                }
                runtime.lives.entry(key).or_insert_with(|| {
                    let surface = initial(doc, object, slot, &images, &materials);
                    session.stack = surface.stack.clone();
                    session.texture_size = surface.size;
                    Live {
                        work: Some(Work::new(surface.clone())),
                        task: None,
                        queue: VecDeque::new(),
                        gpu: None,
                        published: object.materials[slot].paint.clone(),
                        origin: object.materials[slot].clone(),
                        ui_stack: surface.stack.clone(),
                        preview_needed: true,
                        controller: Default::default(),
                    }
                });
            }
        }
    }
    for (&(index, object, slot), live) in &mut runtime.lives {
        let Some(doc) = state.documents.get_mut(&index) else {
            continue;
        };
        let Some(session) = paint.sessions.get_mut(&index) else {
            continue;
        };
        let bound = session.binding == Some((object, slot));
        if let Some(error) = live.controller.take_error() {
            editor.status = format!("Не удалось завершить GPU-мазок · {error}");
        }
        let current = doc
            .objects
            .iter()
            .find(|o| o.id == object)
            .and_then(|o| o.materials.get(slot));
        let Some(current) = current else {
            continue;
        };
        let same = match (&current.paint, &live.published) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if !same {
            // Mesh/document undo can restore a different material snapshot while
            // painting is inactive. Never resurrect the previous paint cache.
            live.task = None;
            live.controller.invalidate(&mut gpu_queue);
            live.queue.clear();
            live.published = current.paint.clone();
            live.origin = current.clone();
            live.preview_needed = true;
            live.gpu = None;
            let object = doc.objects.iter().find(|o| o.id == object).unwrap();
            let surface = initial(doc, object, slot, &images, &materials);
            if bound {
                session.stack = surface.stack.clone();
                session.undo.clear();
                session.redo.clear();
                session.revision += 1;
            }
            live.ui_stack = surface.stack.clone();
            live.work = Some(Work::new(surface));
        }
        if let Some(result) = live.task.as_mut().and_then(|job| block_on(poll_once(job))) {
            live.task = None;
            let finished = true;
            let history_changed = !result.history.is_empty();
            if bound {
                for history in result.history {
                    if session.undo.len() >= 32 {
                        session.undo.remove(0);
                    }
                    session.undo.push(history);
                    session.redo.clear();
                }
                if finished {
                    session.stack = result.work.surface.stack.clone();
                    live.ui_stack = session.stack.clone();
                }
                if finished || history_changed {
                    session.revision += 1;
                }
            }
            if let Some(material) = doc
                .objects
                .iter_mut()
                .find(|o| o.id == object)
                .and_then(|o| o.materials.get_mut(slot))
            {
                let handle = material.material.clone();
                if let Some(updates) = result.maps {
                    upload(
                        live,
                        updates,
                        material,
                        &mut images,
                        &mut materials,
                        &mut uploads,
                    );
                }
                if finished {
                    let mut origin_stack = live
                        .origin
                        .paint
                        .as_ref()
                        .map_or_else(Stack::default, |s| s.stack.clone());
                    origin_stack.selected = result.work.surface.stack.selected;
                    if !data::same_stack(&result.work.surface.stack, &origin_stack) {
                        let surface = Arc::new(result.work.surface.clone());
                        material.paint = Some(surface.clone());
                        live.published = Some(surface);
                    } else if !live.controller.busy() && live.queue.is_empty() {
                        *material = live.origin.clone();
                        live.published = material.paint.clone();
                    }
                    doc.ui_revision += 1;
                }
                if handle != material.material {
                    doc.revision += 1;
                }
            }
            live.work = Some(result.work);
        }
        if live.task.is_none() {
            if let Some(work) = live.work.take() {
                match live.controller.archive(work) {
                    Ok(work) => live.work = Some(work),
                    Err(job) => live.task = Some(job),
                }
            }
        }
        if live.task.is_none() {
            if bound && live.queue.is_empty() && !live.controller.busy() {
                session.busy = false;
                while let Some(action) = session.queued.pop_front() {
                    session.edit(action);
                }
            }
            let work = live.work.as_mut().unwrap();
            let stack_changed = bound && !data::same_stack(&session.stack, &live.ui_stack);
            if stack_changed {
                work.surface.stack = session.stack.clone();
                live.ui_stack = session.stack.clone();
            }
            if stack_changed || live.preview_needed {
                if stack_changed {
                    doc.paint_pending = true;
                }
                let work = live.work.take().unwrap();
                live.controller.invalidate(&mut gpu_queue);
                live.preview_needed = false;
                live.task = Some(AsyncComputeTaskPool::get().spawn(async move { compose(work) }));
            }
        }
        if let Some(gpu) = live.gpu.as_ref() {
            live.controller.pump(
                live.work.as_ref(),
                &mut live.queue,
                &gpu.images,
                &mut gpu_queue,
            );
            if live.controller.busy() {
                if let Some(material) = doc
                    .objects
                    .iter_mut()
                    .find(|o| o.id == object)
                    .and_then(|o| o.materials.get_mut(slot))
                {
                    if material.material != gpu.material {
                        material.material = gpu.material.clone();
                        doc.revision += 1;
                    }
                }
                if session.channel == 4
                    && materials
                        .get(&gpu.material)
                        .is_some_and(|m| m.alpha_mode != AlphaMode::Blend)
                {
                    if let Some(mut material) = materials.get_mut(&gpu.material) {
                        material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
        }
        if bound {
            session.busy = live.task.is_some() || !live.queue.is_empty() || live.controller.busy();
        }
    }
    for (&index, doc) in &mut state.documents {
        doc.paint_busy = runtime.lives.iter().any(|((i, _, _), l)| {
            *i == index && (l.task.is_some() || !l.queue.is_empty() || l.controller.busy())
        });
        doc.paint_pending &= doc.paint_busy;
    }
    let saves = runtime
        .deferred_save
        .iter()
        .filter(|(i, _)| state.documents.get(i).is_some_and(|d| !d.paint_busy))
        .map(|(i, c)| (*i, *c))
        .collect::<Vec<_>>();
    for (index, close) in saves {
        runtime.deferred_save.remove(&index);
        if editor.viewing_model == Some(index) {
            messages.p1().write(if close {
                Action::SaveAndContinue
            } else {
                Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save))
            });
            editor.status = "Сохранение рисунка и слоёв…".into();
        }
    }
}

fn node_rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Option<Rect> {
    let size = node.size() * node.inverse_scale_factor();
    (size.min_element() > 2.).then(|| {
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            size,
        )
    })
}
pub(in crate::editor::ui::model_viewer) fn input(
    mut runtime: ResMut<Runtime>,
    mut paint: ResMut<PaintState>,
    mut state: ResMut<editing::State>,
    mut editor: ResMut<Editor>,
    viewer: Res<Viewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    inputs: Query<(), With<TextInput>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    texture: Query<(&ComputedNode, &UiGlobalTransform), With<TextureCanvas>>,
    controls: Query<(&ComputedNode, &UiGlobalTransform), With<Control>>,
    drawer: Res<DrawerMotion>,
) {
    let active = editor
        .viewing_model
        .filter(|i| state.mode(Some(*i)) == Mode::Paint);
    let blocked = active.is_none()
        || editor.pending.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
        || !window.focused
        || drawer.resizing
        || focus.get().is_some_and(|e| inputs.contains(e))
        || keys.any_pressed([
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ])
        || mouse.pressed(MouseButton::Right)
        || mouse.pressed(MouseButton::Middle);
    if blocked || !mouse.pressed(MouseButton::Left) {
        if let Some((key, _, _)) = runtime.gesture.take() {
            if let Some(live) = runtime.lives.get_mut(&key) {
                live.queue.push_back(Event::End);
            }
        }
        runtime.mapping = None;
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let index = active.unwrap();
    let Some(doc) = state.documents.get(&index) else {
        return;
    };
    let Some(session) = paint.sessions.get_mut(&index) else {
        return;
    };
    let Some((object, slot)) = session.binding else {
        return;
    };
    let Some(object) = doc.objects.iter().find(|o| o.id == object) else {
        return;
    };
    let over_control = controls
        .iter()
        .any(|(n, t)| node_rect(n, t).is_some_and(|r| r.contains(cursor)));
    if over_control || drawer.covers(cursor, &window) {
        return;
    }
    let uv_rect = texture.single().ok().and_then(|(n, t)| node_rect(n, t));
    let model_rect = canvas.single().ok().and_then(|(n, t)| node_rect(n, t));
    let (uv, rect) = if let Some(r) =
        uv_rect.filter(|r| r.contains(cursor) && session.view != View::Three)
    {
        (true, r)
    } else if let Some(r) = model_rect.filter(|r| r.contains(cursor) && session.view != View::Two) {
        (false, r)
    } else {
        return;
    };
    let key = (index, object.id, slot);
    if !has_uv(object, slot) {
        if mouse.just_pressed(MouseButton::Left) {
            editor.status =
                "Для рисования нужна UV-развёртка. Откройте режим UV и создайте развёртку.".into();
        }
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        let Some(live) = runtime.lives.get_mut(&key) else {
            return;
        };
        if session.tool == PaintTool::Picker {
            let map = mapping(doc, object, slot, &viewer, rect, uv);
            let at = if uv {
                Some((cursor - rect.min) / rect.size())
            } else {
                let (o, d) = map.camera.ray(cursor);
                map.geometry()
                    .bvh
                    .cast(o, d, None)
                    .filter(|(id, _, _)| *id == object.id)
                    .and_then(|(_, face, distance)| {
                        map.geometry()
                            .triangles
                            .iter()
                            .filter(|t| t.face == face)
                            .find_map(|t| {
                                let p = o + d * distance;
                                let a = t.points[1] - t.points[0];
                                let b = t.points[2] - t.points[0];
                                let v = p - t.points[0];
                                let denom = a.dot(a) * b.dot(b) - a.dot(b).powi(2);
                                if denom.abs() < 1.0e-12 {
                                    return None;
                                }
                                let u = (b.dot(b) * v.dot(a) - a.dot(b) * v.dot(b)) / denom;
                                let w = (a.dot(a) * v.dot(b) - a.dot(b) * v.dot(a)) / denom;
                                (u >= -0.001 && w >= -0.001 && u + w <= 1.001)
                                    .then(|| t.uv[0] * (1. - u - w) + t.uv[1] * u + t.uv[2] * w)
                            })
                    })
            };
            if let (Some(at), Some(work)) = (at, live.work.as_ref()) {
                let p = (at.clamp(Vec2::ZERO, Vec2::splat(0.999999)) * work.surface.size as f32)
                    .as_uvec2();
                let pixels = work.surface.composite_rect(
                    0,
                    URect {
                        min: p,
                        max: p + UVec2::ONE,
                    },
                );
                session.picked = Some(pixels[..3].try_into().unwrap());
                session.tool = PaintTool::Brush;
                session.revision += 1;
            }
            return;
        }
        if session.tool == PaintTool::Path {
            editor.status = "Контур пока недоступен; выберите кисть, ластик или заливку.".into();
            return;
        }
        if !session
            .stack
            .layers
            .get(session.stack.selected)
            .is_some_and(|l| l.visible && l.kind == LayerKind::Paint)
        {
            editor.status = "Выберите видимый слой краски или добавьте новый слой.".into();
            return;
        }
        let scalar = match session.channel {
            1 => session.roughness,
            2 => session.metallic,
            _ => session.opacity,
        };
        let color = if matches!(session.channel, 1 | 2 | 3 | 4 | 7) {
            [(scalar * 2.55).round() as u8; 3]
        } else if session.channel == 6 {
            [128, 128, 255]
        } else {
            session.brush_color()
        };
        let brush = Brush {
            channel: session.channel,
            color,
            radius: (session.size * 0.5).max(0.5),
            hardness: session.hardness / 100.,
            flow: session.flow / 100.,
            opacity: if session.channel == 4 {
                1.
            } else {
                session.opacity / 100.
            },
            erase: session.tool == PaintTool::Eraser,
            fill: session.tool == PaintTool::Fill,
        };
        live.queue.push_back(Event::Begin(brush));
        runtime.gesture = Some((key, uv, cursor));
        let cache_key = (key, doc.revision, view_pose(&viewer), rect, uv);
        let map = runtime
            .map_cache
            .as_ref()
            .filter(|(key, _)| *key == cache_key)
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| Arc::new(mapping(doc, object, slot, &viewer, rect, uv)));
        runtime.map_cache = Some((cache_key, map.clone()));
        runtime.mapping = Some(map);
    }
    let Some((gesture, was_uv, previous)) = runtime.gesture else {
        return;
    };
    if gesture != key || was_uv != uv {
        return;
    }
    let step = (session.size * session.spacing / 100.).max(1.);
    if !mouse.just_pressed(MouseButton::Left) && previous.distance(cursor) < step {
        return;
    }
    let count = (previous.distance(cursor) / step).ceil().max(1.) as usize;
    let points = (1..=count.min(512))
        .map(|i| previous.lerp(cursor, i as f32 / count.min(512) as f32))
        .collect();
    let Some(map) = runtime.mapping.clone() else {
        return;
    };
    if let Some(live) = runtime.lives.get_mut(&key) {
        live.queue.push_back(Event::Points(points, map));
    }
    runtime.gesture = Some((key, uv, cursor));
    if let Some(doc) = state.documents.get_mut(&index) {
        doc.paint_busy = true;
        doc.paint_pending = true;
    }
}
impl Runtime {
    #[cfg(test)]
    pub(super) fn worker_idle(&self, index: usize) -> bool {
        self.lives
            .iter()
            .filter(|((i, _, _), _)| *i == index)
            .all(|(_, l)| l.task.is_none() && l.queue.is_empty())
    }
    #[cfg(test)]
    pub(super) fn texture_ids(&self, index: usize) -> Vec<bevy::asset::AssetId<Image>> {
        self.lives
            .iter()
            .filter(|((i, _, _), _)| *i == index)
            .flat_map(|(_, l)| l.gpu.iter().flat_map(|g| g.images.iter().map(Handle::id)))
            .collect()
    }
    pub(super) fn preview(
        &self,
        index: usize,
        object: usize,
        slot: usize,
        channel: usize,
    ) -> Option<Handle<Image>> {
        self.lives
            .get(&(index, object, slot))?
            .gpu
            .as_ref()
            .map(|gpu| gpu.preview[channel].clone())
    }
}
pub(in crate::editor::ui::model_viewer) fn cursor(
    editor: Res<Editor>,
    state: Res<editing::State>,
    paint: Res<PaintState>,
    ui: Res<ViewerUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    drawer: Res<DrawerMotion>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    canvases: Query<Entity, Or<(With<Canvas>, With<TextureCanvas>)>>,
    controls: Query<Entity, With<Control>>,
    mut cursor: Query<(&mut Node, &mut Visibility), With<BrushCursor>>,
) {
    let Ok((mut node, mut visible)) = cursor.single_mut() else {
        return;
    };
    *visible = Visibility::Hidden;
    let Some(index) = editor
        .viewing_model
        .filter(|i| state.mode(Some(*i)) == Mode::Paint)
    else {
        return;
    };
    let Some(session) = paint.sessions.get(&index) else {
        return;
    };
    let Some(point) = window.cursor_position() else {
        return;
    };
    if !window.focused
        || editor.pending.is_some()
        || editor.browser_popup
        || drawer.covers(point, &window)
        || mouse.pressed(MouseButton::Right)
        || mouse.pressed(MouseButton::Middle)
        || keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
    {
        return;
    }
    let over = canvases.iter().any(|e| {
        geometry
            .get(e)
            .ok()
            .and_then(|(n, t)| node_rect(n, t))
            .is_some_and(|r| r.contains(point))
    });
    let control = controls.iter().any(|e| {
        geometry
            .get(e)
            .ok()
            .and_then(|(n, t)| node_rect(n, t))
            .is_some_and(|r| r.contains(point))
    });
    if !over || control {
        return;
    }
    let Some(overlay) = geometry
        .get(ui.overlay)
        .ok()
        .and_then(|(n, t)| node_rect(n, t))
    else {
        return;
    };
    let size = if matches!(session.tool, PaintTool::Brush | PaintTool::Eraser) {
        session.size
    } else {
        10.
    };
    let p = point - overlay.min - Vec2::splat(size * 0.5);
    node.left = px(p.x);
    node.top = px(p.y);
    node.width = px(size);
    node.height = px(size);
    *visible = Visibility::Visible;
}
#[cfg(test)]
#[path = "model_paint_perf_tests.rs"]
mod performance;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        model_document::{MaterialSlot, glb::Glb},
        model_mesh::{Corner, Face, Geometry},
    };
    pub(super) fn paint_fixture() -> (Work, Mapping, std::path::PathBuf) {
        let mut mesh = Geometry::default();
        for (z, umin, umax) in [(0., 0.05, 0.45), (-1., 0.55, 0.95)] {
            let start = mesh.positions.len();
            mesh.positions.extend([
                Vec3::new(-1., -1., z),
                Vec3::new(1., -1., z),
                Vec3::new(1., 1., z),
                Vec3::new(-1., 1., z),
            ]);
            let uv = [
                Vec2::new(umin, 0.05),
                Vec2::new(umax, 0.05),
                Vec2::new(umax, 0.95),
                Vec2::new(umin, 0.95),
            ];
            mesh.faces.push(Face {
                corners: (0..4)
                    .map(|i| {
                        let mut c = Corner::new(start + i);
                        c.uv = uv[i];
                        c
                    })
                    .collect(),
                material: 0,
                smooth: false,
            });
        }
        let object = Object {
            id: 0,
            source_node: None,
            name: "Plane".into(),
            geometry: mesh.into(),
            transform: Transform::IDENTITY,
            materials: vec![MaterialSlot {
                material: default(),
                source: serde_json::json!({"material":0}),
                paint: None,
            }],
            hidden: false,
        };
        let folder = std::env::temp_dir().join(format!(
            "bevy-paint-test-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("model.glb");
        std::fs::write(&path, Glb::new_asset(&[object.clone()]).unwrap()).unwrap();
        let doc = Document::new(Glb::load(&path).unwrap(), vec![object]);
        let viewer = Viewer {
            distance: 4.,
            radius: 2.,
            ..default()
        };
        let map = mapping(
            &doc,
            &doc.objects[0],
            0,
            &viewer,
            Rect::from_corners(Vec2::ZERO, Vec2::splat(512.)),
            false,
        );
        let surface = Surface {
            size: 64,
            base: std::array::from_fn(|_| data::solid(64, [255; 4])),
            stack: Stack::default(),
            material: serde_json::json!({}),
        };
        (
            Work {
                surface,
                stroke: None,
            },
            map,
            folder,
        )
    }
    pub(super) fn brush() -> Brush {
        Brush {
            channel: 0,
            color: [220, 20, 30],
            radius: 48.,
            hardness: 1.,
            flow: 1.,
            opacity: 1.,
            erase: false,
            fill: false,
        }
    }
    #[test]
    fn model_paint_brush_hits_visible_surface_and_caps_stroke_opacity() {
        let (work, map, folder) = paint_fixture();
        let map = Arc::new(map);
        let mut brush = brush();
        brush.opacity = 0.5;
        let result = batch(
            work,
            vec![
                Event::Begin(brush),
                Event::Points(vec![Vec2::splat(256.); 8], map),
                Event::End,
            ],
            false,
        );
        let pixels = result.work.surface.stack.layers[0].channels[0]
            .as_ref()
            .unwrap();
        let p = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(p(16, 32), [220, 20, 30, 128]);
        assert_eq!(p(48, 32), [0; 4], "rear plane must stay unpainted");
        assert_eq!(result.history.len(), 1, "one undo entry per stroke");
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn model_paint_eraser_restores_base_and_uv_fill_respects_face() {
        let (work, mut map, folder) = paint_fixture();
        map.uv = true;
        let map = Arc::new(map);
        let mut b = brush();
        b.fill = true;
        let painted = batch(
            work,
            vec![
                Event::Begin(b),
                Event::Points(vec![Vec2::new(128., 256.)], map.clone()),
                Event::End,
            ],
            false,
        );
        let pixels = painted.work.surface.stack.layers[0].channels[0]
            .as_ref()
            .unwrap();
        assert_eq!(
            &pixels[(32 * 64 + 16) * 4..(32 * 64 + 16) * 4 + 4],
            &[220, 20, 30, 255]
        );
        assert_eq!(&pixels[(32 * 64 + 48) * 4..(32 * 64 + 48) * 4 + 4], &[0; 4]);
        b.erase = true;
        b.fill = false;
        let erased = batch(
            painted.work,
            vec![
                Event::Begin(b),
                Event::Points(vec![Vec2::new(128., 256.)], map),
                Event::End,
            ],
            false,
        );
        assert_eq!(
            &erased.work.surface.composite(0)[(32 * 64 + 16) * 4..(32 * 64 + 16) * 4 + 4],
            &[255; 4]
        );
        std::fs::remove_dir_all(folder).unwrap();
    }
}
