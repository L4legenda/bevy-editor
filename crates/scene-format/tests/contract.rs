use bevy_scene_format::*;
use serde_json::json;
use std::collections::BTreeMap;

fn flat() -> Scene {
    let mut scene = Scene::new("Кабинет 🏥");
    scene.resources.insert(
        "chair".into(),
        Resource::external("model", "models/chair.glb"),
    );
    let mut entity = Entity::new("chair_1");
    entity.model = Some("chair".into());
    entity.name = Some("Стул 猫".into());
    scene.entities.push(entity);
    scene
}
fn resolved(scene: &Scene) -> ResolvedScene {
    resolve(scene, &mut |_: &Resource| {
        Err(Error("Unexpected file access".into()))
    })
    .unwrap()
}
fn instance(base: &Scene) -> (Scene, BTreeMap<String, Scene>) {
    let mut scene = Scene::new("Level");
    scene.resources.insert(
        "prefab".into(),
        Resource::external("scene", "prefabs/chair.scene"),
    );
    let mut mount = Entity::new("mount");
    mount.instance = Some(Instance {
        resource: "prefab".into(),
        overrides: BTreeMap::new(),
    });
    scene.entities.push(mount);
    (
        scene,
        BTreeMap::from([("prefabs/chair.scene".into(), base.clone())]),
    )
}

#[test]
fn unicode_unknown_components_and_large_integers_survive_source_and_binary() {
    let mut scene = flat();
    scene.entities[0].components.insert(
        "plugin.Future".into(),
        json!({"value": u64::MAX, "text": "строка\n猫", "null": null, "values": [true, -1.25, 0]}),
    );
    let source = scene.to_source().unwrap();
    assert_eq!(Scene::from_slice(&source).unwrap(), scene);
    let resolved = resolved(&scene);
    let bytes = cook(&resolved).unwrap();
    let view = PackedScene::open(&bytes, Some(&resolved.fingerprint().unwrap())).unwrap();
    let restored = view.unpack().unwrap();
    assert_eq!(restored.entities, resolved.entities);
    assert_eq!(restored.resources, resolved.resources);
    assert_eq!(
        view.entity(0).unwrap().components().unwrap()["plugin.Future"],
        scene.entities[0].components["plugin.Future"]
    );
    assert!(view.entity(usize::MAX).is_none());
    assert!(view.resource(usize::MAX).is_none());
}

#[test]
fn canonical_source_and_binary_ignore_insertion_order_and_keep_diff_local() {
    let mut scene = flat();
    scene.entities[0].transform.translation[0] = 0.5;
    scene.entities.extend([Entity::new("z"), Entity::new("a")]);
    let source = scene.to_source().unwrap();
    let binary = cook(&resolved(&scene)).unwrap();
    scene.entities.reverse();
    assert_eq!(source, scene.to_source().unwrap());
    assert_eq!(binary, cook(&resolved(&scene)).unwrap());
    let mut parsed = Scene::from_slice(&source).unwrap();
    parsed
        .entities
        .iter_mut()
        .find(|e| e.id == "chair_1")
        .unwrap()
        .transform
        .translation[0] = 1.25;
    let edited = parsed.to_source().unwrap();
    let first = std::str::from_utf8(&source).unwrap();
    let second = std::str::from_utf8(&edited).unwrap();
    assert_eq!(
        first
            .lines()
            .zip(second.lines())
            .filter(|(a, b)| a != b)
            .count(),
        1
    );
    assert_eq!(
        Scene::from_slice(&edited).unwrap().to_source().unwrap(),
        edited
    );
}

#[test]
fn root_and_sibling_order_are_explicit_and_parents_precede_children() {
    let mut scene = Scene::new("Hierarchy");
    let mut child = Entity::new("a_child");
    child.parent = Some("z_parent".into());
    let mut first = Entity::new("zz_first");
    first.order = -1;
    scene.entities = vec![child, Entity::new("z_parent"), first];
    let bytes = cook(&resolved(&scene)).unwrap();
    let view = PackedScene::open(&bytes, None).unwrap();
    assert_eq!(view.entity(0).unwrap().id(), "zz_first");
    assert_eq!(view.entity(2).unwrap().parent(), Some(1));
}

