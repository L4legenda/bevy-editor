//! A captured, presentation-only reorder; one complete Choice moves on a valid drop.
use super::*;

pub(super) struct Gesture {
    pub node: u32,
    pub token: u64,
    pub moving: bool,
    pub offset: f32,
    document: usize,
    generation: u64,
    origin: Vec2,
    grab: f32,
    choices: Vec<Choice>,
    from: usize,
    to: usize,
    drop_top: f32,
}
#[derive(Component)]
pub(in super::super) struct Handle {
    node: u32,
    token: u64,
}
#[derive(Component)]
pub(in super::super) struct Indicator(u32);

pub(super) fn handle(
    commands: &mut Commands,
    parent: Entity,
    node_id: u32,
    token: u64,
    index: usize,
) {
    let grip = commands
        .spawn((
            Handle {
                node: node_id,
                token,
            },
            Button,
            Hovered::default(),
            TabIndex(0),
            AccessibleLabel(format!("Переместить ответ {} ноды #{}", index + 1, node_id)),
            crate::editor::ui::control_hints::Help("Перетащите ручку, чтобы изменить порядок ответов. Связи сохраняются. Esc — отменить перетаскивание.".into()),
            Node {
                width: px(18),
                height: px(32),
                flex_shrink: 0.,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..column(3.)
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            ChildOf(parent),
        ))
        .id();
    for _ in 0..3 {
        let pair = node(commands, grip, row(3.));
        commands.entity(pair).insert(Pickable::IGNORE);
        for _ in 0..2 {
            commands.spawn((
                Node {
                    width: px(2),
                    height: px(2),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(MUTED),
                Pickable::IGNORE,
                ChildOf(pair),
            ));
        }
    }
}
pub(super) fn indicator(commands: &mut Commands, card: Entity, id: u32) {
    commands.spawn((
        Indicator(id),
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(7),
            width: px(NODE_WIDTH - 18.),
            height: px(2),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        ZIndex(4),
        BackgroundColor(FOCUS),
        Pickable::IGNORE,
        ChildOf(card),
    ));
}

fn local_point(
    window: &Window,
    cursor: Vec2,
    node: &ComputedNode,
    transform: &UiGlobalTransform,
) -> Vec2 {
    (transform
        .affine()
        .inverse()
        .transform_point2(cursor * window.scale_factor())
        + node.size() * 0.5)
        / window.scale_factor()
}
fn cancel(state: &mut State, now: f64) {
    let Some(drag) = state.drag.take() else {
        return;
    };
    state.now = now;
    if drag.moving {
        state.offsets.insert(
            (drag.node, drag.token),
            Offset {
                from: drag.offset,
                to: 0.,
                started: now,
            },
        );
        let tokens: Vec<_> = state
            .nodes
            .get(&drag.node)
            .into_iter()
            .flatten()
            .map(|s| s.token)
            .collect();
        for token in tokens.into_iter().filter(|t| *t != drag.token) {
            state.target_offset(drag.node, token, 0., now);
        }
    }
}
fn preview(state: &mut State, local_y: f32, now: f64) {
    state.now = now;
    let Some(drag) = &state.drag else { return };
    let slots = &state.nodes[&drag.node];
    let live: Vec<_> = slots.iter().filter(|s| !s.removing).collect();
    let source_top: f32 = slots
        .iter()
        .take_while(|s| s.token != drag.token)
        .map(|s| s.amount(now) * OUTPUT_HEIGHT)
        .sum();
    let row_height = state.find(drag.node, drag.token).unwrap().amount(now) * OUTPUT_HEIGHT;
    let total: f32 = slots.iter().map(|s| s.amount(now) * OUTPUT_HEIGHT).sum();
    let desired_top =
        (local_y - drag.grab - 2. - HEADER - 252.).clamp(0., (total - row_height).max(0.));
    let center = desired_top + row_height * 0.5;
    let mut at = 0;
    let mut y = 0.;
    for slot in slots {
        let h = slot.amount(now) * OUTPUT_HEIGHT;
        let threshold = y
            + h * 0.5
            + if slot.ordinal < drag.from {
                row_height * 0.5
            } else {
                -row_height * 0.5
            };
        if !slot.removing && slot.token != drag.token && center > threshold {
            at += 1;
        }
        y += h;
    }
    let at = at.min(live.len() - 1);
    let mut order: Vec<_> = live.iter().map(|s| s.token).collect();
    order.remove(drag.from);
    order.insert(at, drag.token);
    let mut replacements = order.iter();
    let mut desired = BTreeMap::new();
    y = 0.;
    for slot in slots {
        let token = if slot.removing {
            slot.token
        } else {
            *replacements.next().unwrap()
        };
        desired.insert(token, y);
        y += state.find(drag.node, token).unwrap().amount(now) * OUTPUT_HEIGHT;
    }
    let id = drag.node;
    let token = drag.token;
    let targets: Vec<_> = slots
        .iter()
        .scan(0., |top, slot| {
            let offset = desired[&slot.token] - *top;
            *top += slot.amount(now) * OUTPUT_HEIGHT;
            Some((slot.token, offset))
        })
        .collect();
    for (other, offset) in targets.into_iter().filter(|(t, _)| *t != token) {
        state.target_offset(id, other, offset, now);
    }
    let drag = state.drag.as_mut().unwrap();
    drag.to = at;
    drag.offset = desired_top - source_top;
    drag.drop_top = desired[&token];
}

pub(in super::super) fn input(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<NarrativeUi>,
    mut effects: ResMut<effects::State>,
    mut focus: ResMut<InputFocus>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    drawer: Res<DrawerMotion>,
    handles: Query<(&Handle, &Hovered)>,
    cards: Query<(Entity, &nodes::Card)>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    links: Res<links::State>,
    mut redraw: MessageWriter<bevy::window::RequestRedraw>,
) {
    let now = time.elapsed_secs_f64();
    let state = &mut effects.answers;
    let escape = keys.just_pressed(KeyCode::Escape);
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let allowed = !escape
        && !command
        && window.focused
        && editor.pending.is_none()
        && !editor.browser_popup
        && !ui.preview
        && ui.picker.is_none();
    let Some(doc) = editor.narrative.current_mut() else {
        cancel(state, now);
        return;
    };
    let Content::Dialogue(d) = &doc.content else {
        cancel(state, now);
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        cancel(state, now);
        return;
    };
    if !allowed {
        if state.drag.is_some() && escape {
            keys.clear_just_pressed(KeyCode::Escape);
        }
        cancel(state, now);
        return;
    }
    if state.drag.as_ref().is_some_and(|g| {
        g.document != doc.id
            || g.generation != ui.generation
            || d.node(g.node).is_none_or(|n| n.choices != g.choices)
    }) {
        cancel(state, now);
        return;
    }
    let canvas = geometry.get(ui.canvas).ok().map(|(n, t)| {
        Rect::from_center_size(
            t.translation / window.scale_factor(),
            n.size() / window.scale_factor(),
        )
    });
    let inside = canvas.is_some_and(|r| r.contains(cursor))
        && cursor.y < window.height() - FOOT - drawer.visible_height;
    if state.drag.is_none()
        && inside
        && mouse.just_pressed(MouseButton::Left)
        && links.drag.is_none()
    {
        if let Some((handle, _)) = handles.iter().find(|(_, hover)| hover.0) {
            if let Some(n) = d.node(handle.node).filter(|n| n.choices.len() > 1) {
                if let Some((entity, _)) = cards.iter().find(|(_, card)| card.0 == handle.node) {
                    if let Ok((size, transform)) = geometry.get(entity) {
                        let point = local_point(&window, cursor, size, transform);
                        let slots = &state.nodes[&handle.node];
                        if let Some(from) = slots
                            .iter()
                            .filter(|s| !s.removing)
                            .position(|s| s.token == handle.token)
                        {
                            let top: f32 = slots
                                .iter()
                                .take_while(|s| s.token != handle.token)
                                .map(|s| s.amount(now) * OUTPUT_HEIGHT)
                                .sum();
                            let offset = state.offset(handle.node, handle.token);
                            state.drag = Some(Gesture {
                                node: handle.node,
                                token: handle.token,
                                moving: false,
                                offset,
                                document: doc.id,
                                generation: ui.generation,
                                origin: cursor,
                                grab: point.y - (2. + HEADER + 252. + top + offset),
                                choices: n.choices.clone(),
                                from,
                                to: from,
                                drop_top: top,
                            });
                            ui.clear_node_drag();
                            ui.panning = false;
                        }
                    }
                }
            }
        }
    }
    let Some(drag) = state.drag.as_mut() else {
        return;
    };
    drag.moving |= cursor.distance_squared(drag.origin) >= 16.;
    let id = drag.node;
    if drag.moving {
        doc.selected = id;
        focus.clear();
    }
    let Some((entity, _)) = cards.iter().find(|(_, card)| card.0 == id) else {
        cancel(state, now);
        return;
    };
    let Ok((size, transform)) = geometry.get(entity) else {
        cancel(state, now);
        return;
    };
    let point = local_point(&window, cursor, size, transform);
    if state.drag.as_ref().unwrap().moving {
        preview(state, point.y, now);
        redraw.write(bevy::window::RequestRedraw);
    }
    if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
        let drag = state.drag.as_ref().unwrap();
        let changed = drag.moving
            && drag.from != drag.to
            && inside
            && point.x >= -40.
            && point.x <= NODE_WIDTH + 40.;
        if changed {
            let (from, to) = (drag.from, drag.to);
            doc.checkpoint();
            if let Content::Dialogue(d) = &mut doc.content {
                let n = d.node_mut(id).unwrap();
                let choice = n.choices.remove(from);
                n.choices.insert(to, choice);
            }
            let mut slots = state.nodes.remove(&id).unwrap();
            let mut order: Vec<_> = slots
                .iter()
                .filter(|s| !s.removing)
                .map(|s| s.token)
                .collect();
            let token = order.remove(from);
            order.insert(to, token);
            state.order_slots(id, &mut slots, &order, now);
            state.nodes.insert(id, slots);
            state.drag = None;
            ui.generation += 1;
        } else {
            cancel(state, now);
        }
    }
}

pub(in super::super) fn paint(
    effects: Res<effects::State>,
    mut handles: Query<(&Handle, &Hovered, &mut BackgroundColor, &mut BorderColor)>,
    mut indicators: Query<(&Indicator, &mut Node)>,
) {
    let state = &effects.answers;
    for (handle, hovered, mut bg, mut border) in &mut handles {
        let active = state
            .drag
            .as_ref()
            .is_some_and(|g| g.moving && (g.node, g.token) == (handle.node, handle.token));
        bg.set_if_neq(BackgroundColor(if active {
            ACTIVE
        } else if hovered.0 {
            HOVER
        } else {
            Color::NONE
        }));
        border.set_if_neq(BorderColor::all(if active { FOCUS } else { Color::NONE }));
    }
    for (indicator, mut node) in &mut indicators {
        let drag = state
            .drag
            .as_ref()
            .filter(|g| g.moving && g.node == indicator.0);
        let mut next = node.clone();
        next.display = if drag.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(g) = drag {
            next.top = px(HEADER + 252. + g.drop_top);
        }
        node.set_if_neq(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Dialogue, u32, State) {
        let mut d = Dialogue::new("Тест".into());
        let id = d.add(NodeKind::Line, 0);
        d.node_mut(id).unwrap().choices = (1..=3)
            .map(|i| Choice {
                text: format!("Ответ {i}"),
                target: Some(i + 10),
                dialogue: None,
            })
            .collect();
        let mut state = State::default();
        state.observe(Some((1, &d)), 0.);
        let token = state.nodes[&id][0].token;
        state.drag = Some(Gesture {
            node: id,
            token,
            moving: true,
            offset: 0.,
            document: 1,
            generation: 0,
            origin: Vec2::ZERO,
            grab: 28.,
            choices: d.node(id).unwrap().choices.clone(),
            from: 0,
            to: 0,
            drop_top: 0.,
        });
        (d, id, state)
    }
    #[test]
    fn answer_reorder_preview_preserves_model_and_settles_without_losing_offsets() {
        let (d, id, mut state) = fixture();
        let n = d.node(id).unwrap();
        let token = state.drag.as_ref().unwrap().token;
        preview(&mut state, 2. + HEADER + 252. + 28. + 102., 1.);
        assert_eq!(state.drag.as_ref().unwrap().to, 2);
        assert_eq!(
            state.output_position(n, 0).y,
            nodes::output_position(n, 0).y + 102.
        );
        state.observe(Some((1, &d)), 2.);
        assert_eq!(
            state.offset(id, state.nodes[&id][1].token),
            -OUTPUT_HEIGHT,
            "A settled preview must not jump back while the pointer is held"
        );
        cancel(&mut state, 2.);
        assert_eq!(state.offset(id, token), 102.);
        state.observe(Some((1, &d)), 3.);
        assert_eq!(state.offset(id, token), 0.);
        assert_eq!(state.height(n), nodes::height(n));
        assert_eq!(n.choices[0].target, Some(11));
    }
    #[test]
    fn answer_reorder_commit_and_undo_keep_tokens_and_links_together() {
        let (mut d, id, mut state) = fixture();
        let before = d.clone();
        let original: Vec<_> = state.nodes[&id].iter().map(|s| s.token).collect();
        preview(&mut state, 2. + HEADER + 252. + 28. + 102., 1.);
        let source_y = state.output_position(d.node(id).unwrap(), 0).y;
        let mut slots = state.nodes.remove(&id).unwrap();
        let order = [original[1], original[2], original[0]];
        state.order_slots(id, &mut slots, &order, 1.);
        state.nodes.insert(id, slots);
        state.drag = None;
        let n = d.node_mut(id).unwrap();
        let choice = n.choices.remove(0);
        n.choices.push(choice);
        state.observe(Some((1, &d)), 1.);
        assert_eq!(state.output_position(d.node(id).unwrap(), 2).y, source_y);
        assert_eq!(d.node(id).unwrap().choices[2].target, Some(11));
        assert_eq!(state.key(id, 2), original[0] as usize);
        state.observe(Some((1, &d)), 2.);
        state.observe(Some((1, &before)), 2.);
        assert_eq!(
            state.nodes[&id].iter().map(|s| s.token).collect::<Vec<_>>(),
            original
        );
        state.observe(Some((1, &before)), 3.);
        assert_eq!(
            state.output_position(before.node(id).unwrap(), 0),
            nodes::output_position(before.node(id).unwrap(), 0)
        );
    }
}
