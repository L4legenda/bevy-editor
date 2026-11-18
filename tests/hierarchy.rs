//! End-to-end scene authoring contracts, using the same document API as the UI.
use bevy::{math::Affine3A, prelude::*};
use bevy_editor::{
    LevelEdits, Pose,
    hierarchy::SceneHierarchy,
    scene_format::{Entity as Node, Instance, Resource, Scene},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "bevy-hierarchy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(root.join("models/block.glb"), []).unwrap();
        Self(root)
    }
    fn save(&self, file: &str, scene: &Scene) {
        std::fs::write(self.0.join(file), scene.to_source().unwrap()).unwrap();
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        assert!(
            self.0.is_absolute()
                && self.0.starts_with(std::env::temp_dir())
                && self
                    .0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("bevy-hierarchy-")
        );
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn transformed(id: &str, transform: Transform) -> Node {
    let mut node = Node::new(id);
    node.name = Some(id.into());
    node.transform = bevy_editor::scene_format::Transform {
        translation: transform.translation.to_array(),
        rotation: transform.rotation.to_array(),
        scale: transform.scale.to_array(),
    };
    node
}
fn prefab() -> Scene {
    let mut scene = Scene::new("Лампа");
    scene.resources.insert(
        "mesh".into(),
        Resource::external("model", "models/block.glb"),
    );
    let group = transformed(
        "group",
        Transform::from_xyz(0.5, 0.0, 0.0).with_scale(Vec3::new(2.0, 1.0, 0.5)),
    );
    let mut model = transformed(
        "mesh",
        Transform::from_xyz(0.0, 1.0, 0.0).with_rotation(Quat::from_rotation_y(0.6)),
    );
    model.parent = Some("group".into());
    model.model = Some("mesh".into());
    model.components.insert(
        "plugin.Link".into(),
        serde_json::json!({"target":{"$entity":"group"},"ordinary":"group"}),
    );
    scene.entities = vec![group, model];
    scene
}
fn room() -> Scene {
    let mut scene = Scene::new("Комната");
    scene
        .resources
        .insert("lamp".into(), Resource::external("scene", "lamp.scene"));
    let group = transformed(
        "room",
        Transform::from_xyz(3.0, 0.0, -1.0).with_rotation(Quat::from_rotation_y(-0.3)),
    );
    let mut a = transformed("a", Transform::from_xyz(1.0, 0.0, 0.0));
    a.parent = Some("room".into());
    a.instance = Some(Instance {
        resource: "lamp".into(),
        overrides: BTreeMap::new(),
    });
    let mut b = a.clone();
    b.id = "b".into();
    b.name = Some("b".into());
    b.parent = None;
    b.transform.translation = [-3.0, 0.0, 0.0];
    scene.entities = vec![group, a, b];
    scene
}
fn setup() -> (Files, LevelEdits) {
    let files = Files::new();
    files.save("lamp.scene", &prefab());
    files.save("room.scene", &room());
    let edits = LevelEdits::read(&files.0, "room.scene").unwrap();
    (files, edits)
}

#[test]
fn structural_overrides_round_trip_and_follow_their_original_instance() {
    let (files, edits) = setup();
    let original = std::fs::read(files.0.join("lamp.scene")).unwrap();
    let mut graph = (**edits.hierarchy.as_ref().unwrap()).clone();
    let group = graph.by_id("a/group").unwrap().key;
    let other = graph.by_id("b").unwrap().key;
    let world = graph.node(group).unwrap().affine();
    graph.reparent(group, Some(other)).unwrap();
    near(graph.node(group).unwrap().affine(), world);
    graph.move_sibling(group, true).unwrap();
    let siblings: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.entity.parent.as_deref() == Some("b"))
        .map(|n| n.entity.id.as_str())
        .collect();
    assert_eq!(siblings, ["b/group", "a/group"]);
    let edits: LevelEdits = graph.clone().project().unwrap();
    edits.write(&files.0, "room.scene").unwrap();
    let loaded = LevelEdits::<String>::read(&files.0, "room.scene").unwrap();
    let mut graph = (**loaded.hierarchy.as_ref().unwrap()).clone();
    assert_eq!(graph.parent(group).unwrap().entity.id, "b");
    assert_eq!(std::fs::read(files.0.join("lamp.scene")).unwrap(), original);
    let mount = graph.by_id("a").unwrap().key;
    let copy = graph.duplicate(mount).unwrap();
    let copy_id = graph.node(copy).unwrap().entity.id.clone();
    assert_eq!(
        graph
            .by_id(&format!("{copy_id}/group"))
            .unwrap()
            .entity
            .parent
            .as_deref(),
        Some("b")
    );
    let mut unpacked = graph.clone();
    unpacked.unpack(mount).unwrap();
    let moved = unpacked
        .nodes
        .iter()
        .find(|n| !n.inherited() && n.entity.name.as_deref() == Some("group"))
        .unwrap();
    assert_eq!(moved.entity.parent.as_deref(), Some("b"));
    near(moved.affine(), world);
    graph.delete(&[mount]).unwrap();
    assert!(graph.by_id("a/group").is_none());
    assert!(graph.by_id("b/group").is_some());
}

