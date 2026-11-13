use super::*;

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
    let center = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    };
    let baseline = capture_when(&mut app, &target, &output.join("model.png"), |image| {
        let mut colored = 0;
        for y in center.y as u32 - 80..center.y as u32 + 80 {
            for x in center.x as u32 - 80..center.x as u32 + 80 {
                if image.get_pixel(x, y)[0] > image.get_pixel(x, y)[2] + 15 {
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
        let pixel = image.get_pixel(center.x as u32, center.y as u32);
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
            button: MouseButton::Left,
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
            button: MouseButton::Left,
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
