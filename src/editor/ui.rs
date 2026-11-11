use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    input_focus::{
        InputFocus,
        tab_navigation::{TabGroup, TabIndex},
    },
    picking::hover::Hovered,
    prelude::*,
    text::{EditableText, TextCursorStyle, TextReadWriteMode},
    ui::{ComputedNode, InteractionDisabled, Pressed, UiGlobalTransform},
    ui_widgets::{Activate, Button, TextInput},
    window::PrimaryWindow,
};

use super::Room;
use crate::project::LensKind;

use super::{Action, Editor, Field, Selection, Tool, browser::Browser, starfield, viewport};

#[path = "button_fx.rs"]
mod button_fx;
#[path = "camera_preview.rs"]
mod camera_preview;
#[path = "confirmation.rs"]
mod confirmation;
#[path = "control_hints.rs"]
mod control_hints;
#[path = "drawer.rs"]
mod drawer;
#[path = "icons.rs"]
mod icons;
#[path = "lighting_ui.rs"]
mod lighting_ui;
#[path = "numeric_input.rs"]
mod numeric_input;
#[path = "orbital.rs"]
mod orbital;
#[path = "scene_tabs.rs"]
mod scene_tabs;
#[path = "tool_icons.rs"]
mod tool_icons;
pub(super) use button_fx::ButtonFxPlugin;
pub(super) use camera_preview::PreviewCamera;
pub(super) use camera_preview::{rect as camera_preview_rect, update as update_camera_preview};
pub(super) use confirmation::update as update_confirmation;
pub(super) use control_hints::update as update_control_hints;
pub(super) use drawer::{
    AssetDrag, DrawerMotion, DrawerSize, actions as browser_actions, animate as animate_drawer,
    drag_assets, drag_visuals, refresh as refresh_browser, resize_drawer, search as search_browser,
    update_files as update_browser_files,
};
pub(super) use icons::SvgIconsPlugin;
pub(super) use lighting_ui::{
    LightingUi, edit_controls as edit_lighting_controls, edit_name as edit_light_name,
    refresh as refresh_lighting, refresh_controls as refresh_lighting_controls,
};
pub(super) use numeric_input::{NumericInput, update as update_numeric_input};

pub(super) const TOP: f32 = 94.0;
pub(super) const SIDE: f32 = 316.0;
pub(super) const DRAWER: f32 = 350.0;
pub(super) const FOOT: f32 = 34.0;

const CHROME: Color = Color::srgb(0.077, 0.081, 0.100);
const PANEL: Color = Color::srgb(0.047, 0.055, 0.068);
const WELL: Color = Color::srgb(0.032, 0.038, 0.049);
const EDGE: Color = Color::srgb(0.195, 0.194, 0.221);
const HOVER: Color = Color::srgb(0.160, 0.153, 0.187);
const ACTIVE: Color = Color::srgb(0.225, 0.183, 0.135);
const ACCENT: Color = Color::srgb(0.755, 0.584, 0.359);
const TEXT: Color = Color::srgb(0.944, 0.932, 0.902);
const MUTED: Color = Color::srgb(0.652, 0.659, 0.711);
const FOCUS: Color = Color::srgb(0.909, 0.766, 0.536);
const ON_ACCENT: Color = Color::srgb(0.075, 0.060, 0.048);
const BLUE: Color = Color::srgb(0.510, 0.729, 1.0);
const GREEN: Color = Color::srgb(0.476, 0.803, 0.674);
const AMBER: Color = Color::srgb(1.0, 0.775, 0.433);
const ERROR: Color = Color::srgb(1.0, 0.40, 0.40);
const DISABLED_TEXT: Color = Color::srgb(0.48, 0.50, 0.55);
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
    Tool,
    Tab,
    TabClose,
}

#[derive(Component)]
pub(super) struct Control {
    action: Action,
    look: Look,
}

#[derive(Component)]
pub(super) struct Scroller;

#[derive(Component)]
pub(super) struct HorizontalScroller;

#[derive(Resource)]
pub(super) struct Interface {
    font: Handle<Font>,
    tabs: Entity,
    scene_image: Entity,
    scene_badge: Entity,
    empty_scene: Entity,
    outliner: Entity,
    inspector: Entity,
    status: Entity,
    dirty: Entity,
    selected: Entity,
    kind: Entity,
    perspective: Entity,
    lens_label: Entity,
    transforms: Entity,
    scale_row: Entity,
    lens_row: Entity,
    field_hint: Entity,
    navigation_hint: Entity,
}

