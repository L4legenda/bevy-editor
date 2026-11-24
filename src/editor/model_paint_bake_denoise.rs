//! Geometry-aware à-trous filtering of ray-traced maps before UV padding.
use super::{Maps, Progress, RayDevice, rgb};
use bevy::prelude::*;
use bevy::render::render_resource::Buffer;
use std::sync::atomic::Ordering;

#[path = "model_paint_bake_denoise_gpu.rs"]
mod gpu;
pub(super) use gpu::visibility_atlas;

#[derive(Clone, Copy, Default)]
pub(super) struct Guide {
    position: Vec3,
    texel: f32,
    part: u32,
    island: u32,
}

impl Guide {
    pub fn new(position: Vec3, part: usize, island: usize, two_texels: f32) -> Self {
        Self {
            position,
            texel: two_texels * 0.5,
            part: part as u32,
            island: island as u32,
        }
    }
}

const KERNEL: [f32; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];
pub(super) const RASTER_END: u32 = 850;
const DENOISE_END: u32 = 975;

struct Tap {
    offset: IVec2,
    radius: f32,
    weight: f32,
}

fn decode_normal(bytes: &[u8]) -> Vec3 {
    Vec3::new(bytes[0] as f32, bytes[1] as f32, bytes[2] as f32) * (2.0 / 255.0) - 1.0
}

pub(super) fn filter(
    maps: &mut Maps,
    guides: &[Guide],
    passes: u32,
    progress: &Progress,
    device: Option<&RayDevice>,
    atlas: Option<&Buffer>,
) -> Result<(), String> {
    if let Some(device) = device {
        let result = gpu::filter(device, maps, guides, passes, progress, atlas);
        if atlas.is_some() && !matches!(result, Ok(true)) {
            return Err(result
                .err()
                .unwrap_or_else(|| "GPU bake denoise: resident atlas unavailable".into()));
        }
        match result {
            Ok(true) => {
                maps.gpu_denoise = true;
                return Ok(());
            }
            Ok(false) => {}
            Err(error) => {
                progress.check()?;
                warn!("GPU denoise failed; retrying on the CPU: {error}");
            }
        }
    }
    filter_cpu(maps, guides, passes, progress)
}

