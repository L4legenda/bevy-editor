use super::*;

#[test]
fn cursor_mode_is_the_default_and_tree_picking_is_exclusive_to_it() {
    let mut app = app();
    app.world_mut().resource_mut::<Editor>().ready = true;
    assert_eq!(Tool::default(), Tool::Universal);
    assert_eq!(app.world().resource::<Editor>().tool, Tool::Universal);
    assert_eq!(Tool::Universal.operations(), &[Tool::Move, Tool::Rotate]);
    action(&mut app, Action::Select(Selection::Camera(0)));
    for tool in [Tool::Move, Tool::Rotate, Tool::Scale] {
        // Set mode directly: cameras intentionally don't support the Scale tool.
        app.world_mut().resource_mut::<Editor>().tool = tool;
        action(&mut app, Action::Select(Selection::Environment));
        assert_eq!(
            app.world().resource::<Editor>().selection,
            Some(Selection::Camera(0))
        );
    }
    action(&mut app, Action::Tool(Tool::Universal));
    action(&mut app, Action::Select(Selection::Environment));
    assert_eq!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Environment)
    );
    // The cursor must remain reachable even from the environment inspector.
    action(&mut app, Action::Tool(Tool::Universal));
    action(&mut app, Action::Select(Selection::Camera(0)));
    assert_eq!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Camera(0))
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires GPU adapter; exercises real mesh picking and transform-mode selection locking"]
fn gpu_only_cursor_mode_can_pick_objects_or_clear_selection() {
    use crate::editor::{selection_outline::render_tests::frame, viewport::EditorCamera};
    use bevy::input::{ButtonState, mouse::MouseButtonInput};
    let (mut app, window, _) = lighting_ui_render::fixture();
    let target = Selection::Imported(1);
    let position = {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        assert_eq!(editor.tool, Tool::Universal);
        editor.hidden_objects.clear();
        let point = editor.pose(target).unwrap().transform().translation;
        editor.view = Transform::from_translation(point + Vec3::new(4.0, 3.0, 7.0))
            .looking_at(point, Vec3::Y);
        editor.selection = None;
        point
    };
    for _ in 0..8 {
        frame(&mut app);
    }
    let point = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<(&Camera, &GlobalTransform), With<EditorCamera>>();
        let (camera, view) = query.single(world).unwrap();
        camera.world_to_viewport(view, position).unwrap() + Vec2::Y * ui::TOP
    };
    let click = |app: &mut App, point: Vec2| {
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(point));
        frame(app);
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(MouseButtonInput {
                window,
                button: MouseButton::Left,
                state,
            });
            frame(app);
        }
    };
    for tool in [Tool::Move, Tool::Rotate, Tool::Scale] {
        app.world_mut().resource_mut::<Editor>().tool = tool;
        click(&mut app, point);
        assert_eq!(
            app.world().resource::<Editor>().selection,
            None,
            "{tool:?} must not pick geometry"
        );
    }
    app.world_mut().resource_mut::<Editor>().tool = Tool::Universal;
    click(&mut app, point);
    assert_eq!(app.world().resource::<Editor>().selection, Some(target));
    let empty = Vec2::new(60.0, ui::TOP + 60.0);
    for tool in [Tool::Move, Tool::Rotate, Tool::Scale] {
        app.world_mut().resource_mut::<Editor>().tool = tool;
        click(&mut app, empty);
        assert_eq!(
            app.world().resource::<Editor>().selection,
            Some(target),
            "{tool:?} must retain the selected object"
        );
    }
    app.world_mut().resource_mut::<Editor>().tool = Tool::Universal;
    click(&mut app, empty);
    assert_eq!(app.world().resource::<Editor>().selection, None);
}