#[derive(Default)]
pub(super) struct OutlineState {
    room: Option<Room>,
    imported: usize,
    models: Vec<u32>,
    lights: Vec<(u32, String, bool, crate::lighting::LightKind)>,
}

pub(super) fn viewport_rect(window: &Window) -> Rect {
    Rect::from_corners(
        Vec2::new(0.0, TOP),
        Vec2::new(window.width() - SIDE, window.height() - FOOT),
    )
}

pub(super) fn scene_input_rect(window: &Window, drawer: &DrawerMotion) -> Rect {
    if drawer.resizing {
        return Rect::from_corners(Vec2::splat(-1.0), Vec2::splat(-1.0));
    }
    let mut rect = viewport_rect(window);
    rect.max.y -= drawer.visible_height;
    rect
}

fn node(commands: &mut Commands, parent: Entity, node: Node) -> Entity {
    commands.spawn((node, ChildOf(parent))).id()
}

fn surface(commands: &mut Commands, parent: Entity, node: Node, color: Color) -> Entity {
    let entity = commands
        .spawn((
            node,
            BackgroundColor(color),
            BorderColor::all(EDGE),
            ChildOf(parent),
        ))
        .id();
    // Only chrome surfaces receive a gradient; icon and preview colors stay literal.
    let colors = if color == CHROME {
        Some([
            Color::srgb(0.125, 0.111, 0.110),
            CHROME,
            Color::srgb(0.090, 0.086, 0.128),
        ])
    } else if color == PANEL {
        Some([
            Color::srgb(0.081, 0.078, 0.105),
            PANEL,
            Color::srgb(0.036, 0.045, 0.054),
        ])
    } else if color == WELL {
        Some([Color::srgb(0.060, 0.057, 0.073), WELL, WELL])
    } else {
        None
    };
    if let Some(colors) = colors {
        commands
            .entity(entity)
            .insert(gradient(LinearGradient::TO_BOTTOM_RIGHT, colors));
    }
    entity
}

fn gradient(angle: f32, colors: [Color; 3]) -> BackgroundGradient {
    LinearGradient::new(angle, colors.into_iter().map(ColorStop::auto).collect()).into()
}

