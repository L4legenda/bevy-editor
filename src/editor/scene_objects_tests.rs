use super::*;
use crate::{
    EditorProject,
    editor::{Imported, RoomEdits, Scene, handle_actions, object_visibility},
};

fn app() -> App {
    let project = EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native/project.json"),
    )
    .unwrap();
    let edits = project.read_scene(0).unwrap();
    let mut editor = Editor::new(project, edits);
    editor.ready = true;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<Scene>()
        .init_resource::<viewport::PointerState>()
        .add_message::<Action>()
        .add_systems(Update, (handle_actions, object_visibility::sync).chain());
    app
}

fn send(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    app.update();
}

#[test]
fn scene_objects_names_and_deletion_roundtrip_and_undo_without_changing_assets() {
    let mut app = app();
    let target = Selection::Model(app.world().resource::<Editor>().edits.models[0].id);
    let (id, asset, catalog) = {
        let editor = app.world().resource::<Editor>();
        let Selection::Model(id) = target else {
            unreachable!()
        };
        (
            id,
            editor.model(id).unwrap().asset.clone(),
            serde_json::to_value(&editor.project.catalog).unwrap(),
        )
    };
    let original = app.world().resource::<Editor>().label(target);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.select(Some(target));
        editor.rename_object(target, "  Главный куб  ").unwrap();
        assert_eq!(editor.label(target), "Главный куб");
        assert_eq!(editor.model(id).unwrap().asset, asset);
        assert_eq!(
            serde_json::to_value(&editor.project.catalog).unwrap(),
            catalog
        );
        assert_eq!(editor.undo.len(), 1);
        for invalid in ["", "  ", "Куб\nвторой", &"я".repeat(129)] {
            assert!(editor.rename_object(target, invalid).is_err());
        }
        assert_eq!(editor.undo.len(), 1);
    }
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().label(target), original);
    send(&mut app, Action::Redo);
    assert_eq!(
        app.world().resource::<Editor>().label(target),
        "Главный куб"
    );
    let saved = app.world().resource::<Editor>().edits.clone();
    let json = serde_json::to_vec(&saved).unwrap();
    let reopened: RoomEdits = serde_json::from_slice(&json).unwrap();
    reopened
        .validate(&app.world().resource::<Editor>().root)
        .unwrap();
    assert_eq!(reopened.model_names[&id], "Главный куб");
    assert_eq!(reopened, saved);
    send(&mut app, Action::Delete);
    let editor = app.world().resource::<Editor>();
    assert!(editor.model(id).is_none());
    assert!(!editor.edits.model_names.contains_key(&id));
    assert!(editor.root.join(&asset).is_file());
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits, saved);
    send(&mut app, Action::Redo);
    assert!(app.world().resource::<Editor>().model(id).is_none());
}

#[test]
fn scene_objects_imported_removal_hides_descendants_and_undo_restores_names_and_visibility() {
    let mut app = app();
    // Imported placements belong to legacy GLB-backed scenes.
    app.world_mut().resource_mut::<Editor>().edits.scene = None;
    let root = app
        .world_mut()
        .spawn((Name::new("OriginalNode"), Visibility::Inherited))
        .id();
    let child = app
        .world_mut()
        .spawn((Visibility::Visible, ChildOf(root)))
        .id();
    let target = Selection::Imported(0);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.imported.push(Imported {
            entity: root,
            name: "OriginalNode".into(),
            original: Transform::IDENTITY.into(),
            parent: GlobalTransform::IDENTITY,
            local: Transform::IDENTITY,
        });
        editor.imported.push(Imported {
            entity: child,
            name: "ChildNode".into(),
            original: Transform::IDENTITY.into(),
            parent: GlobalTransform::IDENTITY,
            local: Transform::IDENTITY,
        });
        editor
            .imported_ancestors
            .insert("ChildNode".into(), vec!["OriginalNode".into()]);
        editor.select(Some(target));
        editor.rename_object(target, "Декорация").unwrap();
        assert_eq!(editor.imported[0].name, "OriginalNode");
    }
    send(&mut app, Action::Delete);
    {
        let editor = app.world().resource::<Editor>();
        assert!(editor.object_deleted(target));
        assert!(editor.object_deleted(Selection::Imported(1)));
        assert!(editor.pose(Selection::Imported(1)).is_none());
        assert!(editor.pose(target).is_none());
        assert!(!editor.deletable_targets());
        assert!(editor.edits.deleted_nodes.contains("OriginalNode"));
    }
    assert_eq!(
        *app.world().get::<Visibility>(root).unwrap(),
        Visibility::Hidden
    );
    assert_eq!(
        *app.world().get::<Visibility>(child).unwrap(),
        Visibility::Hidden
    );
    assert_eq!(
        app.world().get::<Name>(root).unwrap().as_str(),
        "OriginalNode"
    );
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().label(target), "Декорация");
    assert!(app.world().resource::<Editor>().pose(target).is_some());
    assert!(
        app.world()
            .resource::<Editor>()
            .pose(Selection::Imported(1))
            .is_some()
    );
    assert_eq!(
        *app.world().get::<Visibility>(root).unwrap(),
        Visibility::Inherited
    );
    assert_eq!(
        *app.world().get::<Visibility>(child).unwrap(),
        Visibility::Visible
    );
    send(&mut app, Action::Redo);
    assert_eq!(
        *app.world().get::<Visibility>(child).unwrap(),
        Visibility::Hidden
    );
}

