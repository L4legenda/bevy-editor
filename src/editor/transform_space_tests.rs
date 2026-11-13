use super::*;

fn imported_app(parent: Transform, local: Transform) -> App {
    let mut app = app();
    let parent = GlobalTransform::from(parent);
    let world = parent.mul_transform(local).compute_transform();
    let mut editor = app.world_mut().resource_mut::<Editor>();
    editor.ready = true;
    editor.imported.push(Imported {
        entity: Entity::PLACEHOLDER,
        name: "Coordinate test".into(),
        original: world.into(),
        parent,
        local,
    });
    editor.selection = Some(Selection::Imported(0));
    drop(editor);
    app
}

#[test]
fn space_starts_global_and_toggling_does_not_move_objects_or_change_history() {
    let mut app = imported_app(
        Transform::from_xyz(10.0, 20.0, 30.0).with_rotation(Quat::from_rotation_y(0.7)),
        Transform::from_xyz(1.0, 2.0, 3.0),
    );
    let editor = app.world().resource::<Editor>();
    assert!(!editor.local_space);
    let before = editor.pose(Selection::Imported(0)).unwrap();
    let edits = editor.edits.clone();
    let view = editor.view;
    assert_eq!(
        editor.field_value(Selection::Imported(0), Field::Position(0)),
        Some(before.position[0])
    );
    for local in [true, false, true] {
        action(&mut app, Action::LocalSpace);
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.local_space, local);
        assert_eq!(editor.edits, edits);
        assert_eq!(editor.pose(Selection::Imported(0)), Some(before));
        assert_eq!(editor.view, view);
        assert!(!editor.dirty());
        assert!(editor.undo.is_empty() && editor.redo.is_empty());
    }
}

#[test]
fn local_inspector_edits_use_the_parent_and_save_world_poses_with_undo_redo() {
    let parent = Transform::from_xyz(10.0, 20.0, 30.0)
        .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
        .with_scale(Vec3::splat(2.0));
    let mut local = Transform::from_xyz(1.0, 2.0, 3.0)
        .with_rotation(Quat::from_rotation_x(0.3))
        .with_scale(Vec3::new(1.0, 2.0, 3.0));
    let mut app = imported_app(parent, local);
    let target = Selection::Imported(0);
    let before = app.world().resource::<Editor>().edits.clone();
    action(&mut app, Action::LocalSpace);
    let displayed = app
        .world()
        .resource::<Editor>()
        .inspector_pose(target)
        .unwrap();
    assert!(same_pose(displayed.into(), local.into()));

    action(&mut app, Action::Number(Field::Position(0), 5.0));
    local.translation.x = 5.0;
    let expected = GlobalTransform::from(parent)
        .mul_transform(local)
        .compute_transform();
    let editor = app.world().resource::<Editor>();
    assert!(same_pose(editor.pose(target).unwrap(), expected.into()));
    assert!((editor.field_value(target, Field::Position(0)).unwrap() - 5.0).abs() < 1e-4);
    assert!(same_pose(
        editor.edits.nodes["Coordinate test"].pose,
        expected.into()
    ));
    let positioned = editor.edits.clone();
    action(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    assert!(app.world().resource::<Editor>().local_space);
    action(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().edits, positioned);

    action(&mut app, Action::Number(Field::Rotation(0), 40.0));
    assert!(
        (app.world()
            .resource::<Editor>()
            .field_value(target, Field::Rotation(0))
            .unwrap()
            - 40.0)
            .abs()
            < 1e-3
    );
    action(&mut app, Action::Number(Field::Scale(2), 7.0));
    assert!(
        (app.world()
            .resource::<Editor>()
            .field_value(target, Field::Scale(2))
            .unwrap()
            - 7.0)
            .abs()
            < 1e-4
    );
    let world = app.world().resource::<Editor>().pose(target).unwrap();
    action(&mut app, Action::LocalSpace);
    assert!(same_pose(
        app.world()
            .resource::<Editor>()
            .inspector_pose(target)
            .unwrap()
            .into(),
        world
    ));
    assert_eq!(app.world().resource::<Editor>().pose(target), Some(world));
}

#[test]
fn local_position_under_a_nonuniform_parent_keeps_world_rotation_and_scale() {
    let mut app = imported_app(
        Transform::from_xyz(10.0, 20.0, 30.0)
            .with_rotation(Quat::from_rotation_y(0.7))
            .with_scale(Vec3::new(3.0, 2.0, 1.0)),
        Transform::from_xyz(1.0, 2.0, 3.0).with_rotation(Quat::from_rotation_z(0.4)),
    );
    action(&mut app, Action::LocalSpace);
    let before = app
        .world()
        .resource::<Editor>()
        .pose(Selection::Imported(0))
        .unwrap();
    action(&mut app, Action::Number(Field::Position(1), 8.0));
    let editor = app.world().resource::<Editor>();
    let after = editor.pose(Selection::Imported(0)).unwrap();
    assert!(Quat::from_array(after.rotation).abs_diff_eq(Quat::from_array(before.rotation), 1e-5));
    assert_eq!(after.scale, before.scale);
    assert!(
        (editor
            .field_value(Selection::Imported(0), Field::Position(1))
            .unwrap()
            - 8.0)
            .abs()
            < 1e-4
    );
}
