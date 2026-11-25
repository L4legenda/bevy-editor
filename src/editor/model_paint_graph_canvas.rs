//! Node canvas for graph-backed paint layers.
use super::*;
use bevy::{
    input::mouse::MouseScrollUnit,
    picking::events::{PointerCancel, PointerClick, PointerDrag, PointerDragEnd, PointerDragStart, PointerScroll},
};
use material_graph::previews::{Previews, Uploads};
use material_graph::{Batch, BatchKind, Edit, GraphInput, WireSource};
use material_painter_graph::{Graph, Kind, Library};
use bevy::tasks::IoTaskPool;
use std::sync::Arc;

#[path = "model_paint_graph_catalogue.rs"]
mod catalogue;
#[cfg(test)]
pub(in crate::editor::ui::model_viewer::paint) use catalogue::PresetSwatch;
#[path = "model_paint_graph_palette.rs"]
mod palette;

const WIDTH: f32 = 176.;

pub(in crate::editor::ui::model_viewer::paint) fn position_next_to_graph(graph: &Graph) -> IVec2 {
    let right = graph.nodes.iter().map(|node| node.pos[0]).fold(0.0, f32::max);
    Vec2::new(right + WIDTH + 44., 0.).round().as_ivec2()
}
const SEGMENTS: usize = 8;
const PORT_Y: f32 = 45.;
const ROW: f32 = 21.;
const PREVIEW_EDGE: f32 = 62.;
const PREVIEW_TOP: f32 = 55.;
const CARD: Color = Color::srgb(0.092, 0.105, 0.13);
const WIRE: Color = Color::srgb(0.62, 0.69, 0.89);
const GRAY: Color = Color::srgb(0.65, 0.72, 0.82);
const COLOR: Color = Color::srgb(0.86, 0.62, 0.32);

struct Card {
    entity: Entity,
    output: Entity,
    preview: Option<Entity>,
    origin: Vec2,
    height: f32,
    source: Option<String>,
}

struct Wire {
    segments: [Entity; SEGMENTS],
    source: u64,
    target: u64,
    input: usize,
}

struct WireDrag {
    handle: Entity,
    source: WireSource,
}

pub(super) struct Canvas {
    root: Entity,
    panes: Entity,
    catalogue: Entity,
    catalogue_nav: [Entity; 3],
    catalogue_stamp: Option<(usize, GraphPage, u64, u64, u64, u64)>,
    preset_previews: catalogue::PresetPreviews,
    reset_layout: Entity,
    pub(super) preview: Entity,
    pub(super) inspector: Entity,
    stage: Entity,
    title: Entity,
    diagnostic: Entity,
    palette: palette::Palette,
    graph_pane: Entity,
    export_tools: [Entity; 5],
    delete: Entity,
    edit_source: Entity,
    copy_source: Entity,
    source_copy: Option<SourceCopy>,
    library: Option<Arc<Library>>,
    secondary_pan: bool,
    typing: bool,
    target: Option<Target>,
    payload: Option<Arc<serde_json::Value>>,
    graph_images: Option<Arc<texture_fill::GraphImages>>,
    folder_labels: Option<Arc<HashMap<u64, String>>>,
    bake_maps: Option<Arc<bake::Maps>>,
    graph_surface: Option<Arc<material_graph::surface::Source>>,
    needs_surface: bool,
    previews: Previews,
    cards: HashMap<u64, Card>,
    wires: Vec<Wire>,
    pending: Option<[Entity; SEGMENTS]>,
    source: Option<u64>,
    wire_drag: Option<WireDrag>,
    selected: Option<u64>,
    selection: Vec<u64>,
    dragging: Option<Vec2>,
    marquee: Option<Rect>,
    marquee_node: Option<Entity>,
    stage_origin: Vec2,
    clipboard: Option<Clipboard>,
    batch: Option<(u64, Arc<serde_json::Value>, Batch)>,
    next_batch: u64,
    pan: Vec2,
    zoom: f32,
    size: Vec2,
    fit: bool,
    fit_new_node: bool,
    dirty: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Texture(usize, usize, usize, bool),
    Shader(usize, usize),
}

struct SourceCopy {
    target: Target,
    payload: Arc<serde_json::Value>,
    node: u64,
    state: SourceCopyState,
}

enum SourceCopyState {
    Writing(Task<Result<(String, Arc<str>), String>>),
    Applying { definition: String, code: Arc<str> },
}

impl Target {
    fn model(self) -> usize {
        match self {
            Self::Texture(model, ..) | Self::Shader(model, _) => model,
        }
    }

    fn for_session(model: usize, session: &Session) -> Self {
        if let Some(shader) = session.preview_shader.filter(|_| session.editing_shader().is_some()) {
            Self::Shader(model, shader)
        } else {
            Self::Texture(model, session.set, session.stack.selected, session.editing_mask)
        }
    }
}

struct Clipboard {
    target: Target,
    nodes: Vec<material_painter_graph::Node>,
    images: Arc<texture_fill::GraphImages>,
}

fn extend_selection(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight])
}

fn action(messages: &mut MessageWriter<Action>, action: PaintAction) {
    messages.write(Action::ModelView(ModelViewAction::Paint(action)));
}

fn end_wire_drag(event: On<PointerDragEnd>, mut ui: ResMut<PaintUi>) {
    if event.button == PointerButton::Primary {
        ui.graph.take_wire(event.entity);
    }
}

