use super::*;
use crate::{CameraPlacement, EditorProject, LevelEdits};

fn editor() -> Editor {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
    )
    .unwrap();
    let edits = project.read_scene(0).unwrap();
    Editor::new(project, edits)
}

#[test]
fn play_snapshot_uses_the_open_level_and_unsaved_objects_camera_and_lighting() {
    let mut editor = editor();
    editor.room = 1;
    editor.edits = editor.project.read_scene(1).unwrap();
    let saved = fs::read(editor.root.join(&editor.definition().file)).unwrap();
    editor.edits.models[0].pose.position = [13.0, 2.0, -7.0];
    let id = editor.definition().cameras[0].id.clone();
    editor.edits.cameras.insert(
        id.clone(),
        CameraPlacement {
            pose: Transform::from_xyz(7.0, 6.0, 9.0).into(),
            lens: 67.0,
        },
    );
    editor.edits.lighting = Some(crate::LightingPreset::Night.settings());
    let launch = snapshot(&editor);
    launch.level.validate(&launch.asset_root).unwrap();
    assert_eq!(launch.level.edits, editor.edits);
    assert_eq!(launch.level.camera, id);
    assert!(launch.level.title.contains(&editor.definition().label));
    assert_eq!(
        fs::read(editor.root.join(&editor.definition().file)).unwrap(),
        saved
    );
}

#[test]
fn play_snapshot_roundtrips_and_only_removes_its_own_temporary_files() {
    let launch = snapshot(&editor());
    let (files, log) = write_snapshot(&launch).unwrap();
    let loaded = GameLaunch::read(&files.snapshot).unwrap();
    assert_eq!(loaded.level.edits, launch.level.edits);
    assert_eq!(loaded.asset_root, launch.asset_root);
    let path = files.snapshot.clone();
    let log_path = files.log.clone();
    drop(log);
    drop(files);
    assert!(!path.exists());
    assert!(!log_path.exists());
}

#[test]
fn play_rejects_missing_camera_and_invalid_unsaved_values() {
    let mut launch = snapshot(&editor());
    launch.level.camera = "missing".into();
    assert!(launch.level.validate(&launch.asset_root).is_err());
    launch.level.camera = launch.level.cameras[0].id.clone();
    launch.level.edits.models[0].pose.scale[0] = f32::NAN;
    assert!(write_snapshot(&launch).is_err());
}

#[test]
fn legacy_play_snapshot_keeps_source_groups_and_node_deletions() {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, LevelEdits::default());
    editor.edits.deleted_nodes.insert("Chair".into());
    let launch = snapshot(&editor);
    assert_eq!(launch.level.asset, editor.definition().asset);
    assert_eq!(launch.level.groups, editor.definition().groups);
    assert!(launch.level.edits.deleted_nodes.contains("Chair"));
    launch.level.validate(&launch.asset_root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn play_stop_reaps_the_owned_process() {
    use std::os::windows::process::CommandExt;
    let mut session = Session::default();
    session.child = Some(
        Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Threading.Thread]::Sleep(30000)",
            ])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    assert!(session.running());
    session.stop().unwrap();
    assert!(!session.running());
    session.stop().unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn play_custom_command_receives_current_level_and_snapshot_and_is_reaped() {
    let mut editor = editor();
    editor.room = 1;
    editor.edits = editor.project.read_scene(1).unwrap();
    editor.edits.models[0].pose.position[0] = 12.0;
    editor.project.scenes[1].play = Some(crate::PlayCommand {
        program: Some("powershell.exe".into()),
        args: vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "[Console]::WriteLine($env:BEVY_EDITOR_LEVEL); [Console]::WriteLine($env:BEVY_EDITOR_PLAY_SNAPSHOT); [Console]::WriteLine($env:BEVY_EDITOR_ASSET_ROOT)".into()],
        working_directory: Some(editor.root.clone()),
    });
    start(&mut editor).unwrap();
    let pid = editor.play.child.as_ref().unwrap().id();
    start(&mut editor).unwrap();
    assert_eq!(editor.play.child.as_ref().unwrap().id(), pid);
    assert!(
        editor
            .play
            .child
            .as_mut()
            .unwrap()
            .wait()
            .unwrap()
            .success()
    );
    let files = editor.play.files.as_ref().unwrap();
    let launch = GameLaunch::read(&files.snapshot).unwrap();
    assert_eq!(launch.level.edits, editor.edits);
    let log = fs::read_to_string(&files.log).unwrap();
    assert!(log.contains(&editor.definition().file));
    assert!(log.contains(&files.snapshot.to_string_lossy().to_string()));
    assert!(log.contains(&editor.root.to_string_lossy().to_string()));
    let snapshot = files.snapshot.clone();
    let mut app = App::new();
    app.insert_resource(editor).add_systems(Update, poll);
    app.update();
    assert!(!app.world().resource::<Editor>().play.running());
    assert!(app.world().resource::<Editor>().play.error.is_none());
    assert!(!snapshot.exists());
}

#[cfg(target_os = "windows")]
#[test]
fn play_process_failure_becomes_visible_feedback_and_allows_retry() {
    let mut editor = editor();
    editor.project.scenes[0].play = Some(crate::PlayCommand {
        program: Some("powershell.exe".into()),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "[Console]::Error.WriteLine('Game fixture failed'); exit 9".into(),
        ],
        working_directory: None,
    });
    start(&mut editor).unwrap();
    editor.play.child.as_mut().unwrap().wait().unwrap();
    let mut app = App::new();
    app.insert_resource(editor).add_systems(Update, poll);
    app.update();
    let mut editor = app.world_mut().resource_mut::<Editor>();
    assert!(!editor.play.running());
    assert!(
        editor
            .play
            .error
            .as_ref()
            .unwrap()
            .contains("Game fixture failed")
    );
    assert!(editor.play.files.is_none());
    editor.project.scenes[0].play.as_mut().unwrap().program =
        Some("missing-bevy-game-fixture.exe".into());
    assert!(start(&mut editor).is_err());
    assert!(!editor.play.running());
}

#[test]
#[ignore = "requires GPU; writes target/play-qa/editor-button.png"]
fn gpu_play_button_is_visible_without_an_external_command() {
    use crate::editor::selection_outline::render_tests::{capture_when, frame};
    use bevy::ui::InteractionDisabled;
    let (mut app, _, target) = crate::editor::tests::lighting_ui_render::fixture();
    for _ in 0..4 {
        frame(&mut app);
    }
    let world = app.world_mut();
    let (_, node, disabled) = world
        .query::<(&AccessibleLabel, &Node, Has<InteractionDisabled>)>()
        .iter(world)
        .find(|(label, _, _)| label.0 == "Запустить открытый уровень (F5)")
        .unwrap();
    assert_ne!(node.display, Display::None);
    assert!(!disabled);
    assert!(world.resource::<Editor>().definition().play.is_none());
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/play-qa/editor-button.png");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    capture_when(&mut app, &target, &output, |_| true);
}
