use super::*;
use crate::scene_format::{Entity as Node, Instance, Resource, Scene as Source};
use std::sync::atomic::{AtomicU32, Ordering};

pub(in crate::editor) struct Fixture {
    pub root: PathBuf,
    pub project: EditorProject,
    pub keep: bool,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.keep {
            assert!(
                self.root.is_absolute()
                    && self.root.starts_with(std::env::temp_dir())
                    && self
                        .root
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("bevy-scene-ui-")
            );
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
pub(in crate::editor) fn fixture() -> Fixture {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "bevy-scene-ui-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/assets/models/block.glb"),
        root.join("models/block.glb"),
    )
    .unwrap();
    let mut prefab = Source::new("Лампа");
    prefab.resources.insert(
        "model".into(),
        Resource::external("model", "models/block.glb"),
    );
    let mut group = Node::new("stand");
    group.name = Some("Стойка".into());
    group.transform.scale = [1.6, 1.0, 0.65];
    let mut mesh = Node::new("mesh");
    mesh.name = Some("Куб лампы".into());
    mesh.parent = Some("stand".into());
    mesh.model = Some("model".into());
    mesh.transform.translation = [0.0, 0.5, 0.0];
    mesh.transform.rotation = Quat::from_rotation_y(0.4).to_array();
    let mut bulb = Node::new("bulb");
    bulb.name = Some("Свет лампы".into());
    bulb.parent = Some("stand".into());
    bulb.transform.translation = [0.0, 2.0, 1.0];
    let mut light = serde_json::to_value(crate::LightPreset::Bulb.create(1)).unwrap();
    for key in ["id", "name", "pose"] {
        light.as_object_mut().unwrap().remove(key);
    }
    bulb.components.insert("light".into(), light);
    prefab.entities = vec![group, mesh, bulb];
    let mut room = Source::new("Комната с экземплярами");
    room.resources
        .insert("lamp".into(), Resource::external("scene", "lamp.scene"));
    let mut group = Node::new("room");
    group.name = Some("Мебель".into());
    let mut a = Node::new("a");
    a.name = Some("Лампа A".into());
    a.parent = Some("room".into());
    a.transform.translation = [-1.0, 0.0, 0.0];
    a.instance = Some(Instance {
        resource: "lamp".into(),
        overrides: default(),
    });
    let mut b = a.clone();
    b.id = "b".into();
    b.name = Some("Лампа B".into());
    b.transform.translation = [1.5, 0.0, 0.0];
    let mut camera = Node::new("camera");
    camera.name = Some("Главная камера".into());
    camera.transform = crate::scene_io::transform(
        Transform::from_xyz(5.0, 4.0, 7.0)
            .looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y)
            .into(),
    );
    camera.components.insert(
        "camera".into(),
        serde_json::json!({"id":"main","projection":"perspective","lens":50.0}),
    );
    room.entities = vec![group, a, b, camera];
    room.settings.insert(
        "environment".into(),
        serde_json::to_value(crate::SceneLighting::editor_default()).unwrap(),
    );
    for (name, source) in [("room.scene", room), ("lamp.scene", prefab)] {
        std::fs::write(root.join(name), source.to_source().unwrap()).unwrap();
    }
    let scenes = ["room.scene", "lamp.scene"]
        .into_iter()
        .map(|file| {
            let edits = RoomEdits::<String>::read(&root, file).unwrap();
            let settings = edits.scene.unwrap();
            SceneDefinition {
                label: settings.label,
                asset: String::new(),
                file: file.into(),
                groups: vec![],
                excluded_nodes: vec![],
                node_policies: default(),
                cameras: settings.cameras,
                initial_selection: None,
                play: None,
            }
        })
        .collect();
    let project = EditorProject {
        title: "Иерархия сцен".into(),
        asset_root: root.clone(),
        scenes,
        catalog: vec![],
        scene_setup: None,
        camera_setup: None,
    };
    project.validate().unwrap();
    Fixture {
        root,
        project,
        keep: false,
    }
}
fn app(f: &Fixture) -> App {
    let edits = f.project.read_scene(0).unwrap();
    let mut editor = Editor::new(f.project.clone(), edits);
    editor.ready = true;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin))
        .insert_resource(editor)
        .init_resource::<Scene>()
        .init_resource::<viewport::PointerState>()
        .add_message::<Action>()
        .add_systems(
            Update,
            (handle_actions, sync, object_visibility::sync).chain(),
        );
    app
}
fn send(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    app.update();
}
fn key(app: &App, id: &str) -> u32 {
    app.world()
        .resource::<Editor>()
        .edits
        .hierarchy
        .as_ref()
        .unwrap()
        .by_id(id)
        .unwrap()
        .key
}

