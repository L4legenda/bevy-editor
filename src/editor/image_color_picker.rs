//! Foreground/background swatches and a modal picker built from the editor's native widgets.
use super::*;
use bevy::{
    color::{Hsva, Srgba},
    input_focus::FocusCause,
    ui_widgets::SelectAllOnFocus,
};

const SIDE: u32 = 256;
#[derive(Clone, Copy)]
enum Drag {
    Plane,
    Hue,
}

#[derive(Component)]
pub(in crate::editor) struct ColorPickerScroll;

#[derive(Resource)]
pub(in crate::editor) struct PickerUi {
    text_host: bool,
    overlay: Entity,
    title: Entity,
    foreground: Entity,
    background: Entity,
    plane: Entity,
    hue_strip: Entity,
    plane_marker: Entity,
    hue_marker: Entity,
    current: Entity,
    next: Entity,
    fields: Vec<(Entity, Option<ColorComponent>)>,
    hex: Entity,
    error: Entity,
    palette: Entity,
    palette_colors: Vec<[f32; 4]>,
    texture: Option<Handle<Image>>,
    hue: Option<f32>,
    shown: Option<ColorTarget>,
    previous_focus: Option<Entity>,
    last_text: Option<(Entity, String)>,
    drag: Option<Drag>,
}

