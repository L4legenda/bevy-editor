use super::*;

fn choices(app: &App) -> Vec<Choice> {
    let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    else {
        panic!()
    };
    d.node(1).unwrap().choices.clone()
}
fn history(app: &App) -> usize {
    app.world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len()
}
fn point(app: &mut App, window: Entity, label: &str) -> Vec2 {
    control_point(app, label) / app.world().get::<Window>(window).unwrap().scale_factor()
}
fn handle(app: &mut App, window: Entity, i: usize) -> Vec2 {
    point(app, window, &format!("Переместить ответ {} ноды #1", i + 1))
}
fn ports_aligned(app: &mut App, window: Entity) {
    for i in 0..3 {
        let field = point(app, window, &format!("Ответ {} ноды #1", i + 1));
        let port = point(app, window, &format!("Связать ответ {} ноды #1", i + 1));
        assert!(
            (field.y - port.y).abs() < 1.1,
            "Detached port {i}: {field:?} {port:?}"
        );
    }
}
fn screenshot(app: &mut App, target: &Handle<Image>, path: &std::path::Path) {
    *app.world_mut()
        .resource_mut::<bevy::time::TimeUpdateStrategy>() =
        bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO);
    warm(app, 2);
    capture_when(app, target, path, |_| true);
    *app.world_mut()
        .resource_mut::<bevy::time::TimeUpdateStrategy>() =
        bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(
            1. / 60.,
        ));
}

