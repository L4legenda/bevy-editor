//! UV triangle ownership. Keep the CPU barycentric interpolation for exact map values.
use super::{Progress, RayDevice};
use crate::editor::model_mesh::UvBarycentric;
use bevy::{
    log::warn,
    prelude::*,
    render::render_resource::{
        BindGroupEntry, BindGroupLayoutEntry, BindingType, BufferBindingType, BufferDescriptor,
        BufferInitDescriptor, BufferUsages, CommandEncoderDescriptor, ComputePassDescriptor,
        MapMode, PipelineLayoutDescriptor, PollType, RawComputePipelineDescriptor,
        ShaderModuleDescriptor, ShaderSource, ShaderStages,
    },
};
use std::{sync::mpsc, time::Duration};

const TILE: u32 = 8;
const BUFFER_BYTES: usize = 8 * 1024 * 1024;
const EMPTY: u32 = u32::MAX;
const CHECK_CPU: u32 = u32::MAX - 1;
const TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct Triangle {
    pub face: usize,
    pub corners: [usize; 3],
    pub uv: [Vec2; 3],
    pub lo: UVec2,
    pub hi: UVec2,
    pub barycentric: Option<UvBarycentric>,
}

impl Triangle {
    pub fn new(face: usize, corners: [usize; 3], uv: [Vec2; 3], size: u32) -> Self {
        let extent = Vec2::splat(size as f32);
        Self {
            face,
            corners,
            uv,
            barycentric: UvBarycentric::new(uv),
            lo: uv[0]
                .min(uv[1])
                .min(uv[2])
                .floor()
                .clamp(Vec2::ZERO, extent)
                .as_uvec2(),
            hi: uv[0]
                .max(uv[1])
                .max(uv[2])
                .ceil()
                .clamp(Vec2::ZERO, extent)
                .as_uvec2(),
        }
    }

    pub fn weights(&self, center: Vec2) -> Option<Vec3> {
        let weights = self.barycentric?.at(center);
        (weights.min_element() >= -1e-5).then_some(weights)
    }

    fn tiles(&self, pitch: u32) -> impl Iterator<Item = usize> {
        (self.lo.y / TILE..self.hi.y.div_ceil(TILE)).flat_map(move |y| {
            (self.lo.x / TILE..self.hi.x.div_ceil(TILE)).map(move |x| (y * pitch + x) as usize)
        })
    }
}

fn bins(
    triangles: &[Triangle],
    pitch: u32,
    limit: usize,
    progress: &Progress,
) -> Result<Option<Vec<u32>>, String> {
    let headers = (pitch * pitch + 1) as usize;
    if headers > limit {
        return Ok(None);
    }
    let mut counts = vec![0u32; headers];
    let mut len = headers;
    for triangle in triangles {
        progress.check()?;
        if !triangle
            .uv
            .iter()
            .all(|uv| uv.is_finite() && uv.abs().max_element() < 1e10)
        {
            return Ok(None);
        }
        for tile in triangle.tiles(pitch) {
            counts[tile] += 1;
            len += 1;
            if len > limit {
                return Ok(None);
            }
        }
    }
    let mut offset = headers as u32;
    let mut bins = Vec::with_capacity(len);
    for count in &mut counts {
        bins.push(offset);
        offset += *count;
        *count = 0;
    }
    bins.resize(len, 0);
    for (index, triangle) in triangles.iter().enumerate() {
        progress.check()?;
        for tile in triangle.tiles(pitch) {
            let pointer = (bins[tile] + counts[tile]) as usize;
            bins[pointer] = index as u32;
            counts[tile] += 1;
        }
    }
    Ok(Some(bins))
}

pub(super) fn pixels(
    gpu: &RayDevice,
    size: u32,
    triangles: &[Triangle],
    progress: &Progress,
) -> Result<Option<Vec<Vec<u32>>>, String> {
    match compute(gpu, size, triangles, progress) {
        Ok(raster) => Ok(raster),
        Err(error) => {
            progress.check()?;
            warn!("GPU bake UV raster failed; retrying on the CPU: {error}");
            Ok(None)
        }
    }
}