#[test]
fn scene_hierarchy_ui_group_duplicate_delete_and_history_keep_authoring_structure() {
    let f = fixture();
    let mut app = app(&f);
    let group = key(&app, "room");
    send(&mut app, Action::Select(Selection::Node(group)));
    let before = app.world().resource::<Editor>().edits.clone();
    send(&mut app, Action::Duplicate);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 4);
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    send(&mut app, Action::Delete);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 2);
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 4);
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    send(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 4);
}

#[test]
fn scene_hierarchy_ui_inherited_structure_actions_are_undoable_and_saveable() {
    let f = fixture();
    let mut app = app(&f);
    let inherited = key(&app, "a/stand");
    let parent = key(&app, "b");
    send(&mut app, Action::Select(Selection::Node(inherited)));
    send(&mut app, Action::Hierarchy(HierarchyAction::PickParent));
    send(&mut app, Action::Select(Selection::Node(parent)));
    let graph = |app: &App| {
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .clone()
            .unwrap()
    };
    assert_eq!(graph(&app).parent(inherited).unwrap().key, parent);
    send(&mut app, Action::Hierarchy(HierarchyAction::SiblingLater));
    assert!(graph(&app).sibling_offset(inherited, true).is_none());
    send(&mut app, Action::Undo);
    assert!(graph(&app).sibling_offset(inherited, true).is_some());
    send(&mut app, Action::Undo);
    assert_eq!(graph(&app).parent(inherited).unwrap().entity.id, "a");
    send(&mut app, Action::Redo);
    send(&mut app, Action::Save);
    let saved = RoomEdits::<String>::read(&f.root, "room.scene").unwrap();
    assert_eq!(
        saved.hierarchy.unwrap().parent(inherited).unwrap().key,
        parent
    );
}

#[test]
fn scene_hierarchy_tree_drop_is_atomic_and_preserves_instance_ids() {
    let f = fixture();
    let mut app = app(&f);
    let stand = key(&app, "a/stand");
    let target = key(&app, "b/stand");
    let mount = key(&app, "b");
    let graph = |app: &App| {
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .clone()
            .unwrap()
    };
    let before = graph(&app);
    send(
        &mut app,
        Action::Hierarchy(HierarchyAction::Drop(
            0,
            Selection::Node(stand),
            TreeDrop::Before(Selection::Node(target)),
        )),
    );
    let after = graph(&app);
    assert_eq!(after.parent(stand).unwrap().key, mount);
    assert!(
        after
            .node(stand)
            .unwrap()
            .affine()
            .abs_diff_eq(before.node(stand).unwrap().affine(), 1e-5)
    );
    assert!(after.node(stand).unwrap().entity.id == "a/stand");
    assert!(after.overridden(stand));
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    send(
        &mut app,
        Action::Hierarchy(HierarchyAction::Drop(
            0,
            Selection::Node(stand),
            TreeDrop::Before(Selection::Node(target)),
        )),
    );
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    send(
        &mut app,
        Action::Hierarchy(HierarchyAction::Drop(
            0,
            Selection::Node(mount),
            TreeDrop::Inside(Selection::Node(stand)),
        )),
    );
    assert_eq!(graph(&app), after);
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    send(&mut app, Action::Undo);
    assert_eq!(graph(&app), before);
}