fn horizon(commands: &mut Commands, parent: Entity) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            bottom: px(0),
            height: px(1),
            ..default()
        },
        gradient(
            LinearGradient::TO_RIGHT,
            [
                Color::srgba(0.91, 0.73, 0.47, 0.65),
                Color::srgba(0.63, 0.54, 0.77, 0.23),
                Color::srgba(0.63, 0.54, 0.77, 0.0),
            ],
        ),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
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
            TabIndex(0),
            Hovered::default(),
            Control { action, look },
            node,
            BackgroundColor(Color::NONE),
            BackgroundGradient::default(),
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
            border: UiRect::all(px(1)),
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
    mut images: ResMut<Assets<Image>>,
    scene_surface: Res<viewport::Surface>,
    outline: Res<super::selection_outline::Outline>,
    stars: Res<starfield::Starfield>,
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
                overflow: Overflow::clip(),
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
            padding: UiRect::axes(px(12), px(5)),
            border: UiRect::bottom(px(1)),
            ..column(3.0)
        },
        CHROME,
    );
    horizon(&mut commands, toolbar);
    starfield::background(&mut commands, toolbar, &stars.chrome);
    let project_bar = node(
        &mut commands,
        toolbar,
        Node {
            height: px(41),
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
    let brand = node(
        &mut commands,
        project_bar,
        Node {
            width: px(190),
            min_width: px(0),
            flex_shrink: 0.0,
            overflow: Overflow::clip(),
            ..row(8.0)
        },
    );
    orbital::brand(&mut commands, brand);
    let name = node(
        &mut commands,
        brand,
        Node {
            min_width: px(0),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
    );
    text(
        &mut commands,
        name,
        &font,
        &editor.project.title,
        16.0,
        TEXT,
    );
    text(&mut commands, name, &font, "РЕДАКТОР ЛОКАЦИЙ", 9.0, MUTED);

    let tabs = scene_tabs::setup(&mut commands, project_bar);
    button(
        &mut commands,
        project_bar,
        &font,
        "Играть",
        Action::Play,
        Look::Regular,
    );
    let tool_bar = node(&mut commands, toolbar, row(12.0));
    tool_icons::setup(&mut commands, tool_bar);
    let tools = group(&mut commands, tool_bar);
    icons::button(
        &mut commands,
        tools,
        icons::Glyph::Magnet,
        "Привязка",
        Action::Snap,
        Look::Regular,
    );
    let navigation_hint = text(
        &mut commands,
        tool_bar,
        &font,
        "Выберите объект в сцене или дереве",
        12.0,
        MUTED,
    );
    commands.entity(navigation_hint).insert(Node {
        min_width: px(0),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        overflow: Overflow::clip(),
        ..default()
    });

    let body = node(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            top: px(TOP),
            bottom: px(FOOT),
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
    let scene_image = commands
        .spawn((
            ImageNode::new(scene_surface.0.clone()),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(viewport),
        ))
        .id();
    commands.spawn((
        MaterialNode(outline.material.clone()),
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
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
        Color::srgba(0.043, 0.049, 0.066, 0.94),
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

    camera_preview::setup(&mut commands, &mut images, viewport, &font);
    let empty_scene = scene_tabs::empty_workspace(&mut commands, viewport, &font, &stars);

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
    starfield::background(&mut commands, side, &stars.chrome);
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
    let tree_title = text(&mut commands, header, &font, "Дерево сцены", 14.0, TEXT);
    commands.entity(tree_title).insert(Node {
        flex_grow: 1.0,
        min_width: px(0),
        overflow: Overflow::clip(),
        ..default()
    });
    horizon(&mut commands, header);
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
            height: px(300),
            flex_shrink: 0.0,
            padding: UiRect::all(px(12)),
            border: UiRect::top(px(1)),
            overflow: Overflow::scroll_y(),
            ..column(6.0)
        },
        CHROME,
    );
    commands.entity(inspector).insert(Scroller);
    starfield::background(&mut commands, inspector, &stars.chrome);
    horizon(&mut commands, inspector);
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
    lighting_ui::setup(&mut commands, header, inspector, root, &font, &mut images);
    let transforms = node(&mut commands, inspector, column(6.0));
    transform_row(&mut commands, transforms, &font, "Позиция", Field::Position);
    transform_row(&mut commands, transforms, &font, "Поворот", Field::Rotation);
    let scale_row = transform_row(&mut commands, transforms, &font, "Масштаб", Field::Scale);
    let lens_row = node(&mut commands, transforms, row(4.0));
    let lens_label = field_caption(&mut commands, lens_row, &font, "Обзор, °");
    number(&mut commands, lens_row, &font, Field::Lens, "·", BLUE);
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
        ("Управление камерой", Action::Pilot),
        ("Из вида", Action::Align),
        ("Дублировать", Action::Duplicate),
        ("Удалить", Action::Delete),
    ] {
        button(&mut commands, actions, &font, label, action, Look::Regular);
    }
    let field_hint = text(
        &mut commands,
        transforms,
        &font,
        "Enter / Tab — применить · Esc — отменить",
        11.0,
        MUTED,
    );
    commands.entity(field_hint).insert(TextLayout::default());

    drawer::setup(&mut commands, root, &font, &stars);

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
    commands.entity(footer).insert(ZIndex(20));
    drawer::footer_toggle(&mut commands, footer, &font);
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
    let dirty = text(&mut commands, footer, &font, "●", 11.0, GREEN);
    control_hints::setup(&mut commands, root, &font);
    confirmation::setup(&mut commands, root, &font);
    commands.insert_resource(Interface {
        font,
        tabs,
        scene_image,
        scene_badge: badge,
        empty_scene,
        outliner,
        inspector,
        status,
        dirty,
        selected,
        kind,
        perspective,
        lens_label,
        transforms,
        scale_row,
        lens_row,
        field_hint,
        navigation_hint,
    });
}

fn outline_entry(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    selection: Selection,
    kind: Option<crate::lighting::LightKind>,
    enabled: bool,
) {
    let entry = button(
        commands,
        parent,
        font,
        label,
        Action::Select(selection),
        Look::Outline,
    );
    commands
        .entity(entry)
        .entry::<Node>()
        .and_modify(|mut node| node.padding.left = px(26));
    let color = if enabled { AMBER } else { MUTED };
    let glyph = match selection {
        Selection::Environment => icons::Glyph::Globe,
        Selection::Camera(_) => icons::Glyph::Camera,
        Selection::Imported(_) | Selection::Model(_) => icons::Glyph::Box,
        Selection::Light(_) => match kind {
            Some(crate::lighting::LightKind::Directional) => icons::Glyph::Sun,
            Some(crate::lighting::LightKind::Spot) => icons::Glyph::Spotlight,
            Some(crate::lighting::LightKind::Rect) => icons::Glyph::Rectangle,
            _ => icons::Glyph::Lightbulb,
        },
    };
    let tint = if matches!(selection, Selection::Light(_) | Selection::Environment) {
        color
    } else {
        TEXT
    };
    let icon = icons::spawn(commands, entry, glyph, 14.0, tint);
    commands
        .entity(icon)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.position_type = PositionType::Absolute;
            node.left = px(8);
            node.top = px(6);
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
) -> Entity {
    let line = node(commands, parent, row(4.0));
    field_caption(commands, line, font, label);
    for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        number(commands, line, font, field(index), axis, AXIS[index]);
    }
    line
}

fn number(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    field: Field,
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
            Field::Light(_) => format!("Параметр света {axis}"),
            Field::Environment(_) => format!("Освещение сцены {axis}"),
        }),
        EditableText::new("0.00"),
        TextCursorStyle::default(),
        font(font_handle, 12.0),
        TextColor(TEXT),
        TextLayout::no_wrap(),
        TabIndex(0),
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

