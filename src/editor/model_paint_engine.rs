//! Surface painting runs on the compute pool. The UI only queues pointer samples
//! and uploads completed textures. No scene materials are modified in place.
use super::*;
use crate::editor::{
    model_document::{Document, Object},
    model_paint_data as data,
};
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    input_focus::InputFocus,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    ui_widgets::TextInput,
};
use std::{collections::VecDeque, sync::Arc};

#[cfg(feature = "mcp")]
#[path = "model_paint_mcp_input.rs"]
mod mcp_input;
#[cfg(feature = "mcp")]
pub(in crate::editor) use mcp_input::McpPaintInput;

type Key = (usize, usize, usize);
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct TextureCanvas;
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct TextureViewport;
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct BrushCursor;
#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct Runtime {
    lives: HashMap<Key, Live>,
    gesture: Option<Gesture>,
    brush_adjustment: Option<BrushAdjustment>,
    uv_panning: bool,
    line_anchor: Option<LineAnchor>,
    map_cache: Option<((Key, GeometryVersion, CameraMap, bool), Arc<Mapping>)>,
    deferred_save: HashMap<usize, bool>,
    fix: fix_brush::Runtime,
    picker: picker::Runtime,
}
#[path = "model_paint_uv_navigation.rs"]
pub(in crate::editor::ui::model_viewer) mod uv_navigation;
struct BrushAdjustment {
    model: usize,
    origin: Vec2,
    size: f32,
    hardness: f32,
}
struct Gesture {
    key: Key,
    mapping: Arc<Mapping>,
    path: stroke::Path,
    fill_drag: Option<fill_drag::Drag>,
    records_anchor: bool,
}
struct LineAnchor {
    key: Key,
    uv: bool,
    camera: CameraMap,
    point: Vec2,
}
impl LineAnchor {
    fn matches(&self, key: Key, map: &Mapping) -> bool {
        self.key == key
            && self.uv == map.uv
            && self.camera.rect == map.camera.rect
            && (map.uv
                || (self.camera.pose == map.camera.pose
                    && self.camera.ortho == map.camera.ortho
                    && self.camera.fov == map.camera.fov
                    && (!map.camera.ortho || self.camera.height == map.camera.height)))
    }
}
struct Live {
    work: Option<Work>,
    task: Option<Task<Batch>>,
    queue: VecDeque<Event>,
    gpu: Option<Gpu>,
    published: Option<Arc<Surface>>,
    origin: crate::editor::model_document::MaterialSlot,
    source_mesh: crate::editor::model_document::MeshData,
    resizing: bool,
    ui_stack: Stack,
    controller: gpu_control::Controller,
    path: path_editor::EditPath,
}
impl Live {
    fn new(
        doc: &Document,
        object: &Object,
        slot: usize,
        images: &Assets<Image>,
        materials: &Assets<StandardMaterial>,
    ) -> Self {
        let origin = object.materials[slot].clone();
        Self {
            work: None,
            task: Some(initial(doc, object, slot, images, materials)),
            queue: VecDeque::new(),
            gpu: None,
            published: origin.paint.clone(),
            ui_stack: origin.paint.as_ref().map_or_else(Stack::default, |surface| surface.stack.clone()),
            origin,
            source_mesh: object.geometry.clone(),
            resizing: false,
            controller: Default::default(),
            path: Default::default(),
        }
    }

    fn enqueue_stamps(&mut self, points: Vec<Stamp>, mapping: Arc<Mapping>) {
        if let Some(Event::Points(batch, previous)) = self.queue.back_mut()
            && Arc::ptr_eq(previous, &mapping)
        {
            batch.extend(points);
        } else {
            self.queue.push_back(Event::Points(points, mapping));
        }
    }
}
struct Work {
    surface: Surface,
    preview: crate::editor::model_document::PaintPreview,
    baked_preview: Option<Arc<composite::PreviewBake>>,
    #[cfg(test)]
    stroke: Option<Stroke>,
}
impl Work {
    fn new(surface: Surface, preview: crate::editor::model_document::PaintPreview) -> Self {
        Self {
            surface,
            preview,
            baked_preview: None,
            #[cfg(test)]
            stroke: None,
        }
    }
}
struct Batch {
    work: Work,
    maps: Option<Updates>,
    history: Vec<Stack>,
    #[cfg(feature = "mcp")]
    surface_selection: Option<Result<super::surface_selection::Report, String>>,
}
struct Gpu {
    material: Handle<StandardMaterial>,
    images: [Handle<Image>; composite::IMAGE_COUNT],
    preview: [Handle<Image>; 8],
    trace_revision: u64,
    baked_preview: Option<Arc<composite::PreviewBake>>,
    original_occlusion: (Option<Handle<Image>>, bevy::mesh::UvChannel),
    emission_strength: f32,
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
    material: Option<BrushMaterial>,
    radius: f32,
    hardness: f32,
    flow: f32,
    opacity: f32,
    erase: bool,
    fill: bool,
    fill_mode: FillMode,
    fill_angle: f32,
    fill_padding: f32,
    variation: stroke::Variation,
    tip: Option<BrushTip>,
    grain: Option<BrushGrain>,
    mirror: [f32; 4],
    projection: BrushProjection,
    paint_through: bool,
    facing_limit: f32,
}
#[derive(Clone, Copy)]
struct BrushMaterial {
    channels: u8,
    colors: [[u8; 3]; data::CHANNEL_COUNT],
}
impl Brush {
    fn channels(self) -> u16 {
        self.material.map_or(1 << self.channel, |material| u16::from(material.channels))
    }

