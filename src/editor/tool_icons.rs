//! Compact transform controls using the editor's shared SVG glyphs.
use bevy::prelude::*;

use super::{Action, EDGE, Look, Tool, control, icons, row, surface};

pub(super) const IDLE: Color = Color::srgb(0.102, 0.105, 0.133);
pub(super) const HOVER: Color = Color::srgb(0.183, 0.168, 0.218);

pub(super) fn label(tool: Tool, model: bool) -> &'static str {
    match (tool, model) {
        (Tool::Universal, true) => "Выделение (Q)",
        (Tool::Move, true) => "Двигать (G)",
        (Tool::Rotate, true) => "Поворот (R)",
        (Tool::Scale, true) => "Масштаб (S)",
        (Tool::Universal, false) => "Выделение (Q)",
        (Tool::Move, false) => "Двигать (W)",
        (Tool::Rotate, false) => "Поворот (E)",
        (Tool::Scale, false) => "Масштаб (R)",
    }
}

pub(super) fn setup(commands: &mut Commands, parent: Entity) {
    group(commands, parent, false, Action::Tool);
}

pub(super) fn group(
    commands: &mut Commands,
    parent: Entity,
    model: bool,
    action: impl Fn(Tool) -> Action,
) -> Entity {
    let group = surface(
        commands,
        parent,
        Node {
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            overflow: Overflow::clip(),
            flex_shrink: 0.0,
            ..row(0.0)
        },
        IDLE,
    );
    for (index, (tool, glyph)) in [
        (Tool::Universal, icons::Glyph::Cursor),
        (Tool::Move, icons::Glyph::Move),
        (Tool::Rotate, icons::Glyph::Rotate),
        (Tool::Scale, icons::Glyph::Scale),
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            commands.spawn((
                Node {
                    width: px(1),
                    height: px(30),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(EDGE),
                Pickable::IGNORE,
                ChildOf(group),
            ));
        }
        let button = control(
            commands,
            group,
            action(tool),
            Look::Tool,
            Node {
                width: px(30),
                height: px(30),
                border: UiRect::all(px(1)),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        );
        commands
            .entity(button)
            .insert(AccessibleLabel(label(tool, model).into()));
        icons::themed(commands, button, glyph, 18.0);
    }
    group
}
