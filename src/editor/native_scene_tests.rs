use super::*;
use std::{fs, path::Path};

fn fixture() -> (CameraSavePath, EditorProject) {
    let scratch = CameraSavePath::new();
    let mut project =
        EditorProject::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"))
            .unwrap();
    fs::create_dir_all(scratch.0.join("models")).unwrap();
    for model in &project.catalog {
        fs::copy(
            project.asset_root.join(&model.asset),
            scratch.0.join(&model.asset),
        )
        .unwrap();
    }
    for index in 0..project.scenes.len() {
        let document = project.read_scene(index).unwrap();
        document
            .write(&scratch.0, &project.scenes[index].file)
            .unwrap();
    }
    project.asset_root = scratch.0.clone();
    (scratch, project)
}

#[test]
fn a_scene_file_contains_objects_cameras_and_lighting_without_a_source_glb() {
    let (scratch, project) = fixture();
    let definition = &project.scenes[0];
    assert!(definition.is_native());
    assert!(definition.asset.is_empty());
    assert!(!scratch.0.join("levels").exists());
    assert!(!scratch.0.join("scenes/workshop.glb").exists());
    let mut document = project.read_scene(0).unwrap();
    assert_eq!(document.models.len(), 3);
    assert_eq!(document.scene.as_ref().unwrap().cameras, definition.cameras);
    document.models[0].pose.position = [3.0, 2.0, -4.0];
    document.lights.push(crate::LightPreset::Bulb.create(7));
    document.lighting = Some(crate::LightingPreset::Studio.settings());
    document.cameras.insert(
        "main".into(),
        CameraPlacement {
            pose: Transform::from_xyz(7.0, 3.0, 4.0).into(),
            lens: 65.0,
        },
    );
    document.write(&scratch.0, &definition.file).unwrap();
    assert_eq!(project.read_scene(0).unwrap(), document);
    let mut manifest = project.clone();
    manifest.asset_root = ".".into();
    fs::write(
        scratch.0.join("project.bevyeditor"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(scratch.0.join("project.bevyeditor")).unwrap()).unwrap();
    assert_eq!(
        json["scenes"][0],
        serde_json::json!({"file":"scenes/workshop.scene"})
    );
    let reopened = EditorProject::load(&scratch.0).unwrap();
    assert_eq!(reopened.read_scene(0).unwrap(), document);
}

#[test]
fn browser_discovers_unlisted_scene_files_and_treats_every_glb_as_a_model() {
    let (_scratch, mut project) = fixture();
    project.scenes.clear();
    project.catalog.clear();
    let mut browser = browser::Browser::new(&mut project);
    assert_eq!(project.scenes.len(), 2);
    assert!(project.scenes.iter().all(SceneDefinition::is_native));
    assert_eq!(project.catalog.len(), 2);
    assert!(browser.files.iter().all(|file| file.extension == "scene"));
    browser.set_search(".scene".into());
    for entry in browser.entries(&project.catalog) {
        assert!(matches!(entry, browser::Entry::Scene { .. }));
        assert!(browser.model_asset(&project.catalog, entry).is_none());
    }
}

#[test]
fn missing_or_invalid_scene_documents_never_silently_open_as_empty_scenes() {
    let (scratch, project) = fixture();
    let path = &project.scenes[0].file;
    let original = fs::read(scratch.0.join(path)).unwrap();
    assert!(
        RoomEdits::<String>::default()
            .write(&scratch.0, path)
            .is_err()
    );
    assert_eq!(fs::read(scratch.0.join(path)).unwrap(), original);
    assert!(RoomEdits::<String>::read(&scratch.0, "scenes/missing.scene").is_err());
    fs::write(scratch.0.join(path), b"{}").unwrap();
    assert!(project.read_scene(0).is_err());
}

#[test]
fn placing_a_glb_with_embedded_cameras_cannot_activate_them() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let root = app
        .world_mut()
        .spawn_empty()
        .observe(prepare_placed_model)
        .id();
    let embedded = app
        .world_mut()
        .spawn((Camera::default(), ChildOf(root)))
        .id();
    let editor_camera = app.world_mut().spawn(Camera::default()).id();
    let mut assets = Assets::<WorldAsset>::default();
    let handle = assets.add(WorldAsset::new(World::new()));
    app.insert_resource(assets);
    let instance_id = WorldInstanceSpawner::default()
        .spawn_sync(app.world_mut(), handle.id())
        .unwrap();
    app.world_mut().trigger(WorldInstanceReady {
        entity: root,
        instance_id,
    });
    app.update();
    assert!(!app.world().get::<Camera>(embedded).unwrap().is_active);
    assert!(app.world().get::<Camera>(editor_camera).unwrap().is_active);
}
