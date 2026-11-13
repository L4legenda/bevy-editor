//! Bevy-native controls using the editor's existing colors, buttons and icons.
use super::*;
use bevy::{
    color::{Hsva, Srgba},
    ui_widgets::{Slider, SliderRange, SliderThumb, SliderValue, TrackClick, ValueChange},
};

#[derive(Clone, Copy, Component)]
enum Binding {
    Size,
    Hardness,
    BrushOpacity,
    LayerOpacity,
    Hue,
}
#[derive(Component)]
pub(in crate::editor) struct PhotoSlider {
    binding: Binding,
    thumb: Entity,
    readout: Entity,
}
#[derive(Component)]
pub(in crate::editor) struct ColorPlane;

#[derive(Resource)]
pub(in crate::editor) struct EditUi {
    center: Entity,
    mode: Entity,
    menu: Entity,
    tools: Entity,
    paint_button: Entity,
    brush_icon: Entity,
    pencil_icon: Entity,
    paint_menu: Entity,
    panel: Entity,
    plane: Handle<Image>,
    plane_node: Entity,
    color_details: Entity,
    marker: Entity,
    chip: Entity,
    hex: Entity,
    name: Entity,
    layers: Entity,
    status: Entity,
    ring: Entity,
    hue: Option<f32>,
    layer_signature: String,
}

