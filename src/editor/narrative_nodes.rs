//! Editable node cards. Text entities survive typing, selection, panning and zooming.
use super::*;

const EDGE_WIDTH: f32 = 2.0;
const FOOTER: f32 = 54.0;
const START_LABEL_HEIGHT: f32 = 14.0;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum NodeAction {
    Speaker,
    Portrait,
    AddChoice,
    RemoveChoice(usize),
    Disconnect(usize),
    Delete,
    EditText,
}
impl NodeAction {
    pub(super) fn action(self) -> NarrativeAction {
        match self {
            Self::Speaker => NarrativeAction::Speaker,
            Self::Portrait => NarrativeAction::Portrait,
            Self::AddChoice => NarrativeAction::AddChoice,
            Self::RemoveChoice(i) => NarrativeAction::RemoveChoice(i),
            Self::Disconnect(i) => NarrativeAction::Disconnect(i),
            Self::Delete => NarrativeAction::Delete,
            Self::EditText => NarrativeAction::EditText,
        }
    }
}

#[derive(Component)]
pub(super) struct Owner(pub u32);
#[derive(Component)]
pub(super) struct Card(pub u32);

fn body_height(n: &DialogueNode) -> f32 {
    match n.kind {
        NodeKind::Start => 116.0,
        NodeKind::Line => 252.0,
        NodeKind::End => 172.0,
    }
}
pub(super) fn height(n: &DialogueNode) -> f32 {
    if n.kind == NodeKind::Start {
        return EDGE_WIDTH * 2.0
            + HEADER
            + body_height(n)
            + n.choices.len().saturating_sub(1) as f32 * OUTPUT_HEIGHT;
    }
    EDGE_WIDTH * 2.0
        + HEADER
        + body_height(n)
        + n.choices.len() as f32 * OUTPUT_HEIGHT
        + if n.kind == NodeKind::Line {
            FOOTER
        } else {
            12.0
        }
}
fn output_y(n: &DialogueNode, choice: usize) -> f32 {
    if n.kind == NodeKind::Start && choice == 0 {
        return EDGE_WIDTH + HEADER + 12.0 + START_LABEL_HEIGHT + 8.0 + 18.0;
    }
    let row = if n.kind == NodeKind::Start {
        choice - 1
    } else {
        choice
    };
    EDGE_WIDTH + HEADER + body_height(n) + row as f32 * OUTPUT_HEIGHT + 29.0
}
pub(super) fn input_position() -> Vec2 {
    Vec2::new(EDGE_WIDTH * 0.5, EDGE_WIDTH + HEADER * 0.5)
}
pub(super) fn output_position(n: &DialogueNode, choice: usize) -> Vec2 {
    Vec2::new(NODE_WIDTH - EDGE_WIDTH * 0.5, output_y(n, choice))
}
pub(super) fn accent(n: &DialogueNode) -> Color {
    match n.kind {
        NodeKind::Start => GREEN,
        NodeKind::Line => FOCUS,
        NodeKind::End => BLUE,
    }
}
fn frame_node(n: &DialogueNode, doc: &Document, height: f32) -> Node {
    let p = Vec2::from_array(n.position) * doc.zoom + doc.pan;
    Node {
        position_type: PositionType::Absolute,
        left: px(p.x - (1.0 - doc.zoom) * NODE_WIDTH * 0.5),
        top: px(p.y - (1.0 - doc.zoom) * height * 0.5),
        width: px(NODE_WIDTH),
        height: px(height),
        border: UiRect::all(px(EDGE_WIDTH)),
        border_radius: BorderRadius::all(px(NODE_RADIUS)),
        ..column(0.0)
    }
}

