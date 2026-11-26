//! Time the complete GPU-assisted bake on the same input as Material Painter.
use super::*;
use crate::editor::model_mesh::{Corner, Face, Geometry};
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use std::{collections::HashMap, path::PathBuf, time::Instant};

#[test]
#[ignore = "requires GPU and tools/paint-perf/prepare.py --bake"]
fn gpu_compare_mesh_bake() {
    let root = PathBuf::from(std::env::var_os("PAINT_PERF_DIR").unwrap());
    let bytes = std::fs::read(root.join("bake-low.bin")).unwrap();
    let mut geometry = Geometry::default();
    let mut welded = HashMap::new();
    for face in bytes.chunks_exact(96) {
        let mut corners = Vec::with_capacity(3);
        for packed in face.chunks_exact(32) {
            let read = |index: usize| {
                f32::from_le_bytes(packed[index * 4..(index + 1) * 4].try_into().unwrap())
            };
            let position = Vec3::new(read(0), read(1), read(2));
            let key = (position * 100_000.).round().as_ivec3();
            let vertex = *welded.entry(key).or_insert_with(|| {
                let vertex = geometry.positions.len();
                geometry.positions.push(position);
                vertex
            });
            let mut corner = Corner::new(vertex);
            corner.uv = Vec2::new(read(6), read(7));
            corner.source_normal = Some(Vec3::new(read(3), read(4), read(5)));
            corners.push(corner);
        }
        geometry.faces.push(Face {
            corners,
            material: 0,
            smooth: true,
        });
    }
    geometry.validate().unwrap();
    let objects = vec![Object {
        id: 0,
        source_node: None,
        name: "Relief_low".into(),
        geometry: geometry.into(),
        transform: Transform::IDENTITY,
        materials: Vec::new(),
        hidden: false,
    }];
    let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
    let world = app.world();
    let gpu = RayDevice::new(
        world.resource::<RenderDevice>(),
        world.resource::<RenderQueue>(),
    );
    let info = world.resource::<RenderAdapterInfo>();
    if std::env::var("PAINT_BAKE_COMPARE_LIVE_PROJECTION").as_deref() == Ok("1") {
        compare_live_projection(objects, root.join("bake-high.obj"), &gpu, info);
        return;
    }
    let serial_gpu = std::env::var("PAINT_BAKE_SERIAL").as_deref() == Ok("1");
    let compare_raster = std::env::var("PAINT_BAKE_COMPARE_RASTER").as_deref() == Ok("1");
    let compare_visibility = std::env::var("PAINT_BAKE_COMPARE_VISIBILITY").as_deref() == Ok("1");
    let compare_projection = std::env::var("PAINT_BAKE_COMPARE_PROJECTION").as_deref() == Ok("1");
    let compare_high_source = std::env::var("PAINT_BAKE_COMPARE_HIGH_SOURCE").as_deref() == Ok("1");
    let comparisons = [
        compare_raster,
        compare_visibility,
        compare_projection,
        compare_high_source,
    ]
    .into_iter()
    .filter(|&enabled| enabled)
    .count();
    assert!(comparisons <= 1);
    let compare = comparisons == 1;
    let cases = [(512, false), (1024, false), (512, true)];
    for (size, high) in cases
        .into_iter()
        .filter(|&(_, high)| !(compare_projection || compare_high_source) || high)
    {
        let settings = ProjectionSettings {
            path: high.then(|| root.join("bake-high.obj")),
            match_names: false,
            quality: super::super::Quality::Standard,
            ..Default::default()
        };
        let mut previous_digest = None;
        for index in 0..if compare { 8 } else { 4 } {
            let iteration = if compare { index / 2 } else { index };
            let alternate = (index % 2 == 1) != (iteration % 2 == 1);
            let gpu_raster = compare_raster && alternate;
            let legacy_visibility_readback = compare_visibility && !alternate;
            let serial_projection = compare_projection && !alternate;
            let fresh_high_source = compare_high_source && !alternate;
            let objects = objects.clone();
            let progress = Progress {
                serial_gpu,
                gpu_raster,
                legacy_visibility_readback,
                serial_projection,
                fresh_high_source,
                profile: true,
                ..Default::default()
            };
            let start = Instant::now();
            let maps =
                bake_on_device(objects, 0, 0, size, &settings, &progress, Some(&gpu)).unwrap();
            let total_ms = start.elapsed().as_secs_f64() * 1000.;
            let coverage = maps.filled as f64 * 100. / f64::from(size * size);
            assert!((75. ..=85.).contains(&coverage), "UV coverage {coverage}");
            assert!(
                maps.gpu_rays
                    && maps.gpu_projection == high
                    && maps.gpu_denoise
                    && maps.gpu_padding
            );
            if high {
                assert!(maps.hits * 100 > maps.filled * 95);
            }
            let resident_visibility = progress.resident_visibility.load(Ordering::Relaxed);
            let projection_peak = progress.projection_peak.load(Ordering::Relaxed);
            let high_source_hit = progress.high_source_hit.load(Ordering::Relaxed);
            let high_relief_hit = progress.high_relief_hit.load(Ordering::Relaxed);
            if compare_high_source && iteration > 0 {
                assert_eq!(high_source_hit, !fresh_high_source);
                assert_eq!(high_relief_hit, !fresh_high_source);
            }
            if compare_projection {
                assert_eq!(projection_peak, if serial_projection { 1 } else { 3 });
            }
            let readback_bytes = progress.visibility_readback_bytes.load(Ordering::Relaxed);
            if compare_visibility {
                assert_eq!(resident_visibility, !legacy_visibility_readback);
                assert_eq!(
                    readback_bytes,
                    if resident_visibility {
                        0
                    } else {
                        (if high {
                            maps.hits + maps.filled_misses
                        } else {
                            maps.filled
                        } * 8) as u64
                    }
                );
            }
            let mut digest = blake3::Hasher::new();
            digest.update(maps.values.as_flattened());
            digest.update(&maps.covered);
            if let Some(projection) = &maps.projection {
                digest.update(projection.as_flattened());
            }
            for plane in &maps.custom {
                digest.update(plane.values.as_slice());
            }
            let digest = digest.finalize();
            if compare {
                assert_eq!(progress.gpu_raster_ms.get().is_some(), gpu_raster);
                if let Some(previous) = previous_digest {
                    assert_eq!(
                        previous, digest,
                        "GPU scheduling changed the bake at {size}, high={high}"
                    );
                }
                previous_digest = Some(digest);
            }
            if iteration > 0 {
                let [prepared_ms, raster_ms, denoise_ms, padding_ms] =
                    *progress.stage_ms.get().unwrap();
                println!(
                    "BAKE_COMPARE {}",
                    serde_json::json!({
                        "app": "Bevy Editor", "adapter": info.name, "backend": format!("{:?}", info.backend),
                        "size": size, "high": high, "run": iteration, "coverage": coverage, "total_ms": total_ms,
                        "serial_gpu": serial_gpu, "prepared_ms": prepared_ms, "raster_ms": raster_ms,
                        "gpu_raster": gpu_raster, "gpu_ownership_ms": progress.gpu_raster_ms.get(),
                        "resident_visibility": resident_visibility, "visibility_readback_bytes": readback_bytes,
                        "serial_projection": serial_projection, "projection_peak": projection_peak,
                        "high_source_hit": high_source_hit, "high_relief_hit": high_relief_hit,
                        "denoise_ms": denoise_ms, "padding_ms": padding_ms,
                        "visibility_read_ms": progress.ray_wait_ns.load(Ordering::Relaxed) as f64 / 1e6,
                        "projection_read_ms": progress.projection_wait_ns.load(Ordering::Relaxed) as f64 / 1e6,
                        "projection_readback_bytes": progress.projection_readback_bytes.load(Ordering::Relaxed),
                        "digest": digest.to_hex().as_str(),
                    })
                );
            }
        }
    }
}