pub(super) fn local_button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
) -> Entity {
    let entity = surface(
        commands,
        parent,
        Node {
            min_height: px(25),
            flex_shrink: 0.,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::horizontal(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..row(5.)
        },
        CARD,
    );
    commands.entity(entity).insert((
        Button,
        BorderColor::all(EDGE),
        AccessibleLabel(label.into()),
    ));
    let label = text(commands, entity, font, label, 12., TEXT);
    commands.entity(label).insert(Pickable::IGNORE);
    entity
}

impl Canvas {
    #[cfg(test)]
    pub(super) fn pending_source(&self) -> Option<u64> {
        self.wire_origin()
    }

    #[cfg(test)]
    pub(super) fn selection(&self) -> &[u64] {
        &self.selection
    }

    pub(super) fn new(commands: &mut Commands, parent: Entity, font: &Handle<Font>) -> Self {
        let workspace = surface(
            commands,
            parent,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                left: px(44),
                right: px(0),
                top: px(118),
                bottom: px(SHELF_HEIGHT),
                border: UiRect::all(px(1.)),
                ..column(0.)
            },
            WELL,
        );
        commands
            .entity(workspace)
            .insert((BorderColor::all(EDGE), ZIndex(20)));
        let controls = surface(commands, workspace, Node {
            height: px(34), flex_shrink: 0., padding: UiRect::horizontal(px(8)), ..row(5.)
        }, CHROME);
        text(commands, controls, font, "Редактор графа", 13., TEXT);
        let catalogue_nav = [
            ("Стартовая", GraphPage::Home),
            ("Графы модели", GraphPage::Project),
            ("К текущему графу", GraphPage::Canvas),
        ].map(|(label, page)| button(commands, controls, font, label, PaintAction::GraphPage(page)));
        let reset = local_button(commands, controls, font, "Сбросить раскладку графа");
        commands.entity(reset).observe(|event: On<PointerClick>, mut dock: ResMut<sidebar::GraphDock>| {
            if event.button == PointerButton::Primary { dock.reset(); }
        });
        button(commands, controls, font, "Закрыть холст", PaintAction::GraphCanvas);
        let catalogue = surface(commands, workspace, Node {
            position_type: PositionType::Absolute,
            top: px(34),
            bottom: px(0),
            left: px(0),
            right: px(0),
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(18)),
            display: Display::None,
            ..column(12.)
        }, WELL);
        commands.entity(catalogue).insert((Scroller, AccessibleLabel("Страница графов материалов".into())));
        let panes = node(commands, workspace, Node {
            width: percent(100), flex_grow: 1., min_height: px(0), ..column(0.)
        });
        let pane = || Node {
            width: percent(100), flex_grow: 1., min_height: px(0), min_width: px(0), ..column(0.)
        };
        let preview = node(commands, panes, pane());
        let root = surface(commands, panes, pane(), WELL);
        let inspector = node(commands, panes, Node {
            padding: UiRect::all(px(7)), overflow: Overflow::scroll_y(), row_gap: px(4), ..pane()
        });
        commands.entity(inspector).insert((Scroller, AccessibleLabel("Параметры графа".into())));
        sidebar::setup_graph(commands, panes, preview, root, inspector);
        let toolbar = surface(
            commands,
            root,
            Node {
                min_height: px(39),
                flex_shrink: 0.,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(px(8)),
                ..row(6.)
            },
            CHROME,
        );
        let add = local_button(commands, toolbar, font, "Добавить узел");
        commands
            .entity(add)
            .observe(|event: On<PointerClick>, mut ui: ResMut<PaintUi>| {
                if event.button == PointerButton::Primary {
                    ui.graph.palette.toggle();
                    ui.graph.source = None;
                }
            });
        let fit = local_button(commands, toolbar, font, "Показать всё");
        commands
            .entity(fit)
            .observe(|event: On<PointerClick>, mut ui: ResMut<PaintUi>| {
                if event.button == PointerButton::Primary {
                    ui.graph.fit = true;
                }
            });
        let delete = local_button(commands, toolbar, font, "Удалить выбранные узлы");
        commands.entity(delete).observe(
            |event: On<PointerClick>, mut ui: ResMut<PaintUi>, mut messages: MessageWriter<Action>| {
                if event.button == PointerButton::Primary {
                    ui.graph.remove_selected(&mut messages);
                }
            },
        );
        let edit_source = local_button(commands, controls, font, "Открыть исходник WGSL");
        commands.entity(edit_source).observe(
            |event: On<PointerClick>, ui: Res<PaintUi>, mut maker: ResMut<ShaderMaker>, mut messages: MessageWriter<Action>| {
                if event.button == PointerButton::Primary
                    && let Some(target) = ui.graph.target
                    && let Some((node, def)) = ui.graph.selected_source()
                    && def.starts_with("user/")
                    && node_library::available(def)
                    && let Some(definition) = node_library::library().find(def)
                {
                    maker.request_graph_source(target.model(), node, definition.id.clone(), Arc::from(definition.source.as_str()));
                    action(&mut messages, PaintAction::Panel(Panel::Shader));
                }
            },
        );
        let copy_source = local_button(commands, controls, font, "Редактировать копию узла");
        commands.entity(copy_source).observe(
            |event: On<PointerClick>, mut ui: ResMut<PaintUi>, maker: Res<ShaderMaker>, mut editor: ResMut<Editor>| {
                if event.button == PointerButton::Primary
                    && let Err(error) = ui.graph.copy_source(&maker)
                {
                    editor.status = error;
                }
            },
        );
        for button in [edit_source, copy_source] {
            commands.entity(button).entry::<Node>().and_modify(|mut node| node.display = Display::None);
        }
        let title = text(commands, toolbar, font, "", 13., TEXT);
        commands
            .entity(title)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.flex_grow = 1.;
            });
        let export = button(
            commands,
            toolbar,
            font,
            ".mpgraph…",
            PaintAction::ExportGraphFile,
        );
        let package = button(
            commands,
            toolbar,
            font,
            "ZIP…",
            PaintAction::ExportGraphPackage,
        );
        let save = button(commands, toolbar, font, "На полку", PaintAction::SaveGraphShelf);
        let outputs = button(commands, toolbar, font, "Карты…", PaintAction::ExportGraphOutputs);
        let submission = button(commands, toolbar, font, "Пакет CC0…", PaintAction::ShareCommunityGraph);
        let diagnostic = text(commands, root, font, "", 12., Color::srgb(1., 0.64, 0.44));
        commands
            .entity(diagnostic)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.display = Display::None;
                node.flex_shrink = 0.;
                node.padding = UiRect::axes(px(8.), px(5.));
                node.max_height = px(84.);
                node.overflow = Overflow::clip();
            });
        let stage = surface(
            commands,
            root,
            Node {
                width: percent(100),
                flex_grow: 1.,
                min_height: px(0),
                overflow: Overflow::clip(),
                ..default()
            },
            Color::srgb(0.047, 0.056, 0.072),
        );
        let help = text(
            commands,
            root,
            font,
            "Рамка — выделить · Shift — добавить · Ctrl+C / V / D — копировать / вставить / дублировать · Del — удалить · СКМ — панорама",
            11.,
            MUTED,
        );
        commands
            .entity(help)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.flex_shrink = 0.;
                node.padding = UiRect::axes(px(8.), px(4.));
            });
        commands
            .entity(stage)
            .insert((GraphCanvasStage, AccessibleLabel("Холст графа материала".into())))
            .observe(|_: On<PointerCancel>, mut ui: ResMut<PaintUi>| ui.graph.cancel_wire())
            .observe(move |event: On<PointerDrag>, mut ui: ResMut<PaintUi>| {
                if event.button != PointerButton::Primary && ui.graph.dragging.is_none() {
                    ui.graph.pan += event.delta;
                    ui.graph.secondary_pan |= event.button == PointerButton::Secondary;
                    ui.graph.dirty = true;
                } else if event.entity == stage && event.button == PointerButton::Primary {
                    let end = event.pointer.position - ui.graph.stage_origin;
                    ui.graph.marquee = Some(Rect::from_corners(end - event.distance, end));
                }
            })
            .observe(move |event: On<PointerDragEnd>, mut ui: ResMut<PaintUi>, keys: Res<ButtonInput<KeyCode>>, mut messages: MessageWriter<Action>| {
                if event.button == PointerButton::Secondary {
                    ui.graph.secondary_pan = false;
                    return;
                }
                if event.entity != stage || event.button != PointerButton::Primary { return; }
                let Some(rect) = ui.graph.marquee.take() else { return; };
                let mut hits: Vec<_> = ui.graph.cards.iter().filter_map(|(&id, card)| {
                    let origin = ui.graph.origin(id)?;
                    let bounds = Rect::from_corners(origin, origin + Vec2::new(WIDTH, card.height) * ui.graph.zoom);
                    (!bounds.intersect(rect).is_empty()).then_some(id)
                }).collect();
                hits.sort_unstable();
                if !extend_selection(&keys) { ui.graph.selection.clear(); }
                for id in hits {
                    if !ui.graph.selection.contains(&id) { ui.graph.selection.push(id); }
                }
                ui.graph.publish_selection(&mut messages);
            })
            .observe(
                move |mut event: On<PointerScroll>,
                      mut ui: ResMut<PaintUi>,
                      geometry: Query<(&ComputedNode, &UiGlobalTransform)>| {
                    if ui.graph.dragging.is_some() {
                        return;
                    }
                    let Ok((size, transform)) = geometry.get(stage) else {
                        return;
                    };
                    let origin = transform.affine().translation * size.inverse_scale_factor()
                        - ui.graph.size * 0.5;
                    let point = event.pointer.position - origin;
                    let old = ui.graph.zoom;
                    let step = if event.unit == MouseScrollUnit::Pixel {
                        event.y / 48.
                    } else {
                        event.y
                    };
                    let zoom = (old * (0.17 * step.clamp(-4., 4.)).exp()).clamp(0.24, 2.4);
                    ui.graph.pan = point - (point - ui.graph.pan) * (zoom / old);
                    ui.graph.zoom = zoom;
                    ui.graph.dirty = true;
                    event.propagate(false);
                },
            )
            .observe(move |event: On<PointerClick>, mut ui: ResMut<PaintUi>, mut messages: MessageWriter<Action>| {
                if event.entity != stage { return; }
                if event.button == PointerButton::Secondary && !ui.graph.secondary_pan {
                    ui.graph.open_palette_at(event.pointer.position, None);
                } else if event.button == PointerButton::Primary && ui.graph.marquee.is_none() {
                    ui.graph.source = None;
                    ui.graph.palette.close();
                    ui.graph.selection.clear();
                    ui.graph.publish_selection(&mut messages);
                }
            })
            .observe(move |event: On<PointerDragDrop>, mut ui: ResMut<PaintUi>| {
                if event.entity == stage && event.button == PointerButton::Primary
                    && let Some(source) = ui.graph.take_wire(event.dropped)
                {
                    ui.graph.open_palette_at(event.pointer.position, Some(source));
                }
            });
        let palette = palette::Palette::new(commands, root, font);
        Self {
            root: workspace,
            panes,
            catalogue,
            catalogue_nav,
            catalogue_stamp: None,
            preset_previews: catalogue::PresetPreviews::default(),
            reset_layout: reset,
            preview,
            inspector,
            stage,
            title,
            diagnostic,
            palette,
            graph_pane: root,
            export_tools: [export, package, outputs, save, submission],
            delete,
            edit_source,
            copy_source,
            source_copy: None,
            library: None,
            secondary_pan: false,
            typing: false,
            target: None,
            payload: None,
            graph_images: None,
            folder_labels: None,
            bake_maps: None,
            graph_surface: None,
            needs_surface: false,
            previews: Previews::default(),
            cards: default(),
            wires: vec![],
            pending: None,
            source: None,
            wire_drag: None,
            selected: None,
            selection: Vec::new(),
            dragging: None,
            marquee: None,
            marquee_node: None,
            stage_origin: Vec2::ZERO,
            clipboard: None,
            batch: None,
            next_batch: 0,
            pan: Vec2::ZERO,
            zoom: 1.,
            size: Vec2::ZERO,
            fit: true,
            fit_new_node: false,
            dirty: true,
        }
    }

    fn rebuild(&mut self, commands: &mut Commands, font: &Handle<Font>, graph: Graph, folder: Option<&Stack>) {
        if self.fit_new_node && graph.nodes.len() > self.cards.len() {
            self.fit = true;
            self.fit_new_node = false;
        }
        commands.entity(self.stage).despawn_related::<Children>();
        self.cards.clear();
        self.wires.clear();
        self.dragging = None;
        self.wire_drag = None;
        self.marquee = None;
        self.selection.retain(|id| graph.node(*id).is_some());
        let marquee = surface(commands, self.stage, Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            border: UiRect::all(px(1.)),
            ..default()
        }, Color::srgba(0.83, 0.64, 0.35, 0.16));
        commands.entity(marquee).insert((Pickable::IGNORE, ZIndex(12), BorderColor::all(FOCUS)));
        self.marquee_node = Some(marquee);
        self.source = self.source.filter(|id| graph.node(*id).is_some());
        self.pending = Some(wire(commands, self.stage, FOCUS));
        let lib = material_graph::library();
        for target in &graph.nodes {
            let Some(def) = lib.find(&target.def) else {
                continue;
            };
            for (i, input) in def.inputs.iter().enumerate() {
                if let Some(source) = target.links.get(&input.name) {
                    self.wires.push(Wire {
                        segments: wire(commands, self.stage, WIRE),
                        source: *source,
                        target: target.id,
                        input: i,
                    });
                }
            }
        }
        for node_data in &graph.nodes {
            let id = node_data.id;
            let managed = folder.and_then(|_| folder_graph::managed(&graph, id));
            let Some(def) = lib.find(&node_data.def) else {
                continue;
            };
            let height = card_height(def.inputs.len(), def.shading);
            let card = surface(
                commands,
                self.stage,
                Node {
                    position_type: PositionType::Absolute,
                    width: px(WIDTH),
                    height: px(height),
                    border: UiRect::all(px(1.)),
                    border_radius: BorderRadius::all(px(7.)),
                    ..column(0.)
                },
                CARD,
            );
            commands.entity(card).insert(BorderColor::all(EDGE));
            let caption = self.folder_labels.as_ref().and_then(|labels| labels.get(&id))
                .cloned().unwrap_or_else(|| format!("{} #{}", def.name, id));
            let title = local_button(commands, card, font, &caption);
            commands
                .entity(title)
                .observe(
                    move |mut event: On<PointerClick>, mut ui: ResMut<PaintUi>, keys: Res<ButtonInput<KeyCode>>, mut messages: MessageWriter<Action>| {
                        if event.button == PointerButton::Primary && ui.graph.dragging.is_none() {
                            if extend_selection(&keys) {
                                if let Some(index) = ui.graph.selection.iter().position(|&node| node == id) {
                                    ui.graph.selection.remove(index);
                                } else {
                                    ui.graph.selection.push(id);
                                }
                            } else {
                                ui.graph.selection = vec![id];
                            }
                            ui.graph.publish_selection(&mut messages);
                            event.propagate(false);
                        }
                    },
                )
                .observe(move |mut event: On<PointerDrag>, mut ui: ResMut<PaintUi>, keys: Res<ButtonInput<KeyCode>>, mut messages: MessageWriter<Action>| {
                    if event.button == PointerButton::Primary {
                        if ui.graph.dragging.is_none() && !ui.graph.selection.contains(&id) {
                            if !extend_selection(&keys) { ui.graph.selection.clear(); }
                            ui.graph.selection.push(id);
                            ui.graph.publish_selection(&mut messages);
                        }
                        ui.graph.dragging = Some(event.distance / ui.graph.zoom);
                        ui.graph.dirty = true;
                        event.propagate(false);
                    }
                })
                .observe(
                    move |event: On<PointerDragEnd>,
                          mut ui: ResMut<PaintUi>,
                          mut messages: MessageWriter<Action>| {
                        if event.button != PointerButton::Primary {
                            return;
                        }
                        if ui.graph.dragging.take().is_none() { return; }
                        let offset = event.distance / ui.graph.zoom;
                        ui.graph.dirty = true;
                        let ids = ui.graph.selection.clone();
                        ui.graph.queue_batch(Batch::Move(ids, offset), BatchKind::Move, &mut messages);
                    },
                );
            let output = pin(
                commands,
                card,
                WIDTH - 6.,
                PORT_Y,
                tint(def.output),
                format!("{} · выход", def.name),
            );
            if managed.is_none() {
                commands
                .entity(output)
                .observe(
                    move |mut event: On<PointerClick>, mut ui: ResMut<PaintUi>| {
                        if event.button == PointerButton::Primary && ui.graph.wire_drag.is_none() {
                            ui.graph.source = if ui.graph.source == Some(id) {
                                None
                            } else {
                                Some(id)
                            };
                            ui.graph.dirty = true;
                            event.propagate(false);
                        }
                    },
                )
                .observe(
                    move |event: On<PointerDragStart>, mut ui: ResMut<PaintUi>| {
                        if event.button == PointerButton::Primary {
                            ui.graph.start_wire(output, WireSource { node: id, input: None });
                        }
                    },
                )
                .observe(end_wire_drag);
            }
            for (index, input) in def.inputs.iter().enumerate() {
                let y = PORT_Y + ROW * index as f32;
                let input = pin(
                    commands,
                    card,
                    5.,
                    y,
                    tint(input.kind),
                    format!("{} · {}", def.name, input.name),
                );
                if managed.is_none() || managed.is_some_and(|tag|
                    matches!(tag.role, material_painter_graph::Role::Channel(_))
                        && def.inputs[index].name == "mask")
                {
                    let port = GraphInput { node: id, index };
                    if let Some(source) = node_data.links.get(&def.inputs[index].name).copied()
                        && folder.is_none_or(|stack| {
                            folder_graph::managed(&graph, source).is_none()
                                && folder_graph::can_edit(stack, &graph, Edit::Disconnect(id, index))
                        })
                    {
                        commands.entity(input).observe(move |event: On<PointerDragStart>, mut ui: ResMut<PaintUi>| {
                            if event.button == PointerButton::Primary {
                                ui.graph.start_wire(input, WireSource { node: source, input: Some(port) });
                            }
                        }).observe(end_wire_drag);
                    }
                    commands
                    .entity(input)
                    .observe(
                        move |mut click: On<PointerClick>,
                              mut ui: ResMut<PaintUi>,
                              mut messages: MessageWriter<Action>| {
                            if ui.graph.wire_drag.is_some() {
                                click.propagate(false);
                                return;
                            }
                            if click.button == PointerButton::Secondary {
                                action(
                                    &mut messages,
                                    PaintAction::GraphEdit(material_graph::Edit::Disconnect(
                                        id, index,
                                    )),
                                );
                            } else if click.button == PointerButton::Primary
                                && let Some(source) = ui.graph.source.take()
                            {
                                action(
                                    &mut messages,
                                    PaintAction::GraphEdit(material_graph::Edit::Connect(
                                        source, id, index,
                                    )),
                                );
                            }
                            ui.graph.dirty = true;
                            click.propagate(false);
                        },
                    )
                    .observe(
                        move |mut event: On<PointerDragDrop>,
                              mut ui: ResMut<PaintUi>,
                              mut messages: MessageWriter<Action>| {
                            if event.button == PointerButton::Primary
                                && let Some(source) = ui.graph.take_wire(event.dropped)
                            {
                                if source.input != Some(port) {
                                    action(&mut messages, PaintAction::GraphEdit(Edit::Reconnect(source, port)));
                                }
                                event.propagate(false);
                            }
                        },
                    );
                }
                let label = text(
                    commands,
                    card,
                    font,
                    &def.inputs[index].name.replace('_', " "),
                    11.,
                    MUTED,
                );
                commands.entity(label).insert((
                    Node {
                        position_type: PositionType::Absolute,
                        top: px(y - 8.),
                        left: px(17.),
                        max_width: px(WIDTH - if def.shading { 34. } else { PREVIEW_EDGE + 34. }),
                        max_height: px(19.),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            let preview_node = Node {
                position_type: PositionType::Absolute,
                width: px(PREVIEW_EDGE),
                height: px(PREVIEW_EDGE),
                top: px(PREVIEW_TOP),
                right: px(9.),
                border_radius: BorderRadius::all(px(3.)),
                ..default()
            };
            let target = def.params.iter()
                .position(|param| param.kind == material_painter_graph::ParamKind::Image)
                .map(|index| material_graph::ParameterTarget::Node(id, index))
                .filter(|&target| {
                    (managed.is_none() && def.id == "bitmap")
                        || (def.id == "layer" && folder.is_some_and(|stack|
                            folder_graph::bitmap_target(stack, &graph, target)))
                });
            let preview = if def.shading {
                None
            } else if let Some(target) = target {
                let pick = PaintAction::GraphBitmapPick(target);
                let button = control(
                    commands,
                    card,
                    Action::ModelView(ModelViewAction::Paint(pick)),
                    Look::Regular,
                    preview_node,
                );
                let label = if def.id == "layer" { "Выбрать текстуру канала" } else { "Выбрать изображение Bitmap" };
                commands
                    .entity(button)
                    .insert((PaintButton(pick), AccessibleLabel(label.into())));
                control_hints::attach(commands, button, label);
                Some(button)
            } else {
                Some(
                    commands
                        .spawn((
                            ChildOf(card),
                            preview_node,
                            BackgroundColor(WELL),
                            Pickable::IGNORE,
                        ))
                        .id(),
                )
            };
            #[cfg(test)]
            if let Some(preview) = preview {
                commands.entity(preview).insert(NodePreview(id));
            }
            self.cards.insert(
                id,
                Card {
                    entity: card,
                    output,
                    preview,
                    origin: Vec2::from_array(node_data.pos),
                    height,
                    source: (managed.is_none() && !node_library::is_embedded(&def.id))
                        .then(|| def.id.clone()),
                },
            );
        }
        self.show_previews(commands);
        self.dirty = true;
    }

    fn show_previews(&self, commands: &mut Commands) {
        for (&id, card) in &self.cards {
            let Some(preview) = card.preview else {
                continue;
            };
            if let Some(image) = self.previews.image(id) {
                commands
                    .entity(preview)
                    .insert(ImageNode::new(image.clone()));
            } else {
                commands.entity(preview).remove::<ImageNode>();
            }
        }
    }

    fn frame(&mut self, only_selected: bool) {
        if self.cards.is_empty() {
            self.pan = Vec2::splat(24.);
            self.zoom = 1.;
            return;
        }
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for (&id, item) in &self.cards {
            if only_selected && !self.selection.contains(&id) { continue; }
            min = min.min(item.origin);
            max = max.max(item.origin + Vec2::new(WIDTH, item.height));
        }
        self.zoom = ((self.size - Vec2::splat(44.)) / (max - min))
            .min_element()
            .clamp(0.24, 1.25);
        self.pan = (self.size - (min + max) * self.zoom) * 0.5;
    }

    fn new_node_position(&mut self, height: f32) -> Vec2 {
        let middle = (self.size * 0.5 - self.pan) / self.zoom - Vec2::new(WIDTH, height) * 0.5;
        let view = Rect::from_corners(-self.pan / self.zoom, (self.size - self.pan) / self.zoom);
        let new_size = Vec2::new(WIDTH + 9., height + 9.);
        for distance in 0_i32..=8 {
            for y in -distance..=distance {
                for x in -distance..=distance {
                    if x.abs().max(y.abs()) != distance {
                        continue;
                    }
                    let pos = middle + Vec2::new(x as f32 * 96., y as f32 * 68.);
                    let candidate = Rect::from_corners(pos, pos + new_size);
                    if view.contains(candidate.min)
                        && view.contains(candidate.max)
                        && self.cards.values().all(|card| {
                            let area = Rect::from_corners(
                                card.origin,
                                card.origin + Vec2::new(WIDTH + 9., card.height + 9.),
                            );
                            area.intersect(candidate).is_empty()
                        })
                    {
                        return pos;
                    }
                }
            }
        }
        self.fit_new_node = true;
        let right = self
            .cards
            .values()
            .map(|card| card.origin.x + WIDTH + 24.)
            .fold(middle.x, f32::max);
        Vec2::new(right, middle.y)
    }

    fn origin(&self, id: u64) -> Option<Vec2> {
        let pos = self.cards.get(&id)?.origin;
        let shift = self
            .dragging
            .filter(|_| self.selection.contains(&id))
            .unwrap_or(Vec2::ZERO);
        Some((pos + shift) * self.zoom + self.pan)
    }

    fn publish_selection(&mut self, messages: &mut MessageWriter<Action>) {
        self.selected = self.selection.last().copied();
        self.dirty = true;
        action(messages, self.selected.map_or(PaintAction::GraphClearSelection, PaintAction::GraphSelectNode));
    }

    fn selected_source(&self) -> Option<(u64, &str)> {
        let &[id] = self.selection.as_slice() else { return None };
        Some((id, self.cards.get(&id)?.source.as_deref()?))
    }

    fn copy_source(&mut self, maker: &ShaderMaker) -> Result<(), String> {
        if self.source_copy.is_some() { return Ok(()); }
        let Some((node, original)) = self.selected_source()
            .filter(|(_, id)| material_graph::replaceable_builtin(id))
        else { return Ok(()) };
        let (Some(target), Some(payload)) = (self.target, self.payload.clone()) else { return Ok(()) };
        let directory = maker.directory().ok_or("Папка пользовательских узлов недоступна")?;
        let original = original.to_owned();
        let task = IoTaskPool::get().spawn(async move {
            let (path, code) = node_library::copy_builtin(&directory, &original)?;
            Ok((node_library::node_id(&path)?, code))
        });
        self.source_copy = Some(SourceCopy { target, payload, node, state: SourceCopyState::Writing(task) });
        Ok(())
    }

    fn update_source_copy(
        &mut self,
        active: Option<(usize, &Session, &Arc<serde_json::Value>)>,
        maker: &mut ShaderMaker,
        messages: &mut MessageWriter<Action>,
    ) -> Result<(), String> {
        let Some(mut copy) = self.source_copy.take() else { return Ok(()) };
        let active = active.filter(|(model, session, _)| copy.target == Target::for_session(*model, session));
        match &mut copy.state {
            SourceCopyState::Writing(task) => {
                let Some(result) = block_on(poll_once(task)) else {
                    self.source_copy = Some(copy);
                    return Ok(());
                };
                let (definition, code) = result?;
                let kind = BatchKind::ReplaceSource;
                let Some((_, session, payload)) = active
                    .filter(|(_, session, payload)| Arc::ptr_eq(&copy.payload, payload)
                        && session.available(PaintAction::GraphEdit(Edit::Batch(0, kind))))
                else { return Err("Копия сохранена в библиотеке · исходный граф изменился".into()) };
                if session.graph_node != Some(copy.node) || self.batch.is_some() {
                    return Err("Копия сохранена в библиотеке · выделение или граф изменились".into());
                }
                let batch = Batch::ReplaceSource { node: copy.node, definition: definition.clone() };
                self.queue_batch(batch, kind, messages);
                copy.state = SourceCopyState::Applying { definition, code };
                copy.payload = payload.clone();
                self.source_copy = Some(copy);
            }
            SourceCopyState::Applying { definition, code } => {
                let Some((_, session, payload)) = active else { return Ok(()) };
                if Arc::ptr_eq(&copy.payload, payload) && session.smart_loading {
                    self.source_copy = Some(copy);
                    return Ok(());
                }
                if !Arc::ptr_eq(&copy.payload, payload)
                    && material_graph::read(payload)?.node(copy.node).is_some_and(|node| node.def == *definition)
                {
                    if session.graph_node == Some(copy.node) {
                        maker.request_graph_source(copy.target.model(), copy.node, definition.clone(), code.clone());
                        action(messages, PaintAction::Panel(Panel::Shader));
                    }
                } else {
                    return Err("Копия сохранена в библиотеке · узел на холсте не заменён".into());
                }
            }
        }
        Ok(())
    }

    fn queue_batch(&mut self, batch: Batch, kind: BatchKind, messages: &mut MessageWriter<Action>) {
        let Some(payload) = self.payload.clone() else { return; };
        self.next_batch = self.next_batch.wrapping_add(1);
        self.batch = Some((self.next_batch, payload, batch));
        action(messages, PaintAction::GraphEdit(Edit::Batch(self.next_batch, kind)));
    }

    pub(super) fn take_batch(&mut self, id: u64, model: usize, session: &Session) -> Option<Batch> {
        if self.batch.as_ref().is_none_or(|(serial, _, _)| *serial != id) { return None; }
        let (_, source, batch) = self.batch.take()?;
        (self.target == Some(Target::for_session(model, session)) && Arc::ptr_eq(&source, session.graph()?))
            .then_some(batch)
    }

    fn remove_selected(&mut self, messages: &mut MessageWriter<Action>) {
        let ids: Vec<_> = self.selection.iter().copied().filter(|id| {
            self.folder_labels.as_ref().is_none_or(|labels| !labels.contains_key(id))
        }).collect();
        if !ids.is_empty() { self.queue_batch(Batch::Remove(ids), BatchKind::Remove, messages); }
    }

    fn copy_selection(&self, graph: &Graph) -> Option<Clipboard> {
        let nodes: Vec<_> = graph.nodes.iter().filter(|node| {
            self.selection.contains(&node.id) && (self.folder_labels.is_none() || folder_graph::managed(graph, node.id).is_none())
        }).cloned().collect();
        if nodes.is_empty() { return None; }
        let graph_images = self.graph_images.as_deref()?;
        let images = nodes.iter().flat_map(|node| node.params.values()).filter_map(|value| {
            let material_painter_graph::Value::Image(Some(path)) = value else { return None; };
            let key = path.to_str()?;
            Some((key.to_owned(), graph_images.get(key)?.clone()))
        }).collect();
        Some(Clipboard { target: self.target?, nodes, images: Arc::new(images) })
    }

    fn keyboard(&mut self, keys: &ButtonInput<KeyCode>, cursor: Vec2, messages: &mut MessageWriter<Action>) {
        if keys.any_just_pressed([KeyCode::Space, KeyCode::Tab]) {
            self.open_palette_at(self.stage_origin + cursor, None);
            return;
        }
        if keys.just_pressed(KeyCode::KeyF) {
            self.frame(!self.selection.is_empty());
            self.dirty = true;
        }
        if keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace]) {
            self.remove_selected(messages);
            return;
        }
        if !keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight])
            || !keys.any_just_pressed([KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyV]) { return; }
        let Some(graph) = self.payload.as_ref().and_then(|payload| material_graph::read(payload).ok()) else { return; };
        if keys.just_pressed(KeyCode::KeyC) {
            if let Some(clipboard) = self.copy_selection(&graph) { self.clipboard = Some(clipboard); }
            return;
        }
        let duplicate = keys.just_pressed(KeyCode::KeyD);
        let source = if duplicate { self.copy_selection(&graph) } else {
            self.clipboard.as_ref().map(|clip| Clipboard { target: clip.target, nodes: clip.nodes.clone(), images: clip.images.clone() })
        };
        if let Some(Clipboard { target, nodes, images }) = source {
            let min = nodes.iter().map(|node| Vec2::from_array(node.pos)).fold(Vec2::splat(f32::INFINITY), Vec2::min);
            let offset = if duplicate { Vec2::splat(40.) } else { (cursor - self.pan) / self.zoom - min };
            let keep_links = self.target == Some(target);
            self.queue_batch(Batch::Paste { nodes, offset, keep_links, images }, BatchKind::Paste(graph.next_id), messages);
        }
    }

    fn wire_origin(&self) -> Option<u64> {
        self.wire_drag.as_ref().map(|drag| drag.source.node).or(self.source)
    }

    fn start_wire(&mut self, handle: Entity, source: WireSource) {
        self.source = None;
        self.wire_drag = Some(WireDrag { handle, source });
        self.dirty = true;
    }

    fn take_wire(&mut self, handle: Entity) -> Option<WireSource> {
        if self.wire_drag.as_ref()?.handle != handle { return None; }
        self.dirty = true;
        self.wire_drag.take().map(|drag| drag.source)
    }

    fn cancel_wire(&mut self) {
        if self.wire_drag.take().is_some() {
            self.dirty = true;
        }
    }

    fn open_palette_at(&mut self, position: Vec2, source: Option<WireSource>) {
        let point = position - self.stage_origin;
        if Rect::from_corners(Vec2::ZERO, self.size).contains(point) {
            let graph = ((point - self.pan) / self.zoom).round().as_ivec2();
            self.source = None;
            self.dirty = true;
            self.palette.at(point, graph, source);
        }
    }

    fn pick_palette_node(&mut self, (index, height): (usize, f32), messages: &mut MessageWriter<Action>) {
        if !self.palette.is_open() { return; }
        let position = if self.palette.uses_default_position() {
            self.new_node_position(height).round().as_ivec2()
        } else { IVec2::ZERO };
        if let Some(edit) = self.palette.pick(index, position) {
            action(messages, PaintAction::GraphEdit(edit));
        }
    }
}

