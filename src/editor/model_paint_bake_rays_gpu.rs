//! Bounded compute batches for the visibility maps of a single bake.
use super::*;
use bevy::render::{
    render_resource::{
        BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingType, Buffer, BufferAsyncError,
        BufferBindingType, BufferDescriptor, BufferInitDescriptor, BufferUsages,
        CommandEncoderDescriptor, ComputePassDescriptor, ComputePipeline, MapMode,
        PipelineLayoutDescriptor, PollType, RawComputePipelineDescriptor, ShaderModule, ShaderModuleDescriptor,
        ShaderSource, ShaderStages,
    },
    renderer::{RenderDevice, RenderQueue},
};
use std::{collections::VecDeque, sync::{Mutex, Weak, mpsc}, time::Duration};

const NODE_BYTES: usize = 48;
const TEXEL_BYTES: usize = 64;
const PROJECTION_REPLY_BYTES: usize = 20;
const DIRECT_PROJECTION: u32 = 1 << 31;
const HEADER_BYTES: usize = 64;
const CONFIG_BYTES: usize = HEADER_BYTES + crate::editor::model_paint_data::custom::MAX_MAPS * 48;
const BASE_WORDS: usize = 2;
const MAX_IN_FLIGHT: usize = 3;
const CACHED_SCENES: usize = 2;
const CACHED_SCENE_BYTES: usize = 96 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(in crate::editor::ui::model_viewer::paint::bake) struct Device {
    pub device: RenderDevice,
    pub queue: RenderQueue,
    pipelines: Arc<Pipelines>,
    scenes: Arc<Mutex<VecDeque<CachedScene>>>,
}

impl Device {
    pub fn new(device: &RenderDevice, queue: &RenderQueue) -> Self {
        Self {
            device: device.clone(), queue: queue.clone(), pipelines: Arc::default(), scenes: Arc::default(),
        }
    }

    #[cfg(test)]
    pub fn with_fresh_scene_cache(&self) -> Self {
        Self { scenes: Arc::default(), ..self.clone() }
    }
}

struct CachedScene {
    relief: Weak<project::HighRelief>,
    triangle_vectors: usize,
    buffers: [Buffer; 2],
    bytes: usize,
}

impl Device {
    fn scene_buffers(
        &self, parts: &PartsOfScene, triangle_vectors: usize, bytes: usize,
        create: impl FnOnce() -> Option<[Buffer; 2]>,
    ) -> Option<[Buffer; 2]> {
        let PartsOfScene::High(relief, _) = parts else { return create() };
        if bytes > CACHED_SCENE_BYTES {
            return create();
        }
        // Relief belongs to one immutable source and supplies the part IDs in the buffers.
        let relief = Arc::downgrade(relief);
        let mut cache = self.scenes.lock().unwrap();
        cache.retain(|scene| scene.relief.strong_count() != 0);
        if let Some(index) = cache.iter().position(|scene| {
            scene.triangle_vectors == triangle_vectors && scene.relief.ptr_eq(&relief)
        }) {
            let scene = cache.remove(index).unwrap();
            let buffers = scene.buffers.clone();
            cache.push_back(scene);
            return Some(buffers);
        }
        let buffers = create()?;
        let mut stored_bytes: usize = cache.iter().map(|scene| scene.bytes).sum();
        while cache.len() == CACHED_SCENES || stored_bytes + bytes > CACHED_SCENE_BYTES {
            stored_bytes -= cache.pop_front().unwrap().bytes;
        }
        cache.push_back(CachedScene { relief, triangle_vectors, buffers: buffers.clone(), bytes });
        Some(buffers)
    }
}

#[derive(Clone, Copy)]
enum Pass {
    Maps,
    ResidentMaps,
    Projection,
}

#[derive(Default)]
struct Pipelines {
    shader: OnceLock<ShaderModule>,
    maps: OnceLock<Pipeline>,
    resident_maps: OnceLock<Pipeline>,
    projection: OnceLock<Pipeline>,
}

struct Pipeline {
    layout: BindGroupLayout,
    compute: ComputePipeline,
}

