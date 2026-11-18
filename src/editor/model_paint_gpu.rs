//! Resident GPU stroke masks, layer buffers and compositing on Bevy's own device.
//! The only readback is an asynchronous layer snapshot at a stroke boundary.
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
    collections::HashMap,
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
}
#[derive(Clone)]
pub(super) struct Scene {
    pub vertices: Vec<u8>,
    pub count: u32,
}
impl Scene {
    pub fn prepare(objects: &[Object]) -> Self {
        let mut vertices = Vec::new();
        let mut count = 0;
        for object in objects.iter().filter(|o| !o.hidden) {
            for (face, f) in object.geometry.faces.iter().enumerate() {
                for t in object.geometry.triangles(f) {
                    for c in t {
                        let corner = &f.corners[c];
                        let p = object
                            .transform
                            .transform_point(object.geometry.positions[corner.vertex]);
                        for v in [p.x, p.y, p.z, corner.uv.x, corner.uv.y] {
                            vertices.extend_from_slice(&v.to_le_bytes());
                        }
                        for v in [face as u32 + 1, object.id as u32 + 1, f.material as u32] {
                            vertices.extend_from_slice(&v.to_le_bytes());
                        }
                        count += 1;
                    }
                }
            }
        }
        Self { vertices, count }
    }
}
#[derive(Clone, Copy)]
pub(super) struct View {
    pub camera: CameraMap,
    pub uv: bool,
}
pub(super) enum Command {
    Setup {
        id: u64,
        surface: Arc<Surface>,
        channel: usize,
        images: [AssetId<Image>; 9],
        scene: Arc<Scene>,
        object: u32,
        slot: u32,
    },
    Begin {
        id: u64,
        stroke: u64,
        brush: Brush,
        view: View,
    },
    Points {
        id: u64,
        points: Vec<Vec2>,
    },
    End {
        id: u64,
        stroke: u64,
    },
    Drop(u64),
}
pub(super) enum Reply {
    Layer {
        id: u64,
        stroke: u64,
        pixels: Result<Vec<u8>, String>,
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
struct State {
    pipelines: Option<Pipelines>,
    contexts: HashMap<u64, Context>,
}
pub(in crate::editor::ui::model_viewer) struct PluginGpu;
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
        | Command::End { id, .. } => *id,
        Command::Drop(id) => *id,
    }
}
struct Target {
    texture: Texture,
    view: TextureView,
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
    apply_layout: BindGroupLayout,
    blend_layout: BindGroupLayout,
    present_layout: BindGroupLayout,
    tiles_layout: BindGroupLayout,
    depth: RenderPipeline,
    uv: RenderPipeline,
    mask: RenderPipeline,
    apply: ComputePipeline,
    blend: ComputePipeline,
    present: ComputePipeline,
    present_full: ComputePipeline,
    tiles: ComputePipeline,
}
impl Pipelines {
    fn new(device: &RenderDevice) -> Self {
        let scene_layout = device.create_bind_group_layout("paint view", &[uniform(0)]);
        let mask_layout = device.create_bind_group_layout(
            "paint stamps",
            &[
                uniform(0),
                storage(1, true),
                texture(2, TextureSampleType::Depth),
                texture(3, TextureSampleType::Uint),
                texture(4, TextureSampleType::Uint),
                storage(5, false),
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
                        buffers: &[Some(RawVertexBufferLayout {
                            array_stride: 32,
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
                            ],
                        })],
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
        let uv = make_render(
            "paint UV ids",
            &scene_layout,
            "vs_uv",
            "fs_uv",
            TextureFormat::R32Uint,
            None,
            None,
        );
        let over = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        };
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
        let tiles = make_compute(
            "paint dirty tiles",
            &tiles_layout,
            include_str!("model_paint_gpu_tiles.wgsl"),
            "compact",
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
            apply_layout,
            blend_layout,
            present_layout,
            tiles_layout,
            depth,
            uv,
            mask,
            apply,
            blend,
            present,
            present_full,
            tiles,
        }
    }
}
struct LayerBuffer {
    buffer: Buffer,
    opacity: f32,
    visible: bool,
}
struct Composition {
    base: Buffer,
    result: Buffer,
    below: Buffer,
    layers: Vec<LayerBuffer>,
}
struct Context {
    size: u32,
    channel: usize,
    selected: usize,
    images: [AssetId<Image>; 9],
    object: u32,
    slot: u32,
    vertices: Buffer,
    count: u32,
    mask: Target,
    uv_ids: Target,
    depth: Target,
    scene_ids: Target,
    outputs: [Target; 2],
    composition: Composition,
    auxiliary: Buffer,
    snapshot: Buffer,
    params: Option<(Brush, View)>,
    dirty: Buffer,
    tiles: Buffer,
    grid: Buffer,
    presented: bool,
}
fn composition(device: &RenderDevice, surface: &Surface, channel: usize) -> Composition {
    let bytes = (surface.size as u64).pow(2) * 4;
    let usage = BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST;
    let base = buffer(device, "paint material base", &surface.base[channel], usage);
    let result = empty(device, "paint composite", bytes, usage);
    let layers = surface
        .stack
        .layers
        .iter()
        .map(|layer| {
            let buffer = match &layer.channels[channel] {
                Some(data) => buffer(device, "paint layer", data, usage),
                None => empty(device, "empty paint layer", bytes, usage),
            };
            LayerBuffer {
                buffer,
                opacity: layer.opacity / 100.,
                visible: layer.visible && layer.kind != LayerKind::Folder,
            }
        })
        .collect();
    Composition {
        base,
        result,
        below: empty(device, "paint cached lower layers", bytes, usage),
        layers,
    }
}
fn blend(
    device: &RenderDevice,
    pipelines: &Pipelines,
    encoder: &mut CommandEncoder,
    composition: &Composition,
    size: u32,
    range: std::ops::Range<usize>,
    tiles: Option<&Buffer>,
) {
    if range.start == 0 && tiles.is_none() {
        encoder.copy_buffer_to_buffer(
            &composition.base,
            0,
            &composition.result,
            0,
            size as u64 * size as u64 * 4,
        );
    }
    let mut first = true;
    for layer in composition.layers[range].iter().filter(|l| l.visible) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&layer.opacity.to_le_bytes());
        bytes.extend_from_slice(&u32::from(tiles.is_some() && first).to_le_bytes());
        bytes.extend_from_slice(&u32::from(tiles.is_some() && size > 1024).to_le_bytes());
        first = false;
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
                    resource: layer.buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: composition.result.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: composition.below.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: tiles.unwrap_or(&composition.base).as_entire_binding(),
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
}
fn params(
    size: u32,
    channel: usize,
    object: u32,
    slot: u32,
    brush: Brush,
    view: View,
    count: u32,
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
            FOV,
            aspect,
            (c.height * 0.0001).max(c.epsilon * 0.01).max(0.000001),
        )
    };
    let vp = projection * c.inverse;
    let mut bytes = Vec::with_capacity(240);
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
            0.,
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
    count: u32,
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
    if count > 0 {
        pass.draw(0..count, 0..1);
    }
}
impl Context {
    fn new(
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        surface: &Surface,
        channel: usize,
        images: [AssetId<Image>; 9],
        scene: &Scene,
        object: u32,
        slot: u32,
    ) -> Self {
        let size = surface.size;
        let rt = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
        let composition = composition(device, surface, channel);
        let auxiliary = if channel == 3 || channel == 6 {
            let aux = composition_for_aux(device, surface, channel);
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
        blend(
            device,
            pipelines,
            encoder,
            &composition,
            size,
            0..surface.stack.selected,
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
            surface.stack.selected..composition.layers.len(),
            None,
        );
        let columns = size.div_ceil(8);
        let tile_count = columns * columns;
        let mut grid_bytes = Vec::new();
        for v in [columns, columns, u32::from(channel == 3), 0] {
            grid_bytes.extend_from_slice(&v.to_le_bytes());
        }
        Self {
            size,
            channel,
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
            uv_ids: target(
                device,
                "paint UV faces",
                size,
                size,
                TextureFormat::R32Uint,
                rt,
            ),
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
            outputs: std::array::from_fn(|_| {
                target(
                    device,
                    "paint display",
                    size,
                    size,
                    TextureFormat::Rgba8Unorm,
                    TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
                )
            }),
            snapshot: empty(
                device,
                "paint stroke snapshot",
                size as u64 * size as u64 * 4,
                BufferUsages::COPY_DST | BufferUsages::STORAGE,
            ),
            composition,
            auxiliary,
            params: None,
            dirty: empty(
                device,
                "paint touched tile flags",
                tile_count as u64 * 4,
                BufferUsages::STORAGE | BufferUsages::COPY_DST,
            ),
            tiles: empty(
                device,
                "paint indirect tiles",
                (tile_count as u64 + 4) * 4,
                BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
            ),
            grid: buffer(
                device,
                "paint tile grid",
                &grid_bytes,
                BufferUsages::UNIFORM,
            ),
            presented: false,
        }
    }
    fn begin(
        &mut self,
        device: &RenderDevice,
        pipelines: &Pipelines,
        encoder: &mut CommandEncoder,
        brush: Brush,
        view: View,
    ) {
        self.params = Some((brush, view));
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
            &self.composition.layers[self.selected].buffer,
            0,
            &self.snapshot,
            0,
            self.size as u64 * self.size as u64 * 4,
        );
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
        raster(
            encoder,
            "paint scene depth",
            &pipelines.depth,
            &bg,
            &self.vertices,
            self.count,
            &self.scene_ids.view,
            Some(&self.depth.view),
            true,
        );
        raster(
            encoder,
            "paint UV faces",
            &pipelines.uv,
            &bg,
            &self.vertices,
            self.count,
            &self.uv_ids.view,
            None,
            true,
        );
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
        points: &[Vec2],
        images: &RenderAssets<GpuImage>,
    ) {
        let Some((brush, view)) = self.params else {
            return;
        };
        // At 1K, a single dense cached-layer pass is cheaper than compaction
        // and indirect dispatch. Larger textures benefit from sparse UV tiles.
        if self.size > 1024 {
            encoder.clear_buffer(&self.dirty, 0, None);
            encoder.clear_buffer(&self.tiles, 0, None);
        }
        let u = buffer(
            device,
            "paint brush",
            &params(
                self.size,
                self.channel,
                self.object,
                self.slot,
                brush,
                view,
                points.len() as u32,
            ),
            BufferUsages::UNIFORM,
        );
        let mut bytes = Vec::with_capacity(points.len() * 16);
        for p in points {
            for f in [p.x, p.y, 0., 0.] {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
        }
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
                    binding: 4,
                    resource: BindingResource::TextureView(&self.uv_ids.view),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: self.dirty.as_entire_binding(),
                },
            ],
        );
        raster(
            encoder,
            "paint stamps",
            &pipelines.mask,
            &mask,
            &self.vertices,
            self.count,
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
        let apply = device.create_bind_group(
            "paint apply",
            &pipelines.apply_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: u.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&self.mask.view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: self.snapshot.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: self.composition.layers[self.selected]
                        .buffer
                        .as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: self.tiles.as_entire_binding(),
                },
            ],
        );
        compute_tiles(
            encoder,
            "paint apply",
            &pipelines.apply,
            &apply,
            &self.tiles,
            self.size,
        );
        blend(
            device,
            pipelines,
            encoder,
            &self.composition,
            self.size,
            self.selected..self.composition.layers.len(),
            Some(&self.tiles),
        );
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
                    resource: self.composition.result.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: self.auxiliary.as_entire_binding(),
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
            ],
        );
        if self.presented {
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
            self.presented = true;
        }
        let destinations: &[usize] = match self.channel {
            0 => &[0],
            1 => &[1, 4],
            2 => &[1, 5],
            3 => &[2, 6],
            4 => &[0, 7],
            5 => &[3],
            6 => &[2],
            _ => &[8],
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
fn composition_for_aux(device: &RenderDevice, surface: &Surface, channel: usize) -> Composition {
    composition(device, surface, if channel == 3 { 6 } else { 3 })
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
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("model paint GPU"),
        });
        let mut readback = None;
        match command {
            Command::Setup {
                id,
                surface,
                channel,
                images,
                scene,
                object,
                slot,
            } => {
                contexts.insert(
                    id,
                    Context::new(
                        &device,
                        pipelines,
                        &mut encoder,
                        &surface,
                        channel,
                        images,
                        &scene,
                        object,
                        slot,
                    ),
                );
            }
            Command::Begin {
                id,
                stroke,
                brush,
                view,
            } => {
                let _ = stroke;
                if let Some(context) = contexts.get_mut(&id) {
                    context.begin(&device, pipelines, &mut encoder, brush, view);
                }
            }
            Command::Points { id, points } => {
                if !points.is_empty() {
                    if let Some(context) = contexts.get_mut(&id) {
                        context.points(&device, pipelines, &mut encoder, &points, &images);
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
                    let staging = empty(
                        &device,
                        "paint layer readback",
                        bytes,
                        BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    );
                    encoder.copy_buffer_to_buffer(
                        &context.composition.layers[context.selected].buffer,
                        0,
                        &staging,
                        0,
                        bytes,
                    );
                    readback = Some((id, stroke, staging));
                    context.params = None;
                    pending.stats.readbacks.fetch_add(1, Ordering::Relaxed);
                }
            }
            Command::Drop(id) => {
                contexts.remove(&id);
            }
        }
        queue.submit([encoder.finish()]);
        if let Some((id, stroke, staging)) = readback {
            let result = pending.replies.clone();
            let mapped = staging.clone();
            staging.slice(..).map_async(MapMode::Read, move |status| {
                let pixels = status
                    .map_err(|e| format!("GPU readback: {e}"))
                    .and_then(|()| {
                        let view = mapped
                            .slice(..)
                            .get_mapped_range()
                            .map_err(|e| e.to_string())?;
                        let pixels = view.to_vec();
                        drop(view);
                        mapped.unmap();
                        Ok(pixels)
                    });
                result
                    .lock()
                    .unwrap()
                    .push(Reply::Layer { id, stroke, pixels });
            });
        }
    }
}