#[cfg(test)]
#[derive(Component)]
pub(in crate::editor::ui::model_viewer::paint) struct NodePreview(pub u64);

fn card_height(inputs: usize, shading: bool) -> f32 {
    let ports = PORT_Y + ROW * inputs.max(1) as f32 - 7.;
    if shading {
        ports
    } else {
        ports.max(PREVIEW_TOP + PREVIEW_EDGE + 9.)
    }
}

fn tint(kind: Kind) -> Color {
    match kind {
        Kind::Gray => GRAY,
        Kind::Color => COLOR,
    }
}

fn pin(
    commands: &mut Commands,
    parent: Entity,
    x: f32,
    y: f32,
    tint: Color,
    name: String,
) -> Entity {
    commands
        .spawn((
            Button,
            AccessibleLabel(name),
            ChildOf(parent),
            Node {
                position_type: PositionType::Absolute,
                left: px(x - 5.),
                top: px(y - 5.),
                width: px(10.),
                height: px(10.),
                border_radius: BorderRadius::MAX,
                border: UiRect::all(px(1.)),
                ..default()
            },
            BackgroundColor(tint),
            BorderColor::all(WELL),
        ))
        .id()
}

fn wire(commands: &mut Commands, stage: Entity, color: Color) -> [Entity; SEGMENTS] {
    std::array::from_fn(|_| {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    ..default()
                },
                ChildOf(stage),
                BackgroundColor(color),
                Pickable::IGNORE,
            ))
            .id()
    })
}

