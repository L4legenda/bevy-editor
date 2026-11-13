use super::documents::{lens, ready, workspace};
use super::*;

fn dirty_scene() -> (App, CameraSavePath) {
    let (mut app, scratch) = workspace();
    lens(&mut app, 60.0);
    assert!(app.world().resource::<Editor>().dirty());
    (app, scratch)
}

fn two_dirty_scenes() -> (App, CameraSavePath) {
    let (mut app, scratch) = dirty_scene();
    action(&mut app, Action::Open(1));
    ready(&mut app);
    lens(&mut app, 80.0);
    (app, scratch)
}

fn close_requests(app: &mut App) -> Entity {
    app.add_message::<WindowCloseRequested>()
        .add_message::<AppExit>()
        .add_systems(Update, close_editor.after(handle_actions));
    app.world_mut().spawn_empty().id()
}

fn request_close(app: &mut App, window: Entity) {
    app.world_mut()
        .write_message(WindowCloseRequested { window });
    app.update();
}

#[test]
fn cancelling_tab_close_preserves_edits_and_blocks_background_actions() {
    let (mut app, scratch) = dirty_scene();
    let tool = app.world().resource::<Editor>().tool;
    let edits = app.world().resource::<Editor>().edits.clone();
    let undo = app.world().resource::<Editor>().undo.clone();
    action(&mut app, Action::CloseTab(0));
    assert_eq!(
        app.world().resource::<Editor>().pending,
        Some(PendingAction::CloseTab(0))
    );
    for blocked in [
        Action::Number(Field::Lens, 75.0),
        Action::Undo,
        Action::Save,
        Action::Tool(Tool::Rotate),
        Action::Open(1),
    ] {
        action(&mut app, blocked);
    }
    {
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.room, 0);
        assert_eq!(editor.open_tabs, [0]);
        assert_eq!(editor.edits, edits);
        assert_eq!(editor.undo, undo);
        assert_eq!(editor.tool, tool);
        assert!(!scratch.0.join(&editor.definition().file).exists());
    }
    action(&mut app, Action::CancelPending);
    let editor = app.world().resource::<Editor>();
    assert!(editor.pending.is_none());
    assert_eq!(editor.room, 0);
    assert_eq!(editor.edits, edits);
    assert_eq!(editor.undo, undo);
    assert!(!editor.exit_requested);
    action(&mut app, Action::Number(Field::Lens, 65.0));
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 65.0);
}

#[test]
fn discarding_an_inactive_tab_keeps_the_other_open_document_and_its_changes() {
    let (mut app, scratch) = two_dirty_scenes();
    let active_edits = app.world().resource::<Editor>().edits.clone();
    let active_history = app.world().resource::<Editor>().undo.clone();
    let closed_file = app.world().resource::<Editor>().project.scenes[0]
        .file
        .clone();

    action(&mut app, Action::CloseTab(0));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.room, 0, "The closing document is shown for review");
    assert_eq!(editor.pending, Some(PendingAction::CloseTab(0)));
    assert_eq!(editor.camera(0).lens, 60.0);
    action(&mut app, Action::DiscardAndContinue);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.open_tabs, [1]);
    assert_eq!(editor.room, 1);
    assert_eq!(editor.edits, active_edits);
    assert_eq!(editor.undo, active_history);
    assert!(editor.dirty());
    assert!(editor.pending.is_none());
    assert!(!scratch.0.join(closed_file).exists());

    action(&mut app, Action::Open(0));
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 45.0);
    assert!(!app.world().resource::<Editor>().dirty());
    assert!(app.world().resource::<Editor>().document_dirty(1));
}

#[test]
fn saving_an_inactive_tab_writes_that_document_and_preserves_the_other_draft() {
    let (mut app, scratch) = two_dirty_scenes();
    let editor = app.world().resource::<Editor>();
    let other_edits = editor.edits.clone();
    let current_file = editor.project.scenes[0].file.clone();
    let other_file = editor.project.scenes[1].file.clone();
    action(&mut app, Action::CloseTab(0));
    let closing_edits = app.world().resource::<Editor>().edits.clone();
    ready(&mut app);
    action(&mut app, Action::SaveAndContinue);
    let written: RoomEdits = RoomEdits::read(&scratch.0, &current_file).unwrap();
    assert_eq!(written, closing_edits);
    assert!(!scratch.0.join(other_file).exists());
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.open_tabs, [1]);
    assert_eq!(editor.room, 1);
    assert_eq!(editor.edits, other_edits);
    assert!(editor.dirty());
    assert!(editor.pending.is_none());

    action(&mut app, Action::Open(0));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).lens, 60.0);
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty());
    assert!(editor.document_dirty(1));
}

#[test]
fn cancelling_an_inactive_tab_close_keeps_both_documents_open() {
    let (mut app, _scratch) = two_dirty_scenes();
    action(&mut app, Action::CloseTab(0));
    action(&mut app, Action::CancelPending);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.open_tabs, [0, 1]);
    assert_eq!(editor.room, 0);
    assert!(editor.pending.is_none());
    assert!(editor.document_dirty(0));
    assert!(editor.document_dirty(1));
    assert_eq!(editor.camera(0).lens, 60.0);
    action(&mut app, Action::Open(1));
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 80.0);
}