#[test]
fn structural_override_detach_reset_and_delete_preserve_source_ownership() {
    let (_files, edits) = setup();
    let mut graph = (**edits.hierarchy.as_ref().unwrap()).clone();
    let group = graph.by_id("a/group").unwrap().key;
    let mount = graph.by_id("a").unwrap().key;
    let world = graph.node(group).unwrap().affine();
    graph.reparent(group, None).unwrap();
    assert!(graph.parent(group).is_none());
    near(graph.node(group).unwrap().affine(), world);
    graph.reset_overrides(group).unwrap();
    assert_eq!(graph.parent(group).unwrap().key, mount);
    let local = graph.add_group(None).unwrap();
    graph.reparent(group, Some(local)).unwrap();
    graph.delete(&[local]).unwrap();
    assert!(graph.by_id("a").is_some());
    assert!(graph.by_id("a/group").is_none());
    graph.reset_overrides(mount).unwrap();
    assert_eq!(graph.parent(group).unwrap().key, mount);
}

#[test]
fn structural_override_reset_rejects_a_cycle_without_losing_the_override() {
    let (_files, edits) = setup();
    let mut graph = (**edits.hierarchy.as_ref().unwrap()).clone();
    let node = graph.by_id("a/group").unwrap().key;
    let mount = graph.by_id("a").unwrap().key;
    graph.reparent(node, None).unwrap();
    graph.reparent(mount, Some(node)).unwrap();
    let before = graph.clone();
    assert!(graph.reset_overrides(node).is_err());
    assert_eq!(graph, before);
}
fn near(a: Affine3A, b: Affine3A) {
    assert!(
        a.to_cols_array()
            .into_iter()
            .zip(b.to_cols_array())
            .all(|(a, b)| (a - b).abs() < 1e-4),
        "{a:?} != {b:?}"
    );
}

#[test]
fn scene_hierarchy_resolves_camera_less_prefabs_and_exact_parent_affines() {
    let (files, edits) = setup();
    let h = edits.hierarchy.as_ref().unwrap();
    assert_eq!(edits.models.len(), 2);
    assert_eq!(h.nodes.len(), 7);
    assert_eq!(
        edits.scene.as_ref().unwrap().cameras[0].id,
        "__scene_preview"
    );
    let expected = h
        .by_id("room")
        .unwrap()
        .local()
        .transform()
        .compute_affine()
        * h.by_id("a").unwrap().local().transform().compute_affine()
        * h.by_id("a/group")
            .unwrap()
            .local()
            .transform()
            .compute_affine()
        * h.by_id("a/mesh")
            .unwrap()
            .local()
            .transform()
            .compute_affine();
    near(h.by_id("a/mesh").unwrap().affine(), expected);
    assert_eq!(
        h.by_id("a/mesh").unwrap().entity.components["plugin.Link"]["target"]["$entity"],
        "a/group"
    );
    edits.write(&files.0, "room.scene").unwrap();
    let source = Scene::from_slice(&std::fs::read(files.0.join("room.scene")).unwrap()).unwrap();
    assert_eq!(source.entities.len(), 3);
    assert!(
        source
            .entities
            .iter()
            .all(|e| !e.components.contains_key("camera"))
    );
    assert_eq!(
        source.to_source().unwrap(),
        room_with_id(source.id.clone()).to_source().unwrap()
    );
}
fn room_with_id(id: String) -> Scene {
    let mut scene = room();
    scene.id = id;
    scene
}

