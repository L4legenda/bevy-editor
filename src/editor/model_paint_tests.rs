use super::*;
use crate::editor::{
    model_document::Mode,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
};
use bevy::input::{
    ButtonState,
    keyboard::{Key, KeyboardInput},
};

#[test]
fn paint_workspace_layer_stack_keeps_order_visibility_and_undo() {
    let mut session = Session::default();
    session.edit(PaintAction::Add(LayerKind::Fill));
    session.edit(PaintAction::Duplicate);
    assert_eq!(session.stack.layers.len(), 3);
    assert_eq!(session.stack.selected, 2);
    session.edit(PaintAction::Visible(2));
    assert!(!session.stack.layers[2].visible);
    session.edit(PaintAction::Undo);
    assert!(session.stack.layers[2].visible);
    session.edit(PaintAction::Undo);
    assert_eq!(session.stack.layers.len(), 2);
    session.edit(PaintAction::Redo);
    assert_eq!(session.stack.layers.len(), 3);
    session.edit(PaintAction::Move(-1));
    assert_eq!(session.stack.selected, 1);
    assert!(session.stack.layers[1].name.ends_with("копия"));
    session.edit(PaintAction::Remove);
    assert_eq!(session.stack.layers.len(), 2);
}

fn warm(app: &mut App, n: usize) {
    for _ in 0..n {
        frame(app);
    }
}
fn click(app: &mut App, window: Entity, action: impl Fn(Action) -> bool) {
    let position = {
        let world = app.world_mut();
        let (_, node, transform) = world
            .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(control, node, _)| node.size().min_element() > 0. && action(control.action))
            .expect("Visible paint control");
        transform.affine().translation * node.inverse_scale_factor()
    };
    move_pointer(app, window, position);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}
