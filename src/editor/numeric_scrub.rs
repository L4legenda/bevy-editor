//! Shared horizontal dragging for numeric text fields, with one scene undo per gesture.
use super::{NumericInput, display_value};
use crate::editor::{Editor, Field, RoomEdits, Selection};
use bevy::{
    input_focus::{FocusCause, InputFocus},
    prelude::*,
    text::{EditableText, TextEdit, TextReadWriteMode},
    ui::{CalculatedClip, InteractionDisabled},
    ui_widgets::{SelectAllOnFocus, TextInput},
    window::{CursorIcon, PrimaryWindow, SystemCursorIcon, WindowCloseRequested},
};

#[cfg(test)]
#[path = "numeric_scrub_tests.rs"]
mod tests;

#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(SelectAllOnFocus)]
pub(in crate::editor) struct Number {
    min: f64,
    max: f64,
    speed: f64,
    precision: usize,
    nonzero: f64,
    blur: bool,
}

impl Number {
    pub fn decimal(min: f32, max: f32, speed: f32, precision: usize) -> Self {
        Self {
            min: min as f64,
            max: max as f64,
            speed: speed as f64,
            precision,
            nonzero: 0.0,
            blur: false,
        }
    }
    pub fn integer(min: u32, max: u32) -> Self {
        Self::decimal(min as f32, max as f32, 0.25, 0)
    }
    pub fn finish(mut self) -> Self {
        self.blur = true;
        self
    }
    pub fn nonzero(mut self, min: f32) -> Self {
        self.nonzero = min as f64;
        self
    }
    pub fn bounds(mut self, min: f32, max: f32) -> Self {
        self.min = min.min(max) as f64;
        self.max = max as f64;
        self
    }
    pub fn scene(field: Field) -> Self {
        match field {
            Field::Position(_) => Self::decimal(-1000.0, 1000.0, 0.05, 2),
            Field::Rotation(_) => Self::decimal(-1.0e6, 1.0e6, 0.5, 2),
            Field::Scale(_) => Self::decimal(-100.0, 100.0, 0.01, 2).nonzero(0.01),
            Field::Lens => Self::decimal(5.0, 145.0, 0.25, 2),
            _ => Self::decimal(-1.0e9, 1.0e9, 1.0, 2),
        }
        .finish()
    }
    fn constrain(self, value: f64, previous: f64) -> f64 {
        let value = value.clamp(self.min, self.max);
        if value.abs() < self.nonzero {
            self.nonzero.copysign(if value == 0.0 {
                value - previous
            } else {
                value
            })
        } else {
            value
        }
    }
    fn text(self, value: f64, comma: bool) -> String {
        let scale = 10.0_f64.powi(self.precision as i32);
        let value = (value * scale).round() / scale;
        let mut text = format!(
            "{:.*}",
            self.precision,
            if value == 0.0 { 0.0 } else { value }
        );
        if comma {
            text = text.replace('.', ",");
        }
        text
    }
}

struct SceneDrag {
    room: usize,
    selection: Selection,
    targets: Vec<Selection>,
    local: bool,
    field: Field,
    before: RoomEdits,
}
struct Drag {
    entity: Entity,
    start: Vec2,
    previous: Vec2,
    original: String,
    value: f64,
    number: Number,
    mode: TextReadWriteMode,
    moving: bool,
    scene: Option<SceneDrag>,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct Scrub {
    drag: Option<Drag>,
    suppressed: bool,
    cursor_owned: bool,
    previous_cursor: Option<CursorIcon>,
}
impl Scrub {
    pub fn capturing(&self) -> bool {
        self.drag.as_ref().is_some_and(|drag| drag.moving) || self.suppressed
    }
    pub fn active(&self, entity: Entity) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|drag| drag.entity == entity && drag.moving)
    }
}

type Fields<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Number,
        &'static mut EditableText,
        &'static TextReadWriteMode,
        Has<InteractionDisabled>,
        Option<&'static Field>,
    ),
    With<TextInput>,
>;