    fn for_channel(mut self, channel: usize) -> Self {
        if let Some(material) = self.material {
            self.color = material.colors[channel];
        }
        self.channel = channel;
        self
    }
}
fn channels_in(mask: u16) -> impl Iterator<Item = usize> {
    (0..=data::MASK_TARGET).filter(move |&channel| mask & (1 << channel) != 0)
}
#[derive(Clone, Copy)]
struct BrushTip {
    image: bevy::asset::AssetId<Image>,
    angle: f32,
    follow: bool,
}
#[derive(Clone, Copy, PartialEq)]
struct BrushGrain {
    image: bevy::asset::AssetId<Image>,
    strength: f32,
    repeat: f32,
    angle: f32,
    invert: bool,
    uv: bool,
}
enum Event {
    Begin(Brush),
    Points(Vec<Stamp>, Arc<Mapping>),
    Region(fill_drag::Region, Arc<Mapping>),
    End,
}
#[path = "model_paint_fill_drag.rs"]
mod fill_drag;
#[path = "model_paint_fix_brush.rs"]
mod fix_brush;
#[path = "model_paint_picker.rs"]
mod picker;
#[path = "model_paint_fill_preview.rs"]
pub(in crate::editor::ui::model_viewer) mod fill_preview;
#[path = "model_paint_live_mask.rs"]
pub(in crate::editor::ui::model_viewer) mod live_mask;
#[path = "model_paint_path.rs"]
mod path_editor;
#[path = "model_paint_projection.rs"]
pub(super) mod projection;
use projection::*;
pub(super) use projection::weights as uv_weights;
#[path = "model_paint_import_uv.rs"]
mod import_uv;
#[path = "model_paint_stroke.rs"]
mod stroke;
use stroke::Stamp;
#[path = "model_paint_composite.rs"]
mod composite;
#[path = "model_paint_surface.rs"]
pub(in crate::editor) mod surface;
#[cfg(test)]
use composite::Dirty;
use composite::Updates;
#[path = "model_paint_gpu.rs"]
pub(in crate::editor::ui::model_viewer) mod gpu;
#[path = "model_paint_gpu_control.rs"]
mod gpu_control;
#[cfg(test)]
fn paint_points(work: &mut Work, points: &[Stamp], map: &Mapping) -> Option<URect> {
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
        .filter_map(|&stamp| {
            let p = stamp.point;
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
            Some((stamp, face))
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return None;
    }
    let mut bounds = Rect::from_center_size(samples[0].0.point, Vec2::ZERO);
    for (stamp, _) in &samples {
        let radius = Vec2::splat(brush.radius * stamp.radius);
        bounds.min = bounds.min.min(stamp.point - radius);
        bounds.max = bounds.max.max(stamp.point + radius);
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
                for (stamp, face) in &samples {
                    let fall = if brush.fill {
                        if *face == Some(triangle.face) { 1. } else { 0. }
                    } else {
                        data::coverage(
                            projected.distance(stamp.point)
                                / (brush.radius * stamp.radius).max(0.5),
                            brush.hardness,
                        )
                    };
                    keep *= 1.
                        - if brush.fill {
                            fall
                        } else {
                            fall * brush.flow * stamp.flow
                        };
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
            Event::Region(_, _) => unreachable!("Region fill uses the GPU stroke path"),
        }
    }
    let maps = dirty
        .iter()
        .any(Option::is_some)
        .then(|| composite::prepare(&work.surface, dirty, work.preview.relief_factor(work.surface.size)));
    Batch {
        work,
        maps,
        history,
        #[cfg(feature = "mcp")]
        surface_selection: None,
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
    let maps = compose_maps(&work.surface, work.preview, work.baked_preview.as_deref());
    Batch {
        work,
        maps: Some(maps),
        history: Vec::new(),
        #[cfg(feature = "mcp")]
        surface_selection: None,
    }
}

fn compose_maps(
    surface: &Surface,
    preview: crate::editor::model_document::PaintPreview,
    baked: Option<&composite::PreviewBake>,
) -> Updates {
    let rect = URect {
        min: UVec2::ZERO,
        max: UVec2::splat(surface.size),
    };
    let mut maps = composite::prepare_preview(surface, [Some(rect); 8], preview.relief_factor(surface.size), baked);
    let mask = surface.stack
        .layers
        .get(surface.stack.selected)
        .and_then(|layer| layer.mask.as_ref());
    maps.textures[composite::MASK_PREVIEW] = Some(composite::Patch {
        rect,
        rgba: mask.map_or_else(
            || vec![255; (surface.size as usize).pow(2) * 4],
            |mask| mask.effective().to_vec(),
        ),
    });
    maps
}

pub(in crate::editor::ui::model_viewer) struct SurfacePreview(Updates);

impl SurfacePreview {
    pub fn prepare(surface: &Surface) -> Self {
        Self(compose_maps(surface, Default::default(), None))
    }

    pub fn upload(
        self,
        slot: &mut crate::editor::model_document::MaterialSlot,
        images: &mut Assets<Image>,
        materials: &mut Assets<StandardMaterial>,
        uploads: &mut crate::editor::image_texture::TextureUploads,
    ) {
        let mut gpu = None;
        upload_textures(&mut gpu, self.0, &slot.material, 1., None, images, materials, uploads);
        slot.material = gpu.unwrap().material;
    }
}

fn initial(
    doc: &Document,
    object: &Object,
    slot: usize,
    images: &Assets<Image>,
    materials: &Assets<StandardMaterial>,
) -> Task<Batch> {
    let source = surface_source(doc, object, slot, images, materials, &mut HashMap::new());
    let mesh = object.geometry.clone();
    let uv_offset = object.uv_offset(slot);
    let transform = object.transform;
    let preview = doc.paint_preview;
    AsyncComputeTaskPool::get().spawn(async move {
        let mut surface = source.build();
        surface.bind_texture_fills(&mesh, slot, uv_offset, transform);
        surface.bind_generators(&mesh, slot, uv_offset, transform);
        compose(Work::new(surface, preview))
    })
}

pub(super) enum SurfaceSource {
    Saved(Arc<Surface>),
    Import(Box<SurfaceImport>),
}

pub(super) struct SurfaceImport {
    size: u32,
    mesh: crate::editor::model_document::MeshData,
    slot: usize,
    uv_offset: Vec2,
    material: StandardMaterial,
    definition: serde_json::Value,
    textures: [Option<Arc<Image>>; 5],
}

impl SurfaceSource {
    pub(super) fn build(self) -> Surface {
        match self {
            Self::Saved(surface) => (*surface).clone(),
            Self::Import(source) => imported_surface(
                &source.mesh,
                source.slot,
                source.uv_offset,
                source.material,
                source.definition,
                source.textures.map(|image| image.map(Arc::unwrap_or_clone)),
                source.size,
            ),
        }
    }
}

pub(super) fn surface_source(
    doc: &Document,
    object: &Object,
    slot: usize,
    images: &Assets<Image>,
    materials: &Assets<StandardMaterial>,
    snapshots: &mut HashMap<bevy::asset::AssetId<Image>, Arc<Image>>,
) -> SurfaceSource {
    let slot_material = &object.materials[slot];
    if let Some(surface) = slot_material.paint.clone() {
        return SurfaceSource::Saved(surface);
    }
    let m = materials
        .get(&slot_material.material)
        .cloned()
        .unwrap_or_default();
    let material = slot_material
        .authored
        .as_deref()
        .or_else(|| {
            let index = slot_material.source["material"].as_u64()? as usize;
            doc.source.json["materials"].get(index)
        })
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"doubleSided":true}));
    let textures = [
        &m.base_color_texture,
        &m.metallic_roughness_texture,
        &m.emissive_texture,
        &m.normal_map_texture,
        &m.specular_texture,
    ]
    .map(|texture| {
        let handle = texture.as_ref()?;
        let image = images.get(handle)?;
        Some(snapshots.entry(handle.id()).or_insert_with(|| Arc::new(image.clone())).clone())
    });
    SurfaceSource::Import(Box::new(SurfaceImport {
        size: doc.paint_default_size,
        mesh: object.geometry.clone(),
        slot,
        uv_offset: object.uv_offset(slot),
        material: m,
        definition: material,
        textures,
    }))
}

fn imported_surface(
    mesh: &crate::editor::model_mesh::Geometry,
    slot: usize,
    uv_offset: Vec2,
    m: StandardMaterial,
    material: serde_json::Value,
    [color, rm, emission, normal, specular_texture]: [Option<Image>; 5],
    size: u32,
) -> Surface {
    let mut layout = import_uv::Layout::new(mesh, slot, size, uv_offset);
    let pbr = &material["pbrMetallicRoughness"];
    let mut pixels = |image, info, channel, default| {
        layout.pixels(image, info, channel, m.uv_transform, default)
    };
    let factor = m.base_color.to_srgba().to_f32_array();
    let mut color = pixels(color, &pbr["baseColorTexture"], m.base_color_channel, [255; 4]);
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
    let rm = pixels(
        rm,
        &pbr["metallicRoughnessTexture"],
        m.metallic_roughness_channel,
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
    let mut emission = pixels(emission, &material["emissiveTexture"], m.emissive_channel, [255; 4]);
    let ef = m.emissive.to_f32_array();
    for p in emission.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = (p[c] as f32 * ef[c]).clamp(0., 255.).round() as u8;
        }
        p[3] = 255;
    }
    let normal = pixels(
        normal,
        &material["normalTexture"],
        m.normal_map_channel,
        [128, 128, 255, 255],
    );
    let specular = &material["extensions"]["KHR_materials_specular"];
    let level = if specular.is_object() {
        let factor = specular["specularFactor"].as_f64().unwrap_or(1.) as f32;
        factor * data::dielectric_f0(&material) / data::SPECULAR_MAX_F0
    } else {
        0.16 * m.reflectance.powi(2) / data::SPECULAR_MAX_F0
    };
    let mut specular_map = pixels(
        specular_texture, &specular["specularTexture"], m.specular_channel, [255; 4],
    );
    for pixel in specular_map.chunks_exact_mut(4) {
        let value = (pixel[3] as f32 * level).clamp(0., 255.).round() as u8;
        pixel.copy_from_slice(&[value, value, value, 255]);
    }
    Surface {
        size,
        stack: Stack::default(),
        material,
        baked: None,
        base: [
            Arc::new(color),
            Arc::new(rough),
            Arc::new(metal),
            data::solid(size, [128, 128, 128, 255]),
            Arc::new(opacity),
            Arc::new(emission),
            Arc::new(normal),
            Arc::new(specular_map),
        ],
    }
}
fn upload(
    live: &mut Live,
    updates: Updates,
    slot: &mut crate::editor::model_document::MaterialSlot,
    work: &Work,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    uploads: &mut crate::editor::image_texture::TextureUploads,
) {
    upload_textures(
        &mut live.gpu, updates, &live.origin.material, work.preview.emission_strength,
        work.baked_preview.as_ref(), images, materials, uploads,
    );
    slot.material = live.gpu.as_ref().unwrap().material.clone();
}

