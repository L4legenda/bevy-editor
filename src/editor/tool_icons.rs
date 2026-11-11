//! Compact transform controls using the editor's shared SVG glyphs.
use bevy::prelude::*;

use super::{Action, EDGE, Look, Tool, control, icons, row, surface};

pub(super) const IDLE: Color = Color::srgb(0.102, 0.105, 0.133);
pub(super) const HOVER: Color = Color::srgb(0.183, 0.168, 0.218);

pub(super) fn setup(commands: &mut Commands, parent: Entity) {
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
    for (index, (tool, glyph, label)) in [
        (Tool::Move, icons::Glyph::Move, "Двигать (W)"),
        (Tool::Rotate, icons::Glyph::Rotate, "Поворот (E)"),
        (Tool::Scale, icons::Glyph::Scale, "Масштаб (R)"),
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
            Action::Tool(tool),
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
            .insert(AccessibleLabel(label.into()));
        icons::themed(commands, button, glyph, 18.0);
    }
}
