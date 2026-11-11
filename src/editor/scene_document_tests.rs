use super::*;

pub(super) fn workspace() -> (App, CameraSavePath) {
    let scratch = CameraSavePath::new();
    std::fs::create_dir_all(&scratch.0).unwrap();
    let mut app = app();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.root = scratch.0.clone();
        editor.project.asset_root = scratch.0.clone();
    }
    ready(&mut app);
    (app, scratch)
}

/// The document harness omits asset loading; simulate its successful completion explicitly.
pub(super) fn ready(app: &mut App) {
    let mut editor = app.world_mut().resource_mut::<Editor>();
    editor.ready = true;
    editor.reload = false;
}

pub(super) fn lens(app: &mut App, value: f32) {
    action(app, Action::Select(Selection::Camera(0)));
    action(app, Action::Number(Field::Lens, value));
}

#[test]
fn only_the_initial_scene_is_open_and_opening_a_document_twice_reuses_its_tab() {
    let (mut app, _scratch) = workspace();
    let editor = app.world().resource::<Editor>();
    assert!(editor.has_scene());
    assert_eq!(editor.open_tabs, [0]);
    assert!(!editor.document_dirty(0));

    action(&mut app, Action::Open(1));
    assert_eq!(app.world().resource::<Editor>().open_tabs, [0, 1]);
    assert_eq!(app.world().resource::<Editor>().room, 1);
    action(&mut app, Action::Open(1));
    action(&mut app, Action::Open(0));
    action(&mut app, Action::Open(1));
    assert_eq!(app.world().resource::<Editor>().open_tabs, [0, 1]);
    assert_eq!(app.world().resource::<Editor>().room, 1);
    assert!(app.world().resource::<Editor>().pending.is_none());
}

#[test]
fn switching_tabs_preserves_each_draft_and_its_undo_redo_history_without_saving() {
    let (mut app, scratch) = workspace();
    lens(&mut app, 60.0);
    lens(&mut app, 70.0);
    action(&mut app, Action::Undo);
    let editor = app.world().resource::<Editor>();
    let edits = editor.edits.clone();
    let undo = editor.undo.clone();
    let redo = editor.redo.clone();
    let first_file = editor.definition().file.clone();

    action(&mut app, Action::Open(1));
    ready(&mut app);
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 45.0);
    assert!(app.world().resource::<Editor>().undo.is_empty());
    assert!(app.world().resource::<Editor>().redo.is_empty());
    lens(&mut app, 80.0);
    let second_file = app.world().resource::<Editor>().definition().file.clone();

    action(&mut app, Action::Open(0));
    ready(&mut app);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits, edits);
    assert_eq!(editor.undo, undo);
    assert_eq!(editor.redo, redo);
    assert!(editor.document_dirty(0));
    assert!(editor.document_dirty(1));
    assert!(editor.pending.is_none());
    assert!(!scratch.0.join(first_file).exists());
    assert!(!scratch.0.join(second_file).exists());
    action(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 70.0);

    action(&mut app, Action::Open(1));
    ready(&mut app);
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 80.0);
    action(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 45.0);
    assert!(app.world().resource::<Editor>().document_dirty(0));
    assert!(!app.world().resource::<Editor>().document_dirty(1));
}

#[test]
fn closing_the_last_clean_tab_leaves_an_empty_workspace_that_can_open_a_scene() {
    let (mut app, scratch) = workspace();
    let file = app.world().resource::<Editor>().definition().file.clone();
    action(&mut app, Action::CloseTab(0));
    let editor = app.world().resource::<Editor>();
    assert!(!editor.has_scene());
    assert!(editor.open_tabs.is_empty());
    assert!(editor.selection.is_none());
    assert!(!editor.ready);
    assert!(!editor.pilot);
    assert!(!editor.dirty());
    for blocked in [Action::Add(0), Action::Save, Action::Undo, Action::Pilot] {
        action(&mut app, blocked);
    }
    assert!(!app.world().resource::<Editor>().has_scene());
    assert!(app.world().resource::<Editor>().edits.models.is_empty());
    assert!(!scratch.0.join(file).exists());

    action(&mut app, Action::OpenBrowser);
    assert!(app.world().resource::<Editor>().drawer);
    action(&mut app, Action::Open(1));
    let editor = app.world().resource::<Editor>();
    assert!(editor.has_scene());
    assert_eq!(editor.open_tabs, [1]);
    assert_eq!(editor.room, 1);
    assert!(!editor.dirty());
    assert!(editor.reload);
}