#[test]
#[ignore = "GPU answer reorder with linked branches, cancel, undo and high DPI"]
fn gpu_narrative_reorder_answers() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/narrative-qa")
        .join(std::env::var("NARRATIVE_QA_PASS").unwrap_or("reorder-pass-1".into()));
    let lab = output.join(format!("reorder-project-{}", std::process::id()));
    fn copy_assets(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for item in std::fs::read_dir(from).unwrap() {
            let item = item.unwrap();
            let path = to.join(item.file_name());
            if item.file_type().unwrap().is_dir() {
                copy_assets(&item.path(), &path);
            } else {
                std::fs::copy(item.path(), path).unwrap();
            }
        }
    }
    let mut project = crate::project::EditorProject::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_assets(&project.asset_root, &lab);
    project.asset_root = lab;
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    action(&mut app, NarrativeAction::Create(false));
    action(&mut app, NarrativeAction::Add(NodeKind::Line));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let doc = editor.narrative.current_mut().unwrap();
        let Content::Dialogue(d) = &mut doc.content else {
            panic!()
        };
        for _ in 0..3 {
            d.add(NodeKind::Line, 1);
        }
        d.node_mut(1).unwrap().choices = vec![
            Choice {
                text: "Проверить источник сигнала".into(),
                target: Some(2),
            },
            Choice {
                text: "Запросить помощь экипажа".into(),
                target: Some(3),
            },
            Choice {
                text: "Вернуться на корабль".into(),
                target: Some(4),
            },
        ];
        d.node_mut(0).unwrap().position = [16., 20.];
        d.node_mut(1).unwrap().position = [400., 20.];
        for (id, y) in [(2, 10.), (3, 190.), (4, 370.)] {
            d.node_mut(id).unwrap().position = [850., y];
        }
    }
    app.world_mut().resource_mut::<NarrativeUi>().generation += 1;
    warm(&mut app, 60);
    let original = choices(&app);
    let mut expected = original.clone();
    let first = expected.remove(0);
    expected.push(first);
    for dpi in [1., 2.] {
        if dpi == 2. {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set_scale_factor_override(Some(2.));
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set(1280., 900.);
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .get_mut(&target)
                .unwrap()
                .resize(bevy::render::render_resource::Extent3d {
                    width: 2560,
                    height: 1800,
                    depth_or_array_layers: 1,
                });
            let world = app.world_mut();
            *world.query_filtered::<&mut bevy::camera::RenderTarget, With<bevy::ui::IsDefaultUiCamera>>().single_mut(world).unwrap() = bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget { handle: target.clone(), scale_factor: 2. });
            warm(&mut app, 12);
        }
        for zoom in [0.65, 1.2] {
            {
                let mut editor = app.world_mut().resource_mut::<Editor>();
                let doc = editor.narrative.current_mut().unwrap();
                doc.zoom = zoom;
                doc.pan = Vec2::new(20., 25.);
            }
            warm(&mut app, 20);
            let before = history(&app);
            let stable_input = named_input(&mut app, "Ответ 1 ноды #1");
            let start = handle(&mut app, window, 0);
            move_pointer(&mut app, window, start);
            mouse(&mut app, window, ButtonState::Pressed);
            assert!(
                app.world().resource::<effects::State>().answers.capturing(),
                "Handle did not capture the pointer"
            );
            let end = start + Vec2::new(0., 102. * zoom);
            move_pointer(&mut app, window, end);
            assert!(
                handle(&mut app, window, 0).distance(end) < 1.2,
                "Dragged row lags behind the cursor"
            );
            assert_eq!(choices(&app), original);
            assert_eq!(history(&app), before);
            assert!(app.world().get::<EditableText>(stable_input).is_some());
            ports_aligned(&mut app, window);
            // Holding still past the movement duration must preserve the preview positions.
            warm(&mut app, 25);
            ports_aligned(&mut app, window);
            assert!(handle(&mut app, window, 0).distance(end) < 1.2);
            screenshot(
                &mut app,
                &target,
                &output.join(format!("01-preview-dpi-{dpi}-zoom-{zoom}.png")),
            );
            mouse(&mut app, window, ButtonState::Released);
            assert_eq!(choices(&app), expected);
            assert_eq!(history(&app), before + 1);
            ports_aligned(&mut app, window);
            warm(&mut app, 20);
            assert!(!app.world().resource::<effects::State>().answers.active());
            assert_eq!(history(&app), before + 1);
            for (i, target) in [(0, 3), (1, 4), (2, 2)] {
                assert_eq!(branch_target(&app, 1, i), Some(target));
            }
            let world = app.world_mut();
            assert_eq!(
                world.query::<&effects::Visual>().iter(world).count(),
                0,
                "Reordering must not break/recreate connections"
            );
            action(&mut app, NarrativeAction::Undo);
            assert_eq!(choices(&app), original);
            action(&mut app, NarrativeAction::Redo);
            assert_eq!(choices(&app), expected);
            action(&mut app, NarrativeAction::Undo);
            warm(&mut app, 25);
            assert_eq!(choices(&app), original);
            // A click and a cancelled drag create no history entries.
            let before = history(&app);
            let start = handle(&mut app, window, 1);
            move_pointer(&mut app, window, start);
            mouse(&mut app, window, ButtonState::Pressed);
            mouse(&mut app, window, ButtonState::Released);
            assert_eq!(history(&app), before);
            move_pointer(&mut app, window, start);
            mouse(&mut app, window, ButtonState::Pressed);
            move_pointer(&mut app, window, start - Vec2::new(0., 52. * zoom));
            key(
                &mut app,
                window,
                KeyCode::Escape,
                bevy::input::keyboard::Key::Escape,
                None,
            );
            mouse(&mut app, window, ButtonState::Released);
            warm(&mut app, 25);
            assert_eq!(choices(&app), original);
            assert_eq!(history(&app), before);
            assert!(!app.world().resource::<effects::State>().answers.capturing());
        }
    }
    let before = history(&app);
    let start = handle(&mut app, window, 2);
    move_pointer(&mut app, window, start);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, start - Vec2::new(0., 100.));
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    warm(&mut app, 3);
    assert!(!app.world().resource::<effects::State>().answers.capturing());
    mouse(&mut app, window, ButtonState::Released);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    warm(&mut app, 25);
    assert_eq!(choices(&app), original);
    assert_eq!(history(&app), before);
    screenshot(&mut app, &target, &output.join("02-restored.png"));
    // Future edits and deletes must address the moved answer's new index.
    let start = handle(&mut app, window, 0);
    move_pointer(&mut app, window, start);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, start + Vec2::new(0., 102. * 1.2));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 25);
    assert_eq!(choices(&app), expected);
    let input = edit_named(&mut app, "Ответ 3 ноды #1", "Изучить координаты сигнала");
    warm(&mut app, 20);
    assert_eq!(choices(&app)[2].target, Some(2));
    assert!(app.world().get::<EditableText>(input).is_some());
    action(&mut app, NarrativeAction::RemoveChoice(2));
    assert_eq!(choices(&app).len(), 2);
    action(&mut app, NarrativeAction::Undo);
    warm(&mut app, 25);
    assert_eq!(choices(&app)[2].target, Some(2));
    action(&mut app, NarrativeAction::Save);
    let editor = app.world().resource::<Editor>();
    let doc = editor.narrative.current().unwrap();
    let Content::Dialogue(saved) = Content::read(&editor.root, &doc.path).unwrap() else {
        panic!()
    };
    assert_eq!(saved.node(1).unwrap().choices, choices(&app));
}
