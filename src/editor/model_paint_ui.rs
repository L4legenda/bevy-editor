use super::*;
use bevy::{
    asset::RenderAssetUsages,
    input_focus::InputFocus,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    text::EditableText,
    ui_widgets::{SelectAllOnFocus, TextInput},
};

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct PaintButton(PaintAction);
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct Value(Parameter);
#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct PaintUi {
    roots: Vec<Entity>,
    original: Vec<Entity>,
    hidden: Vec<(Entity, Display)>,
    canvas: Entity,
    uv: Entity,
    uv_image: Handle<Image>,
    uv_info: Entity,
    content: Entity,
    stack: Entity,
    shelf: Entity,
    layer_name: Entity,
    brush_name: Entity,
    three_title: Entity,
    stamp: Option<(usize, u64, u64)>,
    uv_stamp: Option<(usize, usize, usize, u64)>,
    uv_job: Option<Task<((usize, usize, usize, u64), image::RgbaImage, bool)>>,
    swatches: Vec<Handle<Image>>,
    showing: bool,
}
#[cfg(test)]
impl PaintUi {
    pub(super) fn retained_labels(&self) -> [Entity; 2] {
        [self.layer_name, self.brush_name]
    }
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: PaintAction,
) -> Entity {
    let e = super::super::super::button(
        commands,
        parent,
        font,
        label,
        Action::ModelView(ModelViewAction::Paint(action)),
        Look::Regular,
    );
    commands.entity(e).insert(PaintButton(action));
    e
}
fn icon(
    commands: &mut Commands,
    parent: Entity,
    glyph: icons::Glyph,
    label: &str,
    action: PaintAction,
) -> Entity {
    let e = icons::button(
        commands,
        parent,
        glyph,
        label,
        Action::ModelView(ModelViewAction::Paint(action)),
        Look::Regular,
    );
    commands.entity(e).insert(PaintButton(action));
    e
}
fn title(commands: &mut Commands, parent: Entity, font: &Handle<Font>, value: &str) -> Entity {
    let e = surface(
        commands,
        parent,
        Node {
            width: percent(100),
            height: px(30),
            flex_shrink: 0.,
            padding: UiRect::horizontal(px(10)),
            border: UiRect::bottom(px(1)),
            ..row(6.)
        },
        CHROME,
    );
    text(commands, e, font, value, 12., FOCUS);
    e
}
fn number(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    parameter: Parameter,
    compact: bool,
) {
    let line = node(
        commands,
        parent,
        Node {
            width: if compact { Val::Auto } else { percent(100) },
            ..row(5.)
        },
    );
    let label = text(commands, line, font, parameter.label(), 12., MUTED);
    if !compact {
        commands.entity(label).insert(Node {
            flex_grow: 1.,
            ..default()
        });
    }
    let (min, max) = parameter.bounds();
    commands.spawn((
        TextInput,
        Value(parameter),
        EditableText::new(""),
        SelectAllOnFocus,
        crate::ui_theme::text_cursor_style(),
        Number::decimal(min as f32, max as f32, 1., 0).finish(),
        TabIndex(0),
        AccessibleLabel(format!("Рисование: {}", parameter.label())),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(12.),
            ..default()
        },
        TextColor(TEXT),
        Node {
            width: px(48),
            height: px(27),
            padding: UiRect::axes(px(7), px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..default()
        },
        BackgroundColor(WELL),
        BorderColor::all(EDGE),
        ChildOf(line),
    ));
}
fn sphere(color: [u8; 3], roughness: f32, metal: f32) -> Image {
    let mut image = image::RgbaImage::new(72, 72);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let nx = (x as f32 - 35.5) / 32.;
        let ny = (y as f32 - 35.5) / 32.;
        let r = nx * nx + ny * ny;
        if r > 1. {
            continue;
        }
        let z = (1. - r).sqrt();
        let light = (nx * -0.4 + ny * -0.55 + z * 0.73).max(0.);
        let gloss = light.powf(8. + (100. - roughness) * 0.6) * (0.4 + metal * 0.005);
        let shade = 0.18 + light * 0.78;
        *pixel = image::Rgba([
            (color[0] as f32 * shade + gloss * 100.).min(255.) as u8,
            (color[1] as f32 * shade + gloss * 100.).min(255.) as u8,
            (color[2] as f32 * shade + gloss * 100.).min(255.) as u8,
            255,
        ]);
    }
    Image::from_dynamic(
        image::DynamicImage::ImageRgba8(image),
        true,
        RenderAssetUsages::default(),
    )
}

