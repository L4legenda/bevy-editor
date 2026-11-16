//! A modal draft editor. Apply commits text and formatting as one document edit.
use super::*;
use crate::editor::dialogue::text as rich;
use bevy::text::{FontCx, LayoutCx, LineHeight, Strikethrough, TextEdit, Underline};
use std::ops::Range;
#[path = "narrative_text_color.rs"]
pub(in crate::editor) mod color_menu;
#[path = "narrative_text_ui.rs"]
mod view;
#[cfg(test)]
pub(super) use view::NativeGlyphs;
pub(super) use view::{decorate, hide_native_ink, refresh, setup, style};
#[cfg(test)]
#[path = "narrative_text_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum TextAction {
    Apply,
    Cancel,
    Close,
    KeepEditing,
    Discard,
    Undo,
    Redo,
    Size(i8),
    Bold,
    Italic,
    Underline,
    Strike,
    Align(TextAlign),
    Spacing,
    ColorMenu,
    Picker(color_menu::PickerAction),
    Reset,
    SelectAll,
    Cut,
    Copy,
    Paste,
    Find,
    HideFind,
    Next(bool),
    MatchCase,
    Replace,
    ReplaceAll,
    Bullets,
    Numbered,
    Clean,
    Preview,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum Input {
    Body,
    Size,
    Find,
    Replace,
}
#[derive(Component)]
pub(in crate::editor) struct Tool(TextAction);
#[derive(Component)]
pub(in crate::editor) struct Summary;
#[derive(Component)]
pub(in crate::editor) struct FindSummary;
#[derive(Component)]
pub(in crate::editor) struct Notice;
#[derive(Component)]
pub(in crate::editor) struct PreviewText;

#[derive(Clone, PartialEq, Debug)]
struct Draft {
    text: String,
    format: TextFormat,
    spans: Vec<rich::TextSpan>,
    selection: Range<usize>,
}
impl Draft {
    fn paint(&mut self, change: impl FnMut(&mut TextFormat)) {
        if self.selection.is_empty() {
            return;
        }
        self.spans = rich::apply(
            &self.text,
            &self.format,
            &self.spans,
            self.selection.clone(),
            change,
        );
    }
    fn selected_format(&self) -> TextFormat {
        rich::runs(&self.text, &self.format, &self.spans)
            .into_iter()
            .find(|s| s.start <= self.selection.start && self.selection.start < s.end)
            .map_or_else(|| self.format.character(), |s| s.format)
    }
    fn all_selected(&self, test: impl Fn(&TextFormat) -> bool) -> bool {
        !self.selection.is_empty()
            && rich::runs(&self.text, &self.format, &self.spans)
                .iter()
                .filter(|s| s.start < self.selection.end && s.end > self.selection.start)
                .all(|s| test(&s.format))
    }
    fn replace(&mut self, range: Range<usize>, value: &str) {
        rich::replace(&mut self.text, &self.format, &mut self.spans, range, value);
    }
    fn same_content(&self, other: &Self) -> bool {
        self.text == other.text && self.format == other.format && self.spans == other.spans
    }
}

struct Session {
    document: usize,
    node: u32,
    original: Draft,
    draft: Draft,
    undo: Vec<Draft>,
    redo: Vec<Draft>,
    group: Option<(Input, f64)>,
    focused: Option<Entity>,
    search: String,
    replacement: String,
    match_case: bool,
    find_open: bool,
    preview: bool,
    confirm: bool,
    exit: Option<Entity>,
    error: String,
    size_valid: bool,
    rewrite_fields: bool,
    format_fields: bool,
    size_text: String,
    picker: Option<crate::editor::ui::image_viewer::edit::color::Picker>,
    color_selection: Range<usize>,
    swatches: Vec<[f32; 4]>,
}
impl Session {
    fn new(document: usize, node: &DialogueNode) -> Self {
        let draft = Draft {
            text: node.text.clone(),
            format: node.text_format.normalized(),
            spans: rich::compact(&node.text, &node.text_format, &node.text_spans),
            selection: node.text.len()..node.text.len(),
        };
        Self {
            document,
            node: node.id,
            original: draft.clone(),
            draft,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
            focused: None,
            search: String::new(),
            replacement: String::new(),
            match_case: false,
            find_open: false,
            preview: false,
            confirm: false,
            exit: None,
            error: String::new(),
            size_valid: true,
            rewrite_fields: true,
            format_fields: true,
            size_text: node.text_format.normalized().font_size.to_string(),
            picker: None,
            color_selection: 0..0,
            swatches: crate::editor::ui::image_viewer::edit::color::PRESETS.to_vec(),
        }
    }

