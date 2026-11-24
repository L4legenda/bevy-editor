//! Node canvas for graph-backed paint layers.
use super::*;
use bevy::{
    input::mouse::MouseScrollUnit,
    picking::events::{PointerClick, PointerDrag, PointerDragEnd, PointerDragStart, PointerScroll},
};
use material_graph::previews::{Previews, Uploads};
use material_painter_graph::{Graph, Kind, Library};
use std::sync::Arc;

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
}

struct Wire {
    segments: [Entity; SEGMENTS],
    source: u64,
    target: u64,
    input: usize,
}

#[derive(Component)]
struct Output(u64);

pub(super) struct Canvas {
    root: Entity,
    pub(super) preview: Entity,
    pub(super) inspector: Entity,
    stage: Entity,
    title: Entity,
    diagnostic: Entity,
    palettes: [Entity; 2],
    export_tools: [Entity; 2],
    delete: Entity,
    library: Option<Arc<Library>>,
    open_palette: bool,
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
    selected: Option<u64>,
    dragging: Option<(u64, Vec2)>,
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

fn action(messages: &mut MessageWriter<Action>, action: PaintAction) {
    messages.write(Action::ModelView(ModelViewAction::Paint(action)));
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
        self.source
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
        let reset = local_button(commands, controls, font, "Сбросить раскладку графа");
        commands.entity(reset).observe(|event: On<PointerClick>, mut dock: ResMut<sidebar::GraphDock>| {
            if event.button == PointerButton::Primary { dock.reset(); }
        });
        button(commands, controls, font, "Закрыть холст", PaintAction::GraphCanvas);
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
                    ui.graph.open_palette = !ui.graph.open_palette;
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
        let delete = local_button(commands, toolbar, font, "Удалить узел");
        commands.entity(delete).observe(
            |event: On<PointerClick>, ui: Res<PaintUi>, mut messages: MessageWriter<Action>| {
                if event.button == PointerButton::Primary
                    && let Some(id) = ui.graph.selected
                {
                    action(
                        &mut messages,
                        PaintAction::GraphEdit(material_graph::Edit::Remove(id)),
                    );
                }
            },
        );
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
            "Выход → вход · ПКМ по входу — разъединить · Колесо — масштаб · СКМ — панорама",
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
            .insert(AccessibleLabel("Холст графа материала".into()))
            .observe(|event: On<PointerDrag>, mut ui: ResMut<PaintUi>| {
                if event.button != PointerButton::Primary && ui.graph.dragging.is_none() {
                    ui.graph.pan += event.delta;
                    ui.graph.dirty = true;
                }
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
            .observe(move |event: On<PointerClick>, mut ui: ResMut<PaintUi>| {
                if event.entity == stage && event.button == PointerButton::Primary {
                    ui.graph.source = None;
                    ui.graph.open_palette = false;
                    ui.graph.dirty = true;
                }
            });
        let palettes = [false, true].map(|_| {
            let palette = surface(
                commands,
                root,
                Node {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    left: px(8),
                    top: px(43),
                    bottom: px(8),
                    width: px(244),
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(px(6)),
                    border: UiRect::all(px(1)),
                    ..column(4.)
                },
                CHROME,
            );
            commands
                .entity(palette)
                .insert((BorderColor::all(EDGE), ZIndex(10), Scroller));
            palette
        });
        Self {
            root: workspace,
            preview,
            inspector,
            stage,
            title,
            diagnostic,
            palettes,
            export_tools: [export, package],
            delete,
            library: None,
            open_palette: false,
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
            selected: None,
            dragging: None,
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
                    move |mut event: On<PointerClick>, mut messages: MessageWriter<Action>| {
                        if event.button == PointerButton::Primary {
                            action(&mut messages, PaintAction::GraphSelectNode(id));
                            event.propagate(false);
                        }
                    },
                )
                .observe(move |event: On<PointerDrag>, mut ui: ResMut<PaintUi>| {
                    if event.button == PointerButton::Primary {
                        ui.graph.dragging = Some((id, event.distance / ui.graph.zoom));
                        ui.graph.dirty = true;
                    }
                })
                .observe(
                    move |event: On<PointerDragEnd>,
                          mut ui: ResMut<PaintUi>,
                          mut messages: MessageWriter<Action>| {
                        if event.button != PointerButton::Primary {
                            return;
                        }
                        ui.graph.dragging = None;
                        ui.graph.dirty = true;
                        if let Some(card) = ui.graph.cards.get(&id) {
                            let pos = card.origin + event.distance / ui.graph.zoom;
                            action(
                                &mut messages,
                                PaintAction::GraphEdit(material_graph::Edit::Move(
                                    id,
                                    pos.round().as_ivec2(),
                                )),
                            );
                        }
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
                .insert(Output(id))
                .observe(
                    move |mut event: On<PointerClick>, mut ui: ResMut<PaintUi>| {
                        if event.button == PointerButton::Primary {
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
                            ui.graph.source = Some(id);
                            ui.graph.dirty = true;
                        }
                    },
                );
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
                    commands
                    .entity(input)
                    .observe(
                        move |mut click: On<PointerClick>,
                              mut ui: ResMut<PaintUi>,
                              mut messages: MessageWriter<Action>| {
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
                              outputs: Query<&Output>,
                              mut ui: ResMut<PaintUi>,
                              mut messages: MessageWriter<Action>| {
                            if event.button == PointerButton::Primary
                                && let Ok(source) = outputs.get(event.dropped)
                            {
                                action(
                                    &mut messages,
                                    PaintAction::GraphEdit(material_graph::Edit::Connect(
                                        source.0, id, index,
                                    )),
                                );
                                ui.graph.source = None;
                                ui.graph.dirty = true;
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

    fn frame_all(&mut self) {
        if self.cards.is_empty() {
            self.pan = Vec2::splat(24.);
            self.zoom = 1.;
            return;
        }
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for item in self.cards.values() {
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
            .filter(|(node, _)| *node == id)
            .map_or(Vec2::ZERO, |(_, shift)| shift);
        Some((pos + shift) * self.zoom + self.pan)
    }
}

fn populate_palette(
    commands: &mut Commands,
    palette: Entity,
    font: &Handle<Font>,
    library: &Library,
    archived: &std::collections::HashSet<String>,
    shading: bool,
) {
    commands.entity(palette).despawn_related::<Children>();
    for (category, ids) in library.categories() {
        let mut nodes = library
            .defs
            .iter()
            .enumerate()
            .filter(|(_, def)| {
                def.shading == shading
                    && ids.contains(&def.id)
                    && node_library::visible(&def.id, archived)
            })
            .peekable();
        if nodes.peek().is_none() {
            continue;
        }
        text(commands, palette, font, &category, 11., MUTED);
        for (index, def) in nodes {
            let height = card_height(def.inputs.len(), def.shading);
            let button = local_button(commands, palette, font, &def.name);
            commands.entity(button).observe(
                move |event: On<PointerClick>,
                      mut ui: ResMut<PaintUi>,
                      mut messages: MessageWriter<Action>| {
                    if event.button != PointerButton::Primary {
                        return;
                    }
                    let graph = &mut ui.graph;
                    let pos = graph.new_node_position(height);
                    graph.open_palette = false;
                    action(
                        &mut messages,
                        PaintAction::GraphEdit(material_graph::Edit::Add(
                            index,
                            pos.round().as_ivec2(),
                        )),
                    );
                },
            );
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
    editor: Res<Editor>,
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
    workspace: (Res<shader_preview::Cache>, Res<sidebar::State>),
) {
    let (shaders, shelf) = workspace;
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
    let graph = context
        .filter(|(_, session)| session.graph_canvas_open())
        .and_then(|(model, session)| {
            session
                .graph_data()
                .map(|(payload, images)| (model, session, payload, images))
        });
    let Ok(mut root) = nodes.get_mut(canvas.root) else {
        return;
    };
    root.display = if graph.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    let shading = graph.is_some_and(|(_, session, _, _)| session.editing_shader().is_some());
    root.bottom = px(if shelf.shelf_in_sidebar() { 0. } else { SHELF_HEIGHT });
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
    let renderer = device.as_deref().zip(queue.as_deref());
    let Some((model, session, payload, graph_images)) = graph else {
        if canvas.target.take().is_some() {
            canvas.payload = None;
            canvas.graph_images = None;
            canvas.folder_labels = None;
            canvas.bake_maps = None;
            canvas.graph_surface = None;
            canvas.source = None;
            canvas.dragging = None;
            canvas.open_palette = false;
            canvas.fit_new_node = false;
            canvas.previews.reset();
            canvas.cards.clear();
            canvas.wires.clear();
            canvas.pending = None;
            commands.entity(canvas.stage).despawn_related::<Children>();
        }
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
    let allow_delete = !folder || session.graph_node.is_some_and(|id|
        session.available(PaintAction::GraphEdit(material_graph::Edit::Remove(id))));
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
        for (palette, shading) in canvas.palettes.into_iter().zip([false, true]) {
            populate_palette(
                &mut commands,
                palette,
                &interface.font,
                &library,
                &archived,
                shading,
            );
        }
        canvas.library = Some(library);
    }
    let key = if shading {
        Target::Shader(model, session.preview_shader.unwrap())
    } else {
        Target::Texture(
            model,
            session.set,
            session.stack.selected,
            session.editing_mask,
        )
    };
    if canvas.target != Some(key) {
        canvas.target = Some(key);
        canvas.source = None;
        canvas.open_palette = false;
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
    let cursor = window
        .cursor_position()
        .map(|p| p - transform.affine().translation * stage.inverse_scale_factor() + size * 0.5);
    if size != canvas.size {
        canvas.size = size;
        canvas.dirty = true;
    }
    for (index, entity) in canvas.palettes.into_iter().enumerate() {
        if let Ok(mut palette) = nodes.get_mut(entity) {
            palette.display = if canvas.open_palette && index == usize::from(shading) {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    if canvas.fit {
        canvas.frame_all();
        canvas.fit = false;
        canvas.dirty = true;
    }
    if canvas.selected != session.graph_node {
        canvas.selected = session.graph_node;
        canvas.dirty = true;
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
            let border = if canvas.selected == Some(id) {
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
                .insert(BorderColor::all(if canvas.source == Some(id) {
                    FOCUS
                } else {
                    WELL
                }));
        }
        for wire in &canvas.wires {
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
            .source
            .and_then(|id| canvas.origin(id))
            .zip(cursor)
            .map(|(from, to)| (from + Vec2::new(WIDTH - 6., PORT_Y) * canvas.zoom, to));
        place_wire(&mut commands, &mut nodes, wire, ends);
    }
}
