//! A numeric draft belongs to one field, selection and scene until it is committed.
use std::collections::HashMap;

use bevy::{input_focus::InputFocus, prelude::*, text::EditableText, window::WindowCloseRequested};

use super::{Editor, Field, Selection};
use crate::project::LensKind;

#[cfg(test)]
#[path = "numeric_input_tests.rs"]
mod tests;

struct Draft {
    room: usize,
    selection: Selection,
    local_space: bool,
    field: Field,
    original_text: String,
    changed: bool,
    error: Option<&'static str>,
}

impl Draft {
    fn belongs_to(&self, editor: &Editor) -> bool {
        editor.room == self.room
            && editor.selection == Some(self.selection)
            && editor.local_space == self.local_space
    }

    fn update_changed(&mut self, text: &str) {
        self.changed = text != self.original_text;
        if !self.changed {
            self.error = None;
        }
    }
}

#[derive(Resource, Default)]
pub(in crate::editor) struct NumericInput {
    active: Option<Entity>,
    drafts: HashMap<Entity, Draft>,
}

impl NumericInput {
    fn begin(&mut self, entity: Entity, editor: &Editor, field: Field) {
        if let Some(selection) = editor.selection {
            self.drafts.entry(entity).or_insert_with(|| Draft {
                room: editor.room,
                selection,
                local_space: editor.local_space,
                field,
                original_text: display_value(editor, field),
                changed: false,
                error: None,
            });
        }
    }

    pub(super) fn invalid(&self, entity: Entity) -> bool {
        self.drafts
            .get(&entity)
            .is_some_and(|draft| draft.error.is_some())
    }

    pub(super) fn preserve_text(&self, entity: Entity) -> bool {
        self.invalid(entity)
    }

    pub(in crate::editor) fn has_changes(&self, editor: &Editor) -> bool {
        self.drafts
            .values()
            .any(|draft| draft.changed && draft.belongs_to(editor))
    }

    pub(in crate::editor) fn error_message(&self) -> Option<&'static str> {
        self.active
            .and_then(|entity| self.drafts.get(&entity))
            .and_then(|draft| draft.error)
            .or_else(|| self.drafts.values().find_map(|draft| draft.error))
    }

    fn commit(&mut self, entity: Entity, input: &mut EditableText, editor: &mut Editor) {
        let Some(draft) = self.drafts.get_mut(&entity) else {
            return;
        };
        if !draft.belongs_to(editor) || !editor.field_allowed(draft.selection, draft.field) {
            self.drafts.remove(&entity);
            return;
        }
        let text = input.value().to_string();
        draft.update_changed(&text);
        if !draft.changed {
            self.drafts.remove(&entity);
            return;
        }
        match parse_value(editor, draft, &text) {
            Ok(value) => {
                let field = draft.field;
                let selection = draft.selection;
                editor.finish_camera_navigation();
                let before = editor.edits.clone();
                editor.set_field(field, value);
                let changed = before != editor.edits;
                editor.finish_edit(before);
                restore_value(input, editor, field);
                self.drafts.remove(&entity);
                if changed {
                    editor.status = format!("Изменено · {}", editor.label(selection));
                }
            }
            Err(message) => {
                if let Some(draft) = self.drafts.get_mut(&entity) {
                    draft.error = Some(message);
                }
                editor.status = message.into();
            }
        }
    }
}

fn restore_value(input: &mut EditableText, editor: &Editor, field: Field) {
    input.editor_mut().set_text(&display_value(editor, field));
}

fn display_value(editor: &Editor, field: Field) -> String {
    let value = editor
        .selection
        .and_then(|selection| editor.field_value(selection, field));
    value.map_or_else(|| "—".into(), |value| format!("{value:.2}"))
}