#[test]
fn scene_hierarchy_inspector_overrides_only_one_instance_and_reloads_losslessly() {
    let (files, mut edits) = setup();
    let original = std::fs::read(files.0.join("lamp.scene")).unwrap();
    let graph = edits.hierarchy.as_ref().unwrap();
    let key = graph.by_id("a/mesh").unwrap().key;
    let other = graph.by_id("b/mesh").unwrap().affine();
    edits
        .models
        .iter_mut()
        .find(|m| m.id == key)
        .unwrap()
        .pose
        .position[0] += 2.0;
    edits.model_names.insert(key, "Красная лампа".into());
    edits.write(&files.0, "room.scene").unwrap();
    assert_eq!(std::fs::read(files.0.join("lamp.scene")).unwrap(), original);
    let reloaded: LevelEdits = LevelEdits::read(&files.0, "room.scene").unwrap();
    let graph = reloaded.hierarchy.as_ref().unwrap();
    near(graph.by_id("b/mesh").unwrap().affine(), other);
    assert_eq!(
        graph.by_id("a/mesh").unwrap().entity.name.as_deref(),
        Some("Красная лампа")
    );
    assert!(graph.overridden(key));
    let mount = graph
        .source()
        .entities
        .iter()
        .find(|e| e.id == "a")
        .unwrap();
    assert!(
        mount
            .instance
            .as_ref()
            .unwrap()
            .overrides
            .contains_key("mesh")
    );
    let mut reset = (**graph).clone();
    reset.reset_overrides(key).unwrap();
    assert!(!reset.overridden(key));
    near(
        reset.by_id("a/mesh").unwrap().affine(),
        edits
            .hierarchy
            .as_ref()
            .unwrap()
            .by_id("a/mesh")
            .unwrap()
            .affine(),
    );
}

#[test]
fn scene_hierarchy_reparent_preserves_world_and_rejects_cycles_and_lossy_shear() {
    let files = Files::new();
    let mut source = Scene::new("Родители");
    source.entities = vec![
        transformed(
            "parent",
            Transform::from_xyz(3.0, 2.0, 1.0).with_rotation(Quat::from_rotation_y(0.3)),
        ),
        transformed("child", Transform::from_xyz(1.0, 0.0, -2.0)),
    ];
    let mut h = SceneHierarchy::load(source, &files.0).unwrap();
    let parent = h.by_id("parent").unwrap().key;
    let child = h.by_id("child").unwrap().key;
    let world = h.node(child).unwrap().affine();
    h.reparent(child, Some(parent)).unwrap();
    near(h.node(child).unwrap().affine(), world);
    assert!(
        h.reparent(parent, Some(child))
            .unwrap_err()
            .to_string()
            .contains("самого себя")
    );
    h.reparent(child, None).unwrap();
    near(h.node(child).unwrap().affine(), world);
    h.set_local(
        parent,
        Transform::from_rotation(Quat::from_rotation_y(0.3))
            .with_scale(Vec3::new(2.0, 1.0, 1.0))
            .into(),
    )
    .unwrap();
    assert!(
        h.reparent(child, Some(parent))
            .unwrap_err()
            .to_string()
            .contains("shear")
    );
    assert!(h.node(child).unwrap().entity.parent.is_none());
}

#[test]
fn scene_hierarchy_local_edits_and_world_translation_preserve_shear() {
    let (_files, edits) = setup();
    let mut h = (**edits.hierarchy.as_ref().unwrap()).clone();
    let key = h.by_id("a/mesh").unwrap().key;
    let original = h.node(key).unwrap().affine();
    let mut pose = h.node(key).unwrap().pose();
    pose.position[0] += 1.0;
    h.set_world(key, pose).unwrap();
    let next = h.node(key).unwrap().affine();
    assert!(original.matrix3.abs_diff_eq(next.matrix3, 1e-5));
    assert!((next.translation.x - original.translation.x - 1.0).abs() < 1e-4);
    let mut local = h.node(key).unwrap().local();
    local.scale = [0.5, 1.0, 2.0];
    h.set_local(key, local).unwrap();
    assert_eq!(h.node(key).unwrap().local(), local);
}

