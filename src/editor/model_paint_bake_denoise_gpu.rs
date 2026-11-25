//! GPU filtering in horizontal bands, with enough overlap for all passes.
use super::super::RayDevice;
use super::{CURVATURE_PASSES, Curvature, DENOISE_END, Guide, Maps, Progress, RASTER_END};
use bevy::render::render_resource::{
    BindGroupEntry, BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferDescriptor,
    BufferUsages, CommandEncoderDescriptor, ComputePassDescriptor, MapMode,
    PipelineLayoutDescriptor, PollType, RawComputePipelineDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages,
};
use std::{
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};

const GUIDE_BYTES: usize = 28;
const CHANNEL_BYTES: usize = 8;
const BAND_BYTES: usize = 32 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

pub(in crate::editor::ui::model_viewer::paint::bake) fn visibility_atlas(
    gpu: &RayDevice,
    size: u32,
    passes: u32,
) -> Option<Buffer> {
    band_rows(gpu, size, passes, BAND_BYTES)?;
    let caps = gpu.device.limits();
    let bytes = u64::from(size).pow(2) * CHANNEL_BYTES as u64;
    if caps.max_storage_buffers_per_shader_stage < 5
        || bytes > u64::from(caps.max_storage_buffer_binding_size).min(caps.max_buffer_size)
    {
        return None;
    }
    Some(gpu.device.create_buffer(&BufferDescriptor {
        label: Some("Bake resident visibility atlas"),
        size: bytes,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    }))
}

fn band_rows(
    gpu: &RayDevice,
    size: u32,
    passes: u32,
    bytes: usize,
) -> Option<(usize, usize, usize)> {
    let caps = gpu.device.limits();
    if size == 0
        || passes == 0
        || caps.max_storage_buffers_per_shader_stage < 3
        || caps.max_uniform_buffer_binding_size < 16
        || caps.max_compute_invocations_per_workgroup < 64
        || caps.max_compute_workgroup_size_x < 8
        || caps.max_compute_workgroup_size_y < 8
        || size.div_ceil(8) > caps.max_compute_workgroups_per_dimension
    {
        return None;
    }
    let overlap = 2 * ((1usize << passes) - 1);
    let size = size as usize;
    let limit = u64::from(caps.max_storage_buffer_binding_size).min(caps.max_buffer_size);
    let rows = size
        .min(caps.max_compute_workgroups_per_dimension as usize * 8)
        .min((limit as usize).min(bytes) / (size * GUIDE_BYTES));
    let interior = if rows == size {
        rows
    } else {
        rows.saturating_sub(overlap * 2)
    };
    (interior != 0).then_some((rows, interior, overlap))
}

pub(super) fn filter(
    gpu: &RayDevice,
    maps: &mut Maps,
    guides: &[Guide],
    passes: u32,
    progress: &Progress,
    atlas: Option<&Buffer>,
) -> Result<bool, String> {
    filter_bands(gpu, maps, guides, passes, progress, BAND_BYTES, atlas)
}

pub(super) fn filter_bands(
    gpu: &RayDevice,
    maps: &mut Maps,
    guides: &[Guide],
    passes: u32,
    progress: &Progress,
    bytes: usize,
    atlas: Option<&Buffer>,
) -> Result<bool, String> {
    filter_pass(gpu, maps, guides, passes, progress, bytes, Filter::Visibility(atlas))
}

pub(super) fn filter_curvature(
    gpu: &RayDevice,
    maps: &mut Maps,
    guides: &[Guide],
    samples: &[Curvature],
    radius: f32,
    progress: &Progress,
) -> Result<bool, String> {
    filter_pass(gpu, maps, guides, CURVATURE_PASSES, progress, BAND_BYTES, Filter::Curvature(samples, radius))
}