pub(in crate::editor) fn input(
    mut commands: Commands,
    mut scrub: ResMut<Scrub>,
    mut numeric: ResMut<NumericInput>,
    mut editor: ResMut<Editor>,
    mut focus: ResMut<InputFocus>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut fields: Fields,
    geometry: Query<(&ComputedNode, &UiGlobalTransform, Option<&CalculatedClip>)>,
    mut closing: MessageReader<WindowCloseRequested>,
) {
    let closing = closing.read().last().is_some();
    if !mouse.pressed(MouseButton::Left) || !window.focused {
        scrub.suppressed = false;
    }
    let point = window.cursor_position();
    if scrub.drag.is_none()
        && !scrub.suppressed
        && mouse.just_pressed(MouseButton::Left)
        && window.focused
        && editor.pending.is_none()
        && let Some(point) = point
    {
        let physical = point * window.scale_factor();
        let candidate = fields
            .iter()
            .filter(|(_, _, _, mode, disabled, _)| {
                !disabled && **mode == TextReadWriteMode::Editable
            })
            .find_map(|(entity, number, text, mode, _, field)| {
                // Picking has already focused the topmost input. This also
                // prevents grabbing an inspector field behind a modal overlay.
                if focus.get() != Some(entity) {
                    return None;
                }
                let (node, pose, clip) = geometry.get(entity).ok()?;
                if node.is_empty()
                    || !node.contains_point(*pose, physical)
                    || clip.is_some_and(|clip| !clip.contains_point(physical))
                {
                    return None;
                }
                let original = text.value().to_string();
                let value = original
                    .trim()
                    .replace(',', ".")
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .or_else(|| {
                        let field = field.copied()?;
                        if original != "—" || !editor.inspector_field_allowed(field) {
                            return None;
                        }
                        editor.field_value(editor.selection?, field).map(f64::from)
                    })?;
                Some((entity, *number, original, value, *mode, field.copied()))
            });
        if let Some((entity, mut number, original, value, mode, field)) = candidate {
            if let Some(Field::Lens) = field
                && let Some(Selection::Camera(shot)) = editor.selection
                && editor.definition().cameras[shot].projection
                    == crate::project::LensKind::Orthographic
            {
                number = Number::decimal(0.05, 20.0, 0.01, 2).finish();
            }
            scrub.drag = Some(Drag {
                entity,
                number,
                original,
                value,
                mode,
                start: point,
                previous: point,
                moving: false,
                scene: None,
            });
        }
    }
    let Some(mut drag) = scrub.drag.take() else {
        return;
    };
    let field = fields
        .get(drag.entity)
        .ok()
        .and_then(|(.., field)| field.copied());
    let accessible = editor.pending.is_none()
        && field.is_none_or(|field| editor.inspector_field_allowed(field))
        && geometry
            .get(drag.entity)
            .is_ok_and(|(node, _, _)| !node.is_empty())
        && fields
            .get(drag.entity)
            .is_ok_and(|(_, _, _, _, disabled, _)| !disabled);
    let available = accessible && focus.get() == Some(drag.entity);
    let cancel = drag.moving && keys.just_pressed(KeyCode::Escape);
    let finish = !available
        || !window.focused
        || closing
        || !mouse.pressed(MouseButton::Left)
        || (!drag.moving && keys.just_pressed(KeyCode::Escape))
        || keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Tab]);
    if !drag.moving
        && !finish
        && let Some(point) = point
        && (point.x - drag.start.x).abs() >= 4.0
        && (point.x - drag.start.x).abs() >= (point.y - drag.start.y).abs()
    {
        drag.moving = true;
        if let Some(field) = field {
            if let Ok((_, _, mut input, ..)) = fields.get_mut(drag.entity) {
                numeric.begin(drag.entity, &editor, field);
                numeric.commit(drag.entity, &mut input, &mut editor);
            }
            if let Some(selection) = editor.selection {
                editor.finish_camera_navigation();
                drag.scene = Some(SceneDrag {
                    room: editor.room,
                    selection,
                    targets: editor.inspector_targets(field).collect(),
                    local: editor.local_space,
                    field,
                    before: editor.edits.clone(),
                });
            }
        }
        numeric.drafts.remove(&drag.entity);
        numeric.active = None;
        focus.set(drag.entity, FocusCause::Pressed);
    }
    if drag.moving {
        let same_scene = drag.scene.as_ref().is_none_or(|scene| {
            scene.room == editor.room
                && Some(scene.selection) == editor.selection
                && scene.local == editor.local_space
                && editor
                    .inspector_targets(scene.field)
                    .eq(scene.targets.iter().copied())
        });
        if let Ok((_, _, mut input, ..)) = fields.get_mut(drag.entity) {
            input.pending_edits.clear();
            if cancel {
                input.editor_mut().set_text(&drag.original);
                // TextInput blurs in PreUpdate on Escape. Live text adapters
                // need focus for one more read of the restored value.
                if !drag.number.blur && accessible && window.focused && focus.get().is_none() {
                    focus.set(drag.entity, FocusCause::Auto);
                }
                if let Some(scene) = &drag.scene
                    && same_scene
                {
                    editor.edits = scene.before.clone();
                    editor.sync_hierarchy_metadata();
                }
                keys.clear_just_pressed(KeyCode::Escape);
                scrub.suppressed = true;
            } else if same_scene
                && available
                && let Some(point) = point
            {
                let factor = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                    0.1
                } else if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
                    10.0
                } else {
                    1.0
                };
                let value = drag.number.constrain(
                    drag.value + (point.x - drag.previous.x) as f64 * drag.number.speed * factor,
                    drag.value,
                );
                let text = drag.number.text(value, drag.original.contains(','));
                let accepted = drag.scene.as_ref().is_none_or(|scene| {
                    let draft = super::Draft {
                        room: scene.room,
                        selection: scene.selection,
                        targets: Vec::new(),
                        local_space: scene.local,
                        field: scene.field,
                        original_text: String::new(),
                        changed: true,
                        error: None,
                    };
                    super::parse_value(&editor, &draft, &text).is_ok()
                });
                if accepted {
                    drag.value = value;
                    if let Some(scene) = &drag.scene {
                        editor.set_inspector_field(
                            scene.field,
                            text.replace(',', ".").parse().unwrap(),
                        );
                    }
                    input.editor_mut().set_text(&text);
                }
                drag.previous = point;
            }
            if cancel || finish || !same_scene {
                if !cancel
                    && same_scene
                    && let Some(scene) = drag.scene.take()
                {
                    editor.finish_edit(scene.before);
                    input
                        .editor_mut()
                        .set_text(&display_value(&editor, scene.field));
                }
                commands.entity(drag.entity).insert(drag.mode);
                input.queue_edit(TextEdit::SelectAll);
                if (drag.number.blur || !accessible || !window.focused)
                    && focus.get() == Some(drag.entity)
                {
                    focus.clear();
                }
            } else {
                scrub.drag = Some(drag);
            }
        }
    } else if !finish {
        scrub.drag = Some(drag);
    }
}

