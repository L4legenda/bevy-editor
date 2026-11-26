//! Select one source texel per UV margin on the GPU and reuse it across maps.
use super::{Progress, RayDevice};
use bevy::{
    log::warn,
    render::render_resource::{
        BindGroupEntry, BindGroupLayoutEntry, BindingType, BufferBindingType, BufferDescriptor,
        BufferUsages, CommandEncoderDescriptor, ComputePassDescriptor, MapMode,
        PipelineLayoutDescriptor, PollType, RawComputePipelineDescriptor, ShaderModuleDescriptor,
        ShaderSource, ShaderStages,
    },
};
use std::{sync::mpsc, time::Duration};

const OVERLAP: usize = 2;
const BUFFER_BYTES: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

pub(super) struct Offsets {
    size: usize,
    words: Vec<[u8; 4]>,
}

impl Offsets {
    pub fn apply(
        &self,
        covered: &mut [u8],
        progress: &Progress,
        mut copy_texel: impl FnMut(usize, usize),
    ) -> Result<(), String> {
        progress.check()?;
        let pitch = self.size.div_ceil(4);
        for (y, row) in self.words.chunks_exact(pitch).enumerate() {
            progress.check()?;
            for (block, codes) in row.iter().enumerate() {
                if *codes == [0; 4] {
                    continue;
                }
                for (i, code) in codes.iter().copied().enumerate() {
                    if code == 0 {
                        continue;
                    }
                    let x = block * 4 + i;
                    let dx = (code - 1) as isize % 5 - 2;
                    let dy = (code - 1) as isize / 5 - 2;
                    let pixel = y * self.size + x;
                    let source = y.wrapping_add_signed(dy) * self.size + x.wrapping_add_signed(dx);
                    copy_texel(pixel, source);
                    covered[pixel] = 1 + (dx.abs() + dy.abs()) as u8;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn offsets(
    gpu: &RayDevice,
    size: usize,
    covered: &[u8],
    progress: &Progress,
) -> Result<Option<Offsets>, String> {
    match compute(gpu, size, covered, progress, BUFFER_BYTES) {
        Ok(offsets) => Ok(offsets),
        Err(error) => {
            progress.check()?;
            warn!("GPU UV padding failed; retrying on the CPU: {error}");
            Ok(None)
        }
    }
}

fn compute(
    gpu: &RayDevice,
    size: usize,
    covered: &[u8],
    progress: &Progress,
    buffer_bytes: usize,
) -> Result<Option<Offsets>, String> {
    progress.check()?;
    let device = &gpu.device;
    let caps = device.limits();
    let pitch = size.div_ceil(4);
    let row_bytes = pitch * 4;
    let limit = u64::from(caps.max_storage_buffer_binding_size).min(caps.max_buffer_size);
    if size == 0
        || caps.max_storage_buffers_per_shader_stage < 2
        || caps.max_uniform_buffer_binding_size < 32
        || caps.max_compute_invocations_per_workgroup < 64
        || caps.max_compute_workgroup_size_x < 8
        || caps.max_compute_workgroup_size_y < 8
        || pitch.div_ceil(8) > caps.max_compute_workgroups_per_dimension as usize
    {
        return Ok(None);
    }
    let rows = size
        .min(caps.max_compute_workgroups_per_dimension as usize * 8)
        .min(buffer_bytes.min(limit as usize) / row_bytes);
    let interior = if rows == size {
        rows
    } else {
        rows.saturating_sub(OVERLAP * 2)
    };
    if interior == 0 {
        return Ok(None);
    }
    let buffer = |label, size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: size as u64,
            usage,
            mapped_at_creation: false,
        })
    };
    let input = buffer(
        "Bake UV coverage",
        rows * row_bytes,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
    );
    let output = buffer(
        "Bake UV offsets",
        interior * row_bytes,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    );
    let staging = buffer(
        "Bake UV offset readback",
        interior * row_bytes,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    let uniform = buffer(
        "Bake UV band",
        32,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    );
    let entries = [0, 1, 2].map(|binding| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: if binding == 2 {
                BufferBindingType::Uniform
            } else {
                BufferBindingType::Storage {
                    read_only: binding == 0,
                }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    });
    let layout = device.create_bind_group_layout("Bake UV padding", &entries);
    let bindings = device.create_bind_group(
        "Bake UV padding",
        &layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    );
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("Bake UV padding"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("Bake UV padding"),
        source: ShaderSource::Wgsl(include_str!("model_paint_bake_padding.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("Bake UV padding"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("pad_uv"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut upload = Vec::with_capacity(rows * row_bytes);
    let mut words = Vec::with_capacity(size * pitch);
    for y in (0..size).step_by(interior) {
        progress.check()?;
        let end = (y + interior).min(size);
        let first = y.saturating_sub(OVERLAP);
        let last = (end + OVERLAP).min(size);
        let out_rows = end - y;
        let source = &covered[first * size..last * size];
        if source.iter().all(|&byte| byte == 0) || source.iter().all(|&byte| byte != 0) {
            words.resize(words.len() + out_rows * pitch, [0; 4]);
            continue;
        }
        upload.clear();
        for row in source.chunks_exact(size) {
            upload.extend_from_slice(row);
            upload.extend_from_slice(&[0; 3][..row_bytes - size]);
        }
        let config = [size, pitch, last - first, y - first, out_rows, 0, 0, 0]
            .map(|value| (value as u32).to_le_bytes());
        gpu.queue.write_buffer(&input, 0, &upload);
        gpu.queue.write_buffer(&uniform, 0, config.as_flattened());
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Bake UV padding"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("Bake UV padding"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &*bindings, &[]);
            pass.dispatch_workgroups(pitch.div_ceil(8) as u32, out_rows.div_ceil(8) as u32, 1);
        }
        let bytes = (out_rows * row_bytes) as u64;
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
            .map_err(|error| format!("GPU UV padding: {error}"))?;
        received
            .recv_timeout(TIMEOUT)
            .map_err(|error| format!("GPU UV padding readback: {error}"))?
            .map_err(|error| format!("GPU UV padding readback: {error}"))?;
        {
            let data = slice
                .get_mapped_range()
                .map_err(|error| format!("GPU UV padding readback: {error}"))?;
            words.extend(
                data.chunks_exact(4)
                    .map(|word| <[u8; 4]>::try_from(word).unwrap()),
            );
        }
        staging.unmap();
    }
    progress.check()?;
    Ok(Some(Offsets { size, words }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_baked_padding_matches_cpu_on_islands_and_band_boundaries() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let gpu = RayDevice::new(
            app
                .world()
                .resource::<bevy::render::renderer::RenderDevice>(),
            app
                .world()
                .resource::<bevy::render::renderer::RenderQueue>(),
        );
        let size: usize = 97;
        let original: Vec<_> = (0..size * size)
            .map(|i| {
                let (x, y) = (i % size, i / size);
                u8::from(x % 13 < 3 && y % 7 < 4 || x == size - 1 && y % 13 == 0)
            })
            .collect();
        let original_pixels: Vec<_> = original
            .iter()
            .enumerate()
            .map(|(index, &covered)| if covered != 0 { index } else { usize::MAX })
            .collect();
        let progress = Progress::default();
        let mut expected = original.clone();
        let mut expected_pixels = original_pixels.clone();
        super::super::pad_uv(size, &mut expected, &progress, |pixel, source| {
            expected_pixels[pixel] = expected_pixels[source];
        })
        .unwrap();

        let bytes_for_13_rows = size.div_ceil(4) * 4 * 13;
        let offsets = compute(&gpu, size, &original, &progress, bytes_for_13_rows)
            .unwrap()
            .unwrap();
        let mut actual = original.clone();
        let mut actual_pixels = original_pixels;
        progress.cancelled.store(true, Ordering::Relaxed);
        assert!(compute(&gpu, size, &actual, &progress, bytes_for_13_rows).is_err());
        assert!(
            offsets
                .apply(&mut actual, &progress, |_, _| unreachable!())
                .is_err()
        );
        assert_eq!(actual, original);
        progress.cancelled.store(false, Ordering::Relaxed);
        offsets
            .apply(&mut actual, &progress, |pixel, source| {
                actual_pixels[pixel] = actual_pixels[source];
            })
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual_pixels, expected_pixels);

        let size = 4096;
        let mut covered = vec![0; size * size];
        // This seed sits at the first interior band's last row.
        let y = BUFFER_BYTES / size - OVERLAP * 2 - 1;
        let seed = y * size + size - 1;
        covered[seed] = 1;
        let offsets = super::offsets(&gpu, size, &covered, &progress)
            .unwrap()
            .unwrap();
        let mut copied = 0;
        offsets
            .apply(&mut covered, &progress, |_, source| {
                assert_eq!(source, seed);
                copied += 1;
            })
            .unwrap();
        assert_eq!(copied, 8);
        assert_eq!(covered[seed + size], 2);
        assert_eq!(covered[seed + 2 * size], 3);
        assert_eq!(covered[seed + 3 * size], 0);
    }
}
