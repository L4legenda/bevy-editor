//! Benchmark-only addition to a disposable copy of Material Painter.
use crate::{
    engine::{BrushPaint, Engine, MatValues, StampUniform},
    gpu::{GpuMesh, Tex},
    mesh::{Mesh, Vertex},
    undo::{StrokeTarget, UndoStack},
    viewport::{OrbitCam, ViewRenderer, ViewSettings},
};
use eframe::{egui_wgpu, wgpu};
use glam::{Vec2, Vec3};
use std::{sync::Arc, time::Instant};

#[test]
#[ignore]
fn compare_complex_paint() {
    let root = std::path::PathBuf::from(std::env::var("PAINT_PERF_DIR").unwrap());
    let bytes = std::fs::read(root.join("complex.bin")).unwrap();
    let vertices: Vec<Vertex> = bytes
        .chunks_exact(32)
        .map(|p| {
            let v: Vec<f32> = p
                .chunks_exact(4)
                .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
                .collect();
            Vertex {
                pos: v[0..3].try_into().unwrap(),
                normal: v[3..6].try_into().unwrap(),
                uv: v[6..8].try_into().unwrap(),
                tangent: [1., 0., 0., 1.],
            }
        })
        .collect();
    let indices = (0..vertices.len() as u32).collect();
    let mesh = Mesh::from_saved(
        "Shared 262k relief sphere".into(),
        vertices,
        indices,
        Vec3::ZERO,
        1.,
    );
    let (instance, adapter, device, queue) = crate::bake::tests::gpu_with_adapter();
    println!("ADAPTER {:?}", adapter.get_info());
    let renderer = egui_wgpu::Renderer::new(
        &device,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    );
    let rs = egui_wgpu::RenderState {
        available_adapters: vec![adapter.clone()],
        adapter,
        instance,
        device,
        queue,
        target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
        renderer: Arc::new(eframe::egui::mutex::RwLock::new(renderer)),
    };
    let gpu_mesh = GpuMesh::new(&rs.device, &mesh);
    let cam = OrbitCam {
        yaw: 0.,
        pitch: 0.,
        dist: 4.,
        target: Vec3::ZERO,
        fov_y: 58f32.to_radians(),
    };
    let size = Vec2::new(1024., 768.);
    let bvh = crate::bvh::Bvh::from_mesh(&mesh);
    let wait = || {
        rs.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    };
    let mut report = Vec::new();
    for res in [1024, 2048] {
        let mut engine = Engine::new(&rs, &gpu_mesh, (mesh.bounds_min, mesh.bounds_max), res);
        let mut undo = UndoStack::default();
        let enabled = [true, false, false, false, false, false, false, false];
        for i in 0..8 {
            engine.add_paint_layer(i, None, None, &mut undo);
            let tex = Tex::new(
                &rs.device,
                "populated benchmark layer",
                res,
                res,
                wgpu::TextureFormat::Rgba8Unorm,
                crate::gpu::LAYER_USAGE,
            );
            let pixels = [60u8 + i as u8 * 5, 70, 80, 30].repeat((res * res) as usize);
            rs.queue.write_texture(
                tex.copy_info(),
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(res * 4),
                    rows_per_image: Some(res),
                },
                tex.extent(),
            );
            if let crate::engine::Content::Paint(ch) = &mut engine.layers[i].content {
                ch[0] = Some(tex);
            }
        }
        engine.composite();
        // Use the actual depth rendering once, just as the editor does at a stroke boundary.
        let mut views = ViewRenderer::new(&rs);
        let env = crate::environment::EnvBaker::new(&rs.device).upload(
            &rs,
            &crate::environment::decode(&crate::environment::EnvSource::Builtin(0)).unwrap(),
        );
        views.render_3d(
            &rs,
            [1024, 768],
            &[&engine],
            &env,
            &gpu_mesh,
            &cam,
            &ViewSettings::default(),
            None,
        );
        let su = StampUniform {
            view_proj: cam.view_proj(size.x / size.y).to_cols_array_2d(),
            cam_pos: cam.eye().extend(1.).to_array(),
            view: [size.x, size.y, 0., 0.],
            view2d: [0.; 4],
            brush: [35., 0.6, 0.35, 0.],
            clip: [crate::viewport::NEAR, crate::viewport::FAR, 0., 0.],
            proj: [0., 0., 85f32.to_radians().cos(), 0.],
        };
        let paint = BrushPaint {
            mat: MatValues {
                enabled,
                color: [0.9, 0.1, 0.05],
                ..Default::default()
            },
            mask_value: 1.,
            opacity: 1.,
            erase: false,
            grain: [0.; 4],
        };
        assert!(engine.begin_stroke(7, StrokeTarget::Content, enabled, false));
        wait();
        let mut times = Vec::new();
        let mut cpu = Vec::new();
        for frame in 0..200 {
            let start = Instant::now();
            let raw = (0..6)
                .map(|i| {
                    let t = (frame * 6 + i) as f32 * 0.011;
                    (
                        [512. + t.sin() * 90., 384. + (t * 1.7).cos() * 65., 1., 0.],
                        1.,
                    )
                })
                .collect::<Vec<_>>();
            let stamps = crate::app::resolve_stamps(
                &mesh,
                &bvh,
                &cam,
                size,
                crate::app::BrushProjection::Screen,
                0,
                35.,
                &raw,
            );
            let region = crate::app::stroke_region(
                &mesh,
                &bvh,
                &cam,
                size,
                crate::app::BrushProjection::Screen,
                0,
                35.,
                &stamps,
            );
            engine.stroke_step(&stamps, su, views.view3d.depth_view(), &paint, region);
            engine.composite();
            let submit = start.elapsed().as_secs_f64() * 1000.;
            wait();
            if frame >= 20 {
                cpu.push(submit);
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        engine.end_stroke();
        let out = crate::gpu::read_texture(&rs.device, &rs.queue, engine.final_tex(0));
        assert!(
            out.chunks_exact(4).any(|p| p[0] > p[1] + 30),
            "benchmark must actually paint"
        );
        image::save_buffer(
            root.join(format!("material-painter-{res}.png")),
            &out,
            res,
            res,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        times.sort_by(f64::total_cmp);
        cpu.sort_by(f64::total_cmp);
        let row = serde_json::json!({"app":"material-painter","triangles":mesh.triangle_count(),"texture":res,"layers":8,"frames":times.len(),"stamps_per_frame":6,"mean_ms":times.iter().sum::<f64>()/times.len() as f64,"p50_ms":times[times.len()/2],"p95_ms":times[times.len()*95/100],"cpu_mean_ms":cpu.iter().sum::<f64>()/cpu.len() as f64});
        println!("PAINT_COMPARE {row}");
        report.push(row);
        // Keep the returned texture type in this test's API surface to expose format drift.
        let _: &Tex = engine.final_tex(0);
    }
    std::fs::write(
        root.join("material-painter.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