pub(in crate::editor) fn lock_text(scrub: Res<Scrub>, mut inputs: Query<&mut TextReadWriteMode>) {
    if let Some(drag) = scrub.drag.as_ref().filter(|drag| drag.moving)
        && let Ok(mut mode) = inputs.get_mut(drag.entity)
    {
        *mode = TextReadWriteMode::Static;
    }
}

/// Run after every UI cursor owner and before the native cursor update.
pub(in crate::editor) fn cursor(
    mut commands: Commands,
    mut scrub: ResMut<Scrub>,
    window: Single<(Entity, Option<&CursorIcon>), With<PrimaryWindow>>,
) {
    let (window, current) = *window;
    let resize = CursorIcon::from(SystemCursorIcon::EwResize);
    if scrub.drag.as_ref().is_some_and(|drag| drag.moving) {
        if !scrub.cursor_owned {
            scrub.previous_cursor = current.cloned();
            scrub.cursor_owned = true;
        }
        if current != Some(&resize) {
            commands.entity(window).insert(resize);
        }
    } else if scrub.cursor_owned {
        scrub.cursor_owned = false;
        let previous = scrub.previous_cursor.take();
        if current == Some(&resize) {
            if let Some(previous) = previous {
                commands.entity(window).insert(previous);
            } else {
                commands.entity(window).remove::<CursorIcon>();
            }
        }
    }
}