#[test]
fn scene_objects_renamed_instances_duplicate_independently_and_old_documents_still_load() {
    let mut app = app();
    let target = Selection::Model(app.world().resource::<Editor>().edits.models[0].id);
    let mut editor = app.world_mut().resource_mut::<Editor>();
    editor.select(Some(target));
    editor.rename_object(target, "Куб").unwrap();
    editor.duplicate_targets();
    let copy = editor.selection.unwrap();
    editor.rename_object(copy, "Куб второй").unwrap();
    assert_eq!(editor.label(target), "Куб");
    assert_eq!(editor.label(copy), "Куб второй");
    let old: RoomEdits = serde_json::from_str("{}").unwrap();
    assert!(
        old.model_names.is_empty() && old.node_names.is_empty() && old.deleted_nodes.is_empty()
    );
}

#[cfg(not(target_arch = "wasm32"))]
mod gpu {
    use super::*;
    use crate::editor::{
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render,
    };
    use bevy::{
        camera::RenderTarget,
        input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
            mouse::MouseButtonInput,
        },
        picking::pointer::{Location, PointerAction, PointerId, PointerInput},
        ui::IsDefaultUiCamera,
    };

    fn warm(app: &mut App, count: usize) {
        for _ in 0..count {
            frame(app);
        }
    }
    fn control_point(app: &mut App, wanted: impl Fn(Action) -> bool) -> Vec2 {
        let world = app.world_mut();
        world
            .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find_map(|(control, node, transform)| {
                (wanted(control.action) && !node.is_empty())
                    .then_some(transform.affine().translation * node.inverse_scale_factor())
            })
            .unwrap()
    }
    fn click(app: &mut App, window: Entity, point: Vec2, button: MouseButton) {
        lighting_ui_render::move_pointer(app, window, point);
        let target = app
            .world_mut()
            .query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>()
            .single(app.world())
            .unwrap()
            .normalize(Some(window))
            .unwrap();
        let pointer_button = if button == MouseButton::Right {
            PointerButton::Secondary
        } else {
            PointerButton::Primary
        };
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(MouseButtonInput {
                window,
                button,
                state,
            });
            app.world_mut().write_message(PointerInput {
                pointer_id: PointerId::Mouse,
                location: Location {
                    target: target.clone(),
                    position: point,
                },
                action: if state == ButtonState::Pressed {
                    PointerAction::Press(pointer_button)
                } else {
                    PointerAction::Release(pointer_button)
                },
            });
            warm(app, 2);
        }
        warm(app, 3);
    }
    fn key(app: &mut App, window: Entity, key_code: KeyCode, logical_key: Key) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                window,
                key_code,
                logical_key: logical_key.clone(),
                state,
                text: None,
                repeat: false,
            });
            warm(app, 3);
        }
    }

    #[test]
    #[ignore = "requires GPU; selects outliner rows and deletes objects without leaving W/E/R"]
    fn gpu_scene_objects_tree_and_delete_work_with_every_transform_tool() {
        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/native/project.json"),
        )
        .unwrap();
        let (mut app, window, target_image) = lighting_ui_render::fixture_project(project);
        app.world_mut().resource_mut::<Editor>().drawer = false;
        warm(&mut app, 8);
        let (target, other, original) = {
            let editor = app.world().resource::<Editor>();
            assert!(editor.edits.models.len() >= 2);
            (
                Selection::Model(editor.edits.models[0].id),
                Selection::Model(editor.edits.models[1].id),
                editor.edits.clone(),
            )
        };
        let output =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/scene-objects-qa");
        std::fs::create_dir_all(&output).unwrap();
        for (tool, name) in [
            (Tool::Move, "move"),
            (Tool::Rotate, "rotate"),
            (Tool::Scale, "scale"),
        ] {
            {
                let mut editor = app.world_mut().resource_mut::<Editor>();
                editor.select(None);
                editor.tool = tool;
            }
            warm(&mut app, 4);
            {
                let world = app.world_mut();
                let mut rows = world.query::<(&Control, Has<bevy::ui::InteractionDisabled>)>();
                let (_, disabled) = rows
                    .iter(world)
                    .find(|(control, _)| matches!(control.action, Action::Select(t) if t == target))
                    .unwrap();
                assert!(!disabled, "The tree row must remain enabled in {tool:?}");
            }
            for selection in [target, other] {
                let point = control_point(
                    &mut app,
                    |action| matches!(action, Action::Select(t) if t == selection),
                );
                click(&mut app, window, point, MouseButton::Left);
                let editor = app.world().resource::<Editor>();
                assert_eq!(
                    editor.selection,
                    Some(selection),
                    "Tree selection must work in {tool:?}"
                );
                assert_eq!(editor.tool, tool);
                assert_eq!(
                    editor.edits, original,
                    "Selecting a row must not transform the object"
                );
            }
            // Right-click a different object: deletion must use this row rather
            // than the previously selected object, and must keep the active tool.
            let point = control_point(
                &mut app,
                |action| matches!(action, Action::Select(t) if t == target),
            );
            click(&mut app, window, point, MouseButton::Right);
            assert!(app.world().resource::<Editor>().scene_object_popup);
            assert_eq!(app.world().resource::<Editor>().selection, Some(target));
            let duplicate = control_point(&mut app, |action| {
                matches!(action, Action::SceneObject(ObjectAction::Duplicate))
            });
            click(&mut app, window, duplicate, MouseButton::Left);
            {
                let editor = app.world().resource::<Editor>();
                assert_eq!(editor.edits.models.len(), original.models.len() + 1);
                assert!(editor.pose(target).is_some());
                assert_ne!(editor.selection, Some(target));
            }
            app.world_mut().write_message(Action::Undo);
            warm(&mut app, 5);
            assert_eq!(app.world().resource::<Editor>().edits, original);
            let point = control_point(
                &mut app,
                |action| matches!(action, Action::Select(t) if t == target),
            );
            click(&mut app, window, point, MouseButton::Right);
            capture_when(
                &mut app,
                &target_image,
                &output.join(format!("{name}-context-menu.png")),
                |_| true,
            );
            let point = control_point(&mut app, |action| {
                matches!(action, Action::SceneObject(ObjectAction::Remove))
            });
            click(&mut app, window, point, MouseButton::Left);
            let editor = app.world().resource::<Editor>();
            assert!(editor.pose(target).is_none());
            assert!(editor.pose(other).is_some());
            assert_eq!(editor.tool, tool);
            app.world_mut().write_message(Action::Undo);
            warm(&mut app, 5);
            assert_eq!(app.world().resource::<Editor>().edits, original);
        }
    }

    #[test]
    #[ignore = "requires GPU; moves a nested instance through the outliner context menu"]
    fn gpu_scene_objects_hierarchy_commands_live_in_tree_menu() {
        let files = crate::editor::scene_hierarchy::tests::fixture();
        let (mut app, window, _) = lighting_ui_render::fixture_project(files.project.clone());
        let key = {
            let editor = app.world().resource::<Editor>();
            editor
                .edits
                .hierarchy
                .as_ref()
                .unwrap()
                .by_id("a")
                .unwrap()
                .key
        };
        let target = Selection::Node(key);
        let point = control_point(
            &mut app,
            |action| matches!(action, Action::Select(selection) if selection == target),
        );
        click(&mut app, window, point, MouseButton::Right);
        let world = app.world_mut();
        assert!(
            !world
                .query::<(&Control, &AccessibleLabel)>()
                .iter(world)
                .any(
                    |(control, label)| matches!(control.action, Action::Inspector(_))
                        && label.0 == "Узел сцены"
                )
        );
        control_point(&mut app, |action| {
            matches!(action, Action::Hierarchy(HierarchyAction::OpenSource))
        });
        let point = control_point(&mut app, |action| {
            matches!(
                action,
                Action::SceneObject(ObjectAction::Hierarchy(HierarchyAction::ToRoot))
            )
        });
        click(&mut app, window, point, MouseButton::Left);
        let editor = app.world().resource::<Editor>();
        assert!(
            editor
                .edits
                .hierarchy
                .as_ref()
                .unwrap()
                .parent(key)
                .is_none()
        );
        assert!(!editor.scene_object_popup);
        app.world_mut().write_message(Action::Undo);
        warm(&mut app, 4);
        let editor = app.world().resource::<Editor>();
        assert!(
            editor
                .edits
                .hierarchy
                .as_ref()
                .unwrap()
                .parent(key)
                .is_some()
        );

        let point = control_point(&mut app, |action| matches!(action, Action::SceneAddMenu));
        click(&mut app, window, point, MouseButton::Left);
        let point = control_point(&mut app, |action| {
            matches!(action, Action::Hierarchy(HierarchyAction::AddFolder))
        });
        click(&mut app, window, point, MouseButton::Left);
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.label(editor.selection.unwrap()), "Папка");
        assert!(!app.world().resource::<LightingUi>().is_menu_open());
    }

    #[test]
    #[ignore = "requires GPU; renames and removes a real outliner object using its context menu, F2 and undo"]
    fn gpu_scene_objects_context_rename_delete_and_undo() {
        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/native/project.json"),
        )
        .unwrap();
        let (mut app, window, target_image) = lighting_ui_render::fixture_project(project);
        app.world_mut().resource_mut::<Editor>().drawer = false;
        warm(&mut app, 25);
        let target = Selection::Model(app.world().resource::<Editor>().edits.models[0].id);
        let point = control_point(
            &mut app,
            |action| matches!(action, Action::Select(t) if t == target),
        );
        click(&mut app, window, point, MouseButton::Right);
        assert!(app.world().resource::<Editor>().scene_object_popup);
        let output =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/scene-objects-qa");
        std::fs::create_dir_all(&output).unwrap();
        capture_when(
            &mut app,
            &target_image,
            &output.join("01-context-menu.png"),
            |_| true,
        );
        let point = control_point(&mut app, |action| {
            matches!(action, Action::SceneObject(ObjectAction::EditName))
        });
        click(&mut app, window, point, MouseButton::Left);
        let field = app.world().resource::<ObjectUi>().field;
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text("Главный куб");
        warm(&mut app, 2);
        capture_when(
            &mut app,
            &target_image,
            &output.join("02-rename.png"),
            |_| true,
        );
        key(&mut app, window, KeyCode::Enter, Key::Enter);
        assert_eq!(
            app.world().resource::<Editor>().label(target),
            "Главный куб"
        );
        assert!(!app.world().resource::<Editor>().scene_object_popup);
        // The tree menu and F2 must also work while a transform tool is active.
        app.world_mut().write_message(Action::Tool(Tool::Move));
        warm(&mut app, 3);
        key(&mut app, window, KeyCode::F2, Key::F2);
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text("Не применять");
        key(&mut app, window, KeyCode::Escape, Key::Escape);
        assert_eq!(
            app.world().resource::<Editor>().label(target),
            "Главный куб"
        );
        let point = control_point(
            &mut app,
            |action| matches!(action, Action::Select(t) if t == target),
        );
        click(&mut app, window, point, MouseButton::Right);
        let point = control_point(&mut app, |action| {
            matches!(action, Action::SceneObject(ObjectAction::Remove))
        });
        click(&mut app, window, point, MouseButton::Left);
        assert!(app.world().resource::<Editor>().pose(target).is_none());
        app.world_mut().write_message(Action::Undo);
        warm(&mut app, 4);
        assert!(app.world().resource::<Editor>().pose(target).is_some());
        assert_eq!(
            app.world().resource::<Editor>().label(target),
            "Главный куб"
        );
        capture_when(
            &mut app,
            &target_image,
            &output.join("03-restored.png"),
            |_| true,
        );
    }
}