#[test]
fn scene_hierarchy_duplicate_mount_preserves_overrides_and_delete_removes_subtree() {
    let (files, edits) = setup();
    let mut h = (**edits.hierarchy.as_ref().unwrap()).clone();
    let key = h.by_id("a/mesh").unwrap().key;
    h.rename(key, "Переопределённый объект").unwrap();
    let mount = h.by_id("a").unwrap().key;
    let copy = h.duplicate(mount).unwrap();
    let copy_id = h.node(copy).unwrap().entity.id.clone();
    assert_eq!(
        h.by_id(&format!("{copy_id}/mesh"))
            .unwrap()
            .entity
            .name
            .as_deref(),
        Some("Переопределённый объект")
    );
    assert_eq!(
        h.nodes.iter().filter(|n| n.entity.model.is_some()).count(),
        3
    );
    h.delete(&[mount]).unwrap();
    assert!(h.by_id("a").is_none());
    assert!(h.by_id("a/mesh").is_none());
    assert!(h.node(copy).is_some());
    let edits: LevelEdits = h.project().unwrap();
    edits.write(&files.0, "room.scene").unwrap();
    assert_eq!(
        LevelEdits::<String>::read(&files.0, "room.scene")
            .unwrap()
            .models
            .len(),
        2
    );
}

#[test]
fn scene_hierarchy_unpack_keeps_affines_links_and_unknown_components() {
    let (files, edits) = setup();
    let mut h = (**edits.hierarchy.as_ref().unwrap()).clone();
    let mount = h.by_id("a").unwrap().key;
    let expected = h.by_id("a/mesh").unwrap().affine();
    h.unpack(mount).unwrap();
    assert_eq!(
        h.source().resources.len(),
        2,
        "Unpacking one instance must not copy the other instance's resource aliases"
    );
    assert!(h.node(mount).unwrap().instance.is_none());
    assert!(h.by_id("a/mesh").is_none());
    let unpacked = h
        .nodes
        .iter()
        .find(|n| n.entity.model.is_some() && !n.inherited())
        .unwrap();
    near(unpacked.affine(), expected);
    let target = unpacked.entity.components["plugin.Link"]["target"]["$entity"]
        .as_str()
        .unwrap();
    assert!(h.by_id(target).is_some());
    assert_eq!(
        unpacked.entity.components["plugin.Link"]["ordinary"],
        "group"
    );
    let edits: LevelEdits = h.project().unwrap();
    edits.write(&files.0, "room.scene").unwrap();
    let reloaded: LevelEdits = LevelEdits::read(&files.0, "room.scene").unwrap();
    assert_eq!(reloaded.models.len(), 2);
}

#[test]
fn scene_hierarchy_combined_model_camera_light_and_cooked_runtime_keep_components() {
    let files = Files::new();
    let mut source = prefab();
    let light = bevy_editor::LightPreset::Bulb.create(1);
    let mut value = serde_json::to_value(light).unwrap();
    for key in ["id", "name", "pose"] {
        value.as_object_mut().unwrap().remove(key);
    }
    source.entities[1].components.insert("light".into(), value);
    source.entities[1].components.insert(
        "camera".into(),
        serde_json::json!({"id":"main","projection":"perspective","lens":50.0}),
    );
    files.save("lamp.scene", &source);
    files.save("room.scene", &room());
    let edits: LevelEdits = LevelEdits::read(&files.0, "room.scene").unwrap();
    assert_eq!(edits.models.len(), 2);
    assert_eq!(edits.lights.len(), 2);
    assert_eq!(edits.scene.as_ref().unwrap().cameras.len(), 2);
    let ids: Vec<_> = edits
        .scene
        .as_ref()
        .unwrap()
        .cameras
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert!(ids.contains(&"a/main") && ids.contains(&"b/main"));
    let root = room();
    let packed = bevy_editor::scene_format::cook(
        &bevy_editor::scene_format::resolve(&root, &mut |_: &Resource| Ok(source.clone())).unwrap(),
    )
    .unwrap();
    std::fs::write(files.0.join("room.sceneb"), packed).unwrap();
    let cooked: LevelEdits = LevelEdits::read(&files.0, "room.sceneb").unwrap();
    assert_eq!(cooked.models.len(), 2);
    assert_eq!(cooked.lights.len(), 2);
    assert_eq!(cooked.scene.as_ref().unwrap().cameras.len(), 2);
    let mut a = edits
        .hierarchy
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|n| n.entity.model.is_some())
        .map(|n| n.affine())
        .collect::<Vec<_>>();
    let mut b = cooked
        .hierarchy
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|n| n.entity.model.is_some())
        .map(|n| n.affine())
        .collect::<Vec<_>>();
    a.sort_by(|a, b| a.translation.x.total_cmp(&b.translation.x));
    b.sort_by(|a, b| a.translation.x.total_cmp(&b.translation.x));
    for (a, b) in a.into_iter().zip(b) {
        near(a, b);
    }
}