pub(in crate::editor::ui::model_viewer) fn setup(
    mut commands: Commands,
    interface: Res<Interface>,
    viewer: Res<ViewerUi>,
    canvas: Single<Entity, With<Canvas>>,
    nodes: Query<(Entity, &Node, &ChildOf)>,
    mut images: ResMut<Assets<Image>>,
) {
    let font = &interface.font;
    commands.spawn((
        engine::BrushCursor,
        Node {
            position_type: PositionType::Absolute,
            width: px(70),
            height: px(70),
            border: UiRect::all(px(1.5)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BorderColor::all(Color::srgba(0.98, 0.88, 0.66, 0.95)),
        Visibility::Hidden,
        ZIndex(90),
        Pickable::IGNORE,
        ChildOf(viewer.overlay),
    ));
    let original = nodes
        .iter()
        .filter(|(_, node, parent)| {
            parent.parent() == interface.side && node.position_type == PositionType::Relative
        })
        .map(|(entity, _, _)| entity)
        .collect();
    let toolbar = surface(
        &mut commands,
        viewer.overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(45),
            left: px(0),
            right: px(0),
            height: px(43),
            padding: UiRect::horizontal(px(10)),
            border: UiRect::vertical(px(1)),
            ..row(12.)
        },
        PANEL,
    );
    commands.entity(toolbar).insert(ZIndex(40));
    for parameter in [
        Parameter::Size,
        Parameter::Flow,
        Parameter::Opacity,
        Parameter::Hardness,
    ] {
        number(&mut commands, toolbar, font, parameter, true);
    }
    button(
        &mut commands,
        toolbar,
        font,
        "Сбросить вид",
        PaintAction::Reset,
    );
    super::super::super::button(
        &mut commands,
        toolbar,
        font,
        "Сохранить",
        Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)),
        Look::Primary,
    );
    let views = node(
        &mut commands,
        viewer.overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(185),
            right: px(10),
            top: px(10),
            justify_content: JustifyContent::End,
            ..row(3.)
        },
    );
    commands.entity(views).insert(ZIndex(40));
    for (name, view) in [
        ("3D + 2D", View::Split),
        ("3D", View::Three),
        ("2D", View::Two),
    ] {
        button(&mut commands, views, font, name, PaintAction::View(view));
    }
    let tool_rail = surface(
        &mut commands,
        viewer.overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(88),
            bottom: px(0),
            width: px(44),
            padding: UiRect::axes(px(6), px(9)),
            border: UiRect::right(px(1)),
            ..column(7.)
        },
        CHROME,
    );
    commands.entity(tool_rail).insert(ZIndex(35));
    for (tool, glyph) in [
        (PaintTool::Brush, icons::Glyph::Brush),
        (PaintTool::Eraser, icons::Glyph::Eraser),
        (PaintTool::Fill, icons::Glyph::PaintBucket),
        (PaintTool::Picker, icons::Glyph::Pipette),
        (PaintTool::Path, icons::Glyph::Pencil),
    ] {
        icon(
            &mut commands,
            tool_rail,
            glyph,
            tool.label(),
            PaintAction::Tool(tool),
        );
    }
    let three_title = surface(
        &mut commands,
        viewer.overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(44),
            top: px(88),
            height: px(30),
            width: percent(48),
            padding: UiRect::horizontal(px(10)),
            ..row(8.)
        },
        CHROME,
    );
    commands
        .entity(three_title)
        .insert((ZIndex(25), Pickable::IGNORE));
    icons::themed(&mut commands, three_title, icons::Glyph::Box, 15.);
    text(&mut commands, three_title, font, "3D-вид", 12., TEXT);
    let uv = surface(
        &mut commands,
        viewer.content,
        Node {
            display: Display::None,
            width: percent(49),
            height: percent(100),
            min_width: px(0),
            flex_shrink: 0.,
            border: UiRect::left(px(1)),
            ..column(0.)
        },
        WELL,
    );
    commands
        .entity(uv)
        .insert(AccessibleLabel("Рисование: 2D-вид UV".into()));
    title(&mut commands, uv, font, "2D-вид · UV");
    let uv_info = text(&mut commands, uv, font, "UV · 0–1", 11., MUTED);
    commands.entity(uv_info).insert(TextLayout::default());
    super::super::super::button(
        &mut commands,
        uv,
        font,
        "Открыть UV-развёртку",
        Action::ModelView(ModelViewAction::Edit(editing::EditAction::Mode(Mode::Uv))),
        Look::Regular,
    );
    let uv_view = node(
        &mut commands,
        uv,
        Node {
            width: percent(100),
            min_height: px(0),
            min_width: px(0),
            flex_grow: 1.,
            padding: UiRect::all(px(14)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            overflow: Overflow::clip(),
            ..default()
        },
    );
    let uv_image = images.add(Image::from_dynamic(
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            512,
            512,
            image::Rgba([35, 36, 44, 255]),
        )),
        true,
        RenderAssetUsages::default(),
    ));
    commands.spawn((
        ImageNode::new(uv_image.clone()),
        engine::TextureCanvas,
        Node {
            width: percent(100),
            max_height: percent(100),
            aspect_ratio: Some(1.),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(uv_view),
    ));
    let shelf_root = surface(
        &mut commands,
        viewer.overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(44),
            right: px(0),
            bottom: px(0),
            height: px(198),
            border: UiRect::top(px(1)),
            ..column(0.)
        },
        PANEL,
    );
    commands.entity(shelf_root).insert((
        ZIndex(35),
        AccessibleLabel("Рисование: библиотека ресурсов".into()),
    ));
    title(&mut commands, shelf_root, font, "Библиотека");
    let shelf_body = node(
        &mut commands,
        shelf_root,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            ..row(0.)
        },
    );
    let categories = surface(
        &mut commands,
        shelf_body,
        Node {
            width: px(141),
            height: percent(100),
            padding: UiRect::all(px(5)),
            border: UiRect::right(px(1)),
            flex_shrink: 0.,
            ..column(1.)
        },
        CHROME,
    );
    for (name, category) in [
        ("Материалы", Category::Materials),
        ("Умные материалы", Category::Smart),
        ("Кисти", Category::Brushes),
        ("Текстуры", Category::Textures),
        ("Генераторы", Category::Generators),
        ("Графы", Category::Graphs),
    ] {
        let e = button(
            &mut commands,
            categories,
            font,
            name,
            PaintAction::Category(category),
        );
        commands.entity(e).entry::<Node>().and_modify(|mut n| {
            n.width = percent(100);
            n.height = px(23);
        });
    }
    let shelf = node(
        &mut commands,
        shelf_body,
        Node {
            height: percent(100),
            flex_grow: 1.,
            min_width: px(0),
            overflow: Overflow::scroll_x(),
            padding: UiRect::all(px(8)),
            ..row(7.)
        },
    );
    commands.entity(shelf).insert(HorizontalScroller);
    let sidebar = surface(
        &mut commands,
        interface.side,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(6),
            right: px(0),
            top: px(0),
            bottom: px(0),
            ..column(0.)
        },
        PANEL,
    );
    commands.entity(sidebar).insert((
        ZIndex(25),
        AccessibleLabel("Рисование: слои и свойства".into()),
    ));
    let upper = node(
        &mut commands,
        sidebar,
        Node {
            width: percent(100),
            height: percent(43),
            min_height: px(160),
            flex_shrink: 0.,
            ..column(0.)
        },
    );
    let tabs = surface(
        &mut commands,
        upper,
        Node {
            width: percent(100),
            min_height: px(34),
            padding: UiRect::all(px(3)),
            flex_wrap: FlexWrap::Wrap,
            ..row(1.)
        },
        CHROME,
    );
    for (name, panel) in [
        ("Слои", Panel::Layers),
        ("Карты", Panel::Maps),
        ("Наборы", Panel::Sets),
        ("Просмотр", Panel::Preview),
    ] {
        let e = button(&mut commands, tabs, font, name, PaintAction::Panel(panel));
        commands.entity(e).entry::<Node>().and_modify(|mut n| {
            n.padding = UiRect::horizontal(px(5));
        });
    }
    let stack = node(
        &mut commands,
        upper,
        Node {
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.,
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(7)),
            ..column(5.)
        },
    );
    commands.entity(stack).insert(Scroller);
    let properties = surface(
        &mut commands,
        sidebar,
        Node {
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.,
            border: UiRect::top(px(1)),
            ..column(0.)
        },
        CHROME,
    );
    title(&mut commands, properties, font, "Свойства");
    let body = node(
        &mut commands,
        properties,
        Node {
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.,
            padding: UiRect::all(px(11)),
            overflow: Overflow::scroll_y(),
            ..column(9.)
        },
    );
    commands.entity(body).insert(Scroller);
    let layer_name = text(&mut commands, body, font, "Слой 1", 14., TEXT);
    text(&mut commands, body, font, "СЛОЙ", 11., MUTED);
    number(&mut commands, body, font, Parameter::LayerOpacity, false);
    text(&mut commands, body, font, "КИСТЬ", 11., MUTED);
    let brush_name = text(&mut commands, body, font, "Кисть · круглая", 13., TEXT);
    for parameter in [
        Parameter::Size,
        Parameter::Hardness,
        Parameter::Flow,
        Parameter::Opacity,
        Parameter::Spacing,
    ] {
        number(&mut commands, body, font, parameter, false);
    }
    text(&mut commands, body, font, "МАТЕРИАЛ", 11., MUTED);
    let palette = node(
        &mut commands,
        body,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(4.)
        },
    );
    for (i, rgb) in COLORS.iter().enumerate() {
        let e = super::super::super::control(
            &mut commands,
            palette,
            Action::ModelView(ModelViewAction::Paint(PaintAction::Color(i))),
            Look::Regular,
            Node {
                width: px(24),
                height: px(24),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
        );
        commands.entity(e).insert((
            PaintButton(PaintAction::Color(i)),
            AccessibleLabel(format!("Цвет кисти {}", i + 1)),
        ));
        commands.spawn((
            Node {
                width: percent(100),
                height: percent(100),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(Color::srgb_u8(rgb[0], rgb[1], rgb[2])),
            Pickable::IGNORE,
            ChildOf(e),
        ));
    }
    for parameter in [Parameter::Roughness, Parameter::Metallic] {
        number(&mut commands, body, font, parameter, false);
    }
    text(&mut commands, body, font, "КАНАЛ", 11., MUTED);
    let channel = node(
        &mut commands,
        body,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            ..row(3.)
        },
    );
    for (i, name) in CHANNELS.iter().enumerate() {
        button(&mut commands, channel, font, name, PaintAction::Channel(i));
    }
    let swatches = PRESETS
        .iter()
        .map(|preset| images.add(sphere(preset.1, preset.2, preset.3)))
        .collect();
    commands.insert_resource(PaintUi {
        roots: vec![toolbar, views, tool_rail, three_title, shelf_root, sidebar],
        original,
        hidden: vec![],
        canvas: *canvas,
        uv,
        uv_image,
        uv_info,
        content: viewer.content,
        stack,
        shelf,
        layer_name,
        brush_name,
        three_title,
        stamp: None,
        uv_stamp: None,
        uv_job: None,
        swatches,
        showing: false,
    });
}

