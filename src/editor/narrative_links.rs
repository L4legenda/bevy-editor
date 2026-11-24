//! Connection gestures are transient. Only a valid drop changes the document.
use super::*;

const PORT_DIAMETER: f32 = 12.0;
const HIGHLIGHTED_DIAMETER: f32 = 14.0;
const IDLE_OPACITY: f32 = 0.75;

#[derive(Component)]
pub(super) struct Output(pub u32, pub usize);
#[derive(Component)]
pub(super) struct Input(pub u32);
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Port {
    Input(u32),
    Output(u32, usize),
}
#[derive(Debug)]
pub(super) struct Drag {
    document: usize,
    generation: u64,
    pub from: u32,
    pub choice: usize,
    origin: Vec2,
    pub pointer: Vec2,
    pub target: Option<u32>,
    pub moved: bool,
}
#[derive(Resource, Default)]
pub(super) struct State {
    pub drag: Option<Drag>,
    pub trace: Option<Port>,
    trace_context: Option<(usize, u64)>,
    pub ignore_click: Option<(u32, usize)>,
    pub block_pan: bool,
    ignore_frames: u8,
}
impl State {
    pub fn highlights_edge(&self, from: u32, choice: usize, target: u32) -> bool {
        match self.trace {
            Some(Port::Output(node, output)) => (from, choice) == (node, output),
            Some(Port::Input(node)) => target == node,
            None => false,
        }
    }

    pub fn highlights_node(&self, d: &Dialogue, id: u32) -> bool {
        match self.trace {
            Some(Port::Output(from, choice)) => d
                .node(from)
                .is_some_and(|n| n.output_target(choice) == Some(id)),
            Some(Port::Input(target)) => d
                .node(id)
                .is_some_and(|n| n.output_targets().any(|(_, to)| to == target)),
            None => false,
        }
    }

    fn highlights_port(&self, d: &Dialogue, port: Port) -> bool {
        match port {
            Port::Output(from, choice) => d
                .node(from)
                .and_then(|n| n.output_target(choice))
                .is_some_and(|target| self.highlights_edge(from, choice, target)),
            Port::Input(target) => d.nodes.iter().any(|n| {
                n.output_targets()
                    .any(|(i, to)| to == target && self.highlights_edge(n.id, i, target))
            }),
        }
    }

    fn suppress_click(&mut self, from: u32, choice: usize) {
        self.ignore_click = Some((from, choice));
        self.ignore_frames = 3;
    }
    fn cancel(&mut self, ui: &mut NarrativeUi) {
        if let Some(drag) = self.drag.take() {
            self.suppress_click(drag.from, drag.choice);
            ui.linking = None;
        }
    }
}

/// Inspect a connection without changing selection, history, or graph entities.
pub(super) fn hover(
    editor: Res<Editor>,
    ui: Res<NarrativeUi>,
    mut state: ResMut<State>,
    outputs: Query<(Entity, &Output, &Hovered)>,
    inputs: Query<(Entity, &Input, &Hovered)>,
    focus: Res<InputFocus>,
    focus_visible: Res<bevy::input_focus::InputFocusVisible>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<DrawerMotion>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    effects: Res<effects::State>,
) {
    state.trace = None;
    let Some(doc) = editor.narrative.current() else {
        state.trace_context = None;
        return;
    };
    let Content::Dialogue(d) = &doc.content else {
        state.trace_context = None;
        return;
    };
    let context = (doc.id, ui.generation);
    if state.trace_context != Some(context) {
        state.trace_context = Some(context);
        return;
    }
    if editor.pending.is_some()
        || editor.browser_popup
        || ui.preview
        || ui.picker.is_some()
        || ui.linking.is_some()
        || state.drag.is_some()
        || !window.focused
    {
        return;
    }
    if let Some((node, index)) = effects.answers.dragged_output() {
        state.trace = Some(Port::Output(node, index));
        return;
    }
    let focused = focus_visible.0.then(|| focus.get()).flatten();
    let keyboard = outputs
        .iter()
        .find(|(entity, _, _)| Some(*entity) == focused)
        .map(|(_, output, _)| Port::Output(output.0, output.1))
        .or_else(|| {
            inputs
                .iter()
                .find(|(entity, _, _)| Some(*entity) == focused)
                .map(|(_, input, _)| Port::Input(input.0))
        });
    let inside = window.cursor_position().is_some_and(|point| {
        geometry.get(ui.canvas).is_ok_and(|(size, transform)| {
            Rect::from_center_size(
                transform.translation / window.scale_factor(),
                size.size() / window.scale_factor(),
            )
            .contains(point)
                && point.y < window.height() - FOOT - drawer.visible_height
        })
    });
    let pointed = keyboard.or_else(|| {
        inside.then(|| {
            outputs
                .iter()
                .find(|(_, _, hovered)| hovered.0)
                .map(|(_, output, _)| Port::Output(output.0, output.1))
                .or_else(|| {
                    inputs
                        .iter()
                        .find(|(_, _, hovered)| hovered.0)
                        .map(|(_, input, _)| Port::Input(input.0))
                })
        })?
    });
    state.trace = pointed.filter(|port| match *port {
        Port::Output(from, choice) => d.node(from).is_some_and(|n| n.output_connected(choice)),
        Port::Input(target) => {
            d.node(target).is_some()
                && d.nodes
                    .iter()
                    .any(|n| n.output_targets().any(|(_, to)| to == target))
        }
    });
}

