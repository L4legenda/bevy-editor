//! Bridges Bevy's resident paint assets to the traced viewport.
use super::{
    Environment, Frame, Lighting, MaterialMaps, Pipeline, Scene, Settings, Target, encode,
};
use crate::editor::{
    Editor,
    model_document::{Mode, Object},
};
use bevy::{
    asset::AssetId,
    light::EnvironmentMapLight,
    math::proj,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
    },
    ui::ComputedNode,
    window::RequestRedraw,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::super::super::{
    Canvas, ViewCamera, Viewer, ViewerUi, editing, environment, view_height, view_pose,
};
use super::super::{
    PaintState, View,
    engine::{Runtime, gpu},
    post::{Grade, Processor, RasterGrade},
};

const VIEW_SIZE: f32 = 960.0;
const MAP_SIZE: u32 = 1024;
const MAP_COUNT: usize = 5;
const CAMERA_SETTLE: Duration = Duration::from_millis(100);

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct CanvasImage;

pub(in crate::editor::ui::model_viewer) struct Plugin;

impl bevy::app::Plugin for Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Main>()
            .add_systems(
                Update,
                capture
                    .after(super::super::super::refresh)
                    .after(super::super::engine::input),
            )
            .add_systems(Update, sync_raster_grade.after(capture));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<Gpu>()
                .init_resource::<Extracted>()
                .add_systems(ExtractSchedule, extract)
                .add_systems(
                    Render,
                    dispatch
                        .after(RenderSystems::PrepareAssets)
                        .after(gpu::PaintRender)
                        .before(RenderSystems::Queue),
                );
        }
    }
}

type GeometryKey = (usize, u64, u64);

#[derive(Clone, Copy, PartialEq)]
struct Map {
    id: Option<AssetId<Image>>,
    factor: Vec4,
    mode: u32,
    flag: bool,
    cutoff: f32,
}

#[derive(Clone, PartialEq)]
struct Layer {
    key: (usize, usize),
    maps: [Map; MAP_COUNT],
    revision: u64,
    working: bool,
}

impl Layer {
    fn working(key: (usize, usize), ids: [AssetId<Image>; MAP_COUNT], revision: u64) -> Self {
        Self {
            key,
            maps: std::array::from_fn(|i| Map {
                id: Some(ids[i]),
                factor: Vec4::ONE,
                mode: i as u32,
                flag: false,
                cutoff: -1.0,
            }),
            revision,
            working: true,
        }
    }

    fn original(key: (usize, usize), material: &StandardMaterial) -> Self {
        let color = material.base_color.to_linear();
        let emission = material.emissive;
        let mut maps = [
            (
                material.base_color_texture.as_ref(),
                Vec4::new(color.red, color.green, color.blue, color.alpha),
            ),
            (
                material.metallic_roughness_texture.as_ref(),
                Vec4::new(1.0, material.perceptual_roughness, material.metallic, 1.0),
            ),
            (material.normal_map_texture.as_ref(), Vec4::ONE),
            (
                material.emissive_texture.as_ref(),
                Vec4::new(emission.red, emission.green, emission.blue, 1.0),
            ),
            (
                material.specular_texture.as_ref(),
                Vec4::splat(2.0 * material.reflectance.powi(2)),
            ),
        ]
        .map(|(image, factor)| Map {
            id: image.map(Handle::id),
            factor,
            mode: 4,
            flag: false,
            cutoff: -1.0,
        });
        maps[0].mode = 0;
        maps[0].flag = material.alpha_mode == AlphaMode::Opaque;
        if let AlphaMode::Mask(cutoff) = material.alpha_mode {
            maps[0].cutoff = cutoff;
        }
        maps[2].mode = 2;
        maps[2].flag = material.flip_normal_map_y;
        maps[4].mode = 5;
        Self {
            key,
            maps,
            revision: 0,
            working: false,
        }
    }
}

struct Geometry {
    key: GeometryKey,
    objects: Vec<Object>,
}

