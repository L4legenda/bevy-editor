//! Answer rows animate in presentation state; saved choices and history change immediately.
use super::*;
use std::collections::BTreeMap;
#[path = "narrative_answer_drag.rs"]
pub(super) mod reordering;
const MOVE_TIME: f64 = 0.18;

#[derive(Clone, Copy)]
struct Offset {
    from: f32,
    to: f32,
    started: f64,
}
impl Offset {
    fn value(self, now: f64) -> f32 {
        let t = ((now - self.started) / MOVE_TIME).clamp(0., 1.) as f32;
        self.from + (self.to - self.from) * (1. - (1. - t).powi(3))
    }
}

const OPEN_TIME: f64 = 0.28;
const CLOSE_TIME: f64 = 0.22;

#[derive(Clone)]
struct Slot {
    token: u64,
    ordinal: usize,
    choice: Choice,
    removing: bool,
    from: f32,
    started: f64,
}
impl Slot {
    fn amount(&self, now: f64) -> f32 {
        let duration = if self.removing { CLOSE_TIME } else { OPEN_TIME };
        let t = ((now - self.started) / duration).clamp(0., 1.) as f32;
        let ease = 1. - (1. - t).powi(3);
        self.from + (if self.removing { 0. } else { 1. } - self.from) * ease
    }
    fn retarget(&mut self, removing: bool, now: f64) {
        self.from = self.amount(now);
        self.started = now;
        self.removing = removing;
    }
}