impl Pipelines {
    fn get(&self, device: &RenderDevice, pass: Pass) -> &Pipeline {
        let (slot, entry) = match pass {
            Pass::Maps => (&self.maps, "trace_maps"),
            Pass::ResidentMaps => (&self.resident_maps, "trace_resident_maps"),
            Pass::Projection => (&self.projection, "project_queries"),
        };
        slot.get_or_init(|| {
            let shader = self.shader.get_or_init(|| {
                device.create_and_validate_shader_module(ShaderModuleDescriptor {
                    label: Some("Bake visibility"),
                    source: ShaderSource::Wgsl(include_str!("model_paint_bake_rays.wgsl").into()),
                })
            });
            Pipeline::new(device, shader, entry, matches!(pass, Pass::ResidentMaps))
        })
    }
}

struct Dispatch {
    gpu: Device,
    pipeline: ComputePipeline,
    bindings: BindGroup,
    input: Buffer,
    output: Buffer,
    uniform: Buffer,
    staging: Vec<Buffer>,
}

struct Readback {
    slot: usize,
    bytes: u64,
    wait: PollType,
    done: Option<mpsc::Receiver<Result<(), BufferAsyncError>>>,
}

struct InFlight {
    readback: Readback,
    pixels: Vec<usize>,
}

pub(super) struct Batch {
    scene: [Buffer; 2],
    dispatch: Dispatch,
    limit: usize,
    capacity: usize,
    count: [u32; 3],
    scale: f32,
    restrict_part: bool,
    visibility: bool,
    resident: bool,
    recipe_count: usize,
    triangle_vectors: usize,
    config: Vec<u8>,
    stride: usize,
    pixels: Vec<usize>,
    encoded: Vec<u8>,
    pending: VecDeque<InFlight>,
    available: Vec<Vec<usize>>,
    next_slot: usize,
}

fn words(data: &mut Vec<u8>, values: [u32; 4]) {
    for value in values {
        data.extend_from_slice(&value.to_le_bytes());
    }
}

fn packed_vector(v: Vec3) -> [[u8; 4]; 4] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits(), 0].map(u32::to_le_bytes)
}

fn batch_limit(recipes: &[custom::Prepared], rays: &Rays, visibility: bool) -> usize {
    let sampling = rays.sampling;
    let maps = rays.maps;
    let visibility_cost = if visibility {
        sampling.occlusion
            * u32::from([BakeMap::Occlusion, BakeMap::BentNormal, BakeMap::Cavity]
                .into_iter().any(|map| maps.contains(map)))
            + sampling.thickness * u32::from(maps.contains(BakeMap::Thickness))
            + sampling.sky * u32::from(maps.contains(BakeMap::Sky))
    } else {
        0
    };
    let walk_cost = (sampling.sky / 8).clamp(2, 8) * 32 * 4;
    let recipe_cost: u32 = recipes
        .iter()
        .map(|recipe| {
            if recipe.is_flow() {
                walk_cost
            } else {
                sampling.sky.max(8)
            }
        })
        .sum();
    (1_000_000 / (visibility_cost + recipe_cost).max(1)).clamp(8, 8192) as usize
}

pub(super) enum PartsOfScene<'a> {
    Low(&'a Parts, usize),
    High(&'a Arc<project::HighRelief>, &'a project::Projection),
}

impl Batch {
    pub fn has_flow_normals(&self) -> bool {
        self.triangle_vectors == 7
    }

    pub fn project_batch(&self) -> Option<ProjectBatch> {
        let gpu = &self.dispatch.gpu;
        let caps = gpu.device.limits();
        let capacity = (caps.max_storage_buffer_binding_size as u64)
            .min(caps.max_buffer_size)
            .checked_div(TEXEL_BYTES as u64)?
            .min(4096) as usize;
        if capacity < 65 {
            return None;
        }
        Some(ProjectBatch {
            dispatch: Dispatch::new(
                gpu,
                &self.scene,
                capacity,
                PROJECTION_REPLY_BYTES,
                MAX_IN_FLIGHT,
                Pass::Projection,
                None,
            ),
            triangle_vectors: self.triangle_vectors,
            capacity,
            input: Vec::with_capacity(capacity * TEXEL_BYTES),
        })
    }

