use super::*;

#[test]
fn orthographic_zoom_projects_and_picks_surfaces_behind_the_eye_at_all_scales() {
    let rect = Rect::from_center_size(Vec2::splat(400.0), Vec2::new(800.0, 600.0));
    for radius in [0.0001, 1.0, 10000.0] {
        for zoom in [0.04, 0.2, 2.0, 10.0] {
            let viewer = Viewer {
                radius,
                distance: radius * zoom,
                yaw: 0.37,
                pitch: 0.23,
                orthographic: true,
                ..default()
            };
            let pose = view_pose(&viewer);
            let depth = orthographic_depth(&viewer);
            let lens = OrthographicProjection {
                near: -depth,
                far: depth,
                ..OrthographicProjection::default_3d()
            };
            let clip_from_world = bevy::camera::CameraProjection::get_clip_from_view(&lens)
                * pose.to_matrix().inverse();
            for x in [-0.5, 0.5] {
                for y in [-0.5, 0.5] {
                    for z in [-0.5, 0.5] {
                        let point = Vec3::new(x, y, z) * radius;
                        let clip = clip_from_world * point.extend(1.0);
                        assert!(
                            clip.z > 0.0 && clip.z < clip.w,
                            "Zoom must keep the full model in the orthographic depth slab"
                        );
                        let pixel = editing::input::project(&viewer, rect, point).unwrap();
                        let (origin, direction) = editing::input::ray(&viewer, rect, pixel);
                        let t = (point - origin).dot(direction);
                        assert!(
                            t > 0.0,
                            "Picking must start in front of the model, including surfaces behind the eye"
                        );
                        assert!((origin + direction * t).distance(point) < radius * 0.0001);
                    }
                }
            }
        }
    }
    let Projection::Orthographic(lens) = viewport::free_projection(true, 0.05) else {
        unreachable!()
    };
    assert!(lens.near < -1.0 && lens.far > 1.0);
}

