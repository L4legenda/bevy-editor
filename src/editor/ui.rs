use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    input_focus::{
        InputFocus,
        tab_navigation::{TabGroup, TabIndex},
    },
    picking::hover::Hovered,
    prelude::*,
    text::{EditableText, TextCursorStyle, TextReadWriteMode},
    ui::{ComputedNode, Pressed, UiGlobalTransform},
    ui_widgets::{Activate, Button, TextInput},
    window::PrimaryWindow,
};

use super::Room;
use crate::project::LensKind;

use super::{Action, Editor, Field, Selection, Tool, thumbnails::Thumbnails, viewport};

pub(super) const TOP: f32 = 62.0;
pub(super) const SIDE: f32 = 316.0;
pub(super) const DRAWER: f32 = 163.0;
pub(super) const FOOT: f32 = 32.0;

const CHROME: Color = Color::srgb(0.132, 0.137, 0.160);
const PANEL: Color = Color::srgb(0.106, 0.110, 0.130);
const WELL: Color = Color::srgb(0.080, 0.084, 0.102);
const EDGE: Color = Color::srgb(0.214, 0.227, 0.263);
const HOVER: Color = Color::srgb(0.181, 0.193, 0.228);
const ACTIVE: Color = Color::srgb(0.190, 0.295, 0.474);
const ACCENT: Color = Color::srgb(0.140, 0.405, 0.810);
const TEXT: Color = Color::srgb(0.930, 0.944, 0.973);
const MUTED: Color = Color::srgb(0.660, 0.694, 0.760);
const BLUE: Color = Color::srgb(0.510, 0.729, 1.0);
const GREEN: Color = Color::srgb(0.476, 0.803, 0.674);
const AMBER: Color = Color::srgb(1.0, 0.775, 0.433);
const AXIS: [Color; 3] = [
    Color::srgb(1.0, 0.54, 0.54),
    Color::srgb(0.53, 0.88, 0.64),
    BLUE,
];

#[derive(Clone, Copy)]
enum Look {
    Regular,
    Primary,
    Outline,
    Asset,
}

#[derive(Component)]
pub(super) struct Control {
    action: Action,
    look: Look,
}

#[derive(Component)]
pub(super) struct Scroller;

#[derive(Resource)]
pub(super) struct Interface {
    font: Handle<Font>,
    outliner: Entity,
    body: Entity,
    drawer: Entity,
    status: Entity,
    dirty: Entity,
    selected: Entity,
    kind: Entity,
    perspective: Entity,
    lens_label: Entity,
    transforms: Entity,
    scale_row: Entity,
    lens_row: Entity,
}

#[derive(Default)]
pub(super) struct OutlineState {
    room: Option<Room>,
    imported: usize,
    models: Vec<u32>,
}

pub(super) fn viewport_rect(window: &Window, drawer: bool) -> Rect {
    Rect::from_corners(
        Vec2::new(0.0, TOP),
        Vec2::new(
            window.width() - SIDE,
            window.height() - FOOT - if drawer { DRAWER } else { 0.0 },
        ),
    )
}

fn node(commands: &mut Commands, parent: Entity, node: Node) -> Entity {
    commands.spawn((node, ChildOf(parent))).id()
}

fn surface(commands: &mut Commands, parent: Entity, node: Node, color: Color) -> Entity {
    commands
        .spawn((
            node,
            BackgroundColor(color),
            BorderColor::all(EDGE),
            ChildOf(parent),
        ))
        .id()
}

fn font(handle: &Handle<Font>, size: f32) -> TextFont {
    TextFont::from_font_size(size).with_font(handle.clone())
}