#[test]
fn scene_hierarchy_tree_drop_flat_order_survives_reopen() {
    let f = fixture();
    let model = |id| PlacedModel {
        id,
        asset: "models/block.glb".into(),
        pose: Transform::IDENTITY.into(),
        bounds: None,
    };
    let flat = RoomEdits::<String> {
        scene: Some(crate::SceneSettings {
            label: f.project.scenes[0].label.clone(),
            cameras: f.project.scenes[0].cameras.clone(),
        }),
        models: vec![model(1), model(2)],
        ..default()
    };
    std::fs::write(
        f.root.join("room.scene"),
        serde_json::to_vec(&flat).unwrap(),
    )
    .unwrap();
    let mut app = app(&f);
    send(
        &mut app,
        Action::Hierarchy(HierarchyAction::Drop(
            0,
            Selection::Model(2),
            TreeDrop::Before(Selection::Model(1)),
        )),
    );
    send(&mut app, Action::Save);
    let reloaded = RoomEdits::<String>::read(&f.root, "room.scene").unwrap();
    let graph = reloaded.hierarchy.unwrap();
    let models: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.entity.model.is_some())
        .map(|n| n.entity.id.as_str())
        .collect();
    assert_eq!(models, ["model:2", "model:1"]);
    assert!(graph.nodes.iter().all(|n| n.entity.parent.is_none()));
}

#[test]
fn scene_hierarchy_ui_parent_picker_is_undoable_and_preserves_world_pose() {
    let f = fixture();
    let mut app = app(&f);
    let a = key(&app, "a");
    let b = key(&app, "b");
    let original = app
        .world()
        .resource::<Editor>()
        .edits
        .hierarchy
        .as_ref()
        .unwrap()
        .node(a)
        .unwrap()
        .world;
    send(&mut app, Action::Select(Selection::Node(a)));
    send(&mut app, Action::Hierarchy(HierarchyAction::PickParent));
    send(&mut app, Action::Select(Selection::Node(b)));
    let h = app
        .world()
        .resource::<Editor>()
        .edits
        .hierarchy
        .as_ref()
        .unwrap();
    assert_eq!(h.parent(a).unwrap().key, b);
    assert_eq!(h.node(a).unwrap().world, original);
    send(&mut app, Action::Undo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .parent(a)
            .unwrap()
            .entity
            .id,
        "room"
    );
    send(&mut app, Action::Redo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .parent(a)
            .unwrap()
            .key,
        b
    );
}

#[test]
fn scene_hierarchy_ui_local_inspector_overrides_and_tabs_keep_independent_histories() {
    let f = fixture();
    let mut app = app(&f);
    let model = key(&app, "a/mesh");
    let before = app.world().resource::<Editor>().edits.clone();
    send(&mut app, Action::Select(Selection::Model(model)));
    send(&mut app, Action::LocalSpace);
    send(&mut app, Action::Number(Field::Position(1), 2.0));
    let edited = app.world().resource::<Editor>().edits.clone();
    assert_eq!(
        edited
            .hierarchy
            .as_ref()
            .unwrap()
            .node(model)
            .unwrap()
            .local()
            .position[1],
        2.0
    );
    assert!(edited.hierarchy.as_ref().unwrap().overridden(model));
    send(&mut app, Action::Open(1));
    assert!(!app.world().resource::<Editor>().dirty());
    send(&mut app, Action::Open(0));
    app.world_mut().resource_mut::<Editor>().ready = true;
    assert_eq!(app.world().resource::<Editor>().edits, edited);
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    send(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().edits, edited);
    send(&mut app, Action::Save);
    assert!(!app.world().resource::<Editor>().dirty());
    let loaded = RoomEdits::<String>::read(&f.root, "room.scene").unwrap();
    assert_eq!(
        loaded
            .hierarchy
            .as_ref()
            .unwrap()
            .node(model)
            .unwrap()
            .local()
            .position[1],
        2.0
    );
}

