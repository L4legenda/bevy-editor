use super::*;
use crate::{EditorProject, LevelEdits};
use bevy::input_focus::FocusCause;

fn fixture(field: Field) -> (App, Entity) {
    let project = EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, LevelEdits::default());
    editor.ready = true;
    editor.selection = Some(Selection::Camera(0));
    let text = display_value(&editor, field);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<NumericInput>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_message::<WindowCloseRequested>()
        .add_systems(Update, update);
    let input = app.world_mut().spawn((field, EditableText::new(text))).id();
    app.insert_resource(InputFocus::from_entity(input));
    app.update();
    (app, input)
}

fn type_value(app: &mut App, entity: Entity, value: &str) {
    app.world_mut()
        .get_mut::<EditableText>(entity)
        .unwrap()
        .editor_mut()
        .set_text(value);
}

fn press(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
}

fn draft_changed(app: &App) -> bool {
    app.world()
        .resource::<NumericInput>()
        .has_changes(app.world().resource::<Editor>())
}

#[test]
fn focused_draft_tracks_edits_and_reverting_without_changing_the_document() {
    let (mut app, input) = fixture(Field::Lens);
    assert!(!draft_changed(&app));
    type_value(&mut app, input, "60");
    app.update();
    assert!(draft_changed(&app));
    assert!(!app.world().resource::<Editor>().dirty());
    assert!(app.world().resource::<Editor>().undo.is_empty());

    type_value(&mut app, input, "45.00");
    app.update();
    assert!(!draft_changed(&app));
    assert!(!app.world().resource::<Editor>().dirty());
}

#[test]
fn enter_commits_a_decimal_comma_once_and_leaves_the_field() {
    let (mut app, input) = fixture(Field::Position(0));
    type_value(&mut app, input, " 2,5 ");
    press(&mut app, KeyCode::Enter);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).pose.position[0], 2.5);
    assert_eq!(editor.undo.len(), 1);
    assert!(!draft_changed(&app));
    assert!(app.world().resource::<InputFocus>().get().is_none());
    app.update();
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
}

#[test]
fn native_close_commits_the_focused_draft_before_checking_unsaved_changes() {
    use crate::editor::{PendingAction, close_editor};

    let (mut app, input) = fixture(Field::Position(0));
    let window = app.world_mut().spawn_empty().id();
    app.add_message::<AppExit>()
        .add_systems(Update, close_editor.after(update));
    type_value(&mut app, input, "9.25");
    app.world_mut()
        .write_message(WindowCloseRequested { window });
    app.update();

    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).pose.position[0], 9.25);
    assert_eq!(editor.undo.len(), 1);
    assert!(matches!(editor.pending, Some(PendingAction::Close)));
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
}

#[test]
fn native_close_keeps_an_invalid_draft_and_does_not_exit() {
    use crate::editor::close_editor;

    let (mut app, input) = fixture(Field::Lens);
    let window = app.world_mut().spawn_empty().id();
    app.add_message::<AppExit>()
        .add_systems(Update, close_editor.after(update));
    type_value(&mut app, input, "300");
    app.world_mut()
        .write_message(WindowCloseRequested { window });
    app.update();

    assert!(app.world().resource::<NumericInput>().preserve_text(input));
    assert!(app.world().resource::<Editor>().pending.is_none());
    assert_eq!(app.world().resource::<Editor>().camera(0).lens, 45.0);
    assert!(app.world().resource::<Messages<AppExit>>().is_empty());
}

struct SaveScratch(std::path::PathBuf);

impl SaveScratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "bevy-editor-invalid-number-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
    }
}

impl Drop for SaveScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn control_s_commits_the_focused_draft_and_saves_the_scene() {
    use crate::editor::{Action, Scene, handle_actions, ui, viewport::PointerState};

    let (mut app, input) = fixture(Field::Lens);
    let scratch = SaveScratch::new();
    app.world_mut().resource_mut::<Editor>().root = scratch.0.clone();
    app.init_resource::<Scene>()
        .init_resource::<PointerState>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<Action>()
        .add_systems(
            Update,
            (ui::shortcuts.before(update), handle_actions.after(update)),
        );
    type_value(&mut app, input, "60");
    app.update();
    assert!(draft_changed(&app));
    assert!(!app.world().resource::<Editor>().dirty());

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ControlLeft);
    press(&mut app, KeyCode::KeyS);

    let editor = app.world().resource::<Editor>();
    let saved: LevelEdits = LevelEdits::read(&scratch.0, &editor.definition().file).unwrap();
    assert_eq!(saved.cameras["main"].lens, 60.0);
    assert_eq!(editor.camera(0).lens, 60.0);
    assert!(!editor.dirty());
    assert!(!draft_changed(&app));
    assert!(app.world().resource::<InputFocus>().get().is_none());
}