#[derive(Default)]
pub(super) struct State {
    document: Option<usize>,
    nodes: BTreeMap<u32, Vec<Slot>>,
    serial: u64,
    now: f64,
    offsets: BTreeMap<(u32, u64), Offset>,
    drag: Option<reordering::Gesture>,
}
impl State {
    fn slot(&mut self, choice: &Choice, animate: bool) -> Slot {
        self.serial += 1;
        Slot {
            token: self.serial,
            ordinal: 0,
            choice: choice.clone(),
            removing: false,
            from: if animate { 0. } else { 1. },
            started: self.now,
        }
    }
    pub fn observe(&mut self, current: Option<(usize, &Dialogue)>, now: f64) {
        self.now = now;
        let context = current.map(|(id, _)| id);
        if self.document != context {
            self.nodes.clear();
            self.offsets.clear();
            self.drag = None;
            self.document = context;
        }
        let Some((_, d)) = current else { return };
        self.nodes
            .retain(|id, _| d.node(*id).is_some_and(|n| n.kind == NodeKind::Line));
        for n in d.nodes.iter().filter(|n| n.kind == NodeKind::Line) {
            let Some(mut slots) = self.nodes.remove(&n.id) else {
                let slots = n.choices.iter().map(|c| self.slot(c, false)).collect();
                self.nodes.insert(n.id, slots);
                continue;
            };
            if slots.iter().filter(|s| !s.removing).count() == n.choices.len()
                && slots
                    .iter()
                    .filter(|s| !s.removing)
                    .zip(&n.choices)
                    .any(|(slot, choice)| slot.choice != *choice)
            {
                let mut available: Vec<_> = slots.iter().filter(|s| !s.removing).collect();
                let mut order = Vec::new();
                for choice in &n.choices {
                    let Some(at) = available.iter().position(|s| s.choice == *choice) else {
                        break;
                    };
                    order.push(available.remove(at).token);
                }
                let old: Vec<_> = slots
                    .iter()
                    .filter(|s| !s.removing)
                    .map(|s| s.token)
                    .collect();
                if order.len() == n.choices.len() && order != old {
                    self.order_slots(n.id, &mut slots, &order, now);
                }
            }
            // Undo/redo uses the same transitions, including reviving an unfinished removal.
            while slots.iter().filter(|s| !s.removing).count() > n.choices.len() {
                let live: Vec<_> = slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.removing)
                    .collect();
                let i = live
                    .iter()
                    .zip(&n.choices)
                    .take_while(|((_, s), c)| s.choice == **c)
                    .count();
                let at = live[i.min(live.len() - 1)].0;
                slots[at].retarget(true, now);
            }
            while slots.iter().filter(|s| !s.removing).count() < n.choices.len() {
                let live: Vec<_> = slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.removing)
                    .collect();
                let i = live
                    .iter()
                    .zip(&n.choices)
                    .take_while(|((_, s), c)| s.choice == **c)
                    .count();
                let at = live.get(i).map_or(slots.len(), |(at, _)| *at);
                let restored = slots
                    .iter()
                    .position(|s| s.removing && s.choice == n.choices[i]);
                if let Some(old) = restored {
                    let mut slot = slots.remove(old);
                    slot.retarget(false, now);
                    slots.insert(at - usize::from(old < at), slot);
                } else {
                    slots.insert(at, self.slot(&n.choices[i], true));
                }
            }
            for (i, (slot, choice)) in slots
                .iter_mut()
                .filter(|s| !s.removing)
                .zip(&n.choices)
                .enumerate()
            {
                slot.choice.clone_from(choice);
                slot.ordinal = i;
            }
            slots.retain(|s| !s.removing || s.amount(now) > 0.0001);
            self.nodes.insert(n.id, slots);
        }
        self.offsets.retain(|(id, token), motion| {
            (motion.to.abs() > 0.001 || now - motion.started < MOVE_TIME)
                && self
                    .nodes
                    .get(id)
                    .is_some_and(|slots| slots.iter().any(|s| s.token == *token))
        });
    }
    pub fn add(&mut self, document: usize, n: &DialogueNode, now: f64) {
        if self.document == Some(document) && self.nodes.contains_key(&n.id) {
            self.now = now;
            if let Some(c) = n.choices.last() {
                let slot = self.slot(c, true);
                self.nodes.get_mut(&n.id).unwrap().push(slot);
            }
        }
    }
    pub fn remove(&mut self, document: usize, n: &DialogueNode, index: usize, now: f64) {
        if self.document != Some(document) {
            return;
        }
        if let Some(slot) = self
            .nodes
            .get_mut(&n.id)
            .and_then(|slots| slots.iter_mut().filter(|s| !s.removing).nth(index))
        {
            slot.choice = n.choices[index].clone();
            slot.retarget(true, now);
        }
    }
    pub fn active(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.moving)
            || !self.offsets.is_empty()
            || self.nodes.values().flatten().any(|s| {
                self.now - s.started < if s.removing { CLOSE_TIME } else { OPEN_TIME }
                    && (s.removing || s.from < 1.)
            })
    }
    pub fn height(&self, n: &DialogueNode) -> f32 {
        let Some(slots) = self.nodes.get(&n.id) else {
            return nodes::height(n);
        };
        nodes::height(n)
            + (slots.iter().map(|s| s.amount(self.now)).sum::<f32>() - n.choices.len() as f32)
                * OUTPUT_HEIGHT
    }
    pub fn key(&self, id: u32, index: usize) -> usize {
        self.nodes
            .get(&id)
            .and_then(|slots| slots.iter().filter(|s| !s.removing).nth(index))
            .map_or(index, |s| s.token as usize)
    }
    pub fn amount(&self, id: u32, index: usize) -> f32 {
        self.nodes
            .get(&id)
            .and_then(|slots| slots.iter().filter(|s| !s.removing).nth(index))
            .map_or(1., |s| s.amount(self.now))
    }
    pub fn output_position(&self, n: &DialogueNode, index: usize) -> Vec2 {
        let mut p = nodes::output_position(n, index);
        if let Some(slots) = self.nodes.get(&n.id) {
            let mut y = 0.;
            let mut live = 0;
            for slot in slots {
                let amount = slot.amount(self.now);
                if !slot.removing {
                    if live == index {
                        p.y += self.offset(n.id, slot.token) + y + amount * OUTPUT_HEIGHT * 0.5
                            - (index as f32 * OUTPUT_HEIGHT + OUTPUT_HEIGHT * 0.5);
                        break;
                    }
                    live += 1;
                }
                y += amount * OUTPUT_HEIGHT;
            }
        }
        p
    }
    pub fn dragged_output(&self) -> Option<(u32, usize)> {
        let drag = self.drag.as_ref().filter(|d| d.moving)?;
        let index = self
            .nodes
            .get(&drag.node)?
            .iter()
            .filter(|s| !s.removing)
            .position(|s| s.token == drag.token)?;
        Some((drag.node, index))
    }
    pub fn capturing(&self) -> bool {
        self.drag.is_some()
    }
    fn offset(&self, node: u32, token: u64) -> f32 {
        if let Some(drag) = &self.drag {
            if drag.node == node && drag.token == token && drag.moving {
                return drag.offset;
            }
        }
        self.offsets
            .get(&(node, token))
            .map_or(0., |m| m.value(self.now))
    }
    fn target_offset(&mut self, node: u32, token: u64, target: f32, now: f64) {
        if self
            .offsets
            .get(&(node, token))
            .is_some_and(|m| (m.to - target).abs() < 0.001)
        {
            return;
        }
        let from = self.offset(node, token);
        if (from - target).abs() < 0.001 {
            self.offsets.remove(&(node, token));
        } else {
            self.offsets.insert(
                (node, token),
                Offset {
                    from,
                    to: target,
                    started: now,
                },
            );
        }
    }
    fn order_slots(&mut self, node: u32, slots: &mut [Slot], order: &[u64], now: f64) {
        let mut before = BTreeMap::new();
        let mut y = 0.;
        for slot in slots.iter() {
            before.insert(slot.token, y + self.offset(node, slot.token));
            y += slot.amount(now) * OUTPUT_HEIGHT;
        }
        let mut live: Vec<_> = slots.iter().filter(|s| !s.removing).cloned().collect();
        live.sort_by_key(|s| {
            order
                .iter()
                .position(|id| *id == s.token)
                .unwrap_or(usize::MAX)
        });
        for (slot, replacement) in slots.iter_mut().filter(|s| !s.removing).zip(live) {
            *slot = replacement;
        }
        y = 0.;
        for slot in slots.iter() {
            let from = before[&slot.token] - y;
            self.offsets.insert(
                (node, slot.token),
                Offset {
                    from,
                    to: 0.,
                    started: now,
                },
            );
            y += slot.amount(now) * OUTPUT_HEIGHT;
        }
    }
    fn find(&self, id: u32, token: u64) -> Option<&Slot> {
        self.nodes.get(&id)?.iter().find(|s| s.token == token)
    }
}