fn filter_cpu(
    maps: &mut Maps,
    guides: &[Guide],
    passes: u32,
    progress: &Progress,
) -> Result<(), String> {
    let size = maps.size as usize;
    let mut scratch = vec![[0; 7]; guides.len()];
    for pass in 0..passes {
        let step = 1 << pass;
        let taps: Vec<_> = KERNEL
            .iter()
            .enumerate()
            .flat_map(|(y, &wy)| {
                KERNEL.iter().enumerate().map(move |(x, &wx)| {
                    let offset = IVec2::new(x as i32 - 2, y as i32 - 2) * step;
                    Tap {
                        offset,
                        radius: (offset.as_vec2().length() * 2.0 + 2.0).powi(2),
                        weight: wx * wy,
                    }
                })
            })
            .collect();
        for y in 0..size {
            progress.check()?;
            progress.rows.store(
                RASTER_END
                    + (DENOISE_END - RASTER_END) * (pass * maps.size + y as u32)
                        / (passes * maps.size),
                Ordering::Relaxed,
            );
            for x in 0..size {
                let index = y * size + x;
                let guide = guides[index];
                if guide.texel <= 0.0 {
                    continue;
                }
                let normal_bytes = &maps.values[index][9..12];
                let normal = decode_normal(normal_bytes).normalize_or_zero();
                let distance_scale = guide.texel.powi(2);
                let plane_scale = (guide.texel * step as f32).recip();
                let point = IVec2::new(x as i32, y as i32);
                let mut channels = Vec4::ZERO;
                let mut bent = Vec3::ZERO;
                let mut total = 0.0;
                for tap in &taps {
                    let sample = point + tap.offset;
                    if sample.min_element() < 0 || sample.max_element() >= size as i32 {
                        continue;
                    }
                    let neighbor = sample.y as usize * size + sample.x as usize;
                    let other = guides[neighbor];
                    if other.texel <= 0.0
                        || guide.part != other.part
                        || guide.island != other.island
                    {
                        continue;
                    }
                    let delta = other.position - guide.position;
                    if delta.length_squared() > tap.radius * distance_scale {
                        continue;
                    }
                    let value = &maps.values[neighbor];
                    let facing = if normal_bytes == &value[9..12] {
                        1.0
                    } else {
                        normal
                            .dot(decode_normal(&value[9..12]).normalize_or_zero())
                            .clamp(0.0, 1.0)
                            .powi(64)
                    };
                    let plane = delta.dot(normal) * plane_scale;
                    let weight = tap.weight * facing * (-4.0 * plane * plane).exp();
                    channels += Vec4::new(
                        value[0] as f32,
                        value[1] as f32,
                        value[2] as f32,
                        value[3] as f32,
                    ) * weight;
                    bent += decode_normal(&value[6..9]) * weight;
                    total += weight;
                }
                if total < 1.0e-6 {
                    scratch[index][..4].copy_from_slice(&maps.values[index][..4]);
                    scratch[index][4..].copy_from_slice(&maps.values[index][6..9]);
                } else {
                    scratch[index][..4].copy_from_slice(
                        &(channels / total)
                            .to_array()
                            .map(|value| value.round() as u8),
                    );
                    scratch[index][4..].copy_from_slice(&rgb(bent
                        .try_normalize()
                        .unwrap_or(normal)
                        * 0.5
                        + 0.5));
                }
            }
        }
        for ((output, filtered), guides) in maps
            .values
            .chunks_mut(size)
            .zip(scratch.chunks(size))
            .zip(guides.chunks(size))
        {
            progress.check()?;
            for ((value, filtered), guide) in output.iter_mut().zip(filtered).zip(guides) {
                if guide.texel > 0.0 {
                    value[..4].copy_from_slice(&filtered[..4]);
                    value[6..9].copy_from_slice(&filtered[4..]);
                }
            }
        }
    }
    progress.rows.store(DENOISE_END, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps_with_values(size: usize, values: Vec<[u8; super::super::COMPONENTS]>) -> Maps {
        Maps {
            size: size as u32,
            gpu_rays: false,
            gpu_projection: false,
            gpu_denoise: false,
            gpu_padding: false,
            selection: super::super::MapSelection::ALL,
            filled: values.len(),
            covered: vec![1; values.len()],
            values,
            projection: None,
            hits: 0,
            filled_misses: 0,
            cage_fit: None,
            edge_reach: 1.,
            inputs: Default::default(),
            custom: Vec::new(),
        }
    }

    fn fixture(size: usize) -> (Maps, Vec<Guide>) {
        let mut guides = Vec::new();
        let mut values = Vec::new();
        for y in 0..size {
            for x in 0..size {
                let (part, island, normal, height, level) = match x {
                    0..=3 => (0, 0, Vec3::Z, 0.0, 255 * ((x + y) % 2) as u8),
                    4..=7 => (0, 1, Vec3::Z, 0.0, 220),
                    8..=11 => (1, 1, Vec3::Z, 0.0, 120),
                    12..=15 => (1, 1, Vec3::Y, 0.0, 20),
                    16..=19 => (1, 1, Vec3::Y, 100.0, 190),
                    _ => (1, 1, Vec3::Y, 100.0, 250),
                };
                let mut pixel = super::super::neutral_texel(normal);
                pixel[..4].fill(level);
                pixel[4..6].copy_from_slice(&[17, 77]);
                pixel[12..21].fill(123);
                values.push(pixel);
                guides.push(if x >= 20 {
                    Guide::default()
                } else {
                    Guide::new(Vec3::new(x as f32, height, y as f32), part, island, 2.0)
                });
            }
        }
        (maps_with_values(size, values), guides)
    }

    #[test]
    fn baked_denoise_filters_noise_without_crossing_uv_islands_or_geometry() {
        let size = 24;
        let (mut maps, guides) = fixture(size);
        let covered = maps.covered.clone();
        let before = maps.values.clone();
        filter(&mut maps, &guides, 4, &Progress::default(), None, None).unwrap();
        assert_eq!(maps.covered, covered);
        for (i, (pixel, original)) in maps.values.iter().zip(&before).enumerate() {
            match i % size {
                0..=3 => {
                    assert!(pixel[..4].iter().all(|&value| (90..165).contains(&value)));
                    assert!(decode_normal(&pixel[6..9]).dot(Vec3::Z) > 0.99);
                }
                _ => assert_eq!(pixel, original),
            }
            assert_eq!(&pixel[4..6], &original[4..6]);
            assert_eq!(&pixel[9..], &original[9..]);
        }
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_baked_denoise_matches_cpu_across_band_boundaries() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice {
            device: app
                .world()
                .resource::<bevy::render::renderer::RenderDevice>()
                .clone(),
            queue: app
                .world()
                .resource::<bevy::render::renderer::RenderQueue>()
                .clone(),
        };
        let size = 96;
        let (mut cpu, guides) = fixture(size);
        let (mut actual, _) = fixture(size);
        // The first island contains gradients that continue across band boundaries.
        for (row, pixels) in cpu.values.chunks_mut(size).enumerate() {
            for (x, pixel) in pixels[..4].iter_mut().enumerate() {
                pixel[0] = ((17 * row + x * 83) % 256) as u8;
                pixel[1] = (row * 2) as u8;
                pixel[6] = 128 + ((row + x) % 31) as u8;
            }
        }
        actual.values.clone_from(&cpu.values);
        let original = cpu.values.clone();
        let progress = Progress::default();
        let bytes_for_70_rows = size * 70 * 28;
        progress.cancelled.store(true, Ordering::Relaxed);
        assert!(
            gpu::filter_bands(
                &device,
                &mut actual,
                &guides,
                4,
                &progress,
                bytes_for_70_rows,
                None,
            )
            .is_err()
        );
        assert_eq!(actual.values, original);
        progress.cancelled.store(false, Ordering::Relaxed);
        filter_cpu(&mut cpu, &guides, 4, &progress).unwrap();
        assert!(
            gpu::filter_bands(
                &device,
                &mut actual,
                &guides,
                4,
                &progress,
                bytes_for_70_rows,
                None,
            )
            .unwrap()
        );
        for (i, (expected, output)) in cpu.values.iter().zip(&actual.values).enumerate() {
            if i % size < 4 {
                assert!(
                    expected
                        .iter()
                        .zip(output)
                        .all(|(a, b)| a.abs_diff(*b) <= 2),
                    "denoise output at ({}, {}): CPU {expected:?}, GPU {output:?}",
                    i % size,
                    i / size
                );
            } else {
                assert_eq!(output, &original[i]);
            }
            assert_eq!(&output[4..6], &original[i][4..6]);
            assert_eq!(&output[9..], &original[i][9..]);
        }
        assert_eq!(actual.covered, cpu.covered);
        use bevy::render::render_resource::{BufferInitDescriptor, BufferUsages};
        let raw: Vec<_> = original
            .iter()
            .flat_map(|pixel| {
                [
                    pixel[0], pixel[1], pixel[2], pixel[3], pixel[6], pixel[7], pixel[8], 255,
                ]
            })
            .collect();
        let atlas = device
            .device
            .create_buffer_with_data(&BufferInitDescriptor {
                label: Some("Test resident maps"),
                contents: &raw,
                usage: BufferUsages::COPY_SRC,
            });
        let (mut resident, _) = fixture(size);
        resident.values.clone_from(&original);
        for pixel in &mut resident.values {
            pixel[..4].fill(0);
            pixel[6..9].fill(0);
        }
        assert!(
            gpu::filter_bands(
                &device,
                &mut resident,
                &guides,
                4,
                &progress,
                bytes_for_70_rows,
                Some(&atlas),
            )
            .unwrap()
        );
        assert_eq!(resident.values, actual.values);
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device and a 4096 × 4096 bake atlas"]
    fn gpu_baked_denoise_handles_a_4096_atlas() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice {
            device: app
                .world()
                .resource::<bevy::render::renderer::RenderDevice>()
                .clone(),
            queue: app
                .world()
                .resource::<bevy::render::renderer::RenderQueue>()
                .clone(),
        };
        let size = 4096;
        let mut maps = maps_with_values(size, vec![[0; super::super::COMPONENTS]; size * size]);
        let mut guides = vec![Guide::default(); size * size];
        let x = size - 1;
        let y = size - 1;
        let last = y * size + x;
        let previous = last - size;
        for (index, level) in [(last, 0), (previous, 255)] {
            guides[index] = Guide::new(Vec3::Y * (index / size) as f32, 1, 2, 2.);
            maps.values[index] = super::super::neutral_texel(Vec3::Z);
            maps.values[index][..4].fill(level);
        }
        filter(
            &mut maps,
            &guides,
            4,
            &Progress::default(),
            Some(&device),
            None,
        )
        .unwrap();
        assert!(maps.gpu_denoise);
        assert!(
            maps.values[last][..4]
                .iter()
                .all(|level| (90..165).contains(level))
        );
        assert!(
            maps.values[previous][..4]
                .iter()
                .all(|level| (90..165).contains(level))
        );
        assert_eq!(maps.values[last - 1], [0; super::super::COMPONENTS]);
    }
}
