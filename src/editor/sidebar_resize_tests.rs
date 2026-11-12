use super::*;

fn fixture(axis: Axis, scale: f32) -> (App, Entity, Entity, Vec2) {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
    )
    .unwrap();
    let editor = Editor::new(project, crate::data::LevelEdits::default());
    let point = match axis {
        Axis::Width => Vec2::new(1280.0 - SIDE + 4.0, 200.0),
        Axis::Height => Vec2::new(1120.0, 760.0 - FOOT - 300.0 - GRIP * 0.5),
    };
    let mut window = Window {
        focused: true,
        ..default()
    };
    window.resolution.set_scale_factor_override(Some(scale));
    window.resolution.set(1280.0, 760.0);
    window.set_cursor_position(Some(point));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<DrawerMotion>()
        .init_resource::<DrawerSize>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<InputFocus>()
        .add_systems(Update, resize);
    let window = app
        .world_mut()
        .spawn((
            window,
            PrimaryWindow,
            CursorIcon::from(SystemCursorIcon::Crosshair),
        ))
        .id();
    let size = match axis {
        Axis::Width => Vec2::new(GRIP, 632.0),
        Axis::Height => Vec2::new(SIDE - GRIP - 1.0, GRIP),
    };
    let center = match axis {
        Axis::Width => Vec2::new(1280.0 - SIDE + 4.0, TOP + 316.0),
        Axis::Height => point,
    };
    app.world_mut().spawn((
        axis,
        ComputedNode {
            size: size * scale,
            ..default()
        },
        UiGlobalTransform::from_xy(center.x * scale, center.y * scale),
        BackgroundColor(Color::NONE),
    ));
    app.world_mut().spawn((Sidebar, Node::default()));
    let inspector = app.world_mut().spawn((Inspector, Node::default())).id();
    (app, window, inspector, point)
}

fn frame(app: &mut App) {
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn move_to(app: &mut App, window: Entity, point: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    frame(app);
}

#[test]
fn inspector_height_drag_moves_both_panes_and_does_not_edit_the_scene() {
    let (mut app, window, inspector, point) = fixture(Axis::Height, 1.0);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(&mut app);
    assert!(app.world().resource::<Editor>().sidebar.resizing());
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::NsResize))
    );
    move_to(&mut app, window, point - Vec2::Y * 120.0);
    assert_eq!(app.world().get::<Node>(inspector).unwrap().height, px(420));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.sidebar.inspector_height(760.0, false), 420.0);
    assert_eq!(editor.sidebar.inspector_height(760.0, true), 420.0);
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    move_to(&mut app, window, Vec2::new(400.0, 200.0));
    assert!(!app.world().resource::<Editor>().sidebar.resizing());
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::Crosshair))
    );
    app.world_mut().resource_mut::<Editor>().open_document(1);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .sidebar
            .inspector_height(760.0, false),
        420.0
    );
}

#[test]
fn sidebar_width_drag_recalculates_the_viewport_and_handles_retina_coordinates() {
    for scale in [1.0, 2.0] {
        let (mut app, window, _, point) = fixture(Axis::Width, scale);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app);
        move_to(&mut app, window, point - Vec2::X * 144.0);
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.sidebar.width(1280.0), SIDE + 144.0);
        let window_ref = app.world().get::<Window>(window).unwrap();
        assert_eq!(
            viewport_rect(window_ref, &editor.sidebar).max.x,
            1280.0 - SIDE - 144.0
        );
        assert!(
            !scene_input_rect(
                window_ref,
                app.world().resource::<DrawerMotion>(),
                &editor.sidebar
            )
            .contains(Vec2::new(300.0, 300.0))
        );
        assert_eq!(
            app.world().get::<CursorIcon>(window),
            Some(&CursorIcon::from(SystemCursorIcon::EwResize))
        );
    }
}

#[test]
fn temporary_window_shrinking_clamps_panels_without_losing_preferred_sizes() {
    let layout = SidebarLayout {
        preferred_width: Some(600.0),
        preferred_height: Some(420.0),
        ..default()
    };
    assert_eq!(layout.width(640.0), 320.0);
    assert_eq!(
        layout.inspector_height(400.0, false),
        400.0 - TOP - FOOT - TREE_HEADER - MIN_TREE - GRIP
    );
    assert_eq!(layout.width(1280.0), 600.0);
    assert_eq!(layout.inspector_height(760.0, false), 420.0);
    assert_eq!(layout.inspector_height(760.0, true), 420.0);
}

#[test]
fn keyboard_adjustments_and_home_reset_are_available_on_focused_splitters() {
    let (mut app, _, _, _) = fixture(Axis::Width, 1.0);
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<Axis>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(entity, FocusCause::Navigated);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ArrowLeft);
    frame(&mut app);
    assert_eq!(
        app.world().resource::<Editor>().sidebar.width(1280.0),
        SIDE + 20.0
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Home);
    frame(&mut app);
    assert_eq!(app.world().resource::<Editor>().sidebar.width(1280.0), SIDE);
    assert!(!app.world().resource::<Editor>().dirty());
}

#[test]
fn escape_restores_the_original_size_and_consumes_the_rest_of_the_drag() {
    let (mut app, window, _, point) = fixture(Axis::Width, 1.0);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(&mut app);
    move_to(&mut app, window, point - Vec2::X * 200.0);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    frame(&mut app);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.sidebar.width(1280.0), SIDE);
    assert!(editor.sidebar.resizing());
    move_to(&mut app, window, point - Vec2::X * 300.0);
    assert_eq!(app.world().resource::<Editor>().sidebar.width(1280.0), SIDE);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    frame(&mut app);
    frame(&mut app);
    assert!(!app.world().resource::<Editor>().sidebar.resizing());
}

#[test]
fn the_drawer_overlay_and_lost_focus_prevent_hidden_splitters_from_capturing_input() {
    let (mut app, window, _, point) = fixture(Axis::Height, 1.0);
    app.world_mut()
        .resource_mut::<DrawerMotion>()
        .visible_height = 400.0;
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(&mut app);
    assert!(!app.world().resource::<Editor>().sidebar.resizing());
    app.world_mut()
        .resource_mut::<DrawerMotion>()
        .visible_height = 0.0;
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    frame(&mut app);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    move_to(&mut app, window, point);
    assert!(!app.world().resource::<Editor>().sidebar.resizing());
}