fn text(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    label: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    commands
        .spawn((
            Text::new(label),
            font(handle, size),
            TextColor(color),
            TextLayout::no_wrap(),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
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

fn group(commands: &mut Commands, parent: Entity) -> Entity {
    surface(
        commands,
        parent,
        Node {
            padding: UiRect::all(px(3)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            flex_shrink: 0.0,
            ..row(2.0)
        },
        WELL,
    )
}

fn control(
    commands: &mut Commands,
    parent: Entity,
    action: Action,
    look: Look,
    node: Node,
) -> Entity {
    commands
        .spawn((
            Button,
            Hovered::default(),
            Control { action, look },
            node,
            BackgroundColor(Color::NONE),
            BorderColor::all(EDGE),
            ChildOf(parent),
        ))
        .observe(activate_control)
        .id()
}

fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: impl Into<String>,
    action: Action,
    look: Look,
) -> Entity {
    let outline = matches!(look, Look::Outline);
    let entity = control(
        commands,
        parent,
        action,
        look,
        Node {
            width: if outline { percent(100) } else { Val::Auto },
            height: px(if outline { 27 } else { 29 }),
            padding: UiRect::horizontal(px(9)),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(6)),
            overflow: Overflow::clip(),
            ..row(4.0)
        },
    );
    let label = label.into();
    commands
        .entity(entity)
        .insert(AccessibleLabel(label.clone()));
    text(commands, entity, font, label, 13.0, TEXT);
    entity
}

pub(super) fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    scene_surface: Res<viewport::Surface>,
    previews: Res<Thumbnails>,
    editor: Res<Editor>,
) {
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../assets/font/LiberationSans.ttf").to_vec(),
    ));
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                ..column(0.0)
            },
            Pickable::IGNORE,
            TabGroup::default(),
        ))
        .id();
    let toolbar = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            top: px(0),
            left: px(0),
            right: px(0),
            height: px(TOP),
            padding: UiRect::horizontal(px(12)),
            border: UiRect::bottom(px(1)),
            ..row(6.0)
        },
        CHROME,
    );
    let brand = node(
        &mut commands,
        toolbar,
        Node {
            flex_shrink: 0.0,
            ..row(8.0)
        },
    );
    let mark = surface(
        &mut commands,
        brand,
        Node {
            width: px(31),
            height: px(31),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        ACCENT,
    );
    text(&mut commands, mark, &font, "B", 19.0, TEXT);
    let name = node(&mut commands, brand, column(0.0));
    text(
        &mut commands,
        name,
        &font,
        &editor.project.title,
        16.0,
        TEXT,
    );
    text(&mut commands, name, &font, "РЕДАКТОР ЛОКАЦИЙ", 9.0, MUTED);

    let rooms = group(&mut commands, toolbar);
    for (room, definition) in editor.project.scenes.iter().enumerate() {
        button(
            &mut commands,
            rooms,
            &font,
            &definition.label,
            Action::Open(room),
            Look::Regular,
        );
    }
    let tools = group(&mut commands, toolbar);
    for (label, action) in [
        ("W  Двигать", Action::Tool(Tool::Move)),
        ("E  Поворот", Action::Tool(Tool::Rotate)),
        ("R  Масштаб", Action::Tool(Tool::Scale)),
        ("Привязка", Action::Snap),
    ] {
        button(&mut commands, tools, &font, label, action, Look::Regular);
    }
    node(
        &mut commands,
        toolbar,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    for (label, action) in [("Отмена", Action::Undo), ("Повтор", Action::Redo)] {
        button(&mut commands, toolbar, &font, label, action, Look::Regular);
    }
    button(
        &mut commands,
        toolbar,
        &font,
        "Играть",
        Action::Play,
        Look::Regular,
    );
    button(
        &mut commands,
        toolbar,
        &font,
        "Сохранить",
        Action::Save,
        Look::Primary,
    );

    let body = node(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            top: px(TOP),
            bottom: px(DRAWER + FOOT),
            left: px(0),
            right: px(0),
            min_height: px(0),
            ..default()
        },
    );
    let viewport = node(
        &mut commands,
        body,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            ..default()
        },
    );
    commands.entity(viewport).insert(Pickable::IGNORE);
    commands.spawn((
        ImageNode::new(scene_surface.0.clone()),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(viewport),
    ));
    let badge = surface(
        &mut commands,
        viewport,
        Node {
            position_type: PositionType::Absolute,
            top: px(14),
            left: px(14),
            padding: UiRect::axes(px(12), px(7)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            ..default()
        },
        Color::srgba(0.082, 0.090, 0.112, 0.91),
    );
    commands.entity(badge).insert(Pickable::IGNORE);
    let perspective = text(
        &mut commands,
        badge,
        &font,
        "Перспектива · Свет сцены",
        12.0,
        TEXT,
    );

    let side = surface(
        &mut commands,
        body,
        Node {
            width: px(SIDE),
            min_height: px(0),
            flex_shrink: 0.0,
            border: UiRect::left(px(1)),
            ..column(0.0)
        },
        PANEL,
    );
    let header = surface(
        &mut commands,
        side,
        Node {
            height: px(37),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(13)),
            border: UiRect::bottom(px(1)),
            ..row(0.0)
        },
        CHROME,
    );
    text(&mut commands, header, &font, "Дерево сцены", 14.0, TEXT);
    let outliner = node(
        &mut commands,
        side,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            padding: UiRect::axes(px(6), px(5)),
            ..column(2.0)
        },
    );
    commands.entity(outliner).insert(Scroller);

    let inspector = surface(
        &mut commands,
        side,
        Node {
            height: px(282),
            flex_shrink: 0.0,
            padding: UiRect::all(px(12)),
            border: UiRect::top(px(1)),
            overflow: Overflow::clip(),
            ..column(6.0)
        },
        CHROME,
    );
    text(&mut commands, inspector, &font, "Инспектор", 13.0, MUTED);
    let selected = text(
        &mut commands,
        inspector,
        &font,
        "Выберите объект",
        17.0,
        TEXT,
    );
    let kind = text(&mut commands, inspector, &font, "", 12.0, MUTED);
    let transforms = node(&mut commands, inspector, column(6.0));
    transform_row(
        &mut commands,
        transforms,
        &font,
        "Позиция",
        Field::Position,
        0,
    );
    transform_row(
        &mut commands,
        transforms,
        &font,
        "Поворот",
        Field::Rotation,
        3,
    );
    let scale_row = transform_row(&mut commands, transforms, &font, "Масштаб", Field::Scale, 6);
    let lens_row = node(&mut commands, transforms, row(4.0));
    let lens_label = field_caption(&mut commands, lens_row, &font, "Обзор, °");
    number(&mut commands, lens_row, &font, Field::Lens, 9, "·", BLUE);
    let actions = node(
        &mut commands,
        transforms,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(3.0)
        },
    );
    for (label, action) in [
        ("Фокус", Action::Focus),
        ("Сброс", Action::Reset),
        ("Кадр", Action::Pilot),
        ("Из вида", Action::Align),
        ("Дублировать", Action::Duplicate),
        ("Удалить", Action::Delete),
    ] {
        button(&mut commands, actions, &font, label, action, Look::Regular);
    }
    text(
        &mut commands,
        transforms,
        &font,
        "Enter — применить число",
        11.0,
        MUTED,
    );

    let drawer = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(FOOT),
            left: px(0),
            right: px(0),
            height: px(DRAWER),
            border: UiRect::top(px(1)),
            padding: UiRect::axes(px(13), px(8)),
            ..column(7.0)
        },
        PANEL,
    );
    let catalog_title = node(&mut commands, drawer, row(10.0));
    text(
        &mut commands,
        catalog_title,
        &font,
        "Каталог моделей",
        15.0,
        TEXT,
    );
    text(
        &mut commands,
        catalog_title,
        &font,
        format!(
            "{} моделей · клик добавляет экземпляр",
            editor.project.catalog.len()
        ),
        12.0,
        MUTED,
    );
    let assets = node(
        &mut commands,
        drawer,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::Start,
            row_gap: px(7),
            ..row(7.0)
        },
    );
    commands.entity(assets).insert(Scroller);
    for (index, item) in editor.project.catalog.iter().enumerate() {
        let card = control(
            &mut commands,
            assets,
            Action::Add(index),
            Look::Asset,
            Node {
                width: px(148),
                height: px(110),
                flex_shrink: 0.0,
                overflow: Overflow::clip(),
                padding: UiRect::axes(px(7), px(5)),
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..column(2.0)
            },
        );
        commands.spawn((
            ImageNode::new(previews.images[index].clone()),
            Node {
                width: px(132),
                height: px(66),
                border_radius: BorderRadius::all(px(5)),
                flex_shrink: 0.0,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(card),
        ));
        text(
            &mut commands,
            card,
            &font,
            format!("{}  +", item.category),
            10.0,
            BLUE,
        );
        commands
            .entity(card)
            .insert(AccessibleLabel(format!("Добавить {}", item.label)));
        text(&mut commands, card, &font, &item.label, 12.0, TEXT);
    }

    let footer = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(0),
            left: px(0),
            right: px(0),
            height: px(FOOT),
            padding: UiRect::horizontal(px(8)),
            border: UiRect::top(px(1)),
            overflow: Overflow::clip(),
            ..row(8.0)
        },
        CHROME,
    );
    button(
        &mut commands,
        footer,
        &font,
        "Каталог",
        Action::Drawer,
        Look::Regular,
    );
    let dirty = text(&mut commands, footer, &font, "●", 11.0, GREEN);
    let status_cell = node(
        &mut commands,
        footer,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..row(0.0)
        },
    );
    let status = text(&mut commands, status_cell, &font, "", 12.0, MUTED);
    text(
        &mut commands,
        footer,
        &font,
        "ПКМ + WASD · полёт    Alt + ЛКМ · орбита    F · фокус    Ctrl+S · сохранить",
        12.0,
        MUTED,
    );
    commands.insert_resource(Interface {
        font,
        outliner,
        body,
        drawer,
        status,
        dirty,
        selected,
        kind,
        perspective,
        lens_label,
        transforms,
        scale_row,
        lens_row,
    });
}

