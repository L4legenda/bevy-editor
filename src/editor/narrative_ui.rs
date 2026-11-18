//! Node canvas, character sheets and a playable conversation preview.
use super::*;
#[path = "narrative_answers.rs"]
mod answers;
#[path = "narrative_character.rs"]
mod character_sheet;
#[path = "narrative_connections.rs"]
mod connection_paint;
#[path = "narrative_drag.rs"]
mod dragging;
#[path = "narrative_effects.rs"]
mod effects;
#[path = "narrative_filenames.rs"]
mod filenames;
#[path = "narrative_links.rs"]
mod links;
#[path = "narrative_nodes.rs"]
mod nodes;
#[path = "narrative_picker_fx.rs"]
mod picker_fx;
#[path = "narrative_portraits.rs"]
mod portraits;
#[path = "narrative_speaker.rs"]
mod speaker_menu;
#[path = "narrative_text.rs"]
pub(in crate::editor) mod text_editor;
use crate::editor::{PendingAction, dialogue::*};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct CanvasMaterial {
    #[uniform(0)]
    view: Vec4,
}
impl UiMaterial for CanvasMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("narrative_canvas.wesl"))
                .with_source("embedded"),
        )
    }
}
#[derive(Resource)]
struct CanvasPaint(Handle<CanvasMaterial>);
impl FromWorld for CanvasPaint {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource_mut::<Assets<CanvasMaterial>>()
                .add(CanvasMaterial {
                    view: Vec4::new(0.0, 0.0, 1.0, 0.0),
                }),
        )
    }
}

const NODE_WIDTH: f32 = 360.0;
const NODE_RADIUS: f32 = 12.0;
const HEADER: f32 = 42.0;
const OUTPUT_HEIGHT: f32 = 56.0;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum NarrativeAction {
    Create(bool),
    File(usize),
    Open(usize),
    Close(usize),
    CloseCurrent,
    Save,
    Undo,
    Redo,
    Select(u32),
    Node(u32, nodes::NodeAction),
    Add(NodeKind),
    Delete,
    AddChoice,
    RemoveChoice(usize),
    Link(u32, usize),
    Connect(u32, usize, u32),
    Disconnect(usize),
    Target(u32),
    DialogueTarget(u32),
    FollowJump(usize, u32),
    SetDialogue(Option<usize>, u64),
    DropDialogue {
        document: usize,
        file: usize,
        scan_revision: u64,
        local: Vec2,
    },
    Speaker,
    SetSpeaker(Option<usize>),
    SetPortrait(usize),
    AddPortrait,
    PickImage(usize),
    RemovePortrait(usize),
    MainPortrait(usize),
    Fit,
    Zoom(f32),
    Preview,
    Advance(usize),
    Restart,
    Dismiss,
    EditText,
    Text(text_editor::TextAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
enum DraftField {
    Title,
    Text,
    Choice(usize),
    Description,
    PortraitLabel(usize),
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Picker {
    Speaker,
    Image,
    Portrait,
    Target,
    Dialogue,
    Text,
}

#[derive(Resource)]
pub(in crate::editor) struct NarrativeUi {
    root: Entity,
    toolbar: Entity,
    pub(super) canvas: Entity,
    graph: Entity,
    connections: Entity,
    link_hint: Entity,
    hint_key: String,
    overlay: Entity,
    modal: Entity,
    status: Entity,
    font: Handle<Font>,
    toolbar_key: Option<(usize, u64)>,
    character_scrolls: std::collections::HashMap<usize, Vec2>,
    graph_key: String,
    wire_key: String,
    modal_key: String,
    image_key: String,
    canvas_size: Vec2,
    generation: u64,
    last_edit: Option<Entity>,
    linking: Option<(u32, usize)>,
    picker: Option<Picker>,
    search: String,
    playing: Option<u32>,
    preview_dialogue: Option<(String, Dialogue)>,
    preview_error: Option<String>,
    preview: bool,
    jump_click: Option<(usize, u32, f64, Vec2)>,
    drag_node: Option<u32>,
    drag_started: bool,
    drag_anchor: Option<dragging::Anchor>,
    panning: bool,
}

impl NarrativeUi {
    pub(super) fn accepts_dialogue_drop(
        &self,
        doc: &Document,
        file: &crate::editor::browser::BrowserFile,
    ) -> bool {
        self.picker.is_none()
            && !self.preview
            && matches!(&doc.content, Content::Dialogue(d) if d.nodes.len() < 2000)
            && is_dialogue_file(file)
    }

    fn close_preview(&mut self) {
        self.preview = false;
        self.playing = None;
        self.preview_dialogue = None;
        self.preview_error = None;
    }

    fn clear_node_drag(&mut self) {
        self.drag_node = None;
        self.drag_started = false;
        self.drag_anchor = None;
    }
}

#[derive(Component)]
struct CanvasBackdrop;
#[derive(Component)]
struct ResourceResults;

pub(in crate::editor) struct NarrativePlugin;
impl Plugin for NarrativePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "narrative_canvas.wesl");
        connection_paint::register(app);
        effects::register(app);
        picker_fx::register(app);
        app.add_plugins(UiMaterialPlugin::<CanvasMaterial>::default())
            .init_resource::<CanvasPaint>()
            .init_resource::<links::State>()
            .init_resource::<filenames::State>();
        app.add_systems(
            Update,
            (
                edit_fields,
                speaker_menu::input,
                answers::reordering::input,
                links::input,
                navigate,
            )
                .chain()
                .before(crate::editor::handle_actions),
        )
        .add_systems(
            Update,
            actions
                .after(crate::editor::handle_actions)
                .before(super::refresh),
        )
        .add_systems(
            Update,
            filenames::sync
                .after(actions)
                .before(super::refresh)
                .before(crate::editor::close_editor),
        )
        .add_systems(Update, links::hover.after(super::refresh).before(refresh))
        .add_systems(
            Update,
            effects::observe.after(super::refresh).before(refresh),
        )
        .add_systems(Update, refresh.after(super::refresh))
        .add_systems(Update, effects::animate.after(refresh))
        .add_systems(Update, speaker_menu::refresh.after(refresh))
        .add_systems(Update, picker_fx::animate.after(speaker_menu::refresh))
        .add_systems(
            Update,
            (
                text_editor::sync,
                text_editor::keyboard,
                text_editor::color_menu::input,
                text_editor::actions,
            )
                .chain()
                .after(actions)
                .before(super::refresh)
                .before(crate::editor::close_editor),
        )
        .add_systems(Update, text_editor::refresh.after(refresh))
        .add_systems(Update, text_editor::style.after(text_editor::refresh))
        .add_systems(Update, text_editor::decorate.after(text_editor::style))
        .add_systems(Last, text_editor::hide_native_ink)
        .add_systems(
            Update,
            text_editor::color_menu::refresh.after(text_editor::style),
        )
        .add_systems(
            Update,
            (
                nodes::sync,
                answers::animate,
                answers::reordering::paint,
                links::style,
                connection_paint::style,
            )
                .after(refresh),
        );
    }
}

