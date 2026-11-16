use super::*;
use bevy::camera::{ComputedCameraValues, RenderTargetInfo};

fn fixture(tool: Tool, local: bool, orthographic: bool) -> (App, Entity, Entity, Transform) {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, default());
    let start = Transform::from_xyz(1.0, 2.0, -1.0)
        .with_rotation(Quat::from_euler(EulerRot::YXZ, 0.7, 0.4, 0.3))
        .with_scale(Vec3::new(2.0, 3.0, 4.0));
    editor.edits.models.push(crate::data::PlacedModel {
        id: 42,
        asset: editor.project.catalog[0].asset.clone(),
        pose: start.into(),
        bounds: None,
    });
    editor.selection = Some(Selection::Model(42));
    editor.ready = true;
    editor.tool = tool;
    editor.local_space = local;
    editor.snap = false;
    let view = Transform::from_xyz(4.0, 5.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y);
    let mut lens = if orthographic {
        projection(LensKind::Orthographic, 12.0)
    } else {
        Projection::Perspective(PerspectiveProjection {
            aspect_ratio: 1.0,
            fov: EDITOR_FOV,
            ..default()
        })
    };
    lens.update(1000.0, 1000.0);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<Scene>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<PointerState>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ui::DrawerMotion>()
        .add_message::<super::super::Action>()
        .add_systems(Update, (super::super::handle_actions, manipulate).chain());
    let camera = app
        .world_mut()
        .spawn((
            EditorCamera,
            Camera {
                computed: ComputedCameraValues {
                    clip_from_view: lens.get_clip_from_view(),
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::splat(1000),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
            view,
        ))
        .id();
    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                focused: true,
                resolution: (1600, 1100).into(),
                ..default()
            },
        ))
        .id();
    (app, window, camera, start)
}

fn project(app: &App, camera: Entity, point: Vec3) -> Vec2 {
    let camera_entity = app.world().entity(camera);
    camera_entity
        .get::<Camera>()
        .unwrap()
        .world_to_viewport(
            &GlobalTransform::from(*camera_entity.get::<Transform>().unwrap()),
            point,
        )
        .unwrap()
}

fn cursor(app: &mut App, window: Entity, point: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point + Vec2::Y * ui::TOP));
    app.update();
}

fn begin(app: &mut App, window: Entity, point: Vec2) {
    cursor(app, window, point);
    assert_eq!(
        app.world().resource::<PointerState>().hover,
        Some(TransformHandle::ViewTranslation)
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    assert_eq!(
        app.world()
            .resource::<PointerState>()
            .drag
            .as_ref()
            .unwrap()
            .handle,
        TransformHandle::ViewTranslation
    );
}

fn pose(app: &App) -> Transform {
    let editor = app.world().resource::<Editor>();
    editor.pose(editor.selection.unwrap()).unwrap().transform()
}

#[test]
fn free_move_tracks_the_cursor_without_jumping_and_supports_undo_redo() {
    for tool in [Tool::Move] {
        for local in [false, true] {
            for orthographic in [false, true] {
                let (mut app, window, camera, start) = fixture(tool, local, orthographic);
                let before = app.world().resource::<Editor>().edits.clone();
                let center = project(&app, camera, start.translation);
                // A grab slightly off-center must keep its original offset.
                let grab = center + Vec2::new(3.0, -2.0);
                begin(&mut app, window, grab);
                app.update();
                assert!(pose(&app).translation.abs_diff_eq(start.translation, 1e-5));
                for delta in [Vec2::new(70.0, -45.0), Vec2::new(-80.0, 55.0)] {
                    cursor(&mut app, window, grab + delta);
                    let moved = pose(&app);
                    assert!(
                        project(&app, camera, moved.translation).abs_diff_eq(center + delta, 0.002)
                    );
                    let view = app.world().get::<Transform>(camera).unwrap();
                    assert!(
                        (moved.translation - start.translation)
                            .dot(*view.forward())
                            .abs()
                            < 1e-4
                    );
                    assert_eq!(moved.rotation, start.rotation);
                    assert_eq!(moved.scale, start.scale);
                    assert!(app.world().resource::<Editor>().undo.is_empty());
                }
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .release(MouseButton::Left);
                app.update();
                let editor = app.world().resource::<Editor>();
                assert_eq!(editor.undo.len(), 1);
                assert_eq!(editor.tool, tool);
                let after = editor.edits.clone();
                app.world_mut().write_message(super::super::Action::Undo);
                app.update();
                assert_eq!(app.world().resource::<Editor>().edits, before);
                app.world_mut().write_message(super::super::Action::Redo);
                app.update();
                assert_eq!(app.world().resource::<Editor>().edits, after);
            }
        }
    }
}

#[test]
fn free_move_snaps_in_the_selected_transform_space() {
    for local in [false, true] {
        let (mut app, window, camera, start) = fixture(Tool::Move, local, false);
        app.world_mut().resource_mut::<Editor>().snap = true;
        let center = project(&app, camera, start.translation);
        begin(&mut app, window, center);
        cursor(&mut app, window, center + Vec2::new(76.0, -51.0));
        let frame = app.world().resource::<Editor>().gizmo_rotation(start);
        let shift = frame.inverse() * (pose(&app).translation - start.translation);
        assert!(shift.length() > 0.5);
        for component in shift.to_array() {
            assert!((component * 4.0 - (component * 4.0).round()).abs() < 1e-4);
        }
    }
}

#[test]
fn free_move_cancels_on_escape_or_focus_loss_even_outside_the_viewport() {
    for escape in [true, false] {
        let (mut app, window, camera, start) = fixture(Tool::Move, false, false);
        let before = app.world().resource::<Editor>().edits.clone();
        let center = project(&app, camera, start.translation);
        begin(&mut app, window, center);
        cursor(&mut app, window, Vec2::new(1450.0, 100.0));
        assert!(pose(&app).translation.distance(start.translation) > 1.0);
        if escape {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
        } else {
            app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
        }
        app.update();
        assert!(!app.world().resource::<PointerState>().dragging());
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits, before);
        assert!(editor.undo.is_empty());
    }
}

#[test]
fn free_move_respects_transform_locks_and_is_only_available_for_translation() {
    let (mut app, window, camera, start) = fixture(Tool::Move, false, false);
    let center = project(&app, camera, start.translation);
    for tool in [Tool::Universal, Tool::Rotate, Tool::Scale] {
        app.world_mut().resource_mut::<Editor>().tool = tool;
        cursor(&mut app, window, center);
        assert_ne!(
            app.world().resource::<PointerState>().hover,
            Some(TransformHandle::ViewTranslation)
        );
    }
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.tool = Tool::Move;
        editor.imported.push(super::super::Imported {
            entity: Entity::PLACEHOLDER,
            name: "Locked".into(),
            original: start.into(),
            parent: GlobalTransform::IDENTITY,
            local: start,
        });
        editor.project.scenes[0].node_policies.insert(
            "Locked".into(),
            crate::NodePolicy {
                translation: [true, false, true],
                ..default()
            },
        );
        editor.selection = Some(Selection::Imported(0));
        assert!(!view_translation_allowed(&editor, Selection::Imported(0)));
    }
    cursor(&mut app, window, center);
    assert_ne!(
        app.world().resource::<PointerState>().hover,
        Some(TransformHandle::ViewTranslation)
    );
}
