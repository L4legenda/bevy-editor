//! Commit name edits after a short pause, with immediate flushes on save/leave.
use super::*;
use bevy::window::WindowCloseRequested;
use std::collections::HashMap;

struct Draft {
    title: String,
    due: f64,
    attempted: bool,
    error: Option<String>,
}

#[derive(Resource, Default)]
pub(super) struct State {
    drafts: HashMap<usize, Draft>,
}

#[derive(Component)]
pub(super) struct ErrorLabel;

pub(super) fn error_label(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    let label = wrapped(commands, parent, font, "", 11.0, ERROR);
    commands.entity(label).insert(ErrorLabel);
    commands
        .entity(label)
        .entry::<Node>()
        .and_modify(|mut n| n.display = Display::None);
}

pub(super) fn commit(
    editor: &mut Editor,
    browser: &mut Browser,
    browser_ui: &mut drawer::BrowserUi,
    state: &mut State,
    id: usize,
    title: &str,
) -> bool {
    let root = editor.root.clone();
    match editor.narrative.rename(&root, id, title) {
        Ok(change) => {
            state.drafts.remove(&id);
            if let Some((from, to)) = change {
                if !browser.rename_narrative(&root, &from, &to) {
                    browser.refresh(&mut editor.project);
                    browser_ui.select(None);
                }
                editor.status = format!("Переименовано · {}", to.display());
            }
            true
        }
        Err(error) => {
            editor.status = format!("Не удалось переименовать файл · {error}");
            state.drafts.insert(
                id,
                Draft {
                    title: title.to_owned(),
                    due: 0.0,
                    attempted: true,
                    error: Some(error),
                },
            );
            false
        }
    }
}

pub(super) fn current(
    editor: &mut Editor,
    browser: &mut Browser,
    browser_ui: &mut drawer::BrowserUi,
    state: &mut State,
    id: usize,
) -> bool {
    let Some(title) = editor
        .narrative
        .get(id)
        .map(|d| d.content.title().to_owned())
    else {
        return false;
    };
    commit(editor, browser, browser_ui, state, id, &title)
}

pub(super) fn discard(
    editor: &mut Editor,
    browser: &mut Browser,
    browser_ui: &mut drawer::BrowserUi,
    state: &mut State,
    id: usize,
) -> bool {
    let Some(doc) = editor.narrative.get(id) else {
        return true;
    };
    if doc.filename_title == doc.saved.title() {
        state.drafts.remove(&id);
        return true;
    }
    let title = doc.saved.title().to_owned();
    commit(editor, browser, browser_ui, state, id, &title)
}

pub(super) fn finish(
    editor: &mut Editor,
    browser: &mut Browser,
    browser_ui: &mut drawer::BrowserUi,
    state: &mut State,
    id: usize,
) -> bool {
    let Some(doc) = editor.narrative.get(id) else {
        return true;
    };
    if doc.filename_title == doc.content.title() {
        return true;
    }
    current(editor, browser, browser_ui, state, id)
}

pub(super) fn sync(
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    mut browser_ui: ResMut<drawer::BrowserUi>,
    mut state: ResMut<State>,
    time: Res<Time<Real>>,
    focus: Res<InputFocus>,
    kinds: Query<&DraftField>,
    mut close: MessageReader<WindowCloseRequested>,
    mut labels: Query<(&mut Text, &mut Node), With<ErrorLabel>>,
    mut fields: Query<(Entity, &DraftField, &mut BorderColor)>,
) {
    let closing = close.read().last().is_some();
    state
        .drafts
        .retain(|id, _| editor.narrative.get(*id).is_some());
    if editor.pending.is_none() {
        let now = time.elapsed_secs_f64();
        let typing = focus
            .get()
            .is_some_and(|e| kinds.get(e).is_ok_and(|kind| *kind == DraftField::Title));
        let mut ready = Vec::new();
        let mut recovered = None;
        for doc in &editor.narrative.tabs {
            let title = doc.content.title();
            if title == doc.filename_title {
                if state
                    .drafts
                    .remove(&doc.id)
                    .is_some_and(|draft| draft.error.is_some())
                {
                    recovered = Some(doc.path.clone());
                }
                continue;
            }
            let draft = state.drafts.entry(doc.id).or_insert_with(|| Draft {
                title: title.to_owned(),
                due: now + 0.4,
                attempted: false,
                error: None,
            });
            if draft.title != title {
                *draft = Draft {
                    title: title.to_owned(),
                    due: now + 0.4,
                    attempted: false,
                    error: None,
                };
            }
            if !draft.attempted
                && (closing
                    || editor.narrative.active != Some(doc.id)
                    || !typing
                    || now >= draft.due)
            {
                ready.push((doc.id, title.to_owned()));
            }
        }
        if let Some(path) = recovered {
            if editor.status.starts_with("Не удалось переименовать") {
                editor.status = format!("Имя файла · {}", path.display());
            }
        }
        for (id, title) in ready {
            commit(
                &mut editor,
                &mut browser,
                &mut browser_ui,
                &mut state,
                id,
                &title,
            );
        }
    }
    let error = editor
        .narrative
        .active
        .and_then(|id| state.drafts.get(&id))
        .and_then(|d| d.error.as_deref());
    for (mut text, mut node) in &mut labels {
        text.set_if_neq(Text(error.unwrap_or_default().to_owned()));
        node.display = if error.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (entity, kind, mut border) in &mut fields {
        if *kind == DraftField::Title {
            border.set_if_neq(BorderColor::all(if error.is_some() {
                ERROR
            } else if focus.get() == Some(entity) {
                FOCUS
            } else {
                EDGE
            }));
        }
    }
}