#[derive(Component)]
pub(super) struct Row(u32, u64);
#[derive(Component)]
pub(super) struct Inner(u32, u64);
#[derive(Component)]
pub(super) struct Socket(u32, u64);
#[derive(Component)]
pub(super) struct Veil(u32, u64);

pub(super) fn spawn(
    commands: &mut Commands,
    card: Entity,
    font: &Handle<Font>,
    n: &DialogueNode,
    state: &State,
) {
    let Some(slots) = state.nodes.get(&n.id) else {
        return;
    };
    let mut index = 0;
    let mut y = nodes::output_position(n, 0).y - OUTPUT_HEIGHT * 0.5;
    for slot in slots {
        let amount = slot.amount(state.now);
        let wrapper = surface(
            commands,
            card,
            Node {
                width: percent(100),
                height: px(OUTPUT_HEIGHT * amount),
                flex_shrink: 0.,
                overflow: Overflow::clip(),
                ..default()
            },
            Color::NONE,
        );
        commands
            .entity(wrapper)
            .insert((Row(n.id, slot.token), ZIndex(0)));
        let line = node(
            commands,
            wrapper,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: px(OUTPUT_HEIGHT),
                top: px((amount - 1.) * OUTPUT_HEIGHT * 0.5),
                padding: UiRect {
                    left: px(12),
                    right: px(16),
                    top: px(10),
                    bottom: px(9),
                },
                border: UiRect::top(px(1)),
                ..row(6.)
            },
        );
        commands
            .entity(line)
            .insert((Inner(n.id, slot.token), BorderColor::all(EDGE)));
        if slot.removing {
            // Retiring rows cannot receive focus, edit data, or trigger another removal.
            commands.entity(wrapper).insert(Pickable::IGNORE);
            node(
                commands,
                line,
                Node {
                    width: px(18),
                    height: px(32),
                    flex_shrink: 0.,
                    ..default()
                },
            );
            text(
                commands,
                line,
                font,
                format!("{:02}", slot.ordinal + 1),
                10.,
                MUTED,
            );
            let label = surface(
                commands,
                line,
                Node {
                    flex_grow: 1.,
                    height: px(36),
                    min_width: px(0),
                    padding: UiRect::horizontal(px(9)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(7)),
                    overflow: Overflow::clip(),
                    ..row(0.)
                },
                WELL,
            );
            commands.entity(label).insert(BorderColor::all(EDGE));
            text(commands, label, font, &slot.choice.text, 14., MUTED);
            let close = node(
                commands,
                line,
                Node {
                    width: px(32),
                    height: px(32),
                    justify_content: JustifyContent::Center,
                    ..row(0.)
                },
            );
            icons::spawn(commands, close, icons::Glyph::Close, 16., MUTED);
            let dot = surface(
                commands,
                card,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(NODE_WIDTH - 9.),
                    top: px(y + amount * OUTPUT_HEIGHT * 0.5 - 7.),
                    width: px(12),
                    height: px(12),
                    border: UiRect::all(px(2)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                WELL,
            );
            commands.entity(dot).insert((
                Socket(n.id, slot.token),
                BorderColor::all(FOCUS),
                Pickable::IGNORE,
            ));
        } else {
            reordering::handle(commands, line, n.id, slot.token, index);
            text(
                commands,
                line,
                font,
                format!("{:02}", index + 1),
                10.,
                MUTED,
            );
            nodes::inline_input(
                commands,
                line,
                font,
                &slot.choice.text,
                DraftField::Choice(index),
                n.id,
                format!("Ответ {} ноды #{}", index + 1, n.id),
            );
            icons::button(
                commands,
                line,
                icons::Glyph::Close,
                &format!("Удалить ответ {} ноды #{}", index + 1, n.id),
                Action::Narrative(nodes::local(n.id, nodes::NodeAction::RemoveChoice(index))),
                Look::Regular,
            );
            let socket = links::socket(
                commands,
                card,
                n.id,
                index,
                state.output_position(n, index) - Vec2::splat(2.),
            );
            commands.entity(socket).insert(Socket(n.id, slot.token));
            index += 1;
        }
        // A wash above the controls stays continuous across the whole row.
        let veil = surface(
            commands,
            wrapper,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                left: px(0),
                top: px(0),
                ..default()
            },
            Color::NONE,
        );
        commands
            .entity(veil)
            .insert((Veil(n.id, slot.token), ZIndex(1), Pickable::IGNORE));
        y += amount * OUTPUT_HEIGHT;
    }
    reordering::indicator(commands, card, n.id);
}