#[test]
fn scene_hierarchy_ui_reset_unpack_and_cycle_failure_are_transactional() {
    let f = fixture();
    let mut app = app(&f);
    let mesh = key(&app, "a/mesh");
    let mount = key(&app, "a");
    send(&mut app, Action::Select(Selection::Model(mesh)));
    send(&mut app, Action::Number(Field::Position(0), -2.0));
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    send(&mut app, Action::Hierarchy(HierarchyAction::ResetOverrides));
    assert!(
        !app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    send(&mut app, Action::Select(Selection::Node(mount)));
    send(&mut app, Action::Hierarchy(HierarchyAction::Unpack));
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .node(mount)
            .unwrap()
            .instance
            .is_none()
    );
    send(&mut app, Action::Undo);
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .node(mount)
            .unwrap()
            .instance
            .is_some()
    );
    let before = app.world().resource::<Editor>().edits.clone();
    let history = app.world().resource::<Editor>().undo.len();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let root = editor.root.clone();
        assert!(
            editor
                .edit_hierarchy(|h| h.insert_scene(
                    "room.scene",
                    "Цикл",
                    Transform::IDENTITY.into(),
                    &root
                ))
                .is_none()
        );
    }
    assert_eq!(app.world().resource::<Editor>().edits, before);
    assert_eq!(app.world().resource::<Editor>().undo.len(), history);
}

#[test]
fn scene_hierarchy_ui_spatial_nodes_preserve_shear_and_do_not_change_while_idle() {
    let f = fixture();
    let mut app = app(&f);
    let model = key(&app, "a/mesh");
    let group = key(&app, "room");
    let entity = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::Visible))
        .id();
    app.world_mut()
        .resource_mut::<Scene>()
        .models
        .insert(model, (entity, "models/block.glb".into()));
    app.update();
    app.update();
    let expected = app
        .world()
        .resource::<Editor>()
        .edits
        .hierarchy
        .as_ref()
        .unwrap()
        .node(model)
        .unwrap()
        .affine();
    let world = app.world().get::<GlobalTransform>(entity).unwrap().affine();
    assert!(expected.abs_diff_eq(world, 1e-5));
    let parent = app.world().get::<ChildOf>(entity).unwrap().parent();
    let tick = app
        .world()
        .entity(parent)
        .get_ref::<Transform>()
        .unwrap()
        .last_changed();
    app.update();
    assert_eq!(
        app.world()
            .entity(parent)
            .get_ref::<Transform>()
            .unwrap()
            .last_changed(),
        tick
    );
    // Visibility::Visible children must also be hidden with their authoring group.
    let target = Selection::Node(group);
    let hidden = object_visibility::ObjectKey::Entity("room".into());
    app.world_mut()
        .resource_mut::<Editor>()
        .hidden_objects
        .entry(0)
        .or_default()
        .insert(hidden);
    app.update();
    assert!(
        app.world()
            .resource::<Editor>()
            .object_hidden(Selection::Model(model))
    );
    assert_eq!(
        *app.world().get::<Visibility>(entity).unwrap(),
        Visibility::Hidden
    );
    app.world_mut()
        .resource_mut::<Editor>()
        .hidden_objects
        .clear();
    app.update();
    assert!(!app.world().resource::<Editor>().object_hidden(target));
    assert_eq!(
        *app.world().get::<Visibility>(entity).unwrap(),
        Visibility::Visible
    );
}

#[test]
fn scene_hierarchy_ui_parent_and_child_selection_transforms_each_subtree_once() {
    let f = fixture();
    let mut app = app(&f);
    let group = key(&app, "room");
    let child = key(&app, "a/mesh");
    let mut editor = app.world_mut().resource_mut::<Editor>();
    editor.select_targets(
        vec![Selection::Node(group), Selection::Model(child)],
        area_selection::Mode::Replace,
    );
    assert_eq!(editor.transform_targets(), vec![Selection::Node(group)]);
    let before = editor
        .edits
        .hierarchy
        .as_ref()
        .unwrap()
        .node(child)
        .unwrap()
        .affine();
    let mut pose = editor.pose(Selection::Node(group)).unwrap().transform();
    pose.translation.x += 1.0;
    editor.set_poses(&[(Selection::Node(group), pose)]);
    let after = editor
        .edits
        .hierarchy
        .as_ref()
        .unwrap()
        .node(child)
        .unwrap()
        .affine();
    assert!((after.translation.x - before.translation.x - 1.0).abs() < 1e-4);
}

