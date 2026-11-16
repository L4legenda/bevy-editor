//! Match the rendered grab point to the window cursor, including accelerated/missing raw input.
use super::*;

fn grab_point(app: &mut App, window: Entity) -> Vec2 {
    let dpi = app.world().get::<Window>(window).unwrap().scale_factor();
    let world = app.world_mut();
    let (_, node, transform) = world
        .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .find(|(label, node, _)| label.0 == "Выбрать РЕПЛИКА #1" && !node.is_empty())
        .unwrap();
    transform
        .affine()
        .transform_point2(node.size() * (Vec2::new(0.27, 0.45) - 0.5))
        / dpi
}
fn position(app: &App) -> Vec2 {
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
    Vec2::from_array(d.node(1).unwrap().position)
}
fn undo_len(app: &App) -> usize {
    app.world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len()
}
fn follows(app: &mut App, window: Entity, expected: Vec2) {
    let actual = grab_point(app, window);
    assert!(
        actual.distance(expected) < 1.1,
        "Grab escaped the cursor: {actual:?} vs {expected:?}"
    );
}

#[test]
#[ignore = "GPU node dragging at several zoom levels and 200% display scale"]
fn gpu_narrative_drag_tracks_cursor() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/narrative-qa")
        .join(std::env::var("NARRATIVE_QA_PASS").unwrap_or("drag-pass-1".into()));
    let lab = output.join(format!("drag-project-{}", std::process::id()));
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
    warm(&mut app, 40);
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
            *world.query_filtered::<&mut bevy::camera::RenderTarget,With<bevy::ui::IsDefaultUiCamera>>().single_mut(world).unwrap()=
                bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {handle:target.clone(),scale_factor:2.});
            warm(&mut app, 12);
        }
        for zoom in [0.35, 0.75, 1.5] {
            {
                let mut editor = app.world_mut().resource_mut::<Editor>();
                let doc = editor.narrative.current_mut().unwrap();
                doc.zoom = zoom;
                doc.pan = Vec2::new(80., 50.);
                let Content::Dialogue(d) = &mut doc.content else {
                    panic!()
                };
                d.node_mut(0).unwrap().position = [20., 300.];
                d.node_mut(1).unwrap().position = [380., 80.];
            }
            warm(&mut app, 4);
            let original = position(&app);
            let history = undo_len(&app);
            let start = grab_point(&mut app, window);
            move_pointer(&mut app, window, start);
            mouse(&mut app, window, ButtonState::Pressed);
            assert_eq!(app.world().resource::<NarrativeUi>().drag_node, Some(1));
            // Device motion without cursor movement must never move the card.
            app.world_mut()
                .write_message(bevy::input::mouse::MouseMotion {
                    delta: Vec2::new(600., -300.),
                });
            warm(&mut app, 2);
            assert!(position(&app).abs_diff_eq(original, 0.001));
            assert_eq!(undo_len(&app), history);
            // Absolute cursor jumps work even when no raw mouse event exists.
            for delta in [
                Vec2::new(115., 42.),
                Vec2::new(-85., 63.),
                Vec2::new(72., 25.),
            ] {
                move_pointer(&mut app, window, start + delta);
                follows(&mut app, window, start + delta);
                assert!(position(&app).abs_diff_eq(original + delta / zoom, 0.005));
            }
            // A different raw sensitivity/acceleration cannot cause accumulated drift.
            app.world_mut()
                .write_message(bevy::input::mouse::MouseMotion {
                    delta: Vec2::new(2., 1.),
                });
            move_pointer(&mut app, window, start + Vec2::new(125., 55.));
            follows(&mut app, window, start + Vec2::new(125., 55.));
            capture_when(
                &mut app,
                &target,
                &output.join(format!("drag-dpi-{dpi}-zoom-{zoom}.png")),
                |_| true,
            );
            // Mouse-up can contain the final cursor position without a preceding move frame.
            let end = start + Vec2::new(150., 65.);
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(end));
            mouse(&mut app, window, ButtonState::Released);
            follows(&mut app, window, end);
            assert!(app.world().resource::<NarrativeUi>().drag_node.is_none());
            assert_eq!(undo_len(&app), history + 1);
            action(&mut app, NarrativeAction::Undo);
            assert!(position(&app).abs_diff_eq(original, 0.001));
            // A click without a drag adds no undo checkpoint.
            let point = grab_point(&mut app, window);
            move_pointer(&mut app, window, point);
            mouse(&mut app, window, ButtonState::Pressed);
            mouse(&mut app, window, ButtonState::Released);
            assert_eq!(undo_len(&app), history);
        }
    }
    let before_undo = position(&app);
    let point = grab_point(&mut app, window);
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, point + Vec2::new(45., 20.));
    action(&mut app, NarrativeAction::Undo);
    move_pointer(&mut app, window, point + Vec2::new(90., 40.));
    assert!(position(&app).abs_diff_eq(before_undo, 0.001));
    assert!(app.world().resource::<NarrativeUi>().drag_node.is_none());
    mouse(&mut app, window, ButtonState::Released);
    let point = grab_point(&mut app, window);
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, point + Vec2::new(30., 20.));
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    warm(&mut app, 2);
    let stopped = position(&app);
    move_pointer(&mut app, window, point + Vec2::new(90., 60.));
    assert!(position(&app).abs_diff_eq(stopped, 0.001));
    mouse(&mut app, window, ButtonState::Released);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    warm(&mut app, 2);
    assert!(app.world().resource::<NarrativeUi>().drag_node.is_none());
}