fn compare_live_projection(
    objects: Vec<Object>, high: PathBuf, gpu: &RayDevice, info: &RenderAdapterInfo,
) {
    use crate::editor::model_paint_data::custom::{Kind, Recipe};

    let compare_pipelines = std::env::var("PAINT_BAKE_COMPARE_LIVE_PIPELINES").as_deref() == Ok("1");
    let compare_scene_buffers = std::env::var("PAINT_BAKE_COMPARE_LIVE_SCENE").as_deref() == Ok("1");
    assert!(!(compare_pipelines && compare_scene_buffers));
    let recipe = Recipe::new(Kind::Exposure, &[]);
    let adjusted = Recipe { azimuth: 45, ..recipe.clone() };
    let settings = ProjectionSettings {
        path: Some(high),
        match_names: false,
        ..Default::default()
    };
    for size in [96, 256] {
        let mut expected = None;
        let mut expected_adjusted = None;
        for round in 0..5 {
            for alternate in [round % 2 == 0, round % 2 != 0] {
                let serial = !(compare_pipelines || compare_scene_buffers) && alternate;
                let fresh_pipelines = compare_pipelines && alternate;
                let fresh_scene_buffers = !compare_scene_buffers || alternate;
                let fresh_gpu = if fresh_pipelines {
                    Some(RayDevice::new(&gpu.device, &gpu.queue))
                } else {
                    fresh_scene_buffers.then(|| gpu.with_fresh_scene_cache())
                };
                let gpu = fresh_gpu.as_ref().unwrap_or(gpu);
                let progress = Progress { serial_projection: serial, profile: true, ..default() };
                let scene = live::Scene::prepare(objects.clone(), 0, 0, &settings, &progress).unwrap();
                let started = Instant::now();
                let (pixels, used_gpu) = scene.shade_on_device(
                    &recipe, size, super::super::Quality::Draft, &progress, Some(gpu),
                ).unwrap();
                let total_ms = started.elapsed().as_secs_f64() * 1000.;
                assert!(used_gpu && scene.has_gpu_raster(size));
                let digest = blake3::hash(&pixels);
                assert_eq!(*expected.get_or_insert(digest), digest);
                let peak = progress.projection_peak.load(Ordering::Relaxed);
                assert_eq!(peak, if serial { 1 } else if size == 96 { 2 } else { 3 });
                let adjusted_progress = Progress { serial_gpu: serial, profile: true, ..default() };
                let started = Instant::now();
                let (pixels, used_gpu) = scene.shade_on_device(
                    &adjusted, size, super::super::Quality::Draft, &adjusted_progress, Some(gpu),
                ).unwrap();
                let adjusted_ms = started.elapsed().as_secs_f64() * 1000.;
                assert!(used_gpu);
                assert_eq!(adjusted_progress.projection_peak.load(Ordering::Relaxed), 0);
                let adjusted_digest = blake3::hash(&pixels);
                assert_ne!(digest, adjusted_digest);
                assert_eq!(*expected_adjusted.get_or_insert(adjusted_digest), adjusted_digest);
                if round > 0 {
                    println!("LIVE_PROJECTION_COMPARE {}", serde_json::json!({
                        "app": "Bevy Editor",
                        "adapter": info.name, "backend": format!("{:?}", info.backend),
                        "size": size, "run": round, "serial": serial, "projection_peak": peak,
                        "fresh_pipelines": fresh_pipelines,
                        "fresh_scene_buffers": fresh_scene_buffers,
                        "total_ms": total_ms,
                        "projection_read_ms": progress.projection_wait_ns.load(Ordering::Relaxed) as f64 / 1e6,
                        "adjusted_ms": adjusted_ms,
                        "adjusted_read_ms": adjusted_progress.ray_wait_ns.load(Ordering::Relaxed) as f64 / 1e6,
                        "digest": digest.to_hex().as_str(),
                    }));
                }
            }
        }
    }
}