#[test]
fn scene_hierarchy_ui_flat_upgrade_migrates_history_and_can_undo_then_save() {
    let f = fixture();
    let settings = f.project.scenes[0].clone();
    let flat = RoomEdits::<String> {
        scene: Some(crate::SceneSettings {
            label: settings.label,
            cameras: settings.cameras,
        }),
        models: vec![PlacedModel {
            id: 1,
            asset: "models/block.glb".into(),
            pose: Transform::IDENTITY.into(),
            bounds: None,
        }],
        ..default()
    };
    std::fs::write(
        f.root.join("room.scene"),
        serde_json::to_vec(&flat).unwrap(),
    )
    .unwrap();
    let mut app = app(&f);
    assert!(app.world().resource::<Editor>().edits.hierarchy.is_none());
    send(&mut app, Action::Select(Selection::Model(1)));
    send(&mut app, Action::Number(Field::Position(0), 2.0));
    send(&mut app, Action::Hierarchy(HierarchyAction::GroupSelection));
    assert!(app.world().resource::<Editor>().edits.hierarchy.is_some());
    assert_eq!(app.world().resource::<Editor>().undo.len(), 2);
    assert!(
        app.world()
            .resource::<Editor>()
            .undo
            .iter()
            .all(|e| e.hierarchy.is_some())
    );
    send(&mut app, Action::Save);
    assert!(!app.world().resource::<Editor>().dirty());
    let uuid = app
        .world()
        .resource::<Editor>()
        .edits
        .hierarchy
        .as_ref()
        .unwrap()
        .source()
        .id
        .clone();
    send(&mut app, Action::Undo);
    send(&mut app, Action::Save);
    assert!(!app.world().resource::<Editor>().dirty());
    let source = Source::from_slice(&std::fs::read(f.root.join("room.scene")).unwrap()).unwrap();
    assert_eq!(source.id, uuid);
    assert!(source.entities.iter().all(|e| e.parent.is_none()));
    send(&mut app, Action::Undo);
    send(&mut app, Action::Save);
    assert!(!app.world().resource::<Editor>().dirty());
    assert_eq!(
        RoomEdits::<String>::read(&f.root, "room.scene")
            .unwrap()
            .models[0]
            .pose
            .position[0],
        0.0
    );
    send(&mut app, Action::Redo);
    send(&mut app, Action::Redo);
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|n| n.entity.parent.is_some())
    );
}

#[test]
fn scene_hierarchy_ui_multi_transform_failure_rolls_back_every_target() {
    let f = fixture();
    let mut app = app(&f);
    let mount = key(&app, "b");
    let child = key(&app, "a/mesh");
    let mut editor = app.world_mut().resource_mut::<Editor>();
    let before = editor.edits.clone();
    let mut first = editor.pose(Selection::Node(mount)).unwrap().transform();
    first.translation.x += 1.0;
    let mut second = editor.pose(Selection::Model(child)).unwrap().transform();
    second.rotation = Quat::from_rotation_z(0.4);
    editor.set_poses(&[
        (Selection::Node(mount), first),
        (Selection::Model(child), second),
    ]);
    assert_eq!(editor.edits, before);
    assert!(editor.status.contains("shear"));
}

#[test]
fn scene_hierarchy_ui_camera_transform_and_lens_reset_restore_the_saved_graph() {
    let f = fixture();
    let mut app = app(&f);
    let original = app.world().resource::<Editor>().edits.clone();
    send(&mut app, Action::Select(Selection::Camera(0)));
    send(&mut app, Action::Number(Field::Position(0), 8.0));
    send(&mut app, Action::Number(Field::Lens, 65.0));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).pose.position[0], 8.0);
    assert_eq!(editor.camera(0).lens, 65.0);
    assert!(editor.dirty());
    send(&mut app, Action::Reset(Reset::All));
    assert_eq!(app.world().resource::<Editor>().edits, original);
    assert!(!app.world().resource::<Editor>().dirty());
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 65.0);
}