struct Request {
    serial: u64,
    output: AssetId<Image>,
    size: UVec2,
    geometry: Arc<Geometry>,
    layers: Vec<Layer>,
    hdr: (AssetId<Image>, Arc<Image>),
    camera: super::Camera,
    lighting: Lighting,
    grade: Grade,
    settings: Settings,
}

impl Request {
    fn same_frame(&self, other: &Self) -> bool {
        self.output == other.output
            && self.geometry.key == other.geometry.key
            && self.layers == other.layers
            && self.hdr.0 == other.hdr.0
            && self.camera == other.camera
            && self.lighting == other.lighting
            && self.grade == other.grade
            && self.settings == other.settings
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pause {
    Stroke,
    Camera,
}

#[derive(Clone, Default)]
enum Display {
    #[default]
    Off,
    Paused(Pause),
    Trace(Arc<Request>),
}

#[derive(Clone, Copy, PartialEq)]
struct ViewStamp {
    model: usize,
    pose: Transform,
    viewport: Vec2,
    orthographic: bool,
    fov: f32,
}

#[derive(Default)]
struct CameraMotion {
    previous: Option<ViewStamp>,
    settle_at: Duration,
}

impl CameraMotion {
    fn active(&mut self, next: ViewStamp, gesture: bool, now: Duration) -> bool {
        let previous = self.previous.replace(next);
        if previous.is_none_or(|previous| previous.model != next.model) {
            self.settle_at = Duration::ZERO;
        }
        if gesture
            || previous.is_some_and(|previous| previous.model == next.model && previous != next)
        {
            self.settle_at = now + CAMERA_SETTLE;
        }
        now < self.settle_at
    }
}

#[derive(Clone, Copy, Default)]
enum Report {
    #[default]
    Waiting,
    Shown(u64, u32),
    Unavailable(u64),
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer::paint) struct Main {
    display: Display,
    motion: CameraMotion,
    output: Option<(usize, UVec2, Handle<Image>)>,
    hdr: Option<(AssetId<Image>, Arc<Image>)>,
    geometry: Option<Arc<Geometry>>,
    serial: u64,
    report: Arc<Mutex<Report>>,
    #[cfg(test)]
    bypass_camera_pause: bool,
    #[cfg(test)]
    dispatches: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(feature = "mcp")]
impl Main {
    pub(in crate::editor::ui::model_viewer::paint) fn progress(
        &self,
        index: usize,
        doc: &crate::editor::model_document::Document,
        session: &super::super::Session,
    ) -> serde_json::Value {
        use serde_json::json;
        let settings = session.trace_settings;
        let mut reason = None;
        let (status, samples) = if !session.path_trace || doc.mode != Mode::Paint || session.diagnostic_3d() {
            ("off", None)
        } else if session.view == View::Two {
            reason = Some("uv_only");
            ("paused", None)
        } else {
            match &self.display {
                Display::Paused(pause) if self.motion.previous.is_some_and(|stamp| stamp.model == index) => {
                    reason = Some(match pause {
                        Pause::Stroke => "stroke",
                        Pause::Camera => "camera",
                    });
                    ("paused", None)
                }
                Display::Trace(request) if request.geometry.key == (index, doc.revision, session.set_appearance.visibility.revision)
                    && (request.settings.samples, request.settings.bounces, request.settings.ground)
                        == (settings.samples, settings.bounces, settings.ground) =>
                {
                    match *self.report.lock().unwrap() {
                        Report::Shown(serial, count) if serial == request.serial => (
                            if count >= settings.samples { "complete" } else { "rendering" },
                            Some(count.min(settings.samples)),
                        ),
                        Report::Unavailable(serial) if serial == request.serial => ("unavailable", None),
                        _ => ("preparing", None),
                    }
                }
                _ => ("preparing", None),
            }
        };
        json!({
            "status": status, "reason": reason, "samples": samples, "target_samples": settings.samples,
        })
    }
}

#[cfg(test)]
pub(in crate::editor::ui::model_viewer::paint) fn samples(world: &World) -> Option<u32> {
    let main = world.resource::<Main>();
    match *main.report.lock().unwrap() {
        Report::Shown(serial, count) if matches!(&main.display, Display::Trace(request) if request.serial == serial) => {
            Some(count)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(in crate::editor::ui::model_viewer::paint) fn dispatches(world: &World) -> u64 {
    world
        .resource::<Main>()
        .dispatches
        .load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Resource, Default)]
struct Extracted {
    display: Display,
    report: Arc<Mutex<Report>>,
    #[cfg(test)]
    dispatches: Arc<std::sync::atomic::AtomicU64>,
}

fn extract(main: Extract<Res<Main>>, mut extracted: ResMut<Extracted>) {
    extracted.display = main.display.clone();
    extracted.report = main.report.clone();
    #[cfg(test)]
    {
        extracted.dispatches = main.dispatches.clone();
    }
}

fn output_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

fn capture(
    editor: Res<Editor>,
    paint: Res<PaintState>,
    runtime: (Res<Runtime>, Res<Time<bevy::time::Real>>),
    editing: Res<editing::State>,
    viewer: Res<Viewer>,
    ui: Res<ViewerUi>,
    env: Res<environment::Settings>,
    cameras: Query<&EnvironmentMapLight, With<ViewCamera>>,
    key_light: Query<(&DirectionalLight, &Transform), With<environment::KeyLight>>,
    canvas: Query<&ComputedNode, With<Canvas>>,
    mut image_node: Single<&mut ImageNode, With<CanvasImage>>,
    mut images: ResMut<Assets<Image>>,
    materials: Res<Assets<StandardMaterial>>,
    mut main: ResMut<Main>,
    mut redraw: MessageWriter<RequestRedraw>,
    mut texts: Query<&mut Text>,
) {
    let (runtime, clock) = runtime;
    let mut pause = None;
    let request = (|| {
        let index = editor.viewing_model?;
        let doc = editing.documents.get(&index)?;
        let session = paint.sessions.get(&index)?;
        if !session.path_trace
            || session.view == View::Two
            || session.diagnostic_3d()
            || doc.mode != Mode::Paint
            || editor.pending.is_some()
            || editor.viewing_image.is_some()
            || !viewer.fitted
        {
            return None;
        }
        let rect = canvas.single().ok()?.size();
        if rect.min_element() < 2.0 {
            return None;
        }
        let pose = view_pose(&viewer);
        let moving = main.motion.active(
            ViewStamp {
                model: index,
                pose,
                viewport: rect,
                orthographic: viewer.orthographic,
                fov: viewer.fov(),
            },
            viewer.gesture.is_some(),
            clock.elapsed(),
        );
        #[cfg(test)]
        let moving = moving && !main.bypass_camera_pause;
        pause = if runtime.stroke_active() {
            Some(Pause::Stroke)
        } else if moving {
            Some(Pause::Camera)
        } else {
            None
        };
        if pause.is_some() {
            return None;
        }
        let size = (rect * (VIEW_SIZE / rect.max_element()).min(1.0))
            .round()
            .max(Vec2::ONE)
            .as_uvec2();
        let light = cameras.single().ok()?;
        let hdr = light.specular_map.id();
        if !main.hdr.as_ref().is_some_and(|(id, _)| *id == hdr) {
            main.hdr = Some((hdr, Arc::new(images.get(hdr)?.clone())));
        }
        let visibility = &session.set_appearance.visibility;
        let key = (index, doc.revision, visibility.revision);
        if !main
            .geometry
            .as_ref()
            .is_some_and(|geometry| geometry.key == key)
        {
            main.geometry = Some(Arc::new(Geometry {
                key,
                objects: doc.objects.clone(),
            }));
        }
        let layers: Vec<_> = doc
            .objects
            .iter()
            .filter(|object| !object.hidden)
            .flat_map(|object| {
                let materials = &materials;
                let runtime = &runtime;
                let fallback = StandardMaterial::default();
                object
                    .materials
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, material)| {
                        let key = (object.id, slot);
                        visibility.shown(key.0, key.1).then(|| {
                            runtime
                                .trace_maps(index, key.0, key.1)
                                .map(|(maps, revision)| Layer::working(key, maps, revision))
                                .unwrap_or_else(|| {
                                    Layer::original(
                                        key,
                                        materials.get(&material.material).unwrap_or(&fallback),
                                    )
                                })
                        })
                    })
            })
            .collect();
        if layers.is_empty() {
            return None;
        }
        if !main
            .output
            .as_ref()
            .is_some_and(|(old_index, old_size, _)| *old_index == index && *old_size == size)
        {
            main.output = Some((index, size, images.add(output_image(size))));
            main.serial += 1;
        }
        let height = view_height(&viewer, viewer.distance);
        let aspect = rect.x / rect.y;
        let far = (viewer.distance + viewer.radius * 100.0).max(100.0);
        let projection = if viewer.orthographic {
            let y = height * 0.5;
            proj::orthographic(-y * aspect, y * aspect, -y, y, -far, far)
        } else {
            proj::perspective(
                viewer.fov(),
                aspect,
                (viewer.radius * 0.001).max(0.000001),
                far,
            )
        };
        let view = pose.to_matrix().inverse();
        let (lighting, _, curve) = env.trace_light(key_light.single().ok());
        let post = env.paint_post(Some(doc));
        let mut request = Request {
            serial: main.serial,
            output: main.output.as_ref()?.2.id(),
            size,
            geometry: main.geometry.clone()?,
            layers,
            hdr: main.hdr.clone()?,
            camera: super::Camera {
                inverse_view_projection: (projection * view).inverse(),
                eye: pose.translation,
                orthographic: viewer.orthographic,
            },
            lighting,
            grade: Grade {
                exposure: post.exposure,
                curve,
                effects: post.effects,
            },
            settings: super::Settings {
                emission_strength: doc.paint_preview.emission_strength,
                ..session.trace_settings
            },
        };
        if let Display::Trace(previous) = &main.display
            && previous.serial == request.serial
            && !previous.same_frame(&request)
        {
            main.serial = main.serial.wrapping_add(1);
            request.serial = main.serial;
        }
        Some(Arc::new(request))
    })();
    let next = if let Some(request) = request {
        Display::Trace(request)
    } else if let Some(pause) = pause {
        if !matches!(main.display, Display::Paused(_)) {
            // Keep the GPU resources, but hide the previous image until the
            // current camera has a sample.
            main.serial += 1;
        }
        Display::Paused(pause)
    } else {
        if !matches!(main.display, Display::Off) {
            main.serial += 1;
        }
        main.motion = CameraMotion::default();
        Display::Off
    };
    let mut image = &ui.image;
    let message = match &next {
        Display::Off => None,
        Display::Paused(Pause::Stroke) => Some("Трассировка обновится после мазка".into()),
        Display::Paused(Pause::Camera) => {
            // A wheel event or an orientation snap can be the last input
            // before the settle timeout, even in reactive window mode.
            redraw.write(RequestRedraw);
            Some("Трассировка обновится после движения камеры".into())
        }
        Display::Trace(request) => {
            let limit = request.settings.samples;
            Some(match *main.report.lock().unwrap() {
                Report::Shown(serial, samples) if serial == request.serial => {
                    image = &main.output.as_ref().unwrap().2;
                    if samples < limit {
                        redraw.write(RequestRedraw);
                    }
                    format!("Трассировка · {}/{limit} отсчётов", samples.min(limit))
                }
                Report::Unavailable(serial) if serial == request.serial => {
                    "Трассировка недоступна для этой модели или видеокарты".into()
                }
                _ => {
                    redraw.write(RequestRedraw);
                    "Подготовка трассировки…".into()
                }
            })
        }
    };
    if let Some(message) = message
        && let Ok(mut label) = texts.get_mut(ui.hint)
    {
        label.0 = message;
    }
    if image_node.image != *image {
        image_node.image = image.clone();
    }
    main.display = next;
}

fn sync_raster_grade(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    ui: Res<ViewerUi>,
    env: Res<environment::Settings>,
    canvas: Single<&ImageNode, With<CanvasImage>>,
    mut cameras: Query<
        (
            Entity,
            &mut bevy::render::view::Tonemapping,
            Option<&mut bevy::render::view::DebandDither>,
            Option<&RasterGrade>,
        ),
        With<ViewCamera>,
    >,
) {
    use bevy::render::view::{DebandDither, Tonemapping};
    let raster = editor.viewing_model.is_some_and(|index| {
        editing.mode(Some(index)) == Mode::Paint
            && paint
                .sessions
                .get(&index)
                .is_some_and(|session| !session.diagnostic_3d() && session.view != View::Two)
    }) && canvas.image == ui.image;
    let doc = editor.viewing_model.and_then(|index| editing.documents.get(&index));
    let post = env.paint_post(doc);
    let (_, camera_exposure, curve) = env.trace_light(None);
    let exposure = post.exposure - camera_exposure;
    for (entity, mut tone, dithering, previous) in &mut cameras {
        if raster && (post.effects.enabled() || exposure != 0.) {
            let original_dither =
                previous.map_or_else(|| dithering.as_deref().copied(), |grade| grade.1);
            let grade = RasterGrade(
                Grade {
                    // Raster PBR already receives exposure from the model camera.
                    exposure,
                    curve,
                    effects: post.effects,
                },
                original_dither,
            );
            tone.set_if_neq(Tonemapping::None);
            if let Some(mut dithering) = dithering {
                dithering.set_if_neq(DebandDither::Disabled);
            }
            if previous != Some(&grade) {
                commands.entity(entity).insert(grade);
            }
        } else if let Some(previous) = previous {
            tone.set_if_neq(env.tone_mapper());
            if let (Some(mut dithering), Some(original)) = (dithering, previous.1) {
                dithering.set_if_neq(original);
            }
            commands.entity(entity).remove::<RasterGrade>();
        }
    }
}

struct Programs {
    trace: Pipeline,
    pack: ComputePipeline,
    pack_layout: BindGroupLayout,
    sampler: Sampler,
    white: TextureView,
}

impl Programs {
    fn new(device: &RenderDevice, queue: &RenderQueue) -> Option<Self> {
        let trace = Pipeline::new(device)?;
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("Paint path trace display"),
            source: ShaderSource::Wgsl(include_str!("model_paint_pathtrace_display.wgsl").into()),
        });
        let kernel = |entry| {
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: default(),
                cache: None,
            })
        };
        let pack = kernel("pack_map");
        Some(Self {
            trace,
            pack_layout: BindGroupLayout::from(pack.get_bind_group_layout(0)),
            pack,
            sampler: device.create_sampler(&SamplerDescriptor {
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }),
            white: device
                .create_texture_with_data(
                    queue,
                    &TextureDescriptor {
                        label: Some("Paint trace default input"),
                        size: Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format: TextureFormat::Rgba8Unorm,
                        usage: TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    TextureDataOrder::LayerMajor,
                    &[255; 4],
                )
                .create_view(&default()),
        })
    }
}