    pub fn new(
        gpu: &Device,
        rays: &Rays,
        parts: PartsOfScene,
        recipes: &[custom::Prepared],
        visibility: bool,
        atlas: Option<&Buffer>,
    ) -> Option<Self> {
        let device = &gpu.device;
        let caps = device.limits();
        let max_binding =
            (caps.max_storage_buffer_binding_size as u64).min(caps.max_buffer_size) as usize;
        let nodes = rays.scene.compute_nodes();
        let triangles = rays.scene.compute_triangles();
        let node_capacity = nodes.len().checked_mul(NODE_BYTES)?;
        let triangle_vectors = if rays.flow_normals.is_some() { 7 } else { 4 };
        let triangle_capacity = triangles.len().checked_mul(triangle_vectors * 16)?;
        let sampling = rays.sampling;
        let count = [sampling.occlusion, sampling.thickness, sampling.sky];
        let recipe_count = recipes.len();
        let resident = atlas.is_some();
        let stride = (if resident { 0 } else { BASE_WORDS } + recipe_count) * 4;
        let limit = batch_limit(recipes, rays, visibility);
        if nodes.len() == 0
            || (recipes.iter().any(custom::Prepared::is_flow) && rays.flow_normals.is_none())
            || caps.max_storage_buffers_per_shader_stage < 4
            || caps.max_compute_invocations_per_workgroup < 64
            || caps.max_compute_workgroup_size_x < 64
            || caps.max_uniform_buffer_binding_size < CONFIG_BYTES as u64
            || [
                node_capacity,
                triangle_capacity,
                limit * TEXEL_BYTES,
                limit * stride,
            ]
            .into_iter()
            .any(|size| size > max_binding)
        {
            return None;
        }
        let scene_bytes = node_capacity.checked_add(triangle_capacity)?;
        let scene = gpu.scene_buffers(&parts, triangle_vectors, scene_bytes, || {
            Self::upload_scene(device, rays, &parts, node_capacity, triangle_capacity)
        })?;
        let slots = if visibility || recipe_count > 0 { MAX_IN_FLIGHT } else { 1 };
        let pass = if resident { Pass::ResidentMaps } else { Pass::Maps };
        let dispatch = Dispatch::new(gpu, &scene, limit, stride, slots, pass, atlas);
        let mut batch = Self {
            scene,
            dispatch,
            limit,
            capacity: limit,
            count,
            scale: rays.scale,
            restrict_part: matches!(parts, PartsOfScene::Low(..)),
            visibility,
            resident,
            recipe_count,
            triangle_vectors,
            config: Vec::with_capacity(CONFIG_BYTES),
            stride,
            pixels: Vec::with_capacity(limit),
            encoded: Vec::with_capacity(limit * TEXEL_BYTES),
            pending: VecDeque::with_capacity(slots),
            available: Vec::with_capacity(slots),
            next_slot: 0,
        };
        batch.configure(recipes, rays);
        Some(batch)
    }

