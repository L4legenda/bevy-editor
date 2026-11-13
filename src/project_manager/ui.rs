use super::{
    Action, ContextMenu, Dialog, Manager, effects,
    native::Picker,
    projects::{self, Template},
};

#[path = "context_menu.rs"]
mod context_menu;
use crate::{
    editor::{
        icons::{self, Glyph},
        orbital, starfield,
    },
    ui_theme::{
        self, ACCENT, ACTIVE, CHROME, DISABLED_TEXT, EDGE, ERROR, FOCUS, HOVER, MUTED, ON_ACCENT,
        PANEL, TEXT, WELL, horizon,
    },
};
use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    input_focus::{
        FocusCause, InputFocus,
        tab_navigation::{TabGroup, TabIndex},
    },
    picking::hover::Hovered,
    prelude::*,
    text::{EditableText, LineBreak, TextCursorStyle, TextReadWriteMode},
    ui::{BackgroundGradient, InteractionDisabled, IsDefaultUiCamera, Pressed, UiGlobalTransform},
    ui_widgets::{Activate, Button, SelectAllOnFocus, TextInput},
    window::PrimaryWindow,
};
pub(super) use context_menu::{MenuItem, input as context_input, refresh as refresh_context};

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) enum Input {
    Search,
    Title,
    Folder,
}
#[derive(Component)]
pub(super) struct Control {
    action: Action,
    primary: bool,
    pub(super) modal: bool,
}
#[derive(Component)]
pub(super) struct ScrollArea {
    modal: bool,
}
#[derive(Component)]
pub(super) struct Placeholder(Entity);
#[derive(Component)]
pub(super) struct ControlLabel;
#[derive(Resource)]
pub(super) struct Interface {
    font: Handle<Font>,
    root: Entity,
    list: Entity,
    count: Entity,
    status: Entity,
    sort: Entity,
    dialog: Option<Entity>,
    error: Option<Entity>,
    destination: Option<Entity>,
    checkbox: Option<Entity>,
    submit: Option<Entity>,
    list_revision: u64,
    dialog_revision: u64,
    context_root: Option<Entity>,
    context_state: Option<ContextMenu>,
}

