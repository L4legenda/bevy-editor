use super::*;

fn shortcut_app() -> App {
    let mut app = app();
    app.init_resource::<InputFocus>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_systems(Update, ui::shortcuts.before(handle_actions));
    app.world_mut().resource_mut::<Editor>().ready = true;
    app
}

fn press(app: &mut App, keys: &[KeyCode]) {
    for &key in keys {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
}

#[test]
fn numpad_decimal_focuses_and_the_old_f_and_main_period_do_not() {
    let mut app = shortcut_app();
    for key in [KeyCode::KeyF, KeyCode::Period] {
        press(&mut app, &[key]);
        assert!(!app.world().resource::<Editor>().focus);
    }
    press(&mut app, &[KeyCode::NumpadDecimal]);
    let editor = app.world().resource::<Editor>();
    assert!(editor.focus);
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty());
}

#[test]
fn alt_resets_only_the_requested_parts_of_the_selection_and_supports_undo_redo() {
    let targets = [
        Selection::Imported(0),
        Selection::Model(1),
        Selection::Camera(0),
        Selection::Light(1),
    ];
    for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
        for (key, parts) in [
            (KeyCode::KeyQ, [true, true, true]),
            (KeyCode::KeyW, [true, false, false]),
            (KeyCode::KeyE, [false, true, false]),
            (KeyCode::KeyR, [false, false, true]),
        ] {
            for target in targets {
                let mut app = shortcut_app();
                let imported = Transform::from_xyz(1.0, 2.0, 3.0)
                    .with_rotation(Quat::from_rotation_y(0.5))
                    .with_scale(Vec3::splat(2.0));
                let light = crate::LightPreset::Spot.create(1);
                let original = {
                    let mut editor = app.world_mut().resource_mut::<Editor>();
                    editor.imported.push(Imported {
                        entity: Entity::PLACEHOLDER,
                        name: "Reset test".into(),
                        original: imported.into(),
                        parent: GlobalTransform::IDENTITY,
                        local: imported,
                    });
                    let asset = editor.project.catalog[0].asset.clone();
                    editor.edits.models.push(PlacedModel {
                        id: 1,
                        asset,
                        pose: Transform::IDENTITY.into(),
                        bounds: None,
                    });
                    editor.edits.lights.push(light);
                    editor.selection = Some(target);
                    editor.tool = Tool::Universal;
                    let original = editor.pose(target).unwrap();
                    let edited = Transform::from_xyz(9.0, 8.0, 7.0)
                        .with_rotation(Quat::from_rotation_x(1.0))
                        .with_scale(Vec3::new(4.0, 5.0, 6.0));
                    editor.set_pose(target, edited);
                    let camera = CameraPlacement {
                        lens: 60.0,
                        ..editor.camera(0)
                    };
                    editor.set_camera(0, camera);
                    editor.edits.lights[0].intensity = 123.0;
                    original
                };
                let before = app.world().resource::<Editor>().edits.clone();
                let previous = app.world().resource::<Editor>().pose(target).unwrap();
                let other_poses =
                    targets.map(|other| app.world().resource::<Editor>().pose(other).unwrap());

                press(&mut app, &[alt, key]);
                let editor = app.world().resource::<Editor>();
                let actual = editor.pose(target).unwrap();
                let expected = Pose {
                    position: if parts[0] {
                        original.position
                    } else {
                        previous.position
                    },
                    rotation: if parts[1] {
                        original.rotation
                    } else {
                        previous.rotation
                    },
                    scale: if parts[2] {
                        original.scale
                    } else {
                        previous.scale
                    },
                };
                assert!(same_pose(actual, expected), "{alt:?}+{key:?}: {target:?}");
                for (other, pose) in targets.into_iter().zip(other_poses) {
                    if other != target {
                        assert!(same_pose(editor.pose(other).unwrap(), pose));
                    }
                }
                assert_eq!(editor.selection, Some(target));
                assert_eq!(editor.tool, Tool::Universal);
                assert_eq!(editor.undo.len(), 1);
                if target == Selection::Camera(0) {
                    assert_eq!(
                        editor.camera(0).lens,
                        if key == KeyCode::KeyQ { 45.0 } else { 60.0 }
                    );
                }
                if target == Selection::Light(1) {
                    assert_eq!(
                        editor.edits.lights[0].intensity,
                        if key == KeyCode::KeyQ { 2000.0 } else { 123.0 }
                    );
                }
                let after = editor.edits.clone();
                press(&mut app, &[alt, key]);
                assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
                action(&mut app, Action::Undo);
                assert_eq!(app.world().resource::<Editor>().edits, before);
                action(&mut app, Action::Redo);
                assert_eq!(app.world().resource::<Editor>().edits, after);
            }
        }
    }
}

#[test]
fn reset_shortcuts_respect_text_entry_modal_dialogs_and_camera_controls() {
    for blocked in 0..5 {
        let mut app = shortcut_app();
        action(&mut app, Action::Add(0));
        {
            let mut editor = app.world_mut().resource_mut::<Editor>();
            editor.edits.models[0].pose = Transform::from_xyz(4.0, 5.0, 6.0).into();
            if blocked == 2 {
                editor.pending = Some(PendingAction::Close);
            } else if blocked == 3 {
                editor.pilot = true;
            }
        }
        if blocked <= 1 {
            let input = app.world_mut().spawn(TextInput).id();
            if blocked == 0 {
                app.world_mut().entity_mut(input).insert(Field::Position(0));
            }
            app.insert_resource(InputFocus::from_entity(input));
        } else if blocked == 4 {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Right);
        }
        let before = app.world().resource::<Editor>().edits.clone();
        let history = app.world().resource::<Editor>().undo.len();
        for key in [KeyCode::KeyQ, KeyCode::KeyW, KeyCode::KeyE, KeyCode::KeyR] {
            press(&mut app, &[KeyCode::AltLeft, key]);
            let editor = app.world().resource::<Editor>();
            assert_eq!(editor.edits, before, "blocked case {blocked}: {key:?}");
            assert_eq!(editor.undo.len(), history);
            assert_eq!(editor.tool, Tool::Universal);
        }
    }
}