/// Place a new continuation beside its source, avoiding the full editable cards.
pub(super) fn place(d: &mut Dialogue, id: u32, from: u32) {
    let Some(source) = d.node(from) else { return };
    let mut p = Vec2::from_array(source.position) + Vec2::new(NODE_WIDTH + 80.0, 0.0);
    let new_height = d.node(id).map_or(300.0, height);
    loop {
        let rect = Rect::from_corners(
            p - Vec2::splat(20.0),
            p + Vec2::new(NODE_WIDTH + 20.0, new_height + 20.0),
        );
        let overlap = d.nodes.iter().filter(|n| n.id != id).find(|n| {
            let q = Vec2::from_array(n.position);
            let other = Rect::from_corners(q, q + Vec2::new(NODE_WIDTH, height(n)));
            !rect.intersect(other).is_empty()
        });
        match overlap {
            Some(n) => p.y = n.position[1] + height(n) + 48.0,
            None => break,
        }
    }
    if let Some(n) = d.node_mut(id) {
        n.position = p.to_array();
    }
}

pub(super) fn sync(
    editor: Res<Editor>,
    links: Res<links::State>,
    effects: Res<effects::State>,
    mut cards: Query<(
        &Card,
        &mut Node,
        &mut UiTransform,
        &mut BorderColor,
        &mut ZIndex,
    )>,
) {
    let Some(doc) = editor.narrative.current() else {
        return;
    };
    let Content::Dialogue(d) = &doc.content else {
        return;
    };
    for (card, mut node, mut transform, mut border, mut z) in &mut cards {
        if let Some(n) = d.node(card.0) {
            node.set_if_neq(frame_node(n, doc, effects.answers.height(n)));
            transform.set_if_neq(UiTransform::from_scale(Vec2::splat(
                doc.zoom * effects.scale(n.id),
            )));
            let target = links.highlights_node(d, n.id)
                || links
                    .drag
                    .as_ref()
                    .is_some_and(|d| d.moved && d.target == Some(n.id));
            border.set_if_neq(BorderColor::all(if target {
                GREEN
            } else {
                accent(n).with_alpha(if n.id == doc.selected { 1.0 } else { 0.32 })
            }));
            z.set_if_neq(ZIndex(if n.id == doc.selected { 1 } else { 0 }));
        }
    }
}