fn parse_value(editor: &Editor, draft: &Draft, text: &str) -> Result<f32, &'static str> {
    let value = text
        .trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or("Введите конечное число, например 2,5")?;
    match draft.field {
        Field::Position(_) if !(-1000.0..=1000.0).contains(&value) => {
            Err("Позиция: от −1000 до 1000 м")
        }
        Field::Scale(_) if !(0.01..=100.0).contains(&value.abs()) => {
            Err("Масштаб: от 0,01 до 100 по модулю; ноль запрещён")
        }
        Field::Lens => {
            let Selection::Camera(shot) = draft.selection else {
                return Err("Выберите камеру для настройки объектива");
            };
            let kind = editor.definition().cameras[shot].projection;
            if kind.valid_lens(value) {
                Ok(value)
            } else if kind == LensKind::Orthographic {
                Err("Высота кадра: от 0,05 до 20 м")
            } else {
                Err("Угол обзора: от 5 до 145°")
            }
        }
        Field::Light(_) | Field::Environment(_) => editor
            .validate_lighting_field(draft.field, value)
            .map(|()| value),
        _ => Ok(value),
    }
}

/// Run after focus/shortcut processing, before actions that can save or select another object.
pub(in crate::editor) fn update(
    mut state: ResMut<NumericInput>,
    mut editor: ResMut<Editor>,
    mut focus: ResMut<InputFocus>,
    keys: Res<ButtonInput<KeyCode>>,
    mut fields: Query<(Entity, &Field, &mut EditableText)>,
    mut close_requests: MessageReader<WindowCloseRequested>,
) {
    let closing = close_requests.read().last().is_some();
    if !editor.ready || editor.pending.is_some() {
        if focus.get().is_some_and(|entity| fields.contains(entity)) {
            focus.clear();
        }
        state.active = None;
        state.drafts.clear();
        return;
    }

    // A field entity is reused by the inspector; its old draft must never follow a new selection.
    let stale: Vec<_> = state
        .drafts
        .iter()
        .filter_map(|(&entity, draft)| {
            (!draft.belongs_to(&editor) || !fields.contains(entity)).then_some(entity)
        })
        .collect();
    for entity in stale {
        state.drafts.remove(&entity);
        if let Ok((_, &field, mut input)) = fields.get_mut(entity) {
            restore_value(&mut input, &editor, field);
        }
        if state.active == Some(entity) {
            state.active = None;
        }
    }

    // Native close does not necessarily blur the TextInput. Commit its current value before
    // close_editor decides whether the scene has unsaved changes and opens the confirmation.
    if closing
        && let Some(entity) = focus.get()
        && let Ok((_, &field, mut input)) = fields.get_mut(entity)
    {
        // This also covers typing and closing within the same frame as acquiring focus.
        state.begin(entity, &editor, field);
        state.commit(entity, &mut input, &mut editor);
        focus.clear();
    }

    let active = focus.get().filter(|&entity| {
        fields.get(entity).is_ok_and(|(_, &field, _)| {
            editor
                .selection
                .is_some_and(|selection| editor.field_allowed(selection, field))
        })
    });
    if state.active != active {
        if let Some(previous) = state.active
            && let Ok((_, _, mut input)) = fields.get_mut(previous)
        {
            state.commit(previous, &mut input, &mut editor);
        }
        state.active = active;
    }
    let Some(entity) = active else { return };
    let Ok((_, &field, mut input)) = fields.get_mut(entity) else {
        return;
    };
    state.begin(entity, &editor, field);
    if let Some(draft) = state.drafts.get_mut(&entity) {
        draft.update_changed(&input.value().to_string());
    }

    if keys.just_pressed(KeyCode::Escape) {
        restore_value(&mut input, &editor, field);
        state.drafts.remove(&entity);
        state.active = None;
        focus.clear();
        editor.status = "Ввод отменён".into();
    } else if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Tab]) {
        state.commit(entity, &mut input, &mut editor);
        if !state.invalid(entity) && !keys.just_pressed(KeyCode::Tab) {
            state.active = None;
            focus.clear();
        }
    }
}