    fn dirty(&self) -> bool {
        !self.draft.same_content(&self.original)
    }

    fn edit(&mut self, next: Draft, group: Option<Input>, now: f64) {
        if next.same_content(&self.draft) {
            self.draft.selection = next.selection;
            return;
        }
        let coalesce = group.is_some_and(|kind| {
            self.group
                .is_some_and(|(previous, at)| previous == kind && now - at < 1.0)
        });
        if !coalesce {
            if self.undo.len() == 128 {
                self.undo.remove(0);
            }
            self.undo.push(self.draft.clone());
        }
        self.redo.clear();
        self.group = group.map(|kind| (kind, now));
        self.draft = next;
        self.error.clear();
    }

    fn history(&mut self, redo: bool) {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(previous) = from.pop() {
            to.push(std::mem::replace(&mut self.draft, previous));
            self.group = None;
            self.rewrite_fields = true;
            self.size_valid = true;
            self.error.clear();
        }
    }
}

#[derive(Resource)]
pub(in crate::editor) struct State {
    root: Entity,
    panel: Entity,
    session: Option<Session>,
    built: Option<(usize, u32)>,
    body: Option<Entity>,
    find_row: Option<Entity>,
    confirm_row: Option<Entity>,
    preview_root: Option<Entity>,
    restore: Option<(usize, u32)>,
    color_ui: crate::editor::ui::image_viewer::edit::picker_ui::PickerUi,
    bold: Handle<Font>,
    italic: Handle<Font>,
    bold_italic: Handle<Font>,
}
impl State {
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    pub(super) fn open(&mut self, document: usize, node: &DialogueNode) {
        self.session = Some(Session::new(document, node));
        self.built = None;
    }

    fn finish(&mut self, ui: &mut NarrativeUi) -> Option<Entity> {
        let session = self.session.take()?;
        self.restore = Some((session.document, session.node));
        ui.picker = None;
        ui.last_edit = None;
        session.exit
    }
}

fn matches(text: &str, query: &str, case: bool) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if case {
        return text
            .match_indices(query)
            .map(|(at, value)| at..at + value.len())
            .collect();
    }
    let mut folded = String::new();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for (at, ch) in text.char_indices() {
        let lower: String = ch.to_lowercase().collect();
        starts.extend(std::iter::repeat_n(at, lower.len()));
        ends.extend(std::iter::repeat_n(at + ch.len_utf8(), lower.len()));
        folded.push_str(&lower);
    }
    let query = query.to_lowercase();
    let mut result = Vec::new();
    for (at, value) in folded.match_indices(&query) {
        let range = starts[at]..ends[at + value.len() - 1];
        if result.last().is_none_or(|previous| previous != &range) {
            result.push(range);
        }
    }
    result
}

fn hex(format: &TextFormat) -> String {
    let c = color(format).to_srgba();
    format!(
        "#{:02X}{:02X}{:02X}",
        (c.red * 255.0).round() as u8,
        (c.green * 255.0).round() as u8,
        (c.blue * 255.0).round() as u8
    )
}

fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn update(
    session: &mut Session,
    entity: Entity,
    kind: Input,
    input: &EditableText,
    focus: Option<Entity>,
    now: f64,
) {
    let value = input.value().to_string();
    let mut next = session.draft.clone();
    match kind {
        Input::Body => {
            if !input.editor.is_composing() && focus == Some(entity) {
                let range = input.editor.raw_selection().text_range();
                next.selection = boundary(&value, range.start)..boundary(&value, range.end);
            }
            if value == next.text {
                if next.selection != session.draft.selection {
                    session.group = None;
                    session.format_fields = true;
                }
                session.draft.selection = next.selection;
                return;
            }
            next.spans = rich::remap(&next.text, &value, &next.format, &next.spans);
            next.text = value;
            next.selection = boundary(&next.text, next.selection.start)
                ..boundary(&next.text, next.selection.end);
        }
        Input::Size if focus == Some(entity) && value != session.size_text => {
            session.size_text = value.clone();
            let size = value
                .trim()
                .replace(',', ".")
                .parse::<f32>()
                .ok()
                .filter(|size| size.is_finite() && (8.0..=72.0).contains(size));
            session.size_valid = size.is_some();
            let Some(size) = size else {
                return;
            };
            session.error.clear();
            next.paint(|f| f.font_size = size);
        }
        Input::Find => {
            session.search = value;
            return;
        }
        Input::Replace => {
            session.replacement = value;
            return;
        }
        _ => return,
    }
    session.edit(next, Some(kind), now);
}

pub(super) fn sync(
    mut state: ResMut<State>,
    fields: Query<(Entity, &Input, &EditableText)>,
    focus: Res<InputFocus>,
    time: Res<Time>,
) {
    if state
        .session
        .as_ref()
        .is_none_or(|session| state.built != Some((session.document, session.node)))
    {
        return;
    }
    let Some(session) = state.session.as_mut() else {
        return;
    };
    if session.focused != focus.get() {
        session.focused = focus.get();
        session.group = None;
    }
    for (entity, kind, input) in &fields {
        update(
            session,
            entity,
            *kind,
            input,
            focus.get(),
            time.elapsed_secs_f64(),
        );
    }
}

