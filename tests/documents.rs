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
