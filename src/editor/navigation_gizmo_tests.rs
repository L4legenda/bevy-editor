use super::*;

#[test]
fn navigation_gizmo_six_views_preserve_pivot_distance_and_stable_vertical_up() {
    let pivot = Vec3::new(2.0, -3.0, 4.0);
    let mut view = Transform::from_xyz(7.0, 2.0, 9.0).looking_at(pivot, Vec3::Y);
    let distance = view.translation.distance(pivot);
    for index in 0..6 {
        let axis = direction(index);
        orient(&mut view, pivot, axis);
        assert!(view.rotation.is_finite());
        assert!(view.back().abs_diff_eq(axis, 0.00001));
        assert!((view.translation.distance(pivot) - distance).abs() < 0.0001);
        assert!((view.translation + *view.forward() * distance).abs_diff_eq(pivot, 0.0001));
        if axis.y.abs() > 0.9 {
            assert!(view.up().abs_diff_eq(Vec3::NEG_Z * axis.y, 0.00001));
        }
    }
}

#[test]
fn navigation_gizmo_overlapping_axis_ends_pick_the_nearer_side() {
    assert_eq!(hit(Quat::IDENTITY, Vec2::ZERO), Some(2));
    assert_eq!(
        hit(Quat::from_rotation_y(std::f32::consts::PI), Vec2::ZERO),
        Some(5)
    );
    assert_eq!(hit(Quat::IDENTITY, Vec2::new(ARM, 0.0)), Some(0));
    assert_eq!(hit(Quat::IDENTITY, Vec2::splat(100.0)), None);
}