#[test]
fn scene_hierarchy_dependency_refresh_is_explicit_and_snapshots_survive_source_edits() {
    let (files, edits) = setup();
    let old = edits.hierarchy.clone().unwrap();
    let key = old.by_id("a/mesh").unwrap().key;
    let mut updated = prefab();
    updated.entities[1].transform.translation[1] = 4.0;
    files.save("lamp.scene", &updated);
    let mut h = (*old).clone();
    h.refresh(&files.0).unwrap();
    assert_ne!(h.node(key).unwrap().world, old.node(key).unwrap().world);
    let snapshot: LevelEdits = SceneHierarchy::project_shared(old.clone()).unwrap();
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    let restored: LevelEdits = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        restored
            .hierarchy
            .as_ref()
            .unwrap()
            .node(key)
            .unwrap()
            .world,
        old.node(key).unwrap().world
    );
    assert!(Arc::strong_count(&old) >= 2);
}

#[test]
fn scene_hierarchy_cycle_insertion_can_be_rejected_without_touching_files() {
    let (files, edits) = setup();
    let bytes = std::fs::read(files.0.join("room.scene")).unwrap();
    let mut candidate = (**edits.hierarchy.as_ref().unwrap()).clone();
    assert!(
        candidate
            .insert_scene(
                "room.scene",
                "Цикл",
                Pose::from(Transform::IDENTITY),
                &files.0
            )
            .is_err()
    );
    assert_eq!(std::fs::read(files.0.join("room.scene")).unwrap(), bytes);
    assert!(
        edits
            .hierarchy
            .as_ref()
            .unwrap()
            .by_id("instance_1")
            .is_none()
    );
}

#[test]
fn scene_hierarchy_malformed_camera_components_return_errors_without_panicking() {
    let files = Files::new();
    for value in [
        serde_json::json!({"lens":50,"projection":"perspective"}),
        serde_json::json!([]),
        serde_json::json!({"id":"main","lens":"bad","projection":"perspective"}),
        serde_json::json!({"id":"main","lens":500,"projection":"perspective"}),
    ] {
        let mut source = Scene::new("Invalid camera");
        let group = Node::new("group");
        let mut camera = Node::new("camera");
        camera.parent = Some("group".into());
        camera.components.insert("camera".into(), value);
        source.entities = vec![group, camera];
        files.save("invalid.scene", &source);
        assert!(LevelEdits::<String>::read(&files.0, "invalid.scene").is_err());
    }
}

#[test]
fn scene_hierarchy_camera_reset_under_scaled_parent_restores_exact_source() {
    let files = Files::new();
    let mut source = prefab();
    source.entities[1].components.insert(
        "camera".into(),
        serde_json::json!({"id":"main","projection":"perspective","lens":50.0}),
    );
    let mut h = SceneHierarchy::load(source.clone(), &files.0).unwrap();
    let key = h.by_id("mesh").unwrap().key;
    let original = h.node(key).unwrap().local();
    let mut pose = original;
    pose.position[0] += 2.0;
    h.set_camera_local(key, bevy_editor::CameraPlacement { pose, lens: 65.0 })
        .unwrap();
    let edited: LevelEdits = SceneHierarchy::project_shared(Arc::new(h.clone())).unwrap();
    assert_eq!(edited.cameras["main"].lens, 65.0);
    assert!(
        h.node(key).unwrap().entity.components["camera"]
            .get("reset")
            .is_some()
    );
    h.set_camera_local(
        key,
        bevy_editor::CameraPlacement {
            pose: original,
            lens: 50.0,
        },
    )
    .unwrap();
    assert_eq!(h.source(), &source);
}
