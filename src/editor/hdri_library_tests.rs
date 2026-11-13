//! End-to-end coverage of catalog browsing, downloads and raw HDR/EXR rendering.
#![cfg(not(target_arch = "wasm32"))]

use super::*;
use crate::editor::{
    lighting::{EnvironmentField, LightingAction},
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
    viewport::EditorCamera,
};
use crate::project::EditorProject;
use bevy::{
    input::ButtonState,
    light::{EnvironmentMapLight, Skybox},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn until(app: &mut App, description: &str, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !ready(app) {
        assert!(
            Instant::now() < deadline,
            "{description}: {} / {}",
            app.world().resource::<Editor>().status,
            app.world().resource::<HdriLibrary>().notice
        );
        frame(app);
    }
}

fn send(app: &mut App, action: HdriAction) {
    app.world_mut().write_message(Action::HdriLibrary(action));
    frame(app);
    frame(app);
}

fn click(app: &mut App, window: Entity, accept: impl Fn(Action) -> bool) {
    let point = {
        let world = app.world_mut();
        let overlay = world
            .resource::<Editor>()
            .hdri_library_open
            .then_some(world.resource::<LibraryUi>().overlay);
        let mut query = world.query::<(
            Entity,
            &Control,
            &ComputedNode,
            &UiGlobalTransform,
            Option<&InteractionDisabled>,
        )>();
        let (_, _, _, transform, disabled) = query
            .iter(world)
            .find(|(entity, control, node, _, _)| {
                if !accept(control.action) || node.size().min_element() <= 0.0 {
                    return false;
                }
                let Some(overlay) = overlay else {
                    return true;
                };
                let mut parent = *entity;
                while let Some(relation) = world.get::<ChildOf>(parent) {
                    parent = relation.parent();
                    if parent == overlay {
                        return true;
                    }
                }
                false
            })
            .expect("Expected a visible HDRI control");
        assert!(disabled.is_none(), "HDRI control is unexpectedly disabled");
        transform.affine().translation
    };
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap().flatten() {
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

fn search(app: &mut App, query: &str) {
    let entity = app.world().resource::<LibraryUi>().search;
    app.world_mut()
        .get_mut::<EditableText>(entity)
        .unwrap()
        .editor
        .set_text(query);
    send(app, HdriAction::Search);
}

fn ready_environment(app: &App, path: &str) -> bool {
    app.world()
        .resource::<crate::hdri::HdriLoadStatus>()
        .0
        .get(path)
        == Some(&Ok(true))
}

#[test]
#[ignore = "requires HTTPS access and a GPU; downloads actual Poly Haven HDR and ambientCG EXR panoramas"]
fn gpu_hdri_catalog_download_apply_save_and_reopen() {
    let directory = std::env::temp_dir().join(format!("bevy-hdri-ui-{}", std::process::id()));
    let mut project = EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_tree(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let saved_file = project.scenes[0].file.clone();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    for _ in 0..4 {
        frame(&mut app);
    }
    click(&mut app, window, |action| {
        matches!(action, Action::HdriLibrary(HdriAction::Open))
    });
    assert!(app.world().resource::<Editor>().hdri_library_open);
    let original = app.world().resource::<Editor>().edits.clone();
    // Catalog browsing and blocked scene commands must never author scene changes.
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetEnvironment(Some(
            crate::EnvironmentPreset::Forest,
        ))));
    frame(&mut app);
    assert_eq!(app.world().resource::<Editor>().edits, original);
    until(&mut app, "Poly Haven catalog", |app| {
        app.world()
            .resource::<HdriLibrary>()
            .catalogs
            .contains_key(&HdriProvider::PolyHaven)
    });
    until(&mut app, "Poly Haven previews", |app| {
        app.world().resource::<HdriLibrary>().previews.len() >= 6
    });
    capture_when(
        &mut app,
        &target,
        &directory.join("01-polyhaven.png"),
        |_| true,
    );
    search(&mut app, "studio_small_09");
    send(&mut app, HdriAction::Resolution(1024));
    assert_eq!(app.world().resource::<HdriLibrary>().rows().count(), 1);
    click(&mut app, window, |action| {
        matches!(action, Action::HdriLibrary(HdriAction::Download(..)))
    });
    // Closing the catalog leaves the download running. Reopening must not
    // race a directory scan against the download's library update.
    send(&mut app, HdriAction::Close);
    send(&mut app, HdriAction::Open);
    if app.world().resource::<HdriLibrary>().download.is_some() {
        assert!(app.world().resource::<HdriLibrary>().scan.is_none());
    }
    until(&mut app, "Poly Haven download", |app| {
        app.world().resource::<HdriLibrary>().download.is_none()
    });
    assert_eq!(app.world().resource::<HdriLibrary>().available.len(), 1);
    assert_eq!(app.world().resource::<Editor>().edits, original);
    capture_when(
        &mut app,
        &target,
        &directory.join("02-downloaded.png"),
        |_| true,
    );
    let before = app.world().resource::<Editor>().undo.len();
    click(&mut app, window, |action| {
        matches!(action, Action::UseDownloadedHdri(_))
    });
    assert!(!app.world().resource::<Editor>().hdri_library_open);
    let poly = app
        .world()
        .resource::<Editor>()
        .environment()
        .hdri_asset
        .clone()
        .unwrap();
    assert_eq!(app.world().resource::<Editor>().undo.len(), before + 1);
    until(&mut app, "HDR preparation", |app| {
        ready_environment(app, &poly.asset)
    });
    for _ in 0..12 {
        frame(&mut app);
    }
    {
        let world = app.world_mut();
        let mut camera =
            world.query_filtered::<(&EnvironmentMapLight, &Skybox), With<EditorCamera>>();
        let (light, sky) = camera.single(world).unwrap();
        assert_ne!(light.diffuse_map, light.specular_map);
        assert_ne!(sky.image.as_ref(), Some(&light.specular_map));
    }
    capture_when(
        &mut app,
        &target,
        &directory.join("03-polyhaven-scene.png"),
        |_| true,
    );
    app.world_mut().write_message(Action::Save);
    for _ in 0..4 {
        frame(&mut app);
    }
    let saved = crate::LevelEdits::<String>::read(&directory, &saved_file).unwrap();
    assert_eq!(saved.lighting.unwrap().hdri_asset.as_ref(), Some(&poly));
    assert!(!app.world().resource::<Editor>().dirty());
    // Exercise the second provider's ZIP -> EXR path through the same visible UI.
    send(&mut app, HdriAction::Open);
    send(&mut app, HdriAction::Provider(HdriProvider::AmbientCg));
    search(&mut app, "DaySkyHDRI072B");
    until(&mut app, "ambientCG catalog", |app| {
        app.world()
            .resource::<HdriLibrary>()
            .catalogs
            .contains_key(&HdriProvider::AmbientCg)
    });
    assert_eq!(app.world().resource::<HdriLibrary>().rows().count(), 1);
    click(&mut app, window, |action| {
        matches!(action, Action::HdriLibrary(HdriAction::Download(..)))
    });
    until(&mut app, "ambientCG download", |app| {
        app.world().resource::<HdriLibrary>().download.is_none()
    });
    assert_eq!(app.world().resource::<HdriLibrary>().available.len(), 2);
    capture_when(
        &mut app,
        &target,
        &directory.join("04-ambientcg.png"),
        |_| true,
    );
    click(&mut app, window, |action| {
        matches!(action, Action::UseDownloadedHdri(_))
    });
    let ambient = app
        .world()
        .resource::<Editor>()
        .environment()
        .hdri_asset
        .clone()
        .unwrap();
    assert_eq!(ambient.provider, HdriProvider::AmbientCg);
    assert!(ambient.asset.ends_with(".exr"));
    until(&mut app, "EXR preparation", |app| {
        ready_environment(app, &ambient.asset)
    });
    for _ in 0..12 {
        frame(&mut app);
    }
    capture_when(
        &mut app,
        &target,
        &directory.join("05-ambientcg-scene.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetNumber(
            Field::Environment(EnvironmentField::HdriRotation),
            90.0,
        )));
    frame(&mut app);
    app.world_mut().write_message(Action::Undo);
    frame(&mut app);
    app.world_mut().write_message(Action::Undo);
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_asset
            .as_ref(),
        Some(&poly)
    );
    assert!(!app.world().resource::<Editor>().dirty());
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetEnvironment(Some(
            crate::EnvironmentPreset::Forest,
        ))));
    frame(&mut app);
    assert!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_asset
            .is_none()
    );
    app.world_mut().write_message(Action::Undo);
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_asset
            .as_ref(),
        Some(&poly)
    );
    send(&mut app, HdriAction::Open);
    search(&mut app, "");
    send(&mut app, HdriAction::Downloaded);
    until(&mut app, "Downloaded library", |app| {
        app.world().resource::<HdriLibrary>().scan.is_none()
    });
    assert_eq!(app.world().resource::<HdriLibrary>().rows().count(), 2);
    capture_when(&mut app, &target, &directory.join("06-library.png"), |_| {
        true
    });
    assert_eq!(hdri_catalog::scan(&directory).unwrap().len(), 2);
    send(&mut app, HdriAction::Close);
    app.world_mut().write_message(Action::Open(1));
    frame(&mut app);
    until(&mut app, "Second scene", |app| {
        app.world().resource::<Editor>().ready
    });
    app.world_mut().write_message(Action::Open(0));
    frame(&mut app);
    until(&mut app, "Saved scene reopened", |app| {
        app.world().resource::<Editor>().ready && ready_environment(app, &poly.asset)
    });
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_asset
            .as_ref(),
        Some(&poly)
    );
    println!(
        "HDRI catalog screenshots and portable project: {}",
        directory.display()
    );
}