pub(super) fn local(id: u32, operation: NodeAction) -> NarrativeAction {
    NarrativeAction::Node(id, operation)
}
fn full_button(commands: &mut Commands, e: Entity) {
    commands.entity(e).entry::<Node>().and_modify(|mut n| {
        n.flex_grow = 1.0;
        n.flex_basis = px(0);
        n.flex_shrink = 1.0;
        n.min_width = px(0);
        n.height = px(32);
    });
}
fn node_field(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    caption: &str,
    value: &str,
    kind: DraftField,
    multiline: bool,
    id: u32,
) {
    let input = if kind == DraftField::Text {
        let header = node(
            commands,
            parent,
            Node {
                width: percent(100),
                height: px(20),
                flex_shrink: 0.0,
                ..row(6.0)
            },
        );
        let title = text(commands, header, font, caption, 11.0, MUTED);
        commands.entity(title).insert(Node {
            flex_grow: 1.0,
            ..default()
        });
        let expand = control(
            commands,
            header,
            Action::Narrative(local(id, NodeAction::EditText)),
            Look::Regular,
            Node {
                height: px(20),
                padding: UiRect::axes(px(6), px(1)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                flex_shrink: 0.0,
                ..row(4.0)
            },
        );
        commands.entity(expand).insert(AccessibleLabel(format!(
            "Открыть редактор текста ноды #{id}"
        )));
        icons::spawn(commands, expand, icons::Glyph::Pencil, 12.0, MUTED);
        text(commands, expand, font, "Редактор", 11.0, TEXT);
        input_field(commands, parent, font, caption, value, kind, multiline)
    } else {
        field(commands, parent, font, caption, value, kind, multiline)
    };
    commands
        .entity(input)
        .insert((Owner(id), AccessibleLabel(format!("{caption} ноды #{id}"))));
}
pub(super) fn inline_input(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: &str,
    kind: DraftField,
    owner: u32,
    label: String,
) {
    commands.spawn((
        TextInput,
        EditableText::new(value),
        kind,
        Owner(owner),
        crate::ui_theme::text_cursor_style(),
        AccessibleLabel(label),
        super::font(font, 14.0),
        TextColor(TEXT),
        TabIndex(0),
        TextLayout::no_wrap(),
        BackgroundColor(WELL),
        BorderColor::all(EDGE),
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: px(36),
            padding: UiRect::axes(px(9), px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            overflow: Overflow::clip(),
            ..default()
        },
        ChildOf(parent),
    ));
}

pub(super) fn connections(
    commands: &mut Commands,
    parent: Entity,
    d: &Dialogue,
    doc: &Document,
    drag: Option<&links::Drag>,
    materials: &mut Assets<connection_paint::Material>,
) {
    for n in &d.nodes {
        for (i, c) in n.choices.iter().enumerate() {
            if let Some(next) = c.target.and_then(|id| d.node(id)) {
                let a = (Vec2::from_array(n.position) + output_position(n, i)) * doc.zoom + doc.pan;
                let b = (Vec2::from_array(next.position) + input_position()) * doc.zoom + doc.pan;
                let wire = connection_paint::wire(
                    commands,
                    parent,
                    a,
                    b,
                    if drag.is_some_and(|drag| (drag.from, drag.choice) == (n.id, i)) {
                        FOCUS.with_alpha(0.14)
                    } else if n.id == doc.selected {
                        FOCUS.with_alpha(0.85)
                    } else {
                        FOCUS.with_alpha(0.48)
                    },
                    doc.zoom,
                    materials,
                );
                commands.entity(wire).insert(connection_paint::Connection {
                    from: n.id,
                    choice: i,
                    target: next.id,
                });
            }
        }
    }
    if let Some(drag) = drag {
        if let Some(source) = d.node(drag.from) {
            let a = (Vec2::from_array(source.position) + output_position(source, drag.choice))
                * doc.zoom
                + doc.pan;
            let b = drag
                .target
                .and_then(|id| d.node(id))
                .map_or(drag.pointer, |n| {
                    (Vec2::from_array(n.position) + input_position()) * doc.zoom + doc.pan
                });
            let wire = connection_paint::wire(
                commands,
                parent,
                a,
                b,
                if drag.target.is_some() { GREEN } else { FOCUS },
                doc.zoom,
                materials,
            );
            commands.entity(wire).insert(connection_paint::Preview);
        }
    }
}

pub(super) fn spawn(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    editor: &Editor,
    doc: &Document,
    d: &Dialogue,
    _linking: Option<(u32, usize)>,
    server: &AssetServer,
    answers: &answers::State,
) {
    for n in &d.nodes {
        let card = surface(
            commands,
            parent,
            frame_node(n, doc, answers.height(n)),
            PANEL,
        );
        commands.entity(card).insert((
            Card(n.id),
            ZIndex(if n.id == doc.selected { 1 } else { 0 }),
            UiTransform::from_scale(Vec2::splat(doc.zoom)),
            BorderColor::all(accent(n).with_alpha(if n.id == doc.selected { 1.0 } else { 0.32 })),
            AccessibleLabel(format!("Нода #{}", n.id)),
        ));
        // Ports sit on the outline, while their hit areas extend to both sides.
        if n.kind != NodeKind::Start {
            links::input_socket(
                commands,
                card,
                n.id,
                input_position() - Vec2::splat(EDGE_WIDTH),
            );
        }
        let header = node(
            commands,
            card,
            Node {
                height: px(HEADER),
                width: percent(100),
                flex_shrink: 0.0,
                ..row(0.0)
            },
        );
        let caption = match n.kind {
            NodeKind::Start => "НАЧАЛО".into(),
            NodeKind::Line => format!("РЕПЛИКА #{}", n.id),
            NodeKind::End => format!("ЗАВЕРШЕНИЕ #{}", n.id),
        };
        let head = control(
            commands,
            header,
            Action::Narrative(NarrativeAction::Select(n.id)),
            Look::Regular,
            Node {
                flex_grow: 1.0,
                height: percent(100),
                min_width: px(0),
                padding: UiRect::horizontal(px(if n.kind == NodeKind::Start { 14 } else { 18 })),
                border_radius: if n.kind == NodeKind::Start {
                    BorderRadius::top(px(NODE_RADIUS - EDGE_WIDTH))
                } else {
                    BorderRadius::top_left(px(NODE_RADIUS - EDGE_WIDTH))
                },
                ..row(8.0)
            },
        );
        commands
            .entity(head)
            .insert(AccessibleLabel(format!("Выбрать {caption}")));
        if n.kind == NodeKind::Start {
            text(commands, head, font, "●", 10.0, accent(n));
        }
        text(commands, head, font, caption, 11.0, accent(n));
        if n.kind != NodeKind::Start {
            let delete = icons::button(
                commands,
                header,
                icons::Glyph::Trash,
                &format!("Удалить ноду #{}", n.id),
                Action::Narrative(local(n.id, NodeAction::Delete)),
                Look::Regular,
            );
            commands
                .entity(delete)
                .entry::<Node>()
                .and_modify(|mut node| {
                    node.width = px(38);
                    node.height = px(HEADER);
                    node.border = UiRect::ZERO;
                    node.border_radius = BorderRadius::top_right(px(NODE_RADIUS - EDGE_WIDTH));
                });
        }
        let body = node(
            commands,
            card,
            Node {
                width: percent(100),
                height: px(body_height(n)),
                flex_shrink: 0.0,
                padding: UiRect::all(px(12)),
                ..column(8.0)
            },
        );
        match n.kind {
            NodeKind::Start => {
                let label = text(commands, body, font, "Название диалога", 11.0, MUTED);
                commands.entity(label).insert(Node {
                    height: px(START_LABEL_HEIGHT),
                    flex_shrink: 0.0,
                    ..default()
                });
                let line = node(
                    commands,
                    body,
                    Node {
                        width: percent(100),
                        height: px(36),
                        flex_shrink: 0.0,
                        ..row(0.0)
                    },
                );
                inline_input(
                    commands,
                    line,
                    font,
                    &d.title,
                    DraftField::Title,
                    n.id,
                    format!("Название диалога ноды #{}", n.id),
                );
                filenames::error_label(commands, body, font);
                if !n.choices.is_empty() {
                    links::socket(
                        commands,
                        card,
                        n.id,
                        0,
                        output_position(n, 0) - Vec2::splat(EDGE_WIDTH),
                    );
                }
            }
            NodeKind::Line => {
                text(commands, body, font, "КТО ГОВОРИТ", 10.0, MUTED);
                let speaker_row = node(
                    commands,
                    body,
                    Node {
                        width: percent(100),
                        ..row(6.0)
                    },
                );
                let who = act(
                    commands,
                    speaker_row,
                    font,
                    speaker(editor, n),
                    local(n.id, NodeAction::Speaker),
                    false,
                );
                full_button(commands, who);
                commands.entity(who).insert(AccessibleLabel(format!(
                    "Выбрать говорящего ноды #{}",
                    n.id
                )));
                commands.entity(who).insert(speaker_menu::Anchor(n.id));
                node(
                    commands,
                    who,
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                );
                icons::spawn(commands, who, icons::Glyph::ChevronDown, 13.0, MUTED);
                if let Some(c) = n.speaker.as_ref().and_then(|p| character(editor, p)) {
                    let pick = control(
                        commands,
                        speaker_row,
                        Action::Narrative(local(n.id, NodeAction::Portrait)),
                        Look::Regular,
                        Node {
                            width: px(34),
                            height: px(32),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(6)),
                            overflow: Overflow::clip(),
                            justify_content: JustifyContent::Center,
                            ..row(0.0)
                        },
                    );
                    commands
                        .entity(pick)
                        .insert(AccessibleLabel(format!("Выбрать портрет ноды #{}", n.id)));
                    if let Some(p) = c.portraits.get(n.portrait) {
                        portrait(commands, pick, server, &editor.root, &p.path, 30.0);
                    } else {
                        icons::spawn(commands, pick, icons::Glyph::Image, 18.0, MUTED);
                    }
                }
                node_field(
                    commands,
                    body,
                    font,
                    "Текст реплики",
                    &n.text,
                    DraftField::Text,
                    true,
                    n.id,
                );
                text(commands, body, font, "ОТВЕТЫ И ПЕРЕХОДЫ", 10.0, MUTED);
            }
            NodeKind::End => {
                node_field(
                    commands,
                    body,
                    font,
                    "Подпись завершения",
                    &n.text,
                    DraftField::Text,
                    true,
                    n.id,
                );
            }
        }
        if n.kind == NodeKind::Line {
            answers::spawn(commands, card, font, n, answers);
        }
        for (i, _) in n.choices.iter().enumerate() {
            if n.kind == NodeKind::Line || n.kind == NodeKind::Start && i == 0 {
                continue;
            }
            let line = surface(
                commands,
                card,
                Node {
                    width: percent(100),
                    height: px(OUTPUT_HEIGHT),
                    flex_shrink: 0.0,
                    padding: UiRect {
                        left: px(12),
                        right: px(16),
                        top: px(10),
                        bottom: px(9),
                    },
                    border: UiRect::top(px(1)),
                    ..row(6.0)
                },
                Color::NONE,
            );
            if n.kind == NodeKind::Line {
                text(commands, line, font, format!("{:02}", i + 1), 10.0, MUTED);
                inline_input(
                    commands,
                    line,
                    font,
                    &n.choices[i].text,
                    DraftField::Choice(i),
                    n.id,
                    format!("Ответ {} ноды #{}", i + 1, n.id),
                );
                icons::button(
                    commands,
                    line,
                    icons::Glyph::Close,
                    &format!("Удалить ответ {} ноды #{}", i + 1, n.id),
                    Action::Narrative(local(n.id, NodeAction::RemoveChoice(i))),
                    Look::Regular,
                );
            } else {
                node(
                    commands,
                    line,
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                );
            }
            links::socket(
                commands,
                card,
                n.id,
                i,
                output_position(n, i) - Vec2::splat(EDGE_WIDTH),
            );
        }
        if n.kind == NodeKind::Line {
            let footer = node(
                commands,
                card,
                Node {
                    height: px(FOOTER),
                    width: percent(100),
                    padding: UiRect::all(px(10)),
                    ..row(0.0)
                },
            );
            let add = act(
                commands,
                footer,
                font,
                "+ Вариант ответа",
                local(n.id, NodeAction::AddChoice),
                false,
            );
            full_button(commands, add);
        }
    }
}