fn place_wire(
    commands: &mut Commands,
    nodes: &mut Query<&mut Node>,
    wire: &[Entity; SEGMENTS],
    ends: Option<(Vec2, Vec2)>,
) {
    let Some((from, to)) = ends else {
        for &segment in wire {
            if let Ok(mut node) = nodes.get_mut(segment) {
                node.display = Display::None;
            }
        }
        return;
    };
    let tangent = Vec2::X * ((to.x - from.x).abs() * 0.45).clamp(24., 160.);
    let a = from + tangent;
    let b = to - tangent;
    let mut previous = from;
    for (index, &segment) in wire.iter().enumerate() {
        let t = (index + 1) as f32 / SEGMENTS as f32;
        let start = from.lerp(a, t);
        let middle = a.lerp(b, t);
        let end = b.lerp(to, t);
        let next = start.lerp(middle, t).lerp(middle.lerp(end, t), t);
        if let Ok(mut node) = nodes.get_mut(segment) {
            let direction = next - previous;
            let center = (previous + next) * 0.5;
            let width = direction.length() + 1.;
            node.display = Display::Flex;
            node.left = px(center.x - width * 0.5);
            node.top = px(center.y - 1.);
            node.width = px(width);
            node.height = px(2.);
            commands
                .entity(segment)
                .insert(UiTransform::from_rotation(Rot2::radians(
                    direction.to_angle(),
                )));
        }
        previous = next;
    }
}