fn compute(
    gpu: &RayDevice,
    size: u32,
    triangles: &[Triangle],
    progress: &Progress,
) -> Result<Option<Vec<Vec<u32>>>, String> {
    progress.check()?;
    let device = &gpu.device;
    let caps = device.limits();
    let tiles = size.div_ceil(TILE);
    let limit = u64::from(caps.max_storage_buffer_binding_size).min(caps.max_buffer_size) as usize;
    let rows = size
        .min((limit.min(BUFFER_BYTES) / (size.max(1) as usize * 4)) as u32)
        .min(caps.max_compute_workgroups_per_dimension * TILE);
    if rows == 0
        || triangles.is_empty()
        || triangles.len() > limit / 32
        || caps.max_storage_buffers_per_shader_stage < 3
        || caps.max_compute_invocations_per_workgroup < TILE * TILE
        || caps.max_compute_workgroup_size_x < TILE
        || caps.max_compute_workgroup_size_y < TILE
        || tiles > caps.max_compute_workgroups_per_dimension
    {
        return Ok(None);
    }
    let Some(bins) = bins(triangles, tiles, limit.min(64 * 1024 * 1024) / 4, progress)? else {
        return Ok(None);
    };
    let upload = |label, bytes: &_, usage| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage,
        })
    };
    let triangle_bytes: Vec<_> = triangles
        .iter()
        .flat_map(|triangle| {
            triangle
                .uv
                .into_iter()
                .chain([Vec2::ZERO])
                .flat_map(|uv| uv.to_array().map(f32::to_le_bytes))
        })
        .collect();
    let triangle_buffer = upload(
        "Bake UV triangles",
        triangle_bytes.as_flattened(),
        BufferUsages::STORAGE,
    );
    drop(triangle_bytes);
    let bin_bytes: Vec<_> = bins.iter().map(|word| word.to_le_bytes()).collect();
    let bin_buffer = upload(
        "Bake UV bins",
        bin_bytes.as_flattened(),
        BufferUsages::STORAGE,
    );
    drop(bin_bytes);
    let buffer = |label, size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let bytes = u64::from(size) * u64::from(rows) * 4;
    let output = buffer(
        "Bake UV owners",
        bytes,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    );
    let staging = buffer(
        "Bake UV ownership readback",
        bytes,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    let uniform = buffer(
        "Bake UV raster band",
        16,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    );
    let entries = [0, 1, 2, 3].map(|binding| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: if binding == 3 {
                BufferBindingType::Uniform
            } else {
                BufferBindingType::Storage {
                    read_only: binding < 2,
                }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    });
    let layout = device.create_bind_group_layout("Bake UV raster", &entries);
    let resources = [&triangle_buffer, &bin_buffer, &output, &uniform];
    let bindings = device.create_bind_group(
        "Bake UV raster",
        &layout,
        &std::array::from_fn::<_, 4, _>(|i| BindGroupEntry {
            binding: i as u32,
            resource: resources[i].as_entire_binding(),
        }),
    );
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Bake UV raster"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("Bake UV raster"),
        source: ShaderSource::Wgsl(include_str!("model_paint_bake_raster.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("Bake UV raster"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("raster_uv"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut pixels = vec![Vec::new(); triangles.len()];
    for first in (0..size).step_by(rows as usize) {
        progress.check()?;
        let height = (size - first).min(rows);
        gpu.queue.write_buffer(
            &uniform,
            0,
            [size, tiles, first, height]
                .map(u32::to_le_bytes)
                .as_flattened(),
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Bake UV raster"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("Bake UV raster"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &*bindings, &[]);
            pass.dispatch_workgroups(tiles, height.div_ceil(TILE), 1);
        }
        let bytes = u64::from(size) * u64::from(height) * 4;
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes);
        let submission = gpu.queue.submit([encoder.finish()]);
        let (send, received) = mpsc::sync_channel(1);
        let slice = staging.slice(..bytes);
        slice.map_async(MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(TIMEOUT),
            })
            .map_err(|error| error.to_string())?;
        received
            .recv_timeout(TIMEOUT)
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        {
            let data = slice
                .get_mapped_range()
                .map_err(|error| error.to_string())?;
            for (index, word) in data.chunks_exact(4).enumerate() {
                let pixel = first * size + index as u32;
                let owner = u32::from_le_bytes(word.try_into().unwrap());
                let owner = if owner == CHECK_CPU {
                    let xy = UVec2::new(pixel % size, pixel / size);
                    let tile = ((xy.y / TILE) * tiles + xy.x / TILE) as usize;
                    let candidates = &bins[bins[tile] as usize..bins[tile + 1] as usize];
                    candidates
                        .iter()
                        .copied()
                        .find(|&i| {
                            let t = &triangles[i as usize];
                            xy.cmpge(t.lo).all()
                                && xy.cmplt(t.hi).all()
                                && t.weights(xy.as_vec2() + 0.5).is_some()
                        })
                        .unwrap_or(EMPTY)
                } else {
                    owner
                };
                if owner != EMPTY {
                    pixels[owner as usize].push(pixel);
                }
            }
        }
        staging.unmap();
    }
    progress.check()?;
    Ok(Some(pixels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::renderer::{RenderDevice, RenderQueue};

    #[test]
    #[ignore = "requires a GPU; compares per-texel ownership with CPU coverage"]
    fn gpu_baked_uv_raster_matches_cpu_at_seams_and_overlaps() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let gpu = RayDevice {
            device: app.world().resource::<RenderDevice>().clone(),
            queue: app.world().resource::<RenderQueue>().clone(),
        };
        let size = 79;
        let triangles: Vec<_> = [
            [[0., 0.], [64., 0.], [64., 64.]],
            [[0., 64.], [64., 64.], [0., 0.]],
            [[-2.5, 40.], [70., 1.], [76., 81.]],
            [[0., 78.], [80., 77.9999], [40., 77.99996]],
            [[74., 3.], [78., 11.], [76., 7.]],
            [[76., -1.], [79., 3.], [77., 17.]],
        ]
        .into_iter()
        .enumerate()
        .map(|(index, uv)| Triangle::new(index, [0, 1, 2], uv.map(Vec2::from_array), size))
        .collect();
        let mut expected = vec![Vec::new(); triangles.len()];
        for y in 0..size {
            for x in 0..size {
                let xy = UVec2::new(x, y);
                let center = xy.as_vec2() + 0.5;
                if let Some(owner) = triangles.iter().position(|t| {
                    xy.cmpge(t.lo).all() && xy.cmplt(t.hi).all() && t.weights(center).is_some()
                }) {
                    expected[owner].push(y * size + x);
                }
            }
        }
        assert_eq!(
            compute(&gpu, size, &triangles, &Progress::default())
                .unwrap()
                .unwrap(),
            expected
        );
        let cancelled = Progress::default();
        cancelled
            .cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(compute(&gpu, size, &triangles, &cancelled).is_err());
    }
}
