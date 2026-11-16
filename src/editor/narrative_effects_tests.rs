//! Real GPU frames for every graph transition; only a scratch project is modified.
use super::*;

#[test]
#[ignore = "GPU narrative transitions and shader screenshots"]
fn gpu_narrative_effects() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/narrative-qa")
        .join(std::env::var("NARRATIVE_QA_PASS").unwrap_or("effects-pass-1".into()));
    let lab = output.join(format!("effects-project-{}", std::process::id()));
    std::fs::create_dir_all(&lab).unwrap();
    let mut project = crate::project::EditorProject::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    fn copy_assets(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let destination = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_assets(&entry.path(), &destination);
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    copy_assets(&project.asset_root, &lab);
    project.asset_root = lab;
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    action(&mut app, NarrativeAction::Create(false));
    capture_when(
        &mut app,
        &target,
        &output.join("01-dialogue-birth.png"),
        |_| true,
    );
    warm(&mut app, 40);
    action(&mut app, NarrativeAction::Add(NodeKind::Line));
    capture_when(&mut app, &target, &output.join("02-node-birth.png"), |_| {
        true
    });
    warm(&mut app, 45);
    action(&mut app, NarrativeAction::Fit);
    let history = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len();
    let content = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
        .clone();
    warm(&mut app, 50);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        content
    );
    assert_eq!(
        app.world_mut()
            .query::<&effects::Visual>()
            .iter(app.world())
            .count(),
        0
    );

    action(&mut app, NarrativeAction::Select(0));
    action(&mut app, NarrativeAction::Disconnect(0));
    assert_eq!(branch_target(&app, 0, 0), None);
    assert!(
        app.world_mut()
            .query::<&effects::Visual>()
            .iter(app.world())
            .count()
            > 0
    );
    capture_when(&mut app, &target, &output.join("03-link-break.png"), |_| {
        true
    });
    warm(&mut app, 40);
    action(&mut app, NarrativeAction::Connect(0, 0, 1));
    assert_eq!(branch_target(&app, 0, 0), Some(1));
    capture_when(
        &mut app,
        &target,
        &output.join("04-link-attach.png"),
        |_| true,
    );
    warm(&mut app, 45);
    // A cancel is visually transient and cannot disconnect an existing branch.
    begin_connection(&mut app, window, 0, 0);
    move_pointer(&mut app, window, Vec2::new(600., 600.));
    capture_when(&mut app, &target, &output.join("05-live-link.png"), |_| {
        true
    });
    key(
        &mut app,
        window,
        KeyCode::Escape,
        bevy::input::keyboard::Key::Escape,
        None,
    );
    assert_eq!(branch_target(&app, 0, 0), Some(1));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 4);
    action(&mut app, NarrativeAction::Select(1));
    action(&mut app, NarrativeAction::Delete);
    assert_eq!(branch_target(&app, 0, 0), None);
    capture_when(
        &mut app,
        &target,
        &output.join("06-node-dissolve.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(branch_target(&app, 0, 0), Some(1));
    assert!(
        app.world_mut()
            .query::<&effects::Visual>()
            .iter(app.world())
            .count()
            <= 1,
        "Undo must cancel outgoing deletion ghosts"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("07-undo-restore.png"),
        |_| true,
    );
    warm(&mut app, 60);
    assert_eq!(
        app.world_mut()
            .query::<&effects::Visual>()
            .iter(app.world())
            .count(),
        0
    );
    capture_when(&mut app, &target, &output.join("08-settled.png"), |_| true);
    // Moving to another document clears every transient from the previous graph.
    action(&mut app, NarrativeAction::Select(1));
    action(&mut app, NarrativeAction::Delete);
    action(&mut app, NarrativeAction::Create(false));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .selected,
        0
    );
}