#[test]
#[ignore = "requires GPU; zooms inside a solid model in all axis views and verifies surfaces, picking and handles"]
fn gpu_orthographic_zoom_keeps_model_faces_and_component_picking() {
    use crate::editor::{
        model_document::Mode,
        model_mesh::Element,
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture, mouse, move_pointer},
    };
    use bevy::{
        input::{
            ButtonState,
            mouse::{MouseScrollUnit, MouseWheel},
        },
        prelude::*,
    };
    fn warm(app: &mut App, count: usize) {
        for _ in 0..count {
            frame(app);
        }
    }
    fn edit(app: &mut App, action: editing::EditAction) {
        app.world_mut()
            .write_message(Action::ModelView(ModelViewAction::Edit(action)));
        warm(app, 3);
    }
    let (mut app, window, target) = fixture();
    let edits = app.world().resource::<Editor>().edits.clone();
    // The actual scene camera must also unproject from the front of its slab.
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.view = Transform::from_xyz(0.0, 0.5, 0.05);
        editor.pivot = Vec3::Y * 0.5;
        editor.view_orthographic = true;
    }
    warm(&mut app, 8);
    {
        let world = app.world_mut();
        let (camera, pose) = world
            .query_filtered::<(&Camera, &GlobalTransform), With<viewport::EditorCamera>>()
            .single(world)
            .unwrap();
        let ray = camera
            .viewport_to_world(pose, camera.logical_viewport_size().unwrap() * 0.5)
            .unwrap();
        assert!(ray.origin.z > 0.5 && ray.direction.z < -0.99);
        assert!(
            camera
                .world_to_viewport(pose, Vec3::new(0.0, 0.5, 0.5))
                .is_ok()
        );
    }
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<Viewer>().fitted {
            break;
        }
    }
    warm(&mut app, 8);
    edit(&mut app, editing::EditAction::Mode(Mode::Edit));
    let rect = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    let center = app.world().resource::<Viewer>().center;
    let objects = app.world().resource::<editing::State>().documents[&0]
        .objects
        .clone();
    let output =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/orthographic-zoom-qa");
    std::fs::create_dir_all(&output).unwrap();
    for (name, yaw, pitch) in [
        ("front", 0.0, 0.0),
        ("side", std::f32::consts::FRAC_PI_2, 0.0),
        ("top", 0.0, std::f32::consts::FRAC_PI_2),
    ] {
        edit(&mut app, editing::EditAction::Element(Element::Face));
        edit(&mut app, editing::EditAction::Deselect);
        {
            let mut viewer = app.world_mut().resource_mut::<Viewer>();
            viewer.center = center;
            viewer.yaw = yaw;
            viewer.pitch = pitch;
            viewer.distance = viewer.radius * 7.0;
            viewer.orthographic = true;
        }
        warm(&mut app, 3);
        move_pointer(&mut app, window, rect.center());
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 40.0,
            window,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        warm(&mut app, 8);
        assert!(
            app.world().resource::<Viewer>().distance
                < app.world().resource::<Viewer>().radius * 0.25
        );
        let image = capture_when(
            &mut app,
            &target,
            &output.join(format!("{name}-zoom.png")),
            |_| true,
        );
        let mut solid = 0;
        for x in 0..10 {
            for y in 0..10 {
                let p = rect.min
                    + rect.size() * Vec2::new(0.20 + x as f32 * 0.06, 0.20 + y as f32 * 0.06);
                let [r, g, b, _] = image.get_pixel(p.x as u32, p.y as u32).0.map(u16::from);
                solid += usize::from(r > 180 && g > 90 && r > b + 30);
            }
        }
        assert!(
            solid >= 95,
            "Zooming inside the model must leave its front face solid, without clipping: {name}, {solid}/100 samples"
        );
        let (object_id, face, vertex, point) = {
            let state = app.world().resource::<editing::State>();
            let viewer = app.world().resource::<Viewer>();
            let (origin, direction) = editing::input::ray(viewer, rect, rect.center());
            let bvh = editing::input::Bvh::build(&state.documents[&0]);
            let (object_id, face, _) = bvh.cast(origin, direction, None).unwrap();
            let object = state.documents[&0]
                .objects
                .iter()
                .find(|o| o.id == object_id)
                .unwrap();
            let polygon = &object.geometry.faces[face];
            let point = object
                .transform
                .transform_point(object.geometry.center(polygon));
            assert!(
                (point - view_pose(viewer).translation).dot(*view_pose(viewer).forward()) < 0.0
            );
            let vertex = polygon.corners[0].vertex;
            (
                object_id,
                face,
                vertex,
                object
                    .transform
                    .transform_point(object.geometry.positions[vertex]),
            )
        };
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        assert!(
            app.world().resource::<editing::State>().documents[&0]
                .selection
                .faces
                .contains(&face)
        );
        edit(&mut app, editing::EditAction::Element(Element::Vertex));
        edit(&mut app, editing::EditAction::Deselect);
        {
            let mut viewer = app.world_mut().resource_mut::<Viewer>();
            let pose = view_pose(&viewer);
            let delta = point - viewer.center;
            viewer.center +=
                *pose.right() * delta.dot(*pose.right()) + *pose.up() * delta.dot(*pose.up());
        }
        warm(&mut app, 5);
        let pixel = editing::input::project(app.world().resource::<Viewer>(), rect, point).unwrap();
        assert!(pixel.distance(rect.center()) < 0.1);
        move_pointer(&mut app, window, pixel);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        let state = app.world().resource::<editing::State>();
        let doc = &state.documents[&0];
        assert_eq!(doc.active, Some(object_id));
        assert!(
            doc.selection.vertices.contains(&vertex),
            "A vertex behind the eye must remain selectable: {name}"
        );
        let viewer = app.world().resource::<Viewer>();
        let frame = editing::handles::Frame::new(doc, viewer, rect, false).unwrap();
        let pixel_size = view_height(viewer, viewer.distance) / rect.height();
        assert!((frame.length / pixel_size - 87.5).abs() < 0.001);
        capture_when(
            &mut app,
            &target,
            &output.join(format!("{name}-vertex.png")),
            |_| true,
        );
    }
    assert_eq!(
        app.world().resource::<editing::State>().documents[&0].objects,
        objects
    );
    assert_eq!(app.world().resource::<Editor>().edits, edits);
    eprintln!("Orthographic zoom screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; verifies orthographic drafting backgrounds, world grids, pan/zoom and HDRI restoration"]
