//! Resident GPU stroke masks, layer buffers and compositing on Bevy's own device.
//! Layer snapshots and picker samples are read back asynchronously.
#[cfg(test)]
#[path = "model_paint_gpu_perf_tests.rs"]
mod perf_tests;
use super::*;
use bevy::{
    asset::AssetId,
    render::{
        ExtractSchedule, MainWorld, Render, RenderApp, RenderStartup, RenderSystems,
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Default)]
pub(super) struct Statistics {
    pub batches: AtomicU64,
    pub stamps: AtomicU64,
    pub readbacks: AtomicU64,
    contexts: Mutex<HashMap<u64, ContextMemory>>,
}

struct ContextMemory {
    total: u64,
    layers: Vec<u64>,
}
#[derive(Clone)]
pub(super) struct Scene {
    pub vertices: Vec<u8>,
    pub count: u32,
}
impl Scene {
    pub fn prepare(objects: &[Object], visibility: &sets::SetVisibility) -> Self {
        let mut vertices = Vec::new();
        let mut count = 0;
        for object in objects.iter().filter(|o| !o.hidden) {
            let scale = object.transform.scale;
            let inv_scale = scale.signum() / scale.abs().max(Vec3::splat(1e-6));
            for (face, f) in object.geometry.faces.iter().enumerate() {
                if !visibility.shown(object.id, f.material) {
                    continue;
                }
                let face_normal = object.geometry.normal(f);
                let uv_offset = object.uv_offset(f.material);
                for t in object.geometry.triangles(f) {
                    for c in t {
                        let corner = &f.corners[c];
                        let p = object
                            .transform
                            .transform_point(object.geometry.positions[corner.vertex]);
                        let uv = corner.uv - uv_offset;
                        for v in [p.x, p.y, p.z, uv.x, uv.y] {
                            vertices.extend_from_slice(&v.to_le_bytes());
                        }
                        for v in [face as u32 + 1, object.id as u32 + 1, f.material as u32] {
                            vertices.extend_from_slice(&v.to_le_bytes());
                        }
                        let normal = object.transform.rotation
                            * (corner.source_normal.unwrap_or(face_normal) * inv_scale)
                                .normalize_or_zero();
                        for axis in normal.to_array() {
                            vertices.extend_from_slice(&axis.to_le_bytes());
                        }
                        count += 1;
                    }
                }
            }
        }
        Self { vertices, count }
    }
}
#[derive(Clone, Copy, PartialEq)]
pub(super) struct View {
    pub camera: CameraMap,
    pub uv: bool,
}
pub(super) enum Command {
    Setup {
        id: u64,
        surface: Arc<Surface>,
        channel: usize,
        channels: u16,
        images: [AssetId<Image>; composite::IMAGE_COUNT],
        scene: Arc<Scene>,
        object: u32,
        slot: u32,
        relief_factor: f32,
        baked_preview: Option<Arc<composite::PreviewBake>>,
        fix: Option<(Vec<u8>, bool)>,
    },
    Begin {
        id: u64,
        stroke: u64,
        brush: Brush,
        view: View,
        trace: Option<Arc<surface::SurfaceTrace>>,
    },
    Points {
        id: u64,
        points: Vec<Stamp>,
        ranges: Option<Vec<std::ops::Range<u32>>>,
    },
    End {
        id: u64,
        stroke: u64,
    },
    Pick {
        id: u64,
        images: [AssetId<Image>; 2],
        uv: Vec2,
    },
    CancelFix(u64),
    Drop(u64),
}
pub(super) enum Reply {
    Layer {
        id: u64,
        stroke: u64,
        pixels: Result<Vec<Vec<u8>>, String>,
    },
    Pick {
        id: u64,
        sample: Result<super::picker::MaterialSample, String>,
    },
}
#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct Queue {
    pub(super) commands: Vec<Command>,
    pub(super) replies: Arc<Mutex<Vec<Reply>>>,
    pub(super) stats: Arc<Statistics>,
    next: u64,
}
impl Queue {
    pub fn context_bytes(&self, ids: impl Iterator<Item = u64>) -> u64 {
        let contexts = self.stats.contexts.lock().unwrap();
        ids.filter_map(|id| contexts.get(&id)).map(|memory| memory.total).sum()
    }

    pub fn layer_bytes(&self, id: u64) -> Vec<u64> {
        self.stats.contexts.lock().unwrap().get(&id)
            .map_or_else(Vec::new, |memory| memory.layers.clone())
    }

    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer) fn statistics(&self) -> (u64, u64, u64) {
        (
            self.stats.batches.load(Ordering::Relaxed),
            self.stats.stamps.load(Ordering::Relaxed),
            self.stats.readbacks.load(Ordering::Relaxed),
        )
    }
    pub fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}
#[derive(Resource, Default)]
struct Pending {
    commands: std::collections::VecDeque<Command>,
    replies: Arc<Mutex<Vec<Reply>>>,
    stats: Arc<Statistics>,
}
#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer::paint) struct State {
    pipelines: Option<Pipelines>,
    contexts: HashMap<u64, Context>,
}
impl State {
    pub(in crate::editor::ui::model_viewer::paint) fn trace_revision(&self, target: AssetId<Image>) -> Option<(u64, u64)> {
        self.contexts.iter().find_map(|(&id, context)| {
            (context.images[0] == target).then_some((id, context.trace_revision))
        })
    }
}
pub(in crate::editor::ui::model_viewer) struct PluginGpu;
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::editor::ui::model_viewer::paint) struct PaintRender;
impl Plugin for PluginGpu {
    fn build(&self, app: &mut App) {
        app.init_resource::<Queue>();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<Pending>()
                .init_resource::<State>()
                .add_systems(RenderStartup, initialize)
                .add_systems(ExtractSchedule, extract)
                .add_systems(
                    Render,
                    execute
                        .in_set(PaintRender)
                        .after(RenderSystems::PrepareAssets)
                        .after(crate::editor::image_texture::upload)
                        .before(RenderSystems::Queue),
                );
        }
    }
}
fn initialize(mut state: ResMut<State>, device: Res<RenderDevice>) {
    state.pipelines = Some(Pipelines::new(&device));
}
fn extract(mut world: ResMut<MainWorld>, mut pending: ResMut<Pending>) {
    let mut queue = world.resource_mut::<Queue>();
    for command in std::mem::take(&mut queue.commands) {
        if let Command::Drop(id) = command {
            pending.commands.retain(|c| command_id(c) != id);
            pending.commands.push_back(Command::Drop(id));
        } else {
            pending.commands.push_back(command);
        }
    }
    pending.replies = queue.replies.clone();
    pending.stats = queue.stats.clone();
}
fn command_id(command: &Command) -> u64 {
    match command {
        Command::Setup { id, .. }
        | Command::Begin { id, .. }
        | Command::Points { id, .. }
        | Command::End { id, .. }
        | Command::Pick { id, .. } => *id,
        Command::CancelFix(id) | Command::Drop(id) => *id,
    }
}
struct Target {
    texture: Texture,
    view: TextureView,
}
impl Target {
    fn bytes(&self) -> u64 {
        texture_bytes(self.texture.size(), self.texture.format())
    }
}

pub(super) fn texture_bytes(size: Extent3d, format: TextureFormat) -> u64 {
    let (width, height) = format.block_dimensions();
    u64::from(size.width.div_ceil(width))
        * u64::from(size.height.div_ceil(height))
        * u64::from(size.depth_or_array_layers)
        * u64::from(format.block_copy_size(None).unwrap_or(0))
}