pub(in crate::editor::ui::model_viewer) fn refresh(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    mut ui: ResMut<PaintUi>,
    interface: Res<Interface>,
    mut nodes: Query<&mut Node>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut labels: Query<&mut Text>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut uploads: ResMut<Uploads>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
    queue: Option<Res<bevy::render::renderer::RenderQueue>>,
    baked: Res<bake::State>,
    workspace: (Res<shader_preview::Cache>, Res<sidebar::State>, Res<ButtonInput<KeyCode>>,
        Res<ButtonInput<MouseButton>>, ResMut<bevy::input_focus::InputFocus>, Query<&mut bevy::text::EditableText>,
        Query<(Entity, &catalogue::PresetSwatch)>, MessageWriter<Action>, ResMut<ShaderMaker>),
) {
    let (shaders, shelf, keys, mouse, mut focus, mut editable, swatches, mut messages, mut maker) = workspace;
    let canvas = &mut ui.graph;
    let context = editor
        .viewing_model
        .filter(|id| editing.mode(Some(*id)) == Mode::Paint)
        .filter(|_| {
            editor.pending.is_none()
                && editor.viewing_image.is_none()
                && !editor.browser_popup
                && !editor.hdri_library_open
        })
        .and_then(|model| paint.sessions.get(&model).map(|session| (model, session)));
    let workspace = context.filter(|(_, session)| session.graph_canvas_open());
    let catalogue = workspace.and_then(|(model, session)| {
        Some((model, session, session.graph_catalogue()?, editing.documents.get(&model)?))
    });
    let graph = workspace
        .filter(|(_, session)| session.graph_catalogue().is_none())
        .and_then(|(model, session)| {
            session
                .graph_data()
                .map(|(payload, images)| (model, session, payload, images))
        });
    if let Err(error) = canvas.update_source_copy(graph.map(|(model, session, payload, _)| (model, session, payload)), &mut maker, &mut messages) {
        editor.status = error;
    }
    let Ok(mut root) = nodes.get_mut(canvas.root) else {
        return;
    };
    root.display = if workspace.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    let shading = graph.is_some_and(|(_, session, _, _)| session.editing_shader().is_some());
    root.bottom = px(if shelf.shelf_in_sidebar() { 0. } else { SHELF_HEIGHT });
    let mut display = |entity, show| {
        let next = if show { Display::Flex } else { Display::None };
        if let Ok(mut node) = nodes.get_mut(entity) && node.display != next {
            node.display = next;
        }
    };
    display(canvas.panes, catalogue.is_none());
    display(canvas.catalogue, catalogue.is_some());
    for (index, entity) in canvas.catalogue_nav.into_iter().enumerate() {
        display(entity, !shading && (index != 2 || catalogue.is_some()));
    }
    display(canvas.reset_layout, graph.is_some());
    let renderer = device.as_deref().zip(queue.as_deref());
    if catalogue.is_some_and(|(_, _, page, _)| page == GraphPage::Home) {
        canvas.preset_previews.prepare();
    }
    if canvas.preset_previews.update(&mut images, &mut uploads, renderer) {
        for (entity, swatch) in &swatches {
            if let Some(image) = canvas.preset_previews.image(swatch.0) {
                commands.entity(entity).insert(ImageNode::new(image.clone()));
            }
        }
    }
    if let Some((model, session, page, doc)) = catalogue {
        let stamp = (model, page, session.revision, doc.revision, doc.ui_revision, paint.graph_shelf.revision);
        if canvas.catalogue_stamp != Some(stamp) {
            if canvas.catalogue_stamp.is_none_or(|previous| (previous.0, previous.1) != (model, page)) {
                commands.entity(canvas.catalogue).insert(ScrollPosition::default());
            }
            catalogue::populate(&mut commands, canvas.catalogue, &interface.font, page, model, doc, session,
                &paint.graph_shelf.entries, &canvas.preset_previews, &mut images);
            canvas.catalogue_stamp = Some(stamp);
        }
    } else if canvas.catalogue_stamp.take().is_some() {
        commands.entity(canvas.catalogue).despawn_related::<Children>();
    }
    let diagnostic = graph.and_then(|(model, session, _, _)| {
        session.editing_shader()?;
        shaders.diagnostic(model, session.preview_shader?)
    });
    if let Ok(mut node) = nodes.get_mut(canvas.diagnostic) {
        node.display = if diagnostic.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut label) = labels.get_mut(canvas.diagnostic)
        && label.0 != diagnostic.unwrap_or_default()
    {
        label.0 = diagnostic.unwrap_or_default().to_owned();
    }
    let Some((model, session, payload, graph_images)) = graph else {
        if canvas.target.take().is_some() {
            canvas.payload = None;
            canvas.graph_images = None;
            canvas.folder_labels = None;
            canvas.bake_maps = None;
            canvas.graph_surface = None;
            canvas.source = None;
            canvas.wire_drag = None;
            canvas.dragging = None;
            canvas.selection.clear();
            canvas.selected = None;
            canvas.marquee = None;
            canvas.batch = None;
            canvas.palette.close();
            canvas.secondary_pan = false;
            canvas.typing = false;
            canvas.fit_new_node = false;
            canvas.previews.reset();
            canvas.cards.clear();
            canvas.wires.clear();
            canvas.pending = None;
            commands.entity(canvas.stage).despawn_related::<Children>();
        }
        canvas.palette.clear_focus(&mut focus);
        canvas.previews.update(&mut images, &mut uploads, renderer);
        return;
    };
    let folder_labels = session.active_folder_view().map(|view| &view.labels);
    let folder = folder_labels.is_some();
    let topology_display = if folder { Display::None } else { Display::Flex };
    for entity in canvas.export_tools {
        if let Ok(mut tool) = nodes.get_mut(entity) && tool.display != topology_display {
            tool.display = topology_display;
        }
    }
    let allow_delete = canvas.selection.iter().any(|&id|
        session.available(PaintAction::GraphEdit(Edit::Remove(id))));
    let delete_display = if allow_delete { Display::Flex } else { Display::None };
    if let Ok(mut button) = nodes.get_mut(canvas.delete) && button.display != delete_display {
        button.display = delete_display;
    }
    let (library, archived) = node_library::palette();
    if canvas
        .library
        .as_ref()
        .is_none_or(|current| !Arc::ptr_eq(current, &library))
    {
        canvas.palette.invalidate();
        canvas.library = Some(library.clone());
    }
    let key = Target::for_session(model, session);
    if canvas.target != Some(key) {
        canvas.target = Some(key);
        canvas.source = None;
        canvas.wire_drag = None;
        canvas.selection.clear();
        canvas.selected = None;
        canvas.dragging = None;
        canvas.marquee = None;
        canvas.batch = None;
        canvas.palette.close();
        canvas.palette.clear_focus(&mut focus);
        canvas.secondary_pan = false;
        canvas.typing = false;
        canvas.fit = true;
        canvas.fit_new_node = false;
        canvas.payload = None;
        canvas.graph_surface = None;
        canvas.previews.reset();
    }
    let changed = canvas
        .payload
        .as_ref()
        .is_none_or(|p| !Arc::ptr_eq(p, payload))
        || canvas
            .graph_images
            .as_ref()
            .is_none_or(|p| !Arc::ptr_eq(p, graph_images))
        || !material_graph::mesh_maps::same(canvas.folder_labels.as_ref(), folder_labels);
    let graph = changed
        .then(|| material_graph::read(payload).unwrap_or_else(|_| Graph::new("Граф недоступен")));
    if let Some(graph) = &graph {
        canvas.needs_surface = !shading && material_graph::surface::used(graph);
    }
    let surface = if canvas.needs_surface {
        session.binding.and_then(|binding| {
            let doc = editing.documents.get(&model)?;
            canvas
                .graph_surface
                .as_ref()
                .or_else(|| session.graph_surfaces.get(&binding))
                .filter(|source| source.matches(doc, binding))
                .cloned()
                .or_else(|| material_graph::surface::Source::capture(doc, binding).map(Arc::new))
        })
    } else {
        None
    };
    let maps = session.binding.filter(|_| !shading).and_then(|binding| {
        let doc = editing.documents.get(&model)?;
        baked.maps(model, doc, binding, session.texture_size)
    });
    if changed
        || !material_graph::mesh_maps::same(canvas.bake_maps.as_ref(), maps)
        || !material_graph::mesh_maps::same(canvas.graph_surface.as_ref(), surface.as_ref())
    {
        let graph = graph.unwrap_or_else(|| {
            material_graph::read(payload).unwrap_or_else(|_| Graph::new("Граф недоступен"))
        });
        canvas.bake_maps = maps.cloned();
        if !shading {
            canvas
                .previews
                .set(&graph, graph_images, maps, surface.as_ref());
        }
        canvas.graph_surface = surface;
        if changed {
            canvas.payload = Some(payload.clone());
            canvas.graph_images = Some(graph_images.clone());
            canvas.folder_labels = folder_labels.cloned();
            canvas.previews.update(&mut images, &mut uploads, renderer);
            if let Ok(mut title) = labels.get_mut(canvas.title) {
                title.0 = graph.name.clone();
            }
            canvas.rebuild(&mut commands, &interface.font, graph, folder.then_some(&session.stack));
            // The children enter layout after the commands are applied.
            return;
        }
    }
    if canvas.previews.update(&mut images, &mut uploads, renderer) {
        canvas.show_previews(&mut commands);
    }
    let Ok((stage, transform)) = geometry.get(canvas.stage) else {
        return;
    };
    let size = stage.size() * stage.inverse_scale_factor();
    if size.min_element() < 1. {
        return;
    }
    canvas.stage_origin = transform.affine().translation * stage.inverse_scale_factor() - size * 0.5;
    let cursor = window.cursor_position().map(|p| p - canvas.stage_origin);
    if size != canvas.size {
        canvas.size = size;
        canvas.dirty = true;
    }
    if keys.just_pressed(KeyCode::Escape) || !window.focused {
        canvas.cancel_wire();
    }
    let typing = focus.get().is_some_and(|field| editable.get(field).is_ok());
    // Tab navigation can focus a text field before this system sees the shortcut.
    let tab_on_canvas = keys.just_pressed(KeyCode::Tab) && !canvas.typing;
    canvas.typing = typing;
    if let Some(point) = cursor.filter(|point| Rect::from_corners(Vec2::ZERO, size).contains(*point))
        && (!typing || tab_on_canvas)
        && !canvas.palette.is_open()
        && canvas.wire_drag.is_none()
        && canvas.dragging.is_none() && canvas.marquee.is_none()
    {
        canvas.keyboard(&keys, point, &mut messages);
    }
    let pane_origin = geometry.get(canvas.graph_pane).map_or(canvas.stage_origin, |(node, transform)| {
        (transform.affine().translation - node.size() * 0.5) * node.inverse_scale_factor()
    });
    let stage_bounds = Rect::from_corners(canvas.stage_origin, canvas.stage_origin + size);
    if let Some(choice) = canvas.palette.update(&mut commands, &mut nodes, &interface.font, &library,
        &archived, shading, folder, stage_bounds, pane_origin, window.cursor_position(), &mouse,
        &keys, &mut focus, &mut editable)
    {
        canvas.pick_palette_node(choice, &mut messages);
    }
    if let Some(entity) = canvas.marquee_node && let Ok(mut node) = nodes.get_mut(entity) {
        if let Some(rect) = canvas.marquee {
            node.left = px(rect.min.x);
            node.top = px(rect.min.y);
            node.width = px(rect.width());
            node.height = px(rect.height());
            node.display = Display::Flex;
        } else {
            node.display = Display::None;
        }
    }
    if canvas.fit {
        canvas.frame(false);
        canvas.fit = false;
        canvas.dirty = true;
    }
    let current = session.graph_node.filter(|id| canvas.cards.contains_key(id));
    if canvas.selected != current {
        canvas.selected = current;
        canvas.selection = if let Some((first, last)) = session.graph_pasted.filter(|&(_, last)| current == Some(last)) {
            (first..=last).filter(|id| canvas.cards.contains_key(id)).collect()
        } else {
            current.into_iter().collect()
        };
        canvas.dirty = true;
    }
    let source = canvas.selected_source().map(|(_, id)| id);
    let open = source.is_some_and(|id| id.starts_with("user/") && node_library::available(id));
    let copy = source.is_some_and(material_graph::replaceable_builtin)
        && canvas.source_copy.is_none()
        && session.available(PaintAction::GraphEdit(Edit::Batch(0, BatchKind::ReplaceSource)));
    for (entity, visible) in [(canvas.edit_source, open), (canvas.copy_source, copy)] {
        if let Ok(mut button) = nodes.get_mut(entity) {
            let display = if visible { Display::Flex } else { Display::None };
            if button.display != display { button.display = display; }
        }
    }
    if canvas.dirty {
        for (&id, card) in &canvas.cards {
            let Some(top_left) = canvas.origin(id) else {
                continue;
            };
            if let Ok(mut node) = nodes.get_mut(card.entity) {
                let top_left = top_left + Vec2::new(WIDTH, card.height) * (canvas.zoom - 1.) * 0.5;
                node.left = px(top_left.x);
                node.top = px(top_left.y);
            }
            let border = if canvas.selection.contains(&id) {
                FOCUS
            } else {
                EDGE
            };
            commands.entity(card.entity).insert((
                UiTransform::from_scale(Vec2::splat(canvas.zoom)),
                BorderColor::all(border),
            ));
            commands
                .entity(card.output)
                .insert(BorderColor::all(if canvas.wire_origin() == Some(id) {
                    FOCUS
                } else {
                    WELL
                }));
        }
        let lifted = canvas.wire_drag.as_ref().and_then(|drag| drag.source.input);
        for wire in &canvas.wires {
            if lifted == Some(GraphInput { node: wire.target, index: wire.input }) {
                place_wire(&mut commands, &mut nodes, &wire.segments, None);
                continue;
            }
            let ends = canvas
                .origin(wire.source)
                .zip(canvas.origin(wire.target))
                .map(|(from, to)| {
                    (
                        from + Vec2::new(WIDTH - 6., PORT_Y) * canvas.zoom,
                        to + Vec2::new(5., PORT_Y + ROW * wire.input as f32) * canvas.zoom,
                    )
                });
            place_wire(&mut commands, &mut nodes, &wire.segments, ends);
        }
        canvas.dirty = false;
    }
    if let Some(wire) = &canvas.pending {
        let ends = canvas
            .wire_origin()
            .and_then(|id| canvas.origin(id))
            .zip(cursor)
            .map(|(from, to)| (from + Vec2::new(WIDTH - 6., PORT_Y) * canvas.zoom, to));
        place_wire(&mut commands, &mut nodes, wire, ends);
    }
}