#[derive(PartialEq)]
struct MapStamp {
    layer: Layer,
    resident: [Option<TextureId>; MAP_COUNT],
    stroke: Option<(u64, u64)>,
}

struct Arrays {
    size: u32,
    textures: [Texture; MAP_COUNT],
    views: [TextureView; MAP_COUNT],
    stamp: Vec<Option<MapStamp>>,
}

impl Arrays {
    fn size(device: &RenderDevice, count: usize, largest: u32) -> Option<u32> {
        let limits = device.limits();
        let layers = u32::try_from(count).ok()?;
        if layers == 0 || layers > limits.max_texture_array_layers {
            return None;
        }
        // Preserve the visible set count while bounding the aggregate array size.
        let budget =
            ((160.0 * 1024.0 * 1024.0) / (8.0 * MAP_COUNT as f32 * layers as f32)).sqrt() as u32;
        let size = largest
            .min(MAP_SIZE)
            .min(budget)
            .min(limits.max_texture_dimension_2d);
        (size > 0).then_some(size)
    }

    fn new(device: &RenderDevice, count: usize, size: u32) -> Self {
        let layers = count as u32;
        let textures = std::array::from_fn(|_| {
            device.create_texture(&TextureDescriptor {
                label: Some("Path trace texture-set array"),
                size: Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            })
        });
        let views = textures.each_ref().map(|texture| {
            texture.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..default()
            })
        });
        Self {
            size,
            textures,
            views,
            stamp: (0..count).map(|_| None).collect(),
        }
    }

    fn pack(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        programs: &Programs,
        layers: &[Layer],
        images: &RenderAssets<GpuImage>,
        paint: &gpu::State,
    ) -> bool {
        let mut encoder = None;
        for (index, (layer, stamp)) in layers.iter().zip(&mut self.stamp).enumerate() {
            let inputs = layer.maps.map(|map| map.id.and_then(|id| images.get(id)));
            let current = MapStamp {
                resident: inputs.map(|input| input.map(|image| image.texture.id())),
                stroke: layer
                    .working
                    .then(|| layer.maps[0].id.and_then(|id| paint.trace_revision(id)))
                    .flatten(),
                layer: layer.clone(),
            };
            if stamp.as_ref() == Some(&current) {
                continue;
            }
            *stamp = Some(current);
            let encoder = encoder.get_or_insert_with(|| device.create_command_encoder(&default()));
            for ((map, input), texture) in layer.maps.iter().zip(inputs).zip(&self.textures) {
                let view = texture.create_view(&TextureViewDescriptor {
                    dimension: Some(TextureViewDimension::D2),
                    base_array_layer: index as u32,
                    array_layer_count: Some(1),
                    ..default()
                });
                let values = map.factor.to_array().into_iter().chain([
                    map.mode as f32,
                    u32::from(map.flag) as f32,
                    map.cutoff,
                    u32::from(input.is_some()) as f32,
                ]);
                let uniform = uniform(device, values);
                let group = device.create_bind_group(
                    "Pack paint trace layer",
                    &programs.pack_layout,
                    &[
                        entry(0, uniform.as_entire_binding()),
                        entry(
                            1,
                            BindingResource::TextureView(
                                input.map_or(&programs.white, |image| &image.texture_view),
                            ),
                        ),
                        entry(2, BindingResource::Sampler(&programs.sampler)),
                        entry(3, BindingResource::TextureView(&view)),
                    ],
                );
                run(encoder, &programs.pack, &group, UVec2::splat(self.size));
            }
        }
        if let Some(encoder) = encoder {
            queue.submit([encoder.finish()]);
            true
        } else {
            false
        }
    }
}

