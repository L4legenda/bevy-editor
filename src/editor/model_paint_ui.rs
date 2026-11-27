use super::*;
use super::super::super::control_hints::Help;
#[path = "model_paint_uv_overlay.rs"]
mod uv_overlay;
#[path = "model_paint_graph_canvas.rs"]
mod graph_canvas;
#[path = "model_paint_shader_maker.rs"]
mod shader_maker;
#[path = "model_paint_shader_code.rs"]
mod shader_code;
#[path = "model_paint_layer_thumbs.rs"]
mod layer_thumbs;
pub(in crate::editor::ui::model_viewer) use shader_maker::refresh as refresh_shader_maker;
pub(in crate::editor::ui::model_viewer) use shader_code::decorate as decorate_shader_code;
pub(in crate::editor::ui::model_viewer) use shader_maker::Maker as ShaderMaker;
#[cfg(test)]
pub(super) use graph_canvas::NodePreview;
#[cfg(test)]
pub(super) use graph_canvas::PresetSwatch;
#[cfg(test)]
pub(super) use shader_maker::Swatch as ShaderSwatch;
pub(super) use graph_canvas::position_next_to_graph;
pub(in crate::editor::ui::model_viewer) use graph_canvas::refresh as refresh_graph;
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    input::{ButtonState, keyboard::KeyboardInput},
    input_focus::{FocusedInput, InputFocus},
    picking::{events::{PointerClick, PointerDragDrop, PointerDragLeave, PointerDragOver}, pointer::PointerButton},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    text::EditableText,
    ui::ScrollPosition,
    ui_widgets::{SelectAllOnFocus, TextInput},
};

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct PaintButton(PaintAction);

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct ShelfSearch;

#[derive(Clone, Copy)]
enum PaintDropZone {
    View3d,
    ViewUv,
    Layer { index: usize, folder_open: Option<bool> },
    FillChannel(usize),
}

impl PaintDropZone {
    fn layer(self) -> Option<usize> {
        match self {
            Self::View3d | Self::ViewUv | Self::FillChannel(_) => None,
            Self::Layer { index, .. } => Some(index),
        }
    }

    fn placement(self, y: f32, moving_layer: bool) -> ShelfPlacement {
        match self {
            Self::View3d | Self::ViewUv | Self::FillChannel(_) => ShelfPlacement::Top,
            Self::Layer { index, folder_open: Some(open) }
                if y >= 0.25 && (y <= 0.75 || (moving_layer && open)) => {
                ShelfPlacement::Inside(index)
            }
            Self::Layer { index, .. } if y < 0.5 => ShelfPlacement::Above(index),
            Self::Layer { index, .. } => ShelfPlacement::Below(index),
        }
    }

    fn action(self, source: PaintAction, y: f32) -> Option<PaintAction> {
        if let Self::FillChannel(channel) = self {
            return match source {
                PaintAction::Grain(Some(texture)) | PaintAction::AddShelfTexture(texture) => {
                    Some(PaintAction::TextureToChannel(texture, channel))
                }
                _ => None,
            };
        }
        let material = match source {
            PaintAction::Select(from) if matches!(self, Self::Layer { .. }) => {
                return Some(PaintAction::MoveLayer(from, self.placement(y, true)));
            }
            PaintAction::ApplyGenerator(kind, None) => {
                return Some(PaintAction::ApplyGenerator(kind, self.layer()));
            }
            PaintAction::ApplyShelfMask(index, None) => {
                return Some(PaintAction::ApplyShelfMask(index, self.layer()));
            }
            PaintAction::ApplyShelfMaterial(index) => ShelfMaterial::User(index),
            PaintAction::ApplyDownloaded(index) => ShelfMaterial::Downloaded(index),
            PaintAction::ApplySavedGraph(index) => ShelfMaterial::Graph(index),
            PaintAction::CommunityGraph(index) => ShelfMaterial::CommunityGraph(index),
            PaintAction::Preset(index) | PaintAction::AddMaterial(index) => ShelfMaterial::Basic(index),
            PaintAction::AddSmartMaterial(index) => ShelfMaterial::Smart(index),
            PaintAction::Grain(Some(index)) | PaintAction::AddShelfTexture(index) => ShelfMaterial::Texture(index),
            _ => return None,
        };
        Some(PaintAction::DropMaterial(material, self.placement(y, false)))
    }
}

#[derive(Component)]
struct PaintDropTarget {
    zone: PaintDropZone,
    marker: Entity,
}

#[derive(Component)]
struct PaintDropMarker;

#[derive(SystemParam)]
struct PaintDropInput<'w> {
    editor: Res<'w, Editor>,
    editing: Res<'w, editing::State>,
    paint: Res<'w, PaintState>,
    viewer: Res<'w, Viewer>,
    picker: ResMut<'w, sets::ShelfPick>,
}

impl PaintDropInput<'_> {
    fn action(
        &mut self,
        zone: PaintDropZone,
        source: &PaintButton,
        cursor: Vec2,
        node: &ComputedNode,
        transform: &UiGlobalTransform,
    ) -> Option<PaintAction> {
        let local = transform.affine().inverse().transform_point2(cursor / node.inverse_scale_factor());
        let y = 0.5 + local.y / node.size().y.max(1.);
        let action = zone.action(source.0, y)?;
        let model = self.editor.viewing_model?;
        let session = self.paint.sessions.get(&model)?;
        let doc = self.editing.documents.get(&model)?;
        if doc.mode != Mode::Paint || self.editor.pending.is_some() || self.editor.viewing_image.is_some() {
            return None;
        }
        let action = if let (PaintDropZone::View3d, PaintAction::DropMaterial(material, _)) = (zone, action) {
            let scale = node.inverse_scale_factor();
            let rect = Rect::from_center_size(transform.affine().translation * scale, node.size() * scale);
            let picked = (session.set_appearance.shown_count > 1 && rect.contains(cursor)).then(|| {
                self.picker.hit(model, doc, &session.set_appearance.visibility, editing::input::ray(&self.viewer, rect, cursor))
            }).flatten();
            self.picker.hover = picked.map(|key| (model, key));
            picked.filter(|&key| session.binding != Some(key))
                .map_or(action, |(object, slot)| PaintAction::DropMaterialOnSet(material, object, slot))
        } else {
            action
        };
        session.available(action).then_some(action)
    }
}

fn register_paint_drop(commands: &mut Commands, target: Entity, zone: PaintDropZone) {
    let marker = commands.spawn((
        PaintDropMarker,
        Node { display: Display::None, ..default() },
        BorderColor::all(FOCUS),
        Pickable::IGNORE,
        ChildOf(target),
    )).id();
    commands.entity(target)
        .insert(PaintDropTarget { zone, marker })
        .observe(drop_paint_item)
        .observe(show_paint_drop)
        .observe(hide_paint_drop);
}

fn drop_paint_item(
    mut event: On<PointerDragDrop>,
    sources: Query<&PaintButton>,
    targets: Query<(&PaintDropTarget, &ComputedNode, &UiGlobalTransform)>,
    mut markers: Query<&mut Node, With<PaintDropMarker>>,
    mut actions: MessageWriter<Action>,
    mut pick: PaintDropInput,
) {
    if event.button == PointerButton::Primary
        && let Ok(source) = sources.get(event.dropped)
        && let Ok((target, node, transform)) = targets.get(event.entity)
        && let Some(action) = pick.action(target.zone, source, event.pointer.position, node, transform)
    {
        if let Ok(mut marker) = markers.get_mut(target.marker) {
            marker.display = Display::None;
        }
        actions.write(Action::ModelView(ModelViewAction::Paint(action)));
        event.propagate(false);
    }
    pick.picker.hover = None;
}

fn show_paint_drop(
    event: On<PointerDragOver>,
    sources: Query<&PaintButton>,
    targets: Query<(&PaintDropTarget, &ComputedNode, &UiGlobalTransform)>,
    mut markers: Query<&mut Node, With<PaintDropMarker>>,
    mut pick: PaintDropInput,
) {
    let Ok((target, node, transform)) = targets.get(event.entity) else {
        return;
    };
    let Ok(mut marker) = markers.get_mut(target.marker) else {
        return;
    };
    let action = (event.button == PointerButton::Primary).then(|| {
        let source = sources.get(event.dragged).ok()?;
        pick.action(target.zone, source, event.pointer.position, node, transform)
    }).flatten();
    let Some(action) = action else {
        marker.display = Display::None;
        return;
    };
    let border = match action {
        PaintAction::DropMaterial(_, ShelfPlacement::Above(_))
        | PaintAction::MoveLayer(_, ShelfPlacement::Above(_)) => UiRect::top(px(2)),
        PaintAction::DropMaterial(_, ShelfPlacement::Below(_))
        | PaintAction::MoveLayer(_, ShelfPlacement::Below(_)) => UiRect::bottom(px(2)),
        _ => UiRect::all(px(1.5)),
    };
    marker.set_if_neq(Node {
        position_type: PositionType::Absolute,
        left: px(0.), right: px(0.), top: px(0.), bottom: px(0.),
        border, border_radius: BorderRadius::all(px(4)),
        ..default()
    });
}

fn hide_paint_drop(
    event: On<PointerDragLeave>,
    targets: Query<&PaintDropTarget>,
    mut markers: Query<&mut Node, With<PaintDropMarker>>,
    mut pick: ResMut<sets::ShelfPick>,
) {
    if let Ok(target) = targets.get(event.entity)
        && let Ok(mut marker) = markers.get_mut(target.marker)
    {
        marker.display = Display::None;
    }
    if targets.get(event.entity).is_ok_and(|target| matches!(target.zone, PaintDropZone::View3d)) {
        pick.hover = None;
    }
}

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct Value(Parameter);

#[derive(Component, Clone, Copy)]
pub(in crate::editor::ui::model_viewer) struct PreviewValue(pub(super) PreviewParameter);

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct UvWire;

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct UvSeams2d;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor::ui::model_viewer) enum HistoryPreference {
    Recovery,
    Undo,
}

impl HistoryPreference {
    pub(super) fn range(self) -> (u16, u16) {
        match self {
            Self::Recovery => (1, editing::recovery::MAX_EDITS),
            Self::Undo => (
                editing::recovery::MIN_UNDO_STEPS as u16,
                editing::recovery::MAX_UNDO_STEPS as u16,
            ),
        }
    }

    pub(super) fn parse(self, input: &EditableText) -> Option<u16> {
        let (min, max) = self.range();
        input.value().to_string().trim().parse::<u16>().ok()
            .filter(|number| (min..=max).contains(number))
    }
}
#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct CustomMapName(pub u64);

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct LayerMemoryHint(usize);

fn layer_memory_hint(bytes: u64) -> String {
    format!("Буферы слоя на GPU · {:.2} МиБ", bytes as f64 / 1_048_576.)
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct PaintUi {
    library_open: bool,
    library_motion: DrawerMotion,
    graph: graph_canvas::Canvas,
    roots: Vec<Entity>,
    original: Vec<Entity>,
    hidden: Vec<(Entity, Display)>,
    canvas: Entity,
    canvas_parent: Entity,
    graph_settings_stamp: Option<(usize, u64, u64)>,
    model_pane: Entity,
    workspace: Entity,
    side: Entity,
    workspace_parent: Entity,
    uv_image: Handle<Image>,
    uv_canvas: Entity,
    uv_wire: Entity,
    uv_seams: Entity,
    uv_info: Entity,
    content: Entity,
    stacks: [Entity; 5],
    layer_thumbs: layer_thumbs::Cache,
    online_previews: online::Previews,
    shelf: Entity,
    shelf_layout: ShelfLayout,
    shelf_input: Entity,
    shelf_hint: Entity,
    shelf_types: Entity,
    shelf_type_menu: Entity,
    shelf_type_label: Entity,
    shelf_stamp: Option<((usize, u64, u64, usize, usize), Category, u64, u64, u32)>,
    user_swatches: HashMap<usize, Handle<Image>>,
    graph_swatches: HashMap<usize, Handle<Image>>,
    requested_swatches: std::collections::HashSet<ShelfMaterial>,
    shelf_preview_job: Option<(ShelfMaterial, Task<Result<image::RgbaImage, String>>)>,
    layer_name: Entity,
    brush_name: Entity,
    diagnostic_label: Entity,
    diagnostic_meter: diagnostics::Meter,
    fill_options: Entity,
    fill_angle: Entity,
    path_options: Entity,
    multi_brush: Entity,
    brush_values: [(usize, Entity); 4],
    tool_sections: [(Entity, ToolMask); 4],
    three_title: Entity,
    three_label: Entity,
    stamps: [Option<(usize, u64, u64, usize, usize)>; 5],
    stack_context: [Option<(usize, Option<sets::SetKey>)>; 5],
    texture_value: Option<TextureValue>,
    bake_option_field: Option<(usize, Entity, bake::BakeControl)>,
    preview_field: Option<(usize, Entity)>,
    uv_overlay: uv_overlay::Overlay,
    swatches: Vec<Handle<Image>>,
    smart_swatches: Vec<Handle<Image>>,
    generator_swatches: Vec<Handle<Image>>,
    showing: bool,
}

impl PaintUi {
    pub(super) fn take_graph_batch(&mut self, id: u64, model: usize, session: &Session) -> Option<material_graph::Batch> {
        self.graph.take_batch(id, model, session)
    }

    #[cfg(test)]
    pub(super) fn graph_selection(&self) -> &[u64] {
        self.graph.selection()
    }

}

struct ShelfLayout {
    home: Entity,
    root: Entity,
    handle: Entity,
    header: Entity,
    search: Entity,
    body: Entity,
    categories: Entity,
    category_buttons: Vec<Entity>,
    docked: bool,
}

impl ShelfLayout {
    fn arrange(&mut self, docked: bool, menu: Entity, types: Entity, cards: Entity, nodes: &mut Query<&mut Node>) {
        if self.docked == docked {
            return;
        }
        self.docked = docked;
        if let Ok(mut handle) = nodes.get_mut(self.handle) {
            handle.display = if docked { Display::None } else { Display::Flex };
        }
        if let Ok(mut header) = nodes.get_mut(self.header) {
            header.flex_wrap = if docked { FlexWrap::Wrap } else { FlexWrap::NoWrap };
            header.row_gap = if docked { px(3) } else { px(0) };
            header.padding = if docked { UiRect::all(px(5)) } else { UiRect::horizontal(px(10)) };
        }
        if let Ok(mut search) = nodes.get_mut(self.search) {
            search.width = if docked { percent(100) } else { px(235) };
            search.min_width = if docked { px(0) } else { px(92) };
            search.margin = if docked { UiRect::ZERO } else { UiRect::left(px(5)) };
        }
        if let Ok(mut types) = nodes.get_mut(types) {
            types.width = if docked { percent(100) } else { Val::Auto };
        }
        if let Ok(mut menu) = nodes.get_mut(menu) {
            menu.top = if docked { px(28) } else { Val::Auto };
            menu.bottom = if docked { Val::Auto } else { px(28) };
            menu.max_width = if docked { percent(100) } else { Val::Auto };
            menu.max_height = if docked { px(126) } else { Val::Auto };
            menu.overflow = if docked { Overflow::scroll_y() } else { Overflow::visible() };
        }
        if let Ok(mut body) = nodes.get_mut(self.body) {
            body.flex_direction = if docked { FlexDirection::Column } else { FlexDirection::Row };
        }
        if let Ok(mut categories) = nodes.get_mut(self.categories) {
            categories.flex_direction = if docked { FlexDirection::Row } else { FlexDirection::Column };
            categories.width = if docked { percent(100) } else { px(141) };
            categories.height = if docked { px(33) } else { percent(100) };
            categories.overflow = if docked { Overflow::scroll_x() } else { Overflow::visible() };
            categories.column_gap = px(2);
        }
        for &entity in &self.category_buttons {
            if let Ok(mut button) = nodes.get_mut(entity) {
                button.width = if docked { Val::Auto } else { percent(100) };
            }
        }
        if let Ok(mut cards) = nodes.get_mut(cards) {
            cards.height = if docked { Val::Auto } else { percent(100) };
            cards.width = if docked { percent(100) } else { Val::Auto };
            cards.min_height = px(0);
        }
    }
}

struct TextureValue {
    model: usize,
    binding: Option<(usize, usize)>,
    layer: usize,
    parameter: Parameter,
    value: f32,
}
#[cfg(test)]
impl PaintUi {
    pub(super) fn retained_labels(&self) -> [Entity; 2] {
        [self.layer_name, self.brush_name]
    }

    pub(super) fn pending_graph_source(&self) -> Option<u64> {
        self.graph.pending_source()
    }
}
fn generator_controls(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    index: Option<usize>,
    generator: Option<Generator>,
    id_palette: &[[u8; 3]],
    disabled_inputs: u32,
    mix: Option<bool>,
    custom_maps: &[custom::Recipe],
) {
    let action = |edit| PaintAction::Generator(index, edit);
    let tools = node(
        commands,
        parent,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(4.),
            ..row(4.)
        },
    );
    for kind in GeneratorKind::ALL {
        button(
            commands,
            tools,
            font,
            kind.label(),
            action(GeneratorEdit::Set(kind)),
        );
    }
    let Some(generator) = generator else { return };
    let missing = GeneratorInput::ALL.into_iter().filter(|&input|
        disabled_inputs & (1 << input as u32) != 0 && generator.weight(input) != 0
    ).map(GeneratorInput::label).collect::<Vec<_>>();
    if !missing.is_empty() {
        text(commands, parent, font, &format!("Карты пропущены при запекании · {}", missing.join(", ")), 11., MUTED);
    }
    button(
        commands,
        tools,
        font,
        "Инверсия",
        action(GeneratorEdit::Invert),
    );
    if index.is_none() {
        icon(
            commands,
            tools,
            icons::Glyph::Trash,
            "Убрать генератор основы",
            action(GeneratorEdit::Remove),
        );
    }
    if generator.weight(GeneratorInput::Id) != 0 {
        id_controls(commands, parent, font, index, generator, id_palette);
    }
    if generator.weight(GeneratorInput::Edges) != 0 || generator.weight(GeneratorInput::Creases) != 0 {
        let line = node(commands, parent, row(4.));
        let width = f32::from(generator.edge_width) / 1000.;
        text(commands, line, font, &format!("Ширина · {width:.3}"), 11., MUTED);
        button(commands, line, font, "−", action(GeneratorEdit::EdgeWidth(-1)));
        button(commands, line, font, "+", action(GeneratorEdit::EdgeWidth(1)));
    }
    for (i, &(label, ..)) in generator.controls().iter().enumerate() {
        let line = node(commands, parent, row(4.));
        text(
            commands,
            line,
            font,
            &format!("{label} · {}", generator.value(i)),
            11.,
            MUTED,
        );
        button(
            commands,
            line,
            font,
            "−",
            action(GeneratorEdit::Adjust(i, -1)),
        );
        button(
            commands,
            line,
            font,
            "+",
            action(GeneratorEdit::Adjust(i, 1)),
        );
    }
    if generator.kind.uses_pattern_mapping() {
        return;
    }
    button(commands, parent, font, "Входы генератора", PaintAction::GeneratorInputs(index));
    if let Some(picker) = mix {
        generator_inputs(commands, parent, font, index, generator, picker, custom_maps);
    }
}

fn generator_inputs(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    index: Option<usize>,
    generator: Generator,
    picker: bool,
    custom_maps: &[custom::Recipe],
) {
    let action = |edit| PaintAction::Generator(index, edit);
    let weights = GeneratorInput::ALL.into_iter().zip(generator.weights());
    for (input, weight) in weights.clone().filter(|&(_, weight)| weight != 0) {
        let line = node(commands, parent, row(4.));
        icon(commands, line, icons::Glyph::Trash, "Убрать вход",
            action(GeneratorEdit::Weight(input, 0)));
        text(commands, line, font, input.label(), 11., MUTED);
        button(commands, line, font, &format!("{weight:+}%"),
            action(GeneratorEdit::Weight(input, -weight)));
        button(commands, line, font, "−",
            action(GeneratorEdit::Weight(input, (weight - 5).max(-Generator::WEIGHT_LIMIT))));
        button(commands, line, font, "+",
            action(GeneratorEdit::Weight(input, (weight + 5).min(Generator::WEIGHT_LIMIT))));
    }
    for input in generator.custom.iter().filter(|input| input.id != 0) {
        let line = node(commands, parent, row(4.));
        let weight = input.weight;
        let set = |weight| action(GeneratorEdit::CustomWeight(input.id, weight));
        icon(commands, line, icons::Glyph::Trash, "Убрать карту из маски", set(0));
        let name = custom_maps.iter().find(|map| map.id == input.id)
            .map_or("Удалённая карта", |map| map.name.as_str());
        text(commands, line, font, name, 11., MUTED);
        button(commands, line, font, &format!("{weight:+}%"), set(-weight));
        button(commands, line, font, "−", set((weight - 5).max(-200)));
        button(commands, line, font, "+", set((weight + 5).min(200)));
    }
    let line = node(commands, parent, row(4.));
    button(commands, line, font, "Добавить вход", PaintAction::GeneratorInputPicker(index));
    if generator.weights.is_some() {
        button(commands, line, font, "Веса пресета", action(GeneratorEdit::ResetWeights));
    }
    if picker {
        let list = node(commands, parent, Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(4.),
            ..row(4.)
        });
        for (input, _) in weights.filter(|&(_, weight)| weight == 0) {
            button(commands, list, font, input.label(), action(GeneratorEdit::Weight(input, 100)));
        }
        for map in custom_maps.iter().filter(|map| generator.custom.iter().all(|input| input.id != map.id)) {
            button(commands, list, font, &map.name, action(GeneratorEdit::CustomWeight(map.id, 100)));
        }
        if custom_maps.is_empty() {
            text(commands, parent, font, "Добавить пользовательскую карту можно во вкладке «Карты».", 11., MUTED);
        }
    }
}

fn id_controls(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    index: Option<usize>,
    generator: Generator,
    palette: &[[u8; 3]],
) {
    let action = |edit| PaintAction::Generator(index, edit);
    if palette.is_empty() {
        text(commands, parent, font, "Запеките карту ID для этого набора.", 11., MUTED);
        return;
    }
    let current = generator.id_color.unwrap_or(palette[0]);
    let selected = palette.binary_search(&current).unwrap_or_else(|i| i.min(palette.len() - 1));
    let label = |[r, g, b]: [u8; 3]| format!("#{r:02X}{g:02X}{b:02X}");
    text(commands, parent, font, &format!("Цвет ID · {}", label(current)), 11., MUTED);
    let strip = node(commands, parent, row(4.));
    let first = selected.saturating_sub(2).min(palette.len().saturating_sub(5));
    if first > 0 {
        icon(commands, strip, icons::Glyph::ChevronLeft, "Предыдущие цвета ID",
            action(GeneratorEdit::IdColor(palette[first - 1])));
    }
    for (i, &rgb) in palette.iter().enumerate().skip(first).take(5) {
        let action = action(GeneratorEdit::IdColor(rgb));
        let swatch = super::super::super::control(
            commands, strip, Action::ModelView(ModelViewAction::Paint(action)), Look::Regular,
            Node {
                width: px(24.),
                height: px(24.),
                border: UiRect::all(px(2.)),
                border_radius: BorderRadius::all(px(6.)),
                ..default()
            },
        );
        commands.entity(swatch).insert((
            PaintButton(action),
            AccessibleLabel(format!("Цвет ID {} · {}", i + 1, label(rgb))),
        ));
        commands.spawn((
            Node {
                width: percent(100.),
                height: percent(100.),
                border_radius: BorderRadius::all(px(4.)),
                ..default()
            },
            BackgroundColor(Color::srgb_u8(rgb[0], rgb[1], rgb[2])),
            Pickable::IGNORE,
            ChildOf(swatch),
        ));
    }
    if first + 5 < palette.len() {
        icon(commands, strip, icons::Glyph::ChevronRight, "Следующие цвета ID",
            action(GeneratorEdit::IdColor(palette[first + 5])));
    }
    let line = node(commands, parent, row(4.));
    text(commands, line, font, &format!("Допуск · {}%", generator.id_tolerance), 11., MUTED);
    button(commands, line, font, "−", action(GeneratorEdit::IdTolerance(-1)));
    button(commands, line, font, "+", action(GeneratorEdit::IdTolerance(1)));
}

