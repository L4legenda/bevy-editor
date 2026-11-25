use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use bevy::prelude::*;
use bevy_editor::{CameraPlacement, LevelEdits, NodeEdit, PlacedModel};
use serde::{Deserialize, Serialize};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "bevy-editor-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(root.join("models/block.glb"), []).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GameCamera {
    Main,
}

#[test]
fn missing_roots_read_as_empty_and_are_created_by_the_first_save() {
    let scratch = Scratch::new();
    let root = scratch.0.join("new-root/nested");
    assert!(!root.exists());
    assert_eq!(
        LevelEdits::<GameCamera>::read(&root, "levels/example.json").unwrap(),
        LevelEdits::default()
    );
    assert!(
        !root.exists(),
        "Reading a missing document must not create files"
    );

    let mut edits = LevelEdits::<GameCamera>::default();
    edits.cameras.insert(
        GameCamera::Main,
        CameraPlacement {
            pose: Transform::IDENTITY.into(),
            lens: 60.0,
        },
    );
    edits.write(&root, "levels/example.json").unwrap();
    assert!(root.join("levels/example.json").is_file());
    assert_eq!(
        LevelEdits::<GameCamera>::read(&root, "levels/example.json").unwrap(),
        edits
    );
}

#[test]
fn saves_replace_documents_and_preserve_typed_runtime_camera_keys() {
    let scratch = Scratch::new();
    let mut edits = LevelEdits::<GameCamera>::default();
    edits.cameras.insert(
        GameCamera::Main,
        CameraPlacement {
            pose: Transform::from_xyz(1.0, 2.0, 3.0).into(),
            lens: 45.0,
        },
    );
    edits.models.push(PlacedModel {
        selector: None,
        uid: None,
        id: 1,
        asset: "models/block.glb".into(),
        pose: Transform::IDENTITY.into(),
        bounds: None,
    });
    edits.write(&scratch.0, "levels/example.json").unwrap();
    edits.models[0].pose.position[0] = 2.0;
    edits.write(&scratch.0, "levels/example.json").unwrap();
    assert_eq!(
        LevelEdits::<GameCamera>::read(&scratch.0, "levels/example.json").unwrap(),
        edits
    );
    let editor = LevelEdits::<String>::read(&scratch.0, "levels/example.json").unwrap();
    assert_eq!(editor.cameras["main"], edits.cameras[&GameCamera::Main]);
}

#[test]
fn invalid_documents_do_not_replace_the_last_saved_level() {
    let scratch = Scratch::new();
    let mut edits = LevelEdits::<String>::default();
    edits.write(&scratch.0, "levels/example.json").unwrap();
    let saved = std::fs::read(scratch.0.join("levels/example.json")).unwrap();
    for path in [
        "../outside.glb",
        "/outside.glb",
        "C:/outside.glb",
        "models\\block.glb",
        "models/block.glb#Scene0",
    ] {
        edits.models = vec![PlacedModel {
            selector: None,
            uid: None,
            id: 1,
            asset: path.into(),
            pose: Transform::IDENTITY.into(),
            bounds: None,
        }];
        assert!(
            edits.write(&scratch.0, "levels/example.json").is_err(),
            "{path}"
        );
    }
    edits.models[0].asset = "models/block.glb".into();
    edits.models.push(edits.models[0].clone());
    assert!(edits.write(&scratch.0, "levels/example.json").is_err());
    assert_eq!(
        std::fs::read(scratch.0.join("levels/example.json")).unwrap(),
        saved
    );
    assert!(
        LevelEdits::<String>::default()
            .write(&scratch.0, "../outside.json")
            .is_err()
    );
}