fn paint_click(app: &mut App, window: Entity, action: PaintAction) {
    paint_ready(app);
    click(
        app,
        window,
        |a| matches!(a, Action::ModelView(ModelViewAction::Paint(value)) if value == action),
    );
    paint_ready(app);
}
fn paint_ready(app: &mut App) {
    for _ in 0..600 {
        frame(app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .values()
            .all(|d| !d.paint_busy)
        {
            return;
        }
    }
    panic!("Paint worker did not finish");
}
#[test]
#[ignore = "requires GPU; paints through the real model viewport, composites layers, saves and reopens GLB"]
fn gpu_model_paint_strokes_layers_save_and_reopen() {
    use crate::editor::{model_document::glb::Glb, model_primitives::Primitive};
    let directory = std::env::temp_dir().join(format!(
        "bevy-model-paint-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &to.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let scene = app.world().resource::<Editor>().edits.clone();
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let mut geometry = Primitive::Cube
            .geometry(&Primitive::Cube.defaults())
            .unwrap();
        // Give each triangle its own UV tile so the test also checks isolation.
        for (i, face) in geometry.faces.iter_mut().enumerate() {
            for c in &mut face.corners {
                c.uv = (c.uv * 0.8 + Vec2::splat(0.1) + Vec2::new((i % 4) as f32, (i / 4) as f32))
                    / Vec2::new(4., 3.);
            }
        }
        doc.objects[0].geometry = geometry.into();
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 8);
    paint_ready(&mut app);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.size = 110.;
        session.hardness = 80.;
        session.color = 3;
    }
    let rect = labelled(&mut app, "3D-вид модели");
    let world = Vec3::new(0., 0., 0.5);
    let center = editing::input::project(app.world().resource::<Viewer>(), rect, world).unwrap();
    move_pointer(&mut app, window, center - Vec2::new(24., 0.));
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::new(24., 0.));
    let wait_worker = |app: &mut App| {
        let started = std::time::Instant::now();
        loop {
            frame(app);
            if app.world().resource::<engine::Runtime>().worker_idle(0) {
                break;
            }
            assert!(
                started.elapsed().as_secs() < 20,
                "paint worker did not settle"
            );
        }
    };
    wait_worker(&mut app);
    warm(&mut app, 3);
    let ids = app.world().resource::<engine::Runtime>().texture_ids(0);
    let revision = app.world().resource::<PaintState>().sessions[&0].revision;
    let mut events = bevy::ecs::message::MessageCursor::<bevy::asset::AssetEvent<Image>>::default();
    events.clear(
        app.world()
            .resource::<Messages<bevy::asset::AssetEvent<Image>>>(),
    );
    move_pointer(&mut app, window, center + Vec2::new(38., 0.));
    wait_worker(&mut app);
    warm(&mut app, 3);
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0].revision,
        revision,
        "pointer samples must not rebuild layer panels"
    );
    assert_eq!(
        app.world().resource::<engine::Runtime>().texture_ids(0),
        ids
    );
    assert!(
        !events
            .read(
                app.world()
                    .resource::<Messages<bevy::asset::AssetEvent<Image>>>()
            )
            .any(|event| matches!(event,bevy::asset::AssetEvent::Modified{id}if ids.contains(id))),
        "strokes must patch existing GPU textures without full image reuploads"
    );
    warm(&mut app, 12);
    assert!(
        app.world().resource::<engine::Runtime>().worker_idle(0),
        "holding a stationary brush must not enqueue more stamps"
    );
    let (batches, stamps, readbacks) = app.world().resource::<engine::gpu::Queue>().statistics();
    assert!(
        batches > 0 && stamps > 0,
        "live strokes must execute on the GPU"
    );
    assert_eq!(
        readbacks, 0,
        "painting a held stroke must not read pixel data back to the CPU"
    );
    mouse(&mut app, window, ButtonState::Released);
    // Saving immediately after release must include queued worker stamps.
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(
            editing::EditAction::Save,
        )));
    let saving = std::time::Instant::now();
    loop {
        frame(&mut app);
        if !app.world().resource::<editing::State>().documents[&0].dirty() {
            break;
        }
        assert!(
            saving.elapsed().as_secs() < 90,
            "queued stroke save failed: {}",
            app.world().resource::<Editor>().status
        );
    }
    paint_ready(&mut app);
    let painted = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
        .paint
        .clone()
        .expect("3D stroke must produce editable paint pixels");
    assert!(
        painted.stack.layers[0].channels[0]
            .as_ref()
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[3] > 0)
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/model-paint-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("05-real-3d-paint.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    // Paint a second independent layer directly in the 2D texture view.
    let uv = {
        let w = app.world_mut();
        let (n, t) = w
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<engine::TextureCanvas>>()
            .single(w)
            .unwrap();
        Rect::from_center_size(
            t.affine().translation * n.inverse_scale_factor(),
            n.size() * n.inverse_scale_factor(),
        )
    };
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        paint.sessions.get_mut(&0).unwrap().color = 4;
    }
    let at = {
        let doc = &app.world().resource::<editing::State>().documents[&0];
        let face = &doc.objects[0].geometry.faces[0];
        let center = face.corners.iter().map(|c| c.uv).sum::<Vec2>() / face.corners.len() as f32;
        uv.min + uv.size() * center
    };
    move_pointer(&mut app, window, at);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let two = app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
        .paint
        .clone()
        .unwrap();
    assert!(
        two.stack.layers[1].channels[0].is_some(),
        "2D view must also paint"
    );
    paint_click(&mut app, window, PaintAction::Visible(1));
    assert!(
        !app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap()
            .stack
            .layers[1]
            .visible
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert!(
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap()
            .stack
            .layers[1]
            .visible
    );
    capture_when(
        &mut app,
        &target,
        &output.join("06-layered-paint.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(
            editing::EditAction::Save,
        )));
    let started = std::time::Instant::now();
    loop {
        frame(&mut app);
        if !app.world().resource::<editing::State>().documents[&0].dirty() {
            break;
        }
        assert!(
            started.elapsed().as_secs() < 90,
            "save failed: {}",
            app.world().resource::<Editor>().status
        );
    }
    let saved = Glb::load(&path).unwrap();
    let node = saved.json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["mesh"].is_number())
        .unwrap();
    let mesh = node["mesh"].as_u64().unwrap() as usize;
    let restored = saved
        .paint_surface(&saved.json["meshes"][mesh]["primitives"][0])
        .unwrap()
        .unwrap();
    assert_eq!(restored.stack, two.stack);
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 8);
    app.world_mut().write_message(Action::ViewModel(0));
    let started = std::time::Instant::now();
    while !app
        .world()
        .resource::<editing::State>()
        .documents
        .contains_key(&0)
    {
        frame(&mut app);
        assert!(started.elapsed().as_secs() < 40, "reopen failed");
    }
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
            .paint
            .as_ref()
            .unwrap()
            .stack,
        two.stack
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    drop(app);
    assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(directory).unwrap();
}
fn labelled(app: &mut App, label: &str) -> Rect {
    let world = app.world_mut();
    let (_, node, transform) = world
        .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .find(|(value, _, _)| value.0 == label)
        .unwrap();
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}
#[test]
#[ignore = "requires GPU; verifies all PBR channels, occlusion, opacity, erasing, fill and orthographic painting"]
fn gpu_model_paint_channels_erase_and_occlusion() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let mut mesh = Geometry::default();
    for (z, left, right) in [(0., 0.05, 0.45), (-1., 0.55, 0.95)] {
        let start = mesh.positions.len();
        mesh.positions.extend([
            Vec3::new(-1., -1., z),
            Vec3::new(1., -1., z),
            Vec3::new(1., 1., z),
            Vec3::new(-1., 1., z),
        ]);
        let uv = [
            Vec2::new(left, 0.05),
            Vec2::new(right, 0.05),
            Vec2::new(right, 0.95),
            Vec2::new(left, 0.95),
        ];
        mesh.faces.push(Face {
            corners: (0..4)
                .map(|i| {
                    let mut c = Corner::new(start + i);
                    c.uv = uv[i];
                    c
                })
                .collect(),
            material: 0,
            smooth: false,
        });
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = mesh.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.radius = 2.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
    }
    warm(&mut app, 5);
    paint_ready(&mut app);
    let center = labelled(&mut app, "3D-вид модели").center();
    for channel in 0..8 {
        {
            let mut paint = app.world_mut().resource_mut::<PaintState>();
            let s = paint.sessions.get_mut(&0).unwrap();
            s.channel = channel;
            s.color = 3;
            s.opacity = if channel == 4 { 0. } else { 100. };
            s.roughness = 80.;
            s.metallic = 60.;
            s.flow = 100.;
            s.hardness = 100.;
            s.size = 90.;
            s.tool = PaintTool::Brush;
        }
        move_pointer(&mut app, window, center);
        mouse(&mut app, window, ButtonState::Pressed);
        for _ in 0..20 {
            frame(&mut app);
        }
        if channel == 0 {
            assert_eq!(
                app.world().resource::<engine::gpu::Queue>().statistics().2,
                0
            );
            let uv = {
                let w = app.world_mut();
                let(n,t)=w.query_filtered::<(&ComputedNode,&UiGlobalTransform),With<engine::TextureCanvas>>().single(w).unwrap();
                Rect::from_center_size(
                    t.affine().translation * n.inverse_scale_factor(),
                    n.size() * n.inverse_scale_factor(),
                )
            };
            let point = uv.min + uv.size() * Vec2::new(0.25, 0.5);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/model-paint-qa/07-gpu-live-before-readback.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            capture_when(&mut app, &target, &path, |image| {
                let p = image.get_pixel(point.x as u32, point.y as u32);
                p[0].abs_diff(164) < 4 && p[1].abs_diff(61) < 4 && p[2].abs_diff(68) < 4
            });
            assert!(
                app.world().resource::<editing::State>().documents[&0].objects[0].materials[0]
                    .paint
                    .is_none(),
                "live color must be visible before a CPU snapshot exists"
            );
        }
        mouse(&mut app, window, ButtonState::Released);
        paint_ready(&mut app);
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .clone()
        .unwrap();
        let p = surface.stack.layers[0].channels[channel].as_ref().unwrap();
        let size = surface.size as usize;
        let front = (size / 2 * size + size / 4) * 4;
        let back = (size / 2 * size + size * 3 / 4) * 4;
        let expected = match channel {
            0 | 5 => [164, 61, 68, 255],
            1 => [204, 204, 204, 255],
            2 => [153, 153, 153, 255],
            4 => [0, 0, 0, 255],
            6 => [128, 128, 255, 255],
            _ => [255; 4],
        };
        assert_eq!(&p[front..front + 4], &expected, "GPU channel {channel}");
        assert_eq!(p[back + 3], 0, "GPU stroke must not paint the rear plane");
    }
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Paint(
            PaintAction::Channel(0),
        )));
    warm(&mut app, 2);
    paint_ready(&mut app);
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Eraser));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let pixel = |app: &App, x: usize, y: usize| {
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .as_ref()
        .unwrap();
        let i = (y * surface.size as usize + x) * 4;
        surface.stack.layers[0].channels[0].as_ref().unwrap()[i..i + 4].to_vec()
    };
    assert_eq!(pixel(&app, 256, 512)[3], 0);
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(pixel(&app, 256, 512)[3], 255);
    paint_click(&mut app, window, PaintAction::Color(4));
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Fill));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 103, 103), [67, 118, 166, 255]);
    assert_eq!(pixel(&app, 768, 512)[3], 0);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.orthographic = true;
        viewer.distance = 0.05;
    }
    paint_click(&mut app, window, PaintAction::Tool(PaintTool::Brush));
    paint_click(&mut app, window, PaintAction::Color(3));
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(pixel(&app, 256, 512), [164, 61, 68, 255]);
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let s = paint.sessions.get_mut(&0).unwrap();
        s.opacity = 50.;
        s.flow = 100.;
        s.hardness = 100.;
    }
    // Queue two strokes without waiting for the first readback: GPU snapshots
    // and CPU undo records must retain their submission order.
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    let alpha = |app: &App| {
        let surface = app.world().resource::<editing::State>().documents[&0].objects[0].materials
            [0]
        .paint
        .as_ref()
        .unwrap();
        let i = ((surface.size / 2) * surface.size + surface.size / 4) as usize * 4 + 3;
        surface.stack.layers[1].channels[0].as_ref().unwrap()[i]
    };
    assert_eq!(
        alpha(&app),
        192,
        "second stroke must build on the first GPU layer snapshot"
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(alpha(&app), 128, "one undo record per completed GPU stroke");
}
#[test]
#[ignore = "GPU throughput benchmark on 40k triangles; no readback during the held stroke"]
fn gpu_model_paint_benchmark_40k() {
    use crate::editor::model_mesh::{Corner, Face, Geometry};
    use bevy::render::{RenderApp, render_resource::PollType, renderer::RenderDevice};
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let mut mesh = Geometry::default();
    let nx = 100;
    let ny = 200;
    for y in 0..ny {
        for x in 0..nx {
            let start = mesh.positions.len();
            let uv = [
                Vec2::new(x as f32 / nx as f32, y as f32 / ny as f32),
                Vec2::new((x + 1) as f32 / nx as f32, y as f32 / ny as f32),
                Vec2::new((x + 1) as f32 / nx as f32, (y + 1) as f32 / ny as f32),
                Vec2::new(x as f32 / nx as f32, (y + 1) as f32 / ny as f32),
            ];
            mesh.positions
                .extend(uv.map(|p| Vec3::new(p.x * 2. - 1., p.y * 2. - 1., 0.)));
            mesh.faces.push(Face {
                corners: (0..4)
                    .map(|i| {
                        let mut c = Corner::new(start + i);
                        c.uv = uv[i];
                        c
                    })
                    .collect(),
                material: 0,
                smooth: false,
            });
        }
    }
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = mesh.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.mode = Mode::Paint;
        doc.revision += 1;
    }
    warm(&mut app, 5);
    paint_ready(&mut app);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.;
        viewer.radius = 2.;
        viewer.yaw = 0.;
        viewer.pitch = 0.;
    }
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let s = paint.sessions.get_mut(&0).unwrap();
        s.size = 80.;
        s.flow = 35.;
        s.hardness = 60.;
        s.color = 3;
    }
    warm(&mut app, 3);
    let center = labelled(&mut app, "3D-вид модели").center();
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    let warmup = std::time::Instant::now();
    loop {
        frame(&mut app);
        if app.world().resource::<engine::Runtime>().worker_idle(0) {
            break;
        }
        assert!(warmup.elapsed().as_secs() < 30);
    }
    let wait = |app: &mut App| {
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
    };
    wait(&mut app);
    let baseline_start = std::time::Instant::now();
    for _ in 0..120 {
        app.update();
    }
    wait(&mut app);
    let baseline = baseline_start.elapsed().as_secs_f64();
    let start = std::time::Instant::now();
    let frames = 120;
    for i in 0..frames {
        let x = ((i % 60) as f32 - 30.) * 2.;
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(center + Vec2::new(x, 0.)));
        app.update();
    }
    wait(&mut app);
    let seconds = start.elapsed().as_secs_f64();
    let (batches, stamps, readbacks) = app.world().resource::<engine::gpu::Queue>().statistics();
    assert_eq!(readbacks, 0);
    assert!(batches > 10 && stamps > 20);
    eprintln!(
        "GPU PAINT: triangles=40000 texture=1024 frames={frames} total_ms={:.2} avg_frame_ms={:.2} batches={batches} stamps={stamps} in_stroke_readbacks={readbacks} baseline_avg_ms={:.2} paint_overhead_ms={:.2}",
        seconds * 1000.,
        seconds * 1000. / frames as f64,
        baseline * 1000. / 120.,
        (seconds - baseline) * 1000. / 120.
    );
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(center));
    mouse(&mut app, window, ButtonState::Released);
    paint_ready(&mut app);
    assert_eq!(
        app.world().resource::<engine::gpu::Queue>().statistics().2,
        1
    );
}