fn field_caption(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
) -> Entity {
    let cell = node(
        commands,
        parent,
        Node {
            width: px(70),
            flex_shrink: 0.0,
            ..default()
        },
    );
    text(commands, cell, font, label, 12.0, MUTED)
}

fn transform_row(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    field: fn(usize) -> Field,
    first_tab: i32,
) -> Entity {
    let line = node(commands, parent, row(4.0));
    field_caption(commands, line, font, label);
    for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        number(
            commands,
            line,
            font,
            field(index),
            first_tab + index as i32,
            axis,
            AXIS[index],
        );
    }
    line
}

fn number(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    field: Field,
    tab: i32,
    axis: &str,
    accent: Color,
) {
    let cell = node(
        commands,
        parent,
        Node {
            width: px(69),
            ..row(3.0)
        },
    );
    text(commands, cell, font_handle, axis, 11.0, accent);
    commands.spawn((
        field,
        TextInput,
        AccessibleLabel(match field {
            Field::Position(_) => format!("Позиция {axis}"),
            Field::Rotation(_) => format!("Поворот {axis}"),
            Field::Scale(_) => format!("Масштаб {axis}"),
            Field::Lens => "Объектив камеры".to_owned(),
        }),
        EditableText::new("0.00"),
        TextCursorStyle::default(),
        font(font_handle, 12.0),
        TextColor(TEXT),
        TextLayout::no_wrap(),
        TabIndex(tab),
        BackgroundColor(WELL),
        BorderColor::all(EDGE),
        ChildOf(cell),
        Node {
            width: px(55),
            height: px(27),
            padding: UiRect::axes(px(4), px(3)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
}

fn available(editor: &Editor, action: Action) -> bool {
    match action {
        Action::Save => editor.ready,
        Action::Play => editor.ready && editor.definition().play.is_some(),
        Action::Undo => !editor.undo.is_empty(),
        Action::Redo => !editor.redo.is_empty(),
        Action::Add(_) => editor.ready,
        Action::Pilot => matches!(editor.selection, Some(Selection::Camera(_))),
        Action::Align => matches!(editor.selection, Some(Selection::Camera(_))) && !editor.pilot,
        Action::Focus => editor.ready,
        Action::Reset => matches!(
            editor.selection,
            Some(Selection::Camera(_) | Selection::Imported(_))
        ),
        Action::Delete | Action::Duplicate => matches!(editor.selection, Some(Selection::Model(_))),
        _ => true,
    }
}

pub(super) fn input(
    editor: Res<Editor>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<InputFocus>,
) {
    if mouse.any_just_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle])
        && window
            .cursor_position()
            .is_some_and(|cursor| viewport_rect(&window, editor.drawer).contains(cursor))
    {
        focus.clear();
    }
}

fn activate_control(
    activate: On<Activate>,
    editor: Res<Editor>,
    controls: Query<&Control>,
    mut focus: ResMut<InputFocus>,
    mut messages: MessageWriter<Action>,
) {
    if let Ok(control) = controls.get(activate.entity)
        && available(&editor, control.action)
    {
        focus.clear();
        messages.write(control.action);
    }
}

pub(super) fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<InputFocus>,
    numbers: Query<(&Field, &EditableText)>,
    mut actions: MessageWriter<Action>,
    mut editor: ResMut<Editor>,
) {
    if let Some(entity) = focus.get()
        && let Ok((&field, input)) = numbers.get(entity)
    {
        if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
            let value = input.value().to_string().trim().replace(',', ".");
            if let Ok(value) = value.parse::<f32>()
                && value.is_finite()
            {
                actions.write(Action::Number(field, value));
                focus.clear();
            } else {
                editor.status = "Введите число и нажмите Enter".into();
            }
        } else if keys.just_pressed(KeyCode::Escape) {
            focus.clear();
        }
        return;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if command {
        for (key, action) in [
            (KeyCode::KeyS, Action::Save),
            (
                KeyCode::KeyZ,
                if shift { Action::Redo } else { Action::Undo },
            ),
            (KeyCode::KeyY, Action::Redo),
            (KeyCode::Space, Action::Drawer),
            (KeyCode::KeyD, Action::Duplicate),
        ] {
            if keys.just_pressed(key) {
                actions.write(action);
            }
        }
    } else if !mouse.pressed(MouseButton::Right) {
        for (key, action) in [
            (KeyCode::KeyW, Action::Tool(Tool::Move)),
            (KeyCode::KeyE, Action::Tool(Tool::Rotate)),
            (KeyCode::KeyR, Action::Tool(Tool::Scale)),
            (KeyCode::KeyF, Action::Focus),
            (KeyCode::Delete, Action::Delete),
        ] {
            if keys.just_pressed(key) {
                actions.write(action);
            }
        }
        if editor.pilot && keys.just_pressed(KeyCode::Escape) {
            actions.write(Action::Pilot);
        }
    }
}

pub(super) fn scroll(
    window: Single<&Window, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut areas: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<Scroller>>,
) {
    if scroll.delta.y == 0.0 {
        return;
    }
    let Some(cursor) = window
        .cursor_position()
        .map(|pos| pos * window.scale_factor())
    else {
        return;
    };
    for (computed, transform, mut offset) in &mut areas {
        let center = transform.affine().translation;
        if computed.size().min_element() <= 0.0
            || (cursor - center).abs().cmpgt(computed.size() * 0.5).any()
        {
            continue;
        }
        let amount = if scroll.unit == MouseScrollUnit::Line {
            33.0
        } else {
            1.0
        };
        let max = ((computed.content_size().y - computed.size().y)
            * computed.inverse_scale_factor())
        .max(0.0);
        offset.y = (offset.y - scroll.delta.y * amount).clamp(0.0, max);
    }
}

pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    interface: Res<Interface>,
    focus: Res<InputFocus>,
    mut outline: Local<OutlineState>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
    mut panes: Query<&mut Node, Without<Control>>,
    mut buttons: Query<(
        &Control,
        &Hovered,
        Has<Pressed>,
        &mut BackgroundColor,
        &mut Node,
    )>,
    mut fields: Query<(
        Entity,
        &Field,
        &mut EditableText,
        &mut BorderColor,
        &mut TextReadWriteMode,
    )>,
) {
    let models: Vec<_> = editor.edits.models.iter().map(|model| model.id).collect();
    if outline.room != Some(editor.room)
        || outline.imported != editor.imported.len()
        || outline.models != models
    {
        *outline = OutlineState {
            room: Some(editor.room),
            imported: editor.imported.len(),
            models,
        };
        commands
            .entity(interface.outliner)
            .despawn_related::<Children>();
        let heading = |commands: &mut Commands, label| {
            let section = node(
                commands,
                interface.outliner,
                Node {
                    height: px(25),
                    padding: UiRect::horizontal(px(8)),
                    flex_shrink: 0.0,
                    ..row(0.0)
                },
            );
            text(commands, section, &interface.font, label, 11.0, MUTED);
        };
        heading(&mut commands, "КАМЕРЫ");
        for shot in viewport::shots(&editor) {
            let target = Selection::Camera(shot);
            button(
                &mut commands,
                interface.outliner,
                &interface.font,
                format!("·    {}", editor.label(target)),
                Action::Select(target),
                Look::Outline,
            );
        }
        heading(&mut commands, "ОБЪЕКТЫ КОМНАТЫ");
        for index in 0..editor.imported.len() {
            let target = Selection::Imported(index);
            button(
                &mut commands,
                interface.outliner,
                &interface.font,
                format!("·    {}", editor.label(target)),
                Action::Select(target),
                Look::Outline,
            );
        }
        if !editor.edits.models.is_empty() {
            heading(&mut commands, "ЭКЗЕМПЛЯРЫ КАТАЛОГА");
            for model in &editor.edits.models {
                let target = Selection::Model(model.id);
                button(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    format!("+   {}", editor.label(target)),
                    Action::Select(target),
                    Look::Outline,
                );
            }
        }
    }

    let mut set_label = |entity, content: &str, color| {
        if let Ok((mut text, mut tint)) = labels.get_mut(entity) {
            if text.0 != content {
                text.0 = content.to_owned();
            }
            tint.set_if_neq(TextColor(color));
        }
    };
    set_label(interface.status, &editor.status, MUTED);
    set_label(
        interface.dirty,
        if editor.dirty() {
            "● Изменено"
        } else {
            "● Сохранено"
        },
        if editor.dirty() { AMBER } else { GREEN },
    );
    let camera = matches!(editor.selection, Some(Selection::Camera(_)));
    let filling = matches!(editor.selection, Some(Selection::Camera(shot)) if editor.definition().cameras[shot].projection == LensKind::Orthographic);
    let perspective = if editor.pilot {
        editor.selection.map_or(String::new(), |target| {
            format!("Кадр · {} · Esc", editor.label(target))
        })
    } else {
        "Перспектива · Свет сцены".to_owned()
    };
    set_label(interface.perspective, &perspective, TEXT);
    set_label(
        interface.lens_label,
        if filling {
            "Высота, м"
        } else {
            "Обзор, °"
        },
        MUTED,
    );
    if let Some(target) = editor.selection {
        set_label(interface.selected, &editor.label(target), TEXT);
        let kind = match target {
            Selection::Camera(_) => "Игровой ракурс",
            Selection::Imported(_) if !editor.field_allowed(target, Field::Position(1)) => {
                "Ограничения трансформации проекта"
            }
            Selection::Imported(_) => "Объект исходной сцены",
            Selection::Model(_) => "Экземпляр общей модели",
        };
        set_label(interface.kind, kind, MUTED);
    } else {
        set_label(interface.selected, "Выберите объект или камеру", MUTED);
        set_label(interface.kind, "В 3D-виде или в дереве сцены", MUTED);
    }
    if let Ok(mut body) = panes.get_mut(interface.body) {
        let bottom = px(FOOT + if editor.drawer { DRAWER } else { 0.0 });
        if body.bottom != bottom {
            body.bottom = bottom;
        }
    }
    for (entity, shown) in [
        (interface.drawer, editor.drawer),
        (interface.transforms, editor.selection.is_some()),
        (interface.lens_row, camera),
        (interface.scale_row, !camera),
    ] {
        if let Ok(mut pane) = panes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if pane.display != display {
                pane.display = display;
            }
        }
    }
    for (control, hovered, pressed, mut tint, mut node) in &mut buttons {
        let active = match control.action {
            Action::Open(room) => editor.room == room,
            Action::Select(target) => editor.selection == Some(target),
            Action::Tool(tool) => editor.tool == tool,
            Action::Snap => editor.snap,
            Action::Drawer => editor.drawer,
            Action::Pilot => editor.pilot,
            _ => false,
        };
        let visible = match control.action {
            Action::Pilot | Action::Align => camera,
            Action::Play => editor.definition().play.is_some(),
            Action::Duplicate | Action::Delete => {
                matches!(editor.selection, Some(Selection::Model(_)))
            }
            Action::Reset => !matches!(editor.selection, Some(Selection::Model(_))),
            _ => true,
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let color = if !available(&editor, control.action) {
            Color::NONE
        } else if matches!(control.look, Look::Primary) {
            if hovered.0 { BLUE } else { ACCENT }
        } else if active {
            ACTIVE
        } else if hovered.0 || pressed {
            HOVER
        } else if matches!(control.look, Look::Asset) {
            CHROME
        } else {
            Color::NONE
        };
        tint.set_if_neq(BackgroundColor(color));
    }
    for (entity, &field, mut text, mut border, mut mode) in &mut fields {
        let allowed = editor
            .selection
            .is_some_and(|target| editor.field_allowed(target, field));
        let focused = focus.get() == Some(entity) && allowed;
        border.set_if_neq(BorderColor::all(if focused {
            BLUE
        } else if allowed {
            EDGE
        } else {
            WELL
        }));
        mode.set_if_neq(if allowed {
            TextReadWriteMode::Editable
        } else {
            TextReadWriteMode::ReadOnly
        });
        if !focused {
            let value = editor
                .selection
                .and_then(|target| editor.field_value(target, field));
            let display = value.map_or_else(|| "—".into(), |value| format!("{value:.2}"));
            if text.value().to_string() != display {
                text.editor_mut().set_text(&display);
            }
        }
    }
}