#[test]
fn twenty_thousand_level_hierarchy_is_iterative() {
    let mut scene = Scene::new("Deep");
    for i in 0..20_000 {
        let mut entity = Entity::new(format!("n{i}"));
        entity.parent = (i > 0).then(|| format!("n{}", i - 1));
        scene.entities.push(entity);
    }
    let bytes = cook(&resolved(&scene)).unwrap();
    assert_eq!(
        PackedScene::open(&bytes, None).unwrap().entity_count(),
        20_000
    );
}

#[test]
fn invalid_hierarchies_ids_transforms_and_asset_paths_are_rejected() {
    for path in [
        "../model.glb",
        "/model.glb",
        "C:/model.glb",
        "a\\model.glb",
        "a//model.glb",
        "a/./model.glb",
        "a/../model.glb",
        "model.glb#Scene0",
    ] {
        assert!(!asset_path(path), "{path}");
    }
    assert!(asset_path("模型/стул.glb"));
    let mut scene = flat();
    scene.entities.push(scene.entities[0].clone());
    assert!(scene.validate().is_err());
    scene.entities.pop();
    scene.entities[0].parent = Some("missing".into());
    assert!(scene.validate().is_err());
    scene.entities[0].parent = Some("chair_1".into());
    assert!(scene.validate().is_err());
    scene.entities[0].parent = Some("other".into());
    let mut other = Entity::new("other");
    other.parent = Some("chair_1".into());
    scene.entities.push(other);
    assert!(scene.validate().is_err());
    for transform in [
        Transform {
            translation: [f32::NAN; 3],
            ..Default::default()
        },
        Transform {
            rotation: [0.0; 4],
            ..Default::default()
        },
        Transform {
            scale: [0.0; 3],
            ..Default::default()
        },
    ] {
        assert!(transform.validate().is_err());
    }
    assert!(
        Transform {
            scale: [-1.0, 1.0, 1.0],
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn duplicate_map_keys_future_versions_and_reserved_reference_shapes_fail_loudly() {
    let scene = flat();
    let source = String::from_utf8(scene.to_source().unwrap()).unwrap();
    let duplicate = source.replace(
        "\"chair\":{",
        "\"chair\":{\"kind\":\"model\",\"path\":\"models/other.glb\"},\"chair\":{",
    );
    assert!(
        Scene::from_slice(duplicate.as_bytes())
            .unwrap_err()
            .0
            .contains("Duplicate")
    );
    assert!(
        Scene::from_slice(
            source
                .replace("\"version\":1", "\"version\":999")
                .as_bytes()
        )
        .unwrap_err()
        .0
        .contains("version")
    );
    let mut invalid = scene;
    invalid.entities[0]
        .components
        .insert("test".into(), json!({"$entity": "missing"}));
    assert!(invalid.validate().is_err());
    invalid.entities[0]
        .components
        .insert("test".into(), json!({"$entity": "chair_1", "extra": true}));
    assert!(invalid.validate().is_err());
}

#[test]
fn repeated_instances_load_once_and_scope_internal_references() {
    let mut base = flat();
    base.entities[0].components.insert(
        "Link".into(),
        json!({"target": {"$entity": "chair_1"}, "material": {"$resource": "chair"}}),
    );
    let (mut scene, files) = instance(&base);
    let mut second = scene.entities[0].clone();
    second.id = "second".into();
    scene.entities.push(second);
    let mut loads = 0;
    let output = resolve(&scene, &mut |r: &Resource| {
        loads += 1;
        Ok(files[r.path.as_ref().unwrap()].clone())
    })
    .unwrap();
    assert_eq!(loads, 1);
    assert_eq!(output.entities.len(), 4);
    let child = output
        .entities
        .iter()
        .find(|e| e.id == "second/chair_1")
        .unwrap();
    assert_eq!(child.parent.as_deref(), Some("second"));
    assert_eq!(
        child.components["Link"]["target"]["$entity"],
        "second/chair_1"
    );
    assert_eq!(
        child.components["Link"]["material"]["$resource"],
        "second/chair"
    );
}

#[test]
fn shared_inline_resources_keep_one_origin_uid_across_instances() {
    let mut base = flat();
    base.resources.insert(
        "material".into(),
        Resource {
            kind: "material".into(),
            path: None,
            uid: None,
            selector: None,
            data: Some(json!({"color": [1, 0, 0]})),
            sharing: Sharing::Shared,
        },
    );
    base.entities[0]
        .components
        .insert("Material".into(), json!({"$resource": "material"}));
    let (mut scene, files) = instance(&base);
    let mut second = scene.entities[0].clone();
    second.id = "second".into();
    scene.entities.push(second);
    let output = resolve(&scene, &mut |r: &Resource| {
        Ok(files[r.path.as_ref().unwrap()].clone())
    })
    .unwrap();
    assert_eq!(
        output.resources["mount/material"].uid,
        output.resources["second/material"].uid
    );
    assert!(output.resources["mount/material"].uid.is_some());
}

#[test]
fn resolution_discards_unused_resources_but_keeps_transitive_typed_dependencies() {
    let mut scene = flat();
    scene
        .resources
        .insert("unused".into(), Resource::external("model", "unused.glb"));
    scene.resources.insert(
        "unused_scene".into(),
        Resource::external("scene", "not-loaded.scene"),
    );
    scene.resources.insert(
        "texture".into(),
        Resource::external("texture", "textures/wood.png"),
    );
    scene.resources.insert(
        "material".into(),
        Resource {
            kind: "material".into(),
            path: None,
            uid: None,
            selector: None,
            data: Some(json!({"texture": {"$resource": "texture"}})),
            sharing: Sharing::Shared,
        },
    );
    scene.entities[0]
        .components
        .insert("Material".into(), json!({"$resource": "material"}));
    let output = resolved(&scene);
    assert_eq!(output.resources.len(), 3);
    assert!(
        output.resources.contains_key("chair")
            && output.resources.contains_key("material")
            && output.resources.contains_key("texture")
    );
    assert_eq!(scene.resources.len(), 5);
}

#[test]
fn overrides_keep_stable_ids_and_source_immutable_and_track_transitive_changes() {
    let mut base = flat();
    base.entities[0].components.insert("Power".into(), json!(1));
    let original = base.clone();
    let (mut scene, mut files) = instance(&base);
    scene.entities[0]
        .instance
        .as_mut()
        .unwrap()
        .overrides
        .insert(
            "chair_1".into(),
            EntityPatch {
                name: Some("Новое имя".into()),
                components: BTreeMap::from([
                    ("Power".into(), json!(3)),
                    ("Nullable".into(), Value::Null),
                ]),
                ..Default::default()
            },
        );
    let first = resolve(&scene, &mut |r: &Resource| {
        Ok(files[r.path.as_ref().unwrap()].clone())
    })
    .unwrap();
    assert_eq!(files["prefabs/chair.scene"], original);
    base.entities[0].name = Some("Renamed source".into());
    base.entities[0].components.insert("Power".into(), json!(7));
    files.insert("prefabs/chair.scene".into(), base);
    let second = resolve(&scene, &mut |r: &Resource| {
        Ok(files[r.path.as_ref().unwrap()].clone())
    })
    .unwrap();
    let child = second
        .entities
        .iter()
        .find(|e| e.id == "mount/chair_1")
        .unwrap();
    assert_eq!(child.name.as_deref(), Some("Новое имя"));
    assert_eq!(child.components["Power"], 3);
    assert!(child.components.contains_key("Nullable"));
    assert_ne!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
    let bytes = cook(&first).unwrap();
    assert!(PackedScene::open(&bytes, Some(&second.fingerprint().unwrap())).is_err());
}

use serde_json::Value;

#[test]
fn stale_overrides_scene_cycles_and_expansion_bombs_are_rejected() {
    let base = flat();
    let (mut scene, files) = instance(&base);
    scene.entities[0]
        .instance
        .as_mut()
        .unwrap()
        .overrides
        .insert("missing".into(), EntityPatch::default());
    assert!(
        resolve(&scene, &mut |r: &Resource| Ok(files
            [r.path.as_ref().unwrap()]
        .clone()))
        .unwrap_err()
        .0
        .contains("no longer exists")
    );
    scene.entities[0]
        .instance
        .as_mut()
        .unwrap()
        .overrides
        .clear();
    let cycle = scene.clone();
    assert!(
        resolve(&scene, &mut |_: &Resource| Ok(cycle.clone()))
            .unwrap_err()
            .0
            .contains("Cyclic scene")
    );
    let limits = ResolveLimits {
        entities: 1,
        ..Default::default()
    };
    assert!(
        resolve_with_limits(
            &scene,
            &mut |r: &Resource| Ok(files[r.path.as_ref().unwrap()].clone()),
            limits
        )
        .is_err()
    );
}

#[test]
fn every_truncation_and_single_byte_corruption_is_rejected_without_panics() {
    let bytes = cook(&resolved(&flat())).unwrap();
    for end in 0..bytes.len() {
        assert!(
            PackedScene::open(&bytes[..end], None).is_err(),
            "truncation {end}"
        );
    }
    for i in 0..bytes.len() {
        let mut corrupted = bytes.clone();
        corrupted[i] ^= 0x80;
        assert!(PackedScene::open(&corrupted, None).is_err(), "byte {i}");
    }
}

#[test]
fn forged_checksum_cannot_hide_invalid_offsets_indices_or_non_finite_transforms() {
    let bytes = cook(&resolved(&flat())).unwrap();
    for (offset, value) in [
        (128, u32::MAX),
        (128 + 16, 0),
        (128 + 20, 99),
        (128 + 32, f32::INFINITY.to_bits()),
        (104, 0),
        (120, u32::MAX),
    ] {
        let mut forged = bytes.clone();
        forged[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        let mut hash = blake3::Hasher::new();
        hash.update(&forged[..56]);
        hash.update(&forged[88..]);
        forged[56..88].copy_from_slice(hash.finalize().as_bytes());
        assert!(PackedScene::open(&forged, None).is_err(), "offset {offset}");
    }
}

#[test]
fn removing_a_subtree_is_explicit_and_dangling_references_are_rejected() {
    let mut base = flat();
    let mut child = Entity::new("child");
    child.parent = Some("chair_1".into());
    base.entities.push(child);
    let (mut scene, files) = instance(&base);
    scene.entities[0]
        .instance
        .as_mut()
        .unwrap()
        .overrides
        .insert(
            "chair_1".into(),
            EntityPatch {
                remove: true,
                ..Default::default()
            },
        );
    let output = resolve(&scene, &mut |r: &Resource| {
        Ok(files[r.path.as_ref().unwrap()].clone())
    })
    .unwrap();
    assert_eq!(output.entities.len(), 1);
    scene.entities[0]
        .components
        .insert("Link".into(), json!({"$entity": "mount/child"}));
    assert!(
        resolve(&scene, &mut |r: &Resource| Ok(files
            [r.path.as_ref().unwrap()]
        .clone()))
        .is_err()
    );
}

#[test]
fn compression_is_lossless_and_size_bombs_are_rejected() {
    let raw = cook(&resolved(&flat())).unwrap();
    let compressed = compress(&raw).unwrap();
    assert_eq!(decompress(&compressed).unwrap().as_ref(), raw);
    assert!(matches!(
        decompress(&raw).unwrap(),
        std::borrow::Cow::Borrowed(_)
    ));
    for end in 0..compressed.len() {
        assert!(decompress(&compressed[..end]).is_err(), "truncation {end}");
    }
    let mut bomb = compressed.clone();
    bomb[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decompress(&bomb).is_err());
    let mut trailing = compressed.clone();
    trailing.push(0);
    assert!(decompress(&trailing).is_err());
}

#[test]
fn resource_sharing_is_explicit_and_identical_component_blobs_are_interned() {
    let mut scene = flat();
    scene.resources.get_mut("chair").unwrap().sharing = Sharing::Local;
    scene.entities[0].components.insert(
        "Data".into(),
        json!({"marker": "unique blob that should appear once"}),
    );
    for i in 0..100 {
        let mut entity = scene.entities[0].clone();
        entity.id = format!("copy_{i}");
        scene.entities.push(entity);
    }
    let bytes = cook(&resolved(&scene)).unwrap();
    let marker = b"unique blob that should appear once";
    assert_eq!(
        bytes
            .windows(marker.len())
            .filter(|window| *window == marker)
            .count(),
        1
    );
    let view = PackedScene::open(&bytes, None).unwrap();
    assert_eq!(
        view.resource(0).unwrap().decode().unwrap().sharing,
        Sharing::Local
    );
    let mut first = view.entity(0).unwrap().components().unwrap();
    first.insert("Data".into(), json!(false));
    assert!(view.entity(1).unwrap().components().unwrap()["Data"].is_object());
}

#[test]
fn expanded_payload_budget_and_recursive_instance_depth_are_bounded() {
    let base = flat();
    let (scene, files) = instance(&base);
    for limits in [
        ResolveLimits {
            expanded_bytes: 1,
            ..Default::default()
        },
        ResolveLimits {
            depth: 1,
            ..Default::default()
        },
        ResolveLimits {
            resources: 1,
            ..Default::default()
        },
    ] {
        assert!(
            resolve_with_limits(
                &scene,
                &mut |r: &Resource| Ok(files[r.path.as_ref().unwrap()].clone()),
                limits
            )
            .is_err()
        );
    }
}

#[test]
fn component_removal_and_null_assignment_are_distinct_and_conflicts_fail() {
    let mut base = flat();
    base.entities[0].components.insert("A".into(), json!(1));
    base.entities[0].components.insert("B".into(), json!(2));
    let (mut scene, files) = instance(&base);
    let patch = EntityPatch {
        remove_components: vec!["A".into()],
        components: BTreeMap::from([("B".into(), Value::Null)]),
        ..Default::default()
    };
    scene.entities[0]
        .instance
        .as_mut()
        .unwrap()
        .overrides
        .insert("chair_1".into(), patch);
    let output = resolve(&scene, &mut |r: &Resource| {
        Ok(files[r.path.as_ref().unwrap()].clone())
    })
    .unwrap();
    let child = output
        .entities
        .iter()
        .find(|e| e.id == "mount/chair_1")
        .unwrap();
    assert!(!child.components.contains_key("A"));
    assert_eq!(child.components["B"], Value::Null);
    scene.entities[0]
        .instance
        .as_mut()
        .unwrap()
        .overrides
        .get_mut("chair_1")
        .unwrap()
        .components
        .insert("A".into(), json!(3));
    assert!(scene.validate().is_err());
}

#[test]
fn nested_duplicate_keys_are_rejected_and_inline_null_is_preserved() {
    let mut scene = flat();
    scene.entities[0]
        .components
        .insert("X".into(), json!({"nested": {"marker": 1}}));
    scene.resources.insert(
        "null".into(),
        Resource {
            kind: "data".into(),
            path: None,
            uid: None,
            selector: None,
            data: Some(Value::Null),
            sharing: Sharing::Shared,
        },
    );
    let source = scene.to_source().unwrap();
    assert_eq!(Scene::from_slice(&source).unwrap(), scene);
    let duplicate = String::from_utf8(source)
        .unwrap()
        .replace("\"marker\":1", "\"marker\":1,\"marker\":2");
    assert!(
        Scene::from_slice(duplicate.as_bytes())
            .unwrap_err()
            .0
            .contains("Duplicate nested")
    );
}

#[test]
fn random_malformed_inputs_never_panic() {
    let mut state = 0x123456789abcdef0u64;
    for sample in 0..5000 {
        let length = sample % 513;
        let bytes: Vec<_> = (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        assert!(
            std::panic::catch_unwind(|| {
                let _ = Scene::from_slice(&bytes);
                let _ = PackedScene::open(&bytes, None);
                let _ = decompress(&bytes);
            })
            .is_ok()
        );
    }
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("scene-format-{}", Scene::new("scratch").id));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn source_documents_reject_external_edits_and_leave_no_temporary_files() {
    let directory = Scratch::new();
    let path = directory.0.join("test.scene");
    let scene = flat();
    atomic_write(&path, &scene.to_source().unwrap(), Revision::Missing).unwrap();
    let mut first = SourceDocument::open(&path).unwrap();
    let mut second = SourceDocument::open(&path).unwrap();
    first.scene.name = "First edit".into();
    first.save().unwrap();
    second.scene.name = "Stale edit".into();
    assert!(second.save().unwrap_err().0.contains("changed on disk"));
    assert_eq!(
        SourceDocument::open(&path).unwrap().scene.name,
        "First edit"
    );
    assert!(std::fs::read_dir(&directory.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn simultaneous_writers_cannot_overwrite_each_other_with_the_same_revision() {
    let directory = Scratch::new();
    let path = directory.0.join("test.scene");
    let bytes = flat().to_source().unwrap();
    let revision = atomic_write(&path, &bytes, Revision::Missing).unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let gate = gate.clone();
                let path = path.clone();
                let mut scene = flat();
                scene.name = format!("Writer {i}");
                scope.spawn(move || {
                    gate.wait();
                    atomic_write(&path, &scene.to_source().unwrap(), revision).is_ok()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|ok| *ok)
            .count()
    });
    assert_eq!(results, 1);
    assert!(
        SourceDocument::open(&path)
            .unwrap()
            .scene
            .name
            .starts_with("Writer ")
    );
}