#[derive(Clone, Copy)]
enum Filter<'a> {
    Visibility(Option<&'a Buffer>),
    Curvature(&'a [Curvature], f32),
}

fn filter_pass(
    gpu: &RayDevice,
    maps: &mut Maps,
    guides: &[Guide],
    passes: u32,
    progress: &Progress,
    bytes: usize,
    filter: Filter,
) -> Result<bool, String> {
    let atlas = match filter {
        Filter::Visibility(atlas) => atlas,
        Filter::Curvature(..) => None,
    };
    if atlas.is_none() && !guides.iter().any(|guide| guide.texel > 0.0) {
        return Ok(false);
    }
    progress.check()?;
    let device = &gpu.device;
    let size = maps.size as usize;
    let Some((rows, interior, overlap)) = band_rows(gpu, maps.size, passes, bytes) else {
        return Ok(false);
    };
    let storage = BufferUsages::STORAGE;
    let capacity = rows * size;
    let buffer = |label, size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: size as u64,
            usage,
            mapped_at_creation: false,
        })
    };
    let shape = buffer(
        "Bake denoise guides",
        capacity * GUIDE_BYTES,
        storage | BufferUsages::COPY_DST,
    );
    let channels = ["Bake denoise channels A", "Bake denoise channels B"].map(|label| {
        buffer(
            label,
            capacity * CHANNEL_BYTES,
            storage | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
        )
    });
    let staging = buffer(
        "Bake denoise readback",
        interior * size * CHANNEL_BYTES,
        BufferUsages::COPY_DST | BufferUsages::MAP_READ,
    );
    let uniform = buffer(
        "Bake denoise settings",
        16,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    );
    let layout_entries = [0, 1, 2, 3].map(|binding| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: if binding == 3 {
                BufferBindingType::Uniform
            } else {
                BufferBindingType::Storage {
                    read_only: binding != 2,
                }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    });
    let layout = device.create_bind_group_layout("Bake denoise", &layout_entries);
    let bindings = [0, 1].map(|i| {
        let entries = [&shape, &channels[i], &channels[1 - i], &uniform]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        device.create_bind_group("Bake denoise", &layout, &entries)
    });
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Bake denoise"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("Bake denoise"),
        source: ShaderSource::Wgsl(include_str!("model_paint_bake_denoise.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("Bake denoise"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some(match filter {
            Filter::Visibility(_) => "denoise",
            Filter::Curvature(..) => "smooth_curvature",
        }),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut shape_bytes = Vec::with_capacity(capacity * GUIDE_BYTES);
    let mut channel_bytes = Vec::new();
    // Commit after all bands have completed so a failed dispatch can retry on the CPU.
    let mut filtered = vec![[0; CHANNEL_BYTES]; guides.len()];
    for y in (0..size).step_by(interior) {
        progress.check()?;
        let end = (y + interior).min(size);
        let first_row = y.saturating_sub(overlap);
        let last_row = (end + overlap).min(size);
        let source = first_row * size..last_row * size;
        let destination = y * size..end * size;
        let occupied = if atlas.is_some() {
            maps.covered[destination.clone()].contains(&1)
        } else {
            guides[destination.clone()]
                .iter()
                .any(|guide| guide.texel > 0.0)
        };
        if !occupied {
            continue;
        }
        shape_bytes.clear();
        channel_bytes.clear();
        for (index, (guide, value)) in guides[source.clone()]
            .iter()
            .zip(&maps.values[source.clone()])
            .enumerate()
        {
            for coordinate in guide.position.to_array().into_iter().chain([guide.texel]) {
                shape_bytes.extend_from_slice(&coordinate.to_le_bytes());
            }
            shape_bytes.extend_from_slice(&guide.part.to_le_bytes());
            shape_bytes.extend_from_slice(&guide.island.to_le_bytes());
            shape_bytes.extend_from_slice(&[value[9], value[10], value[11], 0]);
            match filter {
                Filter::Visibility(None) => {
                    channel_bytes.extend_from_slice(&value[..4]);
                    channel_bytes.extend_from_slice(&[value[6], value[7], value[8], 0]);
                }
                Filter::Curvature(samples, _) => {
                    channel_bytes.extend_from_slice(&samples[source.start + index].value.to_le_bytes());
                    channel_bytes.extend_from_slice(&[0; 4]);
                }
                Filter::Visibility(Some(_)) => {}
            }
        }
        gpu.queue.write_buffer(&shape, 0, &shape_bytes);
        if atlas.is_none() {
            gpu.queue.write_buffer(&channels[0], 0, &channel_bytes);
        }
        let height = (last_row - first_row) as u32;
        let offset = ((y - first_row) * size * CHANNEL_BYTES) as u64;
        let count = (destination.len() * CHANNEL_BYTES) as u64;
        for pass in 0..passes {
            progress.check()?;
            let radius = match filter {
                Filter::Visibility(_) => 0.,
                Filter::Curvature(_, radius) => radius,
            };
            let config = [maps.size, height, 1 << pass, radius.to_bits()].map(u32::to_le_bytes);
            gpu.queue.write_buffer(&uniform, 0, config.as_flattened());
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Bake denoise"),
            });
            if pass == 0
                && let Some(atlas) = atlas
            {
                encoder.copy_buffer_to_buffer(
                    atlas,
                    (source.start * CHANNEL_BYTES) as u64,
                    &channels[0],
                    0,
                    (source.len() * CHANNEL_BYTES) as u64,
                );
            }
            let input = (pass % 2) as usize;
            {
                let mut compute = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("Bake denoise"),
                    timestamp_writes: None,
                });
                compute.set_pipeline(&pipeline);
                compute.set_bind_group(0, &*bindings[input], &[]);
                compute.dispatch_workgroups(maps.size.div_ceil(8), height.div_ceil(8), 1);
            }
            let last = pass + 1 == passes;
            if last {
                encoder.copy_buffer_to_buffer(&channels[1 - input], offset, &staging, 0, count);
            }
            let submission = gpu.queue.submit([encoder.finish()]);
            if !last {
                continue;
            }
            let (send, received) = mpsc::sync_channel(1);
            let slice = staging.slice(..count);
            slice.map_async(MapMode::Read, move |result| {
                let _ = send.send(result);
            });
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(TIMEOUT),
                })
                .map_err(|error| format!("GPU bake denoise: {error}"))?;
            received
                .recv_timeout(TIMEOUT)
                .map_err(|error| format!("GPU bake denoise readback: {error}"))?
                .map_err(|error| format!("GPU bake denoise readback: {error}"))?;
            {
                let data = slice
                    .get_mapped_range()
                    .map_err(|error| format!("GPU bake denoise readback: {error}"))?;
                for (texel, bytes) in filtered[destination.clone()]
                    .iter_mut()
                    .zip(data.chunks_exact(CHANNEL_BYTES))
                {
                    texel.copy_from_slice(bytes);
                }
            }
            staging.unmap();
        }
        if matches!(filter, Filter::Visibility(_)) {
            progress.rows.store(
                RASTER_END + (DENOISE_END - RASTER_END) * end as u32 / maps.size,
                Ordering::Relaxed,
            );
        }
    }
    progress.check()?;
    match filter {
        Filter::Visibility(_) => {
            let write_bent = maps.selection.contains(super::super::BakeMap::BentNormal);
            for ((value, texel), guide) in maps.values.iter_mut().zip(filtered).zip(guides) {
                if guide.texel > 0.0 || texel[7] == 255 {
                    value[..4].copy_from_slice(&texel[..4]);
                    if write_bent {
                        value[6..9].copy_from_slice(&texel[4..7]);
                    }
                }
            }
        }
        Filter::Curvature(samples, _) => {
            for (((value, texel), sample), guide) in
                maps.values.iter_mut().zip(filtered).zip(samples).zip(guides)
            {
                let curvature = if guide.texel > 0. {
                    f32::from_le_bytes(texel[..4].try_into().unwrap())
                } else {
                    sample.value
                };
                value[18] = sample.packed(curvature);
            }
        }
    }
    progress.rows.store(DENOISE_END, Ordering::Relaxed);
    Ok(true)
}
