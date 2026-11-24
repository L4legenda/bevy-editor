use super::*;

#[test]
#[ignore = "requires GPU; verifies polygon wireframe, mesh edits, orthographic views and protected skinned/morph GLTFs"]
fn gpu_model_wireframe_button_handles_polygons_edits_and_protected_models() {
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-wireframe-qa");
    std::fs::create_dir_all(&output).unwrap();
    let directory = output.join(format!("assets-{}", bevy::asset::uuid::Uuid::new_v4()));
    let mut project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_directory(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let original_bytes = std::fs::read(&path).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let before = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    send(&mut app, EditAction::Deselect);
    move_pointer(&mut app, window, Vec2::new(20.0, 110.0));
    let region = canvas_rect(&mut app);
    let solid = capture_when(&mut app, &target, &output.join("01-solid.png"), |_| true);
    let choose = |app: &mut App, choice| {
        click_control(app, window, |action| {
            matches!(action, Action::ModelView(ModelViewAction::Shading(value)) if value == choice)
        });
    };
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Wire));
    let surface_layers = |app: &mut App| {
        let world = app.world_mut();
        world
            .query_filtered::<&RenderLayers, With<xray::SourceMaterial>>()
            .iter(world)
            .cloned()
            .collect::<Vec<_>>()
    };
    let layers = surface_layers(&mut app);
    assert!(!layers.is_empty() && layers.iter().all(|layer| *layer == RenderLayers::none()));
    send(&mut app, EditAction::Mode(Mode::Edit));
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Solid));
    {
        let world = app.world_mut();
        let mut surfaces =
            world.query::<(&xray::SourceMaterial, &MeshMaterial3d<StandardMaterial>)>();
        assert!(surfaces.iter(world).all(|(source, displayed)| source.0 != displayed.0));
    }
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Rendered));
    {
        let world = app.world_mut();
        let mut surfaces =
            world.query::<(&xray::SourceMaterial, &MeshMaterial3d<StandardMaterial>)>();
        assert!(surfaces.iter(world).all(|(source, displayed)| source.0 == displayed.0));
        let brightness = world.resource::<Editor>().environment().ambient_brightness;
        assert_eq!(
            world.query_filtered::<&AmbientLight, With<ViewCamera>>()
                .single(world).unwrap().brightness,
            brightness,
        );
    }
    send(&mut app, EditAction::Mode(Mode::Object));
    send(&mut app, EditAction::Deselect);
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Material));
    {
        let world = app.world_mut();
        let camera = world.query_filtered::<Entity, With<ViewCamera>>().single(world).unwrap();
        assert!(world.get::<bevy::light::Skybox>(camera).is_none());
        assert!(world.get::<bevy::light::EnvironmentMapLight>(camera).is_none());
        assert_eq!(world.get::<AmbientLight>(camera).unwrap().brightness, 0.0);
        let mut surfaces = world.query::<(&xray::SourceMaterial, &MeshMaterial3d<StandardMaterial>)>();
        let assets = world.resource::<Assets<StandardMaterial>>();
        for (source, displayed) in surfaces.iter(world) {
            let source = assets.get(&source.0).unwrap();
            let displayed = assets.get(&displayed.0).unwrap();
            assert!(displayed.unlit);
            assert_eq!(displayed.base_color, source.base_color);
            assert_eq!(displayed.base_color_texture, source.base_color_texture);
            assert_eq!(displayed.alpha_mode, source.alpha_mode);
        }
    }
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Wire));
    move_pointer(&mut app, window, Vec2::new(20.0, 110.0));
    let changed = |image: &image::RgbaImage| {
        image
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                region.contains(Vec2::new(*x as f32, *y as f32))
                    && *y > region.min.y as u32 + 100
                    && p.0
                        .iter()
                        .zip(solid.get_pixel(*x, *y).0)
                        .any(|(a, b)| a.abs_diff(b) > 35)
            })
            .count()
    };
    capture_when(
        &mut app,
        &target,
        &output.join("02-wireframe.png"),
        |image| changed(image) > 100,
    );
    let mesh_handle = {
        let world = app.world_mut();
        world
            .query_filtered::<&Mesh3d, With<wireframe::Overlay>>()
            .single(world)
            .unwrap()
            .0
            .clone()
    };
    let expected_edges: usize = app.world().resource::<State>().documents[&0]
        .objects
        .iter()
        .map(|o| o.geometry.edges().len())
        .sum();
    assert_eq!(
        expected_edges, 12,
        "The cube has polygon edges, without triangle diagonals"
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Mesh>>()
            .get(&mesh_handle)
            .unwrap()
            .indices()
            .unwrap()
            .len(),
        expected_edges * 2
    );
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Material));
    capture_when(
        &mut app,
        &target,
        &output.join("03-solid-restored.png"),
        |image| changed(image) < 20,
    );
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Wire));
    app.world_mut().resource_mut::<Viewer>().orthographic = true;
    warm(&mut app, 8);
    capture_when(
        &mut app,
        &target,
        &output.join("04-orthographic.png"),
        |_| true,
    );
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::SelectAll);
    send(&mut app, EditAction::Operate(Operation::Subdivide));
    warm(&mut app, 4);
    let new_handle = {
        let world = app.world_mut();
        world
            .query_filtered::<&Mesh3d, With<wireframe::Overlay>>()
            .single(world)
            .unwrap()
            .0
            .clone()
    };
    assert_ne!(
        new_handle, mesh_handle,
        "Editing topology must rebuild its wireframe"
    );
    let expected: usize = app.world().resource::<State>().documents[&0]
        .objects
        .iter()
        .map(|o| o.geometry.edges().len() * 2)
        .sum();
    assert!(expected > 24);
    assert_eq!(
        app.world()
            .resource::<Assets<Mesh>>()
            .get(&new_handle)
            .unwrap()
            .indices()
            .unwrap()
            .len(),
        expected
    );
    capture_when(&mut app, &target, &output.join("04-subdivided.png"), |_| {
        true
    });
    send(&mut app, EditAction::Undo);
    assert!(!app.world().resource::<State>().documents[&0].dirty());

    // A GLTF with both skinning and morph targets is intentionally read-only,
    // but must still expose the mesh-view control and correctly deform its lines.
    send(&mut app, EditAction::Mode(Mode::Object));
    protected_glb(&directory.join("models/blue_block.glb"));
    app.world_mut().write_message(Action::ViewModel(1));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<State>().errors.contains_key(&1)
            && app.world().resource::<Viewer>().fitted
        {
            break;
        }
    }
    warm(&mut app, 25);
    assert!(app.world().resource::<State>().errors.contains_key(&1));
    let protected_count = {
        let world = app.world_mut();
        world
            .query_filtered::<(
                &Mesh3d,
                &bevy::mesh::skinning::SkinnedMesh,
                &bevy::mesh::morph::MeshMorphWeights,
            ), With<wireframe::Overlay>>()
            .iter(world)
            .count()
    };
    assert_eq!(
        protected_count, 1,
        "The read-only GLTF needs a skinned morph-aware overlay"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("05-skinned-morph.png"),
        |_| true,
    );
    choose(&mut app, shading::ShadingAction::Set(shading::Mode::Material));
    assert!(surface_layers(&mut app).iter().all(|layer| *layer == RenderLayers::layer(LAYER)));
    assert_eq!(app.world().resource::<Editor>().edits, before);
    assert_eq!(std::fs::read(path).unwrap(), original_bytes);
}

