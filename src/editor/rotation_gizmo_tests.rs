use super::*;
use crate::editor::{
    Action,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::fixture,
};
use bevy::input::{ButtonState, mouse::MouseButtonInput};

#[test]
fn rotation_reference_uses_exactly_the_front_half_of_each_axis_circle() {
    let view = Transform::from_xyz(4.0, 4.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
    for axis in 0..3 {
        let complete: Vec<_> = display_ring_points(Vec3::ZERO, axis, 2.0, &view).collect();
        let circumference: f32 = complete.windows(2).map(|p| p[0].distance(p[1])).sum();
        let arc: f32 = universal_ring_segments(Vec3::ZERO, axis, 2.0, &view, Quat::IDENTITY)
            .map(|(a, b)| a.distance(b))
            .sum();
        assert!((arc * 2.0 - circumference).abs() < 0.01);
    }
}

fn camera_state(app: &mut App) -> (Camera, Transform) {
    let world = app.world_mut();
    let mut query = world.query_filtered::<(&Camera, &Transform), With<EditorCamera>>();
    let (camera, view) = query.single(world).unwrap();
    (camera.clone(), *view)
}

fn cursor(app: &mut App, window: Entity, point: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point + Vec2::Y * ui::TOP));
    frame(app);
}

fn mouse(app: &mut App, window: Entity, state: ButtonState) {
    app.world_mut().write_message(MouseButtonInput {
        window,
        button: MouseButton::Left,
        state,
    });
    frame(app);
}

#[test]
#[ignore = "requires GPU adapter; captures the reference-style rotation gizmo and exercises both rotation handles"]
fn gpu_rotation_matches_reference_and_keeps_a_constant_screen_size() {
    let (mut app, window, target) = fixture();
    let selection = Selection::Imported(0);
    let pose = {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.hidden_objects.clear();
        editor.select(Some(selection));
        editor.tool = Tool::Rotate;
        let mut pose = editor.pose(selection).unwrap().transform();
        pose.translation = Vec3::ZERO;
        editor.set_pose(selection, pose);
        editor.view = Transform::from_xyz(4.0, 4.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
        pose
    };
    // An empty grid makes the three arcs and grey outer circle directly
    // comparable with the supplied reference; this never saves scene changes.
    let root = app.world().resource::<Scene>().root.unwrap();
    app.world_mut().entity_mut(root).insert(Visibility::Hidden);
    for _ in 0..8 {
        frame(&mut app);
    }
    let (camera, view) = camera_state(&mut app);
    let global = GlobalTransform::from(view);
    let radius = rotation_radius(&camera, &view, pose.translation);
    let center = camera.world_to_viewport(&global, pose.translation).unwrap();
    let projected = camera
        .world_to_viewport(&global, pose.translation + *view.right() * radius)
        .unwrap();
    assert!((projected.distance(center) - ROTATION_RADIUS_PIXELS).abs() < 0.01);
    let outer = outer_ring_radius(&camera, &view, pose.translation, radius).unwrap();
    let editor = app.world().resource::<Editor>();
    let mut chosen = None;
    for axis in 0..3 {
        for (a, b) in universal_ring_segments(pose.translation, axis, radius, &view, Quat::IDENTITY)
        {
            let point = camera.world_to_viewport(&global, a.lerp(b, 0.5)).unwrap();
            let handle = handle_at(editor, selection, pose, &camera, &view, point, radius);
            assert!(handle.is_some(), "Every visible arc must remain draggable");
            if chosen.is_none()
                && handle
                    == Some(TransformHandle::Axis {
                        tool: Tool::Rotate,
                        axis,
                    })
            {
                chosen = Some((point, axis));
            }
        }
    }
    let outer_point = center + Vec2::X * outer;
    assert_eq!(
        handle_at(editor, selection, pose, &camera, &view, outer_point, radius),
        Some(TransformHandle::ViewRotation)
    );
    let output = std::env::temp_dir().join(format!(
        "bevy-editor-rotation-reference-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&output).unwrap();
    println!("Rotation reference artifacts: {}", output.display());
    capture_when(&mut app, &target, &output.join("reference.png"), |_| true);

    let (point, axis) = chosen.unwrap();
    cursor(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    assert_eq!(
        app.world()
            .resource::<PointerState>()
            .drag
            .as_ref()
            .unwrap()
            .handle,
        TransformHandle::Axis {
            tool: Tool::Rotate,
            axis
        }
    );
    cursor(&mut app, window, point + Vec2::new(22.0, 18.0));
    for _ in 0..4 {
        frame(&mut app);
    }
    capture_when(&mut app, &target, &output.join("active.png"), |_| true);
    mouse(&mut app, window, ButtonState::Released);
    let changed = app
        .world()
        .resource::<Editor>()
        .pose(selection)
        .unwrap()
        .transform();
    assert!(changed.rotation.angle_between(pose.rotation).abs() > 0.01);
    app.world_mut().write_message(Action::Undo);
    frame(&mut app);

    cursor(&mut app, window, outer_point);
    mouse(&mut app, window, ButtonState::Pressed);
    assert_eq!(
        app.world()
            .resource::<PointerState>()
            .drag
            .as_ref()
            .unwrap()
            .handle,
        TransformHandle::ViewRotation
    );
    cursor(&mut app, window, center + Vec2::new(0.94, 0.34) * outer);
    mouse(&mut app, window, ButtonState::Released);
    let changed = app
        .world()
        .resource::<Editor>()
        .pose(selection)
        .unwrap()
        .transform();
    assert!(changed.rotation.angle_between(pose.rotation).abs() > 0.1);
    assert_eq!(changed.translation, pose.translation);
    assert_eq!(changed.scale, pose.scale);
    app.world_mut().write_message(Action::Undo);
    frame(&mut app);

    // Object size and camera distance must not inflate the gizmo.
    let mut large = pose;
    large.scale *= 4.0;
    app.world_mut()
        .resource_mut::<Editor>()
        .set_pose(selection, large);
    app.world_mut().resource_mut::<Editor>().view.translation *= 0.6;
    app.world_mut().entity_mut(root).insert(Visibility::Visible);
    for _ in 0..8 {
        frame(&mut app);
    }
    let (camera, view) = camera_state(&mut app);
    let global = GlobalTransform::from(view);
    let radius = rotation_radius(&camera, &view, large.translation);
    let a = camera
        .world_to_viewport(&global, large.translation)
        .unwrap();
    let b = camera
        .world_to_viewport(&global, large.translation + *view.right() * radius)
        .unwrap();
    assert!((a.distance(b) - ROTATION_RADIUS_PIXELS).abs() < 0.01);
    capture_when(&mut app, &target, &output.join("large-object.png"), |_| {
        true
    });
}
