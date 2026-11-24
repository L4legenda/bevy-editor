// Included by Material Painter's offscreen preview tests in the comparison checkout.

#[test]
#[ignore]
fn compare_paint_shading_viewport() {
    use crate::{
        engine::FillParams,
        graph::{Graph, Library, Value, shading},
        post::Tonemap,
        viewport::Background,
    };

    let directory = std::path::PathBuf::from(std::env::var_os("PAINT_SHADING_DIR").unwrap());
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("input.json")).unwrap()).unwrap();
    let number = |name: &str| input[name].as_f64().unwrap() as f32;
    let size = [
        input["width"].as_u64().unwrap() as u32,
        input["height"].as_u64().unwrap() as u32,
    ];
    let mesh = Mesh::load_file(&directory.join("cube.obj")).unwrap();
    assert_eq!(mesh.bounds_min, -Vec3::ONE);
    assert_eq!(mesh.bounds_max, Vec3::ONE);
    let rs = render_state();
    let info = rs.adapter.get_info();
    std::fs::write(directory.join("material-painter-adapter.txt"), format!("{} {:?}", info.name, info.backend)).unwrap();
    let gpu = GpuMesh::new(&rs.device, &mesh);
    let mut engine = Engine::new(&rs, &gpu, (mesh.bounds_min, mesh.bounds_max), 256);
    let mut fill = FillParams::default();
    fill.mat.color = std::array::from_fn(|i| input["color"][i].as_u64().unwrap() as f32 / 255.0);
    fill.mat.rough = number("roughness");
    fill.mat.metal = number("metallic");
    let mut undo = crate::undo::UndoStack::default();
    engine.add_fill_layer(0, None, "Comparison material".into(), fill, &mut undo);
    engine.composite();

    let lib = Library::load(None);
    let mut views = ViewRenderer::new(&rs);
    let mut coverage = Graph::new("Coverage");
    let color = coverage.add("shade_color", [0., 0.]);
    coverage.node_mut(color).unwrap().params.insert("color".into(), Value::Color([0., 1., 0.]));
    let output = coverage.add(shading::OUTPUT, [200., 0.]);
    coverage.connect(color, output, "color");
    let presets = shading::presets();
    let unlit = presets.iter().find(|graph| graph.name == "Unlit").unwrap().clone();
    let shots = std::iter::once(coverage).chain(presets).chain([shading::starter("PBR")]).chain(std::iter::once({
        let mut graph = unlit;
        graph.name = "Unlit linear".into();
        graph
    }));
    let cam = OrbitCam {
        yaw: number("yaw"), pitch: number("pitch"), dist: number("distance"),
        fov_y: number("fov"), ..Default::default()
    };
    let env = crate::environment::EnvBaker::new(&rs.device)
        .upload(&rs, &crate::environment::decode(&crate::environment::EnvSource::Builtin(0)).unwrap());
    for graph in shots {
        let mut settings = ViewSettings { background: Background::Color, ..Default::default() };
        if graph.name == "Unlit linear" {
            settings.post.tonemap = Tonemap::Linear;
        }
        let compiled = shading::compile(&graph, &lib).unwrap();
        views.set_custom_shader(&rs.device, Some(&compiled.code)).unwrap();
        views.set_custom_params(&rs.queue, &shading::param_values(&compiled, &graph, &lib));
        views.render_3d(&rs, size, &[&engine], &env, &gpu, &cam, &settings, None);
        let img = views.view3d.image();
        let bytes = read_texture(&rs.device, &rs.queue, img);
        image::save_buffer(directory.join(format!("mp-{}.png", graph.name)),
            &bytes, img.width, img.height, image::ExtendedColorType::Rgba8).unwrap();
    }
}
