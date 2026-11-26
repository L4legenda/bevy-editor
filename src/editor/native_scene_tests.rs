#![cfg(not(target_arch = "wasm32"))]

use super::*;
use std::{fs, path::Path};

fn fixture() -> (CameraSavePath, EditorProject) {
    let scratch = CameraSavePath::new();
    let project = create_project(&scratch.0);
    (scratch, project)
}

pub(super) fn create_project(directory: &Path) -> EditorProject {
    let mut project = crate::project_manager::create_test_project(directory).unwrap();
    let mut document = project.read_scene(0).unwrap();
    let model = document.models[0].clone();
    document.model_names.clear();
    document.models = [-2.0, 0.0, 2.0]
        .into_iter()
        .enumerate()
        .map(|(index, x)| {
            let mut placed = model.clone();
            placed.id = index as u32 + 1;
            placed.pose.position[0] = x;
            placed
        })
        .collect();
    document
        .write(&project.asset_root, &project.scenes[0].file)
        .unwrap();

    let mut second = project.scenes[0].clone();
    second.label = "Second test scene".into();
    second.file = "scenes/second.scene".into();
    document.models = vec![model];
    document.scene.as_mut().unwrap().label = second.label.clone();
    document.write(&project.asset_root, &second.file).unwrap();
    project.scenes.push(second);

    project.asset_root = "assets".into();
    fs::write(
        directory.join(crate::project::PROJECT_FILE_NAME),
        serde_json::to_vec_pretty(&project).unwrap(),
    )
    .unwrap();
    EditorProject::load(directory).unwrap()
}

#[test]
fn a_scene_file_contains_objects_cameras_and_lighting_without_a_source_glb() {
    let (scratch, project) = fixture();
    let definition = &project.scenes[0];
    assert!(definition.is_native());
    assert!(definition.asset.is_empty());
    assert!(!project.asset_root.join("levels").exists());
    assert!(!project.asset_root.join("scenes/main.glb").exists());
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
    document
        .write(&project.asset_root, &definition.file)
        .unwrap();
    assert_eq!(project.read_scene(0).unwrap(), document);
    let mut manifest = project.clone();
    manifest.asset_root = "assets".into();
    fs::write(
        scratch.0.join("project.bevyeditor"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(scratch.0.join("project.bevyeditor")).unwrap()).unwrap();
    assert_eq!(
        json["scenes"][0],
        serde_json::json!({"file":"scenes/main.scene"})
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
    assert!(
        project
            .catalog
            .iter()
            .any(|item| item.asset == "characters/superhero_male.glb")
    );
    assert_eq!(
        browser
            .files
            .iter()
            .filter(|file| file.extension == "scene")
            .count(),
        2
    );
    browser.set_search(".scene".into());
    for entry in browser.entries(&project.catalog) {
        assert!(matches!(entry, browser::Entry::Scene { .. }));
        assert!(browser.model_asset(&project.catalog, entry).is_none());
    }
}

#[test]
fn missing_or_invalid_scene_documents_never_silently_open_as_empty_scenes() {
    let (_scratch, project) = fixture();
    let path = &project.scenes[0].file;
    let original = fs::read(project.asset_root.join(path)).unwrap();
    assert!(
        RoomEdits::<String>::default()
            .write(&project.asset_root, path)
            .is_err()
    );
    assert_eq!(fs::read(project.asset_root.join(path)).unwrap(), original);
    assert!(RoomEdits::<String>::read(&project.asset_root, "scenes/missing.scene").is_err());
    fs::write(project.asset_root.join(path), b"{}").unwrap();
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