pub(super) fn connect(doc: &mut Document, from: u32, choice: usize, target: u32) -> bool {
    let Content::Dialogue(d) = &doc.content else {
        return false;
    };
    if d.node(target).is_none_or(|n| n.kind == NodeKind::Start)
        || d.node(from)
            .is_none_or(|n| !n.output_exists(choice) || n.output_target(choice) == Some(target))
    {
        return false;
    }
    doc.checkpoint();
    if let Content::Dialogue(d) = &mut doc.content {
        d.node_mut(from).unwrap().set_output(choice, Some(target));
    }
    true
}

pub(super) fn input(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<NarrativeUi>,
    mut state: ResMut<State>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<DrawerMotion>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    outputs: Query<(&Output, &Hovered)>,
    inputs: Query<(&Input, &Hovered)>,
    headers: Query<(&Control, &Hovered)>,
    mut focus: ResMut<InputFocus>,
    mut messages: MessageWriter<Action>,
    effects: Res<effects::State>,
) {
    if !mouse.pressed(MouseButton::Right) {
        state.block_pan = false;
    }
    if state.ignore_frames > 0 {
        state.ignore_frames -= 1;
    } else {
        state.ignore_click = None;
    }
    let allowed = editor.pending.is_none()
        && !editor.browser_popup
        && !ui.preview
        && ui.picker.is_none()
        && window.focused
        && !effects.answers.capturing();
    if !allowed || keys.just_pressed(KeyCode::Escape) {
        state.cancel(&mut ui);
        return;
    }
    let Some(doc) = editor.narrative.current_mut() else {
        state.cancel(&mut ui);
        return;
    };
    let Content::Dialogue(d) = &doc.content else {
        state.cancel(&mut ui);
        return;
    };
    if state.drag.as_ref().is_some_and(|drag| {
        drag.document != doc.id
            || drag.generation != ui.generation
            || d.node(drag.from)
                .is_none_or(|n| !n.output_exists(drag.choice))
    }) {
        state.cancel(&mut ui);
    }
    let Some(point) = window.cursor_position() else {
        state.cancel(&mut ui);
        return;
    };
    let Ok((size, transform)) = geometry.get(ui.canvas) else {
        return;
    };
    let rect = Rect::from_center_size(
        transform.translation / window.scale_factor(),
        size.size() / window.scale_factor(),
    );
    let inside = rect.contains(point) && point.y < window.height() - FOOT - drawer.visible_height;
    if inside && mouse.just_pressed(MouseButton::Right) {
        if let Some((output, _)) = outputs.iter().find(|(_, hovered)| hovered.0) {
            state.block_pan = true;
            state.cancel(&mut ui);
            ui.panning = false;
            messages.write(Action::Narrative(NarrativeAction::Node(
                output.0,
                nodes::NodeAction::Disconnect(output.1),
            )));
            return;
        }
    }
    if inside && mouse.just_pressed(MouseButton::Left) {
        state.ignore_click = None;
        if let Some((output, _)) = outputs.iter().find(|(_, hovered)| hovered.0) {
            if d.node(output.0).is_some_and(|n| n.output_exists(output.1)) {
                state.drag = Some(Drag {
                    document: doc.id,
                    generation: ui.generation,
                    from: output.0,
                    choice: output.1,
                    origin: point,
                    pointer: point - rect.min,
                    target: None,
                    moved: false,
                });
                ui.clear_node_drag();
                ui.panning = false;
                focus.clear();
            }
        }
    }
    let Some(drag) = state.drag.as_mut() else {
        return;
    };
    drag.pointer = point - rect.min;
    drag.moved |= point.distance_squared(drag.origin) >= 16.0;
    drag.target = if inside && drag.moved {
        headers
            .iter()
            .find_map(|(control, hovered)| match control.action {
                Action::Narrative(NarrativeAction::Select(id))
                    if hovered.0 && d.node(id).is_some_and(|n| n.kind != NodeKind::Start) =>
                {
                    Some(id)
                }
                _ => None,
            })
            .or_else(|| {
                inputs
                    .iter()
                    .find(|(input, hovered)| {
                        hovered.0 && d.node(input.0).is_some_and(|n| n.kind != NodeKind::Start)
                    })
                    .map(|(input, _)| input.0)
            })
    } else {
        None
    };
    if drag.moved {
        ui.linking = Some((drag.from, drag.choice));
        doc.selected = drag.from;
    }
    if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
        let drag = state.drag.take().unwrap();
        if drag.moved {
            if let Some(target) = drag.target {
                messages.write(Action::Narrative(NarrativeAction::Connect(
                    drag.from,
                    drag.choice,
                    target,
                )));
            }
            state.suppress_click(drag.from, drag.choice);
            ui.linking = None;
        }
    }
}

