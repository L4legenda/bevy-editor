//! Benchmark-only addition to the disposable Material Painter checkout.
use crate::{
    bake::{self, BakeSettings, CustomKind, GpuBaker, Quality},
    gpu::{GpuMesh, read_texture},
    mesh::{Mesh, Vertex},
};
use eframe::wgpu;
use glam::Vec3;
use std::{path::PathBuf, time::Instant};

#[test]
#[ignore = "requires GPU and tools/paint-perf/prepare.py --bake"]
fn compare_mesh_bake() {
    let root = PathBuf::from(std::env::var_os("PAINT_PERF_DIR").unwrap());
    let bytes = std::fs::read(root.join("bake-low.bin")).unwrap();
    let vertices: Vec<_> = bytes
        .chunks_exact(32)
        .map(|packed| {
            let read = |index: usize| {
                f32::from_le_bytes(packed[index * 4..(index + 1) * 4].try_into().unwrap())
            };
            let pos = [read(0), read(1), read(2)];
            let normal = Vec3::new(read(3), read(4), read(5));
            let ring = Vec3::new(-pos[2], 0., pos[0]).normalize();
            let tangent = (ring - normal * ring.dot(normal)).normalize();
            Vertex {
                pos,
                normal: normal.to_array(),
                uv: [read(6), read(7)],
                tangent: [tangent.x, tangent.y, tangent.z, 1.],
            }
        })
        .collect();
    let indices = (0..vertices.len() as u32).collect();
    let mesh = Mesh::from_saved("Relief_low".into(), vertices, indices, Vec3::ZERO, 1.);
    let diagonal = mesh.bounds_min.distance(mesh.bounds_max);
    let (_, adapter, device, queue) = bake::tests::gpu_with_adapter();
    let info = adapter.get_info();
    let baker = GpuBaker::new(&device);
    let gpu_mesh = GpuMesh::new(&device, &mesh);
    let context = eframe::egui::Context::default();
    if std::env::var("PAINT_BAKE_COMPARE_LIVE_PROJECTION").as_deref() == Ok("1") {
        let mut settings = BakeSettings {
            high_poly: Some(root.join("bake-high.obj")),
            match_by_name: false,
            front: 0.05 * diagonal,
            rear: 0.05 * diagonal,
            maps: 0,
            ..BakeSettings::default()
        };
        settings.apply_quality(Quality::Draft);
        settings.add_custom(CustomKind::Exposure);
        settings.custom[0].distance = 2. * (mesh.bounds_max - mesh.bounds_min).max_element();
        let scene = bake::prepare(&mesh, &settings, 0).unwrap();
        for size in [96, 256] {
            for round in 0..5 {
                for azimuth in [0., 45.] {
                    settings.custom[0].azimuth = azimuth;
                    let started = Instant::now();
                    let mut run = baker.start(&device, &queue, &gpu_mesh, &scene, &settings.preview(), size);
                    let maps = loop {
                        if let Some(maps) = baker.pump(&mut run, &device, &queue, &context) {
                            break maps;
                        }
                        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    };
                    let pixels = read_texture(&device, &queue, &maps.custom[0]);
                    let total_ms = started.elapsed().as_secs_f64() * 1000.;
                    assert!(run.report.finished() && pixels.iter().any(|&v| v != 0));
                    assert!(run.report.missed_percent().unwrap() < 5.);
                    if round > 0 {
                        println!("LIVE_PROJECTION_COMPARE {}", serde_json::json!({
                            "app": "Material Painter", "adapter": info.name,
                            "backend": format!("{:?}", info.backend),
                            "size": size, "run": round, "azimuth": azimuth, "total_ms": total_ms,
                        }));
                    }
                }
            }
        }
        return;
    }
    for (size, high) in [(512, false), (1024, false), (512, true)] {
        let mut settings = BakeSettings {
            high_poly: high.then(|| root.join("bake-high.obj")),
            match_by_name: false,
            front: 0.05 * diagonal,
            rear: 0.05 * diagonal,
            ao_distance: 0.6 * diagonal,
            cavity_distance: 0.04 * diagonal,
            thickness_distance: 0.5 * diagonal,
            ..BakeSettings::default()
        };
        settings.apply_quality(Quality::Standard);
        for iteration in 0..4 {
            let started = Instant::now();
            let scene = bake::prepare(&mesh, &settings, 0).unwrap();
            let prepared_ms = started.elapsed().as_secs_f64() * 1000.;
            let mut run = baker.start(&device, &queue, &gpu_mesh, &scene, &settings, size);
            let maps = loop {
                if let Some(maps) = baker.pump(&mut run, &device, &queue, &context) {
                    break maps;
                }
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            };
            let packed: Vec<_> = maps
                .maps
                .iter()
                .map(|map| read_texture(&device, &queue, map))
                .collect();
            let total_ms = started.elapsed().as_secs_f64() * 1000.;
            assert!(run.report.finished());
            let report = run.report.cache_json();
            let coverage =
                report["valid_texels"].as_u64().unwrap() as f64 * 100. / f64::from(size * size);
            if high {
                assert!(run.report.missed_percent().unwrap() < 5.);
            }
            assert!((75. ..=85.).contains(&coverage), "UV coverage {coverage}");
            assert!(packed[0].iter().any(|&v| v != 0));
            if iteration > 0 {
                println!(
                    "BAKE_COMPARE {}",
                    serde_json::json!({
                        "app": "Material Painter", "adapter": info.name, "backend": format!("{:?}", info.backend),
                        "size": size, "high": high, "run": iteration, "coverage": coverage,
                        "prepared_ms": prepared_ms, "total_ms": total_ms,
                    })
                );
            }
        }
    }
}
