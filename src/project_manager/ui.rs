use super::{
    Action, Dialog, Manager,
    native::Picker,
    projects::{self, Template},
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
    ui::{
        BackgroundGradient, ColorStop, InteractionDisabled, IsDefaultUiCamera, LinearGradient,
        Pressed, UiGlobalTransform,
    },
    ui_widgets::{Activate, Button, SelectAllOnFocus, TextInput},
    window::PrimaryWindow,
};

const BG: Color = Color::srgb(0.045, 0.049, 0.065);
const PANEL: Color = Color::srgb(0.075, 0.079, 0.098);
const WELL: Color = Color::srgb(0.035, 0.040, 0.052);
const EDGE: Color = Color::srgb(0.20, 0.20, 0.24);
const TEXT: Color = Color::srgb(0.95, 0.94, 0.91);
const MUTED: Color = Color::srgb(0.67, 0.68, 0.73);
const ACCENT: Color = Color::srgb(0.83, 0.66, 0.43);
const SELECTED: Color = Color::srgb(0.20, 0.17, 0.15);
const ERROR: Color = Color::srgb(1.0, 0.58, 0.48);

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
    selected: bool,
}
#[derive(Component)]
pub(super) struct ScrollArea {
    modal: bool,
}
#[derive(Component)]
pub(super) struct Placeholder(Entity);
#[derive(Resource)]
pub(super) struct Interface {
    font: Handle<Font>,
    root: Entity,
    list: Entity,
    actions: Entity,
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
    let entity = node(commands, parent, layout);
    commands
        .entity(entity)
        .insert((BackgroundColor(color), BorderColor::all(EDGE)));
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
    let entity = commands
        .spawn((
            Button,
            Hovered::default(),
            Control {
                action,
                primary,
                modal,
                selected: false,
            },
            AccessibleLabel(label.into()),
            TabIndex(0),
            Node {
                min_height: px(38),
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(px(14), px(9)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                ..row(0.0)
            },
            BackgroundColor(if primary { ACCENT } else { PANEL }),
            BorderColor::all(EDGE),
            ChildOf(parent),
        ))
        .id();
    let label = text(
        commands,
        entity,
        font,
        label,
        14.0,
        if primary { WELL } else { TEXT },
    );
    commands.entity(label).insert(TextLayout::no_wrap());
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
            height: px(40),
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
                font_size: FontSize::Px(14.0),
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
                padding: UiRect::axes(px(12), px(10)),
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
    let hint = text(commands, wrap, font, placeholder, 14.0, MUTED);
    commands.entity(hint).insert((
        Placeholder(entity),
        Node {
            position_type: PositionType::Absolute,
            left: px(13),
            top: px(11),
            ..default()
        },
    ));
    entity
}
fn mark(commands: &mut Commands, parent: Entity, font: &Handle<Font>, name: &str, size: f32) {
    let badge = surface(
        commands,
        parent,
        Node {
            width: px(size),
            height: px(size),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(size * 0.22)),
            justify_content: JustifyContent::Center,
            flex_shrink: 0.0,
            ..row(0.0)
        },
        SELECTED,
    );
    commands
        .entity(badge)
        .insert(BackgroundGradient::from(LinearGradient::new(
            LinearGradient::TO_BOTTOM_RIGHT,
            vec![
                ColorStop::auto(Color::srgb(0.28, 0.22, 0.20)),
                ColorStop::auto(Color::srgb(0.11, 0.12, 0.18)),
            ],
        )));
    let initial = name
        .trim()
        .chars()
        .next()
        .unwrap_or('B')
        .to_uppercase()
        .to_string();
    text(commands, badge, font, initial, size * 0.46, ACCENT);
}

