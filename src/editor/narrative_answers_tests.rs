//! Render real intermediate frames and exercise interrupted row transitions.
use super::*;

fn choices(app: &App) -> Vec<Choice> {
    let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    else {
        panic!()
    };
    d.node(1).unwrap().choices.clone()
}
fn visual_height(app: &App) -> f32 {
    let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    else {
        panic!()
    };
    app.world()
        .resource::<effects::State>()
        .answers
        .height(d.node(1).unwrap())
}
fn history(app: &App) -> usize {
    app.world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len()
}
fn socket_aligned(app: &mut App, index: usize) {
    let field = control_point(app, &format!("Ответ {} ноды #1", index + 1));
    let port = control_point(app, &format!("Связать ответ {} ноды #1", index + 1));
    assert!(
        (field.y - port.y).abs() < 1.1,
        "Port detached from animated row: {field:?} {port:?}"
    );
}
fn freeze(app: &mut App) {
    *app.world_mut()
        .resource_mut::<bevy::time::TimeUpdateStrategy>() =
        bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO);
    warm(app, 2);
}
fn resume(app: &mut App) {
    *app.world_mut()
        .resource_mut::<bevy::time::TimeUpdateStrategy>() =
        bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(
            1. / 60.,
        ));
}

#[test]
#[ignore = "GPU answer add/remove animations, geometry and editing focus"]
fn gpu_narrative_answer_transitions() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/narrative-qa")
        .join(std::env::var("NARRATIVE_QA_PASS").unwrap_or("answers-pass-1".into()));
    let lab = output.join(format!("answers-project-{}", std::process::id()));
    fn copy_assets(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for item in std::fs::read_dir(from).unwrap() {
            let item = item.unwrap();
            let path = to.join(item.file_name());
            if item.file_type().unwrap().is_dir() {
                copy_assets(&item.path(), &path);
            } else {
                std::fs::copy(item.path(), path).unwrap();
            }
        }
    }
    let mut project = crate::project::EditorProject::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_assets(&project.asset_root, &lab);
    project.asset_root = lab;
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    action(&mut app, NarrativeAction::Create(false));
    action(&mut app, NarrativeAction::Add(NodeKind::Line));
    action(&mut app, NarrativeAction::Add(NodeKind::Line));
    action(&mut app, NarrativeAction::Select(1));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let doc = editor.narrative.current_mut().unwrap();
        doc.zoom = 0.9;
        doc.pan = Vec2::new(20., 25.);
        let Content::Dialogue(d) = &mut doc.content else {
            panic!()
        };
        d.node_mut(0).unwrap().position = [16., 20.];
        d.node_mut(1).unwrap().position = [430., 25.];
        d.node_mut(2).unwrap().position = [880., 100.];
    }
    warm(&mut app, 40);
    let original_height = visual_height(&app);
    let original_history = history(&app);
    action(&mut app, NarrativeAction::AddChoice);
    assert_eq!(choices(&app).len(), 2);
    assert!(
        visual_height(&app) > original_height
            && visual_height(&app) < original_height + OUTPUT_HEIGHT
    );
    freeze(&mut app);
    socket_aligned(&mut app, 1);
    capture_when(
        &mut app,
        &target,
        &output.join("01-answer-opening.png"),
        |_| true,
    );
    // Finishing an animation cannot rebuild the graph or steal an active text input.
    let input = edit_named(&mut app, "Ответ 2 ноды #1", "Проверить источник сигнала");
    resume(&mut app);
    warm(&mut app, 30);
    assert!(app.world().get::<EditableText>(input).is_some());
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    assert_eq!(choices(&app)[1].text, "Проверить источник сигнала");
    assert_eq!(history(&app), original_history + 2); // Addition plus the actual text edit.
    action(&mut app, NarrativeAction::Connect(1, 1, 2));
    action(&mut app, NarrativeAction::AddChoice);
    warm(&mut app, 25);
    capture_when(
        &mut app,
        &target,
        &output.join("02-three-answers.png"),
        |_| true,
    );
    let before_remove = choices(&app);
    let height = visual_height(&app);
    action(&mut app, NarrativeAction::RemoveChoice(0));
    assert_eq!(choices(&app).len(), 2);
    assert_eq!(choices(&app)[0].target, Some(2));
    assert!(visual_height(&app) > height - OUTPUT_HEIGHT && visual_height(&app) < height);
    freeze(&mut app);
    let world = app.world_mut();
    assert_eq!(
        world.query::<&effects::Visual>().iter(world).count(),
        1,
        "Only the removed branch dissolves; surviving branches keep their identity"
    );
    socket_aligned(&mut app, 0);
    socket_aligned(&mut app, 1);
    capture_when(
        &mut app,
        &target,
        &output.join("03-answer-closing.png"),
        |_| true,
    );
    // The disappearing row is read only: exactly two live answer fields/outputs remain.
    let world = app.world_mut();
    let count = world
        .query::<(&DraftField, &nodes::Owner)>()
        .iter(world)
        .filter(|(field, owner)| owner.0 == 1 && matches!(field, DraftField::Choice(_)))
        .count();
    assert_eq!(count, 2);
    resume(&mut app);
    action(&mut app, NarrativeAction::Undo);
    assert_eq!(choices(&app), before_remove);
    action(&mut app, NarrativeAction::Redo);
    assert_eq!(choices(&app).len(), 2);
    // Interruptions are handled from the currently visible height.
    for _ in 0..4 {
        action(&mut app, NarrativeAction::AddChoice);
        let last = choices(&app).len() - 1;
        action(&mut app, NarrativeAction::RemoveChoice(last));
    }
    let saved = choices(&app);
    let expected_history = history(&app);
    let input = edit_named(&mut app, "Ответ 1 ноды #1", "Вернуться на корабль");
    warm(&mut app, 45);
    assert_eq!(choices(&app).len(), saved.len());
    assert_eq!(choices(&app)[0].target, Some(2));
    assert_eq!(choices(&app)[0].text, "Вернуться на корабль");
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    assert_eq!(history(&app), expected_history + 1);
    assert!(!app.world().resource::<effects::State>().answers.active());
    socket_aligned(&mut app, 0);
    socket_aligned(&mut app, 1);
    capture_when(&mut app, &target, &output.join("04-settled.png"), |_| true);
    // Use the actual remove button too, then restore it through history.
    click(&mut app, window, "Удалить ответ 2 ноды #1");
    assert_eq!(choices(&app).len(), 1);
    action(&mut app, NarrativeAction::Undo);
    warm(&mut app, 25);
    assert_eq!(choices(&app).len(), 2);
    action(&mut app, NarrativeAction::RemoveChoice(1));
    action(&mut app, NarrativeAction::RemoveChoice(0));
    assert!(choices(&app).is_empty());
    action(&mut app, NarrativeAction::AddChoice);
    warm(&mut app, 25);
    assert_eq!(choices(&app).len(), 1);
    socket_aligned(&mut app, 0);
    action(&mut app, NarrativeAction::Create(false));
    assert!(!app.world().resource::<effects::State>().answers.active());
}