pub(super) fn setup(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    stars: &starfield::Starfield,
    fonts: &mut Assets<Font>,
) {
    let root = surface(
        commands,
        parent,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            top: px(IMAGE_TOP),
            bottom: px(FOOT),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
        PANEL,
    );
    commands.entity(root).insert((
        ZIndex(5),
        AccessibleLabel("Редактор диалогов и персонажей".into()),
    ));
    let toolbar = surface(
        commands,
        root,
        Node {
            height: px(50),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(16)),
            border: UiRect::bottom(px(1)),
            ..row(8.0)
        },
        CHROME,
    );
    let workspace = node(
        commands,
        root,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            ..default()
        },
    );
    let canvas = surface(
        commands,
        workspace,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
        WELL,
    );
    commands.entity(canvas).insert(AccessibleLabel(
        "Граф диалога: перетаскивайте заголовки узлов, колесо — масштаб".into(),
    ));
    starfield::background(commands, canvas, &stars.space);
    commands.spawn((
        CanvasBackdrop,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(canvas),
    ));
    let connections = node(
        commands,
        canvas,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
    );
    commands.entity(connections).insert(Pickable::IGNORE);
    let graph = node(
        commands,
        canvas,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
    );
    commands.entity(graph).insert(Pickable::IGNORE);
    let link_hint = node(
        commands,
        canvas,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
    );
    commands
        .entity(link_hint)
        .insert((Pickable::IGNORE, ZIndex(10)));
    let status = text(commands, root, handle, "", 11.0, MUTED);
    commands.entity(status).insert(Node {
        height: px(31),
        flex_shrink: 0.0,
        padding: UiRect::axes(px(16), px(7)),
        overflow: Overflow::clip(),
        ..default()
    });
    let overlay = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Color::srgba(0.015, 0.019, 0.032, 0.86),
    );
    commands
        .entity(overlay)
        .insert((ZIndex(30), TabGroup::modal()));
    let modal = surface(
        commands,
        overlay,
        Node {
            width: px(600),
            max_width: percent(90),
            max_height: percent(90),
            padding: UiRect::all(px(26)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(18)),
            overflow: Overflow::scroll_y(),
            ..column(14.0)
        },
        PANEL,
    );
    commands
        .entity(modal)
        .insert((Scroller, BorderColor::all(FOCUS.with_alpha(0.45))));
    speaker_menu::setup(commands, root);
    text_editor::setup(commands, parent, handle, fonts);
    commands.insert_resource(NarrativeUi {
        root,
        toolbar,
        canvas,
        graph,
        connections,
        link_hint,
        hint_key: String::new(),
        overlay,
        modal,
        status,
        font: handle.clone(),
        canvas_size: Vec2::new(880.0, 600.0),
        toolbar_key: None,
        character_scrolls: default(),
        graph_key: String::new(),
        wire_key: String::new(),
        modal_key: String::new(),
        image_key: String::new(),
        generation: 0,
        last_edit: None,
        linking: None,
        picker: None,
        search: String::new(),
        playing: None,
        preview_dialogue: None,
        preview_error: None,
        preview: false,
        jump_click: None,
        drag_node: None,
        drag_started: false,
        drag_anchor: None,
        panning: false,
    });
}

fn act(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    label: impl Into<String>,
    action: NarrativeAction,
    primary: bool,
) -> Entity {
    button(
        commands,
        parent,
        handle,
        label,
        Action::Narrative(action),
        if primary {
            Look::Primary
        } else {
            Look::Regular
        },
    )
}
fn wrapped(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    label: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    let t = text(commands, parent, handle, label, size, color);
    commands.entity(t).insert((
        TextLayout::default(),
        Node {
            max_width: percent(100),
            flex_shrink: 0.0,
            ..default()
        },
    ));
    t
}
fn field(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    caption: &str,
    value: &str,
    kind: DraftField,
    multiline: bool,
) -> Entity {
    text(commands, parent, handle, caption, 11.0, MUTED);
    input_field(commands, parent, handle, caption, value, kind, multiline)
}

fn input_field(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    caption: &str,
    value: &str,
    kind: DraftField,
    multiline: bool,
) -> Entity {
    let mut editable = EditableText::new(value);
    editable.allow_newlines = multiline;
    let input = commands
        .spawn((
            TextInput,
            editable,
            kind,
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel(caption.into()),
            font(handle, 14.0),
            TextColor(TEXT),
            TabIndex(0),
            if multiline {
                TextLayout::default()
            } else {
                TextLayout::no_wrap()
            },
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                width: percent(100),
                height: px(if multiline { 116 } else { 36 }),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(10), px(8)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(parent),
        ))
        .id();
    if kind == DraftField::Title {
        filenames::error_label(commands, parent, handle);
    }
    input
}

fn edit_fields(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<NarrativeUi>,
    focus: Res<InputFocus>,
    fields: Query<
        (Entity, &DraftField, &EditableText, Option<&nodes::Owner>),
        Changed<EditableText>,
    >,
    owners: Query<&nodes::Owner>,
) {
    if editor.narrative.active.is_none() || editor.pending.is_some() {
        return;
    }
    if ui.last_edit.is_some_and(|e| focus.get() != Some(e)) {
        ui.last_edit = None;
    }
    if let Some(owner) = focus.get().and_then(|e| owners.get(e).ok()) {
        if let Some(doc) = editor.narrative.current_mut() {
            doc.selected = owner.0;
        }
    }
    for (entity, kind, text, owner) in &fields {
        let value = text.value().to_string();
        if *kind == DraftField::Search {
            if focus.get() == Some(entity) {
                ui.search = value;
            }
            continue;
        }
        let Some(doc) = editor.narrative.current_mut() else {
            continue;
        };
        let mut content = doc.content.clone();
        match (&mut content, kind) {
            (Content::Dialogue(d), DraftField::Title) => d.title = value,
            (Content::Dialogue(d), DraftField::Text) => {
                if let Some(n) = d.node_mut(owner.map_or(doc.selected, |o| o.0)) {
                    n.text_spans = crate::editor::dialogue::text::remap(
                        &n.text,
                        &value,
                        &n.text_format,
                        &n.text_spans,
                    );
                    n.text = value;
                }
            }
            (Content::Dialogue(d), DraftField::Choice(i)) => {
                if let Some(c) = d
                    .node_mut(owner.map_or(doc.selected, |o| o.0))
                    .and_then(|n| n.choices.get_mut(*i))
                {
                    c.text = value;
                }
            }
            (Content::Character(c), DraftField::Title) => c.name = value,
            (Content::Character(c), DraftField::Description) => c.description = value,
            (Content::Character(c), DraftField::PortraitLabel(i)) => {
                if let Some(p) = c.portraits.get_mut(*i) {
                    p.label = value;
                }
            }
            _ => {}
        }
        if content != doc.content {
            if ui.last_edit != Some(entity) {
                doc.checkpoint();
                ui.last_edit = Some(entity);
            }
            doc.content = content;
        }
    }
}

pub(super) fn available(editor: &Editor, action: NarrativeAction) -> bool {
    match action {
        NarrativeAction::Create(_) | NarrativeAction::File(_) => true,
        NarrativeAction::Open(id) | NarrativeAction::Close(id) => editor.narrative.get(id).is_some(),
        _ => editor.narrative.current().is_some_and(|doc| match action {
            NarrativeAction::Node(id, operation) => matches!(&doc.content, Content::Dialogue(d)
                if d.node(id).is_some_and(|n| !matches!(operation, nodes::NodeAction::Delete) || n.kind != NodeKind::Start)),
            NarrativeAction::Undo => !doc.undo.is_empty(),
            NarrativeAction::Redo => !doc.redo.is_empty(),
            NarrativeAction::Delete => matches!(&doc.content,Content::Dialogue(d) if d.node(doc.selected).is_some_and(|n|n.kind!=NodeKind::Start)),
            _ => true,
        }),
    }
}

fn activate(editor: &mut Editor, ui: &mut NarrativeUi) {
    editor.generation.active = false;
    editor.finish_camera_navigation();
    editor.viewing_image = None;
    editor.viewing_model = None;
    ui.generation += 1;
    ui.picker = None;
    ui.close_preview();
    ui.linking = None;
    ui.last_edit = None;
    ui.jump_click = None;
    ui.clear_node_drag();
}
fn save(
    editor: &mut Editor,
    browser: &mut Browser,
    browser_ui: &mut drawer::BrowserUi,
    names: &mut filenames::State,
    id: usize,
) -> bool {
    if !filenames::current(editor, browser, browser_ui, names, id) {
        return false;
    }
    let root = editor.root.clone();
    let Some(d) = editor.narrative.tabs.iter_mut().find(|d| d.id == id) else {
        return false;
    };
    match d.content.save(&root, &d.path) {
        Ok(()) => {
            d.saved = d.content.clone();
            browser.rename_narrative(&root, &d.path, &d.path);
            editor.status = format!("Сохранено · {}", d.path.display());
            true
        }
        Err(e) => {
            editor.status = format!("Не удалось сохранить документ · {e}");
            false
        }
    }
}