fn available(editor: &Editor, browser: &Browser, action: Action) -> bool {
    if editor.pending.is_some() {
        return match action {
            Action::SaveAndContinue => editor.ready,
            Action::DiscardAndContinue | Action::CancelPending => true,
            _ => false,
        };
    }
    match action {
        Action::SaveAndContinue | Action::DiscardAndContinue | Action::CancelPending => false,
        Action::Open(room) => room < editor.project.scenes.len(),
        Action::CloseTab(room) => editor.open_tabs.contains(&room),
        Action::Tool(tool) => {
            editor.has_scene()
                && editor.ready
                && match editor.selection {
                    Some(Selection::Environment) => false,
                    Some(target @ Selection::Light(_)) => (0..3).any(|axis| {
                        editor.field_allowed(
                            target,
                            match tool {
                                Tool::Move => Field::Position(axis),
                                Tool::Rotate => Field::Rotation(axis),
                                Tool::Scale => Field::Scale(axis),
                            },
                        )
                    }),
                    _ => true,
                }
        }
        Action::Snap | Action::Select(_) => editor.has_scene() && editor.ready,
        Action::Lighting(_) | Action::LightMenu | Action::LightAdvanced => {
            editor.has_scene() && editor.ready
        }
        Action::BrowseBack => browser.can_go_back(),
        Action::BrowseForward => browser.can_go_forward(),
        Action::BrowseUp => browser.current != 0,
        Action::Save => editor.has_scene() && editor.ready,
        Action::Play => editor.has_scene() && editor.ready && editor.definition().play.is_some(),
        Action::Undo => {
            editor.has_scene() && (!editor.undo.is_empty() || editor.camera_navigation.is_some())
        }
        Action::Redo => editor.has_scene() && !editor.redo.is_empty(),
        #[cfg(test)]
        Action::Add(_) => editor.has_scene() && editor.ready,
        Action::Pilot => {
            editor.has_scene()
                && editor.ready
                && matches!(editor.selection, Some(Selection::Camera(_)))
        }
        Action::Align => {
            editor.has_scene()
                && matches!(editor.selection, Some(Selection::Camera(_)))
                && !editor.pilot
        }
        Action::Focus => editor.has_scene() && editor.ready,
        Action::Reset => {
            editor.has_scene()
                && matches!(
                    editor.selection,
                    Some(
                        Selection::Camera(_)
                            | Selection::Imported(_)
                            | Selection::Light(_)
                            | Selection::Environment
                    )
                )
        }
        Action::Delete | Action::Duplicate => {
            editor.has_scene()
                && matches!(
                    editor.selection,
                    Some(Selection::Model(_) | Selection::Light(_))
                )
        }
        _ => true,
    }
}

pub(super) fn input(
    editor: Res<Editor>,
    drawer: Res<DrawerMotion>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<InputFocus>,
    lighting: Option<Res<LightingUi>>,
) {
    if editor.pending.is_some()
        || lighting
            .as_ref()
            .is_some_and(|lighting| lighting.blocks_scene_input())
    {
        return;
    }
    if mouse.any_just_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle])
        && window
            .cursor_position()
            .is_some_and(|cursor| scene_input_rect(&window, &drawer).contains(cursor))
    {
        focus.clear();
    }
}