pub(super) fn keyboard(
    state: Res<State>,
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    fields: Query<&Input>,
    mut messages: MessageWriter<Action>,
) {
    if !state.is_open() || state.session.as_ref().is_some_and(|s| s.picker.is_some()) {
        return;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let typing = focus.get().and_then(|e| fields.get(e).ok());
    let action = if command && keys.just_pressed(KeyCode::Enter) {
        Some(TextAction::Apply)
    } else if command && keys.just_pressed(KeyCode::KeyB) {
        Some(TextAction::Bold)
    } else if command && keys.just_pressed(KeyCode::KeyI) {
        Some(TextAction::Italic)
    } else if command && keys.just_pressed(KeyCode::KeyU) {
        Some(TextAction::Underline)
    } else if command && keys.just_pressed(KeyCode::KeyF) {
        Some(TextAction::Find)
    } else if command && keys.just_pressed(KeyCode::KeyH) {
        Some(TextAction::Find)
    } else if command && typing.is_some() && keys.just_pressed(KeyCode::KeyZ) {
        Some(if shift {
            TextAction::Redo
        } else {
            TextAction::Undo
        })
    } else if command && typing.is_some() && keys.just_pressed(KeyCode::KeyY) {
        Some(TextAction::Redo)
    } else if keys.just_pressed(KeyCode::F3)
        || (!command && typing == Some(&Input::Find) && keys.just_pressed(KeyCode::Enter))
    {
        Some(TextAction::Next(shift))
    } else {
        None
    };
    if let Some(action) = action {
        messages.write(Action::Narrative(NarrativeAction::Text(action)));
    }
}

fn focus_body(state: &State, focus: &mut InputFocus) {
    if let Some(body) = state.body {
        focus.set(body, bevy::input_focus::FocusCause::Auto);
    }
}

fn reveal(input: &mut EditableText) {
    if let Some(caret) = input.editor.cursor_geometry(1.5 * input.editor.get_scale()) {
        input.queue_edit(TextEdit::ScrollTo(Vec2::new(
            caret.x0 as f32,
            caret.y0 as f32,
        )));
        input.queue_edit(TextEdit::ScrollTo(Vec2::new(
            caret.x1 as f32,
            caret.y1 as f32,
        )));
    }
}

pub(super) fn actions(
    mut commands: Commands,
    mut messages: MessageReader<Action>,
    mut closing: MessageReader<bevy::window::WindowCloseRequested>,
    mut editor: ResMut<Editor>,
    mut ui: ResMut<NarrativeUi>,
    mut state: ResMut<State>,
    mut focus: ResMut<InputFocus>,
    mut fields: Query<(Entity, &Input, &mut EditableText)>,
    mut fonts: ResMut<FontCx>,
    mut layouts: ResMut<LayoutCx>,
    mut clipboard: ResMut<bevy::clipboard::Clipboard>,
    time: Res<Time>,
    windows: Query<Entity, With<PrimaryWindow>>,
) {
    let requested_exit = closing.read().last().map(|event| event.window);
    let now = time.elapsed_secs_f64();
    let events: Vec<_> = messages.read().copied().collect();
    let save_file = events
        .iter()
        .any(|event| matches!(event, Action::Narrative(NarrativeAction::Save)));
    let mut queue: Vec<TextAction> = events
        .iter()
        .filter_map(|message| match *message {
            Action::Narrative(NarrativeAction::Text(action)) => Some(action),
            Action::Narrative(NarrativeAction::Dismiss | NarrativeAction::CloseCurrent) => {
                Some(TextAction::Close)
            }
            Action::Narrative(NarrativeAction::Save) => Some(TextAction::Apply),
            Action::Narrative(NarrativeAction::Undo) => Some(TextAction::Undo),
            Action::Narrative(NarrativeAction::Redo) => Some(TextAction::Redo),
            _ => None,
        })
        .collect();
    if state.session.is_none() {
        return;
    }
    if requested_exit.is_some() || editor.exit_requested {
        editor.exit_requested = false;
        state.session.as_mut().unwrap().exit = requested_exit.or_else(|| windows.single().ok());
        state.session.as_mut().unwrap().picker = None;
        queue.push(TextAction::Close);
    }
    // Flush queued native edits before commands such as Apply, Undo or Copy.
    if !queue.is_empty()
        && state
            .session
            .as_ref()
            .is_some_and(|session| state.built == Some((session.document, session.node)))
    {
        for (entity, kind, mut input) in &mut fields {
            if !input.pending_edits.is_empty() || input.pending_paste.is_some() {
                input.apply_pending_edits(&mut fonts, &mut layouts.0, &mut clipboard, |_| true);
            }
            if *kind == Input::Body
                && queue.contains(&TextAction::Apply)
                && input.editor.is_composing()
            {
                input
                    .editor
                    .driver(&mut fonts, &mut layouts.0)
                    .finish_compose();
            }
            update(
                state.session.as_mut().unwrap(),
                entity,
                *kind,
                &input,
                focus.get(),
                now,
            );
        }
    }
    for action in queue {
        let Some(session) = state.session.as_mut() else {
            break;
        };
        if session.picker.is_some() {
            match action {
                TextAction::Picker(a) => color_menu::apply_action(session, a, now),
                TextAction::Close | TextAction::Cancel => {
                    session.picker = None;
                    focus_body(&state, &mut focus);
                }
                _ => {}
            }
            continue;
        }
        match action {
            TextAction::ColorMenu => {
                if session.draft.selection.is_empty() {
                    continue;
                }
                let c = color(&session.draft.selected_format()).to_srgba();
                session.color_selection = session.draft.selection.clone();
                session.picker = Some(crate::editor::ui::image_viewer::edit::color::Picker::new(
                    crate::editor::ui::image_viewer::edit::ColorTarget::Foreground,
                    [c.red, c.green, c.blue, 1.],
                ));
            }
            TextAction::Picker(_) => {}
            TextAction::Apply => {
                if !session.size_valid {
                    session.error = "Размер шрифта: от 8 до 72.".into();
                    let kind = Input::Size;
                    if let Some((entity, _, _)) =
                        fields.iter().find(|(_, field, _)| **field == kind)
                    {
                        focus.set(entity, bevy::input_focus::FocusCause::Auto);
                    }
                    continue;
                }
                let Some(doc) = editor
                    .narrative
                    .current_mut()
                    .filter(|d| d.id == session.document)
                else {
                    session.error = "Документ больше не открыт.".into();
                    continue;
                };
                let Content::Dialogue(dialogue) = &doc.content else {
                    continue;
                };
                let Some(node) = dialogue.node(session.node) else {
                    session.error = "Реплика была удалена.".into();
                    continue;
                };
                if node.text != session.original.text
                    || node.text_format.normalized() != session.original.format
                    || rich::compact(&node.text, &node.text_format, &node.text_spans)
                        != session.original.spans
                {
                    session.error = "Реплика изменилась вне этого окна. Скопируйте черновик и откройте редактор снова.".into();
                    continue;
                }
                if session.dirty() {
                    doc.checkpoint();
                    if let Content::Dialogue(dialogue) = &mut doc.content {
                        let node = dialogue.node_mut(session.node).unwrap();
                        node.text = session.draft.text.clone();
                        node.text_format = session.draft.format.clone();
                        node.text_spans = session.draft.spans.clone();
                    }
                    ui.generation += 1;
                }
                let exit = state.finish(&mut ui);
                if save_file {
                    commands.queue(|world: &mut World| {
                        world.write_message(Action::Narrative(NarrativeAction::Save));
                    });
                }
                request_exit(&mut commands, exit);
                focus.clear();
            }
            TextAction::Cancel | TextAction::Discard => {
                let exit = state.finish(&mut ui);
                request_exit(&mut commands, exit);
                focus.clear();
            }
            TextAction::Close => {
                if session.confirm && session.exit.is_none() {
                    session.confirm = false;
                } else if session.dirty() {
                    session.confirm = true;
                } else {
                    let exit = state.finish(&mut ui);
                    request_exit(&mut commands, exit);
                    focus.clear();
                }
            }
            TextAction::KeepEditing => {
                session.confirm = false;
                session.exit = None;
                focus_body(&state, &mut focus);
            }
            TextAction::Undo | TextAction::Redo => {
                session.history(action == TextAction::Redo);
                focus_body(&state, &mut focus);
            }
            TextAction::Find => {
                session.find_open = true;
                if let Some((entity, _, _)) =
                    fields.iter().find(|(_, kind, _)| **kind == Input::Find)
                {
                    focus.set(entity, bevy::input_focus::FocusCause::Auto);
                }
            }
            TextAction::HideFind => {
                session.find_open = false;
                focus_body(&state, &mut focus);
            }
            TextAction::MatchCase => {
                session.match_case = !session.match_case;
            }
            TextAction::Preview => {
                session.preview = !session.preview;
            }
            TextAction::SelectAll | TextAction::Copy | TextAction::Cut | TextAction::Paste => {
                let range = session.draft.selection.clone();
                if let Some((_, _, mut input)) =
                    fields.iter_mut().find(|(_, kind, _)| **kind == Input::Body)
                {
                    input
                        .editor
                        .driver(&mut fonts, &mut layouts.0)
                        .select_byte_range(range.start, range.end);
                    reveal(&mut input);
                    input.queue_edit(match action {
                        TextAction::SelectAll => TextEdit::SelectAll,
                        TextAction::Copy => TextEdit::Copy,
                        TextAction::Cut => TextEdit::Cut,
                        _ => TextEdit::Paste,
                    });
                }
                session.preview = false;
                focus_body(&state, &mut focus);
            }
            TextAction::Next(previous) => {
                let found = matches(&session.draft.text, &session.search, session.match_case);
                let range = if previous {
                    found
                        .iter()
                        .rev()
                        .find(|r| r.end <= session.draft.selection.start)
                        .or(found.last())
                } else {
                    found
                        .iter()
                        .find(|r| r.start >= session.draft.selection.end)
                        .or(found.first())
                };
                if let Some(range) = range {
                    session.draft.selection = range.clone();
                    session.format_fields = true;
                    session.preview = false;
                    if let Some((_, _, mut input)) =
                        fields.iter_mut().find(|(_, kind, _)| **kind == Input::Body)
                    {
                        input
                            .editor
                            .driver(&mut fonts, &mut layouts.0)
                            .select_byte_range(range.start, range.end);
                        reveal(&mut input);
                    }
                    focus_body(&state, &mut focus);
                }
            }
            TextAction::Replace | TextAction::ReplaceAll => {
                let found = matches(&session.draft.text, &session.search, session.match_case);
                let mut next = session.draft.clone();
                if action == TextAction::ReplaceAll {
                    for range in found.iter().rev() {
                        next.replace(range.clone(), &session.replacement);
                    }
                    next.selection = 0..0;
                } else if let Some(range) = found
                    .iter()
                    .find(|r| **r == next.selection)
                    .or_else(|| found.iter().find(|r| r.start >= next.selection.start))
                    .or(found.first())
                {
                    next.replace(range.clone(), &session.replacement);
                    let end = range.start + session.replacement.len();
                    next.selection = end..end;
                }
                session.edit(next, None, now);
                session.rewrite_fields = true;
                session.preview = false;
                focus_body(&state, &mut focus);
            }
            _ => {
                let mut next = session.draft.clone();
                match action {
                    TextAction::Size(delta) => {
                        let size =
                            (next.selected_format().font_size + delta as f32).clamp(8.0, 72.0);
                        next.paint(|f| f.font_size = size)
                    }
                    TextAction::Bold => {
                        let enabled = !next.all_selected(|f| f.bold);
                        next.paint(|f| f.bold = enabled);
                    }
                    TextAction::Italic => {
                        let enabled = !next.all_selected(|f| f.italic);
                        next.paint(|f| f.italic = enabled);
                    }
                    TextAction::Underline => {
                        let enabled = !next.all_selected(|f| f.underline);
                        next.paint(|f| f.underline = enabled);
                    }
                    TextAction::Strike => {
                        let enabled = !next.all_selected(|f| f.strikethrough);
                        next.paint(|f| f.strikethrough = enabled);
                    }
                    TextAction::Align(alignment) => next.format.alignment = alignment,
                    TextAction::Spacing => {
                        next.format.line_height = if next.format.line_height < 1.4 {
                            1.5
                        } else if next.format.line_height < 1.9 {
                            2.0
                        } else {
                            1.2
                        }
                    }
                    TextAction::Reset => next.paint(|f| *f = TextFormat::default()),
                    TextAction::Bullets | TextAction::Numbered => {
                        next = toggle_list(&next, action == TextAction::Numbered)
                    }
                    TextAction::Clean => {
                        let mut offset = 0;
                        let mut ranges = Vec::new();
                        for line in next.text.split('\n') {
                            if line.trim_end().len() != line.len() {
                                ranges.push(offset + line.trim_end().len()..offset + line.len());
                            }
                            offset += line.len() + 1;
                        }
                        for range in ranges.into_iter().rev() {
                            next.replace(range, "");
                        }
                        next.selection = next.text.len()..next.text.len();
                    }
                    _ => {}
                }
                session.edit(next, None, now);
                session.rewrite_fields = true;
                session.size_valid = true;
                focus_body(&state, &mut focus);
            }
        }
    }
}

fn request_exit(commands: &mut Commands, window: Option<Entity>) {
    if let Some(window) = window {
        commands.queue(move |world: &mut World| {
            world.write_message(bevy::window::WindowCloseRequested { window });
        });
    }
}

fn color(format: &TextFormat) -> Color {
    format
        .color
        .map_or(TEXT, |rgb| Color::srgb_u8(rgb[0], rgb[1], rgb[2]))
}
fn alignment(format: &TextFormat) -> TextLayout {
    TextLayout::new(
        match format.alignment {
            TextAlign::Left => bevy::text::Justify::Left,
            TextAlign::Center => bevy::text::Justify::Center,
            TextAlign::Right => bevy::text::Justify::Right,
            TextAlign::Justified => bevy::text::Justify::Justified,
        },
        bevy::text::LineBreak::WordOrCharacter,
    )
}

fn formatted_font(state: &State, regular: &Handle<Font>, format: &TextFormat) -> TextFont {
    let face = match (format.bold, format.italic) {
        (true, true) => &state.bold_italic,
        (true, false) => &state.bold,
        (false, true) => &state.italic,
        _ => regular,
    };
    let mut result = font(face, format.font_size);
    result.weight = if format.bold {
        bevy::text::FontWeight::BOLD
    } else {
        bevy::text::FontWeight::NORMAL
    };
    result.style = if format.italic {
        bevy::text::FontStyle::Italic
    } else {
        bevy::text::FontStyle::Normal
    };
    result
}

pub(super) fn render(
    commands: &mut Commands,
    parent: Entity,
    regular: &Handle<Font>,
    state: &State,
    value: &str,
    format: &TextFormat,
    spans: &[rich::TextSpan],
) -> Entity {
    render_parts(commands, parent, regular, state, value, format, spans).0
}

fn render_parts(
    commands: &mut Commands,
    parent: Entity,
    regular: &Handle<Font>,
    state: &State,
    value: &str,
    format: &TextFormat,
    spans: &[rich::TextSpan],
) -> (Entity, Vec<Entity>) {
    let format = format.normalized();
    let entity = commands
        .spawn((
            Text::new(""),
            formatted_font(state, regular, &format),
            TextColor(color(&format)),
            LineHeight::RelativeToFont(format.line_height),
            alignment(&format),
            Node {
                width: percent(100),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    let spans = render_spans(commands, entity, regular, state, value, &format, spans);
    (entity, spans)
}

fn render_spans(
    commands: &mut Commands,
    entity: Entity,
    regular: &Handle<Font>,
    state: &State,
    value: &str,
    format: &TextFormat,
    spans: &[rich::TextSpan],
) -> Vec<Entity> {
    let mut entities = Vec::new();
    for run in rich::runs(value, format, spans) {
        let span = commands
            .spawn((
                bevy::text::TextSpan::new(&value[run.start..run.end]),
                formatted_font(state, regular, &run.format),
                TextColor(color(&run.format)),
                LineHeight::RelativeToFont(format.line_height),
                Pickable::IGNORE,
                ChildOf(entity),
            ))
            .id();
        decorations(commands, span, &run.format);
        entities.push(span);
    }
    entities
}

fn decorations(commands: &mut Commands, entity: Entity, format: &TextFormat) {
    if format.underline {
        commands.entity(entity).insert(Underline);
    } else {
        commands.entity(entity).remove::<Underline>();
    }
    if format.strikethrough {
        commands.entity(entity).insert(Strikethrough);
    } else {
        commands.entity(entity).remove::<Strikethrough>();
    }
}

fn toggle_list(draft: &Draft, numbered: bool) -> Draft {
    let start = draft.text[..draft.selection.start]
        .rfind('\n')
        .map_or(0, |at| at + 1);
    let mut end = draft.selection.end;
    if end > draft.selection.start && draft.text[..end].ends_with('\n') {
        end -= 1;
    }
    end += draft.text[end..]
        .find('\n')
        .unwrap_or(draft.text.len() - end);
    let strip = |line: &str| -> String {
        if let Some(rest) = line.strip_prefix("• ") {
            return rest.into();
        }
        if let Some((prefix, rest)) = line.split_once(". ") {
            if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit()) {
                return rest.into();
            }
        }
        line.into()
    };
    let lines: Vec<_> = draft.text[start..end].split('\n').collect();
    let remove = lines.iter().all(|line| {
        if numbered {
            line.split_once(". ").is_some_and(|(prefix, _)| {
                !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit())
            })
        } else {
            line.starts_with("• ")
        }
    });
    let replacement = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = strip(line);
            if remove {
                line
            } else if numbered {
                format!("{}. {line}", i + 1)
            } else {
                format!("• {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut next = draft.clone();
    let mut at = start;
    let mut edits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let clean = strip(line);
        let prefix_len = line.len() - clean.len();
        let prefix = if remove {
            String::new()
        } else if numbered {
            format!("{}. ", i + 1)
        } else {
            "• ".into()
        };
        edits.push((at..at + prefix_len, prefix));
        at += line.len() + 1;
    }
    for (range, value) in edits.into_iter().rev() {
        next.replace(range, &value);
    }
    next.selection = start..start + replacement.len();
    next
}