    fn upload_scene(
        device: &RenderDevice, rays: &Rays, parts: &PartsOfScene,
        node_capacity: usize, triangle_capacity: usize,
    ) -> Option<[Buffer; 2]> {
        let mut node_data = Vec::with_capacity(node_capacity / NODE_BYTES);
        for ([lo, hi], links) in rays.scene.compute_nodes() {
            node_data.push([packed_vector(lo), packed_vector(hi), links.map(u32::to_le_bytes)]);
        }
        let mut triangle_data = Vec::with_capacity(triangle_capacity / 16);
        for (index, triangle) in rays.scene.compute_triangles() {
            let face = u32::try_from(triangle.face).ok()?;
            if matches!(parts, PartsOfScene::High(..)) && face >= DIRECT_PROJECTION - 1 {
                return None;
            }
            let part = match parts {
                PartsOfScene::Low(parts, target) if triangle.object == *target => {
                    parts.faces[triangle.face]
                }
                PartsOfScene::Low(..) => 0,
                PartsOfScene::High(parts, _) => parts.part(triangle.face),
            };
            let ids = [
                u32::try_from(triangle.object).ok()?,
                face,
                part as u32,
                u32::from(
                    matches!(parts, PartsOfScene::High(_, projector) if projector.accepts_projection(triangle)),
                ),
            ];
            let [a, b, c] = triangle.points.map(packed_vector);
            triangle_data.extend_from_slice(&[a, b, c, ids.map(u32::to_le_bytes)]);
            if let Some(normals) = &rays.flow_normals {
                triangle_data.extend_from_slice(&normals.corners(index).map(packed_vector));
            }
        }
        let storage = BufferUsages::STORAGE;
        let from_bytes = |label, contents: &[u8]| {
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: storage,
            })
        };
        let nodes = from_bytes("Bake BVH nodes", node_data.as_flattened().as_flattened().as_flattened());
        let triangles = from_bytes("Bake BVH triangles", triangle_data.as_flattened().as_flattened());
        Some([nodes, triangles])
    }

    pub fn configure(&mut self, recipes: &[custom::Prepared], rays: &Rays) {
        assert_eq!(self.recipe_count, recipes.len());
        let sampling = rays.sampling;
        self.limit = self
            .capacity
            .min(batch_limit(recipes, rays, self.visibility));
        self.count = [sampling.occlusion, sampling.thickness, sampling.sky];
        self.config.clear();
        // The first two vectors are updated on dispatch.
        self.config.resize(32, 0);
        words(
            &mut self.config,
            [
                (rays.resolution as f32).to_bits(),
                self.triangle_vectors as u32,
                u32::from(rays.maps.bits()),
                0,
            ],
        );
        words(&mut self.config, sampling.visibility.map(f32::to_bits));
        for recipe in recipes {
            let custom::GpuRecipe {
                kind,
                direction,
                distance,
                point,
                cosine,
                strength,
                wander,
            } = recipe.gpu_parameters();
            words(
                &mut self.config,
                [
                    direction.x.to_bits(),
                    direction.y.to_bits(),
                    direction.z.to_bits(),
                    distance.to_bits(),
                ],
            );
            words(
                &mut self.config,
                [
                    point.x.to_bits(),
                    point.y.to_bits(),
                    point.z.to_bits(),
                    cosine.to_bits(),
                ],
            );
            words(
                &mut self.config,
                [kind, strength.to_bits(), wander.to_bits(), 0],
            );
        }
    }

    pub fn push(&mut self, surface: RaySurface, pixel: usize, width: f32) {
        self.pixels.push(pixel);
        let p = surface.point;
        words(
            &mut self.encoded,
            [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), width.to_bits()],
        );
        for v in [surface.normal, surface.geometric] {
            words(&mut self.encoded, [v.x.to_bits(), v.y.to_bits(), v.z.to_bits(), 0]);
        }
        words(
            &mut self.encoded,
            [
                pixel as u32,
                surface.object as u32,
                surface.face as u32,
                surface.part as u32,
            ],
        );
    }

    pub fn full(&self) -> bool {
        self.pixels.len() == self.limit
    }

    fn read(
        &mut self,
        progress: &Progress,
        drain: bool,
        mut consume: impl FnMut(usize, &[u8]),
    ) -> Result<bool, String> {
        progress.check()?;
        let slots = self.dispatch.staging.len();
        #[cfg(test)]
        let slots = if progress.serial_gpu { 1 } else { slots };
        let mut read = false;
        if !self.pixels.is_empty() {
            if self.pending.len() == slots {
                read = self.retire(progress, &mut consume)?;
            }
            self.enqueue(slots);
        }
        if drain || slots == 1 {
            while !self.pending.is_empty() {
                read |= self.retire(progress, &mut consume)?;
            }
        }
        Ok(read)
    }

    fn enqueue(&mut self, slots: usize) {
        let len = self.pixels.len() as u32;
        let [ao, thickness, sky] = self.count;
        let params = [
            ao,
            thickness,
            sky,
            len,
            self.scale.to_bits(),
            f32::from(u8::from(self.restrict_part)).to_bits(),
            f32::from(u8::from(self.visibility)).to_bits(),
            (self.recipe_count as f32).to_bits(),
        ]
        .map(u32::to_le_bytes);
        self.config[..32].copy_from_slice(params.as_flattened());
        let slot = self.next_slot;
        let readback = self
            .dispatch
            .submit(slot, &self.encoded, &self.config, len, self.stride);
        self.next_slot = (slot + 1) % slots;
        self.encoded.clear();
        let next = self
            .available
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(self.capacity));
        let pixels = std::mem::replace(&mut self.pixels, next);
        self.pending.push_back(InFlight { readback, pixels });
    }

    fn retire(
        &mut self,
        progress: &Progress,
        mut consume: impl FnMut(usize, &[u8]),
    ) -> Result<bool, String> {
        progress.check()?;
        let Some(InFlight {
            readback,
            mut pixels,
        }) = self.pending.pop_front()
        else {
            return Ok(false);
        };
        #[cfg(test)]
        let started = progress.profile.then(std::time::Instant::now);
        #[cfg(test)]
        progress
            .visibility_readback_bytes
            .fetch_add(readback.bytes, Ordering::Relaxed);
        self.dispatch.read(readback, |bytes| {
            for (&pixel, result) in pixels.iter().zip(bytes.chunks_exact(self.stride)) {
                consume(pixel, result);
            }
        })?;
        #[cfg(test)]
        if let Some(started) = started {
            progress
                .ray_wait_ns
                .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        pixels.clear();
        self.available.push(pixels);
        progress.check()?;
        Ok(true)
    }

    pub fn flush(
        &mut self,
        maps: &mut Maps,
        progress: &Progress,
        drain: bool,
    ) -> Result<bool, String> {
        let planes = maps
            .custom
            .iter_mut()
            .map(|plane| Arc::get_mut(&mut plane.values).unwrap())
            .collect::<Vec<_>>();
        let mut planes = planes;
        let visibility = self.visibility && !self.resident;
        let custom_offset = if self.resident { 0 } else { BASE_WORDS * 4 };
        self.read(progress, drain, |pixel, result| {
            if visibility {
                maps.values[pixel][..4].copy_from_slice(&result[..4]);
                maps.values[pixel][6..9].copy_from_slice(&result[4..7]);
            }
            for (plane, value) in planes
                .iter_mut()
                .zip(result[custom_offset..].chunks_exact(4))
            {
                plane[pixel] = value[0];
            }
        })
    }

    pub fn flush_preview(
        &mut self,
        pixels: &mut [[u8; 4]],
        progress: &Progress,
        drain: bool,
    ) -> Result<(), String> {
        self.read(progress, drain, |pixel, result| {
            let value = result[8];
            pixels[pixel] = [value, value, value, 255];
        })?;
        Ok(())
    }
}