fn raster(
    mesh: &crate::editor::model_document::MeshData,
    material: usize,
) -> (image::RgbaImage, bool) {
    let mut image = image::RgbaImage::from_fn(512, 512, |x, y| {
        let value = if (x / 32 + y / 32) % 2 == 0 { 45 } else { 54 };
        image::Rgba([value, value, value + 7, 255])
    });
    let mut has_uv = false;
    for face in mesh.faces.iter().filter(|face| face.material == material) {
        let area = face
            .corners
            .iter()
            .zip(face.corners.iter().cycle().skip(1))
            .map(|(a, b)| a.uv.perp_dot(b.uv))
            .sum::<f32>()
            .abs();
        has_uv |= area > 1e-8;
        for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
            if !a.uv.is_finite() || !b.uv.is_finite() {
                continue;
            }
            let a = Vec2::new(a.uv.x, 1. - a.uv.y).clamp(Vec2::ZERO, Vec2::ONE) * 507.
                + Vec2::splat(2.);
            let b = Vec2::new(b.uv.x, 1. - b.uv.y).clamp(Vec2::ZERO, Vec2::ONE) * 507.
                + Vec2::splat(2.);
            let steps = (b - a).abs().max_element().ceil().max(1.) as usize;
            for i in 0..=steps {
                let p = a.lerp(b, i as f32 / steps as f32).round();
                image.put_pixel(p.x as u32, p.y as u32, image::Rgba([225, 184, 119, 255]));
            }
        }
    }
    (image, has_uv)
}