pub(super) fn animate(
    mut commands: Commands,
    effects: Res<effects::State>,
    mut parts: Query<
        (
            Entity,
            &mut Node,
            Option<&mut BackgroundColor>,
            Option<&mut BorderColor>,
            Option<&Row>,
            Option<&Inner>,
            Option<&Socket>,
            Option<&Veil>,
            Option<&links::Output>,
            Option<&mut ZIndex>,
        ),
        Or<(With<Row>, With<Inner>, With<Socket>, With<Veil>)>,
    >,
) {
    let state = &effects.answers;
    for (entity, mut node, background, border, row, inner, socket, veil, output, z) in &mut parts {
        let key = row
            .map(|r| (r.0, r.1))
            .or_else(|| inner.map(|r| (r.0, r.1)))
            .or_else(|| socket.map(|r| (r.0, r.1)))
            .or_else(|| veil.map(|r| (r.0, r.1)));
        let Some((id, token)) = key else { continue };
        let Some(slot) = state.find(id, token) else {
            if inner.is_none() && veil.is_none() {
                commands.entity(entity).despawn();
            }
            continue;
        };
        let amount = slot.amount(state.now);
        let mut next = node.clone();
        let lifted = state
            .drag
            .as_ref()
            .is_some_and(|d| d.moving && (d.node, d.token) == (id, token));
        if row.is_some() {
            next.height = px(amount * OUTPUT_HEIGHT);
            next.top = px(state.offset(id, token));
            if let Some(mut bg) = background {
                bg.set_if_neq(BackgroundColor(if lifted { PANEL } else { Color::NONE }));
            }
            if let Some(mut z) = z {
                z.set_if_neq(ZIndex(if lifted { 3 } else { 0 }));
            }
        } else if inner.is_some() {
            next.top = px((amount - 1.) * OUTPUT_HEIGHT * 0.5);
            next.left = px((1. - amount) * if slot.removing { -5. } else { 7. });
        } else if veil.is_some() {
            if let Some(mut bg) = background {
                let color = if lifted {
                    FOCUS.with_alpha(0.045)
                } else if slot.removing {
                    PANEL.with_alpha(1. - amount)
                } else {
                    FOCUS.with_alpha(0.012 * 4. * amount * (1. - amount))
                };
                bg.set_if_neq(BackgroundColor(color));
            }
        } else {
            if let Some(mut z) = z {
                z.set_if_neq(ZIndex(if lifted { 4 } else { 2 }));
            }
            let slots = &state.nodes[&id];
            let before: f32 = slots
                .iter()
                .take_while(|s| s.token != token)
                .map(|s| s.amount(state.now) * OUTPUT_HEIGHT)
                .sum();
            // Same center as the clipped input; the connection stays attached throughout.
            let center = HEADER
                + 252.
                + before
                + amount * OUTPUT_HEIGHT * 0.5
                + 1.
                + state.offset(id, token);
            next.top = px(center - if output.is_some() { 18. } else { 6. });
            if output.is_none() {
                if let Some(mut b) = border {
                    b.set_if_neq(BorderColor::all(FOCUS.with_alpha(amount)));
                }
                next.width = px(12. * amount);
                next.height = px(12. * amount);
                next.left = px(NODE_WIDTH - 3. - 6. * amount);
                next.top = px(center - 6. * amount);
            }
        }
        node.set_if_neq(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_expansion_and_collapse_keep_geometry_continuous() {
        let mut d = Dialogue::new("Тест".into());
        let id = d.add(NodeKind::Line, 0);
        let mut state = State::default();
        state.observe(Some((1, &d)), 0.);
        let original = state.height(d.node(id).unwrap());
        let n = d.node_mut(id).unwrap();
        n.choices.push(Choice {
            text: "Второй".into(),
            target: None,
            dialogue: None,
        });
        state.add(1, n, 1.);
        state.observe(Some((1, &d)), 1.);
        assert_eq!(state.height(d.node(id).unwrap()), original);
        state.observe(Some((1, &d)), 1. + OPEN_TIME * 0.25);
        let partial = state.height(d.node(id).unwrap());
        assert!(partial > original && partial < original + OUTPUT_HEIGHT);
        let n = d.node_mut(id).unwrap();
        state.remove(1, n, 1, 1. + OPEN_TIME * 0.25);
        n.choices.remove(1);
        state.observe(Some((1, &d)), 1. + OPEN_TIME * 0.25);
        assert!((state.height(d.node(id).unwrap()) - partial).abs() < 0.001);
        state.observe(Some((1, &d)), 2.);
        assert_eq!(state.height(d.node(id).unwrap()), original);
        assert!(!state.active());
        assert_eq!(state.nodes[&id].len(), 1);
    }

    #[test]
    fn answer_undo_revives_same_row_and_ports_follow_shifted_indices() {
        let mut d = Dialogue::new("Тест".into());
        let id = d.add(NodeKind::Line, 0);
        d.node_mut(id).unwrap().choices.extend([
            Choice {
                text: "Второй".into(),
                target: Some(7),
                dialogue: None,
            },
            Choice {
                text: "Третий".into(),
                target: Some(8),
                dialogue: None,
            },
        ]);
        let before = d.clone();
        let mut state = State::default();
        state.observe(Some((1, &d)), 0.);
        let tokens: Vec<_> = state.nodes[&id].iter().map(|s| s.token).collect();
        let n = d.node_mut(id).unwrap();
        let old_port = state.output_position(n, 2);
        state.remove(1, n, 1, 1.);
        n.choices.remove(1);
        state.observe(Some((1, &d)), 1.);
        assert_eq!(state.output_position(d.node(id).unwrap(), 1), old_port);
        state.observe(Some((1, &d)), 1.1);
        let partial = state.height(d.node(id).unwrap());
        state.observe(Some((1, &before)), 1.1);
        assert!((state.height(before.node(id).unwrap()) - partial).abs() < 0.001);
        assert_eq!(
            state.nodes[&id].iter().map(|s| s.token).collect::<Vec<_>>(),
            tokens
        );
        state.observe(Some((1, &before)), 2.);
        assert_eq!(
            state.height(before.node(id).unwrap()),
            nodes::height(before.node(id).unwrap())
        );
        assert_eq!(state.output_position(before.node(id).unwrap(), 2), old_port);
        // Identical labels must still remove the explicitly selected row.
        let n = d.node_mut(id).unwrap();
        n.choices[1].text = n.choices[0].text.clone();
        state.observe(Some((1, &d)), 3.);
        let n = d.node_mut(id).unwrap();
        let first = state.nodes[&id].iter().find(|s| !s.removing).unwrap().token;
        state.remove(1, n, 0, 3.);
        n.choices.remove(0);
        state.observe(Some((1, &d)), 3.);
        assert!(
            state.nodes[&id]
                .iter()
                .find(|s| s.token == first)
                .unwrap()
                .removing
        );
        state.observe(None, 4.);
        assert!(state.nodes.is_empty());
    }
}
