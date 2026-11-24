// Included only in the Material Painter comparison checkout.

#[test]
#[ignore]
fn compare_paint_pathtrace() {
    use crate::{
        engine::FillParams,
        environment::{EnvBaker, EnvSource},
        mesh::Vertex,
        post::Tonemap,
        viewport::Background,
    };
    use serde::Deserialize;
    use std::{fs, path::PathBuf, sync::Arc, time::Instant};

    #[derive(Deserialize)]
    struct Part {
        slot: usize,
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        uv: Vec<[f32; 2]>,
        indices: Vec<u32>,
    }

    let directory = PathBuf::from(std::env::var_os("PAINT_TRACE_DIR").unwrap());
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("input.json")).unwrap()).unwrap();
    let number = |name: &str| config[name].as_f64().unwrap() as f32;
    let size: [u32; 2] = serde_json::from_value(config["size"].clone()).unwrap();
    let background = serde_json::from_value(config["background"].clone()).unwrap();
    let checkpoints: Vec<u32> = serde_json::from_value(config["checkpoints"].clone()).unwrap();
    let parts: Vec<Part> =
        serde_json::from_slice(&fs::read(directory.join("mesh.json")).unwrap()).unwrap();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut ranges = Vec::new();
    for part in parts {
        let start = indices.len() as u32 / 3;
        let offset = vertices.len() as u32;
        indices.extend(part.indices.into_iter().map(|index| index + offset));
        ranges.push((
            format!("Set {}", part.slot),
            start..indices.len() as u32 / 3,
        ));
        vertices.extend(
            part.positions
                .into_iter()
                .zip(part.normals)
                .zip(part.uv)
                .map(|((pos, normal), uv)| {
                    let tangent = Vec3::from_array(normal).any_orthonormal_vector();
                    Vertex {
                        pos,
                        normal,
                        uv,
                        tangent: tangent.extend(1.0).to_array(),
                    }
                }),
        );
    }
    let mut mesh = Mesh::from_saved(
        "Comparison torus".into(),
        vertices,
        indices,
        Vec3::ZERO,
        1.0,
    );
    mesh.set_ranges(ranges);
    assert_eq!(
        mesh.triangle_count() as u64,
        config["triangles"].as_u64().unwrap()
    );
    assert_eq!(mesh.sets.len(), 2);

    let (instance, adapter, device, queue) = crate::bake::tests::gpu_with_adapter();
    let target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let renderer = egui_wgpu::Renderer::new(&device, target_format, Default::default());
    let rs = egui_wgpu::RenderState {
        available_adapters: vec![adapter.clone()],
        instance,
        adapter,
        device,
        queue,
        target_format,
        renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    };
    let info = rs.adapter.get_info();
    let mesh_gpu = gpu::GpuMesh::new(&rs.device, &mesh);
    let mut engines = Vec::new();
    for slot in 0..mesh.sets.len() {
        let mut engine = Engine::new(
            &rs,
            &mesh_gpu,
            (mesh.bounds_min, mesh.bounds_max),
            number("texture_resolution") as u32,
        );
        let mut fill = FillParams::default();
        fill.mat.color = std::array::from_fn(|channel| {
            config["colors"][slot][channel].as_f64().unwrap() as f32 / 255.0
        });
        fill.mat.rough = config["roughness"][slot].as_f64().unwrap() as f32 / 255.0;
        fill.mat.metal = config["metallic"][slot].as_f64().unwrap() as f32 / 255.0;
        fill.mat.specular = 128.0 / 255.0;
        engine.add_fill_layer(
            0,
            None,
            "Comparison material".into(),
            fill,
            &mut crate::undo::UndoStack::default(),
        );
        engine.composite();
        engines.push(engine);
    }
    let engines: Vec<_> = engines.iter().collect();
    let bvh = Bvh::from_mesh(&mesh);
    let mut tracer = RayTracer::new(&rs);
    tracer.set_mesh(&rs.device, &mesh, &bvh);
    tracer.ensure_target(&rs, size);
    // Only the comparison image requires readback; the regular renderer
    // keeps its scene-linear accumulation on the device.
    tracer.target.as_mut().unwrap().out = Tex::new(
        &rs.device,
        "HDR comparison",
        size[0],
        size[1],
        HDR_FORMAT,
        wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
    );
    let env_img =
        crate::environment::decode(&EnvSource::File(directory.join("editor-sky.hdr"))).unwrap();
    let env = EnvBaker::new(&rs.device).upload(&rs, &env_img);
    let camera = OrbitCam {
        yaw: number("yaw"),
        pitch: number("pitch"),
        dist: number("distance"),
        fov_y: number("fov"),
        ..Default::default()
    };
    let mut view = ViewSettings {
        background: Background::Color,
        bg_color: background,
        env_intensity: 1.0,
        emission: 1.0,
        use_baked_normal: false,
        ..Default::default()
    };
    view.key.on = false;
    view.post.tonemap = Tonemap::Neutral;
    let settings = RtSettings {
        enabled: true,
        max_samples: *checkpoints.last().unwrap(),
        bounces: number("bounces") as u32,
        ground: false,
    };
    let step = |tracer: &mut RayTracer| {
        assert!(tracer.render(&rs, size, &engines, &env, &camera, &view, &settings));
        rs.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    };
    for _ in 0..8 {
        step(&mut tracer);
    }
    tracer.key.clear();
    let mut times = Vec::new();
    let mut captures = Vec::new();
    for sample in 1..=settings.max_samples {
        let start = Instant::now();
        step(&mut tracer);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        if checkpoints.contains(&sample) {
            let data =
                gpu::read_texture(&rs.device, &rs.queue, &tracer.target.as_ref().unwrap().out);
            fs::write(directory.join(format!("mp-{sample}.rgba16")), data).unwrap();
            captures
                .push(serde_json::json!({"samples":sample, "total_ms":times.iter().sum::<f64>()}));
        }
    }
    let report = serde_json::json!({
        "adapter": format!("{} {:?}", info.name, info.backend),
        "frame_ms": times, "checkpoints": captures,
    });
    fs::write(
        directory.join("mp.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
