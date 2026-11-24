//! Presentation-only graph transitions. Observing model changes also covers undo/redo.
use super::*;
use std::collections::BTreeMap;

const NODE_TIME: f32 = 0.38;
const LINK_TIME: f32 = 0.58;
const GHOST_TIME: f32 = 0.52;
const MAX_EFFECTS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Edge(pub u32, pub usize, pub u32);

#[derive(Clone)]
struct CardSnapshot {
    id: u32,
    position: Vec2,
    size: Vec2,
    accent: Color,
}
#[derive(Clone, Default)]
struct Snapshot {
    cards: BTreeMap<u32, CardSnapshot>,
    edges: BTreeMap<Edge, (Vec2, Vec2)>,
}
impl Snapshot {
    #[cfg(test)]
    fn read(d: &Dialogue) -> Self {
        Self::read_animated(d, &answers::State::default())
    }
    fn read_animated(d: &Dialogue, answers: &answers::State) -> Self {
        let cards = d
            .nodes
            .iter()
            .map(|n| {
                (
                    n.id,
                    CardSnapshot {
                        id: n.id,
                        position: Vec2::from_array(n.position),
                        size: Vec2::new(NODE_WIDTH, answers.height(n)),
                        accent: nodes::accent(n),
                    },
                )
            })
            .collect();
        let mut edges = BTreeMap::new();
        for n in &d.nodes {
            for (i, target) in n.output_targets() {
                if let Some(next) = d.node(target) {
                    edges.insert(
                        Edge(n.id, answers.key(n.id, i), next.id),
                        (
                            Vec2::from_array(n.position) + answers.output_position(n, i),
                            Vec2::from_array(next.position) + nodes::input_position(),
                        ),
                    );
                }
            }
        }
        Self { cards, edges }
    }
}

#[derive(Resource, Default)]
pub(super) struct State {
    document: Option<usize>,
    root: Option<Entity>,
    previous: Snapshot,
    births: BTreeMap<u32, f64>,
    arrivals: BTreeMap<Edge, f64>,
    now: f64,
    pub(super) answers: answers::State,
}
impl State {
    pub fn scale(&self, id: u32) -> f32 {
        self.scale_at(id, self.now)
    }
    pub fn scale_at(&self, id: u32, now: f64) -> f32 {
        let Some(start) = self.births.get(&id) else {
            return 1.;
        };
        let t = ((now - start) / NODE_TIME as f64).clamp(0., 1.) as f32;
        // Small overshoot, with exact resting geometry and no delay to interaction.
        1. - 0.025 * (1. - t).powi(2) * (t * std::f32::consts::PI * 1.6).cos()
    }
    pub fn edge(&self, from: u32, index: usize, target: u32) -> Edge {
        Edge(from, self.answers.key(from, index), target)
    }
    pub fn arrival(&self, edge: Edge) -> Option<f32> {
        self.arrivals
            .get(&edge)
            .map(|start| ((self.now - start) / LINK_TIME as f64).clamp(0., 1.) as f32)
    }
}

pub(super) fn point(state: &State, doc: &Document, n: &DialogueNode, local: Vec2) -> Vec2 {
    let center = Vec2::new(NODE_WIDTH, state.answers.height(n)) * 0.5;
    (Vec2::from_array(n.position) + center + (local - center) * state.scale(n.id)) * doc.zoom
        + doc.pan
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct Material {
    #[uniform(0)]
    metrics: Vec4,
    #[uniform(1)]
    tint: Vec4,
}
impl UiMaterial for Material {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("narrative_effect.wesl"))
                .with_source("embedded"),
        )
    }
}
pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "narrative_effect.wesl");
    app.add_plugins(UiMaterialPlugin::<Material>::default())
        .init_resource::<State>();
}

#[derive(Clone)]
enum Shape {
    Card(CardSnapshot, bool),
    Wire(Edge, Vec2, Vec2),
}
impl Shape {
    fn obsolete(&self, next: &Snapshot) -> bool {
        match self {
            Self::Card(card, removing) => next.cards.contains_key(&card.id) == *removing,
            Self::Wire(edge, _, _) => next.edges.contains_key(edge),
        }
    }
}

#[derive(Component)]
pub(super) struct Visual {
    shape: Shape,
    started: f64,
}