#[test]
fn save_does_not_write_a_document_while_a_numeric_draft_is_invalid() {
    use crate::editor::{Action, Scene, handle_actions, viewport::PointerState};

    let (mut app, input) = fixture(Field::Lens);
    let scratch = SaveScratch::new();
    app.world_mut().resource_mut::<Editor>().root = scratch.0.clone();
    app.init_resource::<Scene>()
        .init_resource::<PointerState>()
        .add_message::<Action>()
        .add_systems(Update, handle_actions.after(update));
    type_value(&mut app, input, "300");
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.world_mut().write_message(Action::Save);
    app.update();

    let editor = app.world().resource::<Editor>();
    assert!(!scratch.0.join(&editor.definition().file).exists());
    assert_eq!(editor.camera(0).lens, 45.0);
    assert!(app.world().resource::<NumericInput>().preserve_text(input));
    assert!(!editor.status.starts_with("Сохранено"));
}

#[test]
fn blur_and_tab_apply_the_old_field_without_changing_the_next_one() {
    for tab in [true, false] {
        let (mut app, input) = fixture(Field::Position(0));
        let next = app
            .world_mut()
            .spawn((Field::Position(1), EditableText::new("4.00")))
            .id();
        type_value(&mut app, input, "9.25");
        if tab {
            // Tab navigation changes InputFocus during PreUpdate, before numeric update runs.
            app.world_mut()
                .resource_mut::<InputFocus>()
                .set(next, FocusCause::Navigated);
            press(&mut app, KeyCode::Tab);
        } else {
            app.world_mut().resource_mut::<InputFocus>().clear();
            app.update();
        }
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.camera(0).pose.position, [9.25, 4.0, 7.0]);
        assert_eq!(editor.undo.len(), 1);
    }
}

#[test]
fn leaving_an_unedited_rounded_field_preserves_the_full_precision_pose() {
    let (mut app, input) = fixture(Field::Position(0));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.project.scenes[0].cameras[0].placement.pose.position[0] = 1.234567;
    }
    // Start a fresh focus session after the field has been refreshed by the inspector.
    app.insert_resource(NumericInput::default());
    type_value(&mut app, input, "1.23");
    app.update();
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).pose.position[0], 1.234567);
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty());
}

#[test]
fn escape_discards_even_invalid_input_without_editing_the_scene() {
    let (mut app, input) = fixture(Field::Lens);
    type_value(&mut app, input, "200");
    press(&mut app, KeyCode::Enter);
    assert!(app.world().resource::<NumericInput>().invalid(input));
    assert!(draft_changed(&app));
    press(&mut app, KeyCode::Escape);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).lens, 45.0);
    assert!(editor.undo.is_empty());
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert!(!app.world().resource::<NumericInput>().preserve_text(input));
    assert!(!draft_changed(&app));
    assert_eq!(
        app.world()
            .get::<EditableText>(input)
            .unwrap()
            .value()
            .to_string(),
        "45.00"
    );
}

#[test]
fn invalid_values_survive_blur_as_visible_drafts_and_can_be_corrected() {
    for value in ["wrong", "NaN", "inf", "1001", "-1001"] {
        let (mut app, input) = fixture(Field::Position(0));
        type_value(&mut app, input, value);
        app.world_mut().resource_mut::<InputFocus>().clear();
        app.update();
        assert!(!app.world().resource::<Editor>().dirty());
        assert!(app.world().resource::<NumericInput>().preserve_text(input));
        assert!(draft_changed(&app));
        assert!(
            app.world()
                .resource::<NumericInput>()
                .error_message()
                .is_some()
        );
        assert_eq!(
            app.world()
                .get::<EditableText>(input)
                .unwrap()
                .value()
                .to_string(),
            value
        );
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(input, FocusCause::Navigated);
        type_value(&mut app, input, "8");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.world().resource::<Editor>().camera(0).pose.position[0],
            8.0
        );
        assert!(!app.world().resource::<NumericInput>().invalid(input));
        assert!(!draft_changed(&app));
    }
}