fn solid(commands: &mut Commands, parent: Entity, layout: Node, color: [f32; 4]) -> Entity {
    commands
        .spawn((
            layout,
            BackgroundColor(Color::srgba(color[0], color[1], color[2], color[3])),
            BorderColor::all(EDGE),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

fn pair_button(
    commands: &mut Commands,
    parent: Entity,
    target: ColorTarget,
    left: f32,
    top: f32,
) -> Entity {
    let button = control(
        commands,
        parent,
        Action::ImageEdit(EditAction::ColorPicker(target)),
        Look::Regular,
        Node {
            position_type: PositionType::Absolute,
            left: px(left),
            top: px(top),
            width: px(24),
            height: px(24),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
    );
    commands.entity(button).insert(AccessibleLabel(
        match target {
            ColorTarget::Foreground => "Основной цвет",
            ColorTarget::Background => "Фоновый цвет",
            ColorTarget::LayerField(_) => "Цвет слоя",
        }
        .into(),
    ));
    solid(
        commands,
        button,
        Node {
            width: percent(100),
            height: percent(100),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(2)),
            ..default()
        },
        [1.0; 4],
    )
}

fn picker_button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: EditAction,
    look: Look,
    text_host: bool,
) {
    let entity = button(
        commands,
        parent,
        font,
        label,
        route(text_host, action),
        look,
    );
    if text_host {
        commands.entity(entity).insert(AccessibleLabel(
            match action {
                EditAction::ColorConfirm => "Применить выбранный цвет",
                EditAction::ColorCancel => "Отменить выбор цвета",
                _ => "Добавить цвет в образцы",
            }
            .into(),
        ));
    }
    commands.entity(entity).entry::<Node>().and_modify(|mut n| {
        n.width = percent(100);
        n.justify_content = JustifyContent::Center;
    });
}

pub(super) fn setup(commands: &mut Commands, root: Entity, tools: Entity, font: &Handle<Font>) {
    let pair = node(
        commands,
        tools,
        Node {
            width: px(38),
            height: px(56),
            flex_shrink: 0.0,
            ..default()
        },
    );
    let reset = control(
        commands,
        pair,
        Action::ImageEdit(EditAction::ResetColors),
        Look::Regular,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: px(14),
            height: px(14),
            ..default()
        },
    );
    commands
        .entity(reset)
        .insert(AccessibleLabel("Чёрный и белый · D".into()));
    for (left, top, color) in [(5, 5, [1.0; 4]), (1, 1, [0.0, 0.0, 0.0, 1.0])] {
        solid(
            commands,
            reset,
            Node {
                position_type: PositionType::Absolute,
                left: px(left),
                top: px(top),
                width: px(8),
                height: px(8),
                border: UiRect::all(px(1)),
                ..default()
            },
            color,
        );
    }
    let swap = control(
        commands,
        pair,
        Action::ImageEdit(EditAction::SwapColors),
        Look::Regular,
        Node {
            position_type: PositionType::Absolute,
            right: px(0),
            top: px(0),
            width: px(14),
            height: px(14),
            justify_content: JustifyContent::Center,
            ..row(0.0)
        },
    );
    commands
        .entity(swap)
        .insert(AccessibleLabel("Поменять цвета местами · X".into()));
    icons::themed(commands, swap, icons::Glyph::Refresh, 12.0);
    let background = pair_button(commands, pair, ColorTarget::Background, 14.0, 30.0);
    let foreground = pair_button(commands, pair, ColorTarget::Foreground, 0.0, 16.0);

    let ui = build(commands, root, font, foreground, background, false);
    commands.insert_resource(ui);
}

pub(in crate::editor) fn setup_text(
    commands: &mut Commands,
    root: Entity,
    font: &Handle<Font>,
) -> PickerUi {
    build(commands, root, font, root, root, true)
}

fn route(text_host: bool, action: EditAction) -> Action {
    if text_host {
        Action::Narrative(crate::editor::ui::narrative::NarrativeAction::Text(
            crate::editor::ui::narrative::text_editor::TextAction::Picker(
                crate::editor::ui::narrative::text_editor::color_menu::PickerAction::from_image(
                    action,
                ),
            ),
        ))
    } else {
        Action::ImageEdit(action)
    }
}

fn build(
    commands: &mut Commands,
    root: Entity,
    font: &Handle<Font>,
    foreground: Entity,
    background: Entity,
    text_host: bool,
) -> PickerUi {
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
        Color::srgba(0.0, 0.0, 0.0, 0.58),
    );
    commands.entity(overlay).insert((
        GlobalZIndex(700),
        TabGroup::modal(),
        AccessibleLabel("Выбор цвета".into()),
    ));
    let card = surface(
        commands,
        overlay,
        Node {
            width: px(640),
            max_width: percent(94),
            max_height: percent(92),
            padding: UiRect::all(px(18)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            overflow: Overflow::scroll_y(),
            ..column(12.0)
        },
        PANEL,
    );
    commands.entity(card).insert((Scroller, ColorPickerScroll));
    let header = node(
        commands,
        card,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            flex_shrink: 0.0,
            ..row(12.0)
        },
    );
    let title = text(commands, header, font, "Основной цвет", 18.0, TEXT);
    icons::button(
        commands,
        header,
        icons::Glyph::Close,
        "Закрыть выбор цвета · Esc",
        route(text_host, EditAction::ColorCancel),
        Look::Regular,
    );
    let body = node(
        commands,
        card,
        Node {
            width: percent(100),
            align_items: AlignItems::Start,
            flex_shrink: 0.0,
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(12),
            ..row(14.0)
        },
    );
    let left = node(
        commands,
        body,
        Node {
            width: px(246),
            flex_shrink: 0.0,
            ..column(8.0)
        },
    );
    let spectrum = node(commands, left, row(8.0));
    let plane = commands
        .spawn((
            ImageNode {
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                ..default()
            },
            Node {
                width: px(220),
                height: px(220),
                flex_shrink: 0.0,
                overflow: Overflow::clip(),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            AccessibleLabel("Поле цвета".into()),
            ChildOf(spectrum),
        ))
        .id();
    let plane_marker = solid(
        commands,
        plane,
        Node {
            position_type: PositionType::Absolute,
            width: px(13),
            height: px(13),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        [0.0; 4],
    );
    commands.entity(plane_marker).insert((
        BorderColor::all(Color::BLACK),
        UiTransform::from_translation(Val2::percent(-50, -50)),
    ));
    let inner = solid(
        commands,
        plane_marker,
        Node {
            width: px(9),
            height: px(9),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        [0.0; 4],
    );
    commands
        .entity(inner)
        .insert(BorderColor::all(Color::WHITE));
    let hue_strip = surface(
        commands,
        spectrum,
        Node {
            width: px(18),
            height: px(220),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
        Color::NONE,
    );
    commands.entity(hue_strip).insert((
        AccessibleLabel("Полоса оттенка".into()),
        BackgroundGradient::from(LinearGradient {
            angle: LinearGradient::TO_BOTTOM,
            stops: (0..=6)
                .map(|i| ColorStop::auto(Color::from(Hsva::hsv(360.0 - i as f32 * 60.0, 1.0, 1.0))))
                .collect(),
            color_space: bevy::ui::InterpolationColorSpace::Srgba,
        }),
    ));
    let hue_marker = solid(
        commands,
        hue_strip,
        Node {
            position_type: PositionType::Absolute,
            left: px(-3),
            width: px(24),
            height: px(5),
            border: UiRect::all(px(1)),
            ..default()
        },
        [0.0, 0.0, 0.0, 1.0],
    );
    commands.entity(hue_marker).insert((
        BorderColor::all(Color::WHITE),
        UiTransform::from_translation(Val2::percent(0, -50)),
    ));
    text(commands, left, font, "Образцы", 11.0, MUTED);
    let palette = node(
        commands,
        left,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(4),
            ..row(4.0)
        },
    );

    let middle = node(
        commands,
        body,
        Node {
            width: px(180),
            flex_shrink: 0.0,
            ..column(10.0)
        },
    );
    let previews = node(commands, middle, row(8.0));
    let mut preview = |caption: &str| {
        let col = node(commands, previews, column(3.0));
        text(commands, col, font, caption, 11.0, MUTED);
        solid(
            commands,
            col,
            Node {
                width: px(72),
                height: px(60),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            [0.0, 0.0, 0.0, 1.0],
        )
    };
    let next = preview("Новый");
    let current = preview("Текущий");
    let values = node(commands, middle, row(10.0));
    let mut fields = Vec::new();
    for channels in [
        [
            ColorComponent::Hue,
            ColorComponent::Saturation,
            ColorComponent::Value,
        ],
        [
            ColorComponent::Red,
            ColorComponent::Green,
            ColorComponent::Blue,
        ],
    ] {
        let group = node(commands, values, column(5.0));
        for channel in channels {
            let (label, description) = match channel {
                ColorComponent::Hue => ("H", "Оттенок H · 0–360"),
                ColorComponent::Saturation => ("S", "Насыщенность S · 0–100"),
                ColorComponent::Value => ("V", "Яркость V · 0–100"),
                ColorComponent::Red => ("R", "Красный R · 0–255"),
                ColorComponent::Green => ("G", "Зелёный G · 0–255"),
                ColorComponent::Blue => ("B", "Синий B · 0–255"),
            };
            let row = node(commands, group, row(4.0));
            let caption = text(commands, row, font, label, 12.0, MUTED);
            commands.entity(caption).insert(Node {
                width: px(12),
                ..default()
            });
            let field = interface::input(commands, row, font, description);
            commands
                .entity(field)
                .insert((
                    SelectAllOnFocus,
                    crate::editor::ui::Number::decimal(0.0, channel.max(), 1.0, 0),
                ))
                .entry::<Node>()
                .and_modify(|mut n| n.width = px(58));
            fields.push((field, Some(channel)));
        }
    }
    let hex_row = node(commands, middle, row(5.0));
    text(commands, hex_row, font, "#", 13.0, MUTED);
    let hex = interface::input(commands, hex_row, font, "HEX цвета");
    commands
        .entity(hex)
        .insert(SelectAllOnFocus)
        .entry::<Node>()
        .and_modify(|mut n| n.width = px(144));
    fields.push((hex, None));
    let actions = node(
        commands,
        body,
        Node {
            width: px(126),
            flex_shrink: 0.0,
            align_self: AlignSelf::Start,
            ..column(7.0)
        },
    );
    picker_button(
        commands,
        actions,
        font,
        "ОК",
        EditAction::ColorConfirm,
        Look::Primary,
        text_host,
    );
    picker_button(
        commands,
        actions,
        font,
        "Отмена",
        EditAction::ColorCancel,
        Look::Regular,
        text_host,
    );
    picker_button(
        commands,
        actions,
        font,
        "В образцы",
        EditAction::AddSwatch,
        Look::Regular,
        text_host,
    );
    let error = text(commands, card, font, "", 12.0, ERROR);
    commands.entity(error).insert(TextLayout::default());
    PickerUi {
        text_host,
        overlay,
        title,
        foreground,
        background,
        plane,
        hue_strip,
        plane_marker,
        hue_marker,
        current,
        next,
        fields,
        hex,
        error,
        palette,
        palette_colors: Vec::new(),
        texture: None,
        hue: None,
        shown: None,
        previous_focus: None,
        last_text: None,
        drag: None,
    }
}

pub(in crate::editor) fn input(
    mut viewer: ResMut<ImageViewer>,
    mut ui: ResMut<PickerUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform)>,
    inputs: Query<&EditableText>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
) {
    let mut events = Vec::new();
    input_shared(
        viewer.edit.color_picker.as_mut(),
        &mut ui,
        &window,
        &mouse,
        &keys,
        &nodes,
        &inputs,
        &mut focus,
        &mut events,
    );
    for event in events {
        actions.write(Action::ImageEdit(event));
    }
}

pub(in crate::editor) fn input_shared(
    picker: Option<&mut color::Picker>,
    ui: &mut PickerUi,
    window: &Window,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    nodes: &Query<(&ComputedNode, &UiGlobalTransform)>,
    inputs: &Query<&EditableText>,
    focus: &mut InputFocus,
    actions: &mut Vec<EditAction>,
) {
    let Some(picker) = picker else {
        ui.drag = None;
        ui.last_text = None;
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        ui.drag = None;
        return;
    }
    if let Some(cursor) = window.cursor_position() {
        let hit = |entity| {
            nodes.get(entity).is_ok_and(|(n, t)| {
                !n.is_empty() && n.contains_point(*t, cursor / n.inverse_scale_factor())
            })
        };
        if mouse.just_pressed(MouseButton::Left) {
            ui.drag = if hit(ui.plane) {
                Some(Drag::Plane)
            } else if hit(ui.hue_strip) {
                Some(Drag::Hue)
            } else {
                None
            };
            if ui.drag.is_some() {
                focus.clear();
            }
        }
        if let Some(drag) = ui.drag
            && (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
            && let Ok((node, transform)) = nodes.get(match drag {
                Drag::Plane => ui.plane,
                Drag::Hue => ui.hue_strip,
            })
        {
            let min =
                (transform.affine().translation - node.size() * 0.5) * node.inverse_scale_factor();
            let uv = ((cursor - min) / (node.size() * node.inverse_scale_factor()).max(Vec2::ONE))
                .clamp(Vec2::ZERO, Vec2::ONE);
            actions.push(match drag {
                Drag::Plane => EditAction::ColorSv(uv.x, 1.0 - uv.y),
                Drag::Hue => EditAction::ColorComponent(ColorComponent::Hue, (1.0 - uv.y) * 360.0),
            });
        }
    }
    if !mouse.pressed(MouseButton::Left) {
        ui.drag = None;
    }
    if let Some((entity, channel)) = focus.get().and_then(|entity| {
        ui.fields
            .iter()
            .find(|(field, _)| *field == entity)
            .copied()
    }) && let Ok(field) = inputs.get(entity)
    {
        let value = field.value().to_string();
        if ui.last_text.as_ref() != Some(&(entity, value.clone()))
            || keys.just_pressed(KeyCode::Enter)
        {
            let action = match channel {
                Some(channel) => value
                    .trim()
                    .replace(',', ".")
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite() && (0.0..=channel.max()).contains(v))
                    .map(|v| EditAction::ColorComponent(channel, v)),
                None => color::parse_hex(&value).map(EditAction::ColorDraft),
            };
            if let Some(action) = action {
                picker.error = None;
                actions.push(action);
            } else {
                picker.error = Some(match channel {
                    Some(c) => format!("Введите число от 0 до {:.0}", c.max()),
                    None => "HEX: введите 3 или 6 цифр, например FF8800".into(),
                });
            }
            ui.last_text = Some((entity, value));
        }
    }
    if keys.just_pressed(KeyCode::Enter) && picker.error.is_none() {
        actions.push(EditAction::ColorConfirm);
    }
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    mut ui: ResMut<PickerUi>,
    mut focus: ResMut<InputFocus>,
    mut nodes: Query<&mut Node>,
    mut colors: Query<&mut BackgroundColor>,
    mut labels: Query<&mut Text>,
    mut pictures: Query<&mut ImageNode>,
    mut inputs: Query<&mut EditableText>,
    mut images: ResMut<Assets<Image>>,
) {
    for (entity, rgba) in [
        (ui.foreground, viewer.edit.color()),
        (ui.background, viewer.edit.background()),
    ] {
        if let Ok(mut color) = colors.get_mut(entity) {
            color.set_if_neq(BackgroundColor(Color::srgba(
                rgba[0], rgba[1], rgba[2], 1.0,
            )));
        }
    }
    let picker = viewer
        .edit
        .color_picker
        .as_ref()
        .filter(|_| viewer.edit.editing(editor.viewing_image) && editor.pending.is_none());
    refresh_shared(
        &mut commands,
        &viewer.edit.swatches,
        picker,
        &mut ui,
        &mut focus,
        &mut nodes,
        &mut colors,
        &mut labels,
        &mut pictures,
        &mut inputs,
        &mut images,
    );
}

#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn refresh_shared(
    commands: &mut Commands,
    swatches: &[[f32; 4]],
    picker: Option<&color::Picker>,
    ui: &mut PickerUi,
    focus: &mut InputFocus,
    nodes: &mut Query<&mut Node>,
    colors: &mut Query<&mut BackgroundColor>,
    labels: &mut Query<&mut Text>,
    pictures: &mut Query<&mut ImageNode>,
    inputs: &mut Query<&mut EditableText>,
    images: &mut Assets<Image>,
) {
    if let Ok(mut node) = nodes.get_mut(ui.overlay) {
        node.display = if picker.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    let Some(picker) = picker else {
        if ui.shown.take().is_some() {
            if let Some(previous) = ui.previous_focus.take() {
                focus.set(previous, FocusCause::Auto);
            } else {
                focus.clear();
            }
        }
        return;
    };
    let opening = ui.shown != Some(picker.target);
    if opening {
        ui.previous_focus = focus.get();
        ui.last_text = None;
    }
    if let Ok(mut title) = labels.get_mut(ui.title) {
        title.set_if_neq(Text(
            if ui.text_host {
                "Цвет выделенного текста"
            } else {
                match picker.target {
                    ColorTarget::Foreground => "Основной цвет",
                    ColorTarget::Background => "Фоновый цвет",
                    ColorTarget::LayerField(_) => "Цвет слоя",
                }
            }
            .into(),
        ));
    }
    for (entity, rgba) in [(ui.current, picker.original), (ui.next, picker.draft)] {
        if let Ok(mut color) = colors.get_mut(entity) {
            color.set_if_neq(BackgroundColor(Color::srgba(
                rgba[0], rgba[1], rgba[2], 1.0,
            )));
        }
    }
    for &(entity, channel) in &ui.fields {
        let invalid = picker.error.is_some()
            && ui
                .last_text
                .as_ref()
                .is_some_and(|(last, _)| *last == entity);
        if (opening || focus.get() != Some(entity))
            && !invalid
            && let Ok(mut input) = inputs.get_mut(entity)
        {
            let value =
                channel.map_or_else(|| picker.hex(), |c| format!("{:.0}", picker.component(c)));
            if input.value().to_string() != value {
                input.editor_mut().set_text(&value);
            }
        }
    }
    if let Ok(mut marker) = nodes.get_mut(ui.plane_marker) {
        marker.left = percent(picker.hsv.saturation * 100.0);
        marker.top = percent((1.0 - picker.hsv.value) * 100.0);
    }
    if let Ok(mut marker) = nodes.get_mut(ui.hue_marker) {
        marker.top = percent((1.0 - picker.hsv.hue / 360.0) * 100.0);
    }
    if let Ok(mut error) = labels.get_mut(ui.error) {
        error.set_if_neq(Text(picker.error.clone().unwrap_or_default()));
    }
    if let Ok(mut node) = nodes.get_mut(ui.error) {
        node.display = if picker.error.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if ui.hue != Some(picker.hsv.hue) {
        let mut data = Vec::with_capacity((SIDE * SIDE * 4) as usize);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let rgb = Srgba::from(Hsva::hsv(
                    picker.hsv.hue,
                    x as f32 / (SIDE - 1) as f32,
                    1.0 - y as f32 / (SIDE - 1) as f32,
                ));
                data.extend_from_slice(&[
                    (rgb.red * 255.0).round() as u8,
                    (rgb.green * 255.0).round() as u8,
                    (rgb.blue * 255.0).round() as u8,
                    255,
                ]);
            }
        }
        let texture = image_previews::texture_image(DecodedImage {
            rgba: data,
            size: UVec2::splat(SIDE),
            original: UVec2::splat(SIDE),
        });
        if let Some(mut old) = ui
            .texture
            .as_ref()
            .and_then(|handle| images.get_mut(handle))
        {
            *old = texture;
        } else {
            ui.texture = Some(images.add(texture));
        }
        if let Ok(mut node) = pictures.get_mut(ui.plane) {
            node.image = ui.texture.clone().unwrap();
        }
        ui.hue = Some(picker.hsv.hue);
    }
    if ui.palette_colors.as_slice() != swatches {
        commands.entity(ui.palette).despawn_related::<Children>();
        for &rgba in swatches {
            let swatch = control(
                commands,
                ui.palette,
                route(ui.text_host, EditAction::ColorDraft(rgba)),
                Look::Regular,
                Node {
                    width: px(24),
                    height: px(24),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
            );
            commands
                .entity(swatch)
                .insert(AccessibleLabel(format!("Образец #{}", color::hex(rgba))));
            solid(
                commands,
                swatch,
                Node {
                    width: percent(100),
                    height: percent(100),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(2)),
                    ..default()
                },
                rgba,
            );
        }
        ui.palette_colors = swatches.to_vec();
    }
    if opening {
        focus.set(ui.hex, FocusCause::Auto);
        ui.shown = Some(picker.target);
    }
}