pub(super) fn hint(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    linking: Option<(u32, usize)>,
    dialogue: &Dialogue,
) {
    if let Some((from, choice)) = linking {
        let hint = surface(
            commands,
            parent,
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                bottom: px(16),
                padding: UiRect::all(px(12)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(10)),
                ..column(10.0)
            },
            CHROME,
        );
        commands.entity(hint).insert(ZIndex(10));
        text(
            commands,
            hint,
            font,
            format!("Выход {} ноды #{} → выберите вход ноды", choice + 1, from),
            12.0,
            FOCUS,
        );
        let row = node(commands, hint, row(6.0));
        if dialogue
            .node(from)
            .and_then(|n| n.choices.get(choice))
            .is_some_and(|c| c.target.is_some())
        {
            act(
                commands,
                row,
                font,
                "Разорвать",
                local(from, NodeAction::Disconnect(choice)),
                false,
            );
        }
        for (caption, action) in [
            ("+ Реплика", NarrativeAction::Add(NodeKind::Line)),
            ("+ Конец", NarrativeAction::Add(NodeKind::End)),
            ("Список нод", NarrativeAction::Target(u32::MAX)),
            ("Отмена · Esc", NarrativeAction::Dismiss),
        ] {
            act(commands, row, font, caption, action, false);
        }
    }
}