pub(super) struct ProjectBatch {
    dispatch: Dispatch,
    triangle_vectors: usize,
    pub capacity: usize,
    input: Vec<u8>,
}

pub(super) struct ProjectionReadback(Readback);

impl ProjectBatch {
    pub fn slots(&self, _progress: &Progress) -> usize {
        #[cfg(test)]
        if _progress.serial_projection {
            return 1;
        }
        self.dispatch.staging.len()
    }

    pub fn submit(
        &mut self,
        probes: &[aa::Probe],
        slot: usize,
        progress: &Progress,
    ) -> Result<ProjectionReadback, String> {
        assert!(!probes.is_empty() && probes.len() <= self.capacity);
        progress.check()?;
        self.input.clear();
        for probe in probes {
            let q = &probe.query;
            let p = q.point;
            let dir = q.segment.outward;
            let origin = q.segment.origin;
            words(
                &mut self.input,
                [
                    p.x.to_bits(),
                    p.y.to_bits(),
                    p.z.to_bits(),
                    q.segment.range.to_bits(),
                ],
            );
            words(
                &mut self.input,
                [
                    dir.x.to_bits(),
                    dir.y.to_bits(),
                    dir.z.to_bits(),
                    q.segment.front.to_bits(),
                ],
            );
            words(
                &mut self.input,
                [
                    origin.x.to_bits(),
                    origin.y.to_bits(),
                    origin.z.to_bits(),
                    q.segment.rear.to_bits(),
                ],
            );
            words(
                &mut self.input,
                [
                    q.normal.x.to_bits(),
                    q.normal.y.to_bits(),
                    q.normal.z.to_bits(),
                    u32::from(q.fill) | (u32::from(q.prefer_front_faces) << 1),
                ],
            );
        }
        let config = [
            0,
            0,
            0,
            probes.len() as u32,
            0,
            0,
            0,
            0,
            0,
            self.triangle_vectors as u32,
            0,
            0,
        ]
        .map(u32::to_le_bytes);
        Ok(ProjectionReadback(self.dispatch.submit(
            slot,
            &self.input,
            config.as_flattened(),
            probes.len() as u32,
            PROJECTION_REPLY_BYTES,
        )))
    }