pub(super) fn button(
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

fn texture_set_label(
    doc: &crate::editor::model_document::Document,
    object: &crate::editor::model_document::Object,
    slot: usize,
) -> String {
    let entry = &object.materials[slot];
    let material = entry.name(&doc.source.json).unwrap_or("Материал");
    let size = doc.paint_size(entry);
    let name = entry.udim().map_or_else(|| material.to_owned(), |tile| {
        format!("{material}.{}", tile.number())
    });
    format!("{} · {name} · {size}×{size}", object.name)
}

fn layer_set_selector(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    doc: &crate::editor::model_document::Document,
    session: &Session,
) {
    let slots = doc.objects.iter()
        .flat_map(|object| (0..object.materials.len()).map(move |slot| (object, slot)));
    if slots.clone().nth(1).is_none() {
        return;
    }
    let Some((object, slot)) = slots.clone().nth(session.set) else { return };
    let current = texture_set_label(doc, object, slot);
    let arrow = if session.set_menu { "▴" } else { "▾" };
    button(commands, parent, font, &format!("{current} {arrow}"), PaintAction::SetMenu);
    if session.set_menu {
        let choices = node(commands, parent, column(3.));
        for (index, (object, slot)) in slots.enumerate() {
            let key = (object.id, slot);
            let mut label = texture_set_label(doc, object, slot);
            if object.hidden || !session.set_appearance.visibility.shown(key.0, key.1) {
                label.push_str(" · скрыт");
            }
            let choice = button(commands, choices, font, &label, PaintAction::Set(index));
            commands.entity(choice).insert(sets::SetRow(key));
        }
    }
}

fn mesh_replacement(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    doc: &crate::editor::model_document::Document,
    session: &Session,
) {
    let action = PaintAction::MeshReplacement;
    let title = if session.mesh_loading { "Загрузка сетки…" } else { "Замена сетки" };
    let toggle = button(commands, parent, font, title, action(MeshCommand::Toggle));
    if !session.mesh_loading {
        // The font has no ▾/▴ glyphs, so the disclosure arrow is drawn as an icon.
        let glyph = if session.mesh_menu { icons::Glyph::ChevronUp } else { icons::Glyph::ChevronDown };
        icons::spawn(commands, toggle, glyph, 12., MUTED);
    }
    if !session.mesh_menu || session.mesh_loading {
        return;
    }
    let choices = node(commands, parent, Node { flex_wrap: FlexWrap::Wrap, ..row(3.) });
    for shape in paint_mesh_replacement::Shape::ALL {
        button(commands, choices, font, shape.label(), action(MeshCommand::Primitive(shape)));
    }
    button(commands, parent, font, "Загрузить сетку OBJ / FBX…", action(MeshCommand::Import));
    let omitted = doc.paint_sets_available_on_mesh_undo();
    if omitted > 0 {
        text(commands, parent, font, &format!("Исходные наборы доступны через отмену: {omitted}"), 11., MUTED);
    }
    if doc.can_restore_paint_mesh(false) {
        button(commands, parent, font, "Отменить замену сетки", action(MeshCommand::Undo));
    }
    if doc.can_restore_paint_mesh(true) {
        button(commands, parent, font, "Повторить замену сетки", action(MeshCommand::Redo));
    }
}

fn layer_thumbnail(
    commands: &mut Commands,
    parent: Entity,
    background: Handle<Image>,
    thumbnail: Option<Handle<Image>>,
) -> Entity {
    let frame = commands.spawn((
        ImageNode::new(background),
        Node { width: px(28), height: px(28), flex_shrink: 0., ..default() },
        Pickable::IGNORE,
        ChildOf(parent),
    )).id();
    if let Some(source) = thumbnail {
        commands.spawn((
            ImageNode::new(source),
            Node { width: percent(100), height: percent(100), ..default() },
            Pickable::IGNORE,
            ChildOf(frame),
        ));
    }
    frame
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
/// Tools a property section applies to.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ToolMask(u8);

impl ToolMask {
    const BRUSH: Self = Self(1);
    const ERASER: Self = Self(2);
    const FILL: Self = Self(4);
    const PICKER: Self = Self(8);
    const PATH: Self = Self(16);
    const ALL: Self = Self(31);

    fn contains(self, tool: PaintTool) -> bool {
        let bit = match tool {
            PaintTool::Brush => Self::BRUSH,
            PaintTool::Eraser => Self::ERASER,
            PaintTool::Fill => Self::FILL,
            PaintTool::Picker => Self::PICKER,
            PaintTool::Path => Self::PATH,
        };
        self.0 & bit.0 != 0
    }
}

impl std::ops::BitOr for ToolMask {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitAnd for ToolMask {
    type Output = Self;
    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl std::ops::Not for ToolMask {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0 & Self::ALL.0)
    }
}

/// Collapsible group in the properties panel. Returns the container for its controls.
fn section(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    open: bool,
) -> Entity {
    commands.entity(parent).entry::<Node>().and_modify(|mut node| {
        node.width = percent(100);
        node.flex_shrink = 0.;
        node.padding = UiRect::top(px(8));
        node.border = UiRect::top(px(1));
    });
    commands.entity(parent).insert(BorderColor::all(EDGE));
    let header = commands.spawn((
        Node {
            width: percent(100),
            height: px(24),
            align_items: AlignItems::Center,
            column_gap: px(6),
            ..default()
        },
        BackgroundColor(Color::NONE),
        AccessibleLabel(format!("{label}: свернуть или развернуть")),
        ChildOf(parent),
    )).id();
    let open_icon = icons::spawn(commands, header, icons::Glyph::ChevronDown, 13., MUTED);
    let closed_icon = icons::spawn(commands, header, icons::Glyph::ChevronRight, 13., MUTED);
    let title = text(commands, header, font, &label.to_uppercase(), 11., MUTED);
    for entity in [open_icon, closed_icon, title] {
        commands.entity(entity).insert(Pickable::IGNORE);
    }
    let content = node(commands, parent, Node {
        display: if open { Display::Flex } else { Display::None },
        width: percent(100),
        padding: UiRect::left(px(2)),
        ..column(7.)
    });
    let display = |visible: bool| if visible { Display::Flex } else { Display::None };
    commands.entity(open_icon).entry::<Node>().and_modify(move |mut node| node.display = display(open));
    commands.entity(closed_icon).entry::<Node>().and_modify(move |mut node| node.display = display(!open));
    commands.entity(header).observe(
        move |mut event: On<PointerClick>, mut nodes: Query<&mut Node>| {
            if event.button != PointerButton::Primary {
                return;
            }
            event.propagate(false);
            let Ok(mut body) = nodes.get_mut(content) else { return };
            let open = body.display == Display::None;
            body.display = display(open);
            for (entity, visible) in [(open_icon, open), (closed_icon, !open)] {
                if let Ok(mut node) = nodes.get_mut(entity) {
                    node.display = display(visible);
                }
            }
        },
    ).observe(
        |event: On<bevy::picking::events::PointerEnter>, mut colors: Query<&mut BackgroundColor>| {
            if let Ok(mut color) = colors.get_mut(event.entity) {
                color.0 = Color::srgba(1., 1., 1., 0.04);
            }
        },
    ).observe(
        |event: On<bevy::picking::events::PointerLeave>, mut colors: Query<&mut BackgroundColor>| {
            if let Ok(mut color) = colors.get_mut(event.entity) {
                color.0 = Color::NONE;
            }
        },
    );
    content
}

/// Command column at the start of a library row. It scrolls on its own so a long list
/// (graph presets) never grows past the shelf and over the search bar.
fn shelf_tools(commands: &mut Commands, shelf: Entity) -> Entity {
    let frame = node(commands, shelf, Node {
        height: percent(100),
        min_width: px(170),
        flex_shrink: 0.,
        overflow: Overflow::scroll_y(),
        padding: UiRect::right(px(4)),
        ..column(0.)
    });
    commands.entity(frame).insert(Scroller);
    // Inner column keeps its natural height, so the buttons scroll instead of squashing.
    node(commands, frame, Node { width: percent(100), flex_shrink: 0., ..column(4.) })
}

fn toolbar_divider(commands: &mut Commands, parent: Entity) -> Entity {
    commands.spawn((
        Node {
            width: px(1),
            height: px(22),
            margin: UiRect::horizontal(px(2)),
            flex_shrink: 0.,
            ..default()
        },
        BackgroundColor(EDGE),
        Pickable::IGNORE,
        ChildOf(parent),
    )).id()
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
const SHELF_HEIGHT: f32 = 219.;
const TOOLBAR_BOTTOM: f32 = 88.;
const PATH_TOOLBAR_HEIGHT: f32 = 40.;

fn texture_decimal(parameter: Parameter) -> bool {
    matches!(parameter, Parameter::TextureTiling | Parameter::TextureOffsetU
        | Parameter::TextureOffsetV | Parameter::TextureNormalStrength | Parameter::TextureHeightStrength | Parameter::TextureHeightOffset
        | Parameter::TextureOpacityStrength | Parameter::TextureEmissionTint(_) | Parameter::TextureSpecularStrength
        | Parameter::FillVariationScale | Parameter::ExportHeightStrength)
}

fn number(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    parameter: Parameter,
    compact: bool,
) {
    let (min, max) = parameter.bounds();
    let number = if parameter == Parameter::GrainScale {
        Number::decimal(min as f32, max as f32, 0.25, 2)
    } else if texture_decimal(parameter) {
        Number::decimal(min as f32, max as f32, 0.05, 2)
    } else if parameter == Parameter::CageStrength {
        Number::decimal(min as f32, max as f32, 0.1, 1)
    } else if let Parameter::BakeOption(control) = parameter {
        let digits = control.digits();
        let speed = [1., 0.1, 0.1, 0.005][digits];
        Number::decimal(min as f32, max as f32, speed, digits)
    } else {
        Number::decimal(min as f32, max as f32, 1., 0)
    }.finish();
    let field = number_line(commands, parent, font, parameter.label(), number, compact);
    commands.entity(field).insert(Value(parameter));
    if matches!(parameter, Parameter::BakeOption(_)) {
        commands.entity(field).entry::<Node>().and_modify(|mut node| node.width = px(72));
    }
}

fn number_line(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    name: &str,
    number: Number,
    compact: bool,
) -> Entity {
    let line = node(
        commands,
        parent,
        Node {
            width: if compact { Val::Auto } else { percent(100) },
            ..row(5.)
        },
    );
    let label = text(commands, line, font, name, 12., MUTED);
    if !compact {
        commands.entity(label).insert(Node {
            flex_grow: 1.,
            ..default()
        });
    }
    commands.spawn((
        TextInput,
        EditableText::new(""),
        SelectAllOnFocus,
        crate::ui_theme::text_cursor_style(),
        number,
        TabIndex(0),
        AccessibleLabel(format!("Рисование: {name}")),
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
    )).id()
}
fn alpha_card(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    tip: &tips::Tip,
    action: PaintAction,
    size: f32,
) -> Entity {
    let preview = (size - 8.).max(ShelfSize::MIN as f32);
    let card = button(commands, parent, font, "", action);
    commands.entity(card).insert((
        AccessibleLabel(tip.name.clone()),
        Node {
            width: px(preview + 30.),
            height: px(preview + 62.),
            flex_shrink: 0.,
            padding: UiRect::all(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            align_items: AlignItems::Center,
            ..column(4.)
        },
    ));
    commands.spawn((
        ImageNode::new(tip.image.clone()),
        Node {
            width: px(preview),
            height: px(preview),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(card),
    ));
    text(commands, card, font, &tip.name, 11., TEXT);
    card
}

fn fill_channel_slots(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    session: &Session,
    textures: &[tips::Tip],
) {
    for (channel, label) in CHANNELS.into_iter().enumerate() {
        let slot = node(commands, parent, row(3.));
        let picker = PaintAction::FillTexturePicker(Some(channel));
        let target = button(commands, slot, font, "", picker);
        commands.entity(target).entry::<Node>().and_modify(|mut node| {
            node.width = px(0.);
            node.flex_grow = 1.;
            node.min_width = px(0.);
            node.overflow = Overflow::clip();
        });
        let bitmap = session.active_texture_fill().and_then(|fill| fill.images[channel].as_ref());
        let title = bitmap.map_or_else(|| label.to_owned(), |bitmap| format!("{label} · {}", bitmap.name));
        commands.entity(target).insert(AccessibleLabel(title.clone()));
        text(commands, target, font, &title, 11., TEXT);
        register_paint_drop(commands, target, PaintDropZone::FillChannel(channel));
        icon(
            commands, slot, icons::Glyph::FolderOpen, &format!("Загрузить файл · {label}"),
            PaintAction::FillTextureImport(channel),
        );
        if bitmap.is_some() {
            icon(
                commands, slot, icons::Glyph::Close, &format!("Убрать карту · {label}"),
                PaintAction::FillTextureRemove(channel),
            );
        }
        if session.fill_texture_picker != Some(channel) {
            continue;
        }
        let choices = node(commands, parent, Node {
            flex_wrap: FlexWrap::Wrap, row_gap: px(4.), ..row(4.)
        });
        for (index, texture) in textures.iter().enumerate() {
            let choice = button(commands, choices, font, "", PaintAction::TextureToChannel(index, channel));
            commands.entity(choice).insert((
                AccessibleLabel(format!("{} · {label}", texture.name)),
                Node { width: px(42), height: px(42), padding: UiRect::all(px(3.)), ..default() },
            ));
            control_hints::attach(commands, choice, &texture.name);
            commands.spawn((
                ImageNode::new(texture.image.clone()),
                Node { width: percent(100), height: percent(100), ..default() },
                Pickable::IGNORE,
                ChildOf(choice),
            ));
        }
    }
}

fn material_results<T>(
    items: &[T], filter: &ShelfFilter, describe: impl Fn(&T) -> (&str, MaterialType),
) -> Vec<(usize, MaterialType)> {
    let mut results: Vec<_> = items.iter().enumerate().filter_map(|(index, item)| {
        let (name, kind) = describe(item);
        (filter.matches(name) && filter.allows_material(kind)).then_some((index, kind))
    }).collect();
    results.sort_by_key(|&(_, kind)| kind);
    results
}

fn material_group(
    commands: &mut Commands, parent: Entity, font: &Handle<Font>,
    filter: &ShelfFilter, group: &mut Option<(MaterialType, Entity)>, kind: MaterialType,
) -> Entity {
    if filter.material.is_some() {
        return parent;
    }
    if let Some((previous, row)) = *group
        && previous == kind
    {
        return row;
    }
    let section = node(commands, parent, Node { flex_shrink: 0., ..column(4.) });
    text(commands, section, font, kind.label(), 11., MUTED);
    let cards = node(commands, section, row(7.));
    *group = Some((kind, cards));
    cards
}

fn material_card(
    commands: &mut Commands, parent: Entity, font: &Handle<Font>, name: &str,
    swatch: Handle<Image>, action: PaintAction, size: f32,
) -> Entity {
    let card = button(commands, parent, font, "", action);
    commands.entity(card).insert(Node {
        width: px(size + 28.),
        height: px(size + 56.),
        flex_shrink: 0.,
        padding: UiRect::all(px(5)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(8)),
        align_items: AlignItems::Center,
        ..column(4.)
    });
    commands.spawn((
        ImageNode::new(swatch),
        Node {
            width: px(size),
            height: px(size),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(card),
    ));
    let label = text(commands, card, font, name, 11., TEXT);
    commands.entity(label).insert((
        TextLayout::justify(Justify::Center),
        Node { width: percent(100), ..default() },
    ));
    card
}

fn graph_fields(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    session: &Session,
    payload: &serde_json::Value,
    maps: Option<&bake::Maps>,
    custom_maps: &[custom::Recipe],
) {
    use material_graph::{ParameterTarget, Edit};
    use material_painter_graph::Value;
    let Ok(graph) = material_graph::read(payload) else {
        return;
    };
    let Some((_, graph_images)) = session.graph_data() else {
        return;
    };
    let folder = session.active_folder_view();
    let library = material_graph::library();
    let library = library.as_ref();
    text(
        commands,
        parent,
        font,
        &format!("Граф · {}", graph.name),
        12.,
        TEXT,
    );
    if !session.graph_canvas_open() {
        button(commands, parent, font, "Открыть холст", PaintAction::GraphCanvas);
    }
    if session.smart_loading {
        text(commands, parent, font, "Пересчёт графа…", 11., MUTED);
    }
    let editing = session.graph_canvas_open();
    if editing && session.panel != Panel::Preview && folder.is_none() {
        let current = if graph.match_set {
            format!("По набору · {} × {}", session.texture_size, session.texture_size)
        } else {
            format!("{} × {}", graph.res, graph.res)
        };
        button(commands, parent, font, &current, PaintAction::GraphResolutionMenu);
        if session.graph_resolution_menu {
            let sizes = node(commands, parent, Node { flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(3.) });
            for size in std::iter::once(None).chain(material_graph::RESOLUTIONS.into_iter().map(Some)) {
                let label = size.map_or_else(|| "По набору".into(), |size| size.to_string());
                button(commands, sizes, font, &label, PaintAction::GraphEdit(Edit::Resolution(size)));
            }
        }
    }
    if !graph.params.is_empty() {
        let title = if session.panel == Panel::Preview { "Параметры шейдера" } else { "Параметры материала" };
        text(commands, parent, font, title, 12., TEXT);
    }
    let mut groups = Vec::new();
    for parameter in &graph.params {
        if !groups.contains(&parameter.group.as_str()) {
            groups.push(parameter.group.as_str());
        }
    }
    for group in groups {
        if !group.is_empty() {
            text(commands, parent, font, group, 11., MUTED);
        }
        for (index, published) in graph
            .params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.group == group)
        {
            let Some((param, value)) = graph.param_value(index, library) else {
                continue;
            };
            let line = node(commands, parent, row(4.));
            text(commands, line, font, &published.name, 11., TEXT);
            if published
                .default
                .as_ref()
                .is_some_and(|default| *default != value)
            {
                icon(
                    commands,
                    line,
                    icons::Glyph::Undo,
                    "Сбросить параметр материала",
                    PaintAction::GraphEdit(Edit::ResetParameter(index)),
                );
            }
            if let Value::MeshMap(map) = value {
                graph_map(commands, parent, font, session, ParameterTarget::Published(index), map, maps, custom_maps);
            } else if let Value::Image(path) = &value {
                graph_bitmap(
                    commands,
                    parent,
                    font,
                    path.as_deref(),
                    graph_images,
                    ParameterTarget::Published(index),
                );
            } else {
                graph_value(
                    commands,
                    parent,
                    font,
                    "",
                    &param.kind,
                    &value,
                    |component, direction| {
                        PaintAction::GraphEdit(Edit::AdjustParameter(index, component, direction))
                    },
                );
            }
            if editing && folder.is_none() {
                graph_label(commands, parent, font, index, false, &published.name);
                graph_label(commands, parent, font, index, true, &published.group);
                let tools = node(
                    commands,
                    parent,
                    Node {
                        flex_wrap: FlexWrap::Wrap,
                        ..row(4.)
                    },
                );
                if index > 0 {
                    button(
                        commands,
                        tools,
                        font,
                        "Выше",
                        PaintAction::GraphEdit(Edit::MoveParameter(index, -1)),
                    );
                }
                if index + 1 < graph.params.len() {
                    button(
                        commands,
                        tools,
                        font,
                        "Ниже",
                        PaintAction::GraphEdit(Edit::MoveParameter(index, 1)),
                    );
                }
                icon(
                    commands,
                    tools,
                    icons::Glyph::Trash,
                    "Убрать параметр из свойств материала",
                    PaintAction::GraphEdit(Edit::RemoveParameter(index)),
                );
                for (id, key) in &published.binds {
                    if let Some(def) = graph.node(*id).and_then(|node| library.find(&node.def)) {
                        button(
                            commands,
                            tools,
                            font,
                            &format!("{} #{} · {}", def.name, id, key.replace('_', " ")),
                            PaintAction::GraphSelectNode(*id),
                        );
                    }
                }
            }
        }
    }
    if !editing {
        let nodes = node(commands, parent, Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3.),
            ..row(4.)
        });
        for item in &graph.nodes {
            if let Some(definition) = library.find(&item.def) {
                button(commands, nodes, font, &format!("{} #{}", definition.name, item.id),
                    PaintAction::GraphSelectNode(item.id));
            }
        }
    }
    let Some(selected) = session
        .graph_node
        .and_then(|id| graph.node(id))
        .or(graph.nodes.first())
    else {
        return;
    };
    let Some(definition) = library.find(&selected.def) else {
        return;
    };
    for (index, parameter) in definition.params.iter().enumerate() {
        if let Some(view) = folder
            && !folder_graph::adjustment_allowed(&session.stack, &view.graph, selected.id, index)
            && !folder_graph::can_edit(&session.stack, &view.graph, Edit::SetMeshMap(ParameterTarget::Node(selected.id, index), 0))
            && !folder_graph::bitmap_target(&session.stack, &view.graph, ParameterTarget::Node(selected.id, index))
        {
            continue;
        }
        let Some(value) = library.value(selected, &parameter.name) else {
            continue;
        };
        let mask_input = (folder.is_some() && selected.def == "smart_mask")
            .then(|| parameter.name.strip_prefix("input_")).flatten();
        let mask_weight = (folder.is_some() && selected.def == "smart_mask")
            .then(|| parameter.name.strip_prefix("weight_")).flatten();
        let input_kind = mask_input.map(|_| material_painter_graph::ParamKind::Enum(
            std::iter::once("Нет").chain(GeneratorInput::ALL.into_iter().map(GeneratorInput::label))
                .map(str::to_owned).collect()
        ));
        let name = match (folder.is_some(), selected.def.as_str(), parameter.name.as_str()) {
            (true, "layer", "variation") => "Разброс канала".into(),
            (true, "layer", "variation_scale") => "Масштаб шума".into(),
            (true, "layer", "seed") => "Зерно шума".into(),
            (true, "layer", "part_variation") => "Разброс по деталям".into(),
            _ if mask_input.is_some() => format!("Источник {}", mask_input.unwrap()),
            _ if mask_weight.is_some() => format!("Вес {}", mask_weight.unwrap()),
            _ => parameter.name.replace('_', " "),
        };
        if let Value::MeshMap(map) = value {
            graph_map(commands, parent, font, session, ParameterTarget::Node(selected.id, index), map, maps, custom_maps);
        } else if let Value::Image(path) = &value {
            graph_bitmap(
                commands,
                parent,
                font,
                path.as_deref(),
                graph_images,
                ParameterTarget::Node(selected.id, index),
            );
        } else if editing || material_graph::adjustable(&value) {
            graph_value(
                commands,
                parent,
                font,
                &name,
                input_kind.as_ref().unwrap_or(&parameter.kind),
                &value,
                |component, direction| {
                    PaintAction::GraphAdjust(selected.id, index, component, direction)
                },
            );
        }
        if editing && folder.is_none() {
            let tools = node(
                commands,
                parent,
                Node {
                    flex_wrap: FlexWrap::Wrap,
                    ..row(4.)
                },
            );
            if let Some(current) = graph.param_of(selected.id, &parameter.name) {
                button(
                    commands,
                    tools,
                    font,
                    &format!("Отвязать · {}", graph.params[current].name),
                    PaintAction::GraphEdit(Edit::Unbind(selected.id, index)),
                );
            } else {
                button(
                    commands,
                    tools,
                    font,
                    &format!("Опубликовать · {name}"),
                    PaintAction::GraphEdit(Edit::Publish(selected.id, index)),
                );
            }
            for (target, published) in graph.params.iter().enumerate() {
                if graph.param_of(selected.id, &parameter.name) != Some(target)
                    && graph
                        .param_value(target, library)
                        .is_some_and(|(param, _)| {
                            material_graph::compatible(&param.kind, &parameter.kind)
                        })
                {
                    button(
                        commands,
                        tools,
                        font,
                        &format!("Связать · {}", published.name),
                        PaintAction::GraphEdit(Edit::Bind(selected.id, index, target)),
                    );
                }
            }
        }
    }
}

fn graph_map(
    commands: &mut Commands, parent: Entity, font: &Handle<Font>, session: &Session,
    target: material_graph::ParameterTarget, selected: usize,
    baked: Option<&bake::Maps>, custom_maps: &[custom::Recipe],
) {
    let label = |value| match material_graph::mesh_maps::kind(value) {
        Some(bake::BakeMap::Custom(id)) => custom_maps.iter().find(|map| map.id == id)
            .map_or("Пользовательская карта", |map| map.name.as_str()),
        Some(map) => map.label(),
        None => "Карта модели",
    };
    let available = |value| material_graph::mesh_maps::kind(value)
        .is_some_and(|kind| baked.is_some_and(|baked| baked.has_map(kind)));
    button(commands, parent, font, &format!("{} ▾", label(selected)), PaintAction::GraphMapMenu(target));
    if !available(selected) {
        text(commands, parent, font, "Карта ещё не запечена", 11., MUTED);
    }
    if session.graph_map_menu == Some(target) {
        let choices = node(commands, parent, Node { flex_wrap: FlexWrap::Wrap, row_gap: px(4.), ..row(4.) });
        let custom = custom_maps.iter().filter_map(|map| {
            material_painter_graph::CUSTOM_MAP_VALUE.checked_add(usize::try_from(map.id).ok()?)
        });
        for value in (0..material_painter_graph::MESH_MAPS.len()).chain(custom) {
            button(commands, choices, font, label(value),
                PaintAction::GraphEdit(material_graph::Edit::SetMeshMap(target, value)));
        }
    }
}

fn graph_bitmap(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    path: Option<&std::path::Path>,
    images: &texture_fill::GraphImages,
    target: material_graph::ParameterTarget,
) {
    let name = path.map_or_else(
        || "Без изображения".into(),
        |path| {
            path.to_str()
                .and_then(|key| images.get(key))
                .map(|bitmap| bitmap.name.clone())
                .or_else(|| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .unwrap_or_default()
        },
    );
    text(commands, parent, font, &name, 11., MUTED);
    let tools = node(commands, parent, row(4.));
    button(
        commands,
        tools,
        font,
        "Выбрать изображение…",
        PaintAction::GraphBitmapPick(target),
    );
    if path.is_some() {
        icon(
            commands,
            tools,
            icons::Glyph::Trash,
            "Убрать изображение",
            PaintAction::GraphEdit(material_graph::Edit::ClearBitmap(target)),
        );
    }
}

fn graph_value(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    name: &str,
    kind: &material_painter_graph::ParamKind,
    value: &material_painter_graph::Value,
    action: impl Fn(usize, i8) -> PaintAction,
) {
    use material_painter_graph::{ParamKind, Value};
    let adjustable = material_graph::adjustable(value);
    let components = match value {
        Value::Color(rgb) => rgb
            .iter()
            .zip(["R", "G", "B"])
            .map(|(v, name)| format!("{name} {v:.2}"))
            .collect(),
        Value::Float(v) => vec![format!("{v:.2}")],
        Value::Int(v) => vec![v.to_string()],
        Value::Bool(v) => vec![if *v { "Вкл." } else { "Выкл." }.into()],
        Value::Enum(i) => match kind {
            ParamKind::Enum(options) => vec![options.get(*i).cloned().unwrap_or_default()],
            _ => return,
        },
        Value::Image(_) => return,
        Value::MeshMap(_) => return,
    };
    for (component, value) in components.into_iter().enumerate() {
        let line = node(commands, parent, row(5.));
        let label = if name.is_empty() {
            value
        } else {
            format!("{name} · {value}")
        };
        text(commands, line, font, &label, 11., MUTED);
        if adjustable {
            for (label, direction) in [("−", -1), ("+", 1)] {
                button(commands, line, font, label, action(component, direction));
            }
        }
    }
}

fn graph_label(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    index: usize,
    group: bool,
    value: &str,
) {
    let title = if group { "Группа" } else { "Имя" };
    let line = node(commands, parent, row(4.));
    text(commands, line, font, title, 11., MUTED);
    commands.spawn((
        TextInput, EditableText::new(value), SelectAllOnFocus,
        crate::ui_theme::text_cursor_style(), TabIndex(0), GraphLabel(index, group),
        AccessibleLabel(format!("{title} параметра графа")),
        TextFont { font: font.clone().into(), font_size: FontSize::Px(12.), ..default() },
        TextColor(TEXT),
        Node {
            flex_grow: 1., min_width: px(0.), width: px(80.), height: px(27.),
            padding: UiRect::axes(px(7.), px(5.)), border: UiRect::all(px(1.)),
            overflow: Overflow::clip(), ..default()
        },
        BackgroundColor(WELL), BorderColor::all(EDGE), ChildOf(line),
    ));
    icon(commands, line, icons::Glyph::Check, &format!("Применить · {title}"),
        PaintAction::GraphEdit(material_graph::Edit::LabelParameter(index, group)));
}

#[derive(Component)]
struct ExportTextSubmit(PaintAction);

fn export_text_input(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    field: impl Component,
    value: &str,
    action: PaintAction,
) {
    text(commands, parent, font, label, 11., MUTED);
    let line = node(commands, parent, row(4.));
    commands.spawn((
        TextInput, EditableText::new(value), SelectAllOnFocus, TabIndex(0), field,
        ExportTextSubmit(action),
        AccessibleLabel(label.into()), crate::ui_theme::text_cursor_style(),
        TextFont { font: font.clone().into(), font_size: FontSize::Px(12.), ..default() },
        TextColor(TEXT),
        Node {
            flex_grow: 1., min_width: px(0.), height: px(27.),
            padding: UiRect::axes(px(7.), px(5.)), border: UiRect::all(px(1.)),
            overflow: Overflow::clip(), ..default()
        },
        BackgroundColor(WELL), BorderColor::all(EDGE), ChildOf(line),
    )).observe(export_text_key);
    icon(commands, line, icons::Glyph::Check, &format!("Применить · {label}"), action);
}

fn export_text_key(
    mut event: On<FocusedInput<KeyboardInput>>,
    fields: Query<&ExportTextSubmit>,
    mut actions: MessageWriter<Action>,
) {
    if event.input.state == ButtonState::Pressed
        && !event.input.repeat
        && matches!(event.input.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
        && let Ok(submit) = fields.get(event.focused_entity)
    {
        actions.write(Action::ModelView(ModelViewAction::Paint(submit.0)));
        event.propagate(false);
    }
}

fn layer_name(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    model: usize,
    session: &Session,
    layer: &Layer,
) {
    let line = node(commands, parent, Node { width: percent(100), ..row(4.) });
    let field = commands.spawn((
        TextInput,
        EditableText::new(&layer.name),
        SelectAllOnFocus,
        crate::ui_theme::text_cursor_style(),
        TabIndex(0),
        LayerNameField {
            model,
            binding: session.binding,
            layer: session.stack.selected,
            original: layer.name.clone(),
        },
        AccessibleLabel("Имя выбранного слоя".into()),
        TextFont { font: font.clone().into(), font_size: FontSize::Px(12.), ..default() },
        TextColor(TEXT),
        Node {
            flex_grow: 1.,
            min_width: px(0.),
            height: px(29.),
            padding: UiRect::axes(px(7.), px(5.)),
            border: UiRect::all(px(1.)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(WELL),
        BorderColor::all(EDGE),
        ChildOf(line),
    )).observe(layer_name_key).id();
    icon(commands, line, icons::Glyph::Check, "Применить имя слоя · Enter", PaintAction::RenameLayer(field));
}

fn layer_name_key(
    mut event: On<FocusedInput<KeyboardInput>>,
    mut fields: Query<(&LayerNameField, &mut EditableText)>,
    mut actions: MessageWriter<Action>,
) {
    if event.input.state != ButtonState::Pressed || event.input.repeat {
        return;
    }
    let target = event.focused_entity;
    match event.input.key_code {
        KeyCode::Enter | KeyCode::NumpadEnter => {
            actions.write(Action::ModelView(ModelViewAction::Paint(PaintAction::RenameLayer(target))));
            event.propagate(false);
        }
        KeyCode::Escape if let Ok((field, mut text)) = fields.get_mut(target) => {
            text.editor_mut().set_text(&field.original);
            event.propagate(false);
        }
        _ => {}
    }
}

fn export_recipe_editor(
    commands: &mut Commands, parent: Entity, font: &Handle<Font>,
    prefs: &export::prefs::Store, session: &Session,
) {
    use export::recipe::{Edit, Layout, Source};
    let controls = node(commands, parent, Node {
        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
    });
    button(commands, controls, font, "Новый шаблон", PaintAction::NewExportTemplate);
    button(commands, controls, font, "Копировать шаблон", PaintAction::CopyExportTemplate);
    button(commands, controls, font, "Сохранить JSON…", PaintAction::SaveExportTemplate);
    let export::Template::Custom(index) = session.export_settings.template else { return };
    let Some(recipe) = prefs.recipe(index) else { return };
    icon(commands, controls, icons::Glyph::Trash, "Удалить шаблон", PaintAction::DeleteExportTemplate(index));
    let output = session.export_output.min(recipe.maps.len().saturating_sub(1));
    let channel = recipe.maps.get(output).map_or(0, |map| session.export_component.min(map.layout.len() - 1));
    let edit = |edit| PaintAction::EditExportTemplate { recipe: index, output, channel, edit };
    export_text_input(commands, parent, font, "Имя шаблона",
        export::RecipeName { recipe: index, output: None }, &recipe.name, edit(Edit::RenameTemplate));
    text(commands, parent, font, "Выходные карты", 11., MUTED);
    let outputs = node(commands, parent, Node {
        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
    });
    for (i, item) in recipe.maps.iter().enumerate() {
        button(commands, outputs, font, &item.name, PaintAction::ExportOutput(i));
    }
    let operations = node(commands, parent, row(4.));
    if recipe.maps.len() < 32 {
        button(commands, operations, font, "+ Карта", edit(Edit::Add));
    }
    let Some(map) = recipe.maps.get(output) else { return };
    icon(commands, operations, icons::Glyph::Trash, "Убрать карту из шаблона", edit(Edit::Remove));
    if output > 0 { button(commands, operations, font, "↑", edit(Edit::Move(true))); }
    if output + 1 < recipe.maps.len() { button(commands, operations, font, "↓", edit(Edit::Move(false))); }
    export_text_input(commands, parent, font, "Имя файла",
        export::RecipeName { recipe: index, output: Some(output) }, &map.name, edit(Edit::RenameOutput));
    text(commands, parent, font, "Переменные $mesh, $set, $project, $udim", 11., MUTED);
    let formats = node(commands, parent, row(4.));
    for layout in Layout::ALL {
        button(commands, formats, font, layout.label(), edit(Edit::SetLayout(layout)));
    }
    button(commands, formats, font, "16 бит", edit(Edit::ToggleDepth));
    let channels = node(commands, parent, row(4.));
    for (position, name) in ["R", "G", "B", "A"].into_iter().take(map.layout.len()).enumerate() {
        button(commands, channels, font, name, PaintAction::ExportComponent(position));
    }
    let component = map.channels[channel];
    button(commands, parent, font, &format!("{} ⌄", component.src.label()), PaintAction::ExportSourceMenu);
    if session.export_source_menu {
        let sources = node(commands, parent, Node {
            flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
        });
        for &(source, label) in Source::ALL {
            button(commands, sources, font, label, edit(Edit::SetSource(source)));
        }
    }
    let values = node(commands, parent, row(4.));
    if component.src.has_components() {
        for (i, label) in ["Из R", "Из G", "Из B"].into_iter().enumerate() {
            button(commands, values, font, label, edit(Edit::SetComponent(i as u8)));
        }
    }
    button(commands, values, font, "Инверсия", edit(Edit::ToggleInvert));
}

fn sphere(color: [u8; 3], roughness: f32, metal: f32) -> Image {
    shelf_image(presets::sphere(color, roughness, metal))
}

fn shelf_image(preview: image::RgbaImage) -> Image {
    Image::from_dynamic(
        image::DynamicImage::ImageRgba8(preview),
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
    mut fonts: ResMut<Assets<Font>>,
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
    )).with_child((
        engine::BrushCursorCore,
        Node {
            position_type: PositionType::Absolute,
            left: percent(25),
            top: percent(25),
            width: percent(50),
            height: percent(50),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BorderColor::all(Color::srgba(0.98, 0.88, 0.66, 0.45)),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
    commands.spawn((
        engine::BrushCursorLabel,
        Text::new(""),
        super::super::super::font(font, 11.),
        TextColor(TEXT),
        TextLayout::no_wrap(),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(px(6), px(2)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.06, 0.06, 0.07, 0.82)),
        Visibility::Hidden,
        ZIndex(91),
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
    // Brush values stay on the left; document commands are pushed to the right edge
    // so the bar reads as "tool options … file actions" instead of one long strip.
    node(&mut commands, toolbar, Node { flex_grow: 1., min_width: px(8), ..default() });
    button(&mut commands, toolbar, font, "Редактор графов", PaintAction::GraphPage(GraphPage::Home));
    button(
        &mut commands, toolbar, font, "Экспорт карт…", PaintAction::ExportMaps(true),
    );
    toolbar_divider(&mut commands, toolbar);
    button(&mut commands, toolbar, font, "Сохранить как…", PaintAction::SaveCopy);
    super::super::super::button(
        &mut commands,
        toolbar,
        font,
        "Сохранить",
        Action::ModelView(ModelViewAction::Edit(editing::EditAction::Save)),
        Look::Primary,
    );
    let path_options = surface(
        &mut commands,
        viewer.overlay,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(TOOLBAR_BOTTOM),
            left: px(44),
            right: px(0),
            height: px(PATH_TOOLBAR_HEIGHT),
            padding: UiRect::horizontal(px(10)),
            border: UiRect::bottom(px(1)),
            ..row(4.)
        },
        PANEL,
    );
    commands.entity(path_options).insert(ZIndex(40));
    let path_title = text(&mut commands, path_options, font, "Контур", 12., FOCUS);
    commands.entity(path_title).entry::<Node>().and_modify(|mut node| node.margin.right = px(6));
    for (label, action) in [
        ("Сгладить", PaintAction::PathSmooth),
        ("Замкнуть", PaintAction::PathClosed),
        ("По направлению", PaintAction::PathAlign),
        ("Применить", PaintAction::PathApply),
        ("Очистить", PaintAction::PathClear),
    ] {
        button(&mut commands, path_options, font, label, action);
    }
    number(&mut commands, path_options, font, Parameter::Spacing, true);
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
    let reset_view = icon(&mut commands, views, icons::Glyph::Refresh, "Вернуть стандартную раскладку: 3D + 2D, слои, материалы", PaintAction::Reset);
    commands.entity(reset_view).entry::<Node>().and_modify(|mut node| {
        node.margin.right = px(6);
    });
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
            top: px(TOOLBAR_BOTTOM),
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
    let pane = || Node {
        min_width: px(0),
        min_height: px(0),
        flex_basis: px(0),
        flex_grow: 1.,
        align_self: AlignSelf::Stretch,
        overflow: Overflow::clip(),
        ..column(0.)
    };
    let workspace = surface(
        &mut commands,
        viewer.content,
        Node { display: Display::None, ..pane() },
        PANEL,
    );
    commands.entity(workspace).insert(AccessibleLabel("Рабочая область рисования".into()));
    let model_pane = node(&mut commands, workspace, pane());
    // Statistics sit in the 3D view just above its caption instead of floating over the
    // properties panel.
    let diagnostics_panel = surface(
        &mut commands,
        model_pane,
        Node {
            position_type: PositionType::Absolute,
            left: px(8.),
            bottom: px(58.),
            max_width: percent(100),
            padding: UiRect::axes(px(6.), px(3.)),
            border_radius: BorderRadius::all(px(4.)),
            ..row(0.)
        },
        CHROME,
    );
    commands.entity(diagnostics_panel).insert((Pickable::IGNORE, ZIndex(5)));
    let diagnostic_label = text(&mut commands, diagnostics_panel, font, "", 11., MUTED);
    commands.entity(diagnostic_label).insert((Pickable::IGNORE, AccessibleLabel("Статистика рисования".into())));
    let three_title = surface(
        &mut commands,
        model_pane,
        Node {
            display: Display::None,
            height: px(30),
            width: percent(100),
            flex_shrink: 0.,
            padding: UiRect::horizontal(px(10)),
            ..row(8.)
        },
        CHROME,
    );
    let three_label = text(&mut commands, three_title, font, "", 12., TEXT);
    let uv = surface(
        &mut commands,
        workspace,
        Node {
            display: Display::None,
            ..pane()
        },
        WELL,
    );
    commands
        .entity(uv)
        .insert(AccessibleLabel("Рисование: 2D-вид UV".into()));
    // One tidy header row instead of three stacked lines of text above the canvas.
    let uv_header = surface(
        &mut commands,
        uv,
        Node {
            width: percent(100),
            min_height: px(32),
            flex_shrink: 0.,
            padding: UiRect::axes(px(10), px(3)),
            border: UiRect::bottom(px(1)),
            flex_wrap: FlexWrap::Wrap,
            ..row(8.)
        },
        CHROME,
    );
    let uv_info = text(&mut commands, uv_header, font, "UV · 0–1", 11., MUTED);
    commands.entity(uv_info).insert(TextLayout::default());
    commands.entity(uv_info).entry::<Node>().and_modify(|mut node| {
        node.flex_grow = 1.;
        node.min_width = px(0);
    });
    let uv_hint = text(&mut commands, uv_header, font, "Колесо — масштаб · СКМ — панорама · F — вписать", 11., MUTED);
    commands.entity(uv_hint).insert(TextColor(MUTED.with_alpha(0.7)));
    super::super::super::button(
        &mut commands,
        uv_header,
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
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            overflow: Overflow::clip(),
            ..default()
        },
    );
    commands.entity(uv_view).insert(engine::TextureViewport);
    register_paint_drop(&mut commands, *canvas, PaintDropZone::View3d);
    register_paint_drop(&mut commands, uv_view, PaintDropZone::ViewUv);
    let uv_image = images.add(Image::from_dynamic(
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(64, 64, |x, y| {
            let value = if (x / 4 + y / 4) % 2 == 0 { 45 } else { 54 };
            image::Rgba([value, value, value + 7, 255])
        })),
        true,
        RenderAssetUsages::default(),
    ));
    let uv_canvas = commands.spawn((
        ImageNode::new(uv_image.clone()),
        engine::TextureCanvas,
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(uv_view),
    )).id();
    let overlay = |commands: &mut Commands| {
        commands.spawn((
            ImageNode::default(),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(uv_canvas),
        )).id()
    };
    let uv_wire = overlay(&mut commands);
    commands.entity(uv_wire).insert(UvWire);
    let uv_seams = overlay(&mut commands);
    commands.entity(uv_seams).insert((UvSeams2d, ZIndex(1)));
    let shelf_home = surface(
        &mut commands,
        interface.root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            bottom: px(FOOT - SHELF_HEIGHT),
            height: px(SHELF_HEIGHT),
            overflow: Overflow::clip(),
            padding: UiRect::top(px(drawer::GRIP_HEIGHT)),
            border: UiRect::top(px(1)),
            ..column(0.)
        },
        PANEL,
    );
    commands.entity(shelf_home).insert(ZIndex(11));
    drawer::library_resize_grip(&mut commands, shelf_home);
    let shelf_root = node(&mut commands, shelf_home, Node {
        width: percent(100), min_width: px(0), min_height: px(0), flex_grow: 1., ..column(0.)
    });
    commands.entity(shelf_root).insert(AccessibleLabel("Рисование: библиотека ресурсов".into()));
    let heading = surface(&mut commands, shelf_root, Node {
        width: percent(100), min_height: px(30), flex_shrink: 0.,
        padding: UiRect::horizontal(px(10)), border: UiRect::bottom(px(1)), ..row(6.)
    }, CHROME);
    let shelf_handle = text(&mut commands, heading, font, "Библиотека", 12., FOCUS);
    commands.entity(shelf_handle).insert(Pickable::default());
    control_hints::attach(&mut commands, shelf_handle, "Перетащите заголовок в правую панель, чтобы разместить библиотеку рядом с другими вкладками.");
    let search = surface(
        &mut commands, heading,
        Node {
            width: px(235),
            min_width: px(92),
            height: px(24),
            margin: UiRect::left(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            padding: UiRect::horizontal(px(6)),
            ..row(5.)
        },
        WELL,
    );
    icons::spawn(&mut commands, search, icons::Glyph::Search, 13., MUTED);
    let input_area = node(&mut commands, search, Node {
        flex_grow: 1.,
        min_width: px(0),
        height: percent(100),
        ..default()
    });
    let shelf_input = commands.spawn((
        ShelfSearch, TextInput, EditableText::new(""), crate::ui_theme::text_cursor_style(),
        AccessibleLabel("Поиск ресурсов библиотеки".into()), super::super::super::font(font, 11.),
        TextColor(TEXT), TextLayout::no_wrap(), TabIndex(10),
        Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::vertical(px(3)),
            overflow: Overflow::clip(),
            ..default()
        },
        ChildOf(input_area),
    )).id();
    let shelf_hint = text(&mut commands, input_area, font, "Поиск ресурсов…", 11., MUTED);
    commands.entity(shelf_hint).insert(Node {
        position_type: PositionType::Absolute,
        top: px(3),
        ..default()
    });
    let clear = icon(&mut commands, search, icons::Glyph::Close, "Очистить поиск ресурсов", PaintAction::ShelfClearSearch);
    commands.entity(clear).entry::<Node>().and_modify(|mut node| {
        node.width = px(22);
        node.height = px(22);
    });
    let card_sizes = node(&mut commands, heading, row(2.));
    for (label, name, increase) in [
        ("−", "Уменьшить карточки библиотеки", false),
        ("+", "Увеличить карточки библиотеки", true),
    ] {
        let control = button(&mut commands, card_sizes, font, label, PaintAction::ResizeShelf(increase));
        commands.entity(control).insert(AccessibleLabel(name.into()));
    }
    let shelf_types = node(&mut commands, heading, Node {
        height: px(24),
        min_width: px(0),
        position_type: PositionType::Relative,
        ..row(0.)
    });
    let types = button(&mut commands, shelf_types, font, "", PaintAction::ShelfTypeMenu);
    commands.entity(types).insert(AccessibleLabel("Тип материала".into()));
    commands.entity(types).entry::<Node>().and_modify(|mut node| {
        node.height = percent(100);
    });
    let shelf_type_label = text(&mut commands, types, font, "Все типы", 11., TEXT);
    icons::spawn(&mut commands, types, icons::Glyph::ChevronDown, 12., MUTED);
    let shelf_type_menu = surface(
        &mut commands, shelf_types,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            bottom: px(28),
            width: px(185),
            padding: UiRect::all(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..column(2.)
        },
        CHROME,
    );
    commands.entity(shelf_type_menu).insert((ZIndex(70), Scroller));
    for kind in std::iter::once(None).chain(MaterialType::ALL.into_iter().map(Some)) {
        let item = button(
            &mut commands, shelf_type_menu, font,
            kind.map_or("Все типы", MaterialType::label),
            PaintAction::ShelfType(kind),
        );
        commands.entity(item).entry::<Node>().and_modify(|mut node| {
            node.width = percent(100);
            node.height = px(23);
        });
    }
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
    let mut category_buttons = Vec::new();
    for (name, category) in [
        ("Материалы", Category::Materials),
        ("Онлайн", Category::Online),
        ("Скачанные", Category::Downloaded),
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
            n.height = px(21);
        });
        category_buttons.push(e);
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
    commands.entity(categories).insert(HorizontalScroller);
    let stacks = std::array::from_fn(|_| {
        let stack = node(&mut commands, workspace, Node {
            width: percent(100), min_width: px(0), min_height: px(0), flex_grow: 1.,
            overflow: Overflow::scroll_y(), padding: UiRect::all(px(7)), ..column(5.)
        });
        commands.entity(stack).insert(Scroller);
        stack
    });
    let properties = surface(
        &mut commands,
        workspace,
        Node {
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.,
            border: UiRect::top(px(1)),
            ..column(0.)
        },
        CHROME,
    );
    let shelf_home_preview = commands.spawn((
        Node { display: Display::None, ..default() },
        BackgroundColor(FOCUS.with_alpha(0.22)), BorderColor::all(FOCUS),
        Pickable::IGNORE, ZIndex(90), ChildOf(viewer.overlay),
    )).id();
    sidebar::setup(&mut commands, workspace, stacks, properties, sidebar::ShelfDock {
        home: shelf_home, handle: shelf_handle, body: shelf_root,
        overlay: viewer.overlay, preview: shelf_home_preview,
    }, [model_pane, uv]);
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
    number(&mut commands, body, font, Parameter::LayerOpacity, false);
    let fill_options = node(&mut commands, body, column(5.));
    let fill_content = section(&mut commands, fill_options, font, "Заливка", true);
    text(&mut commands, fill_content, font, "Область заливки", 11., MUTED);
    let fill_modes = node(
        &mut commands,
        fill_content,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3.),
            ..row(4.)
        },
    );
    for (label, mode) in FillMode::OPTIONS {
        button(&mut commands, fill_modes, font, label, PaintAction::FillMode(mode));
    }
    let fill_flags = node(
        &mut commands,
        fill_content,
        Node { flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.) },
    );
    button(&mut commands, fill_flags, font, "Лассо · Shift", PaintAction::FillLasso);
    button(&mut commands, fill_flags, font, "Сквозь модель", PaintAction::FillThrough);
    button(&mut commands, fill_flags, font, "Стирать · Ctrl", PaintAction::FillErase);
    number(&mut commands, fill_content, font, Parameter::FillPadding, false);
    let fill_angle = node(&mut commands, fill_content, column(0.));
    number(&mut commands, fill_angle, font, Parameter::FillAngle, false);

    // Colour and material come first: it is what people change most often while painting.
    let color_section = node(&mut commands, body, column(5.));
    let color = section(&mut commands, color_section, font, "Цвет и материал", true);
    let palette = node(
        &mut commands,
        color,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(4.),
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
        number(&mut commands, color, font, parameter, false);
    }
    text(&mut commands, color, font, "Канал", 11., MUTED);
    let channel = node(
        &mut commands,
        color,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3.),
            ..row(3.)
        },
    );
    for (i, name) in CHANNELS.iter().enumerate() {
        button(&mut commands, channel, font, name, PaintAction::Channel(i));
    }
    button(&mut commands, color, font, "Несколько каналов одним мазком", PaintAction::MultiChannelBrush);
    let multi_brush = node(&mut commands, color, Node { display: Display::None, ..column(4.) });
    let targets = node(&mut commands, multi_brush, Node {
        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
    });
    for (index, name) in CHANNELS.iter().enumerate() {
        button(&mut commands, targets, font, name, PaintAction::BrushChannel(index));
    }
    let brush_values = [
        (3, &[Parameter::BrushHeight][..]),
        (4, &[Parameter::BrushAlpha][..]),
        (5, &[Parameter::BrushEmission(0), Parameter::BrushEmission(1), Parameter::BrushEmission(2)][..]),
        (7, &[Parameter::BrushSpecular][..]),
    ].map(|(channel, fields)| {
        let controls = node(&mut commands, color, Node { display: Display::None, ..column(4.) });
        for &field in fields {
            number(&mut commands, controls, font, field, false);
        }
        (channel, controls)
    });

    // Size, hardness, flow and opacity live in the top bar; only the rest is repeated here.
    let brush_section = node(&mut commands, body, column(5.));
    let brush = section(&mut commands, brush_section, font, "Кисть", true);
    let brush_name = text(&mut commands, brush, font, "Кисть · круглая", 13., TEXT);
    for parameter in [Parameter::Spacing, Parameter::TipAngle, Parameter::Stabilizer] {
        number(&mut commands, brush, font, parameter, false);
    }
    let pen = node(&mut commands, brush, row(4.));
    button(&mut commands, pen, font, "Нажим · размер", PaintAction::PressureSize);
    button(&mut commands, pen, font, "Нажим · поток", PaintAction::PressureFlow);

    let scatter_section = node(&mut commands, body, column(5.));
    let scatter = section(&mut commands, scatter_section, font, "Разброс и зерно", false);
    for parameter in [
        Parameter::BrushScatter,
        Parameter::BrushSizeJitter,
        Parameter::BrushAngleJitter,
        Parameter::GrainStrength,
        Parameter::GrainScale,
        Parameter::GrainAngle,
    ] {
        number(&mut commands, scatter, font, parameter, false);
    }

    let projection_section = node(&mut commands, body, column(5.));
    let projection_body = section(&mut commands, projection_section, font, "Проекция и симметрия", false);
    let symmetry = node(&mut commands, projection_body, row(4.));
    text(&mut commands, symmetry, font, "Симметрия", 12., MUTED);
    for (label, axis) in [
        ("Выкл.", Symmetry::Off),
        ("X", Symmetry::X),
        ("Y", Symmetry::Y),
        ("Z", Symmetry::Z),
    ] {
        button(&mut commands, symmetry, font, label, PaintAction::Symmetry(axis));
    }
    text(&mut commands, projection_body, font, "Проекция кисти", 11., MUTED);
    let projection = node(
        &mut commands,
        projection_body,
        Node {
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(3.),
            ..row(4.)
        },
    );
    for (label, mode) in [
        ("Экран", BrushProjection::Screen),
        ("Поверхность", BrushProjection::Surface),
        ("UV", BrushProjection::Uv),
    ] {
        button(&mut commands, projection, font, label, PaintAction::BrushProjection(mode));
    }
    button(&mut commands, projection_body, font, "Сквозь геометрию", PaintAction::PaintThrough);
    number(&mut commands, projection_body, font, Parameter::BrushAngleLimit, false);

    let settings_section = node(&mut commands, body, column(5.));
    let settings = section(&mut commands, settings_section, font, "Стартовые настройки и история", false);
    let defaults = node(
        &mut commands,
        settings,
        Node { flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.) },
    );
    button(&mut commands, defaults, font, "Сохранить стартовые", PaintAction::SaveBrushDefaults);
    button(&mut commands, defaults, font, "Вернуть стандартные", PaintAction::ResetBrushDefaults);
    history_number(&mut commands, settings, font, "Шагов отмены на набор", HistoryPreference::Undo,
        "Введите число от 8 до 512 и нажмите Enter. Меньше шагов занимают меньше памяти.");
    button(&mut commands, settings, font, "Автокопии", PaintAction::ToggleRecovery);
    history_number(&mut commands, settings, font, "Правок между автокопиями", HistoryPreference::Recovery,
        "Введите число от 1 до 500 и нажмите Enter. После паузы редактор сохраняет накопленные правки.");
    let tool_sections = [
        (color_section, ToolMask::ALL & !ToolMask::PICKER),
        (brush_section, ToolMask::BRUSH | ToolMask::ERASER | ToolMask::PATH),
        (scatter_section, ToolMask::BRUSH | ToolMask::ERASER),
        (projection_section, ToolMask::BRUSH | ToolMask::ERASER | ToolMask::PATH),
    ];
    let swatches = MATERIALS
        .iter()
        .map(|preset| images.add(sphere(preset.rgb(), preset.roughness * 100., preset.metallic * 100.)))
        .collect();
    let smart_swatches = SMART_MATERIALS.iter().map(|preset| {
        let (color, roughness, metallic) = preset.preview();
        images.add(sphere(color, roughness * 100., metallic * 100.))
    }).collect();
    let generator_swatches = GeneratorKind::ALL.iter().map(|&kind| {
        images.add(Image::from_dynamic(
            image::DynamicImage::ImageRgba8(Generator::new(kind).shelf_preview(64)),
            true,
            RenderAssetUsages::default(),
        ))
    }).collect();
    let graph = graph_canvas::Canvas::new(&mut commands, viewer.overlay, font);
    shader_maker::setup(&mut commands, viewer.overlay, font, &mut fonts);
    commands.insert_resource(PaintUi {
        library_open: false,
        library_motion: DrawerMotion::default(),
        graph,
        roots: vec![toolbar, views, tool_rail],
        original,
        hidden: vec![],
        canvas: *canvas,
        canvas_parent: viewer.content,
        graph_settings_stamp: None,
        model_pane,
        workspace,
        side: interface.side,
        workspace_parent: viewer.content,
        uv_image,
        uv_canvas,
        uv_wire,
        uv_seams,
        uv_info,
        content: viewer.content,
        stacks,
        layer_thumbs: layer_thumbs::Cache::new(&mut images),
        online_previews: online::Previews::default(),
        shelf,
        shelf_layout: ShelfLayout {
            home: shelf_home, root: shelf_root, handle: shelf_handle, header: heading,
            search, body: shelf_body, categories, category_buttons, docked: false,
        },
        shelf_input,
        shelf_hint,
        shelf_types,
        shelf_type_menu,
        shelf_type_label,
        shelf_stamp: None,
        user_swatches: HashMap::new(),
        graph_swatches: HashMap::new(),
        requested_swatches: Default::default(),
        shelf_preview_job: None,
        layer_name,
        brush_name,
        diagnostic_label,
        diagnostic_meter: diagnostics::Meter::default(),
        fill_options,
        fill_angle,
        path_options,
        multi_brush,
        brush_values,
        tool_sections,
        three_title,
        three_label,
        stamps: [None; 5],
        stack_context: [None; 5],
        texture_value: None,
        bake_option_field: None,
        preview_field: None,
        uv_overlay: uv_overlay::Overlay::default(),
        swatches,
        smart_swatches,
        generator_swatches,
        showing: false,
    });
}