pub(super) fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
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
            BackgroundColor(BG),
            TabGroup::default(),
            Pickable::IGNORE,
        ))
        .id();
    let header = surface(
        &mut commands,
        root,
        Node {
            padding: UiRect::axes(px(24), px(18)),
            border: UiRect::bottom(px(1)),
            flex_shrink: 0.0,
            ..row(12.0)
        },
        PANEL,
    );
    mark(&mut commands, header, &font, "B", 36.0);
    text(&mut commands, header, &font, "Bevy Editor", 20.0, TEXT);
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
            padding: UiRect::all(px(24)),
            ..column(16.0)
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
    let count = text(&mut commands, toolbar, &font, "Проекты", 20.0, TEXT);
    commands.entity(count).insert(Node {
        min_width: px(150),
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
    let list = surface(
        &mut commands,
        content,
        Node {
            width: percent(100),
            min_width: px(0),
            min_height: px(0),
            height: px(0),
            flex_basis: px(0),
            flex_grow: 1.0,
            padding: UiRect::all(px(8)),
            overflow: Overflow::scroll_y(),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            ..column(6.0)
        },
        WELL,
    );
    commands
        .entity(list)
        .insert((ScrollPosition::default(), ScrollArea { modal: false }));
    let footer = node(
        &mut commands,
        content,
        Node {
            width: percent(100),
            min_width: px(0),
            flex_shrink: 0.0,
            ..row(12.0)
        },
    );
    button(
        &mut commands,
        footer,
        &font,
        "Пример",
        Action::Example,
        false,
        false,
    );
    let actions = node(
        &mut commands,
        footer,
        Node {
            min_width: px(0),
            flex_grow: 1.0,
            justify_content: JustifyContent::End,
            ..row(8.0)
        },
    );
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
    commands.insert_resource(Interface {
        font,
        root,
        list,
        actions,
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
    });
}

pub(super) fn activate(
    event: On<Activate>,
    controls: Query<&Control>,
    keys: Res<ButtonInput<KeyCode>>,
    mut actions: MessageWriter<Action>,
) {
    if let Ok(control) = controls.get(event.entity) {
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
        commands.entity(ui.actions).despawn_children();
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
                    ..column(16.0)
                },
            );
            mark(&mut commands, empty, &ui.font, "B", 56.0);
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
                MUTED,
            );
        }
        for project in entries {
            let exists = project.file.is_file();
            let selected = manager.selected.as_ref() == Some(&project.file);
            let entry = commands
                .spawn((
                    Button,
                    Hovered::default(),
                    Control {
                        action: Action::Select(project.file.clone()),
                        primary: false,
                        modal: false,
                        selected,
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
                        min_height: px(78),
                        padding: UiRect::all(px(14)),
                        flex_shrink: 0.0,
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(7)),
                        ..row(14.0)
                    },
                    BackgroundColor(WELL),
                    BorderColor::all(WELL),
                    ChildOf(ui.list),
                ))
                .id();
            mark(&mut commands, entry, &ui.font, &project.title, 40.0);
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
                16.0,
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
        if let Some(project) = manager
            .visible_projects()
            .into_iter()
            .find(|project| Some(&project.file) == manager.selected.as_ref())
        {
            button(
                &mut commands,
                ui.actions,
                &ui.font,
                if project.favorite {
                    "Избранное"
                } else {
                    "В избранное"
                },
                Action::Favorite,
                false,
                false,
            );
            button(
                &mut commands,
                ui.actions,
                &ui.font,
                "Папка",
                Action::Reveal,
                false,
                false,
            );
            button(
                &mut commands,
                ui.actions,
                &ui.font,
                "Убрать из списка",
                Action::Forget,
                false,
                false,
            );
            button(
                &mut commands,
                ui.actions,
                &ui.font,
                "Открыть",
                Action::Open(project.file.clone()),
                true,
                false,
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
            BackgroundColor(Color::srgba(0.01, 0.012, 0.02, 0.84)),
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
            border_radius: BorderRadius::all(px(14)),
            ..column(16.0)
        },
        PANEL,
    );
    let header = node(
        commands,
        dialog,
        Node {
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
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
        25.0,
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
            (Template::Starter, "3D-сцена", "Куб, пол и свет"),
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
                Control {
                    action: Action::Subfolder,
                    primary: false,
                    modal: true,
                    selected: false,
                },
                AccessibleLabel("Подпапка проекта".into()),
                TabIndex(0),
                Node {
                    min_height: px(28),
                    ..row(9.0)
                },
                BackgroundColor(PANEL),
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
        &mut BackgroundColor,
        &mut BorderColor,
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
    mut placeholders: Query<(&Placeholder, &mut Node)>,
) {
    for (entity, control, hovered, pressed, mut color, mut border, mut tab) in &mut controls {
        let disabled = (!control.modal && manager.form.is_some())
            || (picker.busy() && !matches!(control.action, Action::Close));
        tab.0 = if disabled { -1 } else { 0 };
        if disabled {
            commands.entity(entity).insert(InteractionDisabled);
        } else {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
        let chosen = control.selected
            || matches!(control.action, Action::Template(template) if manager.form.as_ref().is_some_and(|form| form.template == template))
            || matches!(control.action, Action::Favorite)
                && manager.recent.projects.iter().any(|project| {
                    Some(&project.file) == manager.selected.as_ref() && project.favorite
                });
        let highlighted = !disabled && (hovered.0 || pressed);
        color.0 = if control.primary {
            if highlighted {
                Color::srgb(0.96, 0.78, 0.53)
            } else {
                ACCENT
            }
        } else if chosen {
            SELECTED
        } else if highlighted {
            Color::srgb(0.14, 0.14, 0.18)
        } else if matches!(control.action, Action::Select(_)) {
            WELL
        } else {
            PANEL
        };
        *border = BorderColor::all(if chosen || focus.get() == Some(entity) {
            ACCENT
        } else if matches!(control.action, Action::Select(_)) && !highlighted {
            WELL
        } else {
            EDGE
        });
    }
    for (entity, field, mut border, mut mode, mut tab) in &mut fields {
        let disabled = picker.busy() || (*field == Input::Search && manager.form.is_some());
        tab.0 = if disabled { -1 } else { 0 };
        *mode = if disabled {
            TextReadWriteMode::ReadOnly
        } else {
            TextReadWriteMode::Editable
        };
        *border = BorderColor::all(if focus.get() == Some(entity) {
            ACCENT
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
    path.to_string_lossy().into_owned()
}