fn gpu_orthographic_workspaces_render_stable_backgrounds_and_grids() {
    use crate::editor::{
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::fixture,
    };
    use bevy::light::{EnvironmentMapLight, Skybox};

    fn warm(app: &mut App, count: usize) {
        for _ in 0..count {
            frame(app);
        }
    }
    fn bounds(world: &World, entity: Entity) -> Rect {
        let node = world.get::<ComputedNode>(entity).unwrap();
        let transform = world.get::<UiGlobalTransform>(entity).unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    }
    fn maps(world: &World, camera: Entity) -> (AssetId<Image>, AssetId<Image>, f32, Quat) {
        let light = world.get::<EnvironmentMapLight>(camera).unwrap();
        (
            light.diffuse_map.id(),
            light.specular_map.id(),
            light.intensity,
            light.rotation,
        )
    }
    fn check_background(image: &image::RgbaImage, rect: Rect) {
        // Sample the field outside the centered model, toolbar and navigator.
        let mut calm = 0;
        for x in [0.10, 0.22, 0.78, 0.90] {
            for y in [0.25, 0.55, 0.80] {
                let p = rect.min + rect.size() * Vec2::new(x, y);
                let [r, g, b, _] = image.get_pixel(p.x as u32, p.y as u32).0.map(u16::from);
                calm += usize::from(r.max(g).max(b) < 160 && b > r + 6 && b > g + 2);
            }
        }
        assert!(
            calm >= 10,
            "The entire orthographic field must have a calm blue-gray backdrop: {calm}/12"
        );
        let scan = |horizontal: bool, fraction: f32| {
            let mut previous = None;
            let mut transitions = 0;
            let extent = if horizontal {
                rect.width()
            } else {
                rect.height()
            };
            for i in (extent * 0.08) as u32..(extent * 0.92) as u32 {
                let p = rect.min
                    + if horizontal {
                        Vec2::new(i as f32, rect.height() * fraction)
                    } else {
                        Vec2::new(rect.width() * fraction, i as f32)
                    };
                let pixel = image.get_pixel(p.x as u32, p.y as u32).0;
                let value = (pixel[0] as i32 + pixel[1] as i32 + pixel[2] as i32) / 3;
                if previous.is_some_and(|old: i32| (old - value).abs() >= 3) {
                    transitions += 1;
                }
                previous = Some(value);
            }
            transitions
        };
        assert!(
            scan(true, 0.80) + scan(true, 0.25) >= 8,
            "The drafting grid needs visible vertical divisions"
        );
        assert!(
            scan(false, 0.10) + scan(false, 0.90) >= 8,
            "The drafting grid needs visible horizontal divisions"
        );
    }

    let (mut app, _, target) = fixture();
    let edits = app.world().resource::<Editor>().edits.clone();
    let settings = app.world().resource::<environment::Settings>().clone();
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/orthographic-background-qa");
    std::fs::create_dir_all(&output).unwrap();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.view = Transform::from_xyz(0.0, 0.0, 8.0);
        editor.pivot = Vec3::ZERO;
        editor.view_orthographic = true;
    }
    warm(&mut app, 12);
    let scene_camera = app
        .world_mut()
        .query_filtered::<Entity, With<viewport::EditorCamera>>()
        .single(app.world())
        .unwrap();
    assert!(app.world().get::<Skybox>(scene_camera).is_none());
    let scene_lighting = maps(app.world(), scene_camera);
    let scene_rect = bounds(app.world(), app.world().resource::<Interface>().viewport);
    let scene = capture_when(
        &mut app,
        &target,
        &output.join("01-scene-front.png"),
        |_| true,
    );
    check_background(&scene, scene_rect);

    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<Viewer>().fitted {
            break;
        }
    }
    warm(&mut app, 12);
    let camera = app
        .world_mut()
        .query_filtered::<Entity, With<ViewCamera>>()
        .single(app.world())
        .unwrap();
    for _ in 0..120 {
        if app
            .world()
            .get::<Skybox>(camera)
            .and_then(|sky| sky.image.as_ref())
            .is_some_and(|image| app.world().resource::<Assets<Image>>().contains(image))
        {
            break;
        }
        frame(&mut app);
    }
    assert!(app.world().get::<Skybox>(camera).is_some());
    let lighting = maps(app.world(), camera);
    let canvas = app
        .world_mut()
        .query_filtered::<Entity, With<Canvas>>()
        .single(app.world())
        .unwrap();
    let rect = bounds(app.world(), canvas);
    for (name, yaw, pitch) in [
        ("02-model-front", 0.0, 0.0),
        ("03-model-side", std::f32::consts::FRAC_PI_2, 0.0),
        ("04-model-top", 0.0, std::f32::consts::FRAC_PI_2),
    ] {
        {
            let mut viewer = app.world_mut().resource_mut::<Viewer>();
            viewer.yaw = yaw;
            viewer.pitch = pitch;
            viewer.distance = (viewer.radius * 5.0).max(4.0);
            viewer.orthographic = true;
        }
        warm(&mut app, 8);
        assert!(app.world().get::<Skybox>(camera).is_none());
        assert_eq!(maps(app.world(), camera), lighting);
        let image = capture_when(
            &mut app,
            &target,
            &output.join(format!("{name}.png")),
            |_| true,
        );
        check_background(&image, rect);
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        let pose = view_pose(&viewer);
        viewer.center += *pose.right() * 0.31 + *pose.up() * 0.19;
        viewer.distance *= 1.4;
    }
    warm(&mut app, 8);
    let panned = capture_when(
        &mut app,
        &target,
        &output.join("05-model-pan-zoom.png"),
        |_| true,
    );
    check_background(&panned, rect);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.orthographic = false;
        viewer.yaw = 0.55;
        viewer.pitch = 0.48;
    }
    warm(&mut app, 8);
    assert!(app.world().get::<Skybox>(camera).is_some());
    assert_eq!(maps(app.world(), camera), lighting);
    capture_when(
        &mut app,
        &target,
        &output.join("06-model-perspective.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Close));
    app.world_mut().resource_mut::<Editor>().view_orthographic = false;
    warm(&mut app, 8);
    assert!(app.world().get::<Skybox>(scene_camera).is_some());
    assert_eq!(maps(app.world(), scene_camera), scene_lighting);
    assert_eq!(*app.world().resource::<environment::Settings>(), settings);
    assert_eq!(app.world().resource::<Editor>().edits, edits);
    eprintln!("Orthographic background screenshots: {}", output.display());
}

#[test]
fn navigation_gizmo_orthographic_model_projection_uses_parallel_pick_rays() {
    let mut viewer = Viewer {
        distance: 8.0,
        orthographic: true,
        ..default()
    };
    let rect = Rect::from_center_size(Vec2::new(400.0, 300.0), Vec2::new(800.0, 600.0));
    let near = Vec3::new(1.0, 0.5, 2.0);
    let far = Vec3::new(1.0, 0.5, -3.0);
    let a = editing::input::project(&viewer, rect, near).unwrap();
    let b = editing::input::project(&viewer, rect, far).unwrap();
    assert!(
        a.abs_diff_eq(b, 0.0001),
        "Depth must not change orthographic magnification"
    );
    let (origin, direction) = editing::input::ray(&viewer, rect, a);
    assert!((origin + direction * (origin.z - near.z)).abs_diff_eq(near, 0.0001));
    let (other, parallel) = editing::input::ray(&viewer, rect, a + Vec2::new(70.0, 20.0));
    assert!(parallel.abs_diff_eq(direction, 0.00001));
    assert!(!origin.abs_diff_eq(other, 0.01));
    viewer.orthographic = false;
    assert!(
        !editing::input::project(&viewer, rect, near)
            .unwrap()
            .abs_diff_eq(editing::input::project(&viewer, rect, far).unwrap(), 0.1)
    );
}

#[test]
fn navigation_gizmo_model_pan_and_zoom_keep_orthographic_but_orbit_restores_perspective() {
    let mut viewer = Viewer {
        distance: 8.0,
        radius: 1.0,
        orthographic: true,
        ..default()
    };
    let mut window = Window {
        focused: true,
        ..default()
    };
    window.set_cursor_position(Some(Vec2::splat(200.0)));
    let rect = Rect::from_corners(Vec2::ZERO, Vec2::splat(600.0));
    let mut mouse = ButtonInput::default();
    let mut keys = ButtonInput::default();
    let scroll = AccumulatedMouseScroll::default();
    mouse.press(MouseButton::Middle);
    navigation::drive(
        &mut viewer,
        &window,
        &mouse,
        &keys,
        &scroll,
        0.016,
        Vec2::new(12.0, 2.0),
        rect,
        false,
    );
    assert!(viewer.orthographic);
    mouse.release(MouseButton::Middle);
    mouse.clear();
    navigation::drive(
        &mut viewer,
        &window,
        &mouse,
        &keys,
        &AccumulatedMouseScroll {
            delta: Vec2::Y,
            ..default()
        },
        0.016,
        Vec2::ZERO,
        rect,
        false,
    );
    assert!(viewer.orthographic);
    mouse.press(MouseButton::Left);
    keys.press(KeyCode::AltLeft);
    navigation::drive(
        &mut viewer,
        &window,
        &mouse,
        &keys,
        &scroll,
        0.016,
        Vec2::new(12.0, 2.0),
        rect,
        false,
    );
    assert!(!viewer.orthographic);
}

#[test]
fn fitting_handles_tall_wide_and_tiny_models() {
    for radius in [0.0001, 1.0, 10000.0] {
        for aspect in [0.3, 1.0, 3.0] {
            let distance = fit_distance(radius, aspect);
            let angle = (FOV * 0.5).min(((FOV * 0.5).tan() * aspect).atan());
            assert!(distance.is_finite());
            assert!(distance * angle.sin() > radius);
        }
    }
}

#[test]
fn imported_cameras_and_lights_are_isolated_to_the_viewer() {
    use bevy::world_serialization::WorldInstanceSpawner;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let root = app.world_mut().spawn_empty().observe(prepare).id();
    let child = app
        .world_mut()
        .spawn((
            Mesh3d::default(),
            Camera::default(),
            PointLight::default(),
            ChildOf(root),
        ))
        .id();
    let scene_light = app.world_mut().spawn(PointLight::default()).id();
    let mut assets = Assets::<WorldAsset>::default();
    let handle = assets.add(WorldAsset::new(World::new()));
    app.insert_resource(assets);
    let instance_id = WorldInstanceSpawner::default()
        .spawn_sync(app.world_mut(), handle.id())
        .unwrap();
    app.world_mut().trigger(WorldInstanceReady {
        entity: root,
        instance_id,
    });
    app.update();
    assert!(app.world().get::<Prepared>(root).is_some());
    assert_eq!(
        app.world().get::<RenderLayers>(child),
        Some(&RenderLayers::layer(LAYER))
    );
    assert!(!app.world().get::<Camera>(child).unwrap().is_active);
    assert!(app.world().get::<PointLight>(child).is_none());
    assert!(app.world().get::<PointLight>(scene_light).is_some());
    assert!(app.world().get::<Mesh3d>(child).is_some());
}

#[test]
#[ignore = "requires GPU adapter; opens a model card, orbits, zooms and closes without editing the scene"]
fn gpu_model_viewer_opens_renders_navigates_and_releases_its_scene() {
    use crate::editor::{
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture, mouse as click_button, move_pointer},
    };
    use bevy::input::ButtonState;
    let output =
        std::env::temp_dir().join(format!("bevy-editor-model-viewer-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    let (mut app, window, target) = fixture();
    let before = app.world().resource::<Editor>().edits.clone();
    let selection = app.world().resource::<Editor>().selection;
    let history = app.world().resource::<Editor>().undo.clone();
    app.world_mut().write_message(Action::OpenBrowser);
    frame(&mut app);
    let folder = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == std::path::Path::new("models"))
        .unwrap();
    app.world_mut().write_message(Action::Browse(folder));
    for _ in 0..25 {
        frame(&mut app);
    }
    let card = {
        let world = app.world_mut();
        let (_, node, transform) = world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(label, _, _)| label.0 == "Открыть модель block.glb · Блок")
            .unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    };
    move_pointer(&mut app, window, card);
    click_button(&mut app, window, ButtonState::Pressed);
    click_button(&mut app, window, ButtonState::Released);
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<Viewer>().fitted {
            break;
        }
    }
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(0));
    let removed_title = {
        let editor = app.world().resource::<Editor>();
        let model = &editor.project.catalog[0];
        format!("{} · {}", model.label, model.asset)
    };
    assert!(
        app.world_mut()
            .query::<&Text>()
            .iter(app.world())
            .all(|text| text.0 != removed_title),
        "The model viewer must not repeat the model name and path above the canvas"
    );
    assert!(
        app.world_mut()
            .query::<&Control>()
            .iter(app.world())
            .all(|control| !matches!(
                control.action,
                Action::ModelView(
                    ModelViewAction::Close | ModelViewAction::Fit | ModelViewAction::Retry
                )
            )),
        "The model viewer must not have the removed header buttons"
    );
    let ui = app.world().resource::<ViewerUi>();
    let overlay = ui.overlay;
    assert!(
        app.world().get::<TabGroup>(overlay).is_none(),
        "model documents must not trap focus in a modal"
    );
    assert_eq!(
        app.world().get::<ChildOf>(overlay).unwrap().parent(),
        app.world().resource::<Interface>().viewport
    );
    assert!(
        app.world().resource::<Viewer>().fitted,
        "{:?}",
        app.world().resource::<Viewer>().error
    );
    if app.world().resource::<Editor>().drawer {
        app.world_mut().write_message(Action::Drawer);
        for _ in 0..25 {
            frame(&mut app);
        }
    }
    let workspace_rect = |app: &mut App| {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        Rect::from_center_size(transform.affine().translation, node.size())
    };
    let fixed = workspace_rect(&mut app);
    let camera_before = {
        let viewer = app.world().resource::<Viewer>();
        (viewer.center, viewer.distance, viewer.yaw, viewer.pitch)
    };
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
        assert_eq!(
            workspace_rect(&mut app),
            fixed,
            "The file manager must overlay, rather than shrink, the model workspace"
        );
    }
    let point = Vec2::new(
        fixed.center().x,
        900.0 - FOOT - app.world().resource::<DrawerMotion>().visible_height + 64.0,
    );
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.world_mut()
        .write_message(bevy::input::mouse::MouseButtonInput {
            window,
            button: MouseButton::Right,
            state: ButtonState::Pressed,
        });
    frame(&mut app);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point + Vec2::new(60.0, 10.0)));
    frame(&mut app);
    app.world_mut()
        .write_message(bevy::input::mouse::MouseButtonInput {
            window,
            button: MouseButton::Right,
            state: ButtonState::Released,
        });
    frame(&mut app);
    {
        let viewer = app.world().resource::<Viewer>();
        assert_eq!(
            (viewer.center, viewer.distance, viewer.yaw, viewer.pitch),
            camera_before,
            "Dragging on the file manager must not navigate the model underneath"
        );
    }
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
        assert_eq!(workspace_rect(&mut app), fixed);
    }
    let center = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    };
    let material_sample = {
        let rect = {
            let world = app.world_mut();
            let (node, transform) = world
                .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
                .single(world)
                .unwrap();
            Rect::from_center_size(
                transform.affine().translation * node.inverse_scale_factor(),
                node.size() * node.inverse_scale_factor(),
            )
        };
        let state = app.world().resource::<editing::State>();
        let viewer = app.world().resource::<Viewer>();
        let doc = &state.documents[&0];
        let handles = editing::handles::Frame::new(doc, viewer, rect, state.local).unwrap();
        let bvh = editing::input::Bvh::build(doc);
        (-18..=18)
            .step_by(3)
            .flat_map(|x| {
                (-18..=18)
                    .step_by(3)
                    .map(move |y| center + Vec2::new(x as f32, y as f32))
            })
            .find(|&p| {
                let (o, d) = editing::input::ray(viewer, rect, p);
                handles.hit(viewer, rect, state.tool, p).is_none() && bvh.cast(o, d, None).is_some()
            })
            .expect("A visible model pixel must remain outside the handle overlay")
    };
    let baseline = capture_when(&mut app, &target, &output.join("model.png"), |image| {
        let mut colored = 0;
        for y in center.y as u32 - 80..center.y as u32 + 80 {
            for x in center.x as u32 - 80..center.x as u32 + 80 {
                if image.get_pixel(x, y)[0] > image.get_pixel(x, y)[2].saturating_add(15) {
                    colored += 1;
                }
            }
        }
        colored > 200
    });
    let exposure = {
        let world = app.world_mut();
        world
            .query::<(Entity, &AccessibleLabel)>()
            .iter(world)
            .find(|(_, label)| label.0 == "Просмотр: Экспозиция, EV")
            .unwrap()
            .0
    };
    let brightness = |image: &image::RgbaImage| -> u32 {
        // Sample the material away from the shared pivot and handle overlay.
        let pixel = image.get_pixel(material_sample.x as u32, material_sample.y as u32);
        pixel.0[..3].iter().map(|value| *value as u32).sum()
    };
    app.world_mut().trigger(bevy::ui_widgets::ValueChange {
        source: exposure,
        value: -2.0_f32,
        is_final: false,
    });
    frame(&mut app);
    capture_when(&mut app, &target, &output.join("model-dark.png"), |image| {
        brightness(image) + 50 < brightness(&baseline)
    });
    app.world_mut().trigger(bevy::ui_widgets::ValueChange {
        source: exposure,
        value: 0.0_f32,
        is_final: false,
    });
    frame(&mut app);
    frame(&mut app);
    let yaw = app.world().resource::<Viewer>().yaw;
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(center));
    app.world_mut()
        .write_message(bevy::input::mouse::MouseButtonInput {
            window,
            button: MouseButton::Right,
            state: ButtonState::Pressed,
        });
    frame(&mut app);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(center + Vec2::new(80.0, 20.0)));
    frame(&mut app);
    assert_ne!(app.world().resource::<Viewer>().yaw, yaw);
    app.world_mut()
        .write_message(bevy::input::mouse::MouseButtonInput {
            window,
            button: MouseButton::Right,
            state: ButtonState::Released,
        });
    frame(&mut app);
    let distance = app.world().resource::<Viewer>().distance;
    app.world_mut()
        .write_message(bevy::input::mouse::MouseWheel {
            window,
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 2.0,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
    frame(&mut app);
    assert!(app.world().resource::<Viewer>().distance < distance);
    let pose = view_pose(app.world().resource::<Viewer>());
    // Use the real tab controls to return to the scene, then to the model.
    let click_tab = |app: &mut App, predicate: fn(&Control) -> bool| {
        let position = {
            let world = app.world_mut();
            let (_, node, transform) = world
                .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
                .iter(world)
                .find(|(control, _, _)| predicate(control))
                .unwrap();
            transform.affine().translation * node.inverse_scale_factor()
        };
        move_pointer(app, window, position);
        click_button(app, window, ButtonState::Pressed);
        click_button(app, window, ButtonState::Released);
        frame(app);
    };
    click_tab(&mut app, |control| {
        matches!(control.action, Action::Open(0)) && matches!(control.look, Look::Tab)
    });
    assert!(app.world().resource::<Editor>().viewing_model.is_none());
    assert_eq!(app.world().resource::<Editor>().model_tabs, [0]);
    assert!(app.world().resource::<Viewer>().stage.is_none());
    click_tab(&mut app, |control| {
        matches!(control.action, Action::ViewModel(0)) && matches!(control.look, Look::Tab)
    });
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<Viewer>().fitted {
            break;
        }
    }
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(0));
    assert_eq!(view_pose(app.world().resource::<Viewer>()), pose);
    app.world_mut().write_message(Action::ViewModel(1));
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<Viewer>().fitted {
            break;
        }
    }
    assert_eq!(app.world().resource::<Editor>().model_tabs, [0, 1]);
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(1));
    // Closing an inactive model leaves the selected model open.
    click_tab(&mut app, |control| {
        matches!(control.action, Action::CloseModelTab(0))
    });
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(1));
    assert_eq!(app.world().resource::<Editor>().model_tabs, [1]);
    click_tab(&mut app, |control| {
        matches!(control.action, Action::CloseModelTab(1))
    });
    assert!(app.world().resource::<Editor>().viewing_model.is_none());
    assert!(app.world().resource::<Editor>().model_tabs.is_empty());
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Close));
    frame(&mut app);
    let editor = app.world().resource::<Editor>();
    assert!(editor.viewing_model.is_none());
    assert_eq!(editor.edits, before);
    assert_eq!(editor.selection, selection);
    assert_eq!(editor.undo, history);
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<ViewCamera>>()
            .iter(app.world())
            .count(),
        0
    );
    // Model inspection remains available when the scene workspace is closed.
    app.world_mut().write_message(Action::CloseTab(0));
    frame(&mut app);
    assert!(!app.world().resource::<Editor>().has_scene());
    app.world_mut().write_message(Action::ViewModel(1));
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<Viewer>().fitted {
            break;
        }
    }
    assert!(app.world().resource::<Viewer>().fitted);
    assert!(!app.world().resource::<Editor>().has_scene());
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Close));
    frame(&mut app);
    let invalid = {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let mut missing = editor.project.catalog[0].clone();
        missing.asset = "models/missing-preview.glb".into();
        let index = editor.project.catalog.len();
        editor.project.catalog.push(missing);
        index
    };
    app.world_mut().write_message(Action::ViewModel(invalid));
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<Viewer>().error.is_some() {
            break;
        }
    }
    assert!(app.world().resource::<Viewer>().error.is_some());
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Close));
    frame(&mut app);
    assert!(app.world().resource::<Viewer>().stage.is_none());
    println!("Model viewer GPU artifacts: {}", output.display());
}