fn slider(
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
            left: px(7),
            right: px(7),
            top: px(8),
            height: px(4),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    commands.entity(track).insert(Pickable::IGNORE);
    if matches!(binding, Binding::Hue) {
        commands
            .entity(track)
            .insert(BackgroundGradient::from(LinearGradient {
                angle: LinearGradient::TO_RIGHT,
                stops: (0..=6)
                    .map(|i| ColorStop::auto(Color::from(Hsva::hsv(i as f32 * 60.0, 1.0, 1.0))))
                    .collect(),
                color_space: bevy::ui::InterpolationColorSpace::Srgba,
            }));
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
}

fn slider_change(
    event: On<ValueChange<f32>>,
    sliders: Query<&PhotoSlider>,
    mut actions: MessageWriter<Action>,
) {
    if let Ok(slider) = sliders.get(event.source) {
        let action = match slider.binding {
            Binding::Size => EditAction::Size(event.value),
            Binding::Hardness => EditAction::Hardness(event.value),
            Binding::BrushOpacity => EditAction::BrushOpacity(event.value),
            Binding::LayerOpacity => EditAction::Opacity(event.value, event.is_final),
            Binding::Hue => EditAction::Hue(event.value),
        };
        actions.write(Action::ImageEdit(action));
    }
}

fn input(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    label: &str,
) -> Entity {
    commands
        .spawn((
            TextInput,
            EditableText::new(""),
            TextCursorStyle::default(),
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

pub(in crate::editor) fn setup(
    commands: &mut Commands,
    toolbar: Entity,
    body: Entity,
    canvas: Entity,
    font_handle: &Handle<Font>,
) {
    let center = node(
        commands,
        toolbar,
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: px(4),
            height: px(36),
            ..row(0.0)
        },
    );
    commands.entity(center).insert((
        UiTransform::from_translation(Val2::percent(-50, 0)),
        ZIndex(4),
    ));
    let mode_button = control(
        commands,
        center,
        Action::ImageEdit(EditAction::Menu),
        Look::Regular,
        Node {
            height: percent(100),
            padding: UiRect::horizontal(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(10.0)
        },
    );
    let mode = text(commands, mode_button, font_handle, "Просмотр", 12.0, TEXT);
    icons::themed(commands, mode_button, icons::Glyph::ChevronDown, 14.0);
    let menu = surface(
        commands,
        center,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(40),
            left: percent(50),
            width: px(210),
            padding: UiRect::all(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(menu).insert((
        GlobalZIndex(50),
        BorderColor::all(EDGE),
        UiTransform::from_translation(Val2::percent(-50, 0)),
    ));
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
    icons::button(
        commands,
        tools,
        icons::Glyph::Move,
        "Перемещение слоя · V",
        Action::ImageEdit(EditAction::Tool(PhotoTool::Move)),
        Look::Tool,
    );
    let paint_button = control(
        commands,
        tools,
        Action::ImageEdit(EditAction::DrawingTool),
        Look::Tool,
        Node {
            width: px(30),
            height: px(29),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(0.0)
        },
    );
    commands.entity(paint_button).insert(AccessibleLabel(
        "Кисть · B · ПКМ — выбрать кисть или карандаш".into(),
    ));
    let brush_icon = icons::themed(commands, paint_button, icons::Glyph::Brush, 16.0);
    let pencil_icon = icons::themed(commands, paint_button, icons::Glyph::Pencil, 16.0);
    for (glyph, label, tool) in [
        (icons::Glyph::Eraser, "Ластик · E", PhotoTool::Eraser),
        (icons::Glyph::PaintBucket, "Заливка · G", PhotoTool::Fill),
        (icons::Glyph::Pipette, "Пипетка · I", PhotoTool::Eyedropper),
        (icons::Glyph::Hand, "Рука · H / Пробел", PhotoTool::Hand),
    ] {
        icons::button(
            commands,
            tools,
            glyph,
            label,
            Action::ImageEdit(EditAction::Tool(tool)),
            Look::Tool,
        );
    }
    let paint_menu = surface(
        commands,
        tools,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(49),
            top: px(40),
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
    let panel = surface(
        commands,
        body,
        Node {
            display: Display::None,
            width: px(248),
            min_width: px(248),
            min_height: px(0),
            padding: UiRect::all(px(12)),
            border: UiRect::left(px(1)),
            overflow: Overflow::scroll_y(),
            ..column(4.0)
        },
        CHROME,
    );
    commands.entity(panel).insert(Scroller);
    let save = button(
        commands,
        panel,
        font_handle,
        "Сохранить · Ctrl+S",
        Action::ImageEdit(EditAction::Save),
        Look::Primary,
    );
    commands
        .entity(save)
        .entry::<Node>()
        .and_modify(|mut n| n.width = percent(100));
    let status = text(commands, panel, font_handle, "", 11.0, MUTED);
    commands.entity(status).insert(TextLayout::default());
    // The texture is initialized by refresh once Assets<Image> is available.
    let plane_image = Handle::<Image>::default();
    let color_details = node(
        commands,
        panel,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..column(4.0)
        },
    );
    let plane_node = commands
        .spawn((
            ImageNode::new(plane_image.clone()),
            ColorPlane,
            AccessibleLabel("Выбор насыщенности и яркости".into()),
            Node {
                width: percent(100),
                height: px(86),
                min_height: px(86),
                border_radius: BorderRadius::all(px(5)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(color_details),
        ))
        .id();
    let marker = surface(
        commands,
        plane_node,
        Node {
            position_type: PositionType::Absolute,
            width: px(9),
            height: px(9),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        Color::NONE,
    );
    commands.entity(marker).insert((
        BorderColor::all(Color::WHITE),
        Pickable::IGNORE,
        UiTransform::from_translation(Val2::percent(-50, -50)),
    ));
    slider(
        commands,
        color_details,
        font_handle,
        "Оттенок",
        Binding::Hue,
        (0.0, 360.0),
    );
    let colorrow = node(
        commands,
        panel,
        Node {
            width: percent(100),
            ..row(6.0)
        },
    );
    commands.entity(panel).insert_children(2, &[colorrow]);
    let chip = control(
        commands,
        colorrow,
        Action::ImageEdit(EditAction::ColorPanel),
        Look::Regular,
        Node {
            width: px(28),
            height: px(28),
            min_width: px(28),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..default()
        },
    );
    commands.entity(chip).insert((AccessibleLabel(
        "Цвет · Раскрыть или свернуть выбор цвета".into(),
    ),));
    let hex = input(
        commands,
        colorrow,
        font_handle,
        "Цвет HEX · Enter — применить",
    );
    let palette = node(
        commands,
        panel,
        Node {
            width: percent(100),
            ..row(4.0)
        },
    );
    for rgb in [
        [0.05, 0.05, 0.06],
        [1.0, 1.0, 1.0],
        [0.95, 0.24, 0.26],
        [1.0, 0.65, 0.20],
        [0.28, 0.75, 0.62],
        [0.25, 0.55, 1.0],
        [0.65, 0.4, 0.95],
    ] {
        let swatch = control(
            commands,
            palette,
            Action::ImageEdit(EditAction::Color([rgb[0], rgb[1], rgb[2], 1.0])),
            Look::Regular,
            Node {
                width: px(24),
                height: px(22),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
        );
        let color = surface(
            commands,
            swatch,
            Node {
                width: percent(100),
                height: percent(100),
                border_radius: BorderRadius::all(px(3)),
                ..default()
            },
            Color::srgb(rgb[0], rgb[1], rgb[2]),
        );
        commands.entity(color).insert(Pickable::IGNORE);
    }
    slider(
        commands,
        panel,
        font_handle,
        "Размер кисти",
        Binding::Size,
        (1.0, 256.0),
    );
    slider(
        commands,
        panel,
        font_handle,
        "Жёсткость",
        Binding::Hardness,
        (0.0, 1.0),
    );
    slider(
        commands,
        panel,
        font_handle,
        "Непрозрачность кисти",
        Binding::BrushOpacity,
        (0.01, 1.0),
    );
    let layer_header = node(
        commands,
        panel,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(6.0)
        },
    );
    text(commands, layer_header, font_handle, "Слои", 13.0, TEXT);
    let actions = node(commands, layer_header, row(2.0));
    for (glyph, label, action) in [
        (icons::Glyph::Plus, "Новый слой", EditAction::AddLayer),
        (
            icons::Glyph::Restore,
            "Дублировать слой",
            EditAction::DuplicateLayer,
        ),
        (icons::Glyph::ChevronUp, "Слой выше", EditAction::LayerUp),
        (
            icons::Glyph::ChevronDown,
            "Слой ниже",
            EditAction::LayerDown,
        ),
        (icons::Glyph::Close, "Удалить слой", EditAction::DeleteLayer),
    ] {
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
    let name = input(
        commands,
        panel,
        font_handle,
        "Название слоя · Enter — применить",
    );
    slider(
        commands,
        panel,
        font_handle,
        "Непрозрачность слоя",
        Binding::LayerOpacity,
        (0.0, 1.0),
    );
    let layers = node(
        commands,
        panel,
        Node {
            width: percent(100),
            min_height: px(60),
            flex_shrink: 0.0,
            ..column(3.0)
        },
    );
    let ring = surface(
        commands,
        canvas,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(24),
            height: px(24),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        Color::NONE,
    );
    commands.entity(ring).insert((
        BorderColor::all(Color::WHITE),
        Pickable::IGNORE,
        ZIndex(8),
        UiTransform::from_translation(Val2::percent(-50, -50)),
    ));
    commands.insert_resource(EditUi {
        center,
        mode,
        menu,
        tools,
        paint_button,
        brush_icon,
        pencil_icon,
        paint_menu,
        panel,
        plane: plane_image,
        plane_node,
        color_details,
        marker,
        chip,
        hex,
        name,
        layers,
        status,
        ring,
        hue: None,
        layer_signature: String::new(),
    });
}

fn paint_plane(data: &mut [u8], hue: f32) {
    for (i, pixel) in data.chunks_exact_mut(4).enumerate() {
        let rgb = Srgba::from(Hsva::hsv(
            hue,
            (i % 192) as f32 / 191.0,
            1.0 - (i / 192) as f32 / 95.0,
        ));
        pixel.copy_from_slice(&[
            (rgb.red * 255.0) as u8,
            (rgb.green * 255.0) as u8,
            (rgb.blue * 255.0) as u8,
            255,
        ]);
    }
}

pub(in crate::editor) fn color_pointer(
    mut viewer: ResMut<ImageViewer>,
    editor: Res<Editor>,
    ui: Res<EditUi>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    planes: Query<(&ComputedNode, &UiGlobalTransform), With<ColorPlane>>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform)>,
    inputs: Query<&EditableText>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
) {
    viewer.edit.paint_menu_dismissed = false;
    if editor.pending.is_some() || editor.browser_popup {
        return;
    }
    if viewer.edit.menu
        && (mouse.just_pressed(MouseButton::Left)
            && window.cursor_position().is_some_and(|p| {
                ![ui.center, ui.menu].into_iter().any(|e| {
                    nodes
                        .get(e)
                        .is_ok_and(|(n, t)| n.contains_point(*t, p / n.inverse_scale_factor()))
                })
            }))
    {
        viewer.edit.menu = false;
    }
    if !viewer.edit.editing(editor.viewing_image) {
        viewer.edit.paint_menu = false;
        return;
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
        } else if viewer.edit.paint_menu
            && mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
            && !hit(ui.paint_menu)
        {
            viewer.edit.paint_menu = false;
            viewer.edit.paint_menu_dismissed = true;
        }
    }
    if let Ok((node, transform)) = planes.single()
        && mouse.pressed(MouseButton::Left)
        && let Some(cursor) = window.cursor_position()
        && node.contains_point(*transform, cursor / node.inverse_scale_factor())
    {
        let min =
            (transform.affine().translation - node.size() * 0.5) * node.inverse_scale_factor();
        let point = ((cursor - min) / (node.size() * node.inverse_scale_factor()))
            .clamp(Vec2::ZERO, Vec2::ONE);
        actions.write(Action::ImageEdit(EditAction::SaturationValue(
            point.x,
            1.0 - point.y,
        )));
    }
    if keys.just_pressed(KeyCode::Enter) {
        if focus.get() == Some(ui.hex)
            && let Ok(input) = inputs.get(ui.hex)
        {
            let value = input
                .value()
                .to_string()
                .trim()
                .trim_start_matches('#')
                .to_owned();
            if value.len() == 6
                && let Ok(rgb) = u32::from_str_radix(&value, 16)
            {
                actions.write(Action::ImageEdit(EditAction::Color([
                    ((rgb >> 16) & 255) as f32 / 255.0,
                    ((rgb >> 8) & 255) as f32 / 255.0,
                    (rgb & 255) as f32 / 255.0,
                    1.0,
                ])));
                focus.clear();
            } else {
                viewer.edit.error = Some("Цвет: введите 6 цифр HEX, например #E8AD5C".into());
            }
        } else if focus.get() == Some(ui.name)
            && let Ok(input) = inputs.get(ui.name)
            && let Some(id) = editor.viewing_image
            && let Some(session) = viewer.edit.sessions.get_mut(&id)
        {
            session.doc.rename(&input.value().to_string());
            focus.clear();
        }
    }
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    interface: Res<Interface>,
    mut ui: ResMut<EditUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    focus: Res<InputFocus>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut colors: Query<&mut BackgroundColor>,
    mut pictures: Query<&mut ImageNode>,
    sliders: Query<(Entity, &PhotoSlider, &SliderValue, &SliderRange)>,
    mut inputs: Query<&mut EditableText>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
) {
    let editing = viewer.edit.editing(editor.viewing_image);
    for (entity, visible) in [
        (ui.tools, editing),
        (
            ui.paint_menu,
            editing && viewer.edit.paint_menu && editor.pending.is_none(),
        ),
        (ui.brush_icon, viewer.edit.drawing_tool != PhotoTool::Pencil),
        (
            ui.pencil_icon,
            viewer.edit.drawing_tool == PhotoTool::Pencil,
        ),
        (ui.panel, editing),
        (
            ui.color_details,
            editing && viewer.edit.color_expanded.unwrap_or(!viewer.edit.compact),
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
    if let Ok(mut text) = labels.get_mut(ui.status) {
        text.set_if_neq(Text(viewer.edit.error.clone().unwrap_or_else(|| {
            if viewer.edit.busy() {
                "Подождите…".into()
            } else if doc.is_some_and(PhotoDocument::dirty) {
                "Есть несохранённые изменения".into()
            } else {
                "Слои сохранены · .pcraft".into()
            }
        })));
    }
    let rgba = viewer.edit.color();
    let hsv = viewer
        .edit
        .hsv
        .unwrap_or_else(|| Hsva::from(Srgba::new(rgba[0], rgba[1], rgba[2], 1.0)));
    if let Ok(mut color) = colors.get_mut(ui.chip) {
        color.0 = Color::srgba(rgba[0], rgba[1], rgba[2], 1.0);
    }
    if ui.hue != Some(hsv.hue) {
        let mut data = vec![255; 192 * 96 * 4];
        paint_plane(&mut data, hsv.hue);
        let image = image_previews::texture_image(DecodedImage {
            rgba: data,
            size: UVec2::new(192, 96),
            original: UVec2::new(192, 96),
        });
        if ui.hue.is_some()
            && let Some(mut existing) = images.get_mut(&ui.plane)
        {
            *existing = image;
        } else {
            ui.plane = images.add(image);
        }
        ui.hue = Some(hsv.hue);
        if let Ok(mut picture) = pictures.get_mut(ui.plane_node) {
            picture.image = ui.plane.clone();
        }
    }
    if let Ok(mut marker) = nodes.get_mut(ui.marker) {
        marker.left = percent(hsv.saturation * 100.0);
        marker.top = percent((1.0 - hsv.value) * 100.0);
    }
    if focus.get() != Some(ui.hex)
        && let Ok(mut input) = inputs.get_mut(ui.hex)
    {
        input.editor_mut().set_text(&format!(
            "#{:02X}{:02X}{:02X}",
            (rgba[0] * 255.0).round() as u8,
            (rgba[1] * 255.0).round() as u8,
            (rgba[2] * 255.0).round() as u8
        ));
    }
    if let Some(doc) = doc {
        let active = &doc.document.layers[doc.active];
        if focus.get() != Some(ui.name)
            && let Ok(mut input) = inputs.get_mut(ui.name)
        {
            input.editor_mut().set_text(&active.name);
        }
        for (entity, slider, value, range) in &sliders {
            let (number, label) = match slider.binding {
                Binding::Size => (viewer.edit.size, format!("{:.0} px", viewer.edit.size)),
                Binding::Hardness => {
                    let hardness = if viewer.edit.tool == PhotoTool::Pencil {
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
                Binding::LayerOpacity => {
                    (active.opacity, format!("{:.0}%", active.opacity * 100.0))
                }
                Binding::Hue => (hsv.hue, format!("{:.0}°", hsv.hue)),
            };
            if value.0 != number {
                commands.entity(entity).try_insert(SliderValue(number));
            }
            if matches!(slider.binding, Binding::Hardness) {
                if viewer.edit.tool == PhotoTool::Pencil {
                    commands.entity(entity).try_insert(InteractionDisabled);
                } else {
                    commands.entity(entity).try_remove::<InteractionDisabled>();
                }
            }
            if let Ok(mut label_node) = labels.get_mut(slider.readout) {
                label_node.set_if_neq(Text(label));
            }
            if let Ok(mut thumb) = nodes.get_mut(slider.thumb) {
                thumb.left = percent(((number - range.start()) / range.span()) * 94.0);
            }
        }
        let signature = format!(
            "{}:{:?}",
            doc.active,
            doc.document
                .layers
                .iter()
                .map(|l| (&l.name, l.visible))
                .collect::<Vec<_>>()
        );
        if ui.layer_signature != signature {
            commands.entity(ui.layers).despawn_related::<Children>();
            for (index, layer) in doc.document.layers.iter().enumerate().rev() {
                let row_node = surface(
                    &mut commands,
                    ui.layers,
                    Node {
                        width: percent(100),
                        height: px(32),
                        min_height: px(32),
                        border_radius: BorderRadius::all(px(5)),
                        ..row(3.0)
                    },
                    if index == doc.active { ACTIVE } else { WELL },
                );
                icons::button(
                    &mut commands,
                    row_node,
                    if layer.visible {
                        icons::Glyph::Eye
                    } else {
                        icons::Glyph::EyeOff
                    },
                    "Видимость слоя",
                    Action::ImageEdit(EditAction::Visible(index)),
                    Look::Regular,
                );
                let select = button(
                    &mut commands,
                    row_node,
                    &interface.font,
                    &layer.name,
                    Action::ImageEdit(EditAction::SelectLayer(index)),
                    Look::Regular,
                );
                commands.entity(select).entry::<Node>().and_modify(|mut n| {
                    n.flex_grow = 1.0;
                    n.min_width = px(0);
                    n.overflow = Overflow::clip();
                    n.justify_content = JustifyContent::Start;
                });
            }
            ui.layer_signature = signature;
        }
    }
    let ring_shown = editing
        && matches!(
            viewer.edit.tool,
            PhotoTool::Brush | PhotoTool::Pencil | PhotoTool::Eraser
        )
        && !viewer.edit.menu
        && !viewer.edit.paint_menu
        && editor.pending.is_none()
        && canvas.single().is_ok_and(|(n, t)| {
            window
                .cursor_position()
                .is_some_and(|p| n.contains_point(*t, p / n.inverse_scale_factor()))
        });
    if let Ok(mut ring) = nodes.get_mut(ui.ring) {
        ring.border_radius = if viewer.edit.tool == PhotoTool::Pencil {
            BorderRadius::ZERO
        } else {
            BorderRadius::MAX
        };
        ring.display = if ring_shown {
            Display::Flex
        } else {
            Display::None
        };
        if ring_shown
            && let Ok((node, transform)) = canvas.single()
            && let Some(cursor) = window.cursor_position()
            && let Some(dimensions) = viewer.dimensions
        {
            let viewport = node.size() * node.inverse_scale_factor();
            let source = viewer.source_size(dimensions);
            let size =
                viewer.edit.size * viewer.zoom.scale(source, viewport) / window.scale_factor();
            ring.width = px(size.max(3.0));
            ring.height = px(size.max(3.0));
            let min =
                (transform.affine().translation - node.size() * 0.5) * node.inverse_scale_factor();
            ring.left = px(cursor.x - min.x);
            ring.top = px(cursor.y - min.y);
        }
    }
}