    pub fn resolve(
        &self,
        projector: &project::Projection,
        readback: ProjectionReadback,
        results: &mut Vec<Option<project::Hit>>,
        progress: &Progress,
    ) -> Result<(), String> {
        progress.check()?;
        results.clear();
        #[cfg(test)]
        let started = progress.profile.then(std::time::Instant::now);
        #[cfg(test)]
        progress.projection_readback_bytes.fetch_add(readback.0.bytes, Ordering::Relaxed);
        self.dispatch.read(readback.0, |readback| {
            for reply in readback.chunks_exact(PROJECTION_REPLY_BYTES) {
                let word = |index: usize| {
                    u32::from_le_bytes(reply[index * 4..index * 4 + 4].try_into().unwrap())
                };
                let tag = word(0);
                results.push((tag != u32::MAX).then(|| {
                    let point = Vec3::from_array([word(1), word(2), word(3)].map(f32::from_bits));
                    projector.hit((tag & !DIRECT_PROJECTION) as usize, point, f32::from_bits(word(4)),
                        tag & DIRECT_PROJECTION != 0)
                }));
            }
        })?;
        #[cfg(test)]
        if let Some(started) = started {
            progress
                .projection_wait_ns
                .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        progress.check()?;
        Ok(())
    }
}

impl Pipeline {
    fn new(device: &RenderDevice, shader: &ShaderModule, entry: &str, resident: bool) -> Self {
        let layout_entries: Vec<_> = (0..5 + u32::from(resident))
            .map(|binding| BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: if binding == 4 {
                        BufferBindingType::Uniform
                    } else {
                        BufferBindingType::Storage {
                            read_only: binding < 3,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout("Bake rays layout", &layout_entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Bake ray pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let compute = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("Bake visibility"),
            layout: Some(&pipeline_layout),
            module: shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { layout, compute }
    }
}

impl Dispatch {
    fn new(
        gpu: &Device,
        scene: &[Buffer; 2],
        limit: usize,
        stride: usize,
        slots: usize,
        pass: Pass,
        atlas: Option<&Buffer>,
    ) -> Self {
        let device = &gpu.device;
        let storage = BufferUsages::STORAGE;
        let create = |label, size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let input = create(
            "Bake ray inputs",
            (limit * TEXEL_BYTES) as u64,
            storage | BufferUsages::COPY_DST,
        );
        let output = create(
            "Bake ray outputs",
            (limit * stride).max(4) as u64,
            storage | BufferUsages::COPY_SRC,
        );
        let staging = (0..slots)
            .map(|_| {
                create(
                    "Bake ray readback",
                    (limit * stride).max(4) as u64,
                    BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                )
            })
            .collect();
        let uniform = create(
            "Bake ray config",
            CONFIG_BYTES as u64,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let pipeline = gpu.pipelines.get(device, pass);
        let bindings = device.create_bind_group(
            "Bake rays bindings",
            &pipeline.layout,
            &[&scene[0], &scene[1], &input, &output, &uniform]
                .into_iter()
                .chain(atlas)
                .enumerate()
                .map(|(binding, buffer)| BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        );
        Self {
            gpu: gpu.clone(),
            pipeline: pipeline.compute.clone(),
            bindings,
            input,
            output,
            uniform,
            staging,
        }
    }

    fn submit(
        &self,
        slot: usize,
        input: &[u8],
        config: &[u8],
        len: u32,
        stride: usize,
    ) -> Readback {
        self.gpu.queue.write_buffer(&self.input, 0, input);
        self.gpu.queue.write_buffer(&self.uniform, 0, config);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Bake rays"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("Bake rays"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &*self.bindings, &[]);
            pass.dispatch_workgroups(len.div_ceil(64), 1, 1);
        }
        let bytes = u64::from(len) * stride as u64;
        let stage = &self.staging[slot];
        if bytes != 0 {
            encoder.copy_buffer_to_buffer(&self.output, 0, stage, 0, bytes);
        }
        let submission = self.gpu.queue.submit([encoder.finish()]);
        let done = (bytes != 0).then(|| {
            let (send, done) = mpsc::sync_channel(1);
            stage
                .slice(..bytes)
                .map_async(MapMode::Read, move |result| {
                    let _ = send.send(result);
                });
            done
        });
        Readback {
            slot,
            bytes,
            wait: PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(TIMEOUT),
            },
            done,
        }
    }

    fn read(&self, readback: Readback, consume: impl FnOnce(&[u8])) -> Result<(), String> {
        self.gpu
            .device
            .poll(readback.wait)
            .map_err(|error| format!("GPU bake: {error}"))?;
        let Some(done) = readback.done else {
            return Ok(());
        };
        done.recv_timeout(TIMEOUT)
            .map_err(|error| format!("GPU bake readback: {error}"))?
            .map_err(|error| format!("GPU bake readback: {error}"))?;
        let stage = &self.staging[readback.slot];
        if let Ok(readback) = stage.slice(..readback.bytes).get_mapped_range() {
            consume(&readback);
        } else {
            stage.unmap();
            return Err("GPU bake: staging data is unavailable".into());
        }
        stage.unmap();
        Ok(())
    }
}