fn activate_control(
    activate: On<Activate>,
    editor: Res<Editor>,
    browser: Res<Browser>,
    drag: Res<AssetDrag>,
    window: Single<&Window, With<PrimaryWindow>>,
    controls: Query<&Control>,
    mut effects: Query<&mut button_fx::Effect>,
    mut focus: ResMut<InputFocus>,
    mut messages: MessageWriter<Action>,
) {
    if !drag.suppress_activation_at(window.cursor_position())
        && let Ok(control) = controls.get(activate.entity)
        && available(&editor, &browser, control.action)
    {
        if let Ok(mut effect) = effects.get_mut(activate.entity) {
            effect.activate();
        }
        if !matches!(
            control.action,
            Action::SaveAndContinue | Action::DiscardAndContinue | Action::CancelPending
        ) {
            focus.clear();
        }
        messages.write(control.action);
    }
}

pub(super) fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<InputFocus>,
    numbers: Query<(), (With<Field>, With<TextInput>)>,
    inputs: Query<(), With<TextInput>>,
    mut actions: MessageWriter<Action>,
    editor: Res<Editor>,
    drag: Option<Res<AssetDrag>>,
    lighting: Option<Res<LightingUi>>,
) {
    if lighting
        .as_ref()
        .is_some_and(|lighting| lighting.is_menu_open())
        || (keys.just_pressed(KeyCode::Escape)
            && lighting
                .as_ref()
                .is_some_and(|lighting| lighting.is_control_active()))
    {
        return;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if editor.pending.is_some() {
        if keys.just_pressed(KeyCode::Escape) {
            actions.write(Action::CancelPending);
        } else if command && keys.just_pressed(KeyCode::KeyS) {
            actions.write(Action::SaveAndContinue);
        }
        return;
    }
    if keys.just_pressed(KeyCode::Escape)
        && drag.as_ref().is_some_and(|drag| drag.cancelled_this_frame)
    {
        return;
    }
    if command && keys.just_pressed(KeyCode::Space) {
        focus.clear();
        actions.write(Action::Drawer);
        return;
    }
    if command && keys.just_pressed(KeyCode::KeyO) {
        focus.clear();
        actions.write(Action::OpenBrowser);
        return;
    }
    if command && keys.just_pressed(KeyCode::KeyW) && editor.has_scene() {
        focus.clear();
        actions.write(Action::CloseTab(editor.room));
        return;
    }
    if command && keys.just_pressed(KeyCode::KeyS) {
        focus.clear();
        actions.write(Action::Save);
        return;
    }
    if focus.get().is_some_and(|entity| numbers.contains(entity)) {
        return;
    }
    // Typing a model name must not trigger scene tools or Delete.
    if focus.get().is_some_and(|entity| inputs.contains(entity)) {
        if keys.just_pressed(KeyCode::Escape) {
            focus.clear();
            if editor.drawer {
                actions.write(Action::Drawer);
            }
        }
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if keys.just_pressed(KeyCode::Escape) {
        if editor.drawer {
            actions.write(Action::Drawer);
        } else if editor.pilot {
            actions.write(Action::Pilot);
        }
        return;
    }
    if command {
        for (key, action) in [
            (
                KeyCode::KeyZ,
                if shift { Action::Redo } else { Action::Undo },
            ),
            (KeyCode::KeyY, Action::Redo),
            (KeyCode::KeyD, Action::Duplicate),
        ] {
            if keys.just_pressed(key) {
                actions.write(action);
            }
        }
    } else if !editor.pilot && !mouse.pressed(MouseButton::Right) {
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
    }
}

pub(super) fn scroll(
    editor: Res<Editor>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<DrawerMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    lighting: Option<Res<LightingUi>>,
    mut areas: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &mut ScrollPosition,
            Has<drawer::DrawerScroll>,
            Has<HorizontalScroller>,
        ),
        With<Scroller>,
    >,
) {
    if lighting
        .as_ref()
        .is_some_and(|lighting| lighting.is_menu_open())
    {
        return;
    }
    if editor.pending.is_some() || scroll.delta == Vec2::ZERO {
        return;
    }
    let Some(cursor) = window
        .cursor_position()
        .map(|pos| pos * window.scale_factor())
    else {
        return;
    };
    if cursor.y / window.scale_factor() >= window.height() - FOOT {
        return;
    }
    let over_drawer = drawer.visible_height > 0.0
        && cursor.y / window.scale_factor() >= window.height() - FOOT - drawer.visible_height;
    for (computed, transform, mut offset, drawer_scroll, horizontal) in &mut areas {
        if over_drawer != drawer_scroll {
            continue;
        }
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
        if horizontal {
            let delta = if scroll.delta.x != 0.0 {
                scroll.delta.x
            } else {
                scroll.delta.y
            };
            let max = ((computed.content_size().x - computed.size().x)
                * computed.inverse_scale_factor())
            .max(0.0);
            offset.x = (offset.x - delta * amount).clamp(0.0, max);
        } else {
            let max = ((computed.content_size().y - computed.size().y)
                * computed.inverse_scale_factor())
            .max(0.0);
            offset.y = (offset.y - scroll.delta.y * amount).clamp(0.0, max);
        }
    }
}

pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    interface: Res<Interface>,
    browser: Res<Browser>,
    (focus, focus_visible): (Res<InputFocus>, Res<bevy::input_focus::InputFocusVisible>),
    browser_ui: Res<drawer::BrowserUi>,
    numeric: Res<NumericInput>,
    lighting: Res<LightingUi>,
    (mut outline, mut tabs, mut inspector_selection): (
        Local<OutlineState>,
        Local<scene_tabs::Tabs>,
        Local<Option<(Option<Room>, Option<Selection>)>>,
    ),
    mut scroll_positions: Query<&mut ScrollPosition>,
    mut labels: Query<(&mut Text, &mut TextColor)>,
    mut panes: Query<&mut Node, Without<Control>>,
    mut buttons: Query<(
        Entity,
        &Control,
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        &ComputedNode,
        &Children,
        Option<&TabIndex>,
        &mut BackgroundColor,
        &mut BackgroundGradient,
        &mut BorderColor,
        &mut Node,
    )>,
    mut fields: Query<
        (
            Entity,
            &Field,
            &mut EditableText,
            &mut BorderColor,
            &mut TextReadWriteMode,
            &ComputedNode,
            &mut TabIndex,
        ),
        Without<Control>,
    >,
    mut other_inputs: Query<
        (&ComputedNode, &mut TabIndex, &mut TextReadWriteMode),
        (With<TextInput>, Without<Field>, Without<Control>),
    >,
) {
    scene_tabs::refresh(
        &mut commands,
        &editor,
        &numeric,
        &interface,
        &mut tabs,
        &mut labels,
        focus.get().filter(|_| focus_visible.0),
        |entity| {
            buttons
                .get(entity)
                .is_ok_and(|(_, _, hovered, pressed, ..)| hovered.0 || pressed)
        },
    );
    let room = editor.has_scene().then_some(editor.room);
    let selected_document = (room, editor.selection);
    if *inspector_selection != Some(selected_document) {
        if let Ok(mut scroll) = scroll_positions.get_mut(interface.inspector) {
            scroll.y = 0.0;
        }
        *inspector_selection = Some(selected_document);
    }
    let models: Vec<_> = editor.edits.models.iter().map(|model| model.id).collect();
    let lights: Vec<_> = editor
        .edits
        .lights
        .iter()
        .map(|light| (light.id, light.name.clone(), light.enabled, light.kind))
        .collect();
    if outline.room != room
        || outline.imported != editor.imported.len()
        || outline.models != models
        || outline.lights != lights
    {
        *outline = OutlineState {
            room,
            imported: editor.imported.len(),
            models,
            lights,
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
        if editor.has_scene() {
            heading(&mut commands, "ОСВЕЩЕНИЕ");
            outline_entry(
                &mut commands,
                interface.outliner,
                &interface.font,
                "Освещение сцены",
                Selection::Environment,
                None,
                true,
            );
            for light in &editor.edits.lights {
                let target = Selection::Light(light.id);
                outline_entry(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    &light.name,
                    target,
                    Some(light.kind),
                    light.enabled,
                );
            }
            heading(&mut commands, "КАМЕРЫ");
            for shot in viewport::shots(&editor) {
                let target = Selection::Camera(shot);
                outline_entry(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    &editor.label(target),
                    target,
                    None,
                    true,
                );
            }
            heading(&mut commands, "ОБЪЕКТЫ КОМНАТЫ");
            for index in 0..editor.imported.len() {
                let target = Selection::Imported(index);
                outline_entry(
                    &mut commands,
                    interface.outliner,
                    &interface.font,
                    &editor.label(target),
                    target,
                    None,
                    true,
                );
            }
            if !editor.edits.models.is_empty() {
                heading(&mut commands, "ЭКЗЕМПЛЯРЫ КАТАЛОГА");
                for model in &editor.edits.models {
                    let target = Selection::Model(model.id);
                    outline_entry(
                        &mut commands,
                        interface.outliner,
                        &interface.font,
                        &editor.label(target),
                        target,
                        None,
                        true,
                    );
                }
            }
        } else {
            heading(&mut commands, "НЕТ ОТКРЫТОЙ СЦЕНЫ");
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
        interface.navigation_hint,
        if !editor.has_scene() {
            "Откройте сцену в файловом менеджере"
        } else if editor.pilot {
            "ПКМ + WASD / Q E · Shift — быстрее · Esc — выйти"
        } else if editor.selection.is_some() {
            "F — фокус · Alt + ЛКМ — орбита · СКМ — панорама"
        } else {
            "Выберите объект в сцене или дереве · ПКМ + WASD — полёт"
        },
        MUTED,
    );
    set_label(
        interface.field_hint,
        numeric
            .error_message()
            .unwrap_or("Enter / Tab — применить · Esc — отменить"),
        if numeric.error_message().is_some() {
            ERROR
        } else {
            MUTED
        },
    );
    set_label(
        interface.dirty,
        if !editor.has_scene() {
            ""
        } else if editor.dirty()
            || numeric.has_changes(&editor)
            || lighting.name_has_changes(&editor)
        {
            "● Изменено"
        } else {
            "● Сохранено"
        },
        if editor.dirty() || numeric.has_changes(&editor) || lighting.name_has_changes(&editor) {
            AMBER
        } else {
            GREEN
        },
    );
    let camera = editor.has_scene() && matches!(editor.selection, Some(Selection::Camera(_)));
    let filling = camera
        && matches!(editor.selection, Some(Selection::Camera(shot)) if editor.definition().cameras[shot].projection == LensKind::Orthographic);
    let perspective = if editor.pilot {
        editor.selection.map_or(String::new(), |target| {
            format!("Камера · {} · 16:9", editor.label(target))
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
    if !editor.has_scene() {
        set_label(interface.selected, "Сцена не открыта", MUTED);
        set_label(interface.kind, "Откройте файл сцены внизу", MUTED);
    } else if let Some(target) = editor.selection {
        set_label(interface.selected, &editor.label(target), TEXT);
        let kind = match target {
            Selection::Camera(_) => "Игровой ракурс",
            Selection::Imported(_) if !editor.field_allowed(target, Field::Position(1)) => {
                "Ограничения трансформации проекта"
            }
            Selection::Imported(_) => "Объект исходной сцены",
            Selection::Model(_) => "Экземпляр общей модели",
            Selection::Light(_) => "Источник света · настройки ниже",
            Selection::Environment => "Окружение, экспозиция и свет сцены",
        };
        set_label(interface.kind, kind, MUTED);
    } else {
        set_label(interface.selected, "Выберите объект или камеру", MUTED);
        set_label(interface.kind, "В 3D-виде или в дереве сцены", MUTED);
    }
    for (entity, shown) in [
        (interface.scene_image, editor.has_scene()),
        (interface.scene_badge, editor.has_scene()),
        (interface.empty_scene, !editor.has_scene()),
        (
            interface.transforms,
            editor.has_scene()
                && editor
                    .selection
                    .is_some_and(|target| target != Selection::Environment),
        ),
        (interface.lens_row, camera),
        (
            interface.scale_row,
            !camera && !matches!(editor.selection, Some(Selection::Light(_))),
        ),
    ] {
        if let Ok(mut pane) = panes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if pane.display != display {
                pane.display = display;
            }
        }
    }
    if let Ok(mut pane) = panes.get_mut(interface.inspector) {
        let light = matches!(
            editor.selection,
            Some(Selection::Light(_) | Selection::Environment)
        );
        pane.height = if light { percent(65) } else { px(300) };
        pane.min_height = if light { px(280) } else { px(0) };
        pane.max_height = if light { px(520) } else { Val::Auto };
    }
    for (
        entity,
        control,
        hovered,
        pressed,
        disabled,
        computed,
        children,
        tab_index,
        mut tint,
        mut background,
        mut border,
        mut node,
    ) in &mut buttons
    {
        let active = match control.action {
            Action::Open(room) | Action::CloseTab(room) => {
                editor.has_scene() && editor.room == room
            }
            Action::Select(target) => editor.selection == Some(target),
            Action::Tool(tool) => editor.tool == tool,
            Action::Snap => editor.snap,
            Action::Drawer => editor.drawer,
            Action::Browse(index) => browser.current == index,
            Action::BrowseFile(index) => browser_ui.selected == Some(index),
            Action::BrowseModel(index) => browser_ui.selected_model == Some(index),
            Action::Pilot => editor.pilot,
            _ => false,
        };
        let visible = match control.action {
            Action::BrowseClear => !browser.search.is_empty(),
            Action::Pilot | Action::Align => camera,
            Action::Play => editor.has_scene() && editor.definition().play.is_some(),
            Action::Duplicate | Action::Delete => {
                matches!(
                    editor.selection,
                    Some(Selection::Model(_) | Selection::Light(_))
                )
            }
            Action::Reset => {
                editor.has_scene() && !matches!(editor.selection, Some(Selection::Model(_)))
            }
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
        let enabled = available(&editor, &browser, control.action);
        if enabled == disabled {
            if enabled {
                commands.entity(entity).try_remove::<InteractionDisabled>();
            } else {
                commands
                    .entity(entity)
                    .try_insert(InteractionDisabled)
                    .try_remove::<Pressed>();
            }
        }
        let tab = if enabled && visible && computed.size().min_element() > 0.0 {
            0
        } else {
            -1
        };
        if tab_index.is_none_or(|index| index.0 != tab) {
            // Tabs and the outliner can queue a rebuild earlier in this refresh.
            commands.entity(entity).try_insert(TabIndex(tab));
        }
        border.set_if_neq(BorderColor::all(
            if enabled
                && focus.get() == Some(entity)
                && !matches!(control.look, Look::Tab | Look::TabClose)
            {
                FOCUS
            } else {
                Color::NONE
            },
        ));
        for child in children.iter() {
            if let Ok((_, mut color)) = labels.get_mut(child) {
                color.set_if_neq(TextColor(if !enabled {
                    DISABLED_TEXT
                } else if matches!(control.look, Look::Primary) {
                    ON_ACCENT
                } else if matches!(control.look, Look::Tab) {
                    if active || hovered.0 {
                        scene_tabs::LABEL
                    } else {
                        scene_tabs::INACTIVE_LABEL
                    }
                } else {
                    TEXT
                }));
            }
        }
        let color = if !enabled {
            Color::NONE
        } else if matches!(control.look, Look::Tab | Look::TabClose) {
            if hovered.0 || pressed {
                if matches!(control.look, Look::TabClose) {
                    scene_tabs::CLOSE_HOVER
                } else {
                    scene_tabs::HOVER
                }
            } else {
                Color::NONE
            }
        } else if matches!(control.look, Look::Tool) {
            if active {
                ACCENT
            } else if hovered.0 || pressed {
                tool_icons::HOVER
            } else {
                tool_icons::IDLE
            }
        } else if matches!(control.look, Look::Primary) {
            if hovered.0 { FOCUS } else { ACCENT }
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
        let fill = if enabled
            && (matches!(control.look, Look::Primary)
                || (active && matches!(control.look, Look::Tool)))
        {
            gradient(
                LinearGradient::TO_BOTTOM_RIGHT,
                [
                    if hovered.0 || pressed {
                        Color::srgb(0.985, 0.873, 0.660)
                    } else {
                        FOCUS
                    },
                    ACCENT,
                    Color::srgb(0.640, 0.480, 0.300),
                ],
            )
        } else if enabled
            && active
            && !hovered.0
            && !pressed
            && !matches!(control.look, Look::Tab | Look::TabClose)
        {
            gradient(
                LinearGradient::TO_BOTTOM_RIGHT,
                [ACTIVE, Color::srgb(0.125, 0.116, 0.137), PANEL],
            )
        } else {
            BackgroundGradient::default()
        };
        background.set_if_neq(fill);
    }
    for (entity, &field, mut text, mut border, mut mode, computed, mut tab_index) in &mut fields {
        let allowed = editor.ready
            && editor.pending.is_none()
            && editor
                .selection
                .is_some_and(|target| editor.field_allowed(target, field));
        let tab = if allowed && computed.size().min_element() > 0.0 {
            0
        } else {
            -1
        };
        if tab_index.0 != tab {
            tab_index.0 = tab;
        }
        let focused = focus.get() == Some(entity) && allowed;
        border.set_if_neq(BorderColor::all(if numeric.invalid(entity) {
            ERROR
        } else if focused {
            FOCUS
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
        if !focused && !numeric.preserve_text(entity) {
            let value = editor
                .selection
                .and_then(|target| editor.field_value(target, field));
            let display = value.map_or_else(|| "—".into(), |value| format!("{value:.2}"));
            if text.value().to_string() != display {
                text.editor_mut().set_text(&display);
            }
        }
    }
    for (computed, mut tab_index, mut mode) in &mut other_inputs {
        let allowed = editor.pending.is_none() && computed.size().min_element() > 0.0;
        let tab = if allowed { 0 } else { -1 };
        if tab_index.0 != tab {
            tab_index.0 = tab;
        }
        mode.set_if_neq(if allowed {
            TextReadWriteMode::Editable
        } else {
            TextReadWriteMode::ReadOnly
        });
    }
}