#[test]
fn changing_selection_does_not_apply_an_old_draft_to_a_different_camera() {
    let (mut app, input) = fixture(Field::Position(0));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let mut second = editor.project.scenes[0].cameras[0].clone();
        second.id = "other".into();
        second.placement.pose.position[0] = -6.0;
        editor.project.scenes[0].cameras.push(second);
    }
    type_value(&mut app, input, "123");
    app.update();
    assert!(draft_changed(&app));
    app.world_mut()
        .resource_mut::<Editor>()
        .select(Some(Selection::Camera(1)));
    press(&mut app, KeyCode::Enter);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0).pose.position[0], 6.0);
    assert_eq!(editor.camera(1).pose.position[0], -6.0);
    assert!(editor.undo.is_empty());
    assert!(!draft_changed(&app));
}

#[test]
fn blur_commit_precedes_an_already_queued_selection_action() {
    use crate::editor::{Action, Scene, handle_actions, viewport::PointerState};

    let (mut app, input) = fixture(Field::Position(0));
    app.init_resource::<Scene>()
        .init_resource::<PointerState>()
        .add_message::<Action>()
        .add_systems(Update, handle_actions.after(update));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let mut second = editor.project.scenes[0].cameras[0].clone();
        second.id = "other".into();
        second.placement.pose.position[0] = -6.0;
        editor.project.scenes[0].cameras.push(second);
    }
    type_value(&mut app, input, "123");
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.world_mut()
        .write_message(Action::Select(Selection::Camera(1)));
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.selection, Some(Selection::Camera(1)));
    assert_eq!(editor.camera(0).pose.position[0], 123.0);
    assert_eq!(editor.camera(1).pose.position[0], -6.0);
    assert_eq!(editor.undo.len(), 1);
}

#[test]
fn a_valid_draft_commits_in_the_old_space_before_the_space_toggle() {
    use crate::editor::{Action, Imported, Scene, handle_actions, viewport::PointerState};
    let (mut app, input) = fixture(Field::Position(0));
    app.init_resource::<Scene>()
        .init_resource::<PointerState>()
        .add_message::<Action>()
        .add_systems(Update, handle_actions.after(update));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let parent = GlobalTransform::from(
            Transform::from_xyz(10.0, 20.0, 0.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        );
        let local = Transform::from_xyz(1.0, 2.0, 0.0);
        editor.imported.push(Imported {
            entity: Entity::PLACEHOLDER,
            name: "Space draft".into(),
            original: parent.mul_transform(local).compute_transform().into(),
            parent,
            local,
        });
        editor.selection = Some(Selection::Imported(0));
    }
    app.update();
    type_value(&mut app, input, "5");
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.world_mut().write_message(Action::LocalSpace);
    app.update();
    let editor = app.world().resource::<Editor>();
    assert!(editor.local_space);
    assert!((editor.pose(Selection::Imported(0)).unwrap().position[0] - 5.0).abs() < 1e-4);
    assert!(
        (editor
            .field_value(Selection::Imported(0), Field::Position(0))
            .unwrap()
            - 1.0)
            .abs()
            < 1e-4
    );
    assert_eq!(editor.undo.len(), 1);
}

#[test]
fn an_invalid_draft_blocks_space_switching_and_keeps_the_entered_text() {
    use crate::editor::{Action, Scene, handle_actions, viewport::PointerState};
    let (mut app, input) = fixture(Field::Position(0));
    app.init_resource::<Scene>()
        .init_resource::<PointerState>()
        .add_message::<Action>()
        .add_systems(Update, handle_actions.after(update));
    type_value(&mut app, input, "-");
    app.world_mut().resource_mut::<InputFocus>().clear();
    app.world_mut().write_message(Action::LocalSpace);
    app.update();
    assert!(!app.world().resource::<Editor>().local_space);
    assert!(app.world().resource::<NumericInput>().invalid(input));
    assert_eq!(
        app.world()
            .get::<EditableText>(input)
            .unwrap()
            .value()
            .to_string(),
        "-"
    );
    assert!(app.world().resource::<Editor>().undo.is_empty());
}

#[test]
fn changing_scene_discards_invalid_field_state() {
    let (mut app, input) = fixture(Field::Lens);
    type_value(&mut app, input, "300");
    press(&mut app, KeyCode::Enter);
    assert!(draft_changed(&app));
    app.world_mut().resource_mut::<Editor>().room = 1;
    app.update();
    assert!(!app.world().resource::<NumericInput>().invalid(input));
    assert!(!app.world().resource::<Editor>().dirty());
    assert!(!draft_changed(&app));
    assert_eq!(
        app.world()
            .get::<EditableText>(input)
            .unwrap()
            .value()
            .to_string(),
        "45.00"
    );
}