#[test]
fn edited_anchor_follows_translation_and_rotation() {
    let mut edits = LevelEdits::<String>::default();
    edits.nodes.insert(
        "Seat".into(),
        NodeEdit {
            original: Transform::from_xyz(1.0, 0.0, 0.0).into(),
            pose: Transform::from_xyz(3.0, 0.0, 4.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
                .into(),
        },
    );
    assert!(
        edits
            .anchor("Seat", Vec3::new(1.0, 0.0, 1.0))
            .abs_diff_eq(Vec3::new(4.0, 0.0, 4.0), 1e-5)
    );
}

fn native_document() -> LevelEdits {
    let mut edits = LevelEdits::default();
    edits.scene = Some(bevy_editor::SceneSettings {
        label: "Кабинет 🏥".into(),
        cameras: vec![bevy_editor::data::CameraDefinition {
            id: "main".into(),
            label: "Главная".into(),
            projection: bevy_editor::data::LensKind::Perspective,
            placement: CameraPlacement {
                pose: Transform::from_xyz(1.0, 2.0, 3.0).into(),
                lens: 45.0,
            },
        }],
    });
    edits.models.push(PlacedModel {
        selector: None,
        uid: None,
        id: 7,
        asset: "models/block.glb".into(),
        pose: Transform::IDENTITY.into(),
        bounds: None,
    });
    edits.model_names.insert(7, "Модель".into());
    edits.lights.push(bevy_editor::SceneLight::default());
    edits.lighting = Some(bevy_editor::SceneLighting::default());
    edits.cameras.insert(
        "main".into(),
        CameraPlacement {
            pose: Transform::from_xyz(9.0, 3.0, 2.0).into(),
            lens: 65.0,
        },
    );
    edits
}

#[test]
fn native_saves_use_the_versioned_format_and_preserve_uuid_and_editor_semantics() {
    let scratch = Scratch::new();
    let edits = native_document();
    edits.write(&scratch.0, "main.scene").unwrap();
    let first = std::fs::read(scratch.0.join("main.scene")).unwrap();
    let source = bevy_editor::scene_format::Scene::from_slice(&first).unwrap();
    assert_eq!(source.version, 1);
    assert_eq!(source.resources.len(), 1);
    assert_eq!(source.entities.len(), 3);
    assert_eq!(
        LevelEdits::<String>::read(&scratch.0, "main.scene").unwrap(),
        edits
    );
    edits.write(&scratch.0, "main.scene").unwrap();
    assert_eq!(std::fs::read(scratch.0.join("main.scene")).unwrap(), first);
}

#[test]
fn native_edits_preserve_plugin_components_settings_and_non_render_entities() {
    let scratch = Scratch::new();
    let mut edits = native_document();
    edits.write(&scratch.0, "main.scene").unwrap();
    let mut source = bevy_editor::scene_format::Scene::from_slice(
        &std::fs::read(scratch.0.join("main.scene")).unwrap(),
    )
    .unwrap();
    source
        .entities
        .iter_mut()
        .find(|entity| entity.model.is_some())
        .unwrap()
        .components
        .insert(
            "plugin.Future".into(),
            serde_json::json!({"value": u64::MAX}),
        );
    source.settings.insert(
        "plugin.Settings".into(),
        serde_json::json!({"value": "keep"}),
    );
    source
        .entities
        .push(bevy_editor::scene_format::Entity::new("unknown"));
    std::fs::write(scratch.0.join("main.scene"), source.to_source().unwrap()).unwrap();
    edits.models[0].pose.position[0] = 4.0;
    edits.write(&scratch.0, "main.scene").unwrap();
    let saved = bevy_editor::scene_format::Scene::from_slice(
        &std::fs::read(scratch.0.join("main.scene")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.id, source.id);
    assert!(saved.entities.iter().any(|entity| entity.id == "unknown"));
    assert_eq!(
        saved
            .entities
            .iter()
            .find(|entity| entity.model.is_some())
            .unwrap()
            .components["plugin.Future"]["value"],
        u64::MAX
    );
    assert_eq!(
        saved.settings["plugin.Settings"],
        source.settings["plugin.Settings"]
    );
    let loaded = LevelEdits::<String>::read(&scratch.0, "main.scene").unwrap();
    assert_eq!(loaded.models[0].pose, edits.models[0].pose);
    assert_eq!(loaded.models[0].asset, edits.models[0].asset);
    assert!(
        loaded
            .hierarchy
            .as_ref()
            .unwrap()
            .by_id("unknown")
            .is_some()
    );
}

#[test]
fn old_native_documents_migrate_on_save_without_modifying_their_source_on_read() {
    let scratch = Scratch::new();
    let edits = native_document();
    let old = serde_json::to_vec_pretty(&edits).unwrap();
    std::fs::write(scratch.0.join("main.scene"), &old).unwrap();
    assert_eq!(
        LevelEdits::<String>::read(&scratch.0, "main.scene").unwrap(),
        edits
    );
    assert_eq!(std::fs::read(scratch.0.join("main.scene")).unwrap(), old);
    edits.write(&scratch.0, "main.scene").unwrap();
    assert!(
        bevy_editor::scene_format::Scene::from_slice(
            &std::fs::read(scratch.0.join("main.scene")).unwrap()
        )
        .is_ok()
    );
}

#[test]
fn raw_and_compressed_runtime_scenes_decode_without_the_editor_feature() {
    let scratch = Scratch::new();
    let edits = native_document();
    edits.write(&scratch.0, "main.scene").unwrap();
    let source = bevy_editor::scene_format::Scene::from_slice(
        &std::fs::read(scratch.0.join("main.scene")).unwrap(),
    )
    .unwrap();
    let resolved = bevy_editor::scene_format::resolve(
        &source,
        &mut |_: &bevy_editor::scene_format::Resource| unreachable!(),
    )
    .unwrap();
    let raw = bevy_editor::scene_format::cook(&resolved).unwrap();
    let compressed = bevy_editor::scene_format::compress(&raw).unwrap();
    for bytes in [&raw, &compressed] {
        std::fs::write(scratch.0.join("main.sceneb"), bytes).unwrap();
        assert_eq!(
            LevelEdits::<String>::read(&scratch.0, "main.sceneb").unwrap(),
            edits
        );
    }
    assert!(edits.write(&scratch.0, "main.sceneb").is_err());
}

#[test]
fn future_versions_and_stale_flat_edits_never_overwrite_hierarchical_sources() {
    let scratch = Scratch::new();
    let edits = native_document();
    edits.write(&scratch.0, "main.scene").unwrap();
    let mut source = bevy_editor::scene_format::Scene::from_slice(
        &std::fs::read(scratch.0.join("main.scene")).unwrap(),
    )
    .unwrap();
    source.entities[1].parent = Some(source.entities[0].id.clone());
    let hierarchy = source.to_source().unwrap();
    std::fs::write(scratch.0.join("main.scene"), &hierarchy).unwrap();
    assert!(
        LevelEdits::<String>::read(&scratch.0, "main.scene")
            .unwrap()
            .hierarchy
            .is_some()
    );
    assert!(edits.write(&scratch.0, "main.scene").is_err());
    assert_eq!(
        std::fs::read(scratch.0.join("main.scene")).unwrap(),
        hierarchy
    );
    source.version = 999;
    let future = serde_json::to_vec(&source).unwrap();
    std::fs::write(scratch.0.join("main.scene"), &future).unwrap();
    assert!(edits.write(&scratch.0, "main.scene").is_err());
    assert_eq!(std::fs::read(scratch.0.join("main.scene")).unwrap(), future);
}

#[test]
fn oversized_source_is_rejected_before_reading_its_contents() {
    let scratch = Scratch::new();
    let file = std::fs::File::create(scratch.0.join("huge.scene")).unwrap();
    file.set_len(bevy_editor::scene_format::MAX_SOURCE_BYTES as u64 + 1)
        .unwrap();
    assert!(
        LevelEdits::<String>::read(&scratch.0, "huge.scene")
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
}

#[test]
fn native_discriminant_accepts_reordered_keys_and_bom_and_rejects_trailing_data() {
    let scratch = Scratch::new();
    let edits = native_document();
    edits.write(&scratch.0, "main.scene").unwrap();
    let canonical = std::fs::read(scratch.0.join("main.scene")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let reordered = serde_json::to_vec_pretty(&value).unwrap();
    assert!(!reordered.starts_with(b"{\n  \"format\""));
    for bytes in [
        reordered,
        [vec![0xef, 0xbb, 0xbf], canonical.clone()].concat(),
    ] {
        std::fs::write(scratch.0.join("main.scene"), bytes).unwrap();
        assert_eq!(
            LevelEdits::<String>::read(&scratch.0, "main.scene").unwrap(),
            edits
        );
    }
    let mut trailing = canonical;
    trailing.extend_from_slice(b"{}");
    std::fs::write(scratch.0.join("main.scene"), trailing).unwrap();
    assert!(LevelEdits::<String>::read(&scratch.0, "main.scene").is_err());
}
