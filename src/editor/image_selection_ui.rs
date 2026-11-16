//! Native tool menus, transform handles and a cached GPU selection outline.
use super::selection::{Mode, SelectionAction as Op, TransformMode};
use super::*;
use crate::editor::photo_document::SelectionTool;
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use std::hash::{Hash, Hasher};

#[derive(Resource)]
struct ToolsUi {
    button: Entity,
    icon: Entity,
    selected_tool: SelectionTool,
    menu: Entity,
    transform_button: Entity,
    transform_menu: Entity,
    options: Entity,
    selection_options: Entity,
    intersection: Entity,
    transform_options: Entity,
    mode_label: Entity,
    tolerance: Entity,
    quick_size: Entity,
    advanced: Entity,
    wand_options: Entity,
    quick_options: Entity,
    overlay: Entity,
    lines: Vec<Entity>,
    handles: [Entity; 9],
    mask_key: Option<(usize, u64)>,
    mask_hash: Option<u64>,
    mask_source: Option<Surface>,
    mask_image: Option<Handle<Image>>,
}
pub(in crate::editor) struct ImageSelectionPlugin;
impl Plugin for ImageSelectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, popup_input.before(crate::editor::ui::input))
            .add_systems(
                Update,
                refresh
                    .after(super::interface::refresh)
                    .after(super::super::refresh),
            );
    }
}
fn glyph(tool: SelectionTool) -> icons::Glyph {
    match tool {
        SelectionTool::Ellipse => icons::Glyph::SelectionEllipse,
        SelectionTool::Lasso => icons::Glyph::Lasso,
        SelectionTool::Polygon => icons::Glyph::Polygon,
        SelectionTool::Wand | SelectionTool::Object => icons::Glyph::Wand,
        SelectionTool::Quick => icons::Glyph::Brush,
        _ => icons::Glyph::Selection,
    }
}
fn item(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    op: Op,
) -> Entity {
    let entity = button(
        commands,
        parent,
        font,
        label,
        Action::ImageEdit(EditAction::Selection(op)),
        Look::Regular,
    );
    commands
        .entity(entity)
        .insert(AccessibleLabel(label.into()));
    entity
}
fn menu(commands: &mut Commands, root: Entity) -> Entity {
    let entity = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: px(240),
            padding: UiRect::all(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(entity).insert(GlobalZIndex(60));
    entity
}
pub fn setup(
    commands: &mut Commands,
    tools: Entity,
    toolbar: Entity,
    canvas: Entity,
    root: Entity,
    font: &Handle<Font>,
) {
    super::interface::tool_button(
        commands,
        tools,
        icons::Glyph::Move,
        "Перемещение · V",
        PhotoTool::Move,
    );
    let group = node(
        commands,
        tools,
        Node {
            width: px(36),
            height: px(36),
            flex_shrink: 0.0,
            ..default()
        },
    );
    let choose = control(
        commands,
        group,
        Action::ImageEdit(EditAction::Selection(Op::UseTool)),
        Look::Tool,
        super::interface::tool_node(),
    );
    commands.entity(choose).insert(AccessibleLabel(
        "Выделение · M / L / W · ПКМ — инструменты выделения".into(),
    ));
    let icon = icons::themed(commands, choose, icons::Glyph::Selection, 18.0);
    let arrow = control(
        commands,
        group,
        Action::ImageEdit(EditAction::Selection(Op::Menu)),
        Look::Tool,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(0),
            right: px(0),
            width: px(14),
            height: px(14),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    );
    commands
        .entity(arrow)
        .insert((ZIndex(1), AccessibleLabel("Варианты выделения".into())));
    icons::themed(commands, arrow, icons::Glyph::SubmenuDown, 12.0);
    let transform = control(
        commands,
        tools,
        Action::ImageEdit(EditAction::Selection(Op::Transform)),
        Look::Tool,
        super::interface::tool_node(),
    );
    commands
        .entity(transform)
        .insert(AccessibleLabel("Трансформация · Ctrl+T".into()));
    icons::themed(commands, transform, icons::Glyph::Scale, 18.0);
    let selection_menu = menu(commands, root);
    for (label, tool) in [
        ("Прямоугольное выделение · M", SelectionTool::Rectangle),
        ("Эллиптическое выделение", SelectionTool::Ellipse),
        ("Лассо · L", SelectionTool::Lasso),
        ("Многоугольное лассо", SelectionTool::Polygon),
        ("Волшебная палочка · W", SelectionTool::Wand),
        ("Быстрое выделение", SelectionTool::Quick),
        ("Выделение объекта", SelectionTool::Object),
        ("Одна строка", SelectionTool::Row),
        ("Один столбец", SelectionTool::Column),
    ] {
        let e = item(commands, selection_menu, font, label, Op::Tool(tool));
        commands.entity(e).entry::<Node>().and_modify(|mut n| {
            n.width = percent(100);
            n.height = px(31);
        });
        let icon = icons::themed(commands, e, glyph(tool), 16.0);
        commands.entity(e).insert_children(0, &[icon]);
    }
    let options = node(
        commands,
        toolbar,
        Node {
            display: Display::None,
            min_width: px(0),
            flex_shrink: 1.0,
            overflow: Overflow::scroll_x(),
            ..row(4.0)
        },
    );
    commands
        .entity(options)
        .insert((Scroller, HorizontalScroller));
    let select_opts = node(commands, options, row(3.0));
    let mode_buttons = [
        ("Новое", Mode::Replace),
        ("Добавить", Mode::Add),
        ("Вычесть", Mode::Subtract),
        ("Пересечь", Mode::Intersect),
    ]
    .map(|(label, mode)| item(commands, select_opts, font, label, Op::Mode(mode)));
    for (label, command) in [
        ("Всё", "select.all"),
        ("Снять", "select.deselect"),
        ("Инверсия", "select.inverse"),
    ] {
        item(commands, select_opts, font, label, Op::Command(command));
    }
    let advanced = node(commands, select_opts, row(3.0));
    item(commands, advanced, font, "Все слои", Op::SampleAll);
    let wand_options = node(commands, advanced, row(3.0));
    item(commands, wand_options, font, "Смежные", Op::Contiguous);
    item(commands, wand_options, font, "−", Op::Tolerance(-4));
    let tolerance = text(commands, wand_options, font, "Допуск 32", 12.0, TEXT);
    item(commands, wand_options, font, "+", Op::Tolerance(4));
    let quick_options = node(commands, advanced, row(3.0));
    item(commands, quick_options, font, "Кисть −", Op::QuickSize(-5));
    let quick_size = text(commands, quick_options, font, "30 px", 12.0, TEXT);
    item(commands, quick_options, font, "Кисть +", Op::QuickSize(5));
    let trans_opts = node(commands, options, row(3.0));
    let mode_button = item(commands, trans_opts, font, "Свободная", Op::TransformMenu);
    icons::themed(commands, mode_button, icons::Glyph::ChevronDown, 12.0);
    let mode_label = mode_button;
    for (label, op) in [
        ("Пропорции", Op::Linked),
        ("Пиксели", Op::Nearest),
        ("↔", Op::Flip(true)),
        ("↕", Op::Flip(false)),
        ("−90°", Op::Rotate(-90.0)),
        ("+90°", Op::Rotate(90.0)),
        ("Применить", Op::Apply),
        ("Отмена", Op::Cancel),
    ] {
        item(commands, trans_opts, font, label, op);
    }
    let trans_menu = menu(commands, root);
    for (label, mode) in [
        ("Свободная", TransformMode::Free),
        ("Масштаб", TransformMode::Scale),
        ("Поворот", TransformMode::Rotate),
        ("Наклон", TransformMode::Skew),
        ("Искажение", TransformMode::Distort),
        ("Перспектива", TransformMode::Perspective),
    ] {
        let e = item(commands, trans_menu, font, label, Op::TransformMode(mode));
        commands.entity(e).entry::<Node>().and_modify(|mut n| {
            n.width = percent(100);
            n.height = px(31);
        });
    }
    let overlay = node(
        commands,
        canvas,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            ..default()
        },
    );
    commands
        .entity(overlay)
        .insert((ZIndex(5), Pickable::IGNORE));
    let handles = std::array::from_fn(|i| {
        let handle = surface(
            commands,
            overlay,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: px(8),
                height: px(8),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(if i == 8 { 4 } else { 1 })),
                ..default()
            },
            Color::WHITE,
        );
        commands
            .entity(handle)
            .insert((BorderColor::all(Color::BLACK), Pickable::IGNORE));
        handle
    });
    commands.insert_resource(ToolsUi {
        button: choose,
        icon,
        selected_tool: SelectionTool::Rectangle,
        menu: selection_menu,
        transform_button: mode_button,
        transform_menu: trans_menu,
        options,
        selection_options: select_opts,
        intersection: mode_buttons[3],
        transform_options: trans_opts,
        mode_label,
        tolerance,
        quick_size,
        advanced,
        wand_options,
        quick_options,
        overlay,
        lines: Vec::new(),
        handles,
        mask_key: None,
        mask_hash: None,
        mask_source: None,
        mask_image: None,
    });
}
fn popup_input(
    mut viewer: ResMut<ImageViewer>,
    ui: Res<ToolsUi>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
) {
    viewer.edit.selection.dismissed = false;
    let Some(point) = window.cursor_position().map(|p| p * window.scale_factor()) else {
        return;
    };
    let hit = |e| {
        geometry
            .get(e)
            .is_ok_and(|(n, t)| !n.is_empty() && n.contains_point(*t, point))
    };
    if mouse.just_pressed(MouseButton::Right) && hit(ui.button) {
        viewer.edit.selection.menu = if viewer.edit.selection.menu == 1 {
            0
        } else {
            1
        };
    }
    if mouse.just_pressed(MouseButton::Left)
        && viewer.edit.selection.menu != 0
        && !hit(ui.menu)
        && !hit(ui.transform_menu)
        && !hit(ui.button)
        && !hit(ui.transform_button)
    {
        viewer.edit.selection.menu = 0;
        viewer.edit.selection.dismissed = true;
    }
}
fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    viewer: Res<ImageViewer>,
    mut ui: ResMut<ToolsUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    children: Query<&Children>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<super::super::CanvasMaterial>>,
    time: Res<Time>,
) {
    let state = &viewer.edit;
    let tools = &state.selection;
    let editing = state.editing(editor.viewing_image);
    let transforming = tools
        .transform
        .as_ref()
        .is_some_and(|t| Some(t.id) == editor.viewing_image);
    let selecting = matches!(state.tool, PhotoTool::Select(_));
    for (entity, shown) in [
        (ui.options, editing && (selecting || transforming)),
        (ui.selection_options, selecting && !transforming),
        (
            ui.intersection,
            state.tool != PhotoTool::Select(SelectionTool::Quick),
        ),
        (ui.transform_options, transforming),
        (ui.menu, editing && tools.menu == 1),
        (ui.transform_menu, editing && tools.menu == 2),
        (
            ui.wand_options,
            state.tool == PhotoTool::Select(SelectionTool::Wand),
        ),
        (
            ui.quick_options,
            state.tool == PhotoTool::Select(SelectionTool::Quick),
        ),
        (
            ui.advanced,
            matches!(
                state.tool,
                PhotoTool::Select(
                    SelectionTool::Wand | SelectionTool::Quick | SelectionTool::Object
                )
            ),
        ),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if shown { Display::Flex } else { Display::None };
        }
    }
    if ui.selected_tool != tools.tool {
        icons::set_glyph(&mut commands, ui.icon, glyph(tools.tool));
        ui.selected_tool = tools.tool;
    }
    for (entity, value) in [
        (ui.tolerance, format!("Допуск {}", tools.tolerance)),
        (ui.quick_size, format!("{} px", tools.quick_size.max(1))),
    ] {
        if let Ok(mut text) = labels.get_mut(entity) {
            text.set_if_neq(Text(value));
        }
    }
    if let Ok(children) = children.get(ui.mode_label) {
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child) {
                label.set_if_neq(Text(
                    match tools.mode_transform {
                        TransformMode::Free => "Свободная",
                        TransformMode::Scale => "Масштаб",
                        TransformMode::Rotate => "Поворот",
                        TransformMode::Skew => "Наклон",
                        TransformMode::Distort => "Искажение",
                        TransformMode::Perspective => "Перспектива",
                    }
                    .into(),
                ));
            }
        }
    }
    for (entity, anchor, right) in [
        (ui.menu, ui.button, true),
        (ui.transform_menu, ui.transform_button, false),
    ] {
        if let Ok((n, t)) = geometry.get(anchor) {
            let min = (t.affine().translation - n.size() * 0.5) / window.scale_factor();
            let max = (t.affine().translation + n.size() * 0.5) / window.scale_factor();
            if let Ok(mut menu) = nodes.get_mut(entity) {
                menu.left = px(if right { max.x + 5.0 } else { min.x }
                    .clamp(4.0, (window.width() - 244.0).max(4.0)));
                menu.top = px(if right { min.y } else { max.y + 4.0 }
                    .clamp(4.0, (window.height() - 300.0).max(4.0)));
            }
        }
    }
    let doc = state.document(editor.viewing_image).filter(|_| editing);
    if let Some(doc) = doc {
        let key = editor.viewing_image.map(|id| (id, doc.revision));
        if ui.mask_key != key {
            ui.mask_key = key;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            doc.size().width.hash(&mut hasher);
            doc.size().height.hash(&mut hasher);
            if let Some(mask) = &doc.document.selection {
                mask.default_bytes().hash(&mut hasher);
                for (coord, tile) in mask.tiles() {
                    coord.hash(&mut hasher);
                    (std::sync::Arc::as_ptr(tile) as usize).hash(&mut hasher);
                }
                let hash = Some(hasher.finish());
                if ui.mask_hash != hash {
                    let data = mask
                        .read_region(doc.document.bounds())
                        .iter()
                        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                        .collect::<Vec<_>>();
                    let mut gray =
                        image::GrayImage::from_raw(doc.size().width, doc.size().height, data);
                    if let Some(image) = gray.take() {
                        let image = if image.width().max(image.height()) > 4096 {
                            image::DynamicImage::ImageLuma8(image)
                                .thumbnail(4096, 4096)
                                .into_luma8()
                        } else {
                            image
                        };
                        let mut texture = Image::new(
                            Extent3d {
                                width: image.width(),
                                height: image.height(),
                                depth_or_array_layers: 1,
                            },
                            TextureDimension::D2,
                            image.into_raw(),
                            TextureFormat::R8Unorm,
                            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
                        );
                        texture.sampler = ImageSampler::nearest();
                        if let Some(mut image) =
                            ui.mask_image.as_ref().and_then(|h| images.get_mut(h))
                        {
                            *image = texture;
                        } else {
                            ui.mask_image = Some(images.add(texture));
                        }
                    }
                    ui.mask_hash = hash;
                    ui.mask_source = Some(mask.clone());
                }
            } else {
                ui.mask_source = None;
                ui.mask_hash = None;
            }
        }
    } else {
        ui.mask_key = None;
        ui.mask_source = None;
        ui.mask_hash = None;
    }
    if let Some(mut material) = materials.get_mut(&super::super::canvas_material::HANDLE) {
        material.selection = ui.mask_image.clone();
        material.selection_style = Vec4::new(
            if editing && ui.mask_source.is_some() {
                1.0
            } else {
                0.0
            },
            time.elapsed_secs() * 12.0,
            0.0,
            0.0,
        );
        if let Some(draft) = tools
            .draft
            .as_ref()
            .filter(|d| Some(d.id) == editor.viewing_image && editing)
        {
            if matches!(
                draft.tool,
                SelectionTool::Rectangle
                    | SelectionTool::Ellipse
                    | SelectionTool::Object
                    | SelectionTool::Row
                    | SelectionTool::Column
            ) {
                material.preview_rect = Vec4::from_array(draft.bounds);
                material.selection_style.z = if draft.tool == SelectionTool::Ellipse {
                    2.0
                } else {
                    1.0
                };
            }
        }
    }
    let rect = super::super::canvas_rect(&canvas, &window);
    let image = viewer.dimensions.map(|d| viewer.image_rect(d, rect));
    let scale = viewer
        .dimensions
        .zip(image)
        .map_or(1.0, |(d, r)| r.width() / d.x as f32);
    let screen = |p: Vec2| image.map_or(Vec2::ZERO, |r| r.min - rect.min + p * scale);
    let mut lines = Vec::new();
    if editing && let Some(draft) = &tools.draft {
        if matches!(draft.tool, SelectionTool::Lasso | SelectionTool::Polygon) {
            for pair in draft.points.windows(2) {
                lines.push((screen(pair[0]), screen(pair[1])));
            }
            if let Some(last) = draft.points.last() {
                let hover = if draft.tool == SelectionTool::Polygon {
                    window
                        .cursor_position()
                        .and_then(|p| image.map(|r| (p - r.min) / scale))
                        .unwrap_or(draft.hover)
                } else {
                    draft.hover
                };
                lines.push((screen(*last), screen(hover)));
            }
        }
    }
    let transform = tools.transform.as_ref().filter(|_| editing && transforming);
    if let Some(transform) = transform {
        for i in 0..4 {
            lines.push((
                screen(transform.quad[i]),
                screen(transform.quad[(i + 1) % 4]),
            ));
        }
        let h = selection::handles(transform.quad, scale);
        lines.push((screen(h[1]), screen(h[8])));
    }
    for (i, entity) in ui.handles.iter().enumerate() {
        if let Ok(mut node) = nodes.get_mut(*entity) {
            node.display = if transform.is_some() {
                Display::Flex
            } else {
                Display::None
            };
            if let Some(transform) = transform {
                let p = screen(selection::handles(transform.quad, scale)[i]);
                node.left = px(p.x - 4.0);
                node.top = px(p.y - 4.0);
            }
        }
    }
    while ui.lines.len() < lines.len().min(520) {
        let entity = surface(
            &mut commands,
            ui.overlay,
            Node {
                position_type: PositionType::Absolute,
                height: px(3),
                ..default()
            },
            Color::BLACK,
        );
        commands.entity(entity).insert(Pickable::IGNORE);
        let inside = surface(
            &mut commands,
            entity,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(1),
                width: percent(100),
                height: px(1),
                ..default()
            },
            Color::WHITE,
        );
        commands.entity(inside).insert(Pickable::IGNORE);
        ui.lines.push(entity);
    }
    for (i, entity) in ui.lines.iter().enumerate() {
        if let Ok(mut node) = nodes.get_mut(*entity) {
            if let Some((a, b)) = lines.get(i) {
                let delta = *b - *a;
                let center = (*a + *b) * 0.5;
                node.display = Display::Flex;
                node.width = px(delta.length());
                node.left = px(center.x - delta.length() * 0.5);
                node.top = px(center.y - 1.5);
                commands
                    .entity(*entity)
                    .insert(UiTransform::from_rotation(Rot2::radians(
                        delta.y.atan2(delta.x),
                    )));
            } else {
                node.display = Display::None;
            }
        }
    }
}