fn row(gap: f32) -> Node {
    Node {
        align_items: AlignItems::Center,
        column_gap: px(gap),
        ..default()
    }
}
fn column(gap: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(gap),
        ..default()
    }
}
fn node(commands: &mut Commands, parent: Entity, layout: Node) -> Entity {
    commands
        .spawn((layout, ChildOf(parent), Pickable::IGNORE))
        .id()
}
fn text(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    commands
        .spawn((
            Text::new(value),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}
fn surface(commands: &mut Commands, parent: Entity, layout: Node, color: Color) -> Entity {
    let entity = ui_theme::surface(commands, parent, layout, color);
    commands.entity(entity).insert(Pickable::IGNORE);
    entity
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: Action,
    primary: bool,
    modal: bool,
) -> (Entity, Entity) {
    let glyph = match &action {
        Action::New => Some(Glyph::Plus),
        Action::Import
        | Action::Open(_)
        | Action::PickOpen
        | Action::PickParent
        | Action::Reveal => Some(Glyph::FolderOpen),
        Action::Submit => Some(if label == "Создать" {
            Glyph::Plus
        } else {
            Glyph::FolderOpen
        }),
        Action::Template(Template::Empty) => Some(Glyph::Camera),
        Action::Template(Template::Starter) => Some(Glyph::Box),
        Action::Sort => Some(Glyph::ChevronDown),
        Action::Favorite => Some(Glyph::Star),
        Action::Forget | Action::ClearFilter => Some(Glyph::Close),
        Action::Close if label == "×" => Some(Glyph::Close),
        _ => None,
    };
    let icon_only = label == "×";
    let entity = commands
        .spawn((
            Button,
            Hovered::default(),
            effects::Motion::button(primary),
            Control {
                action,
                primary,
                modal,
            },
            AccessibleLabel(label.into()),
            TabIndex(0),
            Node {
                min_height: px(30),
                min_width: px(30),
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(px(if icon_only { 6 } else { 10 }), px(5)),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                ..row(6.0)
            },
            BackgroundColor(if primary { ACCENT } else { Color::NONE }),
            BackgroundGradient::default(),
            BorderColor::all(Color::NONE),
            ChildOf(parent),
        ))
        .id();
    if let Some(glyph) = glyph {
        let icon = icons::spawn(
            commands,
            entity,
            glyph,
            15.0,
            if primary { ON_ACCENT } else { TEXT },
        );
        commands.entity(icon).insert(effects::Content(entity));
    }
    let label = text(
        commands,
        entity,
        font,
        if icon_only {
            ""
        } else {
            label.trim_start_matches("+ ")
        },
        if primary { 14.0 } else { 13.0 },
        if primary { ON_ACCENT } else { TEXT },
    );
    commands
        .entity(label)
        .insert((TextLayout::no_wrap(), ControlLabel));
    (entity, label)
}
fn input(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    field: Input,
    value: &str,
    placeholder: &str,
) -> Entity {
    let wrap = node(
        commands,
        parent,
        Node {
            min_width: px(0),
            width: percent(100),
            height: px(32),
            flex_grow: 1.0,
            ..default()
        },
    );
    let entity = commands
        .spawn((
            field,
            TextInput,
            EditableText::new(value),
            TextCursorStyle::default(),
            AccessibleLabel(placeholder.into()),
            TabIndex(0),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(13.0),
                ..default()
            },
            TextColor(TEXT),
            TextLayout::no_wrap(),
            Node {
                width: percent(100),
                height: percent(100),
                min_width: px(0),
                min_height: px(0),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                padding: UiRect {
                    left: px(if field == Input::Search { 34 } else { 10 }),
                    right: px(10),
                    top: px(7),
                    bottom: px(7),
                },
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            ChildOf(wrap),
        ))
        .id();
    if field != Input::Search {
        commands.entity(entity).insert(SelectAllOnFocus);
    }
    if field == Input::Search {
        let icon = icons::spawn(commands, wrap, Glyph::Search, 16.0, MUTED);
        commands.entity(icon).insert(Node {
            position_type: PositionType::Absolute,
            left: px(10),
            top: px(8),
            width: px(16),
            height: px(16),
            ..default()
        });
    }
    let hint = text(commands, wrap, font, placeholder, 13.0, MUTED);
    commands.entity(hint).insert((
        Placeholder(entity),
        Node {
            position_type: PositionType::Absolute,
            left: px(if field == Input::Search { 35 } else { 11 }),
            top: px(8),
            ..default()
        },
    ));
    entity
}
fn mark(commands: &mut Commands, parent: Entity, glyph: Glyph, size: f32) -> Entity {
    let badge = surface(
        commands,
        parent,
        Node {
            width: px(size),
            height: px(size),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            justify_content: JustifyContent::Center,
            flex_shrink: 0.0,
            ..row(0.0)
        },
        CHROME,
    );
    commands
        .entity(badge)
        .insert(BorderColor::all(ACCENT.with_alpha(0.36)));
    icons::spawn(commands, badge, glyph, size * 0.52, ACCENT);
    badge
}

pub(super) fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    stars: Res<starfield::Starfield>,
    ambience: Res<effects::Ambience>,
) {
    commands.spawn((Camera2d, IsDefaultUiCamera));
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../assets/font/LiberationSans.ttf").to_vec(),
    ));
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                min_width: px(0),
                min_height: px(0),
                overflow: Overflow::clip(),
                ..column(0.0)
            },
            BackgroundColor(PANEL),
            TabGroup::default(),
            Pickable::IGNORE,
        ))
        .id();
    let header = surface(
        &mut commands,
        root,
        Node {
            padding: UiRect::axes(px(16), px(14)),
            border: UiRect::bottom(px(1)),
            flex_shrink: 0.0,
            ..row(12.0)
        },
        CHROME,
    );
    starfield::background(&mut commands, header, &stars.chrome);
    crate::window_chrome::drag_region(&mut commands, header);
    horizon(&mut commands, header);
    orbital::brand(&mut commands, header);
    let brand = node(&mut commands, header, column(1.0));
    text(&mut commands, brand, &font, "Bevy Editor", 16.0, TEXT);
    text(&mut commands, brand, &font, "МЕНЕДЖЕР ПРОЕКТОВ", 9.0, MUTED);
    node(
        &mut commands,
        header,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    button(
        &mut commands,
        header,
        &font,
        "Открыть…",
        Action::Import,
        false,
        false,
    );
    button(
        &mut commands,
        header,
        &font,
        "+ Новый проект",
        Action::New,
        true,
        false,
    );
    crate::window_chrome::controls(&mut commands, header);
    crate::window_chrome::resize_edges(&mut commands, root);

    let content = node(
        &mut commands,
        root,
        Node {
            width: percent(100),
            min_width: px(0),
            min_height: px(0),
            height: px(0),
            flex_basis: px(0),
            flex_grow: 1.0,
            padding: UiRect::all(px(16)),
            ..column(12.0)
        },
    );
    let toolbar = node(
        &mut commands,
        content,
        Node {
            width: percent(100),
            min_width: px(0),
            flex_shrink: 0.0,
            ..row(16.0)
        },
    );
    commands.entity(content).insert(effects::Reveal::panel());
    let count = text(&mut commands, toolbar, &font, "Проекты", 16.0, TEXT);
    commands.entity(count).insert(Node {
        min_width: px(130),
        ..default()
    });
    let search = node(
        &mut commands,
        toolbar,
        Node {
            min_width: px(100),
            flex_basis: px(0),
            width: px(0),
            flex_grow: 1.0,
            ..default()
        },
    );
    input(&mut commands, search, &font, Input::Search, "", "Поиск…");
    let (_, sort) = button(
        &mut commands,
        toolbar,
        &font,
        "По дате",
        Action::Sort,
        false,
        false,
    );
    let list_frame = surface(
        &mut commands,
        content,
        Node {
            width: percent(100),
            min_width: px(0),
            min_height: px(0),
            height: px(0),
            flex_basis: px(0),
            flex_grow: 1.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
        PANEL,
    );
    starfield::background(&mut commands, list_frame, &stars.chrome);
    effects::background(&mut commands, list_frame, &ambience);
    let list = node(
        &mut commands,
        list_frame,
        Node {
            width: percent(100),
            height: percent(100),
            min_height: px(0),
            padding: UiRect::all(px(8)),
            overflow: Overflow::scroll_y(),
            ..column(4.0)
        },
    );
    commands
        .entity(list)
        .insert((ScrollPosition::default(), ScrollArea { modal: false }));
    let status = text(&mut commands, content, &font, "", 12.0, ERROR);
    commands.entity(status).insert((
        Node {
            display: Display::None,
            max_height: px(40),
            overflow: Overflow::clip(),
            flex_shrink: 0.0,
            ..default()
        },
        TextLayout::linebreak(LineBreak::WordOrCharacter),
    ));
    let status_bar = surface(
        &mut commands,
        root,
        Node {
            height: px(34),
            padding: UiRect::horizontal(px(16)),
            border: UiRect::top(px(1)),
            flex_shrink: 0.0,
            ..row(8.0)
        },
        CHROME,
    );
    icons::spawn(&mut commands, status_bar, Glyph::Folder, 14.0, ACCENT);
    text(&mut commands, status_bar, &font, "Проекты", 11.0, TEXT);
    node(
        &mut commands,
        status_bar,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    let hints = text(
        &mut commands,
        status_bar,
        &font,
        "Ctrl+N — новый   ·   Ctrl+O — открыть   ·   Enter — открыть   ·   ПКМ — действия проекта",
        11.0,
        MUTED,
    );
    commands.entity(hints).insert((
        Node {
            min_width: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
        TextLayout::no_wrap(),
    ));
    commands.insert_resource(Interface {
        font,
        root,
        list,
        count,
        status,
        sort,
        dialog: None,
        error: None,
        destination: None,
        checkbox: None,
        submit: None,
        list_revision: 0,
        dialog_revision: u64::MAX,
        context_root: None,
        context_state: None,
    });
}

pub(super) fn activate(
    event: On<Activate>,
    mut controls: Query<(&Control, &mut effects::Motion), Without<InteractionDisabled>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut actions: MessageWriter<Action>,
) {
    if let Ok((control, mut effect)) = controls.get_mut(event.entity) {
        effect.activate();
        let action = if keys.any_just_pressed([KeyCode::Enter, KeyCode::Space]) {
            match &control.action {
                Action::Select(path) => Action::Open(path.clone()),
                other => other.clone(),
            }
        } else {
            control.action.clone()
        };
        actions.write(action);
    }
}

pub(super) fn inputs(
    mut manager: ResMut<Manager>,
    focus: Res<InputFocus>,
    mut inputs: Query<(Entity, &Input, &mut EditableText)>,
) {
    for (entity, field, mut input) in &mut inputs {
        let value = input.value().to_string();
        match field {
            Input::Search => {
                if focus.get() == Some(entity) && manager.form.is_none() {
                    manager.set_filter(value);
                } else if value != manager.filter {
                    input.editor_mut().set_text(&manager.filter);
                }
            }
            Input::Title => {
                if let Some(form) = &mut manager.form
                    && form.title != value
                {
                    form.title = value;
                    form.error.clear();
                }
            }
            Input::Folder => {
                if let Some(form) = &mut manager.form
                    && form.folder != value
                {
                    form.folder = value;
                    form.error.clear();
                }
            }
        }
    }
}

pub(super) fn refresh(
    mut commands: Commands,
    manager: Res<Manager>,
    picker: Res<Picker>,
    mut ui: ResMut<Interface>,
    mut labels: Query<&mut Text>,
    mut nodes: Query<&mut Node>,
    mut focus: ResMut<InputFocus>,
) {
    if ui.list_revision != manager.revision {
        ui.list_revision = manager.revision;
        commands.entity(ui.list).despawn_children();
        let entries = manager.visible_projects();
        if let Ok(mut count) = labels.get_mut(ui.count) {
            count.0 = format!("Проекты · {}", entries.len());
        }
        if let Ok(mut sort) = labels.get_mut(ui.sort) {
            sort.0 = if manager.alphabetic {
                "По имени"
            } else {
                "По дате"
            }
            .into();
        }
        if entries.is_empty() {
            let empty = node(
                &mut commands,
                ui.list,
                Node {
                    flex_grow: 1.0,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(px(16)),
                    flex_shrink: 0.0,
                    ..column(12.0)
                },
            );
            if manager.filter.trim().is_empty() {
                let orbit = orbital::illustration(&mut commands, empty);
                commands.entity(orbit).insert(effects::Orbit);
            } else {
                mark(&mut commands, empty, Glyph::Search, 56.0);
            }
            text(
                &mut commands,
                empty,
                &ui.font,
                if manager.filter.trim().is_empty() {
                    "Нет проектов"
                } else {
                    "Ничего не найдено"
                },
                20.0,
                TEXT,
            );
            text(
                &mut commands,
                empty,
                &ui.font,
                if manager.filter.trim().is_empty() {
                    "Создайте новый проект или откройте существующий"
                } else {
                    "Попробуйте другое название или путь к проекту"
                },
                13.0,
                MUTED,
            );
        }
        for (index, project) in entries.into_iter().enumerate() {
            let exists = project.file.is_file();
            let entry = commands
                .spawn((
                    Button,
                    Hovered::default(),
                    effects::Motion::card(index),
                    Control {
                        action: Action::Select(project.file.clone()),
                        primary: false,
                        modal: false,
                    },
                    AccessibleLabel(format!(
                        "{}{}",
                        project.title,
                        if exists {
                            ""
                        } else {
                            ", файл не найден"
                        }
                    )),
                    TabIndex(0),
                    Node {
                        min_height: px(72),
                        padding: UiRect::axes(px(12), px(10)),
                        flex_shrink: 0.0,
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(6)),
                        ..row(12.0)
                    },
                    BackgroundColor(Color::NONE),
                    BackgroundGradient::default(),
                    BorderColor::all(Color::NONE),
                    ChildOf(ui.list),
                ))
                .id();
            let badge = mark(&mut commands, entry, Glyph::Layers, 38.0);
            commands.entity(badge).insert(effects::Content(entry));
            let info = node(
                &mut commands,
                entry,
                Node {
                    min_width: px(0),
                    flex_basis: px(0),
                    flex_grow: 1.0,
                    overflow: Overflow::clip(),
                    ..column(7.0)
                },
            );
            let title = text(
                &mut commands,
                info,
                &ui.font,
                &project.title,
                14.0,
                if exists { TEXT } else { MUTED },
            );
            commands.entity(title).insert(TextLayout::no_wrap());
            let path = text(
                &mut commands,
                info,
                &ui.font,
                compact_path(project.file.parent().unwrap_or(&project.file)),
                12.0,
                MUTED,
            );
            commands.entity(path).insert(TextLayout::no_wrap());
            let meta = node(
                &mut commands,
                entry,
                Node {
                    flex_shrink: 0.0,
                    align_items: AlignItems::End,
                    ..column(6.0)
                },
            );
            if project.favorite {
                text(&mut commands, meta, &ui.font, "Избранное", 11.0, ACCENT);
            }
            text(
                &mut commands,
                meta,
                &ui.font,
                if exists {
                    last_opened(project.last_opened)
                } else {
                    "Не найден".into()
                },
                12.0,
                if exists { MUTED } else { ERROR },
            );
        }
    }
    if ui.dialog_revision != manager.dialog_revision {
        ui.dialog_revision = manager.dialog_revision;
        if let Some(dialog) = ui.dialog.take() {
            commands.entity(dialog).despawn();
        }
        ui.error = None;
        ui.destination = None;
        ui.checkbox = None;
        ui.submit = None;
        if let Some(form) = &manager.form {
            build_dialog(&mut commands, &mut ui, form, &mut focus);
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.status) {
        node.display = if manager.status.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
    }
    if let Ok(mut status) = labels.get_mut(ui.status) {
        status.0.clone_from(&manager.status);
    }
    if let Some(form) = &manager.form {
        if let Some(entity) = ui.error {
            if let Ok(mut text) = labels.get_mut(entity) {
                text.0.clone_from(&form.error);
            }
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.display = if form.error.is_empty() {
                    Display::None
                } else {
                    Display::Flex
                };
            }
        }
        if let Some(entity) = ui.destination
            && let Ok(mut text) = labels.get_mut(entity)
        {
            text.0 = form
                .destination()
                .map(|path| compact_path(&path))
                .unwrap_or_else(|error| error.to_string());
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.display = if form.subfolder {
                    Display::Flex
                } else {
                    Display::None
                };
            }
        }
        if let Some(entity) = ui.checkbox
            && let Ok(mut node) = nodes.get_mut(entity)
        {
            node.display = if form.subfolder {
                Display::Flex
            } else {
                Display::None
            };
        }
        if let Some(entity) = ui.submit
            && let Ok(mut text) = labels.get_mut(entity)
        {
            text.0 = if picker.busy() {
                "Выбор папки…"
            } else if form.dialog == Dialog::Create {
                "Создать"
            } else {
                "Открыть"
            }
            .into();
        }
    }
}

fn build_dialog(
    commands: &mut Commands,
    ui: &mut Interface,
    form: &super::Form,
    focus: &mut InputFocus,
) {
    let overlay = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            effects::Backdrop::default(),
            GlobalZIndex(100),
            TabGroup::modal(),
            ChildOf(ui.root),
        ))
        .id();
    ui.dialog = Some(overlay);
    let dialog = surface(
        commands,
        overlay,
        Node {
            width: px(640),
            max_width: percent(94),
            max_height: percent(94),
            padding: UiRect::all(px(24)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            ..column(16.0)
        },
        PANEL,
    );
    commands.entity(dialog).insert(effects::Reveal::dialog());
    let header = node(
        commands,
        dialog,
        Node {
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
    orbital::brand(commands, header);
    let heading = node(
        commands,
        header,
        Node {
            flex_grow: 1.0,
            ..column(5.0)
        },
    );
    text(
        commands,
        heading,
        &ui.font,
        if form.dialog == Dialog::Create {
            "Новый проект"
        } else {
            "Открыть проект"
        },
        21.0,
        TEXT,
    );
    button(commands, header, &ui.font, "×", Action::Close, false, true);
    let body = node(
        commands,
        dialog,
        Node {
            min_height: px(0),
            flex_shrink: 1.0,
            overflow: Overflow::scroll_y(),
            padding: UiRect::right(px(8)),
            ..column(14.0)
        },
    );
    commands
        .entity(body)
        .insert((ScrollPosition::default(), ScrollArea { modal: true }));
    let first;
    if form.dialog == Dialog::Create {
        let field = node(
            commands,
            body,
            Node {
                flex_shrink: 0.0,
                ..column(8.0)
            },
        );
        text(commands, field, &ui.font, "Название", 13.0, TEXT);
        first = input(
            commands,
            field,
            &ui.font,
            Input::Title,
            &form.title,
            "Название",
        );
        let templates = node(
            commands,
            body,
            Node {
                align_items: AlignItems::Stretch,
                flex_shrink: 0.0,
                ..row(12.0)
            },
        );
        for (template, title, description) in [
            (Template::Empty, "Пустой", "Только камера"),
            (Template::Starter, "3D-сцена", "Куб, камера и свет"),
        ] {
            let (card, _) = button(
                commands,
                templates,
                &ui.font,
                title,
                Action::Template(template),
                false,
                true,
            );
            commands.entity(card).insert(Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                padding: UiRect::all(px(16)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..column(10.0)
            });
            commands
                .entity(card)
                .insert(effects::Motion::card(usize::from(
                    template == Template::Starter,
                )));
            text(commands, card, &ui.font, description, 13.0, MUTED);
        }
    } else {
        first = Entity::PLACEHOLDER;
    }
    let field = node(
        commands,
        body,
        Node {
            flex_shrink: 0.0,
            ..column(8.0)
        },
    );
    text(
        commands,
        field,
        &ui.font,
        if form.dialog == Dialog::Create {
            "Папка"
        } else {
            "Папка проекта"
        },
        13.0,
        TEXT,
    );
    let location = node(commands, field, row(8.0));
    let folder = input(
        commands,
        location,
        &ui.font,
        Input::Folder,
        &form.folder,
        if form.dialog == Dialog::Create {
            "Путь к папке"
        } else {
            "Путь к папке или файлу"
        },
    );
    button(
        commands,
        location,
        &ui.font,
        "Выбрать…",
        if form.dialog == Dialog::Create {
            Action::PickParent
        } else {
            Action::PickOpen
        },
        false,
        true,
    );
    if form.dialog == Dialog::Create {
        let checkbox = commands
            .spawn((
                Button,
                Hovered::default(),
                effects::Motion::button(false),
                Control {
                    action: Action::Subfolder,
                    primary: false,
                    modal: true,
                },
                AccessibleLabel("Подпапка проекта".into()),
                TabIndex(0),
                Node {
                    min_height: px(28),
                    ..row(9.0)
                },
                BackgroundColor(PANEL),
                BackgroundGradient::default(),
                BorderColor::all(EDGE),
                ChildOf(body),
            ))
            .id();
        let square = surface(
            commands,
            checkbox,
            Node {
                width: px(16),
                height: px(16),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                justify_content: JustifyContent::Center,
                ..row(0.0)
            },
            WELL,
        );
        let checked = node(
            commands,
            square,
            Node {
                width: px(8),
                height: px(8),
                border_radius: BorderRadius::all(px(1)),
                ..default()
            },
        );
        commands.entity(checked).insert(BackgroundColor(ACCENT));
        text(commands, checkbox, &ui.font, "Подпапка проекта", 13.0, TEXT);
        ui.checkbox = Some(checked);
        let destination = text(commands, body, &ui.font, "", 12.0, MUTED);
        commands.entity(destination).insert((
            TextLayout::linebreak(LineBreak::WordOrCharacter),
            Node {
                width: percent(100),
                min_width: px(0),
                flex_shrink: 0.0,
                ..default()
            },
        ));
        ui.destination = Some(destination);
    }
    let error = text(commands, dialog, &ui.font, "", 13.0, ERROR);
    commands.entity(error).insert((
        TextLayout::linebreak(LineBreak::WordOrCharacter),
        Node {
            width: percent(100),
            min_width: px(0),
            flex_shrink: 0.0,
            ..default()
        },
    ));
    ui.error = Some(error);
    let footer = node(
        commands,
        dialog,
        Node {
            justify_content: JustifyContent::End,
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
    button(
        commands,
        footer,
        &ui.font,
        "Отмена",
        Action::Close,
        false,
        true,
    );
    let (_, submit) = button(
        commands,
        footer,
        &ui.font,
        if form.dialog == Dialog::Create {
            "Создать"
        } else {
            "Открыть"
        },
        Action::Submit,
        true,
        true,
    );
    ui.submit = Some(submit);
    focus.set(
        if form.dialog == Dialog::Create {
            first
        } else {
            folder
        },
        FocusCause::Auto,
    );
}

pub(super) fn styles(
    mut commands: Commands,
    manager: Res<Manager>,
    picker: Res<Picker>,
    focus: Res<InputFocus>,
    mut controls: Query<(
        Entity,
        &Control,
        &Hovered,
        Has<Pressed>,
        &Children,
        Has<MenuItem>,
        &mut effects::Motion,
        &mut TabIndex,
    )>,
    mut fields: Query<
        (
            Entity,
            &Input,
            &mut BorderColor,
            &mut TextReadWriteMode,
            &mut TabIndex,
        ),
        Without<Control>,
    >,
    values: Query<&EditableText>,
    mut labels: Query<&mut TextColor, With<ControlLabel>>,
    mut placeholders: Query<(&Placeholder, &mut Node)>,
) {
    for (entity, control, hovered, pressed, children, menu_item, mut motion, mut tab) in
        &mut controls
    {
        let disabled = (!control.modal && manager.form.is_some())
            || (manager.context.is_some() && !menu_item)
            || (picker.busy() && !matches!(control.action, Action::Close));
        tab.0 = if disabled { -1 } else { 0 };
        if disabled {
            commands.entity(entity).insert(InteractionDisabled);
        } else {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
        let chosen = matches!(&control.action, Action::Select(file) if manager.selected.as_ref() == Some(file))
            || matches!(control.action, Action::Template(template) if manager.form.as_ref().is_some_and(|form| form.template == template))
            || matches!(control.action, Action::Favorite)
                && manager.recent.projects.iter().any(|project| {
                    Some(&project.file) == manager.selected.as_ref() && project.favorite
                });
        let highlighted = !disabled && (hovered.0 || pressed);
        let background = if disabled {
            Color::NONE
        } else if control.primary {
            if highlighted { FOCUS } else { ACCENT }
        } else if chosen {
            ACTIVE
        } else if highlighted {
            HOVER
        } else {
            Color::NONE
        };
        motion.color = background;
        motion.disabled = disabled;
        motion.selected = chosen;
        motion.focused = focus.get() == Some(entity);
        motion.border = if !disabled && focus.get() == Some(entity) {
            FOCUS
        } else if chosen {
            ACCENT.with_alpha(0.42)
        } else {
            Color::NONE
        };
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child) {
                label.set_if_neq(TextColor(if disabled {
                    DISABLED_TEXT
                } else if control.primary {
                    ON_ACCENT
                } else {
                    TEXT
                }));
            }
        }
    }
    for (entity, field, mut border, mut mode, mut tab) in &mut fields {
        let disabled = picker.busy()
            || manager.context.is_some()
            || (*field == Input::Search && manager.form.is_some());
        tab.0 = if disabled { -1 } else { 0 };
        *mode = if disabled {
            TextReadWriteMode::ReadOnly
        } else {
            TextReadWriteMode::Editable
        };
        *border = BorderColor::all(if !disabled && focus.get() == Some(entity) {
            FOCUS
        } else {
            EDGE
        });
    }
    for (placeholder, mut node) in &mut placeholders {
        node.display = if values
            .get(placeholder.0)
            .is_ok_and(|input| input.value().to_string().is_empty())
            && focus.get() != Some(placeholder.0)
        {
            Display::Flex
        } else {
            Display::None
        };
    }
}

pub(super) fn scroll(
    window: Single<&Window, With<PrimaryWindow>>,
    wheel: Res<AccumulatedMouseScroll>,
    manager: Res<Manager>,
    mut areas: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        &mut ScrollPosition,
        &ScrollArea,
    )>,
) {
    if wheel.delta.y == 0.0 {
        return;
    }
    let Some(cursor) = window
        .cursor_position()
        .map(|point| point * window.scale_factor())
    else {
        return;
    };
    for (computed, transform, mut position, area) in &mut areas {
        if area.modal != manager.form.is_some() {
            continue;
        }
        if (cursor - transform.affine().translation)
            .abs()
            .cmple(computed.size() * 0.5)
            .all()
        {
            let maximum = ((computed.content_size().y - computed.size().y)
                * computed.inverse_scale_factor())
            .max(0.0);
            let amount = if wheel.unit == MouseScrollUnit::Line {
                34.0
            } else {
                1.0
            };
            position.y = (position.y - wheel.delta.y * amount).clamp(0.0, maximum);
        }
    }
}

fn last_opened(at: u64) -> String {
    let seconds = projects::timestamp().saturating_sub(at);
    if at == 0 {
        "Ещё не открывался".into()
    } else if seconds < 60 {
        "Только что".into()
    } else if seconds < 3600 {
        format!("{} мин назад", seconds / 60)
    } else if seconds < 86400 {
        format!("{} ч назад", seconds / 3600)
    } else {
        format!("{} дн назад", seconds / 86400)
    }
}

fn compact_path(path: &std::path::Path) -> String {
    if let Ok(home) = projects::home()
        && let Ok(relative) = path.strip_prefix(home)
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    let display = path.to_string_lossy();
    #[cfg(target_os = "windows")]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(_) => return display[4..].to_owned(),
                Prefix::VerbatimUNC(_, _) => return format!(r"\\{}", &display[8..]),
                _ => {}
            }
        }
    }
    display.into_owned()
}

#[cfg(all(test, target_os = "windows"))]
mod path_tests {
    use super::compact_path;
    use std::path::Path;

    #[test]
    fn compact_paths_hide_verbatim_prefixes_and_preserve_network_roots() {
        for (stored, shown) in [
            (r"\\?\C:\Projects\Пример", r"C:\Projects\Пример"),
            (r"\\?\UNC\server\share\Project", r"\\server\share\Project"),
            (r"D:\Projects\Example", r"D:\Projects\Example"),
        ] {
            assert_eq!(compact_path(Path::new(stored)), shown);
        }
    }
}