#[cfg(test)]
#[test]
fn paint_texture_memory_counts_depth_and_partial_blocks() {
    let size = Extent3d { width: 7, height: 5, depth_or_array_layers: 2 };
    assert_eq!(texture_bytes(size, TextureFormat::Depth32Float), 280);
    assert_eq!(texture_bytes(size, TextureFormat::Bc3RgbaUnorm), 128);
}
fn target(
    device: &RenderDevice,
    label: &'static str,
    w: u32,
    h: u32,
    format: TextureFormat,
    usage: TextureUsages,
) -> Target {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    Target { texture, view }
}
fn buffer(device: &RenderDevice, label: &'static str, bytes: &[u8], usage: BufferUsages) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage,
    })
}
fn empty(device: &RenderDevice, label: &'static str, size: u64, usage: BufferUsages) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}
fn storage(binding: u32, read_only: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE | ShaderStages::FRAGMENT,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn uniform(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT | ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn texture(binding: u32, sample_type: TextureSampleType) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT | ShaderStages::COMPUTE,
        ty: BindingType::Texture {
            sample_type,
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn output(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::StorageTexture {
            access: StorageTextureAccess::WriteOnly,
            format: TextureFormat::Rgba8Unorm,
            view_dimension: TextureViewDimension::D2,
        },
        count: None,
    }
}
fn shader(device: &RenderDevice, label: &'static str, source: &'static str) -> ShaderModule {
    device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some(label),
        source: ShaderSource::Wgsl(source.into()),
    })
}
struct Pipelines {
    scene_layout: BindGroupLayout,
    mask_layout: BindGroupLayout,
    pad_layout: BindGroupLayout,
    grain_layout: BindGroupLayout,
    apply_layout: BindGroupLayout,
    mask_filter_layout: BindGroupLayout,
    blend_layout: BindGroupLayout,
    present_layout: BindGroupLayout,
    tiles_layout: BindGroupLayout,
    depth: RenderPipeline,
    mask: RenderPipeline,
    occupancy: RenderPipeline,
    padding: RenderPipeline,
    grain: RenderPipeline,
    apply: ComputePipeline,
    mask_filter: ComputePipeline,
    blend: ComputePipeline,
    present: ComputePipeline,
    present_full: ComputePipeline,
    tiles: ComputePipeline,
    tile_bounds: ComputePipeline,
    tip_sampler: Sampler,
    grain_sampler: Sampler,
}
impl Pipelines {
    fn new(device: &RenderDevice) -> Self {
        let scene_layout = device.create_bind_group_layout("paint view", &[uniform(0)]);
        let pad_layout = device.create_bind_group_layout(
            "paint UV padding",
            &[uniform(0), texture(4, TextureSampleType::Uint), storage(5, false)],
        );
        let mask_layout = device.create_bind_group_layout(
            "paint stamps",
            &[
                uniform(0),
                storage(1, true),
                texture(2, TextureSampleType::Depth),
                texture(3, TextureSampleType::Uint),
                storage(5, false),
                texture(6, TextureSampleType::Float { filterable: true }),
                BindGroupLayoutEntry {
                    binding: 7,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let grain_layout = device.create_bind_group_layout(
            "paint grain",
            &[
                uniform(0),
                texture(8, TextureSampleType::Float { filterable: true }),
                BindGroupLayoutEntry {
                    binding: 9,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let apply_layout = device.create_bind_group_layout(
            "paint apply",
            &[
                uniform(0),
                texture(1, TextureSampleType::Float { filterable: false }),
                storage(2, true),
                storage(3, false),
                storage(4, true),
                texture(5, TextureSampleType::Float { filterable: false }),
            ],
        );
        let mask_filter_layout = device.create_bind_group_layout(
            "paint mask filters",
            &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, true),
            ],
        );
        let blend_layout = device.create_bind_group_layout(
            "paint blend",
            &[
                uniform(0),
                storage(1, true),
                storage(2, false),
                storage(3, true),
                storage(4, true),
                storage(5, true),
            ],
        );
        let present_layout = device.create_bind_group_layout(
            "paint present",
            &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                texture(3, TextureSampleType::Float { filterable: false }),
                texture(4, TextureSampleType::Float { filterable: false }),
                output(5),
                output(6),
                storage(7, true),
                storage(8, true),
            ],
        );
        let tiles_layout = device.create_bind_group_layout(
            "paint dirty tiles",
            &[uniform(0), storage(1, true), storage(2, false)],
        );
        let mask_shader = shader(
            device,
            "paint stamps",
            include_str!("model_paint_gpu_mask.wgsl"),
        );
        let padding_layout = [Some(RawVertexBufferLayout {
            array_stride: 8,
            step_mode: VertexStepMode::Vertex,
            attributes: &[VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            }],
        })];
        let make_render =
            |label, layout: &BindGroupLayout, vs, fs, format, blend, depth_stencil| {
                let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[Some(layout)],
                    immediate_size: 0,
                });
                device.create_render_pipeline(&RawRenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: RawVertexState {
                        module: &mask_shader,
                        entry_point: Some(vs),
                        compilation_options: Default::default(),
                        buffers: if vs == "vs_padding" { &padding_layout } else { &[Some(RawVertexBufferLayout {
                            array_stride: 44,
                            step_mode: VertexStepMode::Vertex,
                            attributes: &[
                                VertexAttribute {
                                    format: VertexFormat::Float32x3,
                                    offset: 0,
                                    shader_location: 0,
                                },
                                VertexAttribute {
                                    format: VertexFormat::Float32x2,
                                    offset: 12,
                                    shader_location: 1,
                                },
                                VertexAttribute {
                                    format: VertexFormat::Uint32,
                                    offset: 20,
                                    shader_location: 2,
                                },
                                VertexAttribute {
                                    format: VertexFormat::Uint32,
                                    offset: 24,
                                    shader_location: 3,
                                },
                                VertexAttribute {
                                    format: VertexFormat::Uint32,
                                    offset: 28,
                                    shader_location: 4,
                                },
                                VertexAttribute {
                                    format: VertexFormat::Float32x3,
                                    offset: 32,
                                    shader_location: 5,
                                },
                            ],
                        })] },
                    },
                    fragment: Some(RawFragmentState {
                        module: &mask_shader,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets: &[Some(ColorTargetState {
                            format,
                            blend,
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    primitive: PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let depth = make_render(
            "paint depth",
            &scene_layout,
            "vs_scene",
            "fs_scene",
            TextureFormat::Rg32Uint,
            None,
            Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let over = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        };
        let occupancy = make_render(
            "paint UV occupancy",
            &scene_layout,
            "vs_uv",
            "fs_occupied",
            TextureFormat::R32Uint,
            None,
            None,
        );
        let padding = make_render(
            "paint UV padding",
            &pad_layout,
            "vs_padding",
            "fs_padding",
            TextureFormat::Rgba16Float,
            Some(BlendState { color: over, alpha: over }),
            None,
        );
        let mask = make_render(
            "paint coverage",
            &mask_layout,
            "vs_uv",
            "fs_mask",
            TextureFormat::Rgba16Float,
            Some(BlendState {
                color: over,
                alpha: over,
            }),
            None,
        );
        let grain = make_render(
            "paint grain",
            &grain_layout,
            "vs_uv",
            "fs_grain",
            TextureFormat::R16Float,
            None,
            None,
        );
        let make_compute = |label, layout: &BindGroupLayout, source, entry| {
            let module = shader(device, label, source);
            let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let apply = make_compute(
            "paint apply",
            &apply_layout,
            include_str!("model_paint_gpu_apply.wgsl"),
            "apply",
        );
        let blend = make_compute(
            "paint blend",
            &blend_layout,
            include_str!("model_paint_gpu_blend.wgsl"),
            "blend",
        );
        let present = make_compute(
            "paint present",
            &present_layout,
            include_str!("model_paint_gpu_present.wgsl"),
            "present",
        );
        let mask_filter = make_compute(
            "paint mask filters",
            &mask_filter_layout,
            include_str!("model_paint_gpu_mask_filter.wgsl"),
            "filter_mask",
        );
        let tiles = make_compute(
            "paint dirty tiles",
            &tiles_layout,
            include_str!("model_paint_gpu_tiles.wgsl"),
            "compact",
        );
        let tile_bounds = make_compute(
            "paint dirty bounds",
            &tiles_layout,
            include_str!("model_paint_gpu_tiles.wgsl"),
            "bounds",
        );
        let present_full = make_compute(
            "initialize paint display",
            &present_layout,
            include_str!("model_paint_gpu_present.wgsl"),
            "present_full",
        );
        Self {
            scene_layout,
            mask_layout,
            pad_layout,
            grain_layout,
            apply_layout,
            mask_filter_layout,
            blend_layout,
            present_layout,
            tiles_layout,
            depth,
            mask,
            occupancy,
            padding,
            grain,
            apply,
            mask_filter,
            blend,
            present,
            present_full,
            tiles,
            tile_bounds,
            tip_sampler: device.create_sampler(&SamplerDescriptor {
                label: Some("paint brush tip"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }),
            grain_sampler: device.create_sampler(&SamplerDescriptor {
                label: Some("paint grain repeats"),
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::Repeat,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }),
        }
    }
}
struct LayerBuffer {
    buffer: Buffer,
    mask: Option<Buffer>,
    opacity: f32,
    blend: data::Blend,
}
struct Composition {
    base: Buffer,
    result: Buffer,
    below: Buffer,
    layers: Vec<LayerBuffer>,
    ops: Vec<(usize, data::CompositeOp)>,
    saved: Vec<Buffer>,
    cached_layers: usize,
}
struct StrokeChannel {
    channel: usize,
    composition: Composition,
    snapshot: Buffer,
}
struct Context {
    trace_revision: u64,
    size: u32,
    channel: usize,
    selected: usize,
    images: [AssetId<Image>; composite::IMAGE_COUNT],
    object: u32,
    slot: u32,
    vertices: Buffer,
    count: u32,
    mask: Target,
    grain: Option<(BrushGrain, Target)>,
    depth: Target,
    scene_ids: Target,
    uv_occupancy: Option<Target>,
    outputs: [Target; 2],
    composition: Composition,
    additional: Vec<StrokeChannel>,
    auxiliary: Buffer,
    baked_normal: Buffer,
    relief_factor: f32,
    mask_compositions: Vec<Composition>,
    mask_filter: Option<MaskFilters>,
    mask_affine: bool,
    fix_normal: Option<bool>,
    snapshot: Buffer,
    params: Option<(Brush, View)>,
    trace: Option<Arc<surface::SurfaceTrace>>,
    tip_motion: (Option<Vec2>, f32),
    stamp_noise: stroke::Noise,
    dirty: Buffer,
    tiles: Buffer,
    grid: Buffer,
    presented: bool,
}

struct MaskFilters {
    raw: Buffer,
    passes: Vec<BindGroup>,
    bounds: Option<Buffer>,
    pass_bytes: u64,
}

impl MaskFilters {
    fn new(
        device: &RenderDevice,
        pipelines: &Pipelines,
        mask: &data::mask::Mask,
        size: u32,
        output: &Buffer,
        tiles: &Buffer,
    ) -> Option<Self> {
        use data::mask::MaskPass;
        let operations = mask.passes(size);
        if operations.is_empty() {
            return None;
        }
        let dense = operations
            .iter()
            .any(|op| matches!(op, MaskPass::Spatial(_)));
        let bounds = dense.then(|| {
            empty(
                device,
                "paint dirty bounds",
                16,
                BufferUsages::STORAGE | BufferUsages::COPY_DST,
            )
        });
        let reach = |pass: &MaskPass| match pass {
            MaskPass::Spatial(pass) => pass.reach(size),
            _ => 0,
        };
        let total_reach: u32 = operations.iter().map(reach).sum();
        let mut remaining = total_reach;
        let raw = buffer(
            device,
            "painted mask before filters",
            &mask.pixels,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        );
        let scratch: Vec<_> = (0..(operations.len() - 1).min(2))
            .map(|_| {
                empty(
                    device,
                    "paint mask filter scratch",
                    u64::from(size).pow(2) * 4,
                    BufferUsages::STORAGE,
                )
            })
            .collect();
        let last = operations.len() - 1;
        let mut pass_bytes = scratch.iter().map(|buffer| buffer.size()).sum::<u64>();
        let passes = operations
            .into_iter()
            .enumerate()
            .map(|(i, op)| {
                remaining -= reach(&op);
                let (kind, vertical, params, table) = match op {
                    MaskPass::Lookup(table) => (
                        0,
                        false,
                        [0.; 4],
                        table
                            .iter()
                            .copied()
                            .flat_map(u32::to_le_bytes)
                            .collect::<Vec<_>>(),
                    ),
                    MaskPass::Spatial(pass) => (
                        pass.kind as u32,
                        pass.vertical,
                        pass.params,
                        if pass.weights.is_empty() {
                            vec![0; 4]
                        } else {
                            pass.weights
                                .into_iter()
                                .flat_map(f32::to_le_bytes)
                                .collect()
                        },
                    ),
                    MaskPass::Affine(map) => (
                        8,
                        false,
                        [u32::from(map.is_some()) as f32, 0., 0., 0.],
                        map.map_or_else(
                            || vec![0; 4],
                            |map| map.iter().flat_map(|&v| [v, 0, 0, 0]).collect(),
                        ),
                    ),
                    MaskPass::GeneratorLayer(layer, map) => (
                        7,
                        false,
                        [
                            layer.blend as u32 as f32,
                            layer.opacity as f32 / 100.,
                            0.,
                            0.,
                        ],
                        map.values.iter().enumerate().flat_map(|(i, &value)| {
                            let coverage = map.coverage.as_ref().map_or(255, |coverage| coverage[i]);
                            [value, coverage, 0, 0]
                        }).collect(),
                    ),
                };
                let region = if dense { 2 } else { u32::from(size > 1024) };
                let mut uniform: Vec<_> = [size, kind, region, u32::from(vertical)]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect();
                uniform.extend(params.into_iter().flat_map(f32::to_le_bytes));
                // The final result changes within total_reach of the stamp.
                // Earlier passes also produce the pixels sampled by the remaining passes.
                uniform.extend(
                    [total_reach + remaining, 0, 0, 0]
                        .into_iter()
                        .flat_map(u32::to_le_bytes),
                );
                let uniform = buffer(
                    device,
                    "paint mask filter parameters",
                    &uniform,
                    BufferUsages::UNIFORM,
                );
                let table = buffer(
                    device,
                    "paint mask filter coefficients",
                    &table,
                    BufferUsages::STORAGE,
                );
                pass_bytes += uniform.size() + table.size();
                let src = if i == 0 { &raw } else { &scratch[(i - 1) % 2] };
                let dst = if i == last { output } else { &scratch[i % 2] };
                let entries: Vec<_> =
                    [&uniform, src, &table, dst, bounds.as_ref().unwrap_or(tiles)]
                        .into_iter()
                        .enumerate()
                        .map(|(i, buffer)| BindGroupEntry {
                            binding: i as u32,
                            resource: buffer.as_entire_binding(),
                        })
                        .collect();
                device.create_bind_group(
                    "paint mask filter pass",
                    &pipelines.mask_filter_layout,
                    &entries,
                )
            })
            .collect();
        Some(Self {
            raw,
            passes,
            bounds,
            pass_bytes,
        })
    }
}

impl Context {
    fn texture_bytes(&self) -> u64 {
        self.outputs.iter()
            .chain([&self.mask, &self.depth, &self.scene_ids])
            .chain(self.grain.as_ref().map(|(_, target)| target))
            .chain(self.uv_occupancy.as_ref())
            .map(Target::bytes).sum()
    }

    fn memory(&self) -> ContextMemory {
        let mut layers = vec![0; self.composition.layers.len()];
        let mut seen = HashSet::new();
        let mut unique_size = |buffer: &Buffer| {
            if seen.insert(buffer.id()) {
                buffer.size()
            } else {
                0
            }
        };
        let mut total = self.texture_bytes();
        total += [&self.vertices, &self.auxiliary, &self.baked_normal, &self.snapshot, &self.dirty, &self.tiles, &self.grid]
            .into_iter().chain(self.additional.iter().map(|channel| &channel.snapshot))
            .map(&mut unique_size).sum::<u64>();
        for composition in std::iter::once(&self.composition)
            .chain(self.additional.iter().map(|channel| &channel.composition))
            .chain(&self.mask_compositions)
        {
            total += [&composition.base, &composition.result, &composition.below]
                .into_iter().chain(&composition.saved).map(&mut unique_size).sum::<u64>();
            for (bytes, layer) in layers.iter_mut().zip(&composition.layers) {
                let content = unique_size(&layer.buffer);
                let mask = layer.mask.as_ref().map_or(0, &mut unique_size);
                total += content + mask;
                // Four-byte placeholders are bindings for absent channels.
                *bytes += if content > 4 { content } else { 0 };
                *bytes += mask;
            }
        }
        if let Some(filters) = &self.mask_filter {
            total += unique_size(&filters.raw);
            total += filters.bounds.as_ref().map_or(0, &mut unique_size);
            total += filters.pass_bytes;
        }
        ContextMemory { total, layers }
    }
}
fn composition_for_channel(
    device: &RenderDevice,
    surface: &Surface,
    channel: usize,
    masks: &[Option<Buffer>],
    paint_content: bool,
    composite_layers: bool,
) -> Composition {
    let bytes = (surface.size as u64).pow(2) * 4;
    let scratch = if composite_layers { bytes } else { 4 };
    let usage = BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST;
    let base = buffer(device, "paint material base", &surface.base[channel], usage);
    let result = empty(device, "paint composite", scratch, usage);
    let layers = surface
        .stack
        .layers
        .iter()
        .zip(masks)
        .enumerate()
        .map(|(i, (layer, mask))| {
            let buffer = match layer.channel(channel) {
                Some(data) => buffer(device, "paint layer", data, usage),
                None => empty(
                    device,
                    "empty paint layer",
                    if paint_content && i == surface.stack.selected {
                        bytes
                    } else {
                        4
                    },
                    usage,
                ),
            };
            LayerBuffer {
                buffer,
                mask: mask.clone(),
                opacity: layer.opacity / 100.,
                blend: layer.blend.for_channel(channel),
            }
        })
        .collect();
    let mut ops = surface.stack.composite_ops();
    ops.retain(|&(i, op)| {
        !matches!(op, data::CompositeOp::Layer)
            || surface.stack.layers[i].channel(channel).is_some()
            || (paint_content && i == surface.stack.selected)
    });
    let saved_count = ops
        .iter()
        .filter_map(|(_, op)| match op {
            data::CompositeOp::Save { slot } => Some(slot + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    // A cache boundary cannot split an isolated folder's Save and Fade.
    let cached_layers = if surface.stack.layers.is_empty() {
        0
    } else {
        surface
            .stack
            .parents(surface.stack.selected)
            .filter(|&i| surface.stack.layers[i].isolated() && surface.stack.shown(i))
            .map(|i| surface.stack.block(i).start)
            .min()
            .unwrap_or(surface.stack.selected)
    };
    Composition {
        base,
        result,
        below: empty(device, "paint cached lower layers", scratch, usage),
        layers,
        ops,
        saved: (0..saved_count)
            .map(|_| empty(device, "paint folder backdrop", bytes, usage))
            .collect(),
        cached_layers,
    }
}
fn blend(
    device: &RenderDevice,
    pipelines: &Pipelines,
    encoder: &mut CommandEncoder,
    composition: &Composition,
    size: u32,
    range: std::ops::Range<usize>,
    update: Option<(&Buffer, bool)>,
) {
    if range.start == 0 && update.is_none() {
        encoder.copy_buffer_to_buffer(
            &composition.base,
            0,
            &composition.result,
            0,
            size as u64 * size as u64 * 4,
        );
    }
    let mut reset = update.is_some();
    let tiles = update.and_then(|(tiles, sparse)| sparse.then_some(tiles));
    for &(index, op) in &composition.ops {
        if !range.contains(&index) {
            continue;
        }
        let layer = &composition.layers[index];
        let (source, target, mode) = match op {
            data::CompositeOp::Layer => (&layer.buffer, &composition.result, layer.blend as u32),
            data::CompositeOp::Save { slot } => {
                if reset {
                    dispatch_blend(
                        device,
                        pipelines,
                        encoder,
                        composition,
                        size,
                        tiles,
                        &composition.below,
                        &composition.result,
                        1.,
                        8,
                        false,
                        None,
                    );
                    reset = false;
                }
                (&composition.result, &composition.saved[slot], 8)
            }
            data::CompositeOp::Fade { slot } => (&composition.saved[slot], &composition.result, 9),
        };
        dispatch_blend(
            device,
            pipelines,
            encoder,
            composition,
            size,
            tiles,
            source,
            target,
            layer.opacity,
            mode,
            reset,
            layer.mask.as_ref(),
        );
        reset = false;
    }
}

fn dispatch_blend(
    device: &RenderDevice,
    pipelines: &Pipelines,
    encoder: &mut CommandEncoder,
    composition: &Composition,
    size: u32,
    tiles: Option<&Buffer>,
    source: &Buffer,
    target: &Buffer,
    opacity: f32,
    mode: u32,
    reset: bool,
    mask: Option<&Buffer>,
) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&opacity.to_le_bytes());
    let flags = u32::from(reset)
        | (u32::from(tiles.is_some() && size > 1024) << 1)
        | (u32::from(mask.is_some()) << 2);
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&mode.to_le_bytes());
    let u = buffer(device, "paint layer opacity", &bytes, BufferUsages::UNIFORM);
    let bg = device.create_bind_group(
        "paint layer blend",
        &pipelines.blend_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: u.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: source.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: target.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: composition.below.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 4,
                resource: tiles.unwrap_or(&composition.base).as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: mask.unwrap_or(&composition.base).as_entire_binding(),
            },
        ],
    );
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("paint layer blend"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.blend);
    pass.set_bind_group(0, &*bg, &[]);
    if let Some(tiles) = tiles.filter(|_| size > 1024) {
        pass.dispatch_workgroups_indirect(tiles, 0);
    } else {
        pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
    }
}
fn params(
    size: u32,
    channel: usize,
    object: u32,
    slot: u32,
    brush: Brush,
    view: View,
    count: u32,
    relief_factor: f32,
) -> Vec<u8> {
    let c = view.camera;
    let aspect = c.rect.width() / c.rect.height();
    let projection = if c.ortho {
        bevy::math::proj::orthographic(
            -c.height * aspect * 0.5,
            c.height * aspect * 0.5,
            -c.height * 0.5,
            c.height * 0.5,
            c.depth,
            -c.depth,
        )
    } else {
        bevy::math::proj::perspective_infinite_reverse(
            c.fov,
            aspect,
            (c.height * 0.0001).max(c.epsilon * 0.01).max(0.000001),
        )
    };
    let vp = projection * c.inverse;
    let mut bytes = Vec::with_capacity(304);
    for f in vp
        .to_cols_array()
        .into_iter()
        .chain(vp.inverse().to_cols_array())
        .chain([
            c.pose.translation.x,
            c.pose.translation.y,
            c.pose.translation.z,
            c.epsilon * 4.,
        ])
        .chain([
            c.pose.forward().x,
            c.pose.forward().y,
            c.pose.forward().z,
            relief_factor,
        ])
        .chain([c.rect.min.x, c.rect.min.y, c.rect.width(), c.rect.height()])
        .chain([brush.radius, brush.hardness, brush.flow, brush.opacity])
        .chain([
            brush.color[0] as f32 / 255.,
            brush.color[1] as f32 / 255.,
            brush.color[2] as f32 / 255.,
            1.,
        ])
    {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    for i in [
        size,
        count,
        u32::from(view.uv),
        u32::from(brush.fill),
        object,
        slot,
        u32::from(brush.erase),
        channel as u32,
    ] {
        bytes.extend_from_slice(&i.to_le_bytes());
    }
    let (g, switches) = brush.grain.map_or(([0.; 4], [0u32; 4]), |grain| {
        let (sin, cos) = grain.angle.sin_cos();
        (
            [grain.strength, grain.repeat, sin, cos],
            [u32::from(grain.uv), u32::from(grain.invert), 0, 0],
        )
    });
    bytes.extend(brush.mirror.into_iter().chain(g).flat_map(f32::to_le_bytes));
    bytes.extend(switches.into_iter().flat_map(u32::to_le_bytes));
    bytes.extend(
        [
            brush.projection as u32 as f32,
            brush.facing_limit,
            if brush.paint_through { 1. } else { 0. },
            if view.camera.ortho { 1. } else { 0. },
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes),
    );
    bytes
}
fn color_attachment(view: &TextureView, clear: bool) -> RenderPassColorAttachment<'_> {
    RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: Operations {
            load: if clear {
                LoadOp::Clear(Default::default())
            } else {
                LoadOp::Load
            },
            store: StoreOp::Store,
        },
    }
}
fn raster(
    encoder: &mut CommandEncoder,
    label: &'static str,
    pipeline: &RenderPipeline,
    bg: &BindGroup,
    vertices: &Buffer,
    draws: impl IntoIterator<Item = std::ops::Range<u32>>,
    color: &TextureView,
    depth: Option<&TextureView>,
    clear: bool,
) {
    let attachments = [Some(color_attachment(color, clear))];
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachments,
        depth_stencil_attachment: depth.map(|view| RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(Operations {
                load: LoadOp::Clear(0.),
                store: StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &**bg, &[]);
    pass.set_vertex_buffer(0, (**vertices).slice(..));
    for draw in draws {
        pass.draw(draw, 0..1);
    }
}
impl Context {
    fn new(
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        surface: &Surface,
        channel: usize,
        channels: u16,
        images: [AssetId<Image>; composite::IMAGE_COUNT],
        scene: &Scene,
        object: u32,
        slot: u32,
        relief_factor: f32,
        baked_normal: Option<&[u8]>,
        fix: Option<(Vec<u8>, bool)>,
    ) -> Self {
        let size = surface.size;
        let rt = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
        let masking = channel == data::MASK_TARGET;
        let usage = BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST;
        let masks: Vec<_> = surface
            .stack
            .layers
            .iter()
            .map(|layer| {
                layer
                    .mask
                    .as_ref()
                    .map(|mask| buffer(device, "paint layer mask", mask.effective(), usage))
            })
            .collect();
        let composition = composition_for_channel(
            device,
            surface,
            if masking { 0 } else { channel },
            &masks,
            !masking,
            fix.is_none(),
        );
        let additional: Vec<_> = channels_in(channels).filter(|&target| target != channel)
            .map(|channel| StrokeChannel {
                channel,
                composition: composition_for_channel(device, surface, channel, &masks, true, true),
                snapshot: empty(device, "paint stroke snapshot", u64::from(size).pow(2) * 4, usage),
            })
            .collect();
        let mask_compositions: Vec<_> = if masking {
            (1..data::CHANNEL_COUNT)
                .map(|channel| composition_for_channel(device, surface, channel, &masks, false, true))
                .collect()
        } else {
            Vec::new()
        };
        let fix_normal = fix.as_ref().map(|(_, normal)| *normal);
        let auxiliary = if let Some((base, _)) = fix {
            buffer(device, "baked map before fixes", &base, BufferUsages::STORAGE)
        } else if (channels & ((1 << 3) | (1 << 6))).count_ones() == 1 {
            let aux = composition_for_channel(
                device,
                surface,
                if channels & (1 << 3) != 0 { 6 } else { 3 },
                &masks,
                false,
                true,
            );
            blend(
                device,
                pipelines,
                encoder,
                &aux,
                size,
                0..aux.layers.len(),
                None,
            );
            aux.result
        } else {
            empty(device, "paint auxiliary", 16, BufferUsages::STORAGE)
        };
        let vertices = if scene.vertices.is_empty() {
            empty(device, "empty paint mesh", 32, BufferUsages::VERTEX)
        } else {
            buffer(device, "paint mesh", &scene.vertices, BufferUsages::VERTEX)
        };
        // Document/layer changes invalidate this context, so the lower stack stays
        // valid for every stamp and consecutive stroke on the selected layer.
        for composition in std::iter::once(&composition)
            .chain(&mask_compositions)
            .chain(additional.iter().map(|channel| &channel.composition))
            .filter(|_| fix_normal.is_none())
        {
            blend(
                device,
                pipelines,
                encoder,
                &composition,
                size,
                0..composition.cached_layers,
                None,
            );
            encoder.copy_buffer_to_buffer(
                &composition.result,
                0,
                &composition.below,
                0,
                size as u64 * size as u64 * 4,
            );
            blend(
                device,
                pipelines,
                encoder,
                &composition,
                size,
                composition.cached_layers..composition.layers.len(),
                None,
            );
        }
        let columns = size.div_ceil(8);
        let tile_count = columns * columns;
        let tiles = empty(
            device,
            "paint indirect tiles",
            (tile_count as u64 + 4) * 4,
            BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
        );
        let mask_filter = if masking {
            surface.stack.layers[surface.stack.selected]
                .mask
                .as_ref()
                .and_then(|mask| {
                    MaskFilters::new(
                        device,
                        pipelines,
                        mask,
                        size,
                        masks[surface.stack.selected].as_ref().unwrap(),
                        &tiles,
                    )
                })
        } else {
            None
        };
        let mut grid_bytes = Vec::new();
        for v in [columns, columns, u32::from(channels & (1 << 3) != 0 || masking), 0] {
            grid_bytes.extend_from_slice(&v.to_le_bytes());
        }
        Self {
            size,
            channel,
            trace_revision: 0,
            selected: surface.stack.selected,
            images,
            object,
            slot,
            vertices,
            count: scene.count,
            mask: target(
                device,
                "paint stroke coverage",
                size,
                size,
                TextureFormat::Rgba16Float,
                rt,
            ),
            grain: None,
            depth: target(
                device,
                "paint scene depth",
                1,
                1,
                TextureFormat::Depth32Float,
                rt,
            ),
            scene_ids: target(
                device,
                "paint scene faces",
                1,
                1,
                TextureFormat::Rg32Uint,
                rt,
            ),
            uv_occupancy: None,
            outputs: std::array::from_fn(|index| {
                let extent = if index == 1 && fix_normal.is_some() { 1 } else { size };
                target(
                    device,
                    "paint display",
                    extent,
                    extent,
                    TextureFormat::Rgba8Unorm,
                    TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
                )
            }),
            snapshot: empty(
                device,
                "paint stroke snapshot",
                size as u64 * size as u64 * 4,
                usage,
            ),
            composition,
            additional,
            auxiliary,
            baked_normal: buffer(
                device, "paint baked normal", baked_normal.unwrap_or(&[0; 4]), BufferUsages::STORAGE,
            ),
            relief_factor,
            mask_compositions,
            mask_filter,
            fix_normal,
            mask_affine: masking
                && surface.stack.layers[surface.stack.selected]
                    .mask
                    .as_ref()
                    .is_some_and(|mask| mask.affine),
            params: None,
            trace: None,
            tip_motion: (None, 0.),
            stamp_noise: stroke::Noise::new(0),
            dirty: empty(
                device,
                "paint touched tile flags",
                tile_count as u64 * 4,
                BufferUsages::STORAGE | BufferUsages::COPY_DST,
            ),
            tiles,
            grid: buffer(
                device,
                "paint tile grid",
                &grid_bytes,
                BufferUsages::UNIFORM,
            ),
            presented: false,
        }
    }
    fn stroke_buffer(&self) -> &Buffer {
        let layer = &self.composition.layers[self.selected];
        if self.channel == data::MASK_TARGET {
            self.mask_filter.as_ref().map_or_else(
                || layer.mask.as_ref().expect("mask stroke has a mask"),
                |filter| &filter.raw,
            )
        } else {
            &layer.buffer
        }
    }

    fn begin(
        &mut self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        stroke_id: u64,
        brush: Brush,
        view: View,
        images: &RenderAssets<GpuImage>,
        trace: Option<Arc<surface::SurfaceTrace>>,
    ) {
        self.params = Some((brush, view));
        self.trace = trace;
        self.tip_motion = (None, 0.);
        self.stamp_noise = stroke::Noise::new(stroke_id);
        let extent = if view.uv {
            UVec2::ONE
        } else {
            view.camera.rect.size().ceil().as_uvec2().max(UVec2::ONE)
        };
        if self.depth.texture.size().width != extent.x
            || self.depth.texture.size().height != extent.y
        {
            let usage = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
            self.depth = target(
                device,
                "paint scene depth",
                extent.x,
                extent.y,
                TextureFormat::Depth32Float,
                usage,
            );
            self.scene_ids = target(
                device,
                "paint scene faces",
                extent.x,
                extent.y,
                TextureFormat::Rg32Uint,
                usage,
            );
        }
        encoder.copy_buffer_to_buffer(
            self.stroke_buffer(),
            0,
            &self.snapshot,
            0,
            self.size as u64 * self.size as u64 * 4,
        );
        for target in &self.additional {
            encoder.copy_buffer_to_buffer(
                &target.composition.layers[self.selected].buffer, 0, &target.snapshot, 0,
                u64::from(self.size).pow(2) * 4,
            );
        }
        let u = buffer(
            device,
            "paint view",
            &params(
                self.size,
                self.channel,
                self.object,
                self.slot,
                brush,
                view,
                0,
                self.relief_factor,
            ),
            BufferUsages::UNIFORM,
        );
        let bg = device.create_bind_group(
            "paint view",
            &pipelines.scene_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: u.as_entire_binding(),
            }],
        );
        if !brush.fill && !view.uv {
            raster(
                encoder,
                "paint scene depth",
                &pipelines.depth,
                &bg,
                &self.vertices,
                [0..self.count],
                &self.scene_ids.view,
                Some(&self.depth.view),
                true,
            );
        }
        if brush.fill && brush.fill_padding > 0. && self.uv_occupancy.is_none() {
            let occupancy = target(
                device,
                "paint UV occupancy",
                self.size,
                self.size,
                TextureFormat::R32Uint,
                TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            );
            raster(
                encoder, "paint UV occupancy", &pipelines.occupancy, &bg, &self.vertices,
                [0..self.count], &occupancy.view, None, true,
            );
            self.uv_occupancy = Some(occupancy);
        }
        if let Some(grain) = brush.grain
            && self.grain.as_ref().is_none_or(|(old, _)| *old != grain)
            && let Some(pattern) = images.get(grain.image)
        {
            let target = self.grain.take().map_or_else(
                || {
                    target(
                        device,
                        "paint surface grain",
                        self.size,
                        self.size,
                        TextureFormat::R16Float,
                        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
                    )
                },
                |(_, target)| target,
            );
            let bind = device.create_bind_group(
                "paint grain",
                &pipelines.grain_layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: u.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 8,
                        resource: BindingResource::TextureView(&pattern.texture_view),
                    },
                    BindGroupEntry {
                        binding: 9,
                        resource: BindingResource::Sampler(&pipelines.grain_sampler),
                    },
                ],
            );
            raster(
                encoder,
                "paint surface grain",
                &pipelines.grain,
                &bind,
                &self.vertices,
                [0..self.count],
                &target.view,
                None,
                true,
            );
            self.grain = Some((grain, target));
        }
        // Clear once per stroke; subsequent batches blend coverage into this same allocation.
        let attachments = [Some(color_attachment(&self.mask.view, true))];
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("clear paint stroke"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    fn points(
        &mut self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        points: &[Stamp],
        ranges: Option<Vec<std::ops::Range<u32>>>,
        images: &RenderAssets<GpuImage>,
    ) {
        let Some((brush, view)) = self.params else {
            return;
        };
        let fill_ranges = if let Some(ranges) = ranges {
            if ranges.is_empty() {
                return;
            }
            Some(ranges)
        } else if brush.fill {
            let Some(trace) = &self.trace else { return };
            let ranges = trace.fill_ranges(
                view,
                (self.object - 1) as usize,
                self.slot as usize,
                brush.fill_mode,
                brush.fill_angle,
                points,
            );
            if ranges.is_empty() {
                return;
            }
            Some(ranges)
        } else {
            None
        };
        // At 1K, a single dense cached-layer pass is cheaper than compaction
        // and indirect dispatch. Larger textures benefit from sparse UV tiles.
        let bounds = self
            .mask_filter
            .as_ref()
            .and_then(|filters| filters.bounds.as_ref());
        if self.size > 1024 || bounds.is_some() {
            encoder.clear_buffer(&self.dirty, 0, None);
        }
        if self.size > 1024 {
            encoder.clear_buffer(&self.tiles, 0, None);
        }
        const STAMP_BYTES: usize = 96;
        let mut bytes = Vec::with_capacity(points.len() * STAMP_BYTES);
        for &stamp in points.iter().filter(|_| !brush.fill) {
            let source = stamp.point;
            let (stamp, jitter) = self.stamp_noise.vary(stamp, brush.radius, brush.variation);
            let p = stamp.point;
            let (sin, cos) = brush.tip.map_or((0., 1.), |tip| {
                if tip.follow {
                    let (last, heading) = &mut self.tip_motion;
                    if let Some(delta) = last.replace(source).map(|last| source - last)
                        && delta.length_squared() > 1.0e-5
                    {
                        *heading = delta.to_angle();
                    }
                    (tip.angle + stamp.heading.unwrap_or(*heading) + jitter).sin_cos()
                } else {
                    (tip.angle + jitter).sin_cos()
                }
            });
            let placements = match &self.trace {
                Some(trace) => trace.resolve(
                    view.camera,
                    (self.object - 1) as usize,
                    self.slot as usize,
                    stamp,
                    brush,
                ),
                None => [Some(surface::Placement::default()), None],
            };
            for placement in placements.into_iter().flatten() {
                for f in [
                    p.x,
                    p.y,
                    cos,
                    sin,
                    stamp.radius,
                    stamp.flow,
                    if placement.reflected { -1. } else { 1. },
                    0.,
                ]
                .into_iter()
                .chain(placement.center.extend(placement.radius).to_array())
                .chain(placement.normal.extend(placement.uv_radius).to_array())
                .chain(placement.tangent.extend(0.).to_array())
                .chain([placement.uv.x, placement.uv.y, 0., 0.])
                {
                    bytes.extend_from_slice(&f.to_le_bytes());
                }
            }
        }
        if brush.fill {
            bytes.resize(STAMP_BYTES, 0);
        } else if bytes.is_empty() {
            return;
        }
        let u = buffer(
            device,
            "paint brush",
            &params(
                self.size,
                self.channel
                    | (usize::from(bounds.is_some()) << 31)
                    | (usize::from(self.mask_affine) << 30)
                    | (usize::from(brush.tip.is_some()) << 29)
                    | (usize::from(brush.grain.is_some()) << 28),
                self.object,
                self.slot,
                brush,
                view,
                (bytes.len() / STAMP_BYTES) as u32,
                self.relief_factor,
            ),
            BufferUsages::UNIFORM,
        );
        let tip = brush
            .tip
            .and_then(|tip| images.get(tip.image))
            .unwrap_or_else(|| images.get(self.images[0]).unwrap());
        let stamps = buffer(
            device,
            "paint stamp positions",
            &bytes,
            BufferUsages::STORAGE,
        );
        let mask = device.create_bind_group(
            "paint mask",
            &pipelines.mask_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: u.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: stamps.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&self.depth.view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&self.scene_ids.view),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: self.dirty.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::TextureView(&tip.texture_view),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(&pipelines.tip_sampler),
                },
            ],
        );
        if brush.fill_padding > 0.
            && let (Some(ranges), Some(trace), Some(occupancy)) = (&fill_ranges, &self.trace, &self.uv_occupancy)
        {
            let padding = trace.fill_padding(ranges, brush.fill_padding, self.size);
            if !padding.is_empty() {
                let bind = device.create_bind_group("paint UV padding", &pipelines.pad_layout, &[
                    BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    BindGroupEntry { binding: 4, resource: BindingResource::TextureView(&occupancy.view) },
                    BindGroupEntry { binding: 5, resource: self.dirty.as_entire_binding() },
                ]);
                for strip in padding.chunks(96_000) {
                    let mut bytes = Vec::with_capacity(strip.len() * 8);
                    for uv in strip {
                        bytes.extend_from_slice(&uv.x.to_le_bytes());
                        bytes.extend_from_slice(&uv.y.to_le_bytes());
                    }
                    let vertices = buffer(device, "paint UV padding strips", &bytes, BufferUsages::VERTEX);
                    raster(
                        encoder, "paint UV padding", &pipelines.padding, &bind, &vertices,
                        [0..strip.len() as u32], &self.mask.view, None, false,
                    );
                }
            }
        }
        raster(
            encoder,
            "paint stamps",
            &pipelines.mask,
            &mask,
            &self.vertices,
            fill_ranges.into_iter().flatten().chain((!brush.fill).then_some(0..self.count)),
            &self.mask.view,
            None,
            false,
        );
        if self.size > 1024 {
            let tiles = device.create_bind_group(
                "paint dirty tiles",
                &pipelines.tiles_layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self.grid.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: self.dirty.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: self.tiles.as_entire_binding(),
                    },
                ],
            );
            {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("compact paint tiles"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipelines.tiles);
                pass.set_bind_group(0, &*tiles, &[]);
                pass.dispatch_workgroups((self.size.div_ceil(8).pow(2)).div_ceil(64), 1, 1);
            }
        }
        if let Some(bounds) = bounds {
            encoder.clear_buffer(bounds, 0, None);
            let entries: Vec<_> = [&self.grid, &self.dirty, bounds]
                .into_iter()
                .enumerate()
                .map(|(binding, buffer)| BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect();
            let bg =
                device.create_bind_group("paint dirty bounds", &pipelines.tiles_layout, &entries);
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("paint dirty bounds"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.tile_bounds);
            pass.set_bind_group(0, &*bg, &[]);
            pass.dispatch_workgroups(self.size.div_ceil(8).pow(2).div_ceil(64), 1, 1);
        }
        self.apply_coverage(device, pipelines, encoder, &u, &self.snapshot, self.stroke_buffer());
        for target in &self.additional {
            let channel = target.channel | (usize::from(brush.grain.is_some()) << 28);
            let uniform = buffer(
                device, "paint channel values",
                &params(self.size, channel, self.object, self.slot, brush.for_channel(target.channel), view, 0, self.relief_factor),
                BufferUsages::UNIFORM,
            );
            self.apply_coverage(device, pipelines, encoder, &uniform, &target.snapshot,
                &target.composition.layers[self.selected].buffer);
        }
        if self.fix_normal.is_some() {
            self.present_fix(device, pipelines, encoder, images, brush, view, !self.presented);
            return;
        }
        if let Some(filters) = &self.mask_filter {
            for bg in &filters.passes {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("paint mask filters"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipelines.mask_filter);
                pass.set_bind_group(0, &**bg, &[]);
                if bounds.is_none() && self.size > 1024 {
                    pass.dispatch_workgroups_indirect(&self.tiles, 0);
                } else {
                    pass.dispatch_workgroups(self.size.div_ceil(8), self.size.div_ceil(8), 1);
                }
            }
        }
        let sparse = bounds.is_none();
        for composition in std::iter::once(&self.composition)
            .chain(&self.mask_compositions)
            .chain(self.additional.iter().map(|channel| &channel.composition))
        {
            blend(
                device,
                pipelines,
                encoder,
                composition,
                self.size,
                composition.cached_layers..composition.layers.len(),
                Some((&self.tiles, sparse)),
            );
        }
        if self.channel == data::MASK_TARGET {
            let map = |channel: usize| {
                if channel == 0 {
                    &self.composition.result
                } else {
                    &self.mask_compositions[channel - 1].result
                }
            };
            // Order paired outputs so alpha uses the fresh color and metal uses
            // the fresh roughness. Height and normal present together in pass 3.
            for channel in [0, 4, 1, 2, 3, 5, 7] {
                let uniform = buffer(
                    device,
                    "paint mask display",
                    &params(self.size, channel, self.object, self.slot, brush, view, 0, self.relief_factor),
                    BufferUsages::UNIFORM,
                );
                self.present_map(
                    device,
                    pipelines,
                    encoder,
                    images,
                    &uniform,
                    channel,
                    map(channel),
                    if channel == 3 {
                        map(6)
                    } else {
                        &self.auxiliary
                    },
                    true,
                );
            }
            self.present_map(
                device,
                pipelines,
                encoder,
                images,
                &u,
                data::MASK_TARGET,
                self.composition.layers[self.selected]
                    .mask
                    .as_ref()
                    .unwrap(),
                &self.auxiliary,
                true,
            );
        } else if self.additional.is_empty() {
            self.present_map(
                device,
                pipelines,
                encoder,
                images,
                &u,
                self.channel,
                &self.composition.result,
                &self.auxiliary,
                !self.presented,
            );
        } else {
            self.present_channels(device, pipelines, encoder, images, brush, view);
        }
        self.presented = true;
    }

    fn apply_coverage(
        &self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        uniform: &Buffer,
        snapshot: &Buffer,
        layer: &Buffer,
    ) {
        let apply = device.create_bind_group(
            "paint apply", &pipelines.apply_layout,
            &[
                BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: BindingResource::TextureView(&self.mask.view) },
                BindGroupEntry { binding: 2, resource: snapshot.as_entire_binding() },
                BindGroupEntry { binding: 3, resource: layer.as_entire_binding() },
                BindGroupEntry { binding: 4, resource: self.tiles.as_entire_binding() },
                BindGroupEntry { binding: 5, resource: BindingResource::TextureView(
                    self.grain.as_ref().map_or(&self.mask.view, |(_, target)| &target.view),
                ) },
            ],
        );
        compute_tiles(encoder, "paint apply", &pipelines.apply, &apply, &self.tiles, self.size);
    }

    fn present_channels(
        &self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        images: &RenderAssets<GpuImage>,
        brush: Brush,
        view: View,
    ) {
        let result = |channel| {
            if self.channel == channel {
                Some(&self.composition.result)
            } else {
                self.additional.iter().find(|target| target.channel == channel)
                    .map(|target| &target.composition.result)
            }
        };
        // These pairs write to the same material textures. If both height and
        // tangent normals changed, publish their combined result in one pass.
        for channel in [0, 4, 1, 2, 3, 6, 5, 7] {
            if channel == 6 && result(3).is_some() {
                continue;
            }
            let Some(pixels) = result(channel) else { continue };
            let auxiliary = if channel == 3 {
                result(6).unwrap_or(&self.auxiliary)
            } else {
                &self.auxiliary
            };
            let uniform = buffer(
                device, "paint channel display",
                &params(self.size, channel, self.object, self.slot, brush, view, 0, self.relief_factor),
                BufferUsages::UNIFORM,
            );
            self.present_map(device, pipelines, encoder, images, &uniform, channel, pixels,
                auxiliary, !self.presented);
        }
    }

    fn present_fix(
        &mut self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        images: &RenderAssets<GpuImage>,
        brush: Brush,
        view: View,
        full: bool,
    ) {
        let channel = if self.fix_normal == Some(true) { 10 } else { 9 };
        let uniform = buffer(
            device,
            "bake fix display",
            &params(self.size, channel, self.object, self.slot, brush, view, 0, self.relief_factor),
            BufferUsages::UNIFORM,
        );
        self.present_map(
            device, pipelines, encoder, images, &uniform, channel,
            self.stroke_buffer(), &self.auxiliary, full,
        );
        self.presented = true;
    }

    fn cancel_fix(
        &mut self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        images: &RenderAssets<GpuImage>,
    ) {
        if self.fix_normal.is_some() && let Some((brush, view)) = self.params.take() {
            encoder.copy_buffer_to_buffer(
                &self.snapshot, 0, self.stroke_buffer(), 0, u64::from(self.size).pow(2) * 4,
            );
            self.present_fix(device, pipelines, encoder, images, brush, view, true);
        }
    }

    fn present_map(
        &self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        images: &RenderAssets<GpuImage>,
        u: &Buffer,
        channel: usize,
        result: &Buffer,
        auxiliary: &Buffer,
        full: bool,
    ) {
        let present = device.create_bind_group(
            "paint present",
            &pipelines.present_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: u.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: result.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: auxiliary.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(
                        &images.get(self.images[0]).unwrap().texture_view,
                    ),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::TextureView(
                        &images.get(self.images[1]).unwrap().texture_view,
                    ),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(&self.outputs[0].view),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::TextureView(&self.outputs[1].view),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: self.tiles.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: self.baked_normal.as_entire_binding(),
                },
            ],
        );
        if !full {
            compute_tiles(
                encoder,
                "paint present",
                &pipelines.present,
                &present,
                &self.tiles,
                self.size,
            );
        } else {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("initialize paint display"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.present_full);
            pass.set_bind_group(0, &*present, &[]);
            pass.dispatch_workgroups(self.size.div_ceil(8), self.size.div_ceil(8), 1);
        }
        let destinations: &[usize] = match channel {
            0 => &[0],
            1 => &[1, 4],
            2 => &[1, 5],
            3 => &[2, 6],
            4 => &[0, 7],
            5 => &[3],
            6 => &[2],
            7 => &[composite::PREVIEW[7], composite::SPECULAR_MATERIAL],
            data::MASK_TARGET => &[composite::MASK_PREVIEW],
            9 | 10 => &[0],
            _ => unreachable!("unknown paint output channel {channel}"),
        };
        for (i, &destination) in destinations.iter().enumerate() {
            encoder.copy_texture_to_texture(
                self.outputs[i].texture.as_image_copy(),
                images
                    .get(self.images[destination])
                    .unwrap()
                    .texture
                    .as_image_copy(),
                Extent3d {
                    width: self.size,
                    height: self.size,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}
fn compute_tiles(
    encoder: &mut CommandEncoder,
    label: &'static str,
    pipeline: &ComputePipeline,
    bg: &BindGroup,
    tiles: &Buffer,
    size: u32,
) {
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some(label),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &**bg, &[]);
    if size > 1024 {
        pass.dispatch_workgroups_indirect(tiles, 0);
    } else {
        pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
    }
}
fn execute(
    mut pending: ResMut<Pending>,
    mut state: ResMut<State>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<GpuImage>>,
) {
    let _ = device.poll(PollType::Poll);
    if pending.commands.is_empty() {
        return;
    }
    if state.pipelines.is_none() {
        state.pipelines = Some(Pipelines::new(&device));
    }
    let State {
        pipelines,
        contexts,
    } = &mut *state;
    let pipelines = pipelines.as_ref().unwrap();
    while let Some(command) = pending.commands.pop_front() {
        if let Command::Setup { images: ids, .. } = &command {
            if ids.iter().any(|id| images.get(*id).is_none()) {
                pending.commands.push_front(command);
                break;
            }
        }
        if let Command::Begin { brush, .. } = &command
            && (brush.tip.is_some_and(|tip| images.get(tip.image).is_none())
                || brush
                    .grain
                    .is_some_and(|grain| images.get(grain.image).is_none()))
        {
            pending.commands.push_front(command);
            break;
        }
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("model paint GPU"),
        });
        let mut readback = None;
        match command {
            Command::Setup {
                id,
                surface,
                channel,
                channels,
                images,
                scene,
                object,
                slot,
                relief_factor,
                baked_preview,
                fix,
            } => {
                let context = Context::new(
                        &device,
                        pipelines,
                        &mut encoder,
                        &surface,
                        channel,
                        channels,
                        images,
                        &scene,
                        object,
                        slot,
                        relief_factor,
                        baked_preview.as_ref().and_then(|bake| bake.normal.as_deref()),
                        fix,
                );
                pending.stats.contexts.lock().unwrap().insert(id, context.memory());
                contexts.insert(id, context);
            }
            Command::Begin {
                id,
                stroke,
                brush,
                view,
                trace,
            } => {
                if let Some(context) = contexts.get_mut(&id) {
                    let before = context.texture_bytes();
                    context.begin(
                        &device,
                        pipelines,
                        &mut encoder,
                        stroke,
                        brush,
                        view,
                        &images,
                        trace,
                    );
                    let after = context.texture_bytes();
                    if before != after
                        && let Some(memory) = pending.stats.contexts.lock().unwrap().get_mut(&id)
                    {
                        memory.total = memory.total - before + after;
                    }
                }
            }
            Command::Points { id, points, ranges } => {
                if !points.is_empty() || ranges.is_some() {
                    if let Some(context) = contexts.get_mut(&id) {
                        context.points(&device, pipelines, &mut encoder, &points, ranges, &images);
                        context.trace_revision += 1;
                        pending.stats.batches.fetch_add(1, Ordering::Relaxed);
                        pending
                            .stats
                            .stamps
                            .fetch_add(points.len() as u64, Ordering::Relaxed);
                    }
                }
            }
            Command::End { id, stroke } => {
                if let Some(context) = contexts.get_mut(&id) {
                    let bytes = context.size as u64 * context.size as u64 * 4;
                    let staging = std::iter::once(context.stroke_buffer())
                        .chain(context.additional.iter().map(|target| &target.composition.layers[context.selected].buffer))
                        .map(|source| {
                            let staging = empty(
                                &device, "paint layer readback", bytes,
                                BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                            );
                            encoder.copy_buffer_to_buffer(source, 0, &staging, 0, bytes);
                            staging
                        })
                        .collect();
                    readback = Some((id, stroke, staging));
                    context.params = None;
                    pending.stats.readbacks.fetch_add(1, Ordering::Relaxed);
                }
            }
            Command::CancelFix(id) => {
                if let Some(context) = contexts.get_mut(&id) {
                    context.cancel_fix(&device, pipelines, &mut encoder, &images);
                }
            }
            Command::Pick { id, images: selected, uv } => {
                let textures = selected.map(|image| images.get(image));
                let [Some(color), Some(properties)] = textures else {
                    pending.replies.lock().unwrap().push(Reply::Pick {
                        id, sample: Err("Текстуры выбранного набора ещё не загружены.".into()),
                    });
                    continue;
                };
                sample_texel(&device, &queue, encoder, [color, properties], uv, id, pending.replies.clone());
                continue;
            }
            Command::Drop(id) => {
                contexts.remove(&id);
                pending.stats.contexts.lock().unwrap().remove(&id);
            }
        }
        queue.submit([encoder.finish()]);
        if let Some((id, stroke, staging)) = readback {
            read_stroke(id, stroke, staging, pending.replies.clone());
        }
    }
}

fn read_stroke(id: u64, stroke: u64, buffers: Vec<Buffer>, replies: Arc<Mutex<Vec<Reply>>>) {
    let channels = Arc::new(Mutex::new(vec![None; buffers.len()]));
    for (index, buffer) in buffers.into_iter().enumerate() {
        let channels = channels.clone();
        let replies = replies.clone();
        let mapped = buffer.clone();
        buffer.slice(..).map_async(MapMode::Read, move |status| {
            let pixels = status
                .map_err(|error| format!("GPU readback: {error}"))
                .and_then(|()| {
                    let view = mapped.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
                    let pixels = view.to_vec();
                    drop(view);
                    mapped.unmap();
                    Ok(pixels)
                });
            let mut channels = channels.lock().unwrap();
            channels[index] = Some(pixels);
            if channels.iter().all(Option::is_some) {
                let pixels = std::mem::take(&mut *channels).into_iter().map(Option::unwrap).collect();
                replies.lock().unwrap().push(Reply::Layer { id, stroke, pixels });
            }
        });
    }
}

fn sample_texel(
    device: &RenderDevice,
    queue: &RenderQueue,
    mut encoder: CommandEncoder,
    textures: [&GpuImage; 2],
    uv: Vec2,
    id: u64,
    replies: Arc<Mutex<Vec<Reply>>>,
) {
    const STRIDE: u64 = 256;
    let buffer = empty(device, "material picker", 2 * STRIDE, BufferUsages::COPY_DST | BufferUsages::MAP_READ);
    for (channel, image) in textures.into_iter().enumerate() {
        let size = image.texture.size();
        let x = (uv.x * size.width as f32) as u32;
        let y = (uv.y * size.height as f32) as u32;
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &image.texture, mip_level: 0, aspect: TextureAspect::All,
                origin: Origin3d { x: x.min(size.width - 1), y: y.min(size.height - 1), z: 0 },
            },
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: channel as u64 * STRIDE, bytes_per_row: Some(STRIDE as u32), rows_per_image: None,
                },
            },
            Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
    }
    queue.submit([encoder.finish()]);
    let mapped = buffer.clone();
    buffer.slice(..).map_async(MapMode::Read, move |status| {
        let sample = status.map_err(|error| format!("Не удалось считать материал · {error}"))
            .and_then(|_| {
                let bytes = mapped.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
                let sample = super::picker::MaterialSample {
                    color: [bytes[0], bytes[1], bytes[2]],
                    roughness: bytes[STRIDE as usize + 1],
                    metallic: bytes[STRIDE as usize + 2],
                };
                drop(bytes);
                mapped.unmap();
                Ok(sample)
            });
        replies.lock().unwrap().push(Reply::Pick { id, sample });
    });
}