fn protected_glb(path: &Path) {
    use serde_json::json;
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut add = |bytes: Vec<u8>| {
        let offset = bin.len();
        let len = bytes.len();
        bin.extend(bytes);
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":len}));
        views.len() - 1
    };
    let floats = |values: &[f32]| {
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let positions = add(floats(&[
        -1., -1., 0., 1., -1., 0., 1., 1., 0., -1., 1., 0.,
    ]));
    let normals = add(floats(&[0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1.]));
    let joints = add(vec![0; 16]);
    let weights = add(floats(&[
        1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.,
    ]));
    let indices = add([0u16, 1, 2, 0, 2, 3]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect());
    let morph = add(floats(&[0., 0., 0., 0., 0., 0., 0., 0., 0.8, 0., 0., 0.8]));
    let inverse = add(floats(&Mat4::IDENTITY.to_cols_array()));
    let root = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],
    "nodes":[{"name":"Protected","mesh":0,"skin":0,"weights":[0.7]}, {"name":"Joint","translation":[0.25,0.0,0.0]}],
    "skins":[{"joints":[1],"inverseBindMatrices":6}],
    "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.02,0.02,0.02,1.0],"metallicFactor":0.0,"roughnessFactor":1.0}}],
    "meshes":[{"weights":[0.7],"primitives":[{"material":0,"attributes":{"POSITION":0,"NORMAL":1,"JOINTS_0":2,"WEIGHTS_0":3},"indices":4,"targets":[{"POSITION":5}]}]}],
    "buffers":[{"byteLength":bin.len()}],"bufferViews":views,
    "accessors":[
        {"bufferView":positions,"componentType":5126,"count":4,"type":"VEC3","min":[-1.,-1.,0.],"max":[1.,1.,0.]},
        {"bufferView":normals,"componentType":5126,"count":4,"type":"VEC3"},
        {"bufferView":joints,"componentType":5121,"count":4,"type":"VEC4"},
        {"bufferView":weights,"componentType":5126,"count":4,"type":"VEC4"},
        {"bufferView":indices,"componentType":5123,"count":6,"type":"SCALAR"},
        {"bufferView":morph,"componentType":5126,"count":4,"type":"VEC3","min":[0.,0.,0.],"max":[0.,0.,0.8]},
        {"bufferView":inverse,"componentType":5126,"count":1,"type":"MAT4"}
    ]});
    let mut encoded = serde_json::to_vec(&root).unwrap();
    while encoded.len() % 4 != 0 {
        encoded.push(b' ');
    }
    let mut bytes = Vec::new();
    for v in [
        0x46546c67u32,
        2,
        28 + encoded.len() as u32 + bin.len() as u32,
        encoded.len() as u32,
        0x4e4f534a,
    ] {
        bytes.extend(v.to_le_bytes());
    }
    bytes.extend(encoded);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(0x004e4942u32.to_le_bytes());
    bytes.extend(bin);
    std::fs::write(path, bytes).unwrap();
}