#[test]
fn navigation_gizmo_click_flips_an_aligned_axis_and_drag_captures_release_outside() {
    let mut frame = Frame {
        workspace: Workspace::Model(0),
        view: Transform::from_xyz(0.0, 0.0, 8.0),
        pivot: Vec3::ZERO,
        rect: Rect::from_corners(Vec2::ZERO, Vec2::new(800.0, 600.0)),
        orthographic: false,
    };
    let mut window = Window {
        focused: true,
        ..default()
    };
    let mut state = State::default();
    let mut mouse = ButtonInput::default();
    let keys = ButtonInput::default();
    let center = rect(frame.rect).center();
    window.set_cursor_position(Some(center));
    mouse.press(MouseButton::Left);
    assert!(state.drive(&mut frame, &window, &mouse, &keys, true));
    mouse.clear();
    mouse.release(MouseButton::Left);
    assert!(state.drive(&mut frame, &window, &mouse, &keys, true));
    assert!(frame.view.back().abs_diff_eq(Vec3::NEG_Z, 0.00001));
    assert!(frame.orthographic);
    mouse.clear();
    mouse.press(MouseButton::Left);
    state.drive(&mut frame, &window, &mouse, &keys, true);
    mouse.clear();
    window.set_cursor_position(Some(center - Vec2::new(150.0, -30.0)));
    assert!(state.drive(&mut frame, &window, &mouse, &keys, true));
    assert!(!frame.view.back().abs_diff_eq(Vec3::NEG_Z, 0.01));
    assert!(!frame.orthographic);
    assert!((frame.view.translation.length() - 8.0).abs() < 0.00001);
    mouse.release(MouseButton::Left);
    assert!(state.drive(&mut frame, &window, &mouse, &keys, true));
    mouse.clear();
    assert!(!state.drive(&mut frame, &window, &mouse, &keys, true));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires GPU; exercises the real scene/model axis navigator and compact mesh-mode header"]
fn gpu_navigation_gizmo_navigates_scene_and_model_without_selecting_geometry() {
    use crate::editor::{
        model_document::Mode,
        model_mesh::Element,
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture, mouse, move_pointer},
        ui::model_viewer::{
            ModelViewAction,
            editing::{EditAction, State as ModelState},
        },
    };
    use bevy::input::ButtonState;
    use bevy::input::mouse::MouseMotion;

    fn warm(app: &mut App, count: usize) {
        for _ in 0..count {
            frame(app);
        }
    }
    fn click(app: &mut App, window: Entity, point: Vec2) {
        move_pointer(app, window, point);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
    }
    fn point(app: &mut App, entity: Entity) -> Vec2 {
        let world = app.world();
        let node = world.get::<ComputedNode>(entity).unwrap();
        world
            .get::<UiGlobalTransform>(entity)
            .unwrap()
            .affine()
            .translation
            * node.inverse_scale_factor()
    }
    fn model_projection_is_orthographic(app: &mut App) -> bool {
        let world = app.world_mut();
        let image = world
            .query::<(&AccessibleLabel, &Children)>()
            .iter(world)
            .find(|(label, _)| label.0 == "3D-вид модели")
            .unwrap()
            .1
            .iter()
            .find_map(|child| world.get::<ImageNode>(child).map(|node| node.image.clone()))
            .unwrap();
        world.query::<(&bevy::camera::RenderTarget, &Projection)>().iter(world)
            .find_map(|(target, projection)| {
                matches!(target, bevy::camera::RenderTarget::Image(target) if target.handle == image)
                    .then_some(matches!(projection, Projection::Orthographic(_)))
            }).unwrap()
    }
    fn control(app: &mut App, action: EditAction) -> (Entity, Rect) {
        let world = app.world_mut();
        world.query::<(Entity, &Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find_map(|(entity, control, node, transform)| {
                (matches!(control.action, Action::ModelView(ModelViewAction::Edit(a)) if a == action)
                    && !node.is_empty()).then(|| (entity, Rect::from_center_size(
                        transform.affine().translation * node.inverse_scale_factor(),
                        node.size() * node.inverse_scale_factor(),
                    )))
            }).unwrap()
    }

    let (mut app, window, target) = fixture();
    let output =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/navigation-gizmo-qa");
    std::fs::create_dir_all(&output).unwrap();
    let before = app.world().resource::<Editor>().edits.clone();
    let selection = app.world().resource::<Editor>().selection;
    let history = app.world().resource::<Editor>().undo.len();
    let tip = app.world().resource::<GizmoUi>().tips[0];
    let position = point(&mut app, tip);
    click(&mut app, window, position);
    assert!(
        app.world()
            .resource::<Editor>()
            .view
            .back()
            .abs_diff_eq(Vec3::X, 0.0001)
    );
    assert_eq!(app.world().resource::<Editor>().selection, selection);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    assert_eq!(app.world().resource::<Editor>().undo.len(), history);
    assert!(app.world().resource::<Editor>().view_orthographic);
    assert!(matches!(
        app.world_mut()
            .query_filtered::<&Projection, With<viewport::EditorCamera>>()
            .single(app.world())
            .unwrap(),
        Projection::Orthographic(_)
    ));
    capture_when(
        &mut app,
        &target,
        &output.join("01-scene-axis-view.png"),
        |_| true,
    );

    let center = Vec2::new(450.0, 430.0);
    move_pointer(&mut app, window, center);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    mouse(&mut app, window, ButtonState::Pressed);
    app.world_mut().write_message(MouseMotion {
        delta: Vec2::new(30.0, 12.0),
    });
    warm(&mut app, 2);
    assert!(!app.world().resource::<Editor>().view_orthographic);
    assert!(matches!(
        app.world_mut()
            .query_filtered::<&Projection, With<viewport::EditorCamera>>()
            .single(app.world())
            .unwrap(),
        Projection::Perspective(_)
    ));
    mouse(&mut app, window, ButtonState::Released);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::AltLeft);
    let position = point(&mut app, tip);
    click(&mut app, window, position);
    assert!(app.world().resource::<Editor>().view_orthographic);

    let scene_view = app.world().resource::<Editor>().view;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<ModelState>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    warm(&mut app, 8);
    let mode = control(&mut app, EditAction::Menu).1;
    let gizmo = app.world().resource::<GizmoUi>().root;
    let bounds = {
        let center = point(&mut app, gizmo);
        Rect::from_center_size(center, Vec2::splat(SIZE))
    };
    assert!(mode.max.x < bounds.min.x);
    assert!(mode.min.x < 30.0, "The mode dropdown belongs on the left");
    capture_when(
        &mut app,
        &target,
        &output.join("02-model-object-mode.png"),
        |_| true,
    );
    click(&mut app, window, mode.center());
    let edit_mode = control(&mut app, EditAction::Mode(Mode::Edit)).1;
    click(&mut app, window, edit_mode.center());
    assert_eq!(
        app.world().resource::<ModelState>().mode(Some(0)),
        Mode::Edit
    );
    for element in [Element::Vertex, Element::Edge, Element::Face] {
        let (button, bounds) = control(&mut app, EditAction::Element(element));
        assert!(
            bounds.width() <= 32.0,
            "Component selectors must be compact icon buttons"
        );
        assert!(bounds.min.x > mode.min.x);
        click(&mut app, window, bounds.center());
        assert_eq!(
            app.world().resource::<ModelState>().documents[&0]
                .selection
                .kind,
            element
        );
        assert!(
            app.world()
                .get::<Children>(button)
                .unwrap()
                .iter()
                .any(|child| app.world().get::<ImageNode>(child).is_some())
        );
    }
    let tip = app.world().resource::<GizmoUi>().tips[1];
    let position = point(&mut app, tip);
    let chosen = app.world().resource::<ModelState>().documents[&0]
        .selection
        .clone();
    click(&mut app, window, position);
    warm(&mut app, 3);
    let back = app.world().resource::<State>().rotation * Vec3::Z;
    assert!(
        back.abs_diff_eq(Vec3::Y, 0.0001),
        "The top view must stay aligned: {back:?}"
    );
    assert_eq!(
        app.world().resource::<ModelState>().documents[&0].selection,
        chosen
    );
    assert_eq!(app.world().resource::<Editor>().view, scene_view);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    assert!(app.world().resource::<Editor>().dirty_models.is_empty());
    assert!(model_projection_is_orthographic(&mut app));

    // The actual top face must be pickable through parallel orthographic rays.
    let center = Vec2::new(482.0, 480.0);
    click(&mut app, window, center + Vec2::new(-30.0, -25.0));
    assert_eq!(
        app.world().resource::<ModelState>().documents[&0]
            .selection
            .faces
            .len(),
        1
    );
    capture_when(
        &mut app,
        &target,
        &output.join("03-model-edit-mode.png"),
        |_| true,
    );

    let start = bounds.center() + Vec2::new(-45.0, -45.0);
    move_pointer(&mut app, window, start);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, start + Vec2::new(-140.0, 35.0));
    mouse(&mut app, window, ButtonState::Released);
    assert!(!(app.world().resource::<State>().rotation * Vec3::Z).abs_diff_eq(Vec3::Y, 0.01));
    assert!(!model_projection_is_orthographic(&mut app));
    capture_when(
        &mut app,
        &target,
        &output.join("04-model-orbit.png"),
        |_| true,
    );
}