pub(super) fn style(
    editor: Res<Editor>,
    ui: Res<NarrativeUi>,
    state: Res<State>,
    effects: Res<effects::State>,
    mut ports: Query<(&Port, &mut BorderColor, &mut BackgroundColor, &mut Node)>,
    outputs: Query<(Entity, &Output, &Hovered)>,
    inputs: Query<(Entity, &Input, &Hovered)>,
    focus: Res<InputFocus>,
    focus_visible: Res<bevy::input_focus::InputFocusVisible>,
) {
    let Some(doc) = editor.narrative.current() else {
        return;
    };
    let Content::Dialogue(d) = &doc.content else {
        return;
    };
    for (port, mut border, mut background, mut node) in &mut ports {
        let (active, connected) = match *port {
            Port::Input(id) => (
                state
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.moved && drag.target == Some(id)),
                d.nodes
                    .iter()
                    .any(|n| n.output_targets().any(|(_, to)| to == id)),
            ),
            Port::Output(id, choice) => (
                ui.linking == Some((id, choice)),
                d.node(id).is_some_and(|n| n.output_connected(choice)),
            ),
        };
        let pointed = match *port {
            Port::Output(id, choice) => outputs.iter().any(|(entity, output, hovered)| {
                (output.0, output.1) == (id, choice)
                    && (hovered.0 || focus_visible.0 && focus.get() == Some(entity))
            }),
            Port::Input(id) => inputs.iter().any(|(entity, input, hovered)| {
                input.0 == id && (hovered.0 || focus_visible.0 && focus.get() == Some(entity))
            }),
        };
        let traced = state.highlights_port(d, *port);
        let color = if active || traced {
            GREEN
        } else if pointed || connected {
            FOCUS
        } else {
            FOCUS.with_alpha(IDLE_OPACITY)
        };
        let amount = match *port {
            Port::Output(id, index) => effects.answers.amount(id, index),
            _ => 1.,
        };
        let diameter = px((if active || pointed || traced {
            HIGHLIGHTED_DIAMETER
        } else {
            PORT_DIAMETER
        }) * amount);
        if node.width != diameter {
            node.width = diameter;
            node.height = diameter;
        }
        border.set_if_neq(BorderColor::all(color));
        background.set_if_neq(BackgroundColor(
            if active || connected || pointed || traced {
                if active || pointed || traced {
                    color
                } else {
                    FOCUS.with_alpha(0.9)
                }
            } else {
                WELL
            },
        ));
    }
}

fn dot(commands: &mut Commands, parent: Entity, port: Port) {
    commands.spawn((
        port,
        Node {
            width: px(PORT_DIAMETER),
            height: px(PORT_DIAMETER),
            flex_shrink: 0.0,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(HIGHLIGHTED_DIAMETER * 0.5)),
            ..default()
        },
        BorderColor::all(FOCUS.with_alpha(IDLE_OPACITY)),
        BackgroundColor(WELL),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}

fn hit_area(commands: &mut Commands, parent: Entity, center: Vec2, label: String) -> Entity {
    commands
        .spawn((
            Button,
            Hovered::default(),
            TabIndex(0),
            AccessibleLabel(label),
            Node {
                position_type: PositionType::Absolute,
                left: px(center.x - 14.0),
                top: px(center.y - 18.0),
                width: px(28),
                height: px(36),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            ZIndex(2),
            ChildOf(parent),
        ))
        .id()
}

pub(super) fn input_socket(commands: &mut Commands, parent: Entity, id: u32, center: Vec2) {
    let socket = hit_area(commands, parent, center, format!("Вход ноды #{id}"));
    commands.entity(socket).insert(Input(id)).observe(
        |event: On<Activate>, inputs: Query<&Input>, mut actions: MessageWriter<Action>| {
            if let Ok(input) = inputs.get(event.event_target()) {
                actions.write(Action::Narrative(NarrativeAction::Select(input.0)));
            }
        },
    );
    dot(commands, socket, Port::Input(id));
}

/// A socket is a small dot with an invisible, keyboard-accessible hit area.
pub(super) fn socket(
    commands: &mut Commands,
    parent: Entity,
    id: u32,
    choice: usize,
    center: Vec2,
) -> Entity {
    let socket = hit_area(
        commands,
        parent,
        center,
        if choice == OTHERWISE {
            format!("Связать «Иначе» ноды #{id}")
        } else {
            format!("Связать ответ {} ноды #{id}", choice + 1)
        },
    );
    commands.entity(socket).insert(Output(id, choice)).observe(
        |event: On<Activate>,
         outputs: Query<&Output>,
         mouse: Res<ButtonInput<MouseButton>>,
         mut actions: MessageWriter<Action>| {
            if mouse.any_pressed([MouseButton::Right, MouseButton::Middle])
                || mouse.any_just_released([MouseButton::Right, MouseButton::Middle])
            {
                return;
            }
            if let Ok(output) = outputs.get(event.event_target()) {
                actions.write(Action::Narrative(NarrativeAction::Link(output.0, output.1)));
            }
        },
    );
    dot(commands, socket, Port::Output(id, choice));
    socket
}