pub(super) fn observe(
    mut commands: Commands,
    editor: Res<Editor>,
    ui: Res<NarrativeUi>,
    mut state: ResMut<State>,
    time: Res<Time>,
    visuals: Query<(Entity, &Visual)>,
    mut cards: ResMut<Assets<Material>>,
    mut wires: ResMut<Assets<connection_paint::Material>>,
) {
    let now = time.elapsed_secs_f64();
    state.now = now;
    let current = editor
        .narrative
        .current()
        .and_then(|doc| match &doc.content {
            Content::Dialogue(d) => Some((doc, d)),
            _ => None,
        });
    state
        .answers
        .observe(current.map(|(doc, d)| (doc.id, d)), now);
    let context = current.map(|(doc, _)| doc.id);
    let switched = state.document != context;
    if switched {
        for (entity, _) in &visuals {
            commands.entity(entity).despawn();
        }
        state.previous = Snapshot::default();
        state.births.clear();
        state.arrivals.clear();
        state.document = context;
    }
    let Some((doc, d)) = current else {
        return;
    };
    let opening = state.previous.cards.is_empty();
    let next = Snapshot::read_animated(d, &state.answers);
    if !switched {
        for (entity, visual) in &visuals {
            if visual.shape.obsolete(&next) {
                commands.entity(entity).despawn();
            }
        }
    }
    state
        .births
        .retain(|id, at| next.cards.contains_key(id) && now - *at < NODE_TIME as f64);
    state
        .arrivals
        .retain(|edge, at| next.edges.contains_key(edge) && now - *at < LINK_TIME as f64);
    let root = if let Some(root) = state.root {
        root
    } else {
        let root = node(
            &mut commands,
            ui.canvas,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
        );
        commands.entity(root).insert((ZIndex(3), Pickable::IGNORE));
        state.root = Some(root);
        root
    };
    let mut events = Vec::new();
    for (&id, card) in &next.cards {
        if !state.previous.cards.contains_key(&id) && state.births.len() < 24 {
            state.births.insert(id, now);
            events.push(Shape::Card(card.clone(), false));
        }
    }
    for (&id, card) in &state.previous.cards {
        if !next.cards.contains_key(&id) {
            events.push(Shape::Card(card.clone(), true));
        }
    }
    for (&edge, &(a, b)) in &state.previous.edges {
        if !next.edges.contains_key(&edge) {
            events.push(Shape::Wire(edge, a, b));
        }
    }
    for &edge in next.edges.keys() {
        if state.arrivals.len() < 128
            && !state.previous.edges.contains_key(&edge)
            && (!opening || next.cards.len() < 12)
        {
            state.arrivals.insert(edge, now);
        }
    }
    state.previous = next;
    let remaining = MAX_EFFECTS.saturating_sub(if switched { 0 } else { visuals.iter().count() });
    for shape in events.into_iter().take(remaining) {
        let entity = match &shape {
            Shape::Card(card, removing) => {
                let size = card.size * doc.zoom + Vec2::splat(40.);
                let c = card.accent.to_linear();
                let handle = cards.add(Material {
                    metrics: Vec4::new(size.x, size.y, 0., if *removing { 1. } else { 0. }),
                    tint: Vec4::new(c.red, c.green, c.blue, doc.zoom),
                });
                commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(card.position.x * doc.zoom + doc.pan.x - 20.),
                            top: px(card.position.y * doc.zoom + doc.pan.y - 20.),
                            width: px(size.x),
                            height: px(size.y),
                            ..default()
                        },
                        MaterialNode(handle),
                        Pickable::IGNORE,
                        ChildOf(root),
                    ))
                    .id()
            }
            Shape::Wire(_, a, b) => connection_paint::wire(
                &mut commands,
                root,
                *a * doc.zoom + doc.pan,
                *b * doc.zoom + doc.pan,
                FOCUS.with_alpha(0.7),
                doc.zoom,
                &mut wires,
            ),
        };
        commands.entity(entity).insert(Visual {
            shape,
            started: now,
        });
    }
}