fn actions(
    mut messages: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    mut ui: ResMut<NarrativeUi>,
    mut focus: ResMut<InputFocus>,
    mut names: ResMut<filenames::State>,
    mut browser_ui: ResMut<drawer::BrowserUi>,
    mut link_state: ResMut<links::State>,
    mut text_editor: ResMut<text_editor::State>,
    sockets: Query<&links::Output>,
    mut effects: ResMut<effects::State>,
    time: Res<Time>,
) {
    for action in messages.read().copied() {
        if let Some(
            pending @ (PendingAction::CloseNarrative(id) | PendingAction::ExitNarrative(id)),
        ) = editor.pending
        {
            match action {
                Action::SaveAndContinue => {
                    if save(&mut editor, &mut browser, &mut browser_ui, &mut names, id) {
                        editor.pending = None;
                        if matches!(pending, PendingAction::CloseNarrative(_)) {
                            editor.narrative.close(id);
                            ui.close_preview();
                            ui.picker = None;
                            ui.linking = None;
                            ui.generation += 1;
                        } else {
                            editor.continue_exit();
                        }
                    }
                }
                Action::DiscardAndContinue => {
                    if !filenames::discard(
                        &mut editor,
                        &mut browser,
                        &mut browser_ui,
                        &mut names,
                        id,
                    ) {
                        continue;
                    }
                    editor.pending = None;
                    editor.narrative.close(id);
                    ui.close_preview();
                    ui.picker = None;
                    ui.linking = None;
                    ui.generation += 1;
                    if matches!(pending, PendingAction::ExitNarrative(_)) {
                        editor.continue_exit();
                    }
                }
                Action::CancelPending => {
                    editor.pending = None;
                    editor.exit_queue.clear();
                }
                _ => {}
            }
            // Other document handlers see the same messages. Keep this message handled here.
            continue;
        }
        if editor.pending.is_some() {
            continue;
        }
        let Action::Narrative(action) = action else {
            continue;
        };
        if ui.picker == Some(Picker::Text) {
            continue;
        }
        let action = if matches!(action, NarrativeAction::Delete) {
            focus
                .get()
                .and_then(|e| sockets.get(e).ok())
                .map_or(action, |output| {
                    NarrativeAction::Node(output.0, nodes::NodeAction::Disconnect(output.1))
                })
        } else {
            action
        };
        if let NarrativeAction::Link(from, choice) = action {
            if link_state.ignore_click == Some((from, choice)) {
                link_state.ignore_click = None;
                continue;
            }
        }
        if editor.browser_popup && !matches!(action, NarrativeAction::Create(_)) {
            continue;
        }
        if (ui.preview || ui.picker.is_some())
            && !matches!(
                action,
                NarrativeAction::Dismiss
                    | NarrativeAction::Advance(_)
                    | NarrativeAction::Restart
                    | NarrativeAction::PickImage(_)
                    | NarrativeAction::SetSpeaker(_)
                    | NarrativeAction::SetPortrait(_)
                    | NarrativeAction::Target(_)
                    | NarrativeAction::SetDialogue(..)
                    | NarrativeAction::Open(_)
                    | NarrativeAction::File(_)
                    | NarrativeAction::Close(_)
                    | NarrativeAction::CloseCurrent
                    | NarrativeAction::Save
            )
        {
            continue;
        }
        // Node controls carry their own identity; they never operate on a previously selected card.
        let action = if let NarrativeAction::Node(id, operation) = action {
            let Some(doc) = editor.narrative.current_mut() else {
                continue;
            };
            if !matches!(&doc.content, Content::Dialogue(d) if d.node(id).is_some()) {
                continue;
            }
            doc.selected = id;
            operation.action()
        } else {
            action
        };
        if matches!(
            action,
            NarrativeAction::Preview | NarrativeAction::Restart | NarrativeAction::Advance(_)
        ) {
            if let Some(doc) = editor.narrative.current() {
                play(&editor, doc, &mut ui, action);
            }
            continue;
        }
        if let NarrativeAction::SetSpeaker(index) = action {
            if let Some(selection) = speaker_menu::select(&mut editor, &browser, index) {
                ui.picker = selection.portraits.then_some(Picker::Portrait);
                ui.search.clear();
                ui.last_edit = None;
                if selection.changed || selection.portraits {
                    ui.generation += 1;
                }
                focus.clear();
            }
            continue;
        }
        if let NarrativeAction::SetPortrait(index) = action {
            if let Some(changed) = portraits::select(&mut editor, index) {
                ui.picker = None;
                ui.last_edit = None;
                if changed {
                    ui.generation += 1;
                }
                focus.clear();
            }
            continue;
        }
        ui.last_edit = None;
        match action {
            NarrativeAction::Create(character) => {
                let parent = browser.folders[browser.current].path.clone();
                match create(&editor.root, &parent, character) {
                    Ok(path) => {
                        let root = editor.root.clone();
                        match editor.narrative.open(&root, path.clone()) {
                            Ok(()) => {
                                activate(&mut editor, &mut ui);
                                editor.drawer = false;
                                editor.browser_popup = false;
                                editor.status = format!("Создано · {}", path.display());
                            }
                            Err(e) => editor.status = format!("Не удалось открыть документ · {e}"),
                        }
                        browser.refresh(&mut editor.project);
                    }
                    Err(e) => editor.status = format!("Не удалось создать документ · {e}"),
                }
            }
            NarrativeAction::File(index) => {
                if let Some(file) = browser.files.get(index) {
                    let (root, path) = (editor.root.clone(), file.path.clone());
                    match editor.narrative.open(&root, path) {
                        Ok(()) => {
                            activate(&mut editor, &mut ui);
                            editor.drawer = false;
                        }
                        Err(e) => editor.status = format!("Не удалось открыть документ · {e}"),
                    }
                }
            }
            NarrativeAction::FollowJump(document, id) => {
                let Some(doc) = editor.narrative.current().filter(|doc| doc.id == document) else {
                    continue;
                };
                let Content::Dialogue(d) = &doc.content else {
                    continue;
                };
                let Some(path) = d
                    .node(id)
                    .filter(|n| n.kind == NodeKind::Jump)
                    .and_then(|n| n.dialogue.as_deref())
                else {
                    continue;
                };
                let root = editor.root.clone();
                let result = crate::asset_paths::resolve_asset_path(&root, path)
                    .map_err(|e| e.to_string())
                    .and_then(|path| editor.narrative.open(&root, path.into()));
                match result {
                    Ok(()) => {
                        activate(&mut editor, &mut ui);
                        focus.clear();
                    }
                    Err(e) => editor.status = format!("Не удалось открыть диалог · {e}"),
                }
            }
            NarrativeAction::Open(id) => {
                if editor.narrative.get(id).is_some() {
                    editor.narrative.active = Some(id);
                    activate(&mut editor, &mut ui);
                }
            }
            NarrativeAction::CloseCurrent => {
                if let Some(id) = editor.narrative.active {
                    if editor.narrative.get(id).is_some_and(|d| d.dirty()) {
                        editor.pending = Some(PendingAction::CloseNarrative(id));
                    } else {
                        if !filenames::finish(
                            &mut editor,
                            &mut browser,
                            &mut browser_ui,
                            &mut names,
                            id,
                        ) {
                            continue;
                        }
                        editor.narrative.close(id);
                        ui.close_preview();
                        ui.picker = None;
                        ui.linking = None;
                        ui.generation += 1;
                    }
                }
            }
            NarrativeAction::Close(id) => {
                if editor.narrative.get(id).is_some_and(|d| d.dirty()) {
                    editor.pending = Some(PendingAction::CloseNarrative(id));
                } else {
                    if !filenames::finish(
                        &mut editor,
                        &mut browser,
                        &mut browser_ui,
                        &mut names,
                        id,
                    ) {
                        continue;
                    }
                    editor.narrative.close(id);
                    ui.close_preview();
                    ui.picker = None;
                    ui.linking = None;
                    ui.generation += 1;
                }
            }
            _ => {
                let Some(doc) = editor.narrative.current_mut() else {
                    continue;
                };
                match action {
                    NarrativeAction::EditText => {
                        if let Content::Dialogue(dialogue) = &doc.content {
                            if let Some(node) = dialogue
                                .node(doc.selected)
                                .filter(|n| n.kind != NodeKind::Start)
                            {
                                text_editor.open(doc.id, node);
                                ui.picker = Some(Picker::Text);
                                ui.linking = None;
                                ui.last_edit = None;
                                ui.clear_node_drag();
                                ui.panning = false;
                                focus.clear();
                            }
                        }
                    }
                    NarrativeAction::Save => {
                        let id = doc.id;
                        save(&mut editor, &mut browser, &mut browser_ui, &mut names, id);
                    }
                    NarrativeAction::Undo => {
                        ui.clear_node_drag();
                        doc.undo();
                        ui.generation += 1;
                        focus.clear();
                    }
                    NarrativeAction::Redo => {
                        ui.clear_node_drag();
                        doc.redo();
                        ui.generation += 1;
                        focus.clear();
                    }
                    NarrativeAction::Fit => {
                        fit(doc, ui.canvas_size);
                    }
                    NarrativeAction::Zoom(delta) => {
                        doc.zoom = (doc.zoom + delta).clamp(0.35, 1.5);
                    }
                    NarrativeAction::Dismiss => {
                        let was_speaker = ui.picker == Some(Picker::Speaker);
                        ui.picker = None;
                        ui.close_preview();
                        ui.linking = None;
                        if !was_speaker {
                            ui.generation += 1;
                        }
                        focus.clear();
                    }
                    NarrativeAction::Speaker => {
                        ui.picker = Some(Picker::Speaker);
                        ui.search.clear();
                        ui.linking = None;
                        ui.clear_node_drag();
                        ui.panning = false;
                        focus.clear();
                    }
                    NarrativeAction::AddPortrait => {
                        ui.picker = Some(Picker::Image);
                        ui.search.clear();
                    }
                    NarrativeAction::Link(id, choice) => {
                        ui.linking = if ui.linking == Some((id, choice)) {
                            None
                        } else {
                            Some((id, choice))
                        };
                        doc.selected = id;
                        ui.picker = None;
                        focus.clear();
                    }
                    NarrativeAction::Connect(from, choice, target) => {
                        if links::connect(doc, from, choice, target) {
                            ui.generation += 1;
                        }
                        ui.linking = None;
                        focus.clear();
                    }
                    NarrativeAction::DialogueTarget(id) => {
                        if matches!(&doc.content, Content::Dialogue(d) if d.node(id).is_some_and(|n| n.kind == NodeKind::Jump))
                        {
                            doc.selected = id;
                            ui.linking = None;
                            ui.picker = Some(Picker::Dialogue);
                            ui.search.clear();
                            focus.clear();
                        }
                    }
                    NarrativeAction::DropDialogue {
                        document,
                        file,
                        scan_revision,
                        local,
                    } if doc.id == document && scan_revision == browser.scan_revision => {
                        let Some(file) = browser
                            .files
                            .get(file)
                            .filter(|f| ui.accepts_dialogue_drop(doc, f))
                        else {
                            continue;
                        };
                        let path = file.path.to_string_lossy().replace('\\', "/");
                        doc.checkpoint();
                        if let Content::Dialogue(d) = &mut doc.content {
                            let id = d.add(NodeKind::Jump, doc.selected);
                            let node = d.node_mut(id).unwrap();
                            node.dialogue = Some(path);
                            node.position = ((local - doc.pan) / doc.zoom).to_array();
                            doc.selected = id;
                        }
                        ui.linking = None;
                        ui.generation += 1;
                        focus.clear();
                    }
                    NarrativeAction::SetDialogue(file, revision)
                        if ui.picker == Some(Picker::Dialogue)
                            && revision == browser.scan_revision =>
                    {
                        let path = match file {
                            Some(i) => {
                                let Some(file) =
                                    browser.files.get(i).filter(|f| is_dialogue_file(f))
                                else {
                                    continue;
                                };
                                Some(file.path.to_string_lossy().replace('\\', "/"))
                            }
                            None => None,
                        };
                        if matches!(&doc.content, Content::Dialogue(d) if d.node(doc.selected)
                            .is_some_and(|n| n.kind == NodeKind::Jump && n.dialogue != path))
                        {
                            doc.checkpoint();
                            if let Content::Dialogue(d) = &mut doc.content {
                                d.node_mut(doc.selected).unwrap().dialogue = path;
                            }
                            ui.generation += 1;
                        }
                        ui.picker = None;
                        focus.clear();
                    }
                    NarrativeAction::Target(u32::MAX) => {
                        ui.picker = Some(Picker::Target);
                    }
                    NarrativeAction::Select(id) | NarrativeAction::Target(id) => {
                        if let Content::Dialogue(d) = &mut doc.content {
                            if d.node(id).is_none() {
                                continue;
                            }
                            if let Some((from, choice)) = ui.linking.take() {
                                if links::connect(doc, from, choice, id) {
                                    ui.generation += 1;
                                }
                            } else {
                                doc.selected = id;
                            }
                            ui.picker = None;
                            focus.clear();
                        }
                    }
                    NarrativeAction::Add(kind) => {
                        if let Content::Dialogue(d) = &doc.content {
                            if d.nodes.len() >= 2000 {
                                continue;
                            }
                        }
                        doc.checkpoint();
                        if let Content::Dialogue(d) = &mut doc.content {
                            let source = ui.linking.take();
                            let from = source.map_or(doc.selected, |(from, _)| from);
                            let targets = source
                                .and_then(|(from, _)| d.node(from))
                                .map(|n| n.choices.iter().map(|c| c.target).collect::<Vec<_>>());
                            let id = d.add(kind, from);
                            if let Some((from, choice)) = source {
                                if let Some(n) = d.node_mut(from) {
                                    // d.add connects the first free output. An explicit output takes priority.
                                    for (c, target) in
                                        n.choices.iter_mut().zip(targets.into_iter().flatten())
                                    {
                                        c.target = target;
                                    }
                                    if let Some(c) = n.choices.get_mut(choice) {
                                        c.target = Some(id);
                                        c.dialogue = None;
                                    }
                                }
                            }
                            nodes::place(d, id, from);
                            doc.selected = id;
                        }
                        if let Content::Dialogue(d) = &doc.content {
                            if let Some(n) = d.node(doc.selected) {
                                let at = Vec2::from_array(n.position) * doc.zoom + doc.pan;
                                if at.x < 16.0
                                    || at.x + NODE_WIDTH * doc.zoom > ui.canvas_size.x - 16.0
                                    || at.y < 16.0
                                    || at.y + nodes::height(n) * doc.zoom > ui.canvas_size.y - 16.0
                                {
                                    fit(doc, ui.canvas_size);
                                }
                            }
                        }
                        ui.generation += 1;
                        focus.clear();
                    }
                    NarrativeAction::Delete => {
                        doc.checkpoint();
                        if let Content::Dialogue(d) = &mut doc.content {
                            d.delete(doc.selected);
                            doc.selected = d
                                .nodes
                                .iter()
                                .find(|n| n.kind == NodeKind::Start)
                                .map_or(0, |n| n.id);
                        }
                        ui.generation += 1;
                        ui.linking = None;
                        focus.clear();
                    }
                    NarrativeAction::AddChoice
                    | NarrativeAction::RemoveChoice(_)
                    | NarrativeAction::Disconnect(_) => {
                        if matches!(
                            action,
                            NarrativeAction::RemoveChoice(_) | NarrativeAction::Disconnect(_)
                        ) {
                            ui.linking = None;
                        }
                        if let NarrativeAction::Disconnect(i) = action {
                            if !matches!(&doc.content, Content::Dialogue(d) if d.node(doc.selected).and_then(|n| n.choices.get(i)).is_some_and(Choice::connected))
                            {
                                continue;
                            }
                        }
                        doc.checkpoint();
                        if let Content::Dialogue(d) = &mut doc.content {
                            if let Some(n) = d.node_mut(doc.selected) {
                                match action {
                                    NarrativeAction::AddChoice if n.kind == NodeKind::Line => {
                                        n.choices.push(Choice {
                                            text: format!("Ответ {}", n.choices.len() + 1),
                                            target: None,
                                            dialogue: None,
                                        });
                                        effects.answers.add(doc.id, n, time.elapsed_secs_f64());
                                    }
                                    NarrativeAction::RemoveChoice(i) if i < n.choices.len() => {
                                        effects.answers.remove(
                                            doc.id,
                                            n,
                                            i,
                                            time.elapsed_secs_f64(),
                                        );
                                        n.choices.remove(i);
                                    }
                                    NarrativeAction::Disconnect(i) => {
                                        if let Some(c) = n.choices.get_mut(i) {
                                            c.target = None;
                                            c.dialogue = None;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        ui.generation += 1;
                        focus.clear();
                    }
                    NarrativeAction::PickImage(index) => {
                        if let Some(file) = browser
                            .files
                            .get(index)
                            .filter(|f| f.kind == crate::editor::browser::FileKind::Image)
                        {
                            doc.checkpoint();
                            if let Content::Character(c) = &mut doc.content {
                                let path = file.path.to_string_lossy().replace('\\', "/");
                                if !c.portraits.iter().any(|p| p.path == path) {
                                    c.portraits.push(Portrait {
                                        label: file
                                            .path
                                            .file_stem()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned(),
                                        path,
                                    });
                                }
                            }
                            ui.picker = None;
                            ui.generation += 1;
                            focus.clear();
                        }
                    }
                    NarrativeAction::RemovePortrait(i) | NarrativeAction::MainPortrait(i) => {
                        doc.checkpoint();
                        if let Content::Character(c) = &mut doc.content {
                            if i < c.portraits.len() {
                                if matches!(action, NarrativeAction::RemovePortrait(_)) {
                                    c.portraits.remove(i);
                                } else {
                                    let p = c.portraits.remove(i);
                                    c.portraits.insert(0, p);
                                }
                            }
                        }
                        ui.generation += 1;
                        focus.clear();
                    }
                    _ => {}
                }
            }
        }
    }
}

fn play(editor: &Editor, doc: &Document, ui: &mut NarrativeUi, action: NarrativeAction) {
    let Content::Dialogue(origin) = &doc.content else {
        return;
    };
    if !ui.preview && matches!(action, NarrativeAction::Advance(_)) {
        return;
    }
    ui.preview_error = None;
    let mut next = if let NarrativeAction::Advance(index) = action {
        let dialogue = ui.preview_dialogue.as_ref().map_or(origin, |(_, d)| d);
        ui.playing
            .and_then(|id| dialogue.node(id))
            .and_then(|n| n.choices.get(index))
            .cloned()
    } else {
        ui.preview = true;
        ui.picker = None;
        ui.linking = None;
        ui.preview_dialogue = None;
        ui.playing = start_node(origin);
        ui.playing
            .and_then(|id| origin.node(id))
            .and_then(|n| n.choices.first())
            .cloned()
    };
    // Only automatic start transitions are guarded. Player choices may loop freely.
    let mut visited = std::collections::HashSet::new();
    while let Some(choice) = next.take() {
        if let Some(path) = choice.dialogue {
            let result = crate::asset_paths::resolve_asset_path(&editor.root, &path)
                .map_err(|e| e.to_string())
                .and_then(|resolved| {
                    let content = editor
                        .narrative
                        .tabs
                        .iter()
                        .find(|tab| same_asset_path(&tab.path.to_string_lossy(), &resolved))
                        .map(|tab| Ok(tab.content.clone()))
                        .unwrap_or_else(|| {
                            Content::read(&editor.root, std::path::Path::new(&resolved))
                        })?;
                    match content {
                        Content::Dialogue(d) => Ok((resolved, d)),
                        _ => Err("Ожидается файл .dialog".to_owned()),
                    }
                });
            match result {
                Ok((resolved, dialogue)) => {
                    ui.playing = start_node(&dialogue);
                    ui.preview_dialogue = Some((resolved, dialogue));
                }
                Err(error) => {
                    ui.preview_error = Some(format!("Не удалось открыть «{path}»: {error}"));
                    break;
                }
            }
        } else if let Some(target) = choice.target {
            ui.playing = Some(target);
        } else {
            break;
        }
        let (path, dialogue) = ui.preview_dialogue.as_ref().map_or_else(
            || (doc.path.to_string_lossy().into_owned(), origin),
            |(path, d)| (path.clone(), d),
        );
        let Some(node) = ui.playing.and_then(|id| dialogue.node(id)) else {
            break;
        };
        if !matches!(node.kind, NodeKind::Start | NodeKind::Jump) {
            break;
        }
        if !visited.insert((path, node.id)) || visited.len() > 128 {
            ui.preview_error =
                Some("Проверьте цепочку автоматических переходов между диалогами".into());
            break;
        }
        next = if node.kind == NodeKind::Jump {
            if node.dialogue.is_none() {
                ui.preview_error = Some("Выберите файл в ноде перехода".into());
            }
            node.dialogue.clone().map(|path| Choice {
                text: String::new(),
                target: None,
                dialogue: Some(path),
            })
        } else {
            node.choices.first().cloned()
        };
    }
}

fn start_node(d: &Dialogue) -> Option<u32> {
    d.nodes
        .iter()
        .find(|n| n.kind == NodeKind::Start)
        .map(|n| n.id)
}

fn is_dialogue_file(file: &crate::editor::browser::BrowserFile) -> bool {
    file.extension.eq_ignore_ascii_case("dialog")
}

pub(super) fn shortcuts(
    keys: &ButtonInput<KeyCode>,
    typing: bool,
    messages: &mut MessageWriter<Action>,
) {
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if command && keys.just_pressed(KeyCode::Space) {
        messages.write(Action::Drawer);
        return;
    }
    let action = if command && keys.just_pressed(KeyCode::KeyW) {
        Some(NarrativeAction::CloseCurrent)
    } else if command && keys.just_pressed(KeyCode::KeyS) {
        Some(NarrativeAction::Save)
    } else if keys.just_pressed(KeyCode::Escape) {
        Some(NarrativeAction::Dismiss)
    } else if typing {
        None
    } else if command && keys.just_pressed(KeyCode::KeyZ) {
        Some(if shift {
            NarrativeAction::Redo
        } else {
            NarrativeAction::Undo
        })
    } else if command && keys.just_pressed(KeyCode::KeyY) {
        Some(NarrativeAction::Redo)
    } else if keys.just_pressed(KeyCode::Delete) {
        Some(NarrativeAction::Delete)
    } else if keys.just_pressed(KeyCode::KeyF) {
        Some(NarrativeAction::Fit)
    } else {
        None
    };
    if let Some(action) = action {
        messages.write(Action::Narrative(action));
    }
}

fn fit(doc: &mut Document, size: Vec2) {
    let Content::Dialogue(d) = &doc.content else {
        return;
    };
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for n in &d.nodes {
        let p = Vec2::from_array(n.position);
        min = min.min(p);
        max = max.max(p + Vec2::new(NODE_WIDTH, nodes::height(n)));
    }
    let extent = (max - min).max(Vec2::ONE);
    doc.zoom = ((size.x - 64.0) / extent.x)
        .min((size.y - 100.0) / extent.y)
        .clamp(0.35, 1.2);
    doc.pan = (size - extent * doc.zoom) * 0.5 - min * doc.zoom;
}

fn navigate(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<NarrativeUi>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    headers: Query<(&Control, &Hovered), Without<links::Input>>,
    drawer: Res<DrawerMotion>,
    mut focus: ResMut<InputFocus>,
    link_state: Res<links::State>,
    effects: Res<effects::State>,
    time: Res<Time>,
    mut redraw: MessageWriter<bevy::window::RequestRedraw>,
    mut actions: MessageWriter<Action>,
) {
    if editor.pending.is_some()
        || editor.browser_popup
        || ui.preview
        || ui.picker.is_some()
        || !window.focused
        || effects.answers.capturing()
    {
        ui.jump_click = None;
        ui.clear_node_drag();
        ui.panning = false;
        return;
    }
    let Some(doc) = editor.narrative.current_mut() else {
        ui.jump_click = None;
        ui.clear_node_drag();
        ui.panning = false;
        return;
    };
    if !matches!(doc.content, Content::Dialogue(_)) {
        ui.jump_click = None;
        ui.clear_node_drag();
        ui.panning = false;
        return;
    }
    let Ok((computed, transform)) = geometry.get(ui.canvas) else {
        return;
    };
    let Some(point) = window.cursor_position() else {
        ui.jump_click = None;
        ui.clear_node_drag();
        ui.panning = false;
        return;
    };
    ui.canvas_size = computed.size() / window.scale_factor();
    let rect = Rect::from_center_size(
        transform.translation / window.scale_factor(),
        ui.canvas_size,
    );
    let inside = rect.contains(point) && point.y < window.height() - FOOT - drawer.visible_height;
    if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
        ui.clear_node_drag();
    }
    if !mouse.pressed(MouseButton::Middle) && !mouse.pressed(MouseButton::Right) {
        ui.panning = false;
    }
    if inside
        && !link_state.block_pan
        && (mouse.just_pressed(MouseButton::Middle) || mouse.just_pressed(MouseButton::Right))
    {
        ui.panning = true;
    }
    if ui.panning {
        doc.pan += motion.delta / window.scale_factor();
    }
    let local = point - rect.min;
    if inside && scroll.delta.y != 0.0 {
        let before = doc.zoom;
        doc.zoom = (doc.zoom * (scroll.delta.y * 0.12).exp()).clamp(0.35, 1.5);
        doc.pan = local - (local - doc.pan) * (doc.zoom / before);
    }
    if keys.just_pressed(KeyCode::Escape) {
        ui.clear_node_drag();
        ui.linking = None;
        return;
    }
    if inside
        && ui.linking.is_none()
        && link_state.drag.is_none()
        && mouse.just_pressed(MouseButton::Left)
    {
        // Use the picked header, so covered nodes and fields never start a drag.
        ui.clear_node_drag();
        let id = headers
            .iter()
            .find_map(|(control, hovered)| match control.action {
                Action::Narrative(NarrativeAction::Select(id)) if hovered.0 => Some(id),
                _ => None,
            });
        if let Some(id) = id
            && let Content::Dialogue(d) = &doc.content
            && let Some(n) = d.node(id)
        {
            ui.drag_node = Some(id);
            let center = Vec2::new(NODE_WIDTH, effects.answers.height(n)) * 0.5;
            ui.drag_anchor = Some(dragging::Anchor::new(
                doc.id,
                point,
                local,
                doc.pan,
                doc.zoom,
                Vec2::from_array(n.position),
                center,
                effects.scale(id),
            ));
        }
    }
    if let (Some(id), Some(anchor)) = (ui.drag_node, ui.drag_anchor) {
        if anchor.document != doc.id {
            ui.clear_node_drag();
            return;
        }
        if (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
            && (ui.drag_started || anchor.moved(point))
        {
            let desired = if let Content::Dialogue(d) = &doc.content {
                d.node(id).map(|n| {
                    anchor.position(
                        local,
                        doc.pan,
                        doc.zoom,
                        Vec2::new(NODE_WIDTH, effects.answers.height(n)) * 0.5,
                        effects.scale_at(id, time.elapsed_secs_f64()),
                    )
                })
            } else {
                None
            };
            if let Some(position) = desired {
                let changed = matches!(&doc.content,Content::Dialogue(d) if d.node(id).is_some_and(|n|!Vec2::from_array(n.position).abs_diff_eq(position,0.0001)));
                if changed {
                    if !ui.drag_started {
                        doc.checkpoint();
                        ui.drag_started = true;
                        doc.selected = id;
                        focus.clear();
                    }
                    if let Content::Dialogue(d) = &mut doc.content {
                        d.node_mut(id).unwrap().position = position.to_array();
                    }
                    redraw.write(bevy::window::RequestRedraw);
                }
            } else {
                ui.clear_node_drag();
            }
        }
    }
    // Use the release position too: a fast final cursor event can arrive with mouse-up.
    if mouse.just_released(MouseButton::Left) {
        let previous = ui.jump_click.take();
        if inside && !ui.drag_started
            && let (Some(id), Some(anchor)) = (ui.drag_node, ui.drag_anchor)
            && !anchor.moved(point)
            && matches!(&doc.content, Content::Dialogue(d) if d.node(id)
                .is_some_and(|n| n.kind == NodeKind::Jump && n.dialogue.is_some()))
            && headers.iter().any(|(control, hovered)| hovered.0
                && matches!(control.action, Action::Narrative(NarrativeAction::Select(node)) if node == id))
        {
            let now = time.elapsed_secs_f64();
            if previous.is_some_and(|(document, node, at, position)| {
                document == doc.id && node == id && now - at <= 0.5
                    && point.distance_squared(position) <= 36.0
            }) {
                actions.write(Action::Narrative(NarrativeAction::FollowJump(doc.id, id)));
            } else {
                ui.jump_click = Some((doc.id, id, now, point));
            }
        }
        ui.clear_node_drag();
    }
}

fn character(editor: &Editor, path: &str) -> Option<Character> {
    if let Some(c) = editor
        .narrative
        .tabs
        .iter()
        .find(|d| d.path == std::path::Path::new(path))
        .and_then(|d| match &d.content {
            Content::Character(c) => Some(c),
            _ => None,
        })
    {
        return Some(c.clone());
    }
    match Content::read(&editor.root, std::path::Path::new(path)).ok()? {
        Content::Character(c) => Some(c),
        _ => None,
    }
}
fn speaker(editor: &Editor, node: &DialogueNode) -> String {
    node.speaker.as_ref().map_or_else(
        || "Персонаж не выбран".into(),
        |p| character(editor, p).map_or_else(|| "Персонаж не найден".into(), |c| c.name),
    )
}
fn portrait(
    commands: &mut Commands,
    parent: Entity,
    server: &AssetServer,
    root: &std::path::Path,
    path: &str,
    size: f32,
) {
    let valid = safe_relative(path)
        && root.join(path).is_file()
        && root
            .join(path)
            .canonicalize()
            .ok()
            .is_some_and(|p| root.canonicalize().ok().is_some_and(|r| p.starts_with(r)));
    if valid {
        let (width, height) = image::image_dimensions(root.join(path)).unwrap_or((1, 1));
        let fit = size / width.max(height).max(1) as f32;
        let frame = surface(
            commands,
            parent,
            Node {
                width: px(size),
                height: px(size),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(12)),
                overflow: Overflow::clip(),
                ..default()
            },
            WELL,
        );
        commands.entity(frame).insert(Pickable::IGNORE);
        commands.spawn((
            ImageNode::new(server.load(path.to_string())),
            Node {
                width: px(width as f32 * fit),
                height: px(height as f32 * fit),
                flex_shrink: 0.0,
                border_radius: BorderRadius::all(px(12)),
                overflow: Overflow::clip(),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(frame),
        ));
    } else {
        let p = surface(
            commands,
            parent,
            Node {
                width: px(size),
                height: px(size),
                flex_shrink: 0.0,
                border_radius: BorderRadius::all(px(12)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::all(px(8)),
                ..column(8.0)
            },
            WELL,
        );
        commands.entity(p).insert((
            AccessibleLabel("Портрет не найден".into()),
            Pickable::IGNORE,
        ));
        icons::spawn(
            commands,
            p,
            icons::Glyph::Image,
            (size * 0.24).clamp(12.0, 36.0),
            MUTED,
        );
    }
}

fn graph(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    editor: &Editor,
    doc: &Document,
    linking: Option<(u32, usize)>,
    server: &AssetServer,
    scroll: Vec2,
    answers: &answers::State,
) {
    match &doc.content {
        Content::Dialogue(d) => nodes::spawn(
            commands, parent, handle, editor, doc, d, linking, server, answers,
        ),
        Content::Character(c) => {
            character_sheet::spawn(commands, parent, handle, editor, doc, c, server, scroll)
        }
    }
}

fn modal(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    ui: &NarrativeUi,
    editor: &Editor,
    doc: &Document,
    server: &AssetServer,
    text_state: &text_editor::State,
) {
    let header = node(
        commands,
        parent,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(12.0)
        },
    );
    text(
        commands,
        header,
        handle,
        if ui.preview {
            "ПРЕДПРОСМОТР ДИАЛОГА"
        } else if ui.picker == Some(Picker::Portrait) {
            "ПОРТРЕТ ПЕРСОНАЖА"
        } else {
            "ВЫБОР РЕСУРСА"
        },
        11.0,
        FOCUS,
    );
    let close = act(
        commands,
        header,
        handle,
        "Закрыть · Esc",
        NarrativeAction::Dismiss,
        false,
    );
    commands.queue(move |world: &mut World| {
        world
            .resource_mut::<InputFocus>()
            .set(close, bevy::input_focus::FocusCause::Auto);
    });
    if ui.preview {
        let Content::Dialogue(d) = &doc.content else {
            return;
        };
        let d = ui
            .preview_dialogue
            .as_ref()
            .map_or(d, |(_, dialogue)| dialogue);
        text(commands, parent, handle, &d.title, 13.0, MUTED);
        if let Some(error) = &ui.preview_error {
            wrapped(commands, parent, handle, error, 14.0, AMBER);
        }
        let Some(n) = ui.playing.and_then(|id| d.node(id)) else {
            return;
        };
        if let Some(c) = n.speaker.as_ref().and_then(|p| character(editor, p)) {
            if let Some(p) = c.portraits.get(n.portrait).or(c.portraits.first()) {
                portrait(commands, parent, server, &editor.root, &p.path, 128.0);
            }
        }
        text(
            commands,
            parent,
            handle,
            if n.kind == NodeKind::End {
                "Разговор завершён".into()
            } else {
                speaker(editor, n)
            },
            23.0,
            TEXT,
        );
        if n.kind == NodeKind::Start && ui.preview_error.is_none() {
            wrapped(
                commands,
                parent,
                handle,
                "Начало не соединено с репликой. Закройте предпросмотр и добавьте переход.",
                17.0,
                TEXT,
            );
        } else {
            text_editor::render(
                commands,
                parent,
                handle,
                text_state,
                &n.text,
                &n.text_format,
                &n.text_spans,
            );
        }
        for (i, c) in n.choices.iter().enumerate() {
            if c.connected() {
                let well = surface(
                    commands,
                    parent,
                    Node {
                        width: percent(100),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(9)),
                        ..default()
                    },
                    WELL,
                );
                commands
                    .entity(well)
                    .insert(BorderColor::all(FOCUS.with_alpha(0.22)));
                let b = control(
                    commands,
                    well,
                    Action::Narrative(NarrativeAction::Advance(i)),
                    Look::Regular,
                    Node {
                        width: percent(100),
                        min_height: px(44),
                        padding: UiRect::all(px(12)),
                        ..row(10.0)
                    },
                );
                commands.entity(b).insert(AccessibleLabel(c.text.clone()));
                text(commands, b, handle, format!("{:02}", i + 1), 11.0, FOCUS);
                wrapped(commands, b, handle, &c.text, 14.0, TEXT);
            } else {
                wrapped(
                    commands,
                    parent,
                    handle,
                    format!("{} · переход не задан", c.text),
                    12.0,
                    AMBER,
                );
            }
        }
        act(
            commands,
            parent,
            handle,
            "Начать заново",
            NarrativeAction::Restart,
            false,
        );
        return;
    }
    match ui.picker {
        Some(Picker::Speaker | Picker::Text) => {}
        Some(Picker::Image) => {
            text(commands, parent, handle, "Добавить портрет", 23.0, TEXT);
            wrapped(
                commands,
                parent,
                handle,
                "Изображения из ассетов. Новые фотографии можно перетащить из проводника в файловый менеджер редактора.",
                13.0,
                MUTED,
            );
            field(
                commands,
                parent,
                handle,
                "Найти изображение",
                &ui.search,
                DraftField::Search,
                false,
            );
            let list = node(
                commands,
                parent,
                Node {
                    max_height: px(350),
                    width: percent(100),
                    overflow: Overflow::scroll_y(),
                    ..column(6.0)
                },
            );
            commands.entity(list).insert((Scroller, ResourceResults));
        }
        Some(Picker::Portrait | Picker::Dialogue) => {}
        Some(Picker::Target) => {
            text(commands, parent, handle, "Куда ведёт ответ?", 23.0, TEXT);
            if let Content::Dialogue(d) = &doc.content {
                for n in d.nodes.iter().filter(|n| n.kind != NodeKind::Start) {
                    act(
                        commands,
                        parent,
                        handle,
                        format!(
                            "#{} · {}",
                            n.id,
                            if n.kind == NodeKind::Start {
                                "Начало"
                            } else {
                                &n.text
                            }
                        ),
                        NarrativeAction::Target(n.id),
                        false,
                    );
                }
            }
        }
        None => {}
    }
}

fn image_results(
    commands: &mut Commands,
    list: Entity,
    handle: &Handle<Font>,
    search: &str,
    editor: &Editor,
    browser: &Browser,
    server: &AssetServer,
) {
    let mut count = 0;
    for (i, f) in browser.files.iter().enumerate().filter(|(_, f)| {
        f.kind == crate::editor::browser::FileKind::Image
            && f.path
                .to_string_lossy()
                .to_lowercase()
                .contains(&search.to_lowercase())
    }) {
        count += 1;
        let row = node(commands, list, row(10.0));
        portrait(
            commands,
            row,
            server,
            &editor.root,
            &f.path.to_string_lossy(),
            44.0,
        );
        act(
            commands,
            row,
            handle,
            f.path.to_string_lossy(),
            NarrativeAction::PickImage(i),
            false,
        );
    }
    if count == 0 {
        wrapped(
            commands,
            list,
            handle,
            "Изображения не найдены. Добавьте PNG, JPG или WebP в ассеты проекта.",
            14.0,
            MUTED,
        );
    }
}

fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    browser: Res<Browser>,
    mut ui: ResMut<NarrativeUi>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    server: Res<AssetServer>,
    paint: Res<CanvasPaint>,
    mut materials: ResMut<Assets<CanvasMaterial>>,
    mut wire_materials: ResMut<Assets<connection_paint::Material>>,
    backdrops: Query<Entity, (With<CanvasBackdrop>, Without<MaterialNode<CanvasMaterial>>)>,
    image_lists: Query<Entity, With<ResourceResults>>,
    character_stages: Query<(&character_sheet::Stage, &ScrollPosition)>,
    link_state: Res<links::State>,
    text_state: Res<text_editor::State>,
    effects: Res<effects::State>,
) {
    if let Ok(mut n) = nodes.get_mut(ui.root) {
        n.display = if editor.narrative.active.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (stage, scroll) in &character_stages {
        ui.character_scrolls.insert(stage.0, scroll.0);
    }
    ui.character_scrolls
        .retain(|id, _| editor.narrative.get(*id).is_some());
    let Some(doc) = editor.narrative.current() else {
        ui.toolbar_key = None;
        return;
    };
    if let Some(mut m) = materials.get_mut(&paint.0) {
        m.view = Vec4::new(doc.pan.x, doc.pan.y, doc.zoom, 0.0);
    }
    for e in &backdrops {
        commands.entity(e).insert(MaterialNode(paint.0.clone()));
    }
    let is_dialog = matches!(doc.content, Content::Dialogue(_));
    let key = (doc.id, ui.generation);
    if ui.toolbar_key != Some(key) {
        ui.toolbar_key = Some(key);
        commands.entity(ui.toolbar).despawn_related::<Children>();
        text(
            &mut commands,
            ui.toolbar,
            &ui.font,
            if is_dialog {
                "ГРАФ ДИАЛОГА"
            } else {
                "КАРТОЧКА ПЕРСОНАЖА"
            },
            10.0,
            FOCUS,
        );
        if is_dialog {
            act(
                &mut commands,
                ui.toolbar,
                &ui.font,
                "+ Реплика",
                NarrativeAction::Add(NodeKind::Line),
                false,
            );
            act(
                &mut commands,
                ui.toolbar,
                &ui.font,
                "+ Конец",
                NarrativeAction::Add(NodeKind::End),
                false,
            );
            act(
                &mut commands,
                ui.toolbar,
                &ui.font,
                "+ Другой диалог",
                NarrativeAction::Add(NodeKind::Jump),
                false,
            );
            for (label, delta) in [("−", -0.1), ("+", 0.1)] {
                act(
                    &mut commands,
                    ui.toolbar,
                    &ui.font,
                    label,
                    NarrativeAction::Zoom(delta),
                    false,
                );
            }
            act(
                &mut commands,
                ui.toolbar,
                &ui.font,
                "Весь граф · F",
                NarrativeAction::Fit,
                false,
            );
        }
        let spacer = node(
            &mut commands,
            ui.toolbar,
            Node {
                flex_grow: 1.0,
                ..default()
            },
        );
        commands.entity(spacer).insert(Pickable::IGNORE);
        icons::button(
            &mut commands,
            ui.toolbar,
            icons::Glyph::Undo,
            "Отменить · Ctrl+Z",
            Action::Narrative(NarrativeAction::Undo),
            Look::Regular,
        );
        icons::button(
            &mut commands,
            ui.toolbar,
            icons::Glyph::Redo,
            "Повторить · Ctrl+Shift+Z",
            Action::Narrative(NarrativeAction::Redo),
            Look::Regular,
        );
        if is_dialog {
            act(
                &mut commands,
                ui.toolbar,
                &ui.font,
                "Проверить",
                NarrativeAction::Preview,
                false,
            );
        }
        act(
            &mut commands,
            ui.toolbar,
            &ui.font,
            "Сохранить",
            NarrativeAction::Save,
            true,
        );
    }
    // Editable cards remain alive while typing or navigating the canvas.
    let graph_key = if is_dialog {
        format!("{}:{}", doc.id, ui.generation)
    } else {
        format!("character:{}:{}", doc.id, ui.generation)
    };
    if ui.graph_key != graph_key {
        ui.graph_key = graph_key;
        commands.entity(ui.graph).despawn_related::<Children>();
        graph(
            &mut commands,
            ui.graph,
            &ui.font,
            &editor,
            doc,
            ui.linking,
            &server,
            ui.character_scrolls
                .get(&doc.id)
                .copied()
                .unwrap_or_default(),
            &effects.answers,
        );
    }
    let wire_key = if let Content::Dialogue(d) = &doc.content {
        format!(
            "{}:{:?}:{:?}:{}:{:?}:{:?}",
            doc.id,
            doc.pan,
            doc.zoom,
            doc.selected,
            d.nodes
                .iter()
                .map(|n| (
                    n.id,
                    n.kind,
                    n.position,
                    n.choices.iter().map(|c| c.target).collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>(),
            link_state
                .drag
                .as_ref()
                .filter(|d| d.moved)
                .map(|d| (d.from, d.choice, d.pointer, d.target))
        )
    } else {
        String::new()
    };
    if ui.wire_key != wire_key {
        ui.wire_key = wire_key;
        commands
            .entity(ui.connections)
            .despawn_related::<Children>();
        if let Content::Dialogue(d) = &doc.content {
            nodes::connections(
                &mut commands,
                ui.connections,
                d,
                doc,
                link_state.drag.as_ref().filter(|d| d.moved),
                &mut wire_materials,
            );
        }
    }
    let hint_key = format!("{}:{:?}:{}", doc.id, ui.linking, link_state.drag.is_some());
    if ui.hint_key != hint_key {
        ui.hint_key = hint_key;
        commands.entity(ui.link_hint).despawn_related::<Children>();
        if let Content::Dialogue(d) = &doc.content {
            if link_state.drag.is_none() {
                nodes::hint(&mut commands, ui.link_hint, &ui.font, ui.linking, d);
            }
        }
    }
    let modal_key = format!(
        "{:?}:{}:{:?}:{:?}:{:?}:{:?}:{}",
        ui.picker,
        ui.preview,
        ui.playing,
        ui.preview_dialogue
            .as_ref()
            .filter(|_| ui.preview)
            .map(|(path, _)| path),
        ui.preview_error,
        doc.content,
        browser.scan_revision
    );
    let shown = matches!(ui.picker, Some(Picker::Image | Picker::Target)) || ui.preview;
    if let Ok(mut n) = nodes.get_mut(ui.overlay) {
        n.display = if shown { Display::Flex } else { Display::None };
    }
    let modal_rebuilt = shown && ui.modal_key != modal_key;
    if modal_rebuilt {
        ui.image_key.clear();
        ui.modal_key = modal_key;
        commands.entity(ui.modal).despawn_related::<Children>();
        modal(
            &mut commands,
            ui.modal,
            &ui.font,
            &ui,
            &editor,
            doc,
            &server,
            &text_state,
        );
    }
    if ui.picker == Some(Picker::Image) && !modal_rebuilt {
        let key = format!("{}:{}", ui.search, browser.scan_revision);
        if ui.image_key != key {
            for list in &image_lists {
                commands.entity(list).despawn_related::<Children>();
                image_results(
                    &mut commands,
                    list,
                    &ui.font,
                    &ui.search,
                    &editor,
                    &browser,
                    &server,
                );
                ui.image_key = key.clone();
            }
        }
    }
    if !shown {
        ui.modal_key.clear();
    }
    if let Ok(mut label) = labels.get_mut(ui.status) {
        let status = if let Content::Dialogue(d) = &doc.content {
            let warnings = d.warnings();
            let validation = warnings.first().map_or_else(
                || "Все ветки соединены".to_owned(),
                |first| {
                    if warnings.len() == 1 {
                        first.clone()
                    } else {
                        format!("{first} · ещё {}", warnings.len() - 1)
                    }
                },
            );
            format!(
                "{}%   ·   {}   ·   Выход → вход — перетащить связь   ·   Заголовок — двигать   ·   Колесо — масштаб",
                (doc.zoom * 100.0).round(),
                validation
            )
        } else {
            format!(
                "{}   ·   Изменения {}   ·   Ctrl+S — сохранить",
                doc.path.display(),
                if doc.dirty() {
                    "не сохранены"
                } else {
                    "сохранены"
                }
            )
        };
        label.set_if_neq(Text(status));
    }
}

#[cfg(test)]
#[path = "narrative_ui_tests.rs"]
mod tests;