#[test]
fn failed_save_keeps_the_confirmation_and_all_unsaved_changes() {
    let (mut app, scratch) = dirty_scene();
    let editor = app.world().resource::<Editor>();
    let edits = editor.edits.clone();
    let undo = editor.undo.clone();
    let parent = scratch
        .0
        .join(&editor.definition().file)
        .parent()
        .unwrap()
        .to_owned();
    // A file where a directory is required reliably fails even with elevated permissions.
    std::fs::write(parent, b"not a directory").unwrap();
    action(&mut app, Action::CloseTab(0));
    action(&mut app, Action::SaveAndContinue);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.pending, Some(PendingAction::CloseTab(0)));
    assert_eq!(editor.open_tabs, [0]);
    assert_eq!(editor.room, 0);
    assert_eq!(editor.edits, edits);
    assert_eq!(editor.undo, undo);
    assert!(editor.dirty());
    assert!(!editor.exit_requested);
    assert!(!editor.reload);
    assert!(editor.status.starts_with("Не удалось сохранить"));
}

#[test]
fn repeated_close_never_discards_changes_and_cancel_allows_save_before_exit() {
    let (mut app, scratch) = dirty_scene();
    let window = close_requests(&mut app);
    let edits = app.world().resource::<Editor>().edits.clone();
    let file = app.world().resource::<Editor>().definition().file.clone();
    for _ in 0..2 {
        request_close(&mut app, window);
        assert_eq!(
            app.world().resource::<Editor>().pending,
            Some(PendingAction::Close)
        );
        assert_eq!(app.world().resource::<Editor>().edits, edits);
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
    }
    action(&mut app, Action::CancelPending);
    assert!(app.world().resource::<Editor>().pending.is_none());
    assert!(app.world().resource::<Editor>().dirty());
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());

    request_close(&mut app, window);
    ready(&mut app);
    action(&mut app, Action::SaveAndContinue);
    let written: RoomEdits = RoomEdits::read(&scratch.0, &file).unwrap();
    assert_eq!(written, edits);
    assert!(!app.world().resource::<Editor>().dirty());
    let exits: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<AppExit>>()
        .drain()
        .collect();
    assert_eq!(exits, vec![AppExit::Success]);
}

#[test]
fn cancelling_app_close_after_skipping_one_tab_keeps_both_unsaved_drafts() {
    let (mut app, scratch) = two_dirty_scenes();
    let window = close_requests(&mut app);
    request_close(&mut app, window);
    let first = app.world().resource::<Editor>().room;
    action(&mut app, Action::DiscardAndContinue);
    let editor = app.world().resource::<Editor>();
    assert_ne!(editor.room, first);
    assert_eq!(editor.pending, Some(PendingAction::Close));
    assert!(editor.document_dirty(0));
    assert!(editor.document_dirty(1));
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());

    action(&mut app, Action::CancelPending);
    assert!(app.world().resource::<Editor>().pending.is_none());
    assert_eq!(app.world().resource::<Editor>().open_tabs, [0, 1]);
    assert!(app.world().resource::<Editor>().document_dirty(0));
    assert!(app.world().resource::<Editor>().document_dirty(1));
    for (room, expected) in [(0, 60.0), (1, 80.0)] {
        action(&mut app, Action::Open(room));
        assert_eq!(app.world().resource::<Editor>().camera(0).lens, expected);
        assert!(
            !scratch
                .0
                .join(&app.world().resource::<Editor>().definition().file)
                .exists()
        );
    }
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
}

#[test]
fn app_close_waits_for_a_decision_on_every_dirty_tab() {
    for save in [false, true] {
        let (mut app, scratch) = two_dirty_scenes();
        let window = close_requests(&mut app);
        request_close(&mut app, window);
        let mut handled = Vec::new();
        for index in 0..2 {
            let editor = app.world().resource::<Editor>();
            assert_eq!(editor.pending, Some(PendingAction::Close));
            let room = editor.room;
            assert!(!handled.contains(&room), "Each dirty tab is reviewed once");
            handled.push(room);
            let file = editor.definition().file.clone();
            let expected = editor.edits.clone();
            ready(&mut app);
            action(
                &mut app,
                if save {
                    Action::SaveAndContinue
                } else {
                    Action::DiscardAndContinue
                },
            );
            if save {
                let written: RoomEdits = RoomEdits::read(&scratch.0, &file).unwrap();
                assert_eq!(written, expected);
            } else {
                assert!(!scratch.0.join(file).exists());
            }
            if index == 0 {
                assert!(app.world().resource::<Messages<AppExit>>().is_empty());
            }
        }
        let exits: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<AppExit>>()
            .drain()
            .collect();
        assert_eq!(exits, vec![AppExit::Success]);
    }
}