pub(in crate::editor::ui::model_viewer) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    mut paint: ResMut<PaintState>,
    mut ui: ResMut<PaintUi>,
    interface: Res<Interface>,
    viewer: Res<ViewerUi>,
    focus: Res<InputFocus>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut fields: Query<(Entity, &Value, &mut EditableText, &mut BorderColor)>,
    buttons: Query<
        (
            Entity,
            &PaintButton,
            &BackgroundColor,
            &BorderColor,
            Option<&bevy::ui::InteractionDisabled>,
        ),
        Without<Value>,
    >,
    mut images: ResMut<Assets<Image>>,
    runtime: Res<engine::Runtime>,
    mut texture_nodes: Query<&mut ImageNode, With<engine::TextureCanvas>>,
) {
    let doc = editor
        .viewing_model
        .and_then(|index| editing.documents.get(&index));
    let showing = doc.is_some_and(|doc| doc.mode == Mode::Paint)
        && editor.viewing_image.is_none()
        && editor.pending.is_none();
    let view = editor
        .viewing_model
        .and_then(|i| paint.sessions.get(&i))
        .map_or(View::Split, |s| s.view);
    if showing != ui.showing {
        if showing {
            for entity in ui.original.clone() {
                if let Ok(mut node) = nodes.get_mut(entity) {
                    ui.hidden.push((entity, node.display));
                    node.display = Display::None;
                }
            }
        } else {
            for (entity, display) in ui.hidden.drain(..) {
                if let Ok(mut node) = nodes.get_mut(entity) {
                    node.display = display;
                }
            }
        }
        ui.showing = showing;
    }
    if showing {
        for &entity in &ui.original {
            if let Ok(mut node) = nodes.get_mut(entity) {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
    for &entity in &ui.roots {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if showing && (entity != ui.three_title || view != View::Two) {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.content) {
        let padding = if showing {
            UiRect {
                left: px(44),
                top: px(118),
                bottom: px(198),
                right: px(0),
            }
        } else {
            UiRect::ZERO
        };
        if node.padding != padding {
            node.padding = padding;
        }
    }
    if let Ok(mut node) = nodes.get_mut(viewer.toolbar) {
        let right = if showing {
            px(180)
        } else if !doc.is_some_and(|d| d.mode == Mode::Uv) {
            px(140)
        } else {
            node.right
        };
        if node.right != right {
            node.right = right;
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.canvas) {
        let display = if showing && view == View::Two {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.uv) {
        let display = if showing && view != View::Three {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    if !showing {
        ui.stamp = None;
        return;
    }
    let index = editor.viewing_model.unwrap();
    let doc = doc.unwrap();
    let session = paint.sessions.entry(index).or_default();
    if let Ok(mut node) = nodes.get_mut(ui.uv) {
        let width = percent(if session.view == View::Two { 100. } else { 49. });
        if node.width != width {
            node.width = width;
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.three_title) {
        let display = if session.view == View::Two {
            Display::None
        } else {
            Display::Flex
        };
        let width = percent(if session.view == View::Three {
            96.
        } else {
            48.
        });
        if node.display != display {
            node.display = display;
        }
        if node.width != width {
            node.width = width;
        }
    }
    for (entity, parameter, mut input, mut border) in &mut fields {
        if focus.get() == Some(entity) {
            let parsed = input.value().to_string().replace(',', ".").parse::<f32>();
            let (min, max) = parameter.0.bounds();
            match parsed {
                Ok(value) if value.is_finite() && (min as f32..=max as f32).contains(&value) => {
                    if session.value(parameter.0) != value {
                        session.set_value(parameter.0, value);
                    }
                    border.set_if_neq(BorderColor::all(FOCUS));
                }
                _ => {
                    border.set_if_neq(BorderColor::all(ERROR));
                }
            }
        } else {
            let value = format!("{:.0}", session.value(parameter.0));
            if input.value().to_string() != value {
                input.editor_mut().set_text(&value);
            }
            border.set_if_neq(BorderColor::all(EDGE));
        }
    }
    for (entity, button, background, border, disabled) in &buttons {
        if session.available(button.0) && disabled.is_some() {
            commands
                .entity(entity)
                .try_remove::<bevy::ui::InteractionDisabled>();
        } else if !session.available(button.0) && disabled.is_none() {
            commands
                .entity(entity)
                .try_insert(bevy::ui::InteractionDisabled);
        }
        let active = session.active(button.0);
        let next_background = BackgroundColor(if active { ACTIVE } else { Color::NONE });
        let next_border = BorderColor::all(if active {
            FOCUS.with_alpha(0.7)
        } else {
            EDGE.with_alpha(0.3)
        });
        if *background != next_background || *border != next_border {
            commands
                .entity(entity)
                .try_insert((next_background, next_border));
        }
    }
    if let Ok(mut text) = labels.get_mut(ui.layer_name) {
        let value = session
            .stack
            .layers
            .get(session.stack.selected)
            .map_or_else(|| "Выберите слой".into(), |layer| layer.name.clone());
        if text.0 != value {
            text.0 = value;
        }
    }
    if let Ok(mut text) = labels.get_mut(ui.brush_name) {
        let value = format!("{} · круглая", session.tool.label());
        if text.0 != value {
            text.0 = value;
        }
    }
    let stamp = (index, session.revision, doc.revision);
    if ui.stamp != Some(stamp) {
        commands.entity(ui.stack).despawn_related::<Children>();
        let font = &interface.font;
        match session.panel {
            Panel::Layers => {
                let tools = node(
                    &mut commands,
                    ui.stack,
                    Node {
                        flex_wrap: FlexWrap::Wrap,
                        ..row(3.)
                    },
                );
                for (kind, glyph, name) in [
                    (
                        LayerKind::Paint,
                        icons::Glyph::Brush,
                        "Добавить слой краски",
                    ),
                    (
                        LayerKind::Fill,
                        icons::Glyph::PaintBucket,
                        "Добавить заливку",
                    ),
                    (LayerKind::Folder, icons::Glyph::Folder, "Добавить группу"),
                ] {
                    icon(&mut commands, tools, glyph, name, PaintAction::Add(kind));
                }
                icon(
                    &mut commands,
                    tools,
                    icons::Glyph::Copy,
                    "Дублировать слой",
                    PaintAction::Duplicate,
                );
                icon(
                    &mut commands,
                    tools,
                    icons::Glyph::Trash,
                    "Удалить слой",
                    PaintAction::Remove,
                );
                icon(
                    &mut commands,
                    tools,
                    icons::Glyph::Undo,
                    "Отменить изменение слоёв",
                    PaintAction::Undo,
                );
                icon(
                    &mut commands,
                    tools,
                    icons::Glyph::Redo,
                    "Повторить изменение слоёв",
                    PaintAction::Redo,
                );
                for (i, layer) in session.stack.layers.iter().enumerate().rev() {
                    let line = node(
                        &mut commands,
                        ui.stack,
                        Node {
                            width: percent(100),
                            ..row(3.)
                        },
                    );
                    icon(
                        &mut commands,
                        line,
                        if layer.visible {
                            icons::Glyph::Eye
                        } else {
                            icons::Glyph::EyeOff
                        },
                        "Видимость слоя",
                        PaintAction::Visible(i),
                    );
                    let kind = match layer.kind {
                        LayerKind::Paint => "Краска",
                        LayerKind::Fill => "Заливка",
                        LayerKind::Folder => "Группа",
                    };
                    let e = button(
                        &mut commands,
                        line,
                        font,
                        &format!(
                            "{}\n{kind} · Нормальный · {:.0}%",
                            layer.name, layer.opacity
                        ),
                        PaintAction::Select(i),
                    );
                    commands.entity(e).entry::<Node>().and_modify(|mut n| {
                        n.height = px(43);
                        n.flex_grow = 1.;
                        n.min_width = px(0);
                    });
                }
                let move_row = node(&mut commands, ui.stack, row(4.));
                button(&mut commands, move_row, font, "Выше", PaintAction::Move(1));
                button(&mut commands, move_row, font, "Ниже", PaintAction::Move(-1));
            }
            Panel::Sets => {
                text(&mut commands, ui.stack, font, "Наборы текстур", 13., TEXT);
                let mut i = 0;
                for object in &doc.objects {
                    for slot in 0..object.materials.len().max(1) {
                        let material = object
                            .materials
                            .get(slot)
                            .and_then(|m| m.source.get("name"))
                            .and_then(|name| name.as_str())
                            .unwrap_or("Материал");
                        let e = button(
                            &mut commands,
                            ui.stack,
                            font,
                            &format!("{} · {material}", object.name),
                            PaintAction::Set(i),
                        );
                        commands
                            .entity(e)
                            .entry::<Node>()
                            .and_modify(|mut n| n.width = percent(100));
                        i += 1;
                    }
                }
            }
            Panel::Maps => {
                text(&mut commands, ui.stack, font, "Карты меша", 14., TEXT);
                for name in [
                    "Нормали",
                    "Окклюзия",
                    "Кривизна",
                    "Положение",
                    "Толщина",
                    "ID материала",
                    "Высота",
                ] {
                    let line = node(
                        &mut commands,
                        ui.stack,
                        Node {
                            width: percent(100),
                            height: px(29),
                            ..row(8.)
                        },
                    );
                    text(&mut commands, line, font, name, 12., TEXT);
                    text(&mut commands, line, font, "—", 12., MUTED);
                }
            }
            Panel::Preview => {
                text(
                    &mut commands,
                    ui.stack,
                    font,
                    "Предпросмотр материала",
                    14.,
                    TEXT,
                );
                text(
                    &mut commands,
                    ui.stack,
                    font,
                    "Освещение и окружение сцены",
                    12.,
                    MUTED,
                );
                for (name, view) in [
                    ("3D и UV рядом", View::Split),
                    ("Только 3D", View::Three),
                    ("Только UV", View::Two),
                ] {
                    button(&mut commands, ui.stack, font, name, PaintAction::View(view));
                }
                text(&mut commands, ui.stack, font, "Каналы", 12., MUTED);
                for (i, name) in CHANNELS.iter().enumerate() {
                    button(&mut commands, ui.stack, font, name, PaintAction::Channel(i));
                }
            }
        }
        commands.entity(ui.shelf).despawn_related::<Children>();
        match session.category {
            Category::Materials => {
                for (i, preset) in PRESETS.iter().enumerate() {
                    let card = button(&mut commands, ui.shelf, font, "", PaintAction::Preset(i));
                    commands.entity(card).insert(Node {
                        width: px(104),
                        height: px(138),
                        flex_shrink: 0.,
                        padding: UiRect::all(px(5)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(8)),
                        align_items: AlignItems::Center,
                        ..column(4.)
                    });
                    commands.spawn((
                        ImageNode::new(ui.swatches[i].clone()),
                        Node {
                            width: px(84),
                            height: px(84),
                            ..default()
                        },
                        Pickable::IGNORE,
                        ChildOf(card),
                    ));
                    text(&mut commands, card, font, preset.0, 11., TEXT);
                }
            }
            Category::Brushes => {
                for (tool, glyph) in [
                    (PaintTool::Brush, icons::Glyph::Brush),
                    (PaintTool::Eraser, icons::Glyph::Eraser),
                    (PaintTool::Fill, icons::Glyph::PaintBucket),
                    (PaintTool::Picker, icons::Glyph::Pipette),
                    (PaintTool::Path, icons::Glyph::Pencil),
                ] {
                    let card = button(
                        &mut commands,
                        ui.shelf,
                        font,
                        tool.label(),
                        PaintAction::Tool(tool),
                    );
                    commands.entity(card).entry::<Node>().and_modify(|mut n| {
                        n.height = px(100);
                        n.width = px(104);
                        n.flex_direction = FlexDirection::Column;
                    });
                    icons::themed(&mut commands, card, glyph, 28.);
                }
            }
            _ => {
                text(
                    &mut commands,
                    ui.shelf,
                    font,
                    "В этой категории нет ресурсов",
                    13.,
                    MUTED,
                );
            }
        }
        ui.stamp = Some(stamp);
    }
    let selected = doc
        .objects
        .iter()
        .flat_map(|object| (0..object.materials.len().max(1)).map(move |slot| (object, slot)))
        .nth(session.set)
        .or_else(|| doc.active().map(|object| (object, 0)));
    if let Some((object, slot)) = selected {
        if let Ok(mut image) = texture_nodes.single_mut() {
            let handle = runtime
                .preview(index, object.id, slot, session.channel)
                .unwrap_or_else(|| ui.uv_image.clone());
            if image.image != handle {
                image.image = handle;
            }
        }
        let stamp = (index, object.id, slot, doc.revision);
        if ui.uv_stamp != Some(stamp) {
            let mesh = object.geometry.clone();
            ui.uv_stamp = Some(stamp);
            ui.uv_job = Some(AsyncComputeTaskPool::get().spawn(async move {
                let (image, has_uv) = raster(&mesh, slot);
                (stamp, image, has_uv)
            }));
        }
    }
    if let Some((stamp, image, has_uv)) =
        ui.uv_job.as_mut().and_then(|job| block_on(poll_once(job)))
    {
        ui.uv_job = None;
        if ui.uv_stamp == Some(stamp) {
            if let Ok(mut info) = labels.get_mut(ui.uv_info) {
                info.0 = if has_uv {
                    format!(
                        "{} × {} · UV 0–1",
                        session.texture_size, session.texture_size
                    )
                } else {
                    "UV-развёртка отсутствует".into()
                };
            }
            if let Some(mut target) = images.get_mut(&ui.uv_image) {
                *target = Image::from_dynamic(
                    image::DynamicImage::ImageRgba8(image),
                    true,
                    RenderAssetUsages::default(),
                );
            }
        }
    }
}