pub(in crate::editor::ui::model_viewer) fn animate_library(
    mut actions: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    mut ui: ResMut<PaintUi>,
    mut sidebar: ResMut<sidebar::State>,
    mut drawer: ResMut<DrawerMotion>,
    size: Res<DrawerSize>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut focus: ResMut<InputFocus>,
    parents: Query<&ChildOf>,
    mut nodes: Query<&mut Node>,
    time: Res<Time>,
    mut redraw: MessageWriter<bevy::window::RequestRedraw>,
) {
    let showing = editor.viewing_image.is_none()
        && editor.pending.is_none()
        && editing.mode(editor.viewing_model) == Mode::Paint;
    let was_open = ui.library_open;
    let toggle = actions.read().filter(|action| matches!(action, Action::TogglePaintLibrary)).count() % 2 == 1;
    if showing && toggle && !editor.browser_popup && !editor.hdri_library_open {
        if sidebar.shelf_in_sidebar() {
            sidebar.undock_shelf();
            ui.library_open = true;
        } else {
            ui.library_open = !ui.library_open;
        }
        if ui.library_open {
            editor.drawer = false;
        }
    } else if !showing || editor.drawer {
        ui.library_open = false;
    }
    if was_open && !ui.library_open && !sidebar.shelf_in_sidebar()
        && focus.get().is_some_and(|entity| parents.iter_ancestors(entity).any(|parent| parent == ui.shelf_layout.root))
    {
        focus.clear();
    }
    let docked = sidebar.shelf_in_sidebar();
    let default_height = SHELF_HEIGHT + (paint.shelf_size.pixels as f32 - ShelfSize::DEFAULT as f32).max(0.0);
    let height = size.library_height(window.height(), default_height);
    let previous = ui.library_motion.visible_height;
    if showing && !docked {
        let open = ui.library_open;
        ui.library_motion.advance(open, time.delta_secs(), height);
    } else {
        ui.library_motion = DrawerMotion::default();
    }
    if drawer.paint_shelf_height != ui.library_motion.visible_height {
        drawer.paint_shelf_height = ui.library_motion.visible_height;
    }
    let settled = ui.library_open && ui.library_motion.fully_open();
    if drawer.paint_shelf_settled != settled {
        drawer.paint_shelf_settled = settled;
    }
    if previous != drawer.paint_shelf_height || was_open != ui.library_open {
        redraw.write(bevy::window::RequestRedraw);
    }
    if let Ok(panel) = nodes.get_mut(ui.shelf_layout.home) {
        ui.library_motion.position_panel(panel, height);
    }
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
    mut fields: ParamSet<(
        Query<(Entity, &Value, &mut EditableText, &mut BorderColor)>,
        Query<&mut EditableText, With<ShelfSearch>>,
        Query<&LayerNameField>,
        Query<(Entity, &HistoryPreference, &mut EditableText, &mut BorderColor)>,
        Query<(Entity, &PreviewValue, &mut EditableText, &mut BorderColor)>,
    )>,
    buttons: Query<
        (
            Entity,
            &PaintButton,
            &BackgroundColor,
            &BorderColor,
            Option<&bevy::ui::InteractionDisabled>,
        ),
        (Without<Value>, Without<HistoryPreference>, Without<PreviewValue>),
    >,
    mut images: ResMut<Assets<Image>>,
    rendering: (Res<engine::Runtime>, Res<ShaderMaker>, Res<environment::Settings>, Res<export::prefs::Store>, Res<space_mouse::State>, Res<sidebar::State>, Query<&ChildOf>, Query<&sidebar::PanelBody>, Query<&GraphLabel>, Res<editing::recovery::Preferences>, Res<ButtonInput<KeyCode>>, Res<engine::gpu::Queue>, Res<Time<bevy::time::Real>>, Query<(&LayerMemoryHint, &mut Help)>),
    (baked, mut preview_edits, export_activity): (Res<bake::State>, MessageWriter<Action>, Res<export::Activity>),
    mut texture_nodes: Query<&mut ImageNode, Or<(With<engine::TextureCanvas>, With<UvWire>, With<UvSeams2d>)>>,
) {
    let (runtime, maker, environment_settings, export_preferences, space_mouse, sidebar, parents, pane_bodies, graph_labels, recovery_preferences, keys, gpu_queue, time, mut layer_hints) = rendering;
    let doc = editor
        .viewing_model
        .and_then(|index| editing.documents.get(&index));
    let showing = doc.is_some_and(|doc| doc.mode == Mode::Paint)
        && editor.viewing_image.is_none()
        && editor.pending.is_none();
    let shader_workspace = showing && editor.viewing_model.and_then(|i| paint.sessions.get(&i))
        .is_some_and(|session| session.panel == Panel::Shader);
    let graph_workspace = showing && !editor.browser_popup && !editor.hdri_library_open
        && editor.viewing_model.and_then(|i| paint.sessions.get(&i)).is_some_and(Session::graph_canvas_open);
    let custom_workspace = shader_workspace || graph_workspace;
    let show_path_options = showing && !custom_workspace && editor.viewing_model
        .and_then(|i| paint.sessions.get(&i))
        .is_some_and(|session| session.tool == PaintTool::Path);
    let canvas_parent = if graph_workspace {
        ui.graph.preview
    } else if shader_workspace && maker.shows_model() {
        maker.preview_pane()
    } else if showing && !custom_workspace {
        ui.model_pane
    } else {
        ui.content
    };
    if ui.canvas_parent != canvas_parent {
        commands.entity(ui.canvas).insert(ChildOf(canvas_parent));
        ui.canvas_parent = canvas_parent;
    }
    let workspace_parent = if custom_workspace { ui.side } else { ui.content };
    if ui.workspace_parent != workspace_parent {
        commands.entity(ui.workspace).insert(ChildOf(workspace_parent));
        ui.workspace_parent = workspace_parent;
    }
    if let Ok(mut node) = nodes.get_mut(ui.workspace) {
        let display = if showing { Display::Flex } else { Display::None };
        let (position, left) = if custom_workspace {
            (PositionType::Absolute, px(6.))
        } else {
            (PositionType::Relative, px(0.))
        };
        if (node.display, node.position_type, node.left) != (display, position, left) {
            (node.display, node.position_type, node.left) = (display, position, left);
            (node.top, node.right, node.bottom) = (px(0), px(0), px(0));
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.canvas) {
        let display = if shader_workspace && !graph_workspace && !maker.shows_model() {
            Display::None
        } else {
            Display::Flex
        };
        let height = if canvas_parent == ui.content { percent(100) } else { Val::Auto };
        if (node.display, node.height) != (display, height) {
            (node.display, node.height) = (display, height);
        }
    }
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
    let shelf_docked = sidebar.shelf_in_sidebar();
    if showing && ui.library_open && !shelf_docked {
        commands.entity(interface.paint_library).try_insert(BackgroundColor(ACTIVE));
    }
    let shelf_size = paint.shelf_size.pixels as f32;
    let shelf_visible = if shelf_docked { sidebar.shelf_tab_visible() } else { ui.library_motion.visible_height > 0.0 };
    if ui.shelf_layout.docked != shelf_docked {
        ui.shelf_stamp = None;
    }
    let (menu, types, cards) = (ui.shelf_type_menu, ui.shelf_types, ui.shelf);
    ui.shelf_layout.arrange(shelf_docked, menu, types, cards, &mut nodes);
    if let Ok(mut root) = nodes.get_mut(ui.shelf_layout.root) {
        let display = if showing && shelf_visible {
            Display::Flex
        } else {
            Display::None
        };
        if root.display != display {
            root.display = display;
        }
    }
    for &entity in &ui.roots {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if showing {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.path_options) {
        let display = if show_path_options { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.content) {
        let padding = if showing {
            UiRect {
                left: px(44),
                top: px(TOOLBAR_BOTTOM + if show_path_options { PATH_TOOLBAR_HEIGHT } else { 0. }),
                bottom: px(0.),
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
    if !showing {
        ui.diagnostic_meter = diagnostics::Meter::default();
        ui.stamps = [None; 5];
        ui.stack_context = [None; 5];
        ui.graph_settings_stamp = None;
        ui.layer_thumbs.begin_list();
        ui.layer_thumbs.end_list();
        ui.online_previews.clear();
        ui.online_previews.update(false, &mut images);
        ui.shelf_stamp = None;
        paint.shelf.menu = false;
        ui.texture_value = None;
        ui.bake_option_field = None;
        ui.preview_field = None;
        ui.uv_overlay = uv_overlay::Overlay::default();
        return;
    }
    let index = editor.viewing_model.unwrap();
    let doc = doc.unwrap();
    let paint = &mut *paint;
    paint.activate(index, recovery_preferences.settings().undo_steps);
    let session = paint.sessions.get_mut(&index).unwrap();
    session.refresh_folder_view();
    if let Some(ms) = ui.diagnostic_meter.sample((index, session.set, session.texture_size), time.delta_secs_f64())
        && let Ok(mut label) = labels.get_mut(ui.diagnostic_label)
    {
        let triangles = ui.diagnostic_meter.triangles(index, doc);
        let value = diagnostics::label(index, session, &runtime, &gpu_queue, &images, ms, triangles);
        if label.0 != value {
            label.0 = value;
        }
        let bytes = runtime.layer_gpu_bytes(index, session, &gpu_queue);
        for (layer, mut hint) in &mut layer_hints {
            let value = layer_memory_hint(bytes.get(layer.0).copied().unwrap_or_default());
            if hint.0 != value {
                hint.0 = value;
            }
        }
    }
    if let Ok(mut label) = labels.get_mut(ui.three_label) {
        let current = if let Some(channel) = session.diagnostic_channel() {
            CHANNELS[channel]
        } else if let Some(map) = session.diagnostic_bake() {
            match map {
                bake::BakeMap::Custom(id) => doc.custom_bakes.iter().find(|recipe| recipe.id == id)
                    .map_or_else(|| map.label(), |recipe| recipe.name.as_str()),
                _ => map.label(),
            }
        } else {
            ""
        };
        if let Ok(mut node) = nodes.get_mut(ui.three_title) {
            let display = if current.is_empty() { Display::None } else { Display::Flex };
            if node.display != display {
                node.display = display;
            }
        }
        if label.0 != current {
            label.0 = current.to_owned();
        }
    }
    let shelf_filter = &mut paint.shelf;
    if let Ok(mut input) = fields.p1().get_mut(ui.shelf_input) {
        if std::mem::take(&mut shelf_filter.clear) {
            input.editor_mut().set_text("");
        }
        let value = input.value().to_string();
        shelf_filter.set_query(&value);
        if let Ok(mut hint) = nodes.get_mut(ui.shelf_hint) {
            let display = if value.is_empty() && focus.get() != Some(ui.shelf_input) {
                Display::Flex
            } else {
                Display::None
            };
            if hint.display != display {
                hint.display = display;
            }
        }
    }
    let material_category = matches!(session.category, Category::Materials | Category::Smart);
    for (entity, visible) in [
        (ui.shelf_types, material_category),
        (ui.shelf_type_menu, material_category && shelf_filter.menu),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if visible { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let Ok(mut label) = labels.get_mut(ui.shelf_type_label) {
        let value = shelf_filter.material.map_or("Все типы", MaterialType::label);
        if label.0 != value {
            label.0 = value.into();
        }
    }
    for (entity, visible) in [
        (ui.fill_options, session.tool == PaintTool::Fill),
        (ui.fill_angle, session.fill_mode == FillMode::Flat),
        (ui.multi_brush, session.multi_channel_brush && !session.editing_mask),
    ].into_iter().chain(ui.tool_sections.map(|(entity, tools)| {
        (entity, tools.contains(session.tool))
    })).chain(ui.brush_values.map(|(channel, entity)| {
        let active = !session.editing_mask && session.tool != PaintTool::Eraser
            && (session.stroke_channels() & (1 << channel)) != 0;
        (entity, active)
    })) {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if visible { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let Some((model, entity, control)) = ui.bake_option_field
        && (model != index || focus.get() != Some(entity))
    {
        ui.bake_option_field = None;
        if model == index && !session.bake_busy && !keys.just_pressed(KeyCode::Escape)
            && let Ok((_, _, input, _)) = fields.p0().get_mut(entity)
            && let Ok(value) = input.value().to_string().replace(',', ".").parse::<f32>()
        {
            let (min, max) = control.bounds();
            if (min as f32..=max as f32).contains(&value)
                && value != control.value(&session.bake_projection)
            {
                session.bake_option_edit = Some((control, value));
            }
        }
    }
    let mut editing_texture_value = false;
    let mut editing_bake_option = false;
    for (entity, parameter, mut input, mut border) in &mut fields.p0() {
        if focus.get() == Some(entity) {
            let texture = parameter.0.fill_parameter();
            editing_texture_value |= texture;
            let ray = match parameter.0 {
                Parameter::BakeOption(control) => Some(control),
                _ => None,
            };
            editing_bake_option |= ray.is_some();
            if let Some(control) = ray {
                ui.bake_option_field = Some((index, entity, control));
            }
            let parsed = input.value().to_string().replace(',', ".").parse::<f32>();
            let (min, max) = parameter.0.bounds();
            match parsed {
                Ok(value) if value.is_finite() && (min as f32..=max as f32).contains(&value) => {
                    if texture {
                        ui.texture_value = Some(TextureValue {
                            model: index, binding: session.binding, layer: session.stack.selected,
                            parameter: parameter.0, value,
                        });
                    } else if ray.is_none() && session.value(parameter.0) != value {
                        session.set_value(parameter.0, value);
                    }
                    border.set_if_neq(BorderColor::all(FOCUS));
                }
                _ => {
                    if texture {
                        ui.texture_value = None;
                    }
                    border.set_if_neq(BorderColor::all(ERROR));
                }
            }
        } else {
            let digits = match parameter.0 {
                Parameter::BakeOption(control) => control.digits(),
                parameter if texture_decimal(parameter) => 2,
                _ => 0,
            };
            let value = format!("{:.digits$}", session.value(parameter.0));
            if input.value().to_string() != value {
                input.editor_mut().set_text(&value);
            }
            border.set_if_neq(BorderColor::all(EDGE));
        }
    }
    if !editing_texture_value && !session.busy && let Some(edit) = ui.texture_value.take() {
        if edit.model == index && edit.binding == session.binding && edit.layer == session.stack.selected {
            session.set_value(edit.parameter, edit.value);
        }
    }
    let navigation = paint.navigation.settings;
    if let Some((model, entity)) = ui.preview_field
        && (model != index || focus.get() != Some(entity))
    {
        ui.preview_field = None;
        if model == index && !keys.just_pressed(KeyCode::Escape)
            && let Ok((_, field, input, _)) = fields.p4().get_mut(entity)
            && let Some(value) = field.parse(&input)
            && value != field.display_value(doc, navigation)
        {
            preview_edits.write(Action::ModelView(ModelViewAction::Paint(
                PaintAction::PreviewMaterial(PreviewEdit::Value { model, parameter: field.0, value }),
            )));
        }
    }
    for (entity, field, mut input, mut border) in &mut fields.p4() {
        if focus.get() == Some(entity) {
            ui.preview_field = Some((index, entity));
            let valid = field.parse(&input).is_some();
            border.set_if_neq(BorderColor::all(if valid { FOCUS } else { ERROR }));
        } else {
            let value = format!("{:.2}", field.display_value(doc, navigation));
            if input.value().to_string() != value {
                input.editor_mut().set_text(&value);
            }
            border.set_if_neq(BorderColor::all(EDGE));
        }
    }
    let recovery = recovery_preferences.settings();
    for (entity, &preference, mut input, mut border) in &mut fields.p3() {
        if focus.get() == Some(entity) {
            let valid = preference.parse(&input).is_some();
            border.set_if_neq(BorderColor::all(if valid { FOCUS } else { ERROR }));
        } else {
            let saved = match preference {
                HistoryPreference::Recovery => recovery.edits.to_string(),
                HistoryPreference::Undo => recovery.undo_steps.to_string(),
            };
            if input.value().to_string() != saved {
                input.editor_mut().set_text(&saved);
            }
            border.set_if_neq(BorderColor::all(EDGE));
        }
    }
    let visible_panels = sidebar.visible_panels();
    let use_pane_context = !visible_panels[sidebar::panel_index(session.panel)]
        || visible_panels.into_iter().filter(|&visible| visible).count() > 1;
    for (entity, button, background, border, disabled) in &buttons {
        let current = session.panel;
        if use_pane_context
            && let Some(panel) = parents.iter_ancestors(entity).find_map(|parent| pane_bodies.get(parent).ok())
        {
            session.panel = panel.0;
        }
        let available = match button.0 {
            PaintAction::PreviewMaterial(PreviewEdit::InvertScrollZoom | PreviewEdit::ResetNavigation) => true,
            PaintAction::PreviewMaterial(_) => doc.can_store_paint_preview(),
            PaintAction::Undo | PaintAction::Redo if session.editing_shader().is_none() => {
                !session.busy && session.resize_request.is_none() && !session.smart_loading
                    && session.history_step_set(doc, button.0).is_some()
            }
            PaintAction::ExportMaps(_) => {
                export_preferences.template_has_maps() && !export_activity.exporting() && session.available(button.0)
            }
            PaintAction::ExportMesh => {
                !export_activity.exporting() && session.available(button.0)
            }
            action => session.available(action),
        };
        if available && disabled.is_some() {
            commands
                .entity(entity)
                .try_remove::<bevy::ui::InteractionDisabled>();
        } else if !available && disabled.is_none() {
            commands
                .entity(entity)
                .try_insert(bevy::ui::InteractionDisabled);
        }
        let active = match button.0 {
            PaintAction::Panel(panel) => visible_panels[sidebar::panel_index(panel)],
            PaintAction::ShelfType(kind) => shelf_filter.material == kind,
            PaintAction::ShelfTypeMenu => shelf_filter.menu,
            PaintAction::ToggleRecovery => recovery.enabled,
            PaintAction::PreviewMaterial(PreviewEdit::AutomaticHeight) => doc.paint_preview.height_strength.is_none(),
            PaintAction::PreviewMaterial(PreviewEdit::InvertScrollZoom) => navigation.invert_zoom,
            PaintAction::PreviewMaterial(PreviewEdit::BakedNormal) => doc.paint_preview.use_baked_normal,
            PaintAction::PreviewMaterial(PreviewEdit::UvWire) => doc.paint_uv.show_wire,
            PaintAction::PreviewMaterial(PreviewEdit::UvSeams2d) => doc.paint_uv.show_seams_2d,
            PaintAction::PreviewMaterial(PreviewEdit::UvSeams3d) => doc.paint_uv.show_seams_3d,
            PaintAction::EditExportTemplate { recipe, output, channel, edit } => {
                use export::recipe::Edit;
                export_preferences.recipe(recipe).and_then(|recipe| recipe.maps.get(output))
                    .is_some_and(|map| match edit {
                        Edit::SetLayout(layout) => map.layout == layout,
                        Edit::ToggleDepth => map.force_16,
                        _ => map.channels.get(channel).is_some_and(|channel| match edit {
                            Edit::SetSource(source) => channel.src == source,
                            Edit::SetComponent(component) => channel.comp == component,
                            Edit::ToggleInvert => channel.invert,
                            _ => false,
                        }),
                    })
            }
            action => session.active(action),
        };
        session.panel = current;
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
        let value = if matches!(session.tool, PaintTool::Brush | PaintTool::Eraser) {
            let tip = session
                .tip
                .and_then(|i| paint.tips.entries.get(i))
                .map_or("Круглая", |tip| tip.name.as_str());
            format!("{} · {tip}", session.tool.label())
        } else {
            session.tool.label().into()
        };
        if text.0 != value {
            text.0 = value;
        }
    }
    let stamp_for = |panel| (
        index,
        session.revision,
        doc.revision,
        paint.tips.entries.len(),
        match panel {
            Panel::Shader => node_library::revision(),
            Panel::Sets => export_preferences.revision().wrapping_add(export_activity.revision),
            Panel::Preview => space_mouse.revision(),
            _ => 0,
        },
    );
    let editing_layer_name = focus.get().is_some_and(|entity| {
        fields.p2().get(entity).is_ok_and(|field| field.matches(index, session))
    });
    let shelf_content_stamp = stamp_for(session.panel);
    let panel_stamps = sidebar::PANELS.map(stamp_for);
    let inspector_stamp = (index, session.revision, doc.revision);
    if graph_workspace && session.graph_catalogue().is_none() {
        if ui.graph_settings_stamp != Some(inspector_stamp)
            && !focus.get().is_some_and(|field| graph_labels.get(field).is_ok())
            && let Some(graph) = session.graph()
        {
            let inspector = ui.graph.inspector;
            commands.entity(inspector).despawn_related::<Children>();
            let maps = session.binding.and_then(|binding| baked.maps(index, doc, binding, session.texture_size));
            graph_fields(&mut commands, inspector, &interface.font, session, graph, maps.map(Arc::as_ref), &doc.custom_bakes);
            ui.graph_settings_stamp = Some(inspector_stamp);
        }
    } else {
        ui.graph_settings_stamp = None;
    }
    for (i, panel) in sidebar::PANELS.into_iter().enumerate() {
        if !visible_panels[i] || ui.stamps[i] == Some(panel_stamps[i])
            || editing_texture_value || editing_layer_name || editing_bake_option || ui.preview_field.is_some()
        {
            continue;
        }
        let stack = ui.stacks[i];
        let context = (index, (panel == Panel::Layers).then_some(session.binding).flatten());
        if ui.stack_context[i] != Some(context) {
            commands.entity(stack).insert(ScrollPosition::default());
            ui.stack_context[i] = Some(context);
        }
        commands.entity(stack).despawn_related::<Children>();
        let focus_panel = session.panel;
        session.panel = panel;
        let font = &interface.font;
        match panel {
            Panel::Layers => {
                ui.layer_thumbs.begin_list();
                layer_set_selector(&mut commands, stack, font, doc, session);
                mesh_replacement(&mut commands, stack, font, doc, session);
                let tools = node(
                    &mut commands,
                    stack,
                    Node {
                        flex_wrap: FlexWrap::Wrap,
                        row_gap: px(4.),
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
                toolbar_divider(&mut commands, tools);
                icon(
                    &mut commands,
                    tools,
                    icons::Glyph::Copy,
                    "Дублировать слой",
                    PaintAction::Duplicate,
                );
                if session.stack.layers.get(session.stack.selected).is_some_and(|layer| layer.kind == LayerKind::Paint) {
                    icon(
                        &mut commands,
                        tools,
                        icons::Glyph::Eraser,
                        "Очистить слой краски",
                        PaintAction::ClearLayer,
                    );
                }
                icon(
                    &mut commands,
                    tools,
                    icons::Glyph::Trash,
                    "Удалить слой",
                    PaintAction::Remove,
                );
                toolbar_divider(&mut commands, tools);
                for (action, glyph, verb, shortcut) in [
                    (PaintAction::Undo, icons::Glyph::Undo, "Отменить", "Ctrl+Z"),
                    (PaintAction::Redo, icons::Glyph::Redo, "Повторить", "Ctrl+Shift+Z"),
                ] {
                    let hint = session.history_step_set(doc, action)
                        .and_then(|step| doc.paint_target(step.set).map(|(object, slot)| {
                            let (_, label) = step.label.names();
                            format!("{shortcut} · {verb} · {label} · {}", texture_set_label(doc, object, slot))
                        }));
                    let button = icon(&mut commands, tools, glyph, hint.as_deref().unwrap_or(verb), action);
                    if let Some(hint) = hint {
                        commands.entity(button).insert(Help(hint));
                    }
                }
                // Library/sharing commands are a separate, quieter row: they act on the whole
                // material stack rather than on the selected layer.
                let library = node(&mut commands, stack, Node {
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(4.),
                    ..row(3.)
                });
                text(&mut commands, library, font, "В библиотеку", 11., MUTED);
                icon(
                    &mut commands, library, icons::Glyph::Plus,
                    "Сохранить материал в библиотеке", PaintAction::SaveShelfMaterial,
                );
                icon(
                    &mut commands, library, icons::Glyph::Mask,
                    "Сохранить маску в библиотеке", PaintAction::SaveShelfMask,
                );
                icon(
                    &mut commands, library, icons::Glyph::ArrowUp,
                    "Экспортировать в .mpsmart", PaintAction::ExportShelfMaterial,
                );
                let zip = button(&mut commands, library, font, "ZIP", PaintAction::ExportShelfPackage);
                commands.entity(zip).insert(Help("Экспортировать материал в ZIP-пакет…".into()));
                let cc0 = button(&mut commands, library, font, "CC0", PaintAction::ShareCommunityMaterial);
                commands.entity(cc0).insert(Help("Подготовить пакет CC0 для сообщества…".into()));
                if let Some(layer) = session.stack.layers.get(session.stack.selected) {
                    layer_name(&mut commands, stack, font, index, session, layer);
                }
                let layer_bytes = runtime.layer_gpu_bytes(index, session, &gpu_queue);
                for (i, layer) in session.stack.layers.iter().enumerate().rev() {
                    if !session.stack.listed(i) {
                        continue;
                    }
                    let line = node(
                        &mut commands,
                        stack,
                        Node {
                            width: percent(100),
                            padding: UiRect::left(px(layer.depth as f32 * 9.)),
                            ..row(3.)
                        },
                    );
                    icon(
                        &mut commands,
                        line,
                        if session.stack.shown(i) {
                            icons::Glyph::Eye
                        } else {
                            icons::Glyph::EyeOff
                        },
                        "Видимость слоя",
                        PaintAction::Visible(i),
                    );
                    if layer.kind == LayerKind::Folder {
                        let (glyph, label) = if layer.collapsed {
                            (icons::Glyph::Folder, "Развернуть группу")
                        } else {
                            (icons::Glyph::FolderOpen, "Свернуть группу")
                        };
                        icon(&mut commands, line, glyph, label, PaintAction::CollapseFolder(i));
                    }
                    let kind = match layer.kind {
                        LayerKind::Paint => "Краска",
                        LayerKind::Fill => "Заливка",
                        LayerKind::Folder => "Группа",
                    };
                    let subtitle = if layer.kind == LayerKind::Folder {
                        let children = session.stack.block(i).len() - 1;
                        format!("{kind} · Слоёв {children} · {:.0}%", layer.opacity)
                    } else {
                        format!("{kind} · {} · {:.0}%", layer.blend.label(), layer.opacity)
                    };
                    let e = button(
                        &mut commands,
                        line,
                        font,
                        &format!("{}\n{subtitle}", layer.name),
                        PaintAction::Select(i),
                    );
                    commands.entity(e).insert((
                        LayerMemoryHint(i),
                        Help(layer_memory_hint(layer_bytes.get(i).copied().unwrap_or_default())),
                    ));
                    if layer.kind != LayerKind::Folder {
                        let thumb = layer_thumbs::Source::content(layer)
                            .map(|source| ui.layer_thumbs.thumbnail(source, &images));
                        let frame = layer_thumbnail(&mut commands, e, ui.layer_thumbs.checker.clone(), thumb);
                        commands.entity(e).insert_children(0, &[frame]);
                    }
                    let drop_zone = PaintDropZone::Layer {
                        index: i,
                        folder_open: (layer.kind == LayerKind::Folder).then_some(!layer.collapsed),
                    };
                    register_paint_drop(&mut commands, e, drop_zone);
                    commands.entity(e).entry::<Node>().and_modify(|mut n| {
                        n.height = px(43);
                        n.flex_grow = 1.;
                        n.min_width = px(0);
                    });
                    if let Some(mask) = &layer.mask {
                        let action = PaintAction::SelectLayerMask(i);
                        let mask_button = super::super::super::control(
                            &mut commands, line, Action::ModelView(ModelViewAction::Paint(action)),
                            Look::Regular,
                            Node { width: px(35), height: px(35), justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center, flex_shrink: 0.,
                                border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(5)), ..default() },
                        );
                        commands.entity(mask_button).insert((
                            PaintButton(action), AccessibleLabel(format!("Маска · {}", layer.name)),
                        ));
                        register_paint_drop(&mut commands, mask_button, drop_zone);
                        let thumb = ui.layer_thumbs.thumbnail(layer_thumbs::Source::Mask(mask.clone()), &images);
                        layer_thumbnail(&mut commands, mask_button, ui.layer_thumbs.checker.clone(), Some(thumb));
                    }
                }
                let move_row = node(&mut commands, stack, Node {
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(4.),
                    ..row(4.)
                });
                button(&mut commands, move_row, font, "Выше", PaintAction::Move(1));
                button(&mut commands, move_row, font, "Ниже", PaintAction::Move(-1));
                button(&mut commands, move_row, font, "Сгруппировать", PaintAction::GroupLayer);
                if session
                    .stack
                    .layers
                    .iter()
                    .any(|layer| layer.kind == LayerKind::Folder)
                {
                    button(
                        &mut commands,
                        move_row,
                        font,
                        "В группу",
                        PaintAction::FolderMenu,
                    );
                    if session.available(PaintAction::MoveOut) {
                        button(
                            &mut commands,
                            move_row,
                            font,
                            "Из группы",
                            PaintAction::MoveOut,
                        );
                    }
                    if session.folder_menu {
                        for (i, folder) in session.stack.layers.iter().enumerate().rev() {
                            if session.stack.can_move_into(i) {
                                button(
                                    &mut commands,
                                    stack,
                                    font,
                                    &folder.name,
                                    PaintAction::MoveInto(i),
                                );
                            }
                        }
                    }
                }
                if !session.graph_canvas_open() {
                    if let Some(graph) = session.graph() {
                        let maps = session.binding
                            .and_then(|binding| baked.maps(index, doc, binding, session.texture_size))
                            .map(Arc::as_ref);
                        graph_fields(&mut commands, stack, font, session, graph, maps, &doc.custom_bakes);
                    } else if folder_graph::is_group(&session.stack) && !session.editing_mask {
                        button(&mut commands, stack, font, "Открыть граф группы", PaintAction::GraphCanvas);
                    }
                }
                if session.stack.layers.get(session.stack.selected).is_some_and(|layer| layer.kind == LayerKind::Fill) {
                    text(&mut commands, stack, font, "Текстуры заливки", 12., TEXT);
                    fill_channel_slots(&mut commands, stack, font, session, &paint.tips.grains);
                    if let Some(fill) = session.active_texture_fill().filter(|fill| fill.images.iter().any(Option::is_some)) {
                        if fill.images[4].is_some() {
                            let sources = node(&mut commands, stack, Node {
                                flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
                            });
                            for source in texture_fill::OpacitySource::ALL {
                                button(&mut commands, sources, font, source.label(), PaintAction::FillTextureOpacity(source));
                            }
                        }
                        let projections = node(&mut commands, stack, Node {
                            flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
                        });
                        for projection in texture_fill::Projection::ALL {
                            button(&mut commands, projections, font, projection.label(), PaintAction::FillTextureProjection(projection));
                        }
                        for parameter in [Parameter::TextureTiling, Parameter::TextureRotation, Parameter::TextureOffsetU, Parameter::TextureOffsetV] {
                            number(&mut commands, stack, font, parameter, false);
                        }
                        if fill.settings.projection == texture_fill::Projection::Triplanar {
                            number(&mut commands, stack, font, Parameter::TextureSharpness, false);
                        }
                        if fill.images[6].is_some() {
                            number(&mut commands, stack, font, Parameter::TextureNormalStrength, false);
                        }
                        if fill.images[3].is_some() {
                            number(&mut commands, stack, font, Parameter::TextureHeightStrength, false);
                            number(&mut commands, stack, font, Parameter::TextureHeightOffset, false);
                        }
                        if fill.images[4].is_some() {
                            number(&mut commands, stack, font, Parameter::TextureOpacityStrength, false);
                        }
                        if fill.images[5].is_some() {
                            for channel in 0..3 {
                                number(&mut commands, stack, font, Parameter::TextureEmissionTint(channel), false);
                            }
                        }
                        if fill.images[7].is_some() {
                            number(&mut commands, stack, font, Parameter::TextureSpecularStrength, false);
                        }
                    }
                    button(&mut commands, stack, font, "Вариации заливки", PaintAction::FillVariationOptions);
                    if session.fill_variation_options {
                        for parameter in [
                            Parameter::FillColorVariation, Parameter::FillRoughVariation,
                            Parameter::FillHeightVariation, Parameter::FillVariationScale,
                            Parameter::FillVariationSeed, Parameter::FillPartVariation,
                        ] {
                            number(&mut commands, stack, font, parameter, false);
                        }
                    }
                }
                if session.stack.layers.get(session.stack.selected).is_some() {
                    let controls = node(
                        &mut commands,
                        stack,
                        Node {
                            flex_wrap: FlexWrap::Wrap,
                            row_gap: px(4.),
                            ..row(4.)
                        },
                    );
                    if session.has_mask() {
                        button(
                            &mut commands,
                            controls,
                            font,
                            if session.editing_mask {
                                "Маска · редактируется"
                            } else {
                                "Рисовать по маске"
                            },
                            PaintAction::EditMask,
                        );
                        button(
                            &mut commands,
                            controls,
                            font,
                            if session.show_mask {
                                "Вид UV · маска"
                            } else {
                                "Показать маску"
                            },
                            PaintAction::PreviewMask,
                        );
                        button(
                            &mut commands,
                            controls,
                            font,
                            "Удалить маску",
                            PaintAction::RemoveMask,
                        );
                        button(
                            &mut commands,
                            controls,
                            font,
                            "Фильтры",
                            PaintAction::FilterMenu,
                        );
                        button(
                            &mut commands,
                            controls,
                            font,
                            "Слои маски",
                            PaintAction::MaskLayerMenu,
                        );
                        if session.editing_mask {
                            for (label, value) in [("Чёрный", 0), ("Серый", 128), ("Белый", 255)]
                            {
                                button(
                                    &mut commands,
                                    controls,
                                    font,
                                    label,
                                    PaintAction::MaskValue(value),
                                );
                            }
                        }
                    } else {
                        button(
                            &mut commands,
                            controls,
                            font,
                            "Белая маска",
                            PaintAction::AddMask(true),
                        );
                        button(
                            &mut commands,
                            controls,
                            font,
                            "Чёрная маска",
                            PaintAction::AddMask(false),
                        );
                    }
                    if session.stack.layers[session.stack.selected].mask.as_ref()
                        .is_none_or(|mask| mask.graph.is_none())
                    {
                        button(&mut commands, controls, font, "Граф маски", PaintAction::AddGraphMask);
                    }
                }
                if session.has_mask() && session.mask_layer_menu {
                    text(
                        &mut commands,
                        stack,
                        font,
                        "Генератор основы",
                        11.,
                        MUTED,
                    );
                    text(
                        &mut commands,
                        stack,
                        font,
                        "Для износа, загрязнений и масок деталей запеките набор во вкладке «Карты».",
                        11.,
                        MUTED,
                    );
                    generator_controls(
                        &mut commands,
                        stack,
                        font,
                        None,
                        session.generator(None),
                        session.id_palette(),
                        session.disabled_generator_inputs(),
                        (session.open_generator_inputs == Some(None)).then_some(session.generator_input_picker),
                        &doc.custom_bakes,
                    );
                    text(
                        &mut commands,
                        stack,
                        font,
                        "Добавить слой маски",
                        11.,
                        MUTED,
                    );
                    let add = node(
                        &mut commands,
                        stack,
                        Node {
                            flex_wrap: FlexWrap::Wrap,
                            row_gap: px(4.),
                            ..row(4.)
                        },
                    );
                    button(
                        &mut commands,
                        add,
                        font,
                        "Заливка",
                        PaintAction::AddMaskLayer,
                    );
                    for kind in GeneratorKind::ALL {
                        button(
                            &mut commands,
                            add,
                            font,
                            kind.label(),
                            PaintAction::AddGeneratorLayer(kind),
                        );
                    }
                    for (i, layer) in session.mask_layers().iter().enumerate().rev() {
                        let tools = node(&mut commands, stack, row(4.));
                        let action = |edit| PaintAction::MaskLayer(i, edit);
                        button(
                            &mut commands,
                            tools,
                            font,
                            &format!(
                                "{} {}",
                                layer.generator.map_or("Заливка", |g| g.kind.label()),
                                i + 1
                            ),
                            action(MaskLayerEdit::Toggle),
                        );
                        icon(
                            &mut commands,
                            tools,
                            icons::Glyph::ChevronUp,
                            "Выше",
                            action(MaskLayerEdit::Move(1)),
                        );
                        icon(
                            &mut commands,
                            tools,
                            icons::Glyph::ChevronDown,
                            "Ниже",
                            action(MaskLayerEdit::Move(-1)),
                        );
                        icon(
                            &mut commands,
                            tools,
                            icons::Glyph::Trash,
                            "Удалить слой маски",
                            action(MaskLayerEdit::Remove),
                        );
                        button(
                            &mut commands,
                            stack,
                            font,
                            layer.blend.label(),
                            action(MaskLayerEdit::Blend),
                        );
                        if layer.generator.is_some() {
                            generator_controls(
                                &mut commands,
                                stack,
                                font,
                                Some(i),
                                layer.generator,
                                session.id_palette(),
                                session.disabled_generator_inputs(),
                                (session.open_generator_inputs == Some(Some(i))).then_some(session.generator_input_picker),
                                &doc.custom_bakes,
                            );
                        }
                        let values = [
                            (
                                "Серый",
                                layer.value,
                                MaskLayerEdit::Value(-1),
                                MaskLayerEdit::Value(1),
                            ),
                            (
                                "Непрозрачность",
                                layer.opacity,
                                MaskLayerEdit::Opacity(-1),
                                MaskLayerEdit::Opacity(1),
                            ),
                        ];
                        for (label, value, decrease, increase) in
                            &values[usize::from(layer.generator.is_some())..]
                        {
                            let line = node(&mut commands, stack, row(4.));
                            text(
                                &mut commands,
                                line,
                                font,
                                &format!("{label} · {value}%"),
                                11.,
                                MUTED,
                            );
                            button(&mut commands, line, font, "−", action(*decrease));
                            button(&mut commands, line, font, "+", action(*increase));
                        }
                    }
                }
                if session.has_mask() && session.filter_menu {
                    let tools = node(
                        &mut commands,
                        stack,
                        Node {
                            flex_wrap: FlexWrap::Wrap,
                            row_gap: px(4.),
                            ..row(4.)
                        },
                    );
                    for kind in FilterKind::ALL {
                        button(
                            &mut commands,
                            tools,
                            font,
                            kind.label(),
                            PaintAction::AddFilter(kind),
                        );
                    }
                    for (i, filter) in session.filters().iter().enumerate() {
                        let tools = node(&mut commands, stack, row(4.));
                        button(
                            &mut commands,
                            tools,
                            font,
                            filter.kind.label(),
                            PaintAction::ToggleFilter(i),
                        );
                        icon(
                            &mut commands,
                            tools,
                            icons::Glyph::ChevronUp,
                            "Выше",
                            PaintAction::MoveFilter(i, -1),
                        );
                        icon(
                            &mut commands,
                            tools,
                            icons::Glyph::ChevronDown,
                            "Ниже",
                            PaintAction::MoveFilter(i, 1),
                        );
                        icon(
                            &mut commands,
                            tools,
                            icons::Glyph::Trash,
                            "Удалить фильтр",
                            PaintAction::RemoveFilter(i),
                        );
                        for (slot, &(label, ..)) in filter.kind.controls().iter().enumerate() {
                            let line = node(&mut commands, stack, row(4.));
                            let value = filter.value(slot);
                            let display = match (filter.kind, slot) {
                                (FilterKind::Outline, 2) => match value.round() as u32 {
                                    1 => "Снаружи",
                                    2 => "Внутри",
                                    _ => "С обеих сторон",
                                }
                                .to_owned(),
                                (FilterKind::Threshold, 1) => match value as u32 {
                                    0 => "Больше",
                                    1 => "Больше или равно",
                                    2 => "Меньше",
                                    _ => "Меньше или равно",
                                }
                                .to_owned(),
                                (FilterKind::Outline, 5) | (FilterKind::HistogramScan, 2) => {
                                    if value != 0. { "Вкл." } else { "Выкл." }.to_owned()
                                }
                                _ => value.to_string(),
                            };
                            text(
                                &mut commands,
                                line,
                                font,
                                &format!("{label} · {display}"),
                                11.,
                                MUTED,
                            );
                            button(
                                &mut commands,
                                line,
                                font,
                                "−",
                                PaintAction::AdjustFilter(i, slot, -1),
                            );
                            button(
                                &mut commands,
                                line,
                                font,
                                "+",
                                PaintAction::AdjustFilter(i, slot, 1),
                            );
                        }
                    }
                }
                if let Some(layer) = session
                    .stack
                    .layers
                    .get(session.stack.selected)
                    .filter(|layer| layer.kind != LayerKind::Folder)
                {
                    button(
                        &mut commands,
                        stack,
                        font,
                        &format!("Смешивание · {}", layer.blend.label()),
                        PaintAction::BlendMenu,
                    );
                    if session.blend_menu {
                        let menu = node(
                            &mut commands,
                            stack,
                            Node {
                                flex_wrap: FlexWrap::Wrap,
                                row_gap: px(4),
                                ..row(4.)
                            },
                        );
                        for mode in Blend::ALL {
                            button(
                                &mut commands,
                                menu,
                                font,
                                mode.label(),
                                PaintAction::Blend(mode),
                            );
                        }
                    }
                }
            }
            Panel::Sets => {
                text(&mut commands, stack, font, "Наборы текстур", 13., TEXT);
                let mut i = 0;
                for object in &doc.objects {
                    for slot in 0..object.materials.len() {
                        let key = (object.id, slot);
                        let row = node(&mut commands, stack, Node {
                            width: percent(100.),
                            ..row(3.)
                        });
                        let shown = session.set_appearance.visibility.shown(key.0, key.1);
                        icon(
                            &mut commands, row,
                            if shown { icons::Glyph::Eye } else { icons::Glyph::EyeOff },
                            if shown { "Скрыть набор" } else { "Показать набор" },
                            PaintAction::SetHide(key.0, key.1),
                        );
                        icon(
                            &mut commands, row, icons::Glyph::Lightbulb, "Закрепить подсветку",
                            PaintAction::SetPin(key.0, key.1),
                        );
                        let e = button(
                            &mut commands,
                            row,
                            font,
                            &texture_set_label(doc, object, slot),
                            PaintAction::Set(i),
                        );
                        commands
                            .entity(e)
                            .insert(sets::SetRow(key))
                            .entry::<Node>()
                            .and_modify(|mut n| {
                                n.flex_grow = 1.;
                                n.min_width = px(0.);
                            });
                        i += 1;
                    }
                }
                text(&mut commands, stack, font, "Разрешение набора", 11., MUTED);
                let sizes = node(
                    &mut commands, stack,
                    Node { flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.) },
                );
                for size in crate::editor::model_paint_data::RESOLUTIONS {
                    button(&mut commands, sizes, font, &format!("{size}×{size}"), PaintAction::Resolution(size));
                }
                text(&mut commands, stack, font, "UV-отступ при экспорте", 11., MUTED);
                let modes = node(&mut commands, stack, Node {
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(3.),
                    ..row(4.)
                });
                for padding in export::Padding::ALL {
                    button(&mut commands, modes, font, padding.label(), PaintAction::ExportPadding(padding));
                }
                if session.export_settings.padding == export::Padding::Fixed {
                    number(&mut commands, stack, font, Parameter::ExportPaddingWidth, false);
                }
                let export_actions = node(&mut commands, stack, Node {
                    flex_wrap: FlexWrap::Wrap, row_gap: px(3.), flex_shrink: 0., ..row(4.)
                });
                button(&mut commands, export_actions, font, "Экспорт набора…", PaintAction::ExportMaps(false));
                button(
                    &mut commands, export_actions, font,
                    &format!("{} ⌄", export_preferences.template_label(session.export_settings.template)), PaintAction::ExportTemplateMenu,
                );
                button(
                    &mut commands, export_actions, font,
                    &format!("{} ⌄", session.export_settings.file.format.label()), PaintAction::ExportFileMenu,
                );
                match &export_activity.state {
                    export::ActivityState::Idle => {}
                    export::ActivityState::Exporting => {
                        text(&mut commands, stack, font, "Экспорт карт…", 12., MUTED);
                    }
                    export::ActivityState::Finished { message, folder, failed } => {
                        text(&mut commands, stack, font, message.as_str(), 12., if *failed { ERROR } else { TEXT });
                        if folder.is_some() {
                            button(&mut commands, stack, font, "Открыть папку с картами", PaintAction::RevealExportResult);
                        }
                    }
                }
                if session.export_template_menu {
                    let panel = node(&mut commands, stack, Node { flex_shrink: 0., ..column(5.) });
                    let templates = node(&mut commands, panel, Node {
                        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
                    });
                    for template in export::Template::ALL {
                        button(&mut commands, templates, font, template.label(), PaintAction::ExportTemplate(template));
                    }
                    for (template, label) in export_preferences.recipe_labels() {
                        button(&mut commands, templates, font, label, PaintAction::ExportTemplate(template));
                    }
                    button(&mut commands, panel, font, "Импорт шаблонов JSON…", PaintAction::ImportExportTemplate);
                    export_recipe_editor(&mut commands, panel, font, &export_preferences, session);
                }
                if session.export_file_menu {
                    let destination = export_preferences.destination()
                        .map(|path| path.to_string_lossy()).unwrap_or_default();
                    export_text_input(
                        &mut commands, stack, font, "Папка экспорта карт",
                        export::DestinationField, &destination, PaintAction::ApplyExportDestination,
                    );
                    button(&mut commands, stack, font, "Выбрать папку…", PaintAction::BrowseExportDestination);
                    if destination.is_empty() {
                        text(&mut commands, stack, font, "Папка выбирается при экспорте", 11., MUTED);
                    }
                    text(&mut commands, stack, font, "Формат файла", 11., MUTED);
                    let formats = node(&mut commands, stack, Node {
                        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
                    });
                    for format in export::Format::ALL {
                        button(&mut commands, formats, font, format.label(), PaintAction::ExportFormat(format));
                    }
                    if session.export_settings.file.format.depths().len() > 1 {
                        text(&mut commands, stack, font, "Глубина цвета", 11., MUTED);
                    }
                    let depths = node(&mut commands, stack, row(4.));
                    for &depth in session.export_settings.file.format.depths() {
                        button(&mut commands, depths, font, depth.label(), PaintAction::ExportDepth(depth));
                    }
                    text(&mut commands, stack, font, "Разрешение файлов", 11., MUTED);
                    let sizes = node(&mut commands, stack, Node {
                        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
                    });
                    button(&mut commands, sizes, font, "Как у набора", PaintAction::ExportSize(None));
                    for size in crate::editor::model_paint_data::RESOLUTIONS {
                        button(&mut commands, sizes, font, &size.to_string(), PaintAction::ExportSize(Some(size)));
                    }
                    text(&mut commands, stack, font, "Нормали", 11., MUTED);
                    button(&mut commands, stack, font, "Учитывать нормали high-poly", PaintAction::ExportBakedNormal);
                    number(&mut commands, stack, font, Parameter::ExportHeightStrength, false);
                    button(&mut commands, stack, font, "Открывать папку после экспорта", PaintAction::ExportRevealFolder);
                    export_text_input(
                        &mut commands, stack, font, "Имя модели · $mesh",
                        export::MeshExportName, export::texture_model_name(doc, session),
                        PaintAction::ApplyExportMeshName,
                    );
                    text(&mut commands, stack, font, "Предпросмотр имён файлов", 11., MUTED);
                    let scopes = node(&mut commands, stack, Node {
                        flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(4.)
                    });
                    button(&mut commands, scopes, font, "Текущий набор", PaintAction::ExportPreviewScope(false));
                    button(&mut commands, scopes, font, "Все наборы", PaintAction::ExportPreviewScope(true));
                    let recipe = match session.export_settings.template {
                        export::Template::Custom(id) => export_preferences.recipe(id),
                        _ => None,
                    };
                    let files = export::preview_files(doc, session, &editor.project.title, recipe, (&baked, index));
                    let has_next = files.len() > export::PREVIEW_PAGE_SIZE;
                    if files.is_empty() {
                        text(&mut commands, stack, font, "Нет файлов для предпросмотра", 11., MUTED);
                    }
                    for file in files.into_iter().take(export::PREVIEW_PAGE_SIZE) {
                        let optional = if file.conditional { " · если канал изменён" } else { "" };
                        let upscale = if file.size > file.source_size {
                            format!(" · увеличение с {0} × {0}", file.source_size)
                        } else {
                            String::new()
                        };
                        text(&mut commands, stack, font,
                            format!("{} · {} · {} × {} · {}{upscale}{optional}",
                                file.name, file.layout, file.size, file.size, file.depth.label()),
                            12., TEXT);
                    }
                    let page = session.export_preview_page;
                    if page > 0 || has_next {
                        let pages = node(&mut commands, stack, row(4.));
                        if page > 0 {
                            button(&mut commands, pages, font, "Назад", PaintAction::ExportPreviewPage(page - 1));
                        }
                        text(&mut commands, pages, font, format!("Страница {}", page + 1), 11., MUTED);
                        if has_next {
                            button(&mut commands, pages, font, "Далее", PaintAction::ExportPreviewPage(page + 1));
                        }
                    }
                }
                button(&mut commands, stack, font, "Экспорт сетки OBJ…", PaintAction::ExportMesh);
            }
            Panel::Maps => {
                text(&mut commands, stack, font, "Карты меша", 14., TEXT);
                if session.bake_busy {
                    button(&mut commands, stack, font, "Отменить запекание", PaintAction::BakeCancel);
                } else {
                    button(&mut commands, stack, font, "Запечь выбранный набор", PaintAction::BakeStart);
                }
                let presets = node(&mut commands, stack, Node {
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(3.),
                    ..row(4.)
                });
                for quality in bake::Quality::ALL {
                    button(&mut commands, presets, font, quality.label(), PaintAction::BakeQuality(quality));
                }
                button(&mut commands, stack, font, "Выбор карт для запекания", PaintAction::BakeMapOptions);
                if session.bake_map_options {
                    let options = node(&mut commands, stack, Node {
                        flex_wrap: FlexWrap::Wrap,
                        row_gap: px(3.),
                        ..row(4.)
                    });
                    for map in bake::BakeMap::OPTIONAL {
                        button(&mut commands, options, font, map.label(), PaintAction::BakeToggleMap(map));
                    }
                    button(&mut commands, options, font, "Все карты", PaintAction::BakeSelectAllMaps(true));
                    button(&mut commands, options, font, "Только основные", PaintAction::BakeSelectAllMaps(false));
                    text(&mut commands, stack, font, "Лучи запекания", 11., MUTED);
                    for control in bake::BakeControl::RAYS {
                        number(&mut commands, stack, font, Parameter::BakeOption(control), false);
                    }
                    button(&mut commands, stack, font, "Сбросить параметры лучей", PaintAction::BakeRayOptions(bake::RayOptions::default()));
                    text(&mut commands, stack, font, "Проекция, кривизна и рёбра", 11., MUTED);
                    button(&mut commands, stack, font, "Автодальность поиска рёбер", PaintAction::BakeAutoEdgeReach);
                    for control in bake::BakeControl::SURFACE {
                        if control != bake::BakeControl::EdgeReach || !session.bake_projection.surface.auto_edge_reach {
                            number(&mut commands, stack, font, Parameter::BakeOption(control), false);
                        }
                    }
                    button(&mut commands, stack, font, "Сбросить настройки кривизны и проекции", PaintAction::BakeResetSurface);
                    text(&mut commands, stack, font, "Источник карты ID", 11., MUTED);
                    let sources = node(&mut commands, stack, Node {
                        flex_wrap: FlexWrap::Wrap,
                        row_gap: px(3.),
                        ..row(4.)
                    });
                    for source in bake::IdSource::ALL {
                        button(&mut commands, sources, font, source.label(), PaintAction::BakeIdSource(source));
                    }
                    button(
                        &mut commands, stack, font,
                        if session.bake_cache_clearing { "Очистка кэша…" } else { "Очистить дисковый кэш" },
                        PaintAction::BakeClearDiskCache,
                    );
                }
                button(&mut commands, stack, font, "Выбрать high-poly…", PaintAction::BakePickHigh);
                button(&mut commands, stack, font, "Предпросмотр оболочки", PaintAction::BakeInspectCage);
                if session.bake_inspect_cage {
                    let label = text(&mut commands, stack, font, &session.bake_cage_status, 11., MUTED);
                    commands.entity(label).insert((TextLayout::default(), Node { max_width: percent(100.), min_width: px(0.), ..default() }));
                }
                button(&mut commands, stack, font, "Усреднять нормали лучей", PaintAction::BakeAverageNormals);
                if let Some(path) = &session.bake_projection.path {
                    let filename = path.file_name().unwrap_or_default().to_string_lossy();
                    text(&mut commands, stack, font, filename.as_ref(), 11., MUTED);
                    button(&mut commands, stack, font, "Убрать high-poly", PaintAction::BakeClearHigh);
                    if session.bake_projection.cage.is_none() {
                        button(&mut commands, stack, font, "Кисть оболочки", PaintAction::BakePaintCage);
                        if session.bake_paint_cage {
                            number(&mut commands, stack, font, Parameter::CageRadius, false);
                            number(&mut commands, stack, font, Parameter::CageStrength, false);
                            button(&mut commands, stack, font, "Сбросить ручную правку", PaintAction::BakeResetCagePaint);
                        }
                    }
                    if session.bake_inspect_cage {
                        let legend = "Зелёный — попадание · жёлтый — предел\nКрасный — промах · фиолетовый — чужая часть";
                        let label = text(&mut commands, stack, font, legend, 11., MUTED);
                        commands.entity(label).insert((TextLayout::default(), Node { max_width: percent(100.), min_width: px(0.), ..default() }));
                    }
                    button(&mut commands, stack, font, "Сопоставлять по именам", PaintAction::BakeMatchNames);
                    button(&mut commands, stack, font, "Заполнять промахи", PaintAction::BakeFillMisses);
                    button(&mut commands, stack, font, "Приоритет лицевых сторон", PaintAction::BakePreferFrontFaces);
                    button(&mut commands, stack, font, "Выбрать оболочку…", PaintAction::BakePickCage);
                    if let Some(path) = &session.bake_projection.cage {
                        let filename = path.file_name().unwrap_or_default().to_string_lossy();
                        text(&mut commands, stack, font, filename.as_ref(), 11., MUTED);
                        button(&mut commands, stack, font, "Убрать оболочку", PaintAction::BakeClearCage);
                    } else {
                        button(&mut commands, stack, font, "Подгонять оболочку", PaintAction::BakeAutoFitCage);
                    }
                }
                let has_source = session.bake_projection.path.is_some();
                for (title, front) in [("Снаружи", true), ("Изнутри", false)] {
                    if (front && has_source && session.bake_projection.cage.is_some()) || (!front && !has_source) {
                        continue;
                    }
                    text(&mut commands, stack, font, title, 11., MUTED);
                    let distances = node(&mut commands, stack, Node {
                        flex_wrap: FlexWrap::Wrap,
                        row_gap: px(3.),
                        ..row(4.)
                    });
                    for percent in [1, 5, 10, 25, 50] {
                        button(&mut commands, distances, font, &format!("{percent}%"), PaintAction::BakeRange { front, percent });
                    }
                }
                button(&mut commands, stack, font, "Текстура материала", PaintAction::BakePreview(None));
                button(&mut commands, stack, font, "Карты на модели", PaintAction::BakeShowOnModel);
                button(&mut commands, stack, font, "Материал с живыми масками", PaintAction::BakeLiveMasks);
                if session.bake_live_masks && session.has_mask() {
                    button(&mut commands, stack, font, "Итоговая маска в UV", PaintAction::PreviewMask);
                }
                if session.bake_ready
                    && let Some(map) = session.bake_view.filter(|map| map.supports_fix())
                    && let Some((object, slot)) = session.binding
                    && let Some(material) = doc.objects.iter().find(|item| item.id == object)
                        .and_then(|item| item.materials.get(slot))
                {
                    let active = session.active_bake_fix() == Some(map);
                    let line = node(&mut commands, stack, row(5.));
                    button(&mut commands, line, font, if active { "Завершить правку" } else { "Исправить кистью" }, PaintAction::BakeFixTool(map));
                    if material.bake_fixes.iter().any(|(key, _)| *key == map) {
                        button(&mut commands, line, font, "Очистить мазки", PaintAction::BakeClearFix(map));
                    }
                    if active {
                        let tools = node(&mut commands, stack, row(5.));
                        button(&mut commands, tools, font, "Кисть", PaintAction::Tool(PaintTool::Brush));
                        button(&mut commands, tools, font, "Восстановить", PaintAction::Tool(PaintTool::Eraser));
                        if bake::levels::supports(map) {
                            let value = session.bake_fix_value;
                            text(&mut commands, stack, font, &format!("Значение кисти · {value}%"), 11., MUTED);
                            let values = node(&mut commands, stack, row(5.));
                            for (label, target) in [("−", value.saturating_sub(1)), ("0%", 0), ("50%", 50), ("100%", 100), ("+", value.saturating_add(1).min(100))] {
                                button(&mut commands, values, font, label, PaintAction::BakeFixValue(target));
                            }
                        }
                    }
                }
                if session.bake_ready
                    && let Some(map) = session.bake_view.filter(|&map| bake::levels::supports(map))
                    && let Some((object, slot)) = session.binding
                    && let Some(material) = doc.objects.iter().find(|item| item.id == object)
                        .and_then(|item| item.materials.get(slot))
                {
                    use bake::levels::Field;
                    let levels = bake::levels::get(material, map);
                    title(&mut commands, stack, font, &format!("Уровни · {}", map.label()));
                    for (field, label, step) in [
                        (Field::Black, format!("Чёрная точка · {}%", levels.black), 1),
                        (Field::White, format!("Белая точка · {}%", levels.white), 1),
                        (Field::Gamma, format!("Гамма · {:.2}", levels.gamma as f32 / 100.), 10),
                    ] {
                        let line = node(&mut commands, stack, row(5.));
                        text(&mut commands, line, font, &label, 11., MUTED);
                        for (label, delta) in [("−", -step), ("+", step)] {
                            button(&mut commands, line, font, label, PaintAction::BakeAdjustLevels(map, field, delta));
                        }
                    }
                    button(&mut commands, stack, font, "Сбросить уровни", PaintAction::BakeResetLevels(map));
                }
                button(&mut commands, stack, font, "Живой предпросмотр", PaintAction::BakeLivePreview);
                for map in bake::BakeMap::ALL.into_iter().filter(|map| {
                    session.bake_projection.maps.contains(*map)
                        && (!map.requires_high_poly() || session.bake_projection.path.is_some())
                }) {
                    button(&mut commands, stack, font, map.label(), PaintAction::BakePreview(Some(map)));
                }
                title(&mut commands, stack, font, "Пользовательские карты");
                let add = node(&mut commands, stack, Node {
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: px(4.),
                    ..row(4.)
                });
                if doc.custom_bakes.len() < custom::MAX_MAPS {
                    for kind in custom::Kind::ALL {
                        button(&mut commands, add, font, kind.label(), PaintAction::BakeCustomAdd(kind));
                    }
                }
                for (position, recipe) in doc.custom_bakes.iter().enumerate() {
                    let id = recipe.id;
                    let card = surface(
                        &mut commands,
                        stack,
                        Node {
                            width: percent(100),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(4.),
                            padding: UiRect::all(px(5.)),
                            border_radius: BorderRadius::all(px(5.)),
                            ..default()
                        },
                        WELL,
                    );
                    let line = node(&mut commands, card, row(4.));
                    commands.spawn((
                        TextInput,
                        CustomMapName(id),
                        EditableText::new(&recipe.name),
                        SelectAllOnFocus,
                        crate::ui_theme::text_cursor_style(),
                        TabIndex(0),
                        AccessibleLabel(format!("Имя карты: {}", recipe.name)),
                        TextFont { font: font.clone().into(), font_size: FontSize::Px(12.), ..default() },
                        TextColor(TEXT),
                        Node {
                            flex_grow: 1.,
                            flex_basis: px(0.),
                            min_width: px(0.),
                            height: px(27.),
                            padding: UiRect::axes(px(7.), px(5.)),
                            border: UiRect::all(px(1.)),
                            border_radius: BorderRadius::all(px(5.)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(WELL),
                        BorderColor::all(EDGE),
                        ChildOf(line),
                    ));
                    icon(&mut commands, line, icons::Glyph::Check, "Применить имя карты · Enter",
                        PaintAction::BakeCustomRename(id));
                    let tools = node(&mut commands, card, row(4.));
                    icon(&mut commands, tools, icons::Glyph::Eye, "Показать карту в UV",
                        PaintAction::BakePreview(Some(bake::BakeMap::Custom(id))));
                    if recipe.kind.has_direction() || recipe.kind == custom::Kind::Point {
                        icon(&mut commands, tools, icons::Glyph::Move, "Настроить на модели",
                            PaintAction::BakeCustomGizmo(id));
                    }
                    if doc.custom_bakes.len() < custom::MAX_MAPS {
                        icon(&mut commands, tools, icons::Glyph::Copy, "Дублировать карту",
                            PaintAction::BakeCustomDuplicate(id));
                    }
                    if position > 0 {
                        icon(&mut commands, tools, icons::Glyph::ChevronUp, "Переместить карту вверх",
                            PaintAction::BakeCustomMove(id, -1));
                    }
                    if position + 1 < doc.custom_bakes.len() {
                        icon(&mut commands, tools, icons::Glyph::ChevronDown, "Переместить карту вниз",
                            PaintAction::BakeCustomMove(id, 1));
                    }
                    icon(&mut commands, tools, icons::Glyph::Trash, "Удалить карту",
                        PaintAction::BakeCustomRemove(id));
                    if session.bake_custom_gizmo == Some(id) && session.view != View::Two {
                        let hint = if recipe.kind == custom::Kind::Point {
                            "Укажите точку на модели · Esc — отмена"
                        } else {
                            "Потяните за маркер направления · Esc — отмена"
                        };
                        text(&mut commands, card, font, hint, 11., MUTED);
                    }
                    for &field in recipe.fields() {
                        let line = node(&mut commands, card, row(4.));
                        let value = recipe.value(field);
                        let value = if field.is_angle() { value.to_string() }
                            else { format!("{:.2}", f32::from(value) / 100.) };
                        text(&mut commands, line, font, &format!("{} · {value}", field.label()), 11., MUTED);
                        button(&mut commands, line, font, "−", PaintAction::BakeCustomAdjust(recipe.id, field, -1));
                        button(&mut commands, line, font, "+", PaintAction::BakeCustomAdjust(recipe.id, field, 1));
                    }
                }
            }
            Panel::Shader => {},
            Panel::Preview => {
                text(
                    &mut commands,
                    stack,
                    font,
                    "Предпросмотр материала",
                    14.,
                    TEXT,
                );
                text(
                    &mut commands,
                    stack,
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
                    button(&mut commands, stack, font, name, PaintAction::View(view));
                }
                if doc.can_store_paint_preview() {
                    text(&mut commands, stack, font, "Камера", 12., TEXT);
                    preview_number(&mut commands, stack, font, PreviewParameter::FieldOfView);
                    text(&mut commands, stack, font, "Карты материала", 12., TEXT);
                    button(&mut commands, stack, font, "Автоматический рельеф",
                        PaintAction::PreviewMaterial(PreviewEdit::AutomaticHeight));
                    button(&mut commands, stack, font, "Запечённые нормали",
                        PaintAction::PreviewMaterial(PreviewEdit::BakedNormal));
                    for parameter in [PreviewParameter::Height, PreviewParameter::Emission, PreviewParameter::Occlusion] {
                        preview_number(&mut commands, stack, font, parameter);
                    }
                    button(&mut commands, stack, font, "UV-сетка",
                        PaintAction::PreviewMaterial(PreviewEdit::UvWire));
                    super::super::super::button(&mut commands, stack, font, "Каркас 3D · W",
                        Action::ModelView(ModelViewAction::Edit(editing::EditAction::Wireframe)), Look::Regular);
                    preview_number(&mut commands, stack, font, PreviewParameter::UvWireOpacity);
                    button(&mut commands, stack, font, "UV-швы в 2D",
                        PaintAction::PreviewMaterial(PreviewEdit::UvSeams2d));
                    button(&mut commands, stack, font, "UV-швы на 3D-модели",
                        PaintAction::PreviewMaterial(PreviewEdit::UvSeams3d));
                }
                text(&mut commands, stack, font, "Навигация мышью", 12., TEXT);
                preview_number(&mut commands, stack, font, PreviewParameter::OrbitSpeed);
                preview_number(&mut commands, stack, font, PreviewParameter::ZoomSpeed);
                button(&mut commands, stack, font, "Инвертировать зум колесом",
                    PaintAction::PreviewMaterial(PreviewEdit::InvertScrollZoom));
                button(&mut commands, stack, font, "Сбросить навигацию мышью",
                    PaintAction::PreviewMaterial(PreviewEdit::ResetNavigation));
                space_mouse::controls(&mut commands, stack, font, &space_mouse);
                let title = if session.post_panel { "− Обработка изображения" } else { "+ Обработка изображения" };
                button(&mut commands, stack, font, title, PaintAction::PostPanel);
                if session.post_panel {
                    environment::paint_post_controls(&mut commands, stack, font, environment_settings.paint_post(Some(doc)));
                }
                if doc.can_store_paint_preview() {
                    button(&mut commands, stack, font, "Сбросить настройки предпросмотра",
                        PaintAction::PreviewMaterial(PreviewEdit::Reset { model: index }));
                }
                text(&mut commands, stack, font, "Шейдер 3D-вида", 12., TEXT);
                button(&mut commands, stack, font, "PBR сцены", PaintAction::PreviewShader(None));
                button(&mut commands, stack, font, "Трассировка света", PaintAction::PathTrace);
                if session.path_trace {
                    for (label, minus, plus) in [
                        (format!("Отсчётов · {}", session.trace_settings.samples), PaintAction::TraceSamples(-1), PaintAction::TraceSamples(1)),
                        (format!("Отражений · {}", session.trace_settings.bounces), PaintAction::TraceBounces(-1), PaintAction::TraceBounces(1)),
                    ] {
                        let line = node(&mut commands, stack, row(5.0));
                        text(&mut commands, line, font, &label, 12.0, TEXT);
                        button(&mut commands, line, font, "−", minus);
                        button(&mut commands, line, font, "+", plus);
                    }
                    button(&mut commands, stack, font, "Плоскость под моделью", PaintAction::TraceGround);
                }
                for (index, shader) in shader_preview::presets().iter().enumerate() {
                    button(&mut commands, stack, font, &shader.name, PaintAction::PreviewShader(Some(index)));
                }
                for (&index, shader) in session.shader_graphs.range(shader_preview::presets().len()..) {
                    button(&mut commands, stack, font, shader.name(), PaintAction::PreviewShader(Some(index)));
                }
                button(&mut commands, stack, font, "Открыть граф шейдинга…", PaintAction::ImportShaderGraph);
                if session.shader_import.is_some() {
                    text(&mut commands, stack, font, "Загрузка графа…", 11., MUTED);
                } else if let Some(error) = &session.shader_import_error {
                    text(&mut commands, stack, font, error, 12., Color::srgb(1., 0.64, 0.44));
                }
                if let Some(graph) = session.graph() {
                    if !session.graph_canvas_open() {
                        graph_fields(&mut commands, stack, font, session, graph, None, &[]);
                    }
                    button(&mut commands, stack, font, "Отменить правку шейдера", PaintAction::Undo);
                    button(&mut commands, stack, font, "Повторить правку шейдера", PaintAction::Redo);
                }
                text(&mut commands, stack, font, "Карты на 3D-модели", 12., MUTED);
                button(&mut commands, stack, font, "Материал", PaintAction::Preview3d(Preview3d::Material));
                for (i, name) in CHANNELS.iter().enumerate() {
                    button(&mut commands, stack, font, name, PaintAction::Preview3d(Preview3d::Channel(i)));
                }
                for &map in &session.preview_bakes {
                    let name = match map {
                        bake::BakeMap::Custom(id) => doc.custom_bakes.iter().find(|recipe| recipe.id == id)
                            .map_or_else(|| map.label(), |recipe| recipe.name.as_str()),
                        _ => map.label(),
                    };
                    button(&mut commands, stack, font, name, PaintAction::Preview3d(Preview3d::Baked(map)));
                }
            }
        }
        if panel == Panel::Layers {
            ui.layer_thumbs.end_list();
        }
        session.panel = focus_panel;
        ui.stamps[i] = Some(panel_stamps[i]);
    }
    ui.layer_thumbs.update(&mut images, !session.busy && !runtime.stroke_active());
    if let Some((material, result)) = ui.shelf_preview_job.as_mut()
        .and_then(|(material, task)| bevy::tasks::block_on(bevy::tasks::poll_once(task))
            .map(|result| (*material, result)))
    {
        ui.shelf_preview_job = None;
        let handle = match material {
            ShelfMaterial::User(i) => ui.user_swatches.get(&i),
            ShelfMaterial::Smart(i) => ui.smart_swatches.get(i),
            ShelfMaterial::Basic(_) | ShelfMaterial::Downloaded(_) | ShelfMaterial::Texture(_)
            | ShelfMaterial::Graph(_) | ShelfMaterial::CommunityGraph(_) => None,
        };
        if let Ok(preview) = result
            && let Some(mut target) = handle.and_then(|handle| images.get_mut(handle))
        {
            *target = shelf_image(preview);
        }
    }
    let shelf_stamp = (shelf_content_stamp, session.category, shelf_filter.revision,
        paint.library.revision.wrapping_add(paint.online.revision)
            .wrapping_add(paint.community.revision).wrapping_add(paint.downloaded.revision)
            .wrapping_add(paint.graph_shelf.revision), paint.shelf_size.pixels);
    if ui.shelf_stamp != Some(shelf_stamp) {
        if ui.shelf_stamp.is_none_or(|(previous, category, revision, _, _)| {
            previous.0 != index || category != session.category || revision != shelf_filter.revision
        }) {
            commands.entity(ui.shelf).insert(ScrollPosition::default());
        }
        commands.entity(ui.shelf).despawn_related::<Children>();
        let font = &interface.font;
        let mut shown = 0;
        let shelf = ui.shelf;
        ui.user_swatches.retain(|index, _| paint.library.entry(*index).is_some());
        ui.requested_swatches.retain(|material| match material {
            ShelfMaterial::User(index) => paint.library.entry(*index).is_some(),
            _ => true,
        });
        if !matches!(session.category, Category::Online | Category::Downloaded) {
            ui.online_previews.clear();
        }
        if session.category == Category::Smart
            && let Some(index) = shelf_filter.saved_material_menu
            && let Some(entry) = paint.library.entry(index)
        {
            let menu = node(&mut commands, shelf, column(4.));
            text(&mut commands, menu, font, &entry.name, 11., TEXT);
            button(&mut commands, menu, font, "Пакет CC0…", PaintAction::ShareSavedMaterial(index));
            button(&mut commands, menu, font, "Показать файл", PaintAction::ShowSavedMaterial(index));
            button(&mut commands, menu, font, "Удалить пресет", PaintAction::DeleteSavedMaterial(index));
            button(&mut commands, menu, font, "Закрыть", PaintAction::SavedMaterialMenu(None));
        }
        for (i, entry) in paint.library.entries().rev() {
            if !shelf_filter.matches(&entry.name) {
                continue;
            }
            let (action, label) = match (&entry.kind, session.category) {
                (presets::Kind::Material { metallic, .. }, Category::Smart)
                    if shelf_filter.allows_material(MaterialType::of(&entry.name, Some(*metallic))) =>
                {
                    (PaintAction::ApplyShelfMaterial(i), "Добавить сохранённый материал")
                }
                (presets::Kind::Mask(_), Category::Generators) => {
                    (PaintAction::ApplyShelfMask(i, None), "Заменить маску из библиотеки")
                }
                _ => continue,
            };
            let swatch = ui.user_swatches.entry(i).or_insert_with(|| {
                let preview = match &entry.kind {
                    presets::Kind::Material { color, roughness, metallic, .. } => {
                        entry.thumbnail.as_ref().map_or_else(
                            || sphere(*color, roughness * 100., metallic * 100.),
                            |image| shelf_image((**image).clone()),
                        )
                    }
                    presets::Kind::Mask(mask) => {
                        if let Some(image) = &entry.thumbnail {
                            shelf_image((**image).clone())
                        } else if let Some(generator) = mask.preview_generator() {
                            Image::from_dynamic(
                                image::DynamicImage::ImageRgba8(generator.shelf_preview(64)),
                                true, RenderAssetUsages::default(),
                            )
                        } else {
                            sphere([mask.brightness(); 3], 100., 0.)
                        }
                    }
                };
                images.add(preview)
            }).clone();
            let saved_material = matches!(action, PaintAction::ApplyShelfMaterial(_));
            let parent = if saved_material {
                node(&mut commands, shelf, Node {
                    width: px(shelf_size + 28.), height: px(shelf_size + 56.), flex_shrink: 0., ..default()
                })
            } else {
                shelf
            };
            let card = material_card(&mut commands, parent, font, &entry.name, swatch, action, shelf_size);
            text(&mut commands, card, font, "Мой пресет", 10., MUTED);
            commands.entity(card).insert(AccessibleLabel(format!("{label} · {}", entry.name)));
            if saved_material {
                let action = PaintAction::SavedMaterialMenu(Some(i));
                let menu = button(&mut commands, parent, font, "⋯", action);
                commands.entity(menu).insert(AccessibleLabel(format!("Действия · {}", entry.name)));
                commands.entity(menu).entry::<Node>().and_modify(|mut node| {
                    node.position_type = PositionType::Absolute;
                    node.right = px(5);
                    node.top = px(5);
                });
                commands.entity(card).observe(move |mut event: On<PointerClick>, mut actions: MessageWriter<Action>| {
                    if event.button == PointerButton::Secondary {
                        event.propagate(false);
                        actions.write(Action::ModelView(ModelViewAction::Paint(action)));
                    }
                });
            }
            shown += 1;
        }
        match session.category {
            Category::Online => {
                let online = &paint.online;
                let controls = node(&mut commands, shelf, column(4.));
                let providers = node(&mut commands, controls, row(4.));
                for provider in [online::Provider::PolyHaven, online::Provider::AmbientCg, online::Provider::Community] {
                    let name = if provider == online.provider { format!("✓ {}", provider.name()) } else { provider.name().into() };
                    button(&mut commands, providers, font, &name, PaintAction::OnlineProvider(provider));
                }
                button(&mut commands, controls, font, "Обновить каталог", PaintAction::OnlineRefresh);
                if online.provider != online::Provider::Community {
                    let resolutions = node(&mut commands, controls, row(4.));
                    for resolution in [1, 2, 4] {
                        let marker = if resolution == online.resolution() { "✓ " } else { "" };
                        button(&mut commands, resolutions, font, &format!("{marker}{resolution}K"), PaintAction::OnlineResolution(resolution));
                    }
                }
                if online.provider == online::Provider::PolyHaven {
                    text(&mut commands, controls, font, "Powered by Poly Haven", 10., MUTED);
                }
                let matching: Vec<_> = online.entries.iter().enumerate()
                    .filter(|(_, entry)| entry.matches(&shelf_filter.query)).collect();
                let page_count = matching.len().div_ceil(online::PAGE);
                let page = online.page.min(page_count.saturating_sub(1));
                if online.job.is_some() {
                    text(&mut commands, controls, font, "Загрузка каталога…", 12., MUTED);
                } else if let Some(error) = &online.error {
                    text(&mut commands, controls, font, error, 12., MUTED);
                } else if matching.is_empty() {
                    let message = if online.provider == online::Provider::Community {
                        if online.entries.is_empty() { "В каталоге сообщества пока нет ресурсов" }
                        else { "Ресурсы не найдены" }
                    } else { "Материалы не найдены" };
                    text(&mut commands, controls, font, message, 12., MUTED);
                }
                if session.smart_loading {
                    text(&mut commands, controls, font, "Загрузка текстур…", 12., MUTED);
                }
                if matches!(paint.downloaded.operation(), Some(online::downloaded::Operation::Download)) {
                    text(&mut commands, controls, font, "Сохранение материала на диск…", 12., MUTED);
                }
                if online.provider == online::Provider::Community && paint.community.busy() {
                    text(&mut commands, controls, font, "Установка пакета…", 12., MUTED);
                }
                if page_count > 1 {
                    if page > 0 {
                        button(&mut commands, controls, font, "Предыдущая", PaintAction::OnlinePage(page - 1));
                    }
                    text(&mut commands, controls, font, &format!("{} / {page_count}", page + 1), 11., MUTED);
                    if page + 1 < page_count {
                        button(&mut commands, controls, font, "Следующая", PaintAction::OnlinePage(page + 1));
                    }
                }
                let results = node(&mut commands, shelf, if shelf_docked { column(4.) } else { row(4.) });
                let entries = matching.into_iter().skip(page * online::PAGE).take(online::PAGE);
                let placeholder = ui.layer_thumbs.checker.clone();
                if online.job.is_some() {
                    ui.online_previews.clear();
                } else {
                    ui.online_previews.page(online.provider, entries.clone().map(|(_, entry)| entry), &mut images, &placeholder);
                }
                for (index, entry) in entries {
                    let preview = ui.online_previews.image(online.provider, entry).unwrap_or_else(|| placeholder.clone());
                    if let Some(package) = &entry.community {
                        let card = material_card(&mut commands, results, font, &entry.name, preview,
                            PaintAction::InstallCommunity(index), shelf_size);
                        let installed = paint.community.installed(&entry.id);
                        let status = if installed { "Установлено" } else { package.kind.label() };
                        text(&mut commands, card, font, status, 10., MUTED);
                        let label = if installed { "Пакет установлен" } else { "Установить пакет сообщества" };
                        commands.entity(card).insert(AccessibleLabel(format!("{label} · {}", entry.name)));
                    } else {
                        let tile = node(&mut commands, results, Node {
                            width: px(shelf_size + 28.), flex_shrink: 0., ..column(3.)
                        });
                        let card = material_card(&mut commands, tile, font, &entry.name, preview,
                            PaintAction::OnlineMaterial(online.provider, index), shelf_size);
                        commands.entity(card).insert(AccessibleLabel(format!("Добавить сетевой материал · {}", entry.name)));
                        button(&mut commands, tile, font, "Скачать",
                            PaintAction::DownloadOnlineMaterial(online.provider, index));
                    }
                }
                shown += 1;
            }
            Category::Downloaded => {
                let library = &paint.downloaded;
                let controls = node(&mut commands, shelf, column(4.));
                button(&mut commands, controls, font, "Импорт папки…", PaintAction::ImportDownloaded);
                let mut matching: Vec<_> = library.records()
                    .filter(|(_, record)| record.asset.matches(&shelf_filter.query)).collect();
                matching.sort_by_cached_key(|(_, record)| (record.asset.name.to_lowercase(), record.key()));
                let page_count = matching.len().div_ceil(online::PAGE);
                let page = library.page.min(page_count.saturating_sub(1));
                if let Some(error) = &library.error {
                    text(&mut commands, controls, font, error, 12., MUTED);
                } else if matching.is_empty() {
                    let message = if library.scan.is_some() {
                        "Чтение скачанных материалов…"
                    } else if library.root.is_none() {
                        "Папка скачанных материалов недоступна"
                    } else if library.records().next().is_none() {
                        "Скачайте материалы на вкладке «Онлайн»"
                    } else {
                        "Материалы не найдены"
                    };
                    text(&mut commands, controls, font, message, 12., MUTED);
                }
                if page_count > 1 {
                    if page > 0 {
                        button(&mut commands, controls, font, "Предыдущая", PaintAction::DownloadedPage(page - 1));
                    }
                    text(&mut commands, controls, font, &format!("{} / {page_count}", page + 1), 11., MUTED);
                    if page + 1 < page_count {
                        button(&mut commands, controls, font, "Следующая", PaintAction::DownloadedPage(page + 1));
                    }
                }
                if let Some(id) = library.menu
                    && let Some(record) = library.record(id)
                {
                    text(&mut commands, controls, font, &record.asset.name, 11., TEXT);
                    button(&mut commands, controls, font, "Открыть на сайте", PaintAction::OpenDownloadedSource(id));
                    button(&mut commands, controls, font, "Показать файлы", PaintAction::ShowDownloadedFiles(id));
                    button(&mut commands, controls, font, "Удалить загрузку", PaintAction::DeleteDownload(id));
                    button(&mut commands, controls, font, "Закрыть", PaintAction::DownloadedMenu(None));
                }
                let results = node(&mut commands, shelf, if shelf_docked { column(4.) } else { row(4.) });
                let records = matching.into_iter().skip(page * online::PAGE).take(online::PAGE);
                let placeholder = ui.layer_thumbs.checker.clone();
                if let Some(root) = library.root.as_deref() {
                    ui.online_previews.local_page(root, records.clone().map(|(_, record)| record), &mut images, &placeholder);
                } else {
                    ui.online_previews.clear();
                }
                for (index, record) in records {
                    let preview = ui.online_previews.local_image(record).unwrap_or_else(|| placeholder.clone());
                    let tile = node(&mut commands, results, Node {
                        width: px(shelf_size + 28.), flex_shrink: 0., ..column(3.)
                    });
                    let card = material_card(&mut commands, tile, font, &record.asset.name, preview,
                        PaintAction::ApplyDownloaded(index), shelf_size);
                    let label = format!("{} · {}K", record.provider.name(), record.resolution);
                    text(&mut commands, card, font, &label, 10., MUTED);
                    let menu_action = PaintAction::DownloadedMenu(Some(index));
                    let menu = button(&mut commands, tile, font, "⋯", menu_action);
                    commands.entity(menu).insert(AccessibleLabel(format!("Действия · {}", record.asset.name)));
                    commands.entity(card).observe(move |mut event: On<PointerClick>, mut actions: MessageWriter<Action>| {
                        if event.button == PointerButton::Secondary {
                            event.propagate(false);
                            actions.write(Action::ModelView(ModelViewAction::Paint(menu_action)));
                        }
                    });
                }
                shown += 1;
            }
            Category::Materials => {
                let mut group = None;
                for (i, kind) in material_results(MATERIALS, shelf_filter, |preset| (preset.name, preset.kind())) {
                    shown += 1;
                    let preset = &MATERIALS[i];
                    let parent = material_group(&mut commands, ui.shelf, font, shelf_filter, &mut group, kind);
                    let tile = node(&mut commands, parent, Node {
                        width: px(shelf_size + 28.),
                        height: px(shelf_size + 56.),
                        flex_shrink: 0.,
                        position_type: PositionType::Relative,
                        ..default()
                    });
                    let card = material_card(
                        &mut commands, tile, font, preset.name,
                        ui.swatches[i].clone(), PaintAction::Preset(i), shelf_size,
                    );
                    commands.entity(card).insert(AccessibleLabel(format!(
                        "Использовать материал для кисти · {}", preset.name
                    )));
                    let add = icon(
                        &mut commands,
                        tile,
                        icons::Glyph::Plus,
                        &format!("Добавить слой заливки · {}", preset.name),
                        PaintAction::AddMaterial(i),
                    );
                    commands.entity(add).entry::<Node>().and_modify(|mut node| {
                        node.position_type = PositionType::Absolute;
                        node.right = px(5);
                        node.top = px(5);
                    });
                }
            }
            Category::Smart => {
                let tools = shelf_tools(&mut commands, ui.shelf);
                button(
                    &mut commands, tools, font, "Импорт .mpsmart или ZIP…", PaintAction::ImportShelfMaterial,
                );
                if session.smart_loading {
                    text(&mut commands, ui.shelf, font, "Загрузка материала…", 12., MUTED);
                }
                if let Some(index) = shelf_filter.smart_recipe_menu
                    && let Some(preset) = SMART_MATERIALS.get(index)
                {
                    let menu = shelf_tools(&mut commands, ui.shelf);
                    text(&mut commands, menu, font, preset.name, 11., TEXT);
                    button(&mut commands, menu, font, "Добавить группой", PaintAction::AddSmartMaterial(index));
                    button(&mut commands, menu, font, "Добавить одним графом", PaintAction::AddSmartGraph(index));
                    button(&mut commands, menu, font, "Закрыть", PaintAction::SmartRecipeMenu(None));
                }
                let mut group = None;
                for (i, kind) in material_results(SMART_MATERIALS, shelf_filter, |preset| (preset.name, preset.kind())) {
                    shown += 1;
                    let preset = &SMART_MATERIALS[i];
                    let parent = material_group(&mut commands, ui.shelf, font, shelf_filter, &mut group, kind);
                    let tile = node(&mut commands, parent, Node {
                        width: px(shelf_size + 28.), height: px(shelf_size + 56.), flex_shrink: 0., ..default()
                    });
                    let card = material_card(
                        &mut commands, tile, font, preset.name,
                        ui.smart_swatches[i].clone(), PaintAction::AddSmartMaterial(i), shelf_size,
                    );
                    commands.entity(card).insert(AccessibleLabel(format!(
                        "Добавить умный материал · {}", preset.name
                    )));
                    let action = PaintAction::SmartRecipeMenu(Some(i));
                    let menu = button(&mut commands, tile, font, "⋯", action);
                    commands.entity(menu).insert(AccessibleLabel(format!("Действия · {}", preset.name)));
                    commands.entity(menu).entry::<Node>().and_modify(|mut node| {
                        node.position_type = PositionType::Absolute;
                        node.right = px(5);
                        node.top = px(5);
                    });
                    commands.entity(card).observe(move |mut event: On<PointerClick>, mut actions: MessageWriter<Action>| {
                        if event.button == PointerButton::Secondary {
                            event.propagate(false);
                            actions.write(Action::ModelView(ModelViewAction::Paint(action)));
                        }
                    });
                }
            }
            Category::Generators => {
                let maps = session.binding.and_then(|binding| {
                    baked.maps(index, doc, binding, session.texture_size)
                        .map(|maps| maps.generator_inputs().as_ref())
                        .or_else(|| doc.objects.iter().find(|object| object.id == binding.0)?
                            .materials.get(binding.1)?.paint.as_ref()?.baked.as_deref())
                });
                for (i, kind) in GeneratorKind::ALL.into_iter().enumerate() {
                    if !shelf_filter.matches(kind.label()) {
                        continue;
                    }
                    shown += 1;
                    let card = material_card(
                        &mut commands, ui.shelf, font, kind.label(),
                        ui.generator_swatches[i].clone(), PaintAction::ApplyGenerator(kind, None), shelf_size,
                    );
                    let generator = Generator::new(kind);
                    let missing = if kind == GeneratorKind::CustomMap {
                        doc.custom_bakes.first().filter(|recipe| maps.is_some_and(|maps|
                            maps.custom.iter().any(|map| map.recipe.id == recipe.id)
                        )).is_none()
                    } else {
                        generator.missing_bake_inputs(maps)
                    };
                    let label = if missing {
                        text(&mut commands, card, font, "Нужны карты", 10., MUTED);
                        format!("Применить маску · {} · Нужны карты", kind.label())
                    } else {
                        format!("Применить маску · {}", kind.label())
                    };
                    commands.entity(card).insert(AccessibleLabel(label));
                }
            }
            Category::Brushes => {
                let tools = shelf_tools(&mut commands, ui.shelf);
                for (label, action) in [
                    ("Загрузить…", PaintAction::ImportBrushTip),
                    ("По направлению мазка", PaintAction::FollowStroke),
                    ("Круглая", PaintAction::BrushTip(None)),
                ] {
                    button(&mut commands, tools, font, label, action);
                }
                for (i, tip) in paint.tips.entries.iter().enumerate() {
                    if !shelf_filter.matches(&tip.name) {
                        continue;
                    }
                    shown += 1;
                    alpha_card(
                        &mut commands,
                        ui.shelf,
                        font,
                        tip,
                        PaintAction::BrushTip(Some(i)),
                        shelf_size,
                    );
                }
            }
            Category::Textures => {
                let tools = shelf_tools(&mut commands, ui.shelf);
                for (label, action) in [
                    ("Без зерна", PaintAction::Grain(None)),
                    ("Инверсия зерна", PaintAction::GrainInvert),
                    ("Координаты UV", PaintAction::GrainUv),
                ] {
                    button(&mut commands, tools, font, label, action);
                }
                if let Some(texture) = shelf_filter.texture_menu
                    && let Some(grain) = paint.tips.grains.get(texture)
                {
                    let menu = node(&mut commands, shelf, column(3.));
                    text(&mut commands, menu, font, &grain.name, 11., TEXT);
                    button(&mut commands, menu, font, "Заливка высоты", PaintAction::AddShelfTexture(texture));
                    let channels = node(&mut commands, menu, Node {
                        max_width: px(200.), flex_wrap: FlexWrap::Wrap, row_gap: px(3.), ..row(3.)
                    });
                    for channel in [3, 0, 1, 2] {
                        button(&mut commands, channels, font, CHANNELS[channel], PaintAction::TextureToChannel(texture, channel));
                    }
                    button(&mut commands, menu, font, "Закрыть", PaintAction::ShelfTextureMenu(None));
                }
                for (i, grain) in paint.tips.grains.iter().enumerate() {
                    if !tips::texture_matches(i, &shelf_filter.query) {
                        continue;
                    }
                    shown += 1;
                    let preview = (shelf_size - 8.).max(ShelfSize::MIN as f32);
                    let tile = node(&mut commands, shelf, Node {
                        width: px(preview + 30.),
                        height: px(preview + 62.),
                        flex_shrink: 0.,
                        ..default()
                    });
                    let card = alpha_card(
                        &mut commands,
                        tile,
                        font,
                        grain,
                        PaintAction::Grain(Some(i)),
                        shelf_size,
                    );
                    let add = icon(
                        &mut commands, tile, icons::Glyph::Plus,
                        &format!("Добавить заливку высоты · {}", grain.name),
                        PaintAction::AddShelfTexture(i),
                    );
                    commands.entity(add).entry::<Node>().and_modify(|mut node| {
                        node.position_type = PositionType::Absolute;
                        node.right = px(5);
                        node.top = px(5);
                    });
                    commands.entity(card).observe(move |mut event: On<PointerClick>, mut actions: MessageWriter<Action>| {
                        let action = match event.button {
                            PointerButton::Primary if event.count == 2 => PaintAction::AddShelfTexture(i),
                            PointerButton::Secondary => PaintAction::ShelfTextureMenu(Some(i)),
                            _ => return,
                        };
                        event.propagate(false);
                        actions.write(Action::ModelView(ModelViewAction::Paint(action)));
                    });
                }
            }
            Category::Graphs => {
                let tools = shelf_tools(&mut commands, ui.shelf);
                button(&mut commands, tools, font, "Импорт .mpgraph или ZIP…", PaintAction::ImportGraphFile);
                if session.smart_loading {
                    text(&mut commands, tools, font, "Расчёт материала…", 12., MUTED);
                }
                if let Some(index) = shelf_filter.saved_graph_menu
                    && let Some(entry) = paint.graph_shelf.entries.get(index)
                {
                    text(&mut commands, tools, font, &entry.name, 11., TEXT);
                    button(&mut commands, tools, font, "Открыть на холсте", PaintAction::OpenSavedGraph(index));
                    button(&mut commands, tools, font, "Пакет CC0…", PaintAction::ShareSavedGraph(index));
                    button(&mut commands, tools, font, "Закрыть", PaintAction::SavedGraphMenu(None));
                }
                let placeholder = ui.layer_thumbs.checker.clone();
                for (index, entry) in paint.graph_shelf.entries.iter().enumerate() {
                    if !shelf_filter.matches(&entry.name) {
                        continue;
                    }
                    shown += 1;
                    let swatch = entry.thumbnail.as_ref().map_or_else(|| placeholder.clone(), |bitmap| {
                        ui.graph_swatches.entry(index)
                            .or_insert_with(|| images.add(shelf_image(bitmap.pixels().clone()))).clone()
                    });
                    let tile = node(&mut commands, shelf, Node {
                        width: px(shelf_size + 28.), height: px(shelf_size + 56.), flex_shrink: 0., ..default()
                    });
                    let card = material_card(&mut commands, tile, font, &entry.name, swatch, PaintAction::ApplySavedGraph(index), shelf_size);
                    let action = PaintAction::SavedGraphMenu(Some(index));
                    let menu = button(&mut commands, tile, font, "⋯", action);
                    commands.entity(menu).insert(AccessibleLabel(format!("Действия · {}", entry.name)));
                    commands.entity(menu).entry::<Node>().and_modify(|mut node| {
                        node.position_type = PositionType::Absolute;
                        node.right = px(5);
                        node.top = px(5);
                    });
                    commands.entity(card).observe(move |mut event: On<PointerClick>, mut actions: MessageWriter<Action>| {
                        if event.button == PointerButton::Secondary {
                            event.propagate(false);
                            actions.write(Action::ModelView(ModelViewAction::Paint(action)));
                        }
                    });
                }
                for (i, record) in paint.community.records.iter().enumerate() {
                    if record.kind == online::community::Kind::Graph && shelf_filter.matches(&record.name) {
                        shown += 1;
                        material_card(&mut commands, ui.shelf, font, &record.name, placeholder.clone(), PaintAction::CommunityGraph(i), shelf_size);
                    }
                }
                for (i, recipe) in material_graph::PRESETS.iter().enumerate() {
                    if shelf_filter.matches(recipe.name) {
                        shown += 1;
                        button(&mut commands, tools, font, recipe.name, PaintAction::AddGraphMaterial(i));
                    }
                }
            }
        }
        if shown == 0 {
            let message = if shelf_filter.query.is_empty()
                && (!material_category || shelf_filter.material.is_none())
            {
                "В этой категории нет ресурсов"
            } else {
                "Ничего не найдено"
            };
            text(&mut commands, ui.shelf, font, message, 12., MUTED);
        }
        ui.shelf_stamp = Some(shelf_stamp);
    }
    ui.online_previews.update(matches!(session.category, Category::Online | Category::Downloaded)
        && shelf_visible
        && !editor.browser_popup && !editor.hdri_library_open, &mut images);
    if session.category == Category::Smart && ui.shelf_preview_job.is_none() {
        let user = paint.library.entries().rev().find(|(i, entry)| {
            entry.thumbnail.is_none()
                && !ui.requested_swatches.contains(&ShelfMaterial::User(*i))
                && ui.user_swatches.contains_key(i)
                && shelf_filter.matches(&entry.name)
                && matches!(&entry.kind, presets::Kind::Material { metallic, .. }
                    if shelf_filter.allows_material(MaterialType::of(&entry.name, Some(*metallic))))
        });
        if let Some((i, entry)) = user {
            let path = entry.path.clone();
            ui.requested_swatches.insert(ShelfMaterial::User(i));
            let task = AsyncComputeTaskPool::get().spawn(async move {
                shelf_preview::render(presets::load_material(&path, shelf_preview::SIZE)?)
            });
            ui.shelf_preview_job = Some((ShelfMaterial::User(i), task));
        } else if let Some((i, recipe)) = SMART_MATERIALS.iter().enumerate().filter(|(i, recipe)| {
            !ui.requested_swatches.contains(&ShelfMaterial::Smart(*i))
                && shelf_filter.matches(recipe.name)
                && shelf_filter.allows_material(recipe.kind())
        }).min_by_key(|(_, recipe)| recipe.kind()) {
            ui.requested_swatches.insert(ShelfMaterial::Smart(i));
            let task = AsyncComputeTaskPool::get().spawn(async move {
                shelf_preview::render(recipe.prepare(shelf_preview::SIZE)?)
            });
            ui.shelf_preview_job = Some((ShelfMaterial::Smart(i), task));
        }
    }
    let selected = doc
        .objects
        .iter()
        .flat_map(|object| (0..object.materials.len().max(1)).map(move |slot| (object, slot)))
        .nth(session.set)
        .or_else(|| doc.active().map(|object| (object, 0)));
    if let Some((object, slot)) = selected {
        if let Ok(mut image) = texture_nodes.get_mut(ui.uv_canvas) {
            let bake_view = (session.panel == Panel::Maps).then_some(session.bake_view).flatten();
            let live_mask = session.live_mask_map().is_some() && session.show_mask && session.has_mask();
            let handle = if live_mask {
                baked.masks.mask(index, doc, session).or_else(|| runtime.preview(
                    index, object.id, slot, crate::editor::model_paint_data::MASK_TARGET,
                ))
            } else if let Some(map) = bake_view {
                baked.preview_for(index, doc, session, map).cloned()
            } else { runtime
                .preview(
                    index,
                    object.id,
                    slot,
                    if session.show_mask && session.has_mask() {
                        crate::editor::model_paint_data::MASK_TARGET
                    } else {
                        session.channel
                    },
                ) }.unwrap_or_else(|| ui.uv_image.clone());
            if image.image != handle {
                image.image = handle;
            }
        }
    } else if let Ok(mut image) = texture_nodes.get_mut(ui.uv_canvas)
        && image.image != ui.uv_image
    {
        image.image = ui.uv_image.clone();
    }
    let overlay_source = selected.map(|(object, slot)| uv_overlay::Source {
        key: (index, object.id, slot),
        mesh: object.geometry.clone(),
        offset: object.uv_offset(slot),
    });
    let uv_visible = !custom_workspace && session.view != View::Three;
    let show_wire = uv_visible && doc.paint_uv.show_wire;
    let show_seams = uv_visible && doc.paint_uv.show_seams_2d;
    if let Some(drawn) = ui.uv_overlay.update(overlay_source, show_wire, show_seams) {
        for (entity, layer) in [(ui.uv_wire, drawn.wire), (ui.uv_seams, drawn.seams)] {
            if let Some(layer) = layer && let Ok(mut image) = texture_nodes.get_mut(entity) {
                image.image = images.add(Image::from_dynamic(
                    image::DynamicImage::ImageRgba8(layer), true, RenderAssetUsages::default(),
                ));
            }
        }
    }
    for (entity, visible) in [
        (ui.uv_wire, show_wire && ui.uv_overlay.wire_ready),
        (ui.uv_seams, show_seams && ui.uv_overlay.seams_ready),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if visible { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let Ok(mut image) = texture_nodes.get_mut(ui.uv_wire) {
        let tint = Color::WHITE.with_alpha(doc.paint_uv.wire_opacity);
        if image.color != tint {
            image.color = tint;
        }
    }
    if session.panel != Panel::Maps && let Ok(mut info) = labels.get_mut(ui.uv_info) {
        let label = match ui.uv_overlay.has_uv {
            Some(true) => {
                let tile = selected.and_then(|(object, slot)| object.materials.get(slot))
                    .and_then(|material| material.udim())
                    .map_or_else(|| "UV 0–1".to_owned(), |tile| format!("UDIM {}", tile.number()));
                format!("{} × {} · {tile}", session.texture_size, session.texture_size)
            }
            Some(false) => "UV-развёртка отсутствует".into(),
            None => "Подготовка UV…".into(),
        };
        if info.0 != label {
            info.0 = label;
        }
    }
    if session.panel == Panel::Maps && let Ok(mut info) = labels.get_mut(ui.uv_info) {
        let summary = if let Some(progress) = baked.progress() {
            format!("Запекание · {:.0}%", progress * 100.)
        } else if session.live_mask_map().is_some() && session.show_mask && session.has_mask()
        {
            "Итоговая маска слоя".into()
        } else if let Some(map) = session.bake_view {
            if let Some(status) = baked.live_status(map) {
                status
            } else if session.bake_ready {
                let label = match map {
                    bake::BakeMap::Custom(id) => doc.custom_bakes.iter().find(|recipe| recipe.id == id)
                        .map_or(map.label(), |recipe| recipe.name.as_str()),
                    _ => map.label(),
                };
                format!("{} × {} · {label}", session.texture_size, session.texture_size)
            } else {
                "Запеките выбранный набор".into()
            }
        } else if let Some(status) = session.live_mask_map().and_then(|map| baked.live_status(map)) {
            status
        } else {
            format!("{} × {} · Материал", session.texture_size, session.texture_size)
        };
        if info.0 != summary {
            info.0 = summary;
        }
    }
}

impl PreviewValue {
    fn parse(&self, input: &EditableText) -> Option<f32> {
        input.value().to_string().trim().replace(',', ".").parse::<f32>().ok()
            .filter(|value| (self.0.min()..=self.0.max()).contains(value))
    }

    fn display_value(&self, doc: &crate::editor::model_document::Document, navigation: navigation::Settings) -> f32 {
        (self.0.value(doc, navigation) * 100.).round() / 100.
    }
}

fn preview_number(commands: &mut Commands, parent: Entity, font: &Handle<Font>, parameter: PreviewParameter) {
    let step = match parameter {
        PreviewParameter::FieldOfView => 1.,
        PreviewParameter::Occlusion | PreviewParameter::UvWireOpacity => 0.5,
        _ => 0.05,
    };
    let number = Number::decimal(parameter.min(), parameter.max(), step, 2).finish();
    let field = number_line(commands, parent, font, parameter.label(), number, false);
    commands.entity(field).insert(PreviewValue(parameter))
        .entry::<Node>().and_modify(|mut node| node.width = px(64));
    commands.entity(field).observe(
        |mut event: On<FocusedInput<KeyboardInput>>, mut focus: ResMut<InputFocus>| {
            if event.input.state == ButtonState::Pressed && !event.input.repeat
                && matches!(event.input.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
            {
                focus.clear();
                event.propagate(false);
            }
        },
    );
}

fn history_number(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    preference: HistoryPreference,
    hint: &str,
) {
    let (min, max) = preference.range();
    let field = number_line(commands, parent, font, label, Number::integer(min.into(), max.into()), false);
    control_hints::attach(commands, field, hint);
    commands.entity(field).insert((preference, Pickable::default())).observe(
        |mut event: On<FocusedInput<KeyboardInput>>, mut actions: MessageWriter<Action>| {
            if event.input.state == ButtonState::Pressed && !event.input.repeat
                && matches!(event.input.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
            {
                actions.write(Action::ModelView(ModelViewAction::Paint(
                    PaintAction::ApplyHistoryPreference(event.focused_entity),
                )));
                event.propagate(false);
            }
        },
    );
}
