use super::*;
use std::time::Duration;

fn fixture() -> (App, Entity, Entity) {
    let (mut app, window, camera) = camera_app();
    action(&mut app, Action::Pilot);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let asset = editor.project.catalog[0].asset.clone();
        editor.edits.models.push(PlacedModel {
            id: 42,
            asset,
            pose: Transform::from_xyz(8.0, 1.0, -6.0).into(),
            bounds: None,
        });
        editor.saved = editor.edits.clone();
        editor.selection = Some(Selection::Model(42));
    }
    (app, window, camera)
}

fn request_focus(app: &mut App) {
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.reset_all();
    keys.press(KeyCode::NumpadDecimal);
}

fn destination(app: &App) -> (Vec3, Vec3) {
    let editor = app.world().resource::<Editor>();
    let pivot = Vec3::from(editor.pose(editor.selection.unwrap()).unwrap().position);
    (pivot + *editor.view.back() * 3.0, pivot)
}

#[test]
fn focus_flies_smoothly_to_the_object_without_editing_the_scene() {
    let (mut app, _, camera) = fixture();
    let start = app.world().resource::<Editor>().view;
    let before = app.world().resource::<Editor>().edits.clone();
    let (end, pivot) = destination(&app);
    let travel = end - start.translation;
    let mut previous = 0.0;
    let mut steps = Vec::new();
    request_focus(&mut app);
    for frame in 0..30 {
        camera_frame(&mut app, Vec2::ZERO);
        let editor = app.world().resource::<Editor>();
        let progress =
            (editor.view.translation - start.translation).dot(travel) / travel.length_squared();
        assert!(progress >= previous - 1e-5 && progress <= 1.0 + 1e-5);
        assert!(
            editor
                .view
                .translation
                .abs_diff_eq(start.translation + travel * progress, 1e-4)
        );
        if frame == 0 {
            assert!(progress > 0.0 && progress < 0.02, "focus must start gently");
        } else if frame == 11 {
            assert!(
                (0.4..0.6).contains(&progress),
                "focus must visibly pass through the midpoint"
            );
        }
        steps.push(progress - previous);
        previous = progress;
        assert_eq!(editor.view.rotation, start.rotation);
        assert_eq!(editor.view.scale, start.scale);
        assert_eq!(*app.world().get::<Transform>(camera).unwrap(), editor.view);
    }
    assert!(steps[0] < steps[10] && steps[24] < steps[12]);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.view.translation, end);
    assert_eq!(editor.pivot, pivot);
    assert_eq!(editor.edits, before);
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty() && editor.redo.is_empty());
}

#[test]
fn focus_travel_uses_elapsed_time_at_different_frame_rates() {
    let mut samples: Vec<(Vec3, Vec3)> = Vec::new();
    for milliseconds in [10, 20, 50] {
        let (mut app, _, _) = fixture();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            milliseconds,
        )));
        request_focus(&mut app);
        for _ in 0..200 / milliseconds {
            camera_frame(&mut app, Vec2::ZERO);
        }
        let editor = app.world().resource::<Editor>();
        samples.push((editor.view.translation, editor.pivot));
        for _ in 0..400 / milliseconds {
            camera_frame(&mut app, Vec2::ZERO);
        }
        let (end, pivot) = destination(&app);
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.view.translation, end);
        assert_eq!(editor.pivot, pivot);
    }
    for &(position, pivot) in &samples[1..] {
        assert!(position.abs_diff_eq(samples[0].0, 1e-4));
        assert!(pivot.abs_diff_eq(samples[0].1, 1e-4));
    }
}

#[test]
fn focus_can_be_interrupted_and_does_not_resume_after_controls_are_released() {
    for interrupt in [
        "escape",
        "fly",
        "pan",
        "orbit",
        "click",
        "scroll",
        "blur",
        "typing",
        "selection",
        "scene",
    ] {
        let (mut app, window, _) = fixture();
        request_focus(&mut app);
        for _ in 0..8 {
            camera_frame(&mut app, Vec2::ZERO);
        }
        let mut motion = Vec2::ZERO;
        match interrupt {
            "escape" => app
                .world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape),
            "fly" => {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Right);
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyW);
            }
            "pan" => {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Middle);
                motion = Vec2::new(8.0, 3.0);
            }
            "orbit" => {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::AltLeft);
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
                motion = Vec2::new(8.0, 3.0);
            }
            "click" => app
                .world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left),
            "scroll" => {
                app.world_mut()
                    .resource_mut::<AccumulatedMouseScroll>()
                    .delta = Vec2::Y
            }
            "blur" => app.world_mut().get_mut::<Window>(window).unwrap().focused = false,
            "typing" => {
                let input = app.world_mut().spawn(TextInput).id();
                app.insert_resource(InputFocus::from_entity(input));
            }
            "selection" => {
                app.world_mut().resource_mut::<Editor>().selection = Some(Selection::Camera(0))
            }
            "scene" => action(&mut app, Action::Open(1)),
            _ => unreachable!(),
        }
        camera_frame(&mut app, motion);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut().resource_mut::<InputFocus>().clear();
        app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
        app.world_mut().resource_mut::<Editor>().ready = true;
        let stopped = app.world().resource::<Editor>().view;
        let stopped_pivot = app.world().resource::<Editor>().pivot;
        for _ in 0..40 {
            camera_frame(&mut app, Vec2::ZERO);
            let editor = app.world().resource::<Editor>();
            assert_eq!(editor.view, stopped, "{interrupt}");
            assert_eq!(editor.pivot, stopped_pivot, "{interrupt}");
        }
    }
}

#[test]
fn another_focus_request_starts_from_the_current_position() {
    let (mut app, _, _) = fixture();
    request_focus(&mut app);
    for _ in 0..8 {
        camera_frame(&mut app, Vec2::ZERO);
    }
    let current = app.world().resource::<Editor>().view.translation;
    let (end, _) = destination(&app);
    request_focus(&mut app);
    camera_frame(&mut app, Vec2::ZERO);
    let next = app.world().resource::<Editor>().view.translation;
    assert!(next.distance(current) > 0.0);
    assert!(next.distance(current) < end.distance(current) * 0.02);
    for _ in 0..30 {
        camera_frame(&mut app, Vec2::ZERO);
    }
    assert_eq!(app.world().resource::<Editor>().view.translation, end);
}
