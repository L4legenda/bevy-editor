//! The image editor's colour dialog, with a separate transactional text draft.
use super::*;
use crate::editor::ui::image_viewer::edit::{self as image_edit, ColorComponent, EditAction};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum PickerAction {
    Confirm,
    Cancel,
    Component(ColorComponent, f32),
    Draft([f32; 4]),
    Sv(f32, f32),
    Swatch,
}
impl PickerAction {
    pub fn from_image(action: EditAction) -> Self {
        match action {
            EditAction::ColorConfirm => Self::Confirm,
            EditAction::ColorCancel => Self::Cancel,
            EditAction::ColorComponent(c, v) => Self::Component(c, v),
            EditAction::ColorDraft(c) => Self::Draft(c),
            EditAction::ColorSv(s, v) => Self::Sv(s, v),
            EditAction::AddSwatch => Self::Swatch,
            _ => unreachable!("shared colour dialog emitted a non-colour action"),
        }
    }
}
pub(super) fn apply_action(session: &mut Session, action: PickerAction, now: f64) {
    let Some(picker) = session.picker.as_mut() else {
        return;
    };
    match action {
        PickerAction::Cancel => session.picker = None,
        PickerAction::Confirm if picker.error.is_none() => {
            let rgb = picker.draft.map(|c| (c * 255.).round() as u8);
            let mut next = session.draft.clone();
            next.selection = session.color_selection.clone();
            next.paint(|f| f.color = Some([rgb[0], rgb[1], rgb[2]]));
            session.edit(next, None, now);
            session.picker = None;
            session.rewrite_fields = true;
        }
        PickerAction::Component(c, v) => picker.set_component(c, v),
        PickerAction::Draft(c) => picker.set_color(c),
        PickerAction::Sv(s, v) => picker.sv(s, v),
        PickerAction::Swatch => {
            if !session.swatches.contains(&picker.draft) {
                session.swatches.push(picker.draft);
            }
        }
        _ => {}
    }
}

pub(in crate::editor) fn input(
    mut state: ResMut<State>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform)>,
    inputs: Query<&EditableText>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
) {
    let State {
        session, color_ui, ..
    } = &mut *state;
    let picker = session.as_mut().and_then(|s| s.picker.as_mut());
    let mut events = Vec::new();
    image_edit::picker_ui::input_shared(
        picker,
        color_ui,
        &window,
        &mouse,
        &keys,
        &nodes,
        &inputs,
        &mut focus,
        &mut events,
    );
    for event in events {
        actions.write(Action::Narrative(NarrativeAction::Text(
            TextAction::Picker(PickerAction::from_image(event)),
        )));
    }
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    mut state: ResMut<State>,
    mut focus: ResMut<InputFocus>,
    mut nodes: Query<&mut Node>,
    mut colors: Query<&mut BackgroundColor>,
    mut labels: Query<&mut Text>,
    mut pictures: Query<&mut ImageNode>,
    mut inputs: Query<&mut EditableText>,
    mut images: ResMut<Assets<Image>>,
) {
    let State {
        session, color_ui, ..
    } = &mut *state;
    let picker = session.as_ref().and_then(|s| s.picker.as_ref());
    let swatches = session.as_ref().map_or(&[][..], |s| s.swatches.as_slice());
    image_edit::picker_ui::refresh_shared(
        &mut commands,
        swatches,
        picker,
        color_ui,
        &mut focus,
        &mut nodes,
        &mut colors,
        &mut labels,
        &mut pictures,
        &mut inputs,
        &mut images,
    );
}