#[test]
fn an_unreadable_scene_does_not_replace_the_current_draft_or_add_a_tab() {
    let (mut app, scratch) = workspace();
    lens(&mut app, 60.0);
    let editor = app.world().resource::<Editor>();
    let edits = editor.edits.clone();
    let undo = editor.undo.clone();
    let target = scratch.0.join(&editor.project.scenes[1].file);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, b"invalid JSON").unwrap();

    action(&mut app, Action::Open(1));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.room, 0);
    assert_eq!(editor.open_tabs, [0]);
    assert_eq!(editor.edits, edits);
    assert_eq!(editor.undo, undo);
    assert!(editor.dirty());
    assert!(editor.pending.is_none());
    assert!(editor.status.starts_with("Не удалось открыть"));
}

#[test]
fn reopening_a_closed_document_loads_its_saved_data_instead_of_an_old_session() {
    let (mut app, scratch) = workspace();
    let editor = app.world().resource::<Editor>();
    let file = editor.definition().file.clone();
    let mut saved = RoomEdits::default();
    saved.cameras.insert(
        "main".into(),
        CameraPlacement {
            lens: 55.0,
            ..editor.camera(0)
        },
    );
    action(&mut app, Action::CloseTab(0));
    saved.write(&scratch.0, &file).unwrap();
    action(&mut app, Action::Open(0));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.open_tabs, [0]);
    assert_eq!(editor.edits, saved);
    assert_eq!(editor.camera(0).lens, 55.0);
    assert!(editor.undo.is_empty());
    assert!(editor.redo.is_empty());
    assert!(!editor.dirty());
}

#[test]
fn closing_a_clean_inactive_tab_does_not_reload_or_change_the_active_document() {
    let (mut app, _scratch) = workspace();
    action(&mut app, Action::Open(1));
    ready(&mut app);
    lens(&mut app, 80.0);
    lens(&mut app, 90.0);
    action(&mut app, Action::Undo);
    let editor = app.world().resource::<Editor>();
    assert!(!editor.document_dirty(0));
    let edits = editor.edits.clone();
    let saved = editor.saved.clone();
    let undo = editor.undo.clone();
    let redo = editor.redo.clone();
    let selection = editor.selection;

    action(&mut app, Action::CloseTab(0));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.open_tabs, [1]);
    assert_eq!(editor.room, 1);
    assert_eq!(editor.edits, edits);
    assert_eq!(editor.saved, saved);
    assert_eq!(editor.undo, undo);
    assert_eq!(editor.redo, redo);
    assert_eq!(editor.selection, selection);
    assert!(editor.ready);
    assert!(!editor.reload);
    assert!(editor.pending.is_none());
    assert!(editor.dirty());
}

#[test]
fn returning_to_a_tab_rebinds_imported_selection_by_name_and_restores_the_free_view() {
    for target_removed in [false, true] {
        let (mut app, _scratch) = workspace();
        record_placements(&mut app, &["Zebra", "Target", "Anchor"]);
        action(&mut app, Action::Select(Selection::Imported(1)));
        action(&mut app, Action::Number(Field::Position(0), 4.0));
        let view = Transform::from_xyz(12.0, 7.0, -8.0).looking_at(Vec3::ZERO, Vec3::Y);
        let pivot = Vec3::new(2.0, 3.0, 4.0);
        {
            let mut editor = app.world_mut().resource_mut::<Editor>();
            assert_eq!(editor.imported[1].name, "Target");
            editor.view = view;
            editor.pivot = pivot;
        }
        let editor = app.world().resource::<Editor>();
        let old_entity = editor.imported[1].entity;
        let edits = editor.edits.clone();
        let history = editor.undo.clone();

        action(&mut app, Action::Open(1));
        record_placements(&mut app, &["Other"]);
        {
            let mut editor = app.world_mut().resource_mut::<Editor>();
            editor.view = Transform::from_xyz(-15.0, 2.0, 11.0);
            editor.pivot = Vec3::splat(-5.0);
        }
        action(&mut app, Action::Open(0));
        assert!(app.world().resource::<Editor>().selection.is_none());
        assert!(!app.world().resource::<Editor>().ready);
        // Change the set and source order, shifting the sorted index of Target from 1 to 2.
        let mut names = vec!["Zebra", "Aardvark", "Anchor"];
        if !target_removed {
            names.push("Target");
        }
        record_placements(&mut app, &names);
        let editor = app.world().resource::<Editor>();
        assert!(editor.ready);
        assert_eq!(editor.view, view);
        assert_eq!(editor.pivot, pivot);
        assert_eq!(editor.edits, edits);
        assert_eq!(editor.undo, history);
        if target_removed {
            assert!(
                editor.selection.is_none(),
                "A missing node must not select its old index"
            );
        } else {
            assert_eq!(editor.selection, Some(Selection::Imported(2)));
            assert_eq!(editor.imported[2].name, "Target");
            assert_ne!(editor.imported[2].entity, old_entity);
        }
    }
}