#[test]
fn scene_hierarchy_ui_visibility_is_shared_by_components_and_cameras_and_survives_restart() {
    let f = fixture();
    let mut source =
        Source::from_slice(&std::fs::read(f.root.join("lamp.scene")).unwrap()).unwrap();
    let light = source
        .entities
        .iter()
        .find(|e| e.id == "bulb")
        .unwrap()
        .components["light"]
        .clone();
    source
        .entities
        .iter_mut()
        .find(|e| e.id == "mesh")
        .unwrap()
        .components
        .insert("light".into(), light);
    std::fs::write(f.root.join("lamp.scene"), source.to_source().unwrap()).unwrap();
    let mut app = app(&f);
    let mesh = key(&app, "a/mesh");
    let group = key(&app, "room");
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.toggle_visibility(Selection::Model(mesh));
        assert!(editor.object_hidden(Selection::Light(mesh)));
        editor.toggle_visibility(Selection::Light(mesh));
        assert!(!editor.object_hidden(Selection::Model(mesh)));
    }
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.toggle_visibility(Selection::Camera(0));
        assert!(editor.object_hidden(Selection::Camera(0)));
        editor.toggle_visibility(Selection::Node(group));
        assert!(editor.object_hidden(Selection::Model(mesh)));
        assert!(!editor.can_toggle_visibility(Selection::Model(mesh)));
        let flags = editor.hidden_objects.clone();
        editor.toggle_visibility(Selection::Model(mesh));
        assert_eq!(editor.hidden_objects, flags);
        editor.toggle_visibility(Selection::Node(group));
        assert!(!editor.object_hidden(Selection::Model(mesh)));
    }
    let reopened = Editor::new(f.project.clone(), f.project.read_scene(0).unwrap());
    assert!(reopened.object_hidden(Selection::Camera(0)));
    assert!(!reopened.object_hidden(Selection::Model(mesh)));
    assert!(!reopened.dirty());
}

#[test]
fn scene_hierarchy_ui_renaming_source_remaps_open_drafts_and_history() {
    let f = fixture();
    let mut app = app(&f);
    let mesh = key(&app, "a/mesh");
    let mount = key(&app, "a");
    send(&mut app, Action::Select(Selection::Model(mesh)));
    send(&mut app, Action::Number(Field::Position(0), -2.0));
    let mut editor = app.world_mut().resource_mut::<Editor>();
    let mut browser = browser::Browser::new(&mut editor.project);
    let file = browser
        .files
        .iter()
        .position(|f| f.path == std::path::Path::new("lamp.scene"))
        .unwrap();
    assert!(
        file_moves::rename_entry(
            &mut editor,
            &mut browser,
            browser::Entry::Scene { file, room: 1 },
            "renamed.scene"
        )
        .unwrap()
    );
    assert_eq!(
        editor
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .node(mount)
            .unwrap()
            .instance
            .as_deref(),
        Some("renamed.scene")
    );
    assert_eq!(
        editor.undo[0]
            .hierarchy
            .as_ref()
            .unwrap()
            .source_path(mount),
        Some("renamed.scene")
    );
    assert!(editor.dirty());
    editor.edits.write(&f.root, "room.scene").unwrap();
    let snapshot = serde_json::to_vec(&editor.edits).unwrap();
    let restored: RoomEdits = serde_json::from_slice(&snapshot).unwrap();
    assert_eq!(restored, editor.edits);
    drop(editor);
    send(&mut app, Action::Undo);
    assert!(!app.world().resource::<Editor>().dirty());
    assert!(RoomEdits::<String>::read(&f.root, "room.scene").is_ok());
}

#[test]
fn scene_hierarchy_ui_browser_scene_drop_is_one_undo_step_and_preserves_saved_source() {
    let mut f = fixture();
    let browser = browser::Browser::new(&mut f.project);
    let mut app = app(&f);
    let file = browser
        .files
        .iter()
        .position(|e| e.path == std::path::Path::new("lamp.scene"))
        .unwrap();
    let source = std::fs::read(f.root.join("lamp.scene")).unwrap();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        assert!(apply_asset_drop(
            &mut editor,
            &browser,
            browser::Entry::Scene { file, room: 1 },
            Vec3::new(4.0, 0.0, 0.0),
            None
        ));
        assert_eq!(editor.edits.models.len(), 3);
        assert_eq!(editor.undo.len(), 1);
    }
    send(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 2);
    send(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 3);
    assert_eq!(std::fs::read(f.root.join("lamp.scene")).unwrap(), source);
}