pub(super) fn animate(
    mut commands: Commands,
    editor: Res<Editor>,
    time: Res<Time>,
    state: Res<State>,
    links: Res<links::State>,
    mut redraw: MessageWriter<bevy::window::RequestRedraw>,
    mut visuals: Query<(
        Entity,
        &Visual,
        &mut Node,
        Option<&MaterialNode<Material>>,
        Option<&MaterialNode<connection_paint::Material>>,
    )>,
    mut cards: ResMut<Assets<Material>>,
    mut wires: ResMut<Assets<connection_paint::Material>>,
) {
    let Some(doc) = editor.narrative.current() else {
        return;
    };
    if state.answers.active()
        || !state.births.is_empty()
        || !state.arrivals.is_empty()
        || !visuals.is_empty()
        || links.drag.as_ref().is_some_and(|drag| drag.moved)
    {
        redraw.write(bevy::window::RequestRedraw);
    }
    for (entity, visual, mut node, card, wire) in &mut visuals {
        let age = (time.elapsed_secs_f64() - visual.started).max(0.) as f32;
        let duration = match &visual.shape {
            Shape::Card(_, false) => NODE_TIME,
            _ => GHOST_TIME,
        };
        if age >= duration {
            commands.entity(entity).despawn();
            continue;
        }
        let progress = (age / duration).clamp(0., 1.);
        match &visual.shape {
            Shape::Card(snapshot, removing) => {
                let live = match &doc.content {
                    Content::Dialogue(d) if !*removing => d.node(snapshot.id),
                    _ => None,
                };
                let position = live.map_or(snapshot.position, |n| Vec2::from_array(n.position));
                let dimensions = live.map_or(snapshot.size, |n| {
                    Vec2::new(NODE_WIDTH, state.answers.height(n))
                });
                let size = dimensions * doc.zoom + Vec2::splat(40.);
                node.left = px(position.x * doc.zoom + doc.pan.x - 20.);
                node.top = px(position.y * doc.zoom + doc.pan.y - 20.);
                node.width = px(size.x);
                node.height = px(size.y);
                if let Some(mut material) = card.and_then(|h| cards.get_mut(&h.0)) {
                    material.metrics.x = size.x;
                    material.metrics.y = size.y;
                    material.metrics.z = progress;
                    material.tint.w = doc.zoom;
                }
            }
            Shape::Wire(_, a, b) => {
                if let Some(mut material) = wire.and_then(|h| wires.get_mut(&h.0)) {
                    connection_paint::geometry(
                        &mut node,
                        &mut material,
                        *a * doc.zoom + doc.pan,
                        *b * doc.zoom + doc.pan,
                        doc.zoom,
                    );
                    material.motion = Vec4::new(progress, age, 2., 0.);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn answer_renumbering_preserves_surviving_wire_identity() {
        let mut d = Dialogue::new("Тест".into());
        let from = d.add(NodeKind::Line, 0);
        let target = d.add(NodeKind::Line, from);
        d.node_mut(from).unwrap().choices.push(Choice {
            text: "Второй".into(),
            target: Some(target),
            condition: Default::default(),
            dialogue: None,
        });
        let mut answers = answers::State::default();
        answers.observe(Some((1, &d)), 0.);
        let before = Snapshot::read_animated(&d, &answers);
        let n = d.node_mut(from).unwrap();
        answers.remove(1, n, 0, 1.);
        n.choices.remove(0);
        answers.observe(Some((1, &d)), 1.);
        let after = Snapshot::read_animated(&d, &answers);
        assert_eq!(
            before
                .edges
                .keys()
                .filter(|e| !after.edges.contains_key(e))
                .count(),
            1
        );
        assert_eq!(
            after
                .edges
                .keys()
                .filter(|e| !before.edges.contains_key(e))
                .count(),
            0
        );
    }
    #[test]
    fn rapid_undo_cancels_both_arrival_and_deletion_visuals() {
        let mut d = Dialogue::new("Тест".into());
        let id = d.add(NodeKind::Line, 0);
        let live = Snapshot::read(&d);
        let card = live.cards[&id].clone();
        d.delete(id);
        let deleted = Snapshot::read(&d);
        assert!(Shape::Card(card.clone(), false).obsolete(&deleted));
        assert!(!Shape::Card(card.clone(), true).obsolete(&deleted));
        assert!(Shape::Card(card, true).obsolete(&live));
        let (&edge, &(a, b)) = live.edges.iter().next().unwrap();
        assert!(Shape::Wire(edge, a, b).obsolete(&live));
    }
    #[test]
    fn birth_scale_settles_and_snapshot_ignores_text_edits() {
        let mut state = State::default();
        state.births.insert(1, 0.);
        assert!((state.scale(1) - 0.975).abs() < 0.001);
        state.now = NODE_TIME as f64;
        assert_eq!(state.scale(1), 1.);
        let mut d = Dialogue::new("Тест".into());
        d.add(NodeKind::Line, 0);
        let before = Snapshot::read(&d);
        d.node_mut(1).unwrap().text = "Другой текст".into();
        let after = Snapshot::read(&d);
        assert_eq!(before.edges, after.edges);
        assert_eq!(before.cards.len(), after.cards.len());
    }
}