fn entry(binding: u32, resource: BindingResource<'_>) -> BindGroupEntry<'_> {
    BindGroupEntry { binding, resource }
}

fn uniform(device: &RenderDevice, values: impl IntoIterator<Item = f32>) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("Path trace viewport values"),
        contents: &encode(values),
        usage: BufferUsages::UNIFORM,
    })
}

fn run(encoder: &mut CommandEncoder, pipeline: &ComputePipeline, group: &BindGroup, size: UVec2) {
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(size.x.div_ceil(8), size.y.div_ceil(8), 1);
}

#[derive(Resource, Default)]
struct Gpu {
    programs: Option<Programs>,
    scene: Option<(GeometryKey, Scene)>,
    environment: Option<(AssetId<Image>, TextureId, Environment)>,
    arrays: Option<Arrays>,
    target: Option<(AssetId<Image>, Target)>,
    grade: Option<Grade>,
    revision: u64,
}

fn dispatch(
    request: Res<Extracted>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    assets: Res<RenderAssets<GpuImage>>,
    paint: Res<gpu::State>,
    mut gpu: ResMut<Gpu>,
    mut post: ResMut<Processor>,
) {
    let requested = match &request.display {
        Display::Trace(request) => request,
        Display::Paused(_) => return,
        Display::Off => {
            gpu.scene = None;
            gpu.arrays = None;
            gpu.target = None;
            gpu.grade = None;
            return;
        }
    };
    let report = |state| *request.report.lock().unwrap() = state;
    let previous = *request.report.lock().unwrap();
    if matches!(previous, Report::Unavailable(serial) if serial == requested.serial) {
        return;
    }
    let Some(hdr) = assets.get(requested.hdr.0) else {
        return;
    };
    let Some(output) = assets.get(requested.output) else {
        return;
    };
    if requested
        .layers
        .iter()
        .flat_map(|layer| layer.maps.iter())
        .any(|map| map.id.is_some_and(|id| assets.get(id).is_none()))
    {
        return;
    }
    let Gpu {
        programs,
        scene,
        environment,
        arrays,
        target,
        grade,
        revision,
    } = &mut *gpu;
    if programs.is_none() {
        *programs = Programs::new(&device, &queue);
    }
    let Some(programs) = programs else {
        report(Report::Unavailable(requested.serial));
        return;
    };
    if !scene
        .as_ref()
        .is_some_and(|(key, _)| *key == requested.geometry.key)
    {
        let layers: std::collections::HashMap<_, _> = requested
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| (layer.key, index as u32))
            .collect();
        *scene = Scene::new(&device, &requested.geometry.objects, |object, slot| {
            layers.get(&(object, slot)).copied()
        })
        .map(|scene| (requested.geometry.key, scene));
        *revision += 1;
    }
    if !environment
        .as_ref()
        .is_some_and(|(id, texture, _)| *id == requested.hdr.0 && *texture == hdr.texture.id())
    {
        *environment =
            Environment::from_cubemap(&device, &requested.hdr.1, hdr.texture_view.clone())
                .map(|env| (requested.hdr.0, hdr.texture.id(), env));
        *revision += 1;
    }
    let largest = requested
        .layers
        .iter()
        .flat_map(|layer| layer.maps.iter())
        .filter_map(|map| assets.get(map.id?))
        .map(|gpu| {
            gpu.texture_descriptor
                .size
                .width
                .max(gpu.texture_descriptor.size.height)
        })
        .max()
        .unwrap_or(1);
    let map_size = Arrays::size(&device, requested.layers.len(), largest);
    if !arrays.as_ref().is_some_and(|arrays| {
        arrays.stamp.len() == requested.layers.len() && Some(arrays.size) == map_size
    }) {
        *arrays = map_size.map(|size| Arrays::new(&device, requested.layers.len(), size));
    }
    if !target
        .as_ref()
        .is_some_and(|(output, _)| *output == requested.output)
    {
        *target = Target::new(&device, requested.size).map(|target| (requested.output, target));
    }
    let (Some((_, scene)), Some((_, _, environment)), Some(arrays), Some((_, target))) = (
        scene.as_ref(),
        environment.as_ref(),
        arrays.as_mut(),
        target.as_mut(),
    ) else {
        report(Report::Unavailable(requested.serial));
        return;
    };
    if arrays.pack(
        &device,
        &queue,
        programs,
        &requested.layers,
        &assets,
        &paint,
    ) {
        *revision += 1;
    }
    let [color, rm, normal, emission, specular] = &arrays.views;
    let frame = Frame {
        scene,
        maps: MaterialMaps {
            color,
            roughness: rm,
            metallic: rm,
            baked_normal: normal,
            height: rm,
            emission,
            normal,
            specular,
            resolution: arrays.size,
        },
        environment,
        camera: requested.camera,
        lighting: requested.lighting,
        settings: requested.settings,
        revision: *revision,
    };
    let traced = target.step(&device, &queue, &programs.trace, &frame);
    #[cfg(test)]
    if traced {
        request
            .dispatches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if traced || *grade != Some(requested.grade) {
        show(
            &device,
            &queue,
            &mut post,
            &target.view,
            &output.texture,
            requested.size,
            requested.grade,
        );
        *grade = Some(requested.grade);
    }
    if traced || !matches!(previous, Report::Shown(serial, _) if serial == requested.serial) {
        report(Report::Shown(requested.serial, target.samples()));
    }
}

fn show(
    device: &RenderDevice,
    queue: &RenderQueue,
    post: &mut Processor,
    source: &TextureView,
    output: &Texture,
    size: UVec2,
    grade: Grade,
) {
    let display = output.create_view(&default());
    let mut encoder = device.create_command_encoder(&default());
    post.record(device, &mut encoder, source, &display, size, grade, true);
    queue.submit([encoder.finish()]);
}

#[cfg(test)]
#[path = "model_paint_pathtrace_bench.rs"]
mod benchmark;
