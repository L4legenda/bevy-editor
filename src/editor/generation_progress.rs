//! Determinate transfers and an honest activity pulse for unknown work sizes.
use super::*;
use bevy::window::RequestRedraw;

#[derive(Component, Clone, Copy, PartialEq)]
pub(super) enum Value {
    Hidden,
    Busy,
    Fraction(f32),
}

pub(super) fn bar(commands: &mut Commands, parent: Entity, height: f32) -> (Entity, Entity) {
    let track = surface(
        commands,
        parent,
        Node {
            width: percent(100),
            height: px(height),
            flex_shrink: 0.0,
            overflow: Overflow::clip(),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        WELL,
    );
    let fill = surface(
        commands,
        track,
        Node {
            position_type: PositionType::Absolute,
            left: percent(0),
            width: percent(0),
            height: percent(100),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        FOCUS,
    );
    commands.entity(fill).insert(Value::Hidden);
    (track, fill)
}

pub(super) fn set(commands: &mut Commands, fill: Entity, value: Value) {
    commands.entity(fill).insert(value);
}

pub(super) fn animate(
    time: Res<Time<Real>>,
    mut fills: Query<(&Value, &mut Node)>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let mut busy = false;
    for (value, mut node) in &mut fills {
        let (left, width) = match *value {
            Value::Hidden => (0.0, 0.0),
            Value::Fraction(value) => (0.0, value.clamp(0.0, 1.0) * 100.0),
            Value::Busy => {
                busy = true;
                ((time.elapsed_secs() * 52.0).rem_euclid(132.0) - 32.0, 32.0)
            }
        };
        if node.left != percent(left) {
            node.left = percent(left);
        }
        if node.width != percent(width) {
            node.width = percent(width);
        }
    }
    if busy {
        redraw.write(RequestRedraw);
    }
}