fn upload_textures(
    output: &mut Option<Gpu>,
    mut updates: Updates,
    template: &Handle<StandardMaterial>,
    emission_strength: f32,
    baked_preview: Option<&Arc<composite::PreviewBake>>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    uploads: &mut crate::editor::image_texture::TextureUploads,
) {
    let fresh = output.is_none();
    if fresh {
        let textures: [Handle<Image>; composite::IMAGE_COUNT] = std::array::from_fn(|i| {
            let patch = updates.textures[i]
                .take()
                .expect("first paint upload initializes every texture");
            let mut image = Image::from_dynamic(
                image::DynamicImage::ImageRgba8(
                    image::RgbaImage::from_raw(patch.rect.width(), patch.rect.height(), patch.rgba)
                        .unwrap(),
                ),
                i == 0 || i == 3 || i == composite::MASK_PREVIEW,
                RenderAssetUsages::default(),
            );
            if i < 2 || (cfg!(test) && (i == 2 || i == composite::SPECULAR_MATERIAL)) {
                image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
            }
            images.add(image)
        });
        let template = materials.get(template).cloned().unwrap_or_default();
        let original_occlusion = (template.occlusion_texture.clone(), template.occlusion_channel.clone());
        let material = materials.add(template);
        let preview = std::array::from_fn(|c| textures[composite::PREVIEW[c]].clone());
        *output = Some(Gpu {
            material,
            images: textures,
            preview,
            trace_revision: 0,
            baked_preview: None,
            original_occlusion,
            emission_strength,
        });
    } else {
        let gpu = output.as_mut().unwrap();
        if updates.textures.iter().any(Option::is_some) {
            gpu.trace_revision += 1;
        }
        for (i, patch) in updates.textures.into_iter().enumerate() {
            if let Some(p) = patch {
                uploads.patch(gpu.images[i].id(), p.rect.min, p.rect.size(), p.rgba);
            }
        }
    }
    let gpu = output.as_mut().unwrap();
    let bake_changed = !composite::same_preview_bake(gpu.baked_preview.as_ref(), baked_preview);
    if bake_changed {
        gpu.baked_preview = baked_preview.cloned();
        if let Some(mut material) = materials.get_mut(&gpu.material) {
            let occlusion = baked_preview.and_then(|bake| bake.ao.as_ref().map(|ao| (bake.maps.size, ao)));
            (material.occlusion_texture, material.occlusion_channel) = if let Some((size, ao)) = occlusion {
                let texture = Image::new(
                    Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                    TextureDimension::D2, ao.clone(), TextureFormat::R8Unorm, RenderAssetUsages::default(),
                );
                (Some(images.add(texture)), bevy::mesh::UvChannel::Uv0)
            } else {
                gpu.original_occlusion.clone()
            };
        }
    }
    let emission_changed = gpu.emission_strength != emission_strength;
    gpu.emission_strength = emission_strength;
    if fresh || emission_changed || updates.transparent.is_some() {
        if let Some(mut m) = materials.get_mut(&gpu.material) {
            if fresh || emission_changed {
                m.emissive = LinearRgba::new(emission_strength, emission_strength, emission_strength, 1.);
            }
            if fresh {
                m.base_color = Color::WHITE;
                m.base_color_texture = Some(gpu.images[0].clone());
                m.metallic = 1.;
                m.perceptual_roughness = 1.;
                m.metallic_roughness_texture = Some(gpu.images[1].clone());
                m.normal_map_texture = Some(gpu.images[2].clone());
                m.flip_normal_map_y = false;
                m.emissive_texture = Some(gpu.images[3].clone());
                m.reflectance = 2.;
                m.specular_texture = Some(gpu.images[composite::SPECULAR_MATERIAL].clone());
                m.specular_channel = bevy::mesh::UvChannel::Uv0;
                m.uv_transform = bevy::math::Affine2::IDENTITY;
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
        }
    }
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
    mut baked: ResMut<bake::State>,
    recovery: Res<editing::recovery::Preferences>,
    mut messages: ParamSet<(MessageReader<Action>, MessageWriter<Action>)>,
    #[cfg(feature = "mcp")] render_requests: Query<&mcp_preview::offscreen::RenderRequest>,
) {
    let replies = std::mem::take(&mut *gpu_queue.replies.lock().unwrap());
    for reply in replies {
        match reply {
            gpu::Reply::Layer { id, stroke, pixels } => {
                if let Some(live) = runtime.lives.values_mut().find(|l| l.controller.owns(id)) {
                    live.controller.reply(id, stroke, pixels);
                } else {
                    runtime.fix.reply(id, stroke, pixels.and_then(|mut channels| {
                        if channels.len() == 1 {
                            Ok(channels.pop().unwrap())
                        } else {
                            Err("Некорректное число каналов исправления".into())
                        }
                    }));
                }
            }
            gpu::Reply::Pick { id, sample } => runtime.picker.reply(id, sample),
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
            paint.activate(index, recovery.settings().undo_steps);
            let session = paint.sessions.get_mut(&index).unwrap();
            session.set_appearance.count(doc);
            let target = doc.paint_target(session.set);
            if let Some((object, slot)) = target {
                let key = (index, object.id, slot);
                if session.binding != Some((object.id, slot)) {
                    session.editing_mask = false;
                    session.show_mask = false;
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
                    // Keep the saved graph result when opening a set. A new bake
                    // or a subsequent mesh change will refresh the generated maps.
                    if !runtime.lives.contains_key(&key)
                        && !session.mesh_graph_refresh.contains(&(object.id, slot))
                        && !super::material_graph::needs_initial_render(&session.stack)
                        && let Some(source) = super::material_graph::surface::Source::capture(
                            doc, (object.id, slot),
                        )
                    {
                        session.graph_surfaces.insert((object.id, slot), Arc::new(source));
                    }
                    session.texture_size = doc.paint_size(&object.materials[slot]);
                    session.revision += 1;
                }
                runtime.lives.entry(key).or_insert_with(|| Live::new(doc, object, slot, &images, &materials));
            }
            #[cfg(feature = "mcp")]
            let render_channels = render_requests.iter().any(|request| request.needs_channels(index));
            #[cfg(not(feature = "mcp"))]
            let render_channels = false;
            if (session.diagnostic_channel().is_some() || render_channels)
                && !runtime.lives.iter().any(|((model, _, _), live)| *model == index && live.task.is_some())
                && let Some((object, slot)) = doc.objects.iter()
                    .flat_map(|object| (0..object.materials.len()).map(move |slot| (object, slot)))
                    .find(|(object, slot)| {
                        !object.hidden && session.set_appearance.visibility.shown(object.id, *slot)
                            && !runtime.lives.contains_key(&(index, object.id, *slot))
                            && has_uv(object, *slot)
                    })
            {
                runtime.lives.insert((index, object.id, slot), Live::new(doc, object, slot, &images, &materials));
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
        let Some(source) = doc.objects.iter().find(|o| o.id == object) else {
            continue;
        };
        let Some(current) = source.materials.get(slot) else {
            continue;
        };
        let same = match (&current.paint, &live.published) {
            (None, None) if !bound || doc.mode != Mode::Paint => true,
            (None, None) => {
                let own_gpu = live.gpu.as_ref().is_some_and(|gpu| current.material == gpu.material);
                (own_gpu || current.material == live.origin.material)
                    && current.authored == live.origin.authored
                    && current.source["material"] == live.origin.source["material"]
                    && current.udim() == live.origin.udim()
                    && source.geometry.shares_storage_with(&live.source_mesh)
            }
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
            live.source_mesh = source.geometry.clone();
            live.resizing = false;
            live.gpu = None;
            let surface = current.paint.as_deref();
            let stack = surface.map_or_else(Stack::default, |surface| surface.stack.clone());
            if bound {
                session.stack = stack.clone();
                session.texture_size = doc.paint_size(current);
                session.resize_request = None;
                session.undo.clear();
                session.redo.clear();
                session.revision += 1;
            }
            live.ui_stack = stack;
            live.work = None;
            live.task = Some(initial(doc, source, slot, &images, &materials));
        }
        if let Some(result) = live.task.as_mut().and_then(|job| block_on(poll_once(job))) {
            live.task = None;
            let resized = std::mem::take(&mut live.resizing);
            let finished = true;
            let history_changed = !result.history.is_empty();
            doc.recovery_edits += result.history.len() as u64 + u64::from(resized);
            if bound {
                #[cfg(feature = "mcp")]
                if let Some(selection) = result.surface_selection {
                    if selection.is_ok() {
                        session.editing_mask = true;
                    }
                    session.surface_selection = Some(((object, slot), selection));
                }
                for history in result.history {
                    session.record_undo(history);
                }
                if finished {
                    session.stack = result.work.surface.stack.clone();
                    session.texture_size = result.work.surface.size;
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
                        &result.work,
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
                    let origin_size = live.origin.paint.as_ref().map_or(doc.paint_default_size, |s| s.size);
                    if result.work.surface.size != origin_size
                        || !data::same_stack(&result.work.surface.stack, &origin_stack)
                        || result.work.surface.has_texture_fills()
                        || (result.work.surface.needs_bake()
                            && !result.work.surface.same_bake(live.origin.paint.as_deref()))
                    {
                        let surface = Arc::new(result.work.surface.clone());
                        material.paint = Some(surface.clone());
                        live.published = Some(surface);
                    } else if !live.controller.busy() && live.queue.is_empty() {
                        material.material = live.origin.material.clone();
                        material.paint = live.origin.paint.clone();
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
                match live.controller.archive(work, &mut gpu_queue) {
                    Ok(work) => live.work = Some(work),
                    Err(job) => live.task = Some(job),
                }
            }
        }
        if live.task.is_none() {
            if bound && live.queue.is_empty() && !live.controller.busy() {
                if let Some(size) = session.resize_request.take() {
                    let mut work = live.work.take().unwrap();
                    let source = doc.objects.iter().find(|o| o.id == object).unwrap();
                    let mesh = source.geometry.clone();
                    let uv_offset = source.uv_offset(slot);
                    let transform = source.transform;
                    live.controller.invalidate(&mut gpu_queue);
                    live.gpu = None;
                    live.resizing = true;
                    session.undo.clear();
                    session.redo.clear();
                    session.queued.clear();
                    session.busy = true;
                    live.task = Some(AsyncComputeTaskPool::get().spawn(async move {
                        work.surface.resize(size);
                        work.surface.bind_texture_fills(&mesh, slot, uv_offset, transform);
                        work.surface.bind_generators(&mesh, slot, uv_offset, transform);
                        compose(work)
                    }));
                    continue;
                }
                session.busy = false;
                while let Some(action) = session.queued.pop_front() {
                    session.edit(action);
                }
            }
            let work = live.work.as_mut().unwrap();
            let layers_changed = bound && !data::same_layer_list(&session.stack, &live.ui_stack);
            let stack_changed = bound
                && (layers_changed || session.stack.selected != live.ui_stack.selected);
            let composite_changed = stack_changed
                && (session.stack.selected != live.ui_stack.selected
                    || !data::same_composite(&session.stack, &live.ui_stack));
            if stack_changed {
                doc.recovery_edits += u64::from(layers_changed);
                work.surface.stack = session.stack.clone();
                live.ui_stack = session.stack.clone();
            }
            let source = doc
                .objects
                .iter()
                .find(|candidate| candidate.id == object);
            let bake_changed = work.surface.needs_bake()
                .then(|| baked.maps(index, doc, (object, slot), work.surface.size))
                .flatten()
                .filter(|maps| !maps.has_generator_inputs(work.surface.baked.as_ref()))
                .cloned();
            let generators_changed = source.is_some_and(|object| {
                work.surface.bind_generators(&object.geometry, slot, object.uv_offset(slot), object.transform)
            });
            let textures_changed = source.is_some_and(|object| {
                work.surface.texture_fills_need_binding(&object.geometry, slot, object.uv_offset(slot), object.transform)
            });
            let preview_idle = live.queue.is_empty() && !live.controller.busy();
            let preview = if preview_idle { doc.paint_preview } else { work.preview };
            let preview_changed = work.preview != preview;
            let preview_bake_change = if preview_idle {
                let maps = (preview.use_baked_normal || preview.ao_strength > 0.)
                    .then(|| baked.maps(index, doc, (object, slot), work.surface.size))
                    .flatten()
                    .filter(|maps| composite::PreviewBake::available(maps, preview));
                let same = match (work.baked_preview.as_ref(), maps) {
                    (Some(current), Some(next)) => Arc::ptr_eq(&current.maps, next),
                    (None, None) => true,
                    _ => false,
                };
                let bake_settings_changed = maps.is_some()
                    && (preview.use_baked_normal != work.preview.use_baked_normal
                        || preview.ao_strength != work.preview.ao_strength);
                (!same || bake_settings_changed).then(|| maps.cloned())
            } else {
                None
            };
            let recompose = composite_changed || generators_changed || textures_changed
                || bake_changed.is_some() || preview_bake_change.is_some() || preview_changed;
            if stack_changed || recompose {
                if stack_changed || bake_changed.is_some() {
                    doc.paint_pending = true;
                }
                let mut work = live.work.take().unwrap();
                let geometry = if recompose {
                    live.controller.invalidate(&mut gpu_queue);
                    source.map(|object| (object.geometry.clone(), object.uv_offset(slot), object.transform))
                } else {
                    None
                };
                live.task = Some(AsyncComputeTaskPool::get().spawn(async move {
                    if let Some((mesh, uv_offset, transform)) = geometry {
                        if let Some(maps) = bake_changed {
                            work.surface.baked = Some(maps.generator_inputs().clone());
                            work.surface.bind_generators(&mesh, slot, uv_offset, transform);
                        }
                        work.surface.bind_texture_fills(&mesh, slot, uv_offset, transform);
                    }
                    work.preview = preview;
                    if let Some(maps) = preview_bake_change {
                        work.baked_preview = maps.map(|maps| Arc::new(composite::PreviewBake::new(maps, work.preview)));
                    }
                    if recompose {
                        compose(work)
                    } else {
                        Batch {
                            work, maps: None, history: Vec::new(),
                            #[cfg(feature = "mcp")]
                            surface_selection: None,
                        }
                    }
                }));
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
                if live.controller.affects_opacity()
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
    runtime.fix.update(&mut state.documents, &mut baked, &mut images, &mut gpu_queue, &mut editor.status);
    let runtime = &mut *runtime;
    runtime.picker.update(&runtime.lives, &state.documents, &mut paint, &mut editor, &images, &materials, &mut gpu_queue);
    for (&index, doc) in &mut state.documents {
        if runtime.fix.busy(index) && let Some(session) = paint.sessions.get_mut(&index) {
            session.busy = true;
        }
        doc.paint_busy = paint.sessions.get(&index).is_some_and(|s| s.resize_request.is_some())
            || runtime.fix.busy(index)
            || runtime.lives.iter().any(|((i, _, _), l)| {
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

#[derive(Clone, Copy)]
struct StrokeStyle {
    projection: BrushProjection,
    symmetry: Symmetry,
    paint_through: bool,
    angle_limit: f32,
    tip: Option<usize>,
    tip_angle: f32,
    follow_stroke: bool,
    grain: Option<usize>,
    grain_strength: f32,
    grain_scale: f32,
    grain_angle: f32,
    grain_invert: bool,
    grain_uv: bool,
    variation: stroke::Variation,
}

impl From<&Session> for StrokeStyle {
    fn from(session: &Session) -> Self {
        Self {
            projection: session.projection,
            symmetry: session.symmetry,
            paint_through: session.paint_through,
            angle_limit: session.angle_limit,
            tip: session.tip,
            tip_angle: session.tip_angle,
            follow_stroke: session.follow_stroke,
            grain: session.grain,
            grain_strength: session.grain_strength,
            grain_scale: session.grain_scale,
            grain_angle: session.grain_angle,
            grain_invert: session.grain_invert,
            grain_uv: session.grain_uv,
            variation: stroke::Variation {
                scatter: session.brush_scatter / 100.,
                size: session.brush_size_jitter / 100.,
                angle: session.brush_angle_jitter / 100.,
            },
        }
    }
}

fn brush_for(session: &Session, tips: &tips::Library, object: &Object, uv: bool, erase: bool, fill: bool) -> Brush {
    brush_for_style(session, tips, object, uv, erase, fill, StrokeStyle::from(session))
}

fn brush_for_style(session: &Session, tips: &tips::Library, object: &Object, uv: bool, erase: bool, fill: bool, style: StrokeStyle) -> Brush {
    let masking = session.editing_mask && session.has_mask();
    let channel = session.stroke_channels().trailing_zeros() as usize;
    let percent_byte = |value: f32| (value * 2.55).round() as u8;
    let material = (!masking && session.multi_channel_brush).then(|| BrushMaterial {
        channels: session.brush_channels,
        colors: [
            session.brush_color(),
            [percent_byte(session.roughness); 3],
            [percent_byte(session.metallic); 3],
            [percent_byte(session.brush_height); 3],
            [percent_byte(session.brush_alpha); 3],
            session.brush_emission.map(percent_byte),
            [128, 128, 255],
            [percent_byte(session.brush_specular); 3],
        ],
    });
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
    let texture = style
        .grain
        .and_then(|i| tips.grains.get(i))
        .filter(|_| style.grain_strength > 0. && !fill);
    let projection = if uv || fill {
        BrushProjection::Screen
    } else {
        style.projection
    };
    let grain_uv = style.grain_uv || projection == BrushProjection::Uv;
    let symmetry = !uv && style.symmetry != Symmetry::Off && !fill;
    let mut mirror = [0.; 4];
    let mut extent = 1.;
    if symmetry || texture.is_some() {
        let (min, max) = object.geometry.positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        extent = ((max - min) * object.transform.scale.abs())
            .max_element()
            .max(1e-6);
        if symmetry {
            let normal = object.transform.rotation * style.symmetry.normal();
            let center = object.transform.transform_point((min + max) * 0.5);
            mirror = normal.extend(normal.dot(center)).to_array();
        }
    }
    Brush {
        channel,
        material,
        color: if masking {
            [if erase {
                0
            } else {
                session.mask_value
            }; 3]
        } else {
            material.map_or(color, |material| material.colors[channel])
        },
        radius: (session.size * 0.5).max(0.5),
        hardness: session.hardness / 100.,
        flow: session.flow / 100.,
        opacity: if channel == 4 && material.is_none() {
            1.
        } else {
            session.opacity / 100.
        },
        erase: erase
            && (!masking
                || session.stack.layers[session.stack.selected]
                    .mask
                    .as_ref()
                    .is_some_and(|m| m.affine)),
        fill,
        fill_mode: session.fill_mode,
        fill_angle: session.fill_angle,
        fill_padding: session.fill_padding,
        variation: style.variation,
        tip: if fill {
            None
        } else {
            style
                .tip
                .and_then(|i| tips.entries.get(i))
                .map(|tip| BrushTip {
                    image: tip.image.id(),
                    angle: style.tip_angle.to_radians(),
                    follow: style.follow_stroke,
                })
        },
        grain: texture.map(|image| BrushGrain {
            image: image.image.id(),
            strength: style.grain_strength / 100.,
            repeat: if grain_uv {
                style.grain_scale
            } else {
                style.grain_scale / extent
            },
            angle: style.grain_angle.to_radians(),
            invert: style.grain_invert,
            uv: grain_uv,
        }),
        mirror,
        projection,
        paint_through: !uv
            && !fill
            && projection == BrushProjection::Screen
            && style.paint_through,
        facing_limit: style.angle_limit.to_radians().cos(),
    }
}

pub(super) fn node_rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Option<Rect> {
    let size = node.size() * node.inverse_scale_factor();
    (size.min_element() > 2.).then(|| {
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            size,
        )
    })
}

#[derive(SystemParam)]
pub(in crate::editor::ui::model_viewer) struct UvCanvas<'w, 's> {
    image: Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform), With<TextureCanvas>>,
    viewport: Query<'w, 's, (&'static ComputedNode, &'static UiGlobalTransform), With<TextureViewport>>,
}

impl UvCanvas<'_, '_> {
    fn viewport_rect(&self) -> Option<Rect> {
        let (node, transform) = self.viewport.single().ok()?;
        node_rect(node, transform)
    }

    fn camera(&self, viewer: &Viewer) -> Option<CameraMap> {
        let (node, transform) = self.image.single().ok()?;
        let rect = node_rect(node, transform)?;
        let clip = rect.intersect(self.viewport_rect()?);
        (clip.size().min_element() > 0.).then(|| CameraMap { clip, ..camera_map(viewer, rect) })
    }
}
use super::super::super::image_viewer::paint_input::{PointerFrame, Sample};

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
    texture: UvCanvas,
    controls: Query<(&ComputedNode, &UiGlobalTransform), With<Control>>,
    drawer: Res<DrawerMotion>,
    pointer: Res<PointerFrame>,
    baked: Res<bake::State>,
) {
    let active = editor
        .viewing_model
        .filter(|i| state.mode(Some(*i)) == Mode::Paint);
    let session = active.and_then(|index| paint.sessions.get(&index));
    let tool = session.map(|session| session.tool);
    let fill_tool = tool == Some(PaintTool::Fill);
    let path_tool = tool == Some(PaintTool::Path);
    let fix_map = session.and_then(Session::active_bake_fix);
    let uv_view = session.filter(|s| s.view != View::Three)
        .and_then(|_| texture.camera(&viewer));
    let control = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let blocked = active.is_none()
        || session.is_some_and(|session| session.cage_brush_active()
            || session.custom_gizmo_active() || session.graph_canvas_open() || session.panel == Panel::Shader)
        || editor.viewing_image.is_some()
        || editor.pending.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
        || !window.focused
        || drawer.resizing
        || focus.get().is_some_and(|e| inputs.contains(e))
        || keys.any_pressed([
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ])
        || mouse.pressed(MouseButton::Middle)
        || pointer.pen_pan_handled;
    let brush_tool = matches!(tool, Some(PaintTool::Brush | PaintTool::Eraser));
    let bake_read_only = session.is_some_and(Session::paint_blocked_by_bake);
    let cursor = window.cursor_position();
    let set_brush_value = |session: &mut Session, parameter: Parameter, value: f32| {
        let (min, max) = parameter.bounds();
        session.set_value(parameter, value.clamp(min as f32, max as f32));
    };
    if let Some(adjustment) = runtime.brush_adjustment.take() {
        if !blocked && brush_tool && active == Some(adjustment.model) {
            let session = paint.sessions.get_mut(&adjustment.model).unwrap();
            if keys.just_pressed(KeyCode::Escape) {
                set_brush_value(session, Parameter::Size, adjustment.size);
                set_brush_value(session, Parameter::Hardness, adjustment.hardness);
            } else if control && mouse.pressed(MouseButton::Right) {
                if let Some(point) = cursor {
                    let delta = point - adjustment.origin;
                    set_brush_value(session, Parameter::Size, adjustment.size + delta.x);
                    set_brush_value(
                        session,
                        Parameter::Hardness,
                        adjustment.hardness - delta.y * 0.5,
                    );
                }
                runtime.brush_adjustment = Some(adjustment);
            }
        }
        return;
    }
    let start_adjustment = control && mouse.just_pressed(MouseButton::Right);
    let grow = keys.just_pressed(KeyCode::BracketRight);
    let shrink = keys.just_pressed(KeyCode::BracketLeft);
    let resize_by_key = grow != shrink
        && !control
        && !mouse.pressed(MouseButton::Right)
        && !keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if (start_adjustment || resize_by_key)
        && !blocked
        && !bake_read_only
        && brush_tool
        && runtime.gesture.is_none()
        && !runtime.fix.drawing()
        && !mouse.pressed(MouseButton::Left)
        && let Some(index) = active
        && let Some(point) = cursor
        && !drawer.covers(point, &window)
    {
        let session = paint.sessions.get_mut(&index).unwrap();
        let contains = |(node, transform)| {
            node_rect(node, transform).is_some_and(|rect| rect.contains(point))
        };
        let over_uv = uv_view.is_some_and(|camera| camera.clip.contains(point));
        let over_model = session.view != View::Two
            && (!session.showing_bake() || session.showing_map_on_model())
            && canvas.single().ok().is_some_and(contains);
        if (over_uv || over_model) && !controls.iter().any(contains) {
            if start_adjustment {
                runtime.brush_adjustment = Some(BrushAdjustment {
                    model: index,
                    origin: point,
                    size: session.size,
                    hardness: session.hardness,
                });
                return;
            }
            let factor = if grow { 1.15 } else { 1. / 1.15 };
            set_brush_value(session, Parameter::Size, session.size * factor);
        }
    }
    let blocked = blocked || runtime.uv_panning
        || (control && !fill_tool)
        || (!path_tool && mouse.pressed(MouseButton::Right));
    if bake_read_only && !blocked && let Some(index) = active {
        let session = paint.sessions.get_mut(&index).unwrap();
        session.path_command = None;
        let model = (session.view != View::Two).then(|| canvas.single().ok()
            .and_then(|(node, transform)| node_rect(node, transform))).flatten();
        if pointer.samples.iter().any(|sample| matches!(sample, Sample::Down(point)
            if (model.is_some_and(|rect| rect.contains(*point))
                || uv_view.is_some_and(|camera| camera.clip.contains(*point)))
                && !drawer.covers(*point, &window)
                && !controls.iter().any(|(node, transform)| node_rect(node, transform)
                    .is_some_and(|rect| rect.contains(*point)))))
        {
            editor.status = "Запечённая карта открыта для просмотра. Для рисования вернитесь к материалу или включите кисть исправлений.".into();
        }
    }
    if runtime.fix.drawing() && keys.just_pressed(KeyCode::Escape) {
        runtime.fix.request_cancel();
        return;
    }
    if blocked || bake_read_only
        || (keys.just_pressed(KeyCode::Escape)
            && runtime.gesture.as_ref().is_some_and(|g| g.fill_drag.is_some()))
    {
        runtime.end_stroke();
        runtime.fix.finish();
        if let Some(index) = active
            && let Some((object, slot)) = paint.sessions.get(&index).and_then(|s| s.binding)
            && let Some(live) = runtime.lives.get_mut(&(index, object, slot))
        {
            live.path.cancel_pointer();
        }
        if active.is_none() {
            runtime.line_anchor = None;
        }
        return;
    }
    let index = active.unwrap();
    let Some(doc) = state.documents.get(&index) else {
        return;
    };
    let paint = &mut *paint;
    let Some(session) = paint.sessions.get_mut(&index) else {
        return;
    };
    let Some((object, slot)) = session.binding else {
        runtime.end_stroke();
        return;
    };
    let Some(object) = doc.objects.iter().find(|o| o.id == object) else {
        return;
    };
    let model_rect = canvas.single().ok().and_then(|(n, t)| node_rect(n, t));
    let key = (index, object.id, slot);
    runtime.fix.finish_unless(key, fix_map);
    let visibility = session.set_appearance.visibility.clone();
    if !visibility.shown(object.id, slot) && tool != Some(PaintTool::Picker) {
        runtime.end_stroke();
        runtime.fix.finish();
        if pointer.samples.iter().any(|sample| matches!(sample, Sample::Down(_))) {
            editor.status = "Покажите набор текстур, чтобы рисовать по нему.".into();
        }
        return;
    }
    if session.resize_request.is_some() || runtime.lives.get(&key).is_some_and(|live| live.resizing) {
        runtime.end_stroke();
        runtime.fix.finish();
        return;
    }
    if let Some(map) = fix_map {
        runtime.end_stroke();
        let mut pressure = 1.;
        let mut started = false;
        for &sample in &pointer.samples {
            let point = match sample {
                Sample::Pressure(value) => {
                    pressure = value;
                    continue;
                }
                Sample::Move(point) | Sample::Up(point) => {
                    runtime.fix.move_to(point, pressure);
                    if matches!(sample, Sample::Up(_)) {
                        runtime.fix.finish();
                        pressure = 1.;
                    }
                    continue;
                }
                Sample::Down(point) => point,
            };
            if drawer.covers(point, &window)
                || controls.iter().any(|(node, transform)| {
                    node_rect(node, transform).is_some_and(|rect| rect.contains(point))
                })
            {
                continue;
            }
            let target = uv_view.filter(|camera| camera.clip.contains(point))
                .map(|camera| (true, camera))
                .or_else(|| model_rect.filter(|rect| session.showing_map_on_model() && rect.contains(point))
                    .map(|rect| (false, camera_map(&viewer, rect))));
            let Some((uv, camera)) = target else { continue };
            let Some(source) = baked.maps(index, doc, (object.id, slot), session.texture_size)
                .filter(|maps| maps.has_map(map)) else { continue };
            if !has_uv(object, slot) {
                continue;
            }
            let erase = session.tool == PaintTool::Eraser;
            let mut brush = brush_for(session, &paint.tips, object, uv, erase, fill_tool);
            brush.channel = 1;
            brush.material = None;
            brush.color = [(session.bake_fix_value as f32 * 2.55).round() as u8; 3];
            brush.opacity = session.opacity / 100.;
            brush.erase = erase;
            brush.fill = false;
            brush.grain = None;
            let path = stroke::Path::new(
                point, pressure, session.size * session.spacing / 100.,
                session.stabilizer, session.pressure_size, session.pressure_flow,
            );
            started |= runtime.fix.begin(
                key, map, source.clone(), Arc::new(Mapping::for_camera(doc, object, slot, camera, uv, &visibility)),
                object.materials[slot].bake_fixes.clone(), brush, path,
            );
        }
        if started && let Some(doc) = state.documents.get_mut(&index) {
            doc.paint_busy = true;
            doc.paint_pending = true;
        }
        return;
    }
    if path_tool {
        runtime.end_stroke();
        let Some(live) = runtime.lives.get_mut(&key) else { return };
        let views = path_editor::Views {
            model: model_rect.filter(|_| session.view != View::Two).map(|r| camera_map(&viewer, r)),
            uv: uv_view,
        };
        let input = path_editor::Input {
            doc,
            object,
            visibility: &visibility,
            slot,
            views,
            samples: &pointer.samples,
            cursor: pointer.pen_cursor().or_else(|| window.cursor_position()),
            keys: &keys,
            mouse: &mouse,
            can_start: &|point| {
                !drawer.covers(point, &window)
                    && !controls.iter().any(|(n, t)| node_rect(n, t).is_some_and(|r| r.contains(point)))
            },
        };
        match path_editor::edit(live, session, &paint.tips, input) {
            path_editor::Outcome::Idle => {}
            path_editor::Outcome::Applied => {
                let doc = state.documents.get_mut(&index).unwrap();
                doc.paint_busy = true;
                doc.paint_pending = true;
            }
            path_editor::Outcome::Status(text) => editor.status = text.into(),
        }
        return;
    }
    session.path_command = None;
    if let Some(live) = runtime.lives.get_mut(&key) {
        live.path.cancel_pointer();
    }
    if runtime
        .gesture
        .as_ref()
        .is_some_and(|gesture| {
            gesture.key != key || gesture.fill_drag.as_ref().is_some_and(|drag| {
                gesture.mapping.revision != doc.revision
                    || session.tool != PaintTool::Fill
                    || session.stack.selected != drag.layer
                    || session.stroke_channels() != drag.brush.channels()
                    || !session.can_paint_layer()
            })
        })
    {
        runtime.end_stroke();
    }
    let mut pressure = 1.;
    let mut enqueued = false;
    for &sample in &pointer.samples {
        let cursor = match sample {
            Sample::Pressure(value) => {
                pressure = value;
                continue;
            }
            Sample::Move(point) | Sample::Up(point) => {
                if let Some(gesture) = runtime.gesture.as_mut()
                    && let Some(drag) = &mut gesture.fill_drag
                {
                    drag.advance(point, gesture.mapping.camera.clip);
                    if matches!(sample, Sample::Up(_)) {
                        enqueued |= runtime.finish_fill_selection();
                        pressure = 1.;
                    }
                    continue;
                }
                if let Some(gesture) = runtime.gesture.as_mut()
                    && gesture.mapping.camera.clip.contains(point)
                {
                    let points = gesture.path.advance(point, pressure);
                    if !points.is_empty() {
                        let map = gesture.mapping.clone();
                        if let Some(live) = runtime.lives.get_mut(&key) {
                            live.enqueue_stamps(points, map);
                            enqueued = true;
                        }
                    }
                }
                if matches!(sample, Sample::Up(_)) {
                    runtime.end_stroke();
                    pressure = 1.;
                }
                continue;
            }
            Sample::Down(point) => {
                runtime.end_stroke();
                point
            }
        };
        let over_control = controls
            .iter()
            .any(|(n, t)| node_rect(n, t).is_some_and(|r| r.contains(cursor)));
        if over_control || drawer.covers(cursor, &window) {
            continue;
        }
        let (uv, camera) = if let Some(camera) =
            uv_view.filter(|camera| camera.clip.contains(cursor))
        {
            (true, camera)
        } else if let Some(r) =
            model_rect.filter(|r| r.contains(cursor) && session.view != View::Two)
        {
            (false, camera_map(&viewer, r))
        } else {
            continue;
        };
        if session.tool == PaintTool::Picker {
            runtime.picker.begin(index, doc, session, &viewer, (uv, camera.rect), cursor);
            continue;
        }
        if !has_uv(object, slot) {
            editor.status =
                "Для рисования нужна UV-развёртка. Откройте режим UV и создайте развёртку.".into();
            continue;
        }
        if !runtime.lives.contains_key(&key) {
            continue;
        }
        if !session.can_paint_layer() {
            editor.status = "Выберите видимый слой краски или добавьте новый слой.".into();
            continue;
        }
        let erase = session.tool == PaintTool::Eraser || (fill_tool && (session.fill_erase != control));
        let brush = brush_for(session, &paint.tips, object, uv, erase, fill_tool);
        let cache_key = (key, (doc.revision, visibility.revision), camera, uv);
        let map = runtime
            .map_cache
            .as_ref()
            .filter(|(key, _)| *key == cache_key)
            .map(|(_, m)| m.clone())
            .unwrap_or_else(|| Arc::new(Mapping::for_camera(doc, object, slot, camera, uv, &visibility)));
        runtime.map_cache = Some((cache_key, map.clone()));
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let anchor = runtime.line_anchor.as_ref()
            .filter(|anchor| shift && !brush.fill && anchor.matches(key, &map))
            .map(|anchor| anchor.point);
        let mut path = stroke::Path::new(
            anchor.unwrap_or(cursor),
            pressure,
            session.size * session.spacing / 100.,
            session.stabilizer,
            !brush.fill && session.pressure_size,
            !brush.fill && session.pressure_flow,
        );
        let fill_drag = (brush.fill && shift)
            .then(|| fill_drag::Drag::new(
                brush, session.stack.selected, cursor, session.fill_lasso, session.fill_through, erase,
            ));
        if fill_drag.is_none()
            && let Some(live) = runtime.lives.get_mut(&key)
        {
            live.queue.push_back(Event::Begin(brush));
            let mut points = vec![path.first()];
            if anchor.is_some() {
                points.extend(path.line_to(cursor, pressure));
            }
            live.enqueue_stamps(points, map.clone());
            enqueued = true;
        }
        runtime.gesture = Some(Gesture {
            key,
            mapping: map,
            path,
            fill_drag,
            records_anchor: !brush.fill,
        });
    }
    if enqueued && let Some(doc) = state.documents.get_mut(&index) {
        doc.paint_busy = true;
        doc.paint_pending = true;
    }
}
impl Runtime {
    pub(super) fn layer_gpu_bytes(&self, model: usize, session: &Session, queue: &gpu::Queue) -> Vec<u64> {
        session.binding
            .and_then(|(object, slot)| self.lives.get(&(model, object, slot)))
            .map_or_else(Vec::new, |live| queue.layer_bytes(live.controller.context_id()))
    }

    pub(super) fn paint_gpu_bytes(&self, model: usize, queue: &gpu::Queue, images: &Assets<Image>) -> u64 {
        let lives = self.lives.iter().filter(|&(&(index, _, _), _)| index == model).map(|(_, live)| live);
        let mut textures = std::collections::HashSet::new();
        let image_bytes: u64 = lives.clone()
            .filter_map(|live| live.gpu.as_ref())
            .flat_map(|gpu| &gpu.images)
            .filter(|image| textures.insert(image.id()))
            .filter_map(|image| images.get(image))
            .map(|image| {
                let descriptor = &image.texture_descriptor;
                (0..descriptor.mip_level_count)
                    .filter_map(|level| descriptor.mip_level_size(level))
                    .map(|size| gpu::texture_bytes(size, descriptor.format))
                    .sum::<u64>() * u64::from(descriptor.sample_count)
            })
            .sum();
        image_bytes + queue.context_bytes(lives.map(|live| live.controller.context_id()))
    }

    pub(super) fn discard_mesh(&mut self, model: usize, queue: &mut gpu::Queue) {
        self.lives.retain(|(index, _, _), live| {
            if *index == model {
                live.controller.invalidate(queue);
            }
            *index != model
        });
        self.line_anchor = None;
        self.map_cache = None;
    }

    pub(in crate::editor::ui::model_viewer) fn stroke_active(&self) -> bool {
        self.gesture.is_some() || self.fix.drawing()
    }

    pub(super) fn trace_maps(
        &self,
        model: usize,
        object: usize,
        slot: usize,
    ) -> Option<([bevy::asset::AssetId<Image>; 5], u64)> {
        let gpu = self.lives.get(&(model, object, slot))?.gpu.as_ref()?;
        Some((
            [0, 1, 2, 3, composite::PREVIEW[7]].map(|index| gpu.images[index].id()),
            gpu.trace_revision,
        ))
    }

    pub(in crate::editor::ui::model_viewer) fn working_material(
        &self,
        model: usize,
        object: usize,
        slot: usize,
    ) -> Option<&Handle<StandardMaterial>> {
        self.lives.get(&(model, object, slot))?.gpu.as_ref().map(|gpu| &gpu.material)
    }

    fn finish_fill_selection(&mut self) -> bool {
        let Some(gesture) = self.gesture.take() else { return false };
        let Some(drag) = gesture.fill_drag else { return false };
        let Some(live) = self.lives.get_mut(&gesture.key) else { return false };
        live.queue.push_back(Event::Begin(drag.brush));
        if let Some(region) = drag.region() {
            live.queue.push_back(Event::Region(region, gesture.mapping));
        } else {
            live.enqueue_stamps(vec![Stamp::from(drag.start)], gesture.mapping);
        }
        live.queue.push_back(Event::End);
        true
    }

    fn end_stroke(&mut self) {
        if let Some(gesture) = self.gesture.take()
            && gesture.fill_drag.is_none()
            && let Some(live) = self.lives.get_mut(&gesture.key)
        {
            live.queue.push_back(Event::End);
            if gesture.records_anchor {
                self.line_anchor = Some(LineAnchor {
                    key: gesture.key,
                    uv: gesture.mapping.uv,
                    camera: gesture.mapping.camera,
                    point: gesture.path.end(),
                });
            }
        }
    }
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
    pub(in crate::editor::ui::model_viewer) fn preview(
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
            .map(|gpu| {
                if channel == data::MASK_TARGET {
                    gpu.images[composite::MASK_PREVIEW].clone()
                } else {
                    gpu.preview[channel].clone()
                }
            })
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
    canvases: Query<(Entity, Has<TextureCanvas>), Or<(With<Canvas>, With<TextureCanvas>)>>,
    texture: UvCanvas,
    controls: Query<Entity, With<Control>>,
    mut cursor: Query<(&mut Node, &mut Visibility), With<BrushCursor>>,
    pointer: Res<PointerFrame>,
    runtime: Res<Runtime>,
) {
    let Ok((mut node, mut visible)) = cursor.single_mut() else {
        return;
    };
    let placement = (|| {
        let Some(index) = editor
            .viewing_model
            .filter(|i| state.mode(Some(*i)) == Mode::Paint)
        else {
            return None;
        };
        let Some(session) = paint.sessions.get(&index) else {
            return None;
        };
        if session.paint_blocked_by_bake()
            || session.cage_brush_active() || session.custom_gizmo_active()
            || session.graph_canvas_open() || session.panel == Panel::Shader {
            return None;
        }
        if session.binding.is_none_or(|(object, slot)| !session.set_appearance.visibility.shown(object, slot)) {
            return None;
        }
        let adjustment = runtime
            .brush_adjustment
            .as_ref()
            .filter(|state| state.model == index);
        let Some(point) = adjustment
            .map(|state| state.origin)
            .or_else(|| pointer.pen_cursor())
            .or_else(|| window.cursor_position())
        else {
            return None;
        };
        if !window.focused
            || editor.pending.is_some()
            || editor.browser_popup
            || drawer.covers(point, &window)
            || (adjustment.is_none() && mouse.pressed(MouseButton::Right))
            || mouse.pressed(MouseButton::Middle)
            || pointer.pen_pan_handled
            || keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
        {
            return None;
        }
        let over = canvases.iter().any(|(e, uv)| {
            let visible = if uv {
                texture.viewport_rect().is_some_and(|r| r.contains(point))
            } else {
                session.view != View::Two
            };
            if !visible || (uv && session.view == View::Three) {
                return false;
            }
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
            return None;
        }
        let Some(overlay) = geometry
            .get(ui.overlay)
            .ok()
            .and_then(|(n, t)| node_rect(n, t))
        else {
            return None;
        };
        let gesture = runtime
            .gesture
            .as_ref()
            .filter(|g| g.mapping.camera.clip.contains(point));
        let size = if matches!(session.tool, PaintTool::Brush | PaintTool::Eraser) {
            session.size * gesture.map_or(1., |g| g.path.first().radius)
        } else {
            10.
        };
        let p = gesture.map_or(point, |g| g.path.lazy) - overlay.min - Vec2::splat(size * 0.5);
        Some((p, size))
    })();
    let Some((p, size)) = placement else {
        visible.set_if_neq(Visibility::Hidden);
        return;
    };
    if node.left != px(p.x) {
        node.left = px(p.x);
    }
    if node.top != px(p.y) {
        node.top = px(p.y);
    }
    if node.width != px(size) {
        node.width = px(size);
    }
    if node.height != px(size) {
        node.height = px(size);
    }
    visible.set_if_neq(Visibility::Visible);
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
                bake_fixes: Default::default(),
                authored: None,
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
            &sets::SetVisibility::default(),
        );
        let surface = Surface {
            size: 64,
            base: std::array::from_fn(|_| data::solid(64, [255; 4])),
            stack: Stack::default(),
            material: serde_json::json!({}),
            baked: None,
        };
        (
            Work {
                surface,
                preview: Default::default(),
                baked_preview: None,
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
            material: None,
            radius: 48.,
            hardness: 1.,
            flow: 1.,
            opacity: 1.,
            erase: false,
            fill: false,
            fill_mode: FillMode::default(),
            fill_angle: 30.,
            fill_padding: 0.,
            variation: stroke::Variation::default(),
            tip: None,
            grain: None,
            mirror: [0.; 4],
            projection: BrushProjection::Screen,
            paint_through: false,
            facing_limit: 85_f32.to_radians().cos(),
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
                Event::Points(vec![Stamp::from(Vec2::splat(256.)); 8], map),
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
                Event::Points(vec![Stamp::from(Vec2::new(128., 256.))], map.clone()),
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
                Event::Points(vec![Stamp::from(Vec2::new(128., 256.))], map),
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
