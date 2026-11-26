//! Bevy-native controls using the editor's existing colors, buttons and icons.
use super::*;
use crate::editor::image_brush_rail_fx as rail_fx;
#[path = "image_brush_library_ui.rs"]
mod library_ui;
use bevy::ui_widgets::{
    Slider, SliderDragState, SliderOrientation, SliderRange, SliderStep, SliderThumb, SliderValue,
    TrackClick, ValueChange,
};
use library_ui::BrushLibraryUi;

#[derive(Clone, Copy, Component)]
pub(super) enum Binding {
    Size,
    Hardness,
    BrushOpacity,
    LayerOpacity(photocraft_doc::LayerId),
}
#[derive(Component)]
pub(in crate::editor) struct PhotoSlider {
    binding: Binding,
    thumb: Entity,
    readout: Entity,
}

#[cfg(test)]
pub(in crate::editor) fn model_handle_center(world: &World) -> Vec2 {
    model_handle_geometry(world).0
}

#[cfg(test)]
pub(in crate::editor) fn model_handle_geometry(world: &World) -> (Vec2, [f32; 4]) {
    world.resource::<EditUi>().model_overlay.handle_geometry()
}

#[derive(Resource)]
pub(in crate::editor) struct EditUi {
    mode_button: Entity,
    mode: Entity,
    menu: Entity,
    tools: Entity,
    raster_tools: Entity,
    color_tools: Entity,
    paint_button: Entity,
    eraser_button: Entity,
    brush_icon: Entity,
    pencil_icon: Entity,
    paint_menu: Entity,
    panel: Entity,
    eraser_modes: Entity,
    error: Entity,
    size_group: Entity,
    model_tools: Entity,
    model_pose: Entity,
    model_pose_reset: Entity,
    model_overlay: super::super::model_transform::Overlay,
    brush_rail: Entity,
    size_button: Entity,
    size_value: Entity,
    size_menu: Entity,
    size_settings: Entity,
    library: BrushLibraryUi,
    size_title: Entity,
    hardness_group: Entity,
    pencil_shapes: Entity,
    shape_items: [(Entity, &'static str); 2],
    size_shown: bool,
    size_input: Entity,
    size_original: Option<f32>,
    size_last_text: Option<String>,
    size_invalid: bool,
    size_confirm: bool,
    cursor: super::super::tool_cursor::Overlay,
}

pub(super) fn slider(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    title: &str,
    binding: Binding,
    range: (f32, f32),
) {
    let header = node(
        commands,
        parent,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(8.0)
        },
    );
    text(commands, header, font_handle, title, 11.0, MUTED);
    let readout = text(commands, header, font_handle, "", 11.0, TEXT);
    slider_track(commands, parent, binding, range, readout, title);
}

fn slider_track(
    commands: &mut Commands,
    parent: Entity,
    binding: Binding,
    range: (f32, f32),
    readout: Entity,
    title: &str,
) -> Entity {
    let slider = commands
        .spawn((
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderRange::new(range.0, range.1),
            SliderValue(range.0),
            TabIndex(0),
            Hovered::default(),
            AccessibleLabel(title.into()),
            Node {
                width: percent(100),
                height: px(20),
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(parent),
        ))
        .observe(slider_change)
        .id();
    let track = surface(
        commands,
        slider,
        Node {
            position_type: PositionType::Absolute,
            left: px(6),
            right: px(6),
            top: px(8),
            height: px(4),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    if matches!(binding, Binding::LayerOpacity(_)) {
        layers::ux::help(
            commands,
            slider,
            "Непрозрачность всего слоя вместе с эффектами. 100% — полностью виден, 0% — невидим. Двигайте ползунок, чтобы просвечивали нижние слои.",
        );
    }
    commands.entity(track).insert(Pickable::IGNORE);
    if matches!(
        binding,
        Binding::Size | Binding::Hardness | Binding::BrushOpacity | Binding::LayerOpacity(_)
    ) {
        commands
            .entity(track)
            .insert((BackgroundColor(EDGE), BackgroundGradient::default()));
    }
    let thumb = surface(
        commands,
        slider,
        Node {
            position_type: PositionType::Absolute,
            width: px(12),
            height: px(12),
            top: px(4),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        PANEL,
    );
    commands
        .entity(thumb)
        .insert((SliderThumb, Pickable::IGNORE, BorderColor::all(FOCUS)));
    commands.entity(slider).insert(PhotoSlider {
        binding,
        thumb,
        readout,
    });
    slider
}

fn size_label(size: f32) -> String {
    format!("{size:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

const RAIL_THUMB_HEIGHT: f32 = 28.0;

#[derive(Component)]
pub(in crate::editor) struct RailControl {
    fill: Entity,
    badge: Entity,
    handle: Entity,
    emphasis: f32,
}

// Eight octaves give small brushes useful travel without limiting large tips.
fn rail_size(position: f32) -> f32 {
    2.0_f32.powf(position.clamp(0.0, 1.0) * 8.0)
}
fn rail_position(size: f32) -> f32 {
    size.clamp(1.0, 256.0).log2() / 8.0
}

/// Floating controls belong to the canvas so they stay clear of the layers panel.
fn brush_rail(commands: &mut Commands, canvas: Entity, font_handle: &Handle<Font>) -> Entity {
    let rail = surface(
        commands,
        canvas,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(8),
            top: percent(50),
            width: px(46.8),
            height: percent(68),
            max_height: px(391),
            padding: UiRect::axes(px(4), px(7)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            ..column(14.0)
        },
        Color::NONE,
    );
    commands.entity(rail).insert((
        rail_fx::Reveal::default(),
        rail_fx::RevealTarget(false),
        CanvasControl,
        ZIndex(8),
        BorderColor::all(Color::NONE),
        BoxShadow(vec![ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.45),
            x_offset: px(3),
            y_offset: px(4),
            blur_radius: px(8),
            spread_radius: px(1),
        }]),
        UiTransform::from_translation(Val2::percent(0, -50)),
        AccessibleLabel("Быстрые настройки кисти".into()),
    ));
    rail_fx::attach(commands, rail, rail_fx::Kind::Panel);
    for (label, binding, range) in [
        ("Размер кисти · слева", Binding::Size, (0.0, 1.0)),
        (
            "Непрозрачность кисти · слева",
            Binding::BrushOpacity,
            (0.01, 1.0),
        ),
    ] {
        let group = node(
            commands,
            rail,
            Node {
                width: percent(100),
                flex_grow: 1.0,
                flex_basis: px(0),
                min_height: px(0),
                align_items: AlignItems::Center,
                ..column(6.0)
            },
        );
        let slider = commands
            .spawn((
                Slider {
                    orientation: SliderOrientation::Vertical,
                    track_click: TrackClick::Drag,
                },
                SliderRange::new(range.0, range.1),
                SliderValue(range.0),
                SliderStep(0.01),
                TabIndex(0),
                Hovered::default(),
                AccessibleLabel(label.into()),
                Node {
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(RAIL_THUMB_HEIGHT),
                    ..default()
                },
                ChildOf(group),
            ))
            .observe(slider_change)
            .id();
        let track = surface(
            commands,
            slider,
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: px(RAIL_THUMB_HEIGHT * 0.5),
                bottom: px(RAIL_THUMB_HEIGHT * 0.5),
                width: px(15),
                border_radius: BorderRadius::MAX,
                border: UiRect::all(px(1)),
                ..default()
            },
            WELL,
        );
        commands.entity(track).insert((
            rail_fx::Track,
            Pickable::IGNORE,
            BorderColor::all(EDGE),
            UiTransform::from_translation(Val2::percent(-50, 0)),
        ));
        commands.entity(track).remove::<BackgroundGradient>();
        let fill = surface(
            commands,
            slider,
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                bottom: px(RAIL_THUMB_HEIGHT * 0.5),
                width: px(9),
                height: px(0),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            Color::NONE,
        );
        commands.entity(fill).insert((
            Pickable::IGNORE,
            UiTransform::from_translation(Val2::percent(-50, 0)),
        ));
        rail_fx::attach(commands, fill, rail_fx::Kind::Fill);
        let thumb = surface(
            commands,
            slider,
            Node {
                position_type: PositionType::Absolute,
                left: px(1),
                right: px(1),
                top: px(0),
                height: px(RAIL_THUMB_HEIGHT),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..column(3.0)
            },
            Color::NONE,
        );
        commands.entity(thumb).insert((
            SliderThumb,
            Pickable::IGNORE,
            BorderColor::all(Color::NONE),
        ));
        let handle = rail_fx::handle(commands, thumb);
        let badge = surface(
            commands,
            slider,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: percent(100),
                margin: UiRect::left(px(18)),
                height: px(30),
                min_width: px(68),
                padding: UiRect::horizontal(px(10)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            Color::srgb(0.13, 0.14, 0.18),
        );
        commands.entity(badge).insert((
            Pickable::IGNORE,
            BorderColor::all(FOCUS),
            AccessibleLabel(format!("Значение: {label}")),
        ));
        let readout = text(commands, badge, font_handle, "", 13.0, TEXT);
        commands.entity(readout).insert(Pickable::IGNORE);
        commands.entity(slider).insert((
            PhotoSlider {
                binding,
                thumb,
                readout,
            },
            RailControl {
                fill,
                badge,
                handle,
                emphasis: 0.0,
            },
        ));
    }
    rail
}

fn slider_change(
    event: On<ValueChange<f32>>,
    sliders: Query<(&PhotoSlider, Option<&RailControl>)>,
    mut actions: MessageWriter<Action>,
) {
    if let Ok((slider, rail)) = sliders.get(event.source) {
        let action = match slider.binding {
            Binding::Size => EditAction::Size(if rail.is_some() {
                rail_size(event.value)
            } else {
                event.value
            }),
            Binding::Hardness => EditAction::Hardness(event.value),
            Binding::BrushOpacity => EditAction::BrushOpacity(event.value),
            Binding::LayerOpacity(layer) => EditAction::Opacity(layer, event.value, event.is_final),
        };
        actions.write(Action::ImageEdit(action));
    }
}

pub(in crate::editor) fn input(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    label: &str,
) -> Entity {
    commands
        .spawn((
            TextInput,
            EditableText::new(""),
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel(label.into()),
            font(font_handle, 12.0),
            TextColor(TEXT),
            TextLayout::no_wrap(),
            TabIndex(0),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                width: percent(100),
                height: px(28),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(7), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

pub(super) fn tool_node() -> Node {
    Node {
        width: px(36),
        height: px(36),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(7)),
        ..row(0.0)
    }
}

pub(super) fn tool_button(
    commands: &mut Commands,
    parent: Entity,
    glyph: icons::Glyph,
    label: &str,
    tool: PhotoTool,
) {
    let button = control(
        commands,
        parent,
        Action::ImageEdit(EditAction::Tool(tool)),
        Look::Tool,
        tool_node(),
    );
    commands
        .entity(button)
        .insert(AccessibleLabel(label.into()));
    icons::themed(commands, button, glyph, 18.0);
}

pub(super) fn tool_menu_hint(commands: &mut Commands, parent: Entity) {
    let hint = icons::spawn(commands, parent, icons::Glyph::SubmenuDown, 12.0, MUTED);
    commands.entity(hint).insert((
        Node {
            position_type: PositionType::Absolute,
            right: px(1),
            bottom: px(1),
            width: px(12),
            height: px(12),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

pub(in crate::editor) fn setup(
    commands: &mut Commands,
    toolbar: Entity,
    body: Entity,
    canvas: Entity,
    root: Entity,
    font_handle: &Handle<Font>,
    checker_image: &Handle<Image>,
    handles: &mut Assets<crate::editor::viewport::HandleMaterial>,
) {
    let brush_rail = brush_rail(commands, canvas, font_handle);
    let model_overlay =
        super::super::model_transform::Overlay::setup(commands, canvas, font_handle, handles);
    super::super::model_fields::setup(commands, canvas, font_handle);
    gradient::setup(commands, toolbar, canvas, font_handle);
    let size_group = node(
        commands,
        toolbar,
        Node {
            display: Display::None,
            height: px(28),
            flex_shrink: 0.0,
            ..default()
        },
    );
    let size_button = control(
        commands,
        size_group,
        Action::ImageEdit(EditAction::SizeMenu),
        Look::Regular,
        Node {
            height: percent(100),
            padding: UiRect::horizontal(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(4.0)
        },
    );
    commands
        .entity(size_button)
        .insert(AccessibleLabel("Библиотека кистей".into()));
    icons::themed(commands, size_button, icons::Glyph::Brush, 14.0);
    let size_value = text(commands, size_button, font_handle, "24 px", 12.0, TEXT);
    commands.entity(size_value).insert(Node {
        flex_shrink: 0.0,
        ..default()
    });
    icons::themed(commands, size_button, icons::Glyph::ChevronDown, 12.0);
    let size_menu = surface(
        commands,
        size_group,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(36),
            left: px(0),
            width: px(264),
            padding: UiRect::all(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..column(10.0)
        },
        PANEL,
    );
    commands.entity(size_menu).insert((
        GlobalZIndex(50),
        TabGroup::modal(),
        AccessibleLabel("Выбор кисти или параметры инструмента".into()),
    ));
    let library = BrushLibraryUi::setup(commands, size_menu, font_handle);
    let size_settings = node(
        commands,
        size_menu,
        Node {
            width: percent(100),
            ..column(10.0)
        },
    );
    let size_header = node(
        commands,
        size_settings,
        Node {
            width: percent(100),
            ..row(6.0)
        },
    );
    let size_title = text(
        commands,
        size_header,
        font_handle,
        "Размер кисти",
        12.0,
        TEXT,
    );
    commands.entity(size_title).insert(Node {
        flex_grow: 1.0,
        ..default()
    });
    let size_input = input(
        commands,
        size_header,
        font_handle,
        "Размер кисти · px · Shift + движение мыши",
    );
    commands
        .entity(size_input)
        .insert((
            bevy::ui_widgets::SelectAllOnFocus,
            crate::editor::ui::Number::decimal(1.0, 256.0, 0.25, 2),
        ))
        .entry::<Node>()
        .and_modify(|mut n| {
            n.width = px(64);
            n.height = px(28);
        });
    text(commands, size_header, font_handle, "px", 11.0, MUTED);
    slider_track(
        commands,
        size_settings,
        Binding::Size,
        (1.0, 256.0),
        size_input,
        "Размер кисти",
    );
    let pencil_shapes = node(
        commands,
        size_settings,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(6.0)
        },
    );
    text(commands, pencil_shapes, font_handle, "Форма", 11.0, MUTED);
    let shape_buttons = node(commands, pencil_shapes, row(4.0));
    let shape_items = [
        ("Круг", PixelShape::Round),
        ("Квадрат", PixelShape::Square),
    ].map(|(title, shape)| {
        let item = button(
            commands,
            shape_buttons,
            font_handle,
            title,
            Action::ImageEdit(EditAction::PixelShape(shape)),
            Look::Regular,
        );
        commands
            .entity(item)
            .insert(AccessibleLabel(format!("Форма карандаша · {title}")));
        layers::ux::help(
            commands,
            item,
            match shape {
                PixelShape::Round => {
                    "Круглый отпечаток из целых пикселей. Предпросмотр показывает область действия инструмента."
                }
                PixelShape::Square => {
                    "Квадратный отпечаток без сглаживания. Действует на все пиксели внутри квадрата выбранного размера."
                }
            },
        );
        (item, title)
    });
    let mut hardness_group = Entity::PLACEHOLDER;
    for (title, binding, range) in [
        ("Жёсткость", Binding::Hardness, (0.0, 1.0)),
        ("Непрозрачность кисти", Binding::BrushOpacity, (0.01, 1.0)),
    ] {
        let setting = node(
            commands,
            size_settings,
            Node {
                width: percent(100),
                ..column(4.0)
            },
        );
        if matches!(binding, Binding::Hardness) {
            hardness_group = setting;
        }
        slider(commands, setting, font_handle, title, binding, range);
    }
    let selector = surface(
        commands,
        toolbar,
        Node {
            height: px(32),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        PANEL,
    );
    commands.entity(toolbar).insert_children(0, &[selector]);
    commands.entity(selector).insert(ZIndex(30));
    let mode_button = control(
        commands,
        selector,
        Action::ImageEdit(EditAction::Menu),
        Look::Regular,
        Node {
            height: px(32),
            padding: UiRect::horizontal(px(9)),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            overflow: Overflow::clip(),
            ..row(4.0)
        },
    );
    commands
        .entity(mode_button)
        .insert((CanvasControl, AccessibleLabel("Режим изображения".into())));
    let mode = text(commands, mode_button, font_handle, "Просмотр", 13.0, TEXT);
    icons::themed(commands, mode_button, icons::Glyph::ChevronDown, 12.0);
    let menu = surface(
        commands,
        selector,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(40),
            left: px(0),
            width: px(185),
            padding: UiRect::all(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(4.0)
        },
        PANEL,
    );
    commands
        .entity(menu)
        .insert((GlobalZIndex(50), BorderColor::all(EDGE)));
    for (label, action) in [
        ("Просмотр", EditAction::View),
        ("Редактирование", EditAction::Edit),
    ] {
        let item = button(
            commands,
            menu,
            font_handle,
            label,
            Action::ImageEdit(action),
            Look::Regular,
        );
        commands.entity(item).insert(CanvasControl);
        commands.entity(item).entry::<Node>().and_modify(|mut n| {
            n.width = percent(100);
            n.justify_content = JustifyContent::Start;
        });
    }
    let tools = surface(
        commands,
        body,
        Node {
            display: Display::None,
            width: px(54),
            min_width: px(54),
            align_items: AlignItems::Center,
            min_height: px(0),
            padding: UiRect::all(px(7)),
            border: UiRect::right(px(1)),
            ..column(6.0)
        },
        CHROME,
    );
    commands.entity(body).insert_children(0, &[tools]);
    let model_tools = node(
        commands,
        tools,
        Node {
            display: Display::None,
            ..column(6.0)
        },
    );
    for (tool, glyph) in crate::editor::ui::tool_icons::TRANSFORMS {
        let item = control(
            commands,
            model_tools,
            Action::ImageEdit(EditAction::ModelTool(tool)),
            Look::Tool,
            tool_node(),
        );
        commands.entity(item).insert((
            CanvasControl,
            AccessibleLabel(crate::editor::ui::tool_icons::label(tool, true).into()),
        ));
        icons::themed(commands, item, glyph, 18.0);
    }
    let model_pose = button(
        commands,
        model_tools,
        font_handle,
        "Поза",
        Action::ImageEdit(EditAction::ModelPose),
        Look::Tool,
    );
    commands
        .entity(model_pose)
        .insert(CanvasControl)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = px(38);
            node.height = px(36);
            node.padding = UiRect::horizontal(px(2));
            node.justify_content = JustifyContent::Center;
        });
    let model_pose_reset = control(
        commands,
        model_tools,
        Action::ImageEdit(EditAction::ModelPoseReset),
        Look::Tool,
        tool_node(),
    );
    icons::themed(commands, model_pose_reset, icons::Glyph::Refresh, 18.0);
    commands.entity(model_pose_reset).insert((
        CanvasControl,
        AccessibleLabel("Сбросить позу".into()),
    ));
    let raster_tools = node(commands, tools, column(6.0));
    super::selection_ui::setup(commands, raster_tools, toolbar, canvas, root, font_handle);
    let paint_group = node(
        commands,
        raster_tools,
        Node {
            width: px(36),
            height: px(36),
            flex_shrink: 0.0,
            ..default()
        },
    );
    let paint_button = control(
        commands,
        paint_group,
        Action::ImageEdit(EditAction::DrawingTool),
        Look::Tool,
        tool_node(),
    );
    commands.entity(paint_button).insert(AccessibleLabel(
        "Кисть · B · ПКМ — выбрать кисть или карандаш".into(),
    ));
    let brush_icon = icons::themed(commands, paint_button, icons::Glyph::Brush, 18.0);
    let pencil_icon = icons::themed(commands, paint_button, icons::Glyph::Pencil, 18.0);
    tool_menu_hint(commands, paint_button);
    let eraser_group = node(
        commands,
        raster_tools,
        Node {
            width: px(36),
            height: px(36),
            flex_shrink: 0.0,
            ..default()
        },
    );
    let eraser_button = control(
        commands,
        eraser_group,
        Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser)),
        Look::Tool,
        tool_node(),
    );
    icons::themed(commands, eraser_button, icons::Glyph::Eraser, 18.0);
    tool_menu_hint(commands, eraser_button);
    let eraser_modes = surface(
        commands,
        eraser_group,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(42),
            top: px(0),
            width: px(230),
            padding: UiRect::all(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(eraser_modes).insert((
        GlobalZIndex(60),
        BorderColor::all(EDGE),
        AccessibleLabel("Тип ластика".into()),
    ));
    for (label, mode) in [
        ("Обычный ластик", EraserMode::Brush),
        ("Пиксельный ластик", EraserMode::Pixel),
    ] {
        let item = control(
            commands,
            eraser_modes,
            Action::ImageEdit(EditAction::EraserMode(mode)),
            Look::Regular,
            Node {
                width: percent(100),
                height: px(31),
                padding: UiRect::horizontal(px(8)),
                border_radius: BorderRadius::all(px(4)),
                ..row(8.0)
            },
        );
        commands.entity(item).insert(AccessibleLabel(label.into()));
        icons::themed(commands, item, icons::Glyph::Eraser, 16.0);
        text(commands, item, font_handle, label, 12.0, TEXT);
    }
    let paint_menu = surface(
        commands,
        paint_group,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(42),
            top: px(0),
            width: px(230),
            padding: UiRect::all(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(paint_menu).insert((
        GlobalZIndex(60),
        BorderColor::all(EDGE),
        AccessibleLabel("Инструменты рисования".into()),
    ));
    tool_button(
        commands,
        raster_tools,
        icons::Glyph::Gradient,
        "Градиент · U",
        PhotoTool::Gradient,
    );
    for (glyph, label, tool) in [
        (icons::Glyph::Brush, "Кисть", PhotoTool::Brush),
        (icons::Glyph::Pencil, "Карандаш", PhotoTool::Pencil),
    ] {
        let item = control(
            commands,
            paint_menu,
            Action::ImageEdit(EditAction::Tool(tool)),
            Look::Regular,
            Node {
                width: percent(100),
                height: px(31),
                padding: UiRect::horizontal(px(8)),
                border_radius: BorderRadius::all(px(4)),
                ..row(8.0)
            },
        );
        icons::themed(commands, item, glyph, 16.0);
        text(commands, item, font_handle, label, 12.0, TEXT);
        node(
            commands,
            item,
            Node {
                flex_grow: 1.0,
                ..default()
            },
        );
        text(commands, item, font_handle, "B", 11.0, MUTED);
    }
    node(
        commands,
        tools,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    for (glyph, label, action) in [
        (icons::Glyph::Undo, "Отменить · Ctrl+Z", EditAction::Undo),
        (
            icons::Glyph::Redo,
            "Повторить · Ctrl+Shift+Z",
            EditAction::Redo,
        ),
    ] {
        icons::button(
            commands,
            tools,
            glyph,
            label,
            Action::ImageEdit(action),
            Look::Regular,
        );
    }
    let color_tools = node(commands, tools, column(0.0));
    super::picker_ui::setup(commands, root, color_tools, font_handle);
    let panel_root = node(
        commands,
        body,
        Node {
            display: Display::None,
            width: px(300),
            min_width: px(0),
            min_height: px(0),
            flex_shrink: 0.0,
            overflow: Overflow::clip(),
            ..default()
        },
    );
    let panel = surface(
        commands,
        panel_root,
        Node {
            width: percent(100),
            height: percent(100),
            min_width: px(0),
            min_height: px(0),
            padding: UiRect::all(px(12)),
            border: UiRect::left(px(1)),
            overflow: Overflow::clip(),
            ..column(4.0)
        },
        CHROME,
    );
    commands.entity(panel).insert(Scroller);
    super::super::panel_resize::setup(commands, panel_root);
    let error = text(commands, panel, font_handle, "", 11.0, ERROR);
    commands.entity(error).insert(Node {
        display: Display::None,
        ..default()
    });
    super::super::overview::setup(commands, panel, checker_image);
    let layer_header = node(
        commands,
        panel,
        Node {
            width: percent(100),
            justify_content: JustifyContent::FlexEnd,
            ..row(6.0)
        },
    );
    let actions = node(commands, layer_header, row(2.0));
    for (glyph, label, action) in [(
        icons::Glyph::Search,
        "Поиск и фильтры слоёв",
        EditAction::Layers(layers::LayerAction::PanelSection(
            layers::PanelSection::Search,
        )),
    )] {
        let item = icons::button(
            commands,
            actions,
            glyph,
            label,
            Action::ImageEdit(action),
            Look::Regular,
        );
        commands.entity(item).entry::<Node>().and_modify(|mut n| {
            n.width = px(25);
            n.height = px(25);
        });
    }
    let layers = node(
        commands,
        panel,
        Node {
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.0,
            ..column(3.0)
        },
    );
    layers::ui::setup(commands, layers, panel_root, root, font_handle);
    let cursor = super::super::tool_cursor::Overlay::setup(commands, canvas, font_handle);
    commands.insert_resource(EditUi {
        mode_button,
        mode,
        menu,
        tools,
        raster_tools,
        color_tools,
        paint_button,
        eraser_button,
        brush_icon,
        pencil_icon,
        paint_menu,
        panel: panel_root,
        eraser_modes,
        error,
        size_group,
        model_tools,
        model_pose,
        model_pose_reset,
        model_overlay,
        brush_rail,
        size_button,
        size_value,
        size_menu,
        size_settings,
        library,
        size_title,
        hardness_group,
        pencil_shapes,
        shape_items,
        size_shown: false,
        size_input,
        size_original: None,
        size_last_text: None,
        size_invalid: false,
        size_confirm: false,
        cursor,
    });
}

pub(in crate::editor) fn color_pointer(
    mut viewer: ResMut<ImageViewer>,
    editor: Res<Editor>,
    mut ui: ResMut<EditUi>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Option<Res<DrawerMotion>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform)>,
    inputs: Query<&EditableText>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
    scrub: Option<Res<crate::editor::ui::NumberScrub>>,
) {
    viewer.edit.paint_menu_dismissed = false;
    viewer.edit.mode_menu_dismissed = false;
    viewer.edit.size_menu_dismissed = false;
    if editor.pending.is_some()
        || editor.browser_popup
        || viewer.panel.blocks_canvas()
        || viewer.edit.color_picker.is_some()
        || viewer.edit.layers.menu
        || viewer.edit.layers.form.is_some()
        || window.cursor_position().is_some_and(|point| {
            drawer
                .as_ref()
                .is_some_and(|drawer| drawer.covers(point, &window))
        })
    {
        return;
    }
    if viewer.edit.menu
        && (mouse.just_pressed(MouseButton::Left)
            && window.cursor_position().is_some_and(|p| {
                ![ui.mode_button, ui.menu].into_iter().any(|e| {
                    nodes
                        .get(e)
                        .is_ok_and(|(n, t)| n.contains_point(*t, p / n.inverse_scale_factor()))
                })
            }))
    {
        viewer.edit.menu = false;
        viewer.edit.mode_menu_dismissed = true;
    }
    if !viewer.edit.editing(editor.viewing_image) {
        viewer.edit.paint_menu = false;
        return;
    }
    if viewer.edit.model_layer_selected(editor.viewing_image) {
        viewer.edit.paint_menu = false;
        viewer.edit.size_menu = false;
        viewer.edit.selection.menu = 0;
        return;
    }
    let hit_size = |entity| {
        window.cursor_position().is_some_and(|cursor| {
            nodes.get(entity).is_ok_and(|(n, t)| {
                !n.is_empty() && n.contains_point(*t, cursor / n.inverse_scale_factor())
            })
        })
    };
    if viewer.edit.size_menu {
        let dismiss = mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
            && !hit_size(ui.size_button)
            && !hit_size(ui.size_menu);
        let rect = super::super::canvas_rect(&canvas, &window);
        let input_rect = drawer
            .as_ref()
            .map_or(rect, |d| d.input_rect(rect, window.height()));
        let shift = keys.any_just_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
            && !scrub.as_ref().is_some_and(|scrub| scrub.capturing())
            && window
                .cursor_position()
                .is_some_and(|p| input_rect.contains(p));
        if dismiss || shift {
            viewer.edit.size_menu = false;
            viewer.edit.size_menu_dismissed = dismiss;
            ui.size_original = None;
            focus.clear();
        }
    }
    if viewer.edit.sizeable()
        && !viewer.edit.busy()
        && hit_size(ui.size_button)
        && scroll.delta.y != 0.0
    {
        let delta = if scroll.unit == MouseScrollUnit::Line {
            scroll.delta.y
        } else {
            scroll.delta.y / 24.0
        };
        let step = if delta.abs() < 1.0 {
            delta.signum()
        } else {
            delta.round()
        };
        actions.write(Action::ImageEdit(EditAction::Size(
            (viewer.edit.size.round() + step).clamp(1.0, 256.0),
        )));
        viewer.edit.size_menu = false;
        ui.size_original = None;
        focus.clear();
    }
    if focus.get() == Some(ui.size_input) {
        let confirm = ui.size_confirm;
        ui.size_confirm = false;
        if ui.size_original.is_none() {
            ui.size_original = Some(viewer.edit.size);
            ui.size_last_text = None;
        }
        if let Ok(input) = inputs.get(ui.size_input) {
            let value = input.value().to_string();
            if ui.size_last_text.as_ref() != Some(&value) {
                let size = value
                    .trim()
                    .replace(',', ".")
                    .parse::<f32>()
                    .ok()
                    .filter(|size| size.is_finite() && (1.0..=256.0).contains(size));
                ui.size_invalid = size.is_none();
                if let Some(size) = size {
                    actions.write(Action::ImageEdit(EditAction::Size(size)));
                }
                ui.size_last_text = Some(value);
            }
        }
        // Native text edits are applied in PostUpdate. Read the last typed character
        // on the next frame before blurring, even when typing and Enter arrive together.
        if keys.just_pressed(KeyCode::Enter) {
            ui.size_confirm = true;
        }
        if confirm && !ui.size_invalid {
            focus.clear();
            ui.size_original = None;
            viewer.edit.size_menu = false;
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        viewer.edit.size_menu = false;
        if let Some(original) = ui.size_original.take() {
            actions.write(Action::ImageEdit(EditAction::Size(original)));
            focus.clear();
        }
    }
    if focus.get() != Some(ui.size_input) {
        ui.size_original = None;
        ui.size_last_text = None;
        ui.size_invalid = false;
        ui.size_confirm = false;
    }
    if let Some(cursor) = window.cursor_position() {
        let hit = |entity| {
            nodes.get(entity).is_ok_and(|(node, transform)| {
                !node.is_empty()
                    && node.contains_point(*transform, cursor / node.inverse_scale_factor())
            })
        };
        if mouse.just_pressed(MouseButton::Right) && hit(ui.paint_button) && !viewer.edit.busy() {
            actions.write(Action::ImageEdit(EditAction::PaintMenu));
            viewer.edit.paint_menu_dismissed = true;
            focus.clear();
        } else if mouse.just_pressed(MouseButton::Right)
            && hit(ui.eraser_button)
            && !viewer.edit.busy()
        {
            actions.write(Action::ImageEdit(EditAction::EraserMenu));
            viewer.edit.paint_menu_dismissed = true;
            focus.clear();
        } else if viewer.edit.paint_menu
            && mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
            && !hit(if viewer.edit.tool_menu == ToolMenu::Eraser {
                ui.eraser_modes
            } else {
                ui.paint_menu
            })
        {
            viewer.edit.paint_menu = false;
            viewer.edit.paint_menu_dismissed = true;
        }
    }
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    (viewer, references, mut images, mut handles): (
        Res<ImageViewer>,
        Res<crate::editor::image_reference::ReferenceRender>,
        ResMut<Assets<Image>>,
        ResMut<Assets<crate::editor::viewport::HandleMaterial>>,
    ),
    window: Single<&Window, With<PrimaryWindow>>,
    mut ui: ResMut<EditUi>,
    mut focus: ResMut<InputFocus>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    sliders: Query<(
        Entity,
        &PhotoSlider,
        &SliderValue,
        &SliderRange,
        &Slider,
        &ComputedNode,
    )>,
    mut inputs: Query<&mut EditableText>,
    mut borders: Query<&mut BorderColor>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    parents: Query<&ChildOf>,
    mut colors: Query<&mut BackgroundColor>,
    mut rails: Query<(&mut RailControl, &Hovered, &SliderDragState)>,
    time: Res<Time>,
) {
    let editing = viewer.edit.editing(editor.viewing_image);
    let model_active = editing && viewer.edit.model_layer_selected(editor.viewing_image);
    ui.model_overlay.refresh(
        &editor,
        &viewer,
        &references,
        super::super::canvas_rect(&canvas, &window),
        window.cursor_position(),
        window.scale_factor(),
        &mut images,
        &mut handles,
        &mut nodes,
        &mut commands,
    );
    commands.entity(ui.brush_rail).insert(rail_fx::RevealTarget(
        editing && viewer.edit.sizeable() && !model_active,
    ));
    for (entity, visible) in [
        (ui.hardness_group, !viewer.edit.pixel_tool()),
        (ui.pencil_shapes, viewer.edit.pixel_tool()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    for (entity, title) in ui.shape_items {
        let tool = if viewer.edit.tool == PhotoTool::Eraser {
            "ластика"
        } else {
            "карандаша"
        };
        commands
            .entity(entity)
            .insert(AccessibleLabel(format!("Форма {tool} · {title}")));
    }
    if let Ok(mut group) = nodes.get_mut(ui.size_group) {
        group.display = if editing && viewer.edit.sizeable() && !model_active {
            Display::Flex
        } else {
            Display::None
        };
    }
    let size_shown =
        editing && viewer.edit.sizeable() && viewer.edit.size_menu && !model_active;
    let library_shown = size_shown && viewer.edit.tool == PhotoTool::Brush;
    if let Ok(mut menu) = nodes.get_mut(ui.size_menu) {
        menu.width = px(if library_shown {
            (window.width() - 165.0).clamp(250.0, 460.0)
        } else {
            264.0
        });
        menu.height = if library_shown {
            px((window.height() - 115.0).clamp(140.0, 560.0))
        } else {
            Val::Auto
        };
        menu.display = if size_shown {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (entity, visible) in [
        (ui.library.root, library_shown),
        (ui.size_settings, !library_shown),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    if library_shown {
        ui.library
            .show_category(&mut commands, &mut images, viewer.edit.brush_category);
    }
    if size_shown && !library_shown && !ui.size_shown {
        if let Ok(mut input) = inputs.get_mut(ui.size_input) {
            input.editor_mut().set_text(&size_label(viewer.edit.size));
        }
        focus.set(ui.size_input, bevy::input_focus::FocusCause::Auto);
    } else if !size_shown && ui.size_shown {
        if focus.get().is_some_and(|e| {
            parents
                .iter_ancestors(e)
                .any(|parent| parent == ui.size_menu)
        }) {
            focus.clear();
        }
        ui.size_original = None;
        ui.size_last_text = None;
        ui.size_invalid = false;
        ui.size_confirm = false;
    }
    ui.size_shown = size_shown;
    if let Ok(mut label) = labels.get_mut(ui.size_value) {
        let size = size_label((viewer.edit.size * 10.0).round() / 10.0);
        let title = if viewer.edit.tool == PhotoTool::Brush {
            let name = viewer
                .edit
                .brush_preset
                .and_then(|index| brush_library::library().presets.get(index))
                .map_or("Кисти", |preset| preset.name.as_str());
            format!("{name} · {size} px")
        } else {
            format!("{size} px")
        };
        label.set_if_neq(Text(title));
    }
    if let Ok(mut title) = labels.get_mut(ui.size_title) {
        title.set_if_neq(Text(
            match viewer.edit.tool {
                PhotoTool::Eraser => "Размер ластика",
                PhotoTool::Pencil => "Размер карандаша",
                _ => "Размер кисти",
            }
            .into(),
        ));
    }
    if focus.get() != Some(ui.size_input)
        && let Ok(mut input) = inputs.get_mut(ui.size_input)
    {
        let value = size_label(viewer.edit.size);
        if input.value().to_string() != value {
            input.editor_mut().set_text(&value);
        }
    }
    if let Ok(mut border) = borders.get_mut(ui.size_input) {
        border.set_if_neq(BorderColor::all(if ui.size_invalid {
            ERROR
        } else if focus.get() == Some(ui.size_input) {
            FOCUS
        } else {
            EDGE
        }));
    }
    if let Ok(mut panel) = nodes.get_mut(ui.panel) {
        panel.width = px(viewer.panel.width(window.width()));
    }
    for (entity, visible) in [
        (ui.tools, editing),
        (ui.raster_tools, !model_active),
        (ui.color_tools, !model_active),
        (ui.model_tools, model_active),
        (ui.model_pose, model_active && viewer.edit.model_has_rig),
        (
            ui.model_pose_reset,
            model_active && viewer.edit.model_has_rig && viewer.edit.model_posing,
        ),
        (
            ui.paint_menu,
            editing
                && !model_active
                && viewer.edit.paint_menu
                && viewer.edit.tool_menu == ToolMenu::Paint
                && editor.pending.is_none(),
        ),
        (ui.brush_icon, viewer.edit.drawing_tool != PhotoTool::Pencil),
        (
            ui.pencil_icon,
            viewer.edit.drawing_tool == PhotoTool::Pencil,
        ),
        (ui.panel, editing),
        (
            ui.eraser_modes,
            editing
                && !model_active
                && viewer.edit.paint_menu
                && viewer.edit.tool_menu == ToolMenu::Eraser
                && editor.pending.is_none(),
        ),
        (ui.menu, viewer.edit.menu && editor.viewing_image.is_some()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    commands.entity(ui.paint_button).insert(AccessibleLabel(
        if viewer.edit.drawing_tool == PhotoTool::Pencil {
            "Карандаш · B · ПКМ — выбрать кисть или карандаш"
        } else {
            "Кисть · B · ПКМ — выбрать кисть или карандаш"
        }
        .into(),
    ));
    commands.entity(ui.eraser_button).insert(AccessibleLabel(
        if viewer.edit.eraser_mode == EraserMode::Pixel {
            "Пиксельный ластик · E · ПКМ — выбрать тип ластика"
        } else {
            "Обычный ластик · E · ПКМ — выбрать тип ластика"
        }
        .into(),
    ));
    if let Ok(mut text) = labels.get_mut(ui.mode) {
        text.set_if_neq(Text(
            if editing {
                "Редактирование"
            } else {
                "Просмотр"
            }
            .into(),
        ));
    }
    let doc = viewer.edit.document(editor.viewing_image);
    let message = viewer.edit.error.as_deref().or_else(|| {
        references.pending_for(editor.viewing_image).then_some("Рендер слоя-референса…")
    });
    if let Ok(mut text) = labels.get_mut(ui.error) {
        text.set_if_neq(Text(message.unwrap_or_default().into()));
    }
    if let Ok(mut node) = nodes.get_mut(ui.error) {
        let display = if message.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    if let Some(doc) = doc {
        for (entity, slider, value, range, control, computed) in &sliders {
            let (number, label) = match slider.binding {
                Binding::Size => (
                    viewer.edit.size,
                    format!("{} px", size_label(viewer.edit.size)),
                ),
                Binding::Hardness => {
                    let hardness = if viewer.edit.pixel_tool() {
                        1.0
                    } else {
                        viewer.edit.hardness
                    };
                    (hardness, format!("{:.0}%", hardness * 100.0))
                }
                Binding::BrushOpacity => (
                    viewer.edit.opacity,
                    format!("{:.0}%", viewer.edit.opacity * 100.0),
                ),
                Binding::LayerOpacity(id) => {
                    let Some(layer) = doc.document.layer(id) else {
                        continue;
                    };
                    (layer.opacity, format!("{:.0}%", layer.opacity * 100.0))
                }
            };
            let rail_position = if matches!(control.orientation, SliderOrientation::Vertical)
                && matches!(slider.binding, Binding::Size)
            {
                rail_position(number)
            } else {
                number
            };
            if value.0 != rail_position {
                commands
                    .entity(entity)
                    .try_insert(SliderValue(rail_position));
            }
            if matches!(control.orientation, SliderOrientation::Vertical)
                && matches!(slider.binding, Binding::Size)
            {
                let step = if viewer.edit.pixel_tool() {
                    ((number + 1.0).min(256.0).log2() / 8.0 - rail_position).max(0.001)
                } else {
                    0.01
                };
                commands.entity(entity).try_insert(SliderStep(step));
            }
            if matches!(slider.binding, Binding::Hardness) {
                if viewer.edit.pixel_tool() {
                    commands.entity(entity).try_insert(InteractionDisabled);
                } else {
                    commands.entity(entity).try_remove::<InteractionDisabled>();
                }
            }
            if let Ok(mut label_node) = labels.get_mut(slider.readout) {
                label_node.set_if_neq(Text(label));
            }
            let position = ((rail_position - range.start()) / range.span()).clamp(0.0, 1.0);
            if let Ok((mut rail, hovered, drag)) = rails.get_mut(entity) {
                let travel = (computed.size().y * computed.inverse_scale_factor()
                    - RAIL_THUMB_HEIGHT)
                    .max(0.0);
                let top = travel * (1.0 - position);
                if let Ok(mut thumb) = nodes.get_mut(slider.thumb) {
                    thumb.top = px(top);
                }
                if let Ok(mut fill) = nodes.get_mut(rail.fill) {
                    fill.height = px(travel * position);
                }
                let engaged = editing && (hovered.0 || drag.dragging);
                if let Ok(mut badge) = nodes.get_mut(rail.badge) {
                    badge.display = if engaged {
                        Display::Flex
                    } else {
                        Display::None
                    };
                    badge.top = px(top - 1.0);
                }
                let goal = if engaged { 1.0 } else { 0.0 };
                let emphasis = rail.emphasis
                    + (goal - rail.emphasis) * (1.0 - (-time.delta_secs().min(0.05) * 18.0).exp());
                rail.emphasis = emphasis;
                for surface in [rail.handle, rail.fill] {
                    commands
                        .entity(surface)
                        .insert(rail_fx::Interaction { emphasis });
                }
            } else if let Ok(mut thumb) = nodes.get_mut(slider.thumb) {
                let travel = (computed.size().x * computed.inverse_scale_factor() - 12.0).max(0.0);
                thumb.left = px(position * travel);
            }
        }
    }

    let position = canvas.single().ok().and_then(|(node, transform)| {
        let min =
            (transform.affine().translation - node.size() * 0.5) * node.inverse_scale_factor();
        viewer.brush_cursor_position.map(|point| point - min)
    });
    ui.cursor.refresh(
        &mut nodes,
        &mut labels,
        viewer.brush_cursor.filter(|_| !viewer.native_brush_cursor),
        position,
        viewer.edit.resizing_brush().then_some(viewer.edit.size),
        viewer.edit.pixel_tool(),
    );
    let pipette = canvas.single().ok().and_then(|(node, transform)| {
        let min =
            (transform.affine().translation - node.size() * 0.5) * node.inverse_scale_factor();
        viewer.pipette_preview.map(|mut sample| {
            sample.position -= min;
            sample
        })
    });
    ui.cursor.refresh_pipette(&mut nodes, &mut colors, pipette);
}