#[test]
#[ignore = "requires GPU; exercises the native Material Paint layout, panel controls and mode isolation"]
fn gpu_model_paint_workspace_layout_keeps_model_and_scene_intact() {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    warm(&mut app, 8);
    let original = app.world().resource::<editing::State>().documents[&0]
        .objects
        .clone();
    let scene = app.world().resource::<Editor>().edits.clone();
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Menu))
        )
    });
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(
                Mode::Paint
            )))
        )
    });
    warm(&mut app, 15);
    assert_eq!(
        app.world().resource::<editing::State>().mode(Some(0)),
        Mode::Paint
    );
    assert!(!app.world().resource::<editing::State>().available(
        editing::EditAction::Operate(crate::editor::model_document::Operation::Extrude),
        Some(0)
    ));
    let side = labelled(&mut app, "Рисование: слои и свойства");
    let shelf = labelled(&mut app, "Рисование: библиотека ресурсов");
    let uv = labelled(&mut app, "Рисование: 2D-вид UV");
    let view = labelled(&mut app, "3D-вид модели");
    assert!(
        side.width() > 200. && shelf.height() > 150. && uv.width() > 200. && view.width() > 200.
    );
    assert!(view.max.x <= uv.min.x + 2. && uv.max.x <= side.min.x + 2.);
    assert!(view.max.y <= shelf.min.y + 2. && uv.max.y <= shelf.min.y + 2.);
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/model-paint-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("01-paint-workspace.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::Add(LayerKind::Paint));
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0]
            .stack
            .layers
            .len(),
        2
    );
    paint_click(&mut app, window, PaintAction::Undo);
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0]
            .stack
            .layers
            .len(),
        1
    );
    paint_click(&mut app, window, PaintAction::Panel(Panel::Sets));
    paint_click(&mut app, window, PaintAction::Set(0));
    capture_when(
        &mut app,
        &target,
        &output.join("02-texture-sets.png"),
        |_| true,
    );
    paint_click(&mut app, window, PaintAction::Panel(Panel::Preview));
    paint_click(&mut app, window, PaintAction::View(View::Three));
    assert_eq!(
        labelled(&mut app, "Рисование: 2D-вид UV").size(),
        Vec2::ZERO
    );
    assert!(labelled(&mut app, "3D-вид модели").width() > view.width() * 1.7);
    paint_click(&mut app, window, PaintAction::Reset);
    paint_click(&mut app, window, PaintAction::Category(Category::Brushes));
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyE,
            logical_key: Key::Character("e".into()),
            state,
            text: None,
            repeat: false,
            window,
        });
        warm(&mut app, 3);
    }
    assert_eq!(
        app.world().resource::<PaintState>().sessions[&0].tool,
        PaintTool::Eraser
    );
    capture_when(&mut app, &target, &output.join("03-brushes.png"), |_| true);
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects,
        original
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Menu))
        )
    });
    click(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(
                Mode::Object
            )))
        )
    });
    assert_eq!(
        app.world().resource::<editing::State>().mode(Some(0)),
        Mode::Object
    );
    let shelf_hidden = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &Node)>()
            .iter(world)
            .find(|(label, _)| label.0 == "Рисование: библиотека ресурсов")
            .unwrap()
            .1
            .display
    };
    assert_eq!(shelf_hidden, Display::None);
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects,
        original
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-return-to-object.png"),
        |_| true,
    );
    eprintln!("Paint layout screenshots: {}", output.display());
}
