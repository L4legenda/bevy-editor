//! Shared visual language for the editor and the project manager.
use bevy::prelude::*;

pub(crate) const CHROME: Color = Color::srgb(0.077, 0.081, 0.100);
pub(crate) const PANEL: Color = Color::srgb(0.047, 0.055, 0.068);
pub(crate) const WELL: Color = Color::srgb(0.032, 0.038, 0.049);
pub(crate) const EDGE: Color = Color::srgb(0.195, 0.194, 0.221);
pub(crate) const HOVER: Color = Color::srgb(0.160, 0.153, 0.187);
pub(crate) const ACTIVE: Color = Color::srgb(0.225, 0.183, 0.135);
pub(crate) const ACCENT: Color = Color::srgb(0.755, 0.584, 0.359);
pub(crate) const TEXT: Color = Color::srgb(0.944, 0.932, 0.902);
pub(crate) const MUTED: Color = Color::srgb(0.652, 0.659, 0.711);
pub(crate) const FOCUS: Color = Color::srgb(0.909, 0.766, 0.536);
pub(crate) const ON_ACCENT: Color = Color::srgb(0.025, 0.020, 0.016);
pub(crate) const BLUE: Color = Color::srgb(0.510, 0.729, 1.0);
pub(crate) const GREEN: Color = Color::srgb(0.476, 0.803, 0.674);
pub(crate) const AMBER: Color = Color::srgb(1.0, 0.775, 0.433);
pub(crate) const ERROR: Color = Color::srgb(1.0, 0.40, 0.40);
pub(crate) const DISABLED_TEXT: Color = Color::srgb(0.48, 0.50, 0.55);
pub(crate) const AXIS: [Color; 3] = [
    Color::srgb(1.0, 0.54, 0.54),
    Color::srgb(0.53, 0.88, 0.64),
    BLUE,
];

pub(crate) fn gradient(angle: f32, colors: [Color; 3]) -> BackgroundGradient {
    LinearGradient::new(angle, colors.into_iter().map(ColorStop::auto).collect()).into()
}

pub(crate) fn surface(
    commands: &mut Commands,
    parent: Entity,
    layout: Node,
    color: Color,
) -> Entity {
    let entity = commands
        .spawn((
            layout,
            BackgroundColor(color),
            BorderColor::all(EDGE),
            ChildOf(parent),
        ))
        .id();
    // Icon and preview colors stay literal; only chrome surfaces get a gradient.
    let colors = if color == CHROME {
        Some([
            Color::srgb(0.125, 0.111, 0.110),
            CHROME,
            Color::srgb(0.090, 0.086, 0.128),
        ])
    } else if color == PANEL {
        Some([
            Color::srgb(0.081, 0.078, 0.105),
            PANEL,
            Color::srgb(0.036, 0.045, 0.054),
        ])
    } else if color == WELL {
        Some([Color::srgb(0.060, 0.057, 0.073), WELL, WELL])
    } else {
        None
    };
    if let Some(colors) = colors {
        commands
            .entity(entity)
            .insert(gradient(LinearGradient::TO_BOTTOM_RIGHT, colors));
    }
    entity
}

pub(crate) fn primary_gradient(highlighted: bool) -> BackgroundGradient {
    gradient(
        LinearGradient::TO_BOTTOM_RIGHT,
        [
            if highlighted {
                Color::srgb(0.985, 0.873, 0.660)
            } else {
                FOCUS
            },
            ACCENT,
            Color::srgb(0.700, 0.535, 0.330),
        ],
    )
}

pub(crate) fn active_gradient() -> BackgroundGradient {
    gradient(
        LinearGradient::TO_BOTTOM_RIGHT,
        [ACTIVE, Color::srgb(0.125, 0.116, 0.137), PANEL],
    )
}

pub(crate) fn horizon(commands: &mut Commands, parent: Entity) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            right: px(0),
            bottom: px(0),
            height: px(1),
            ..default()
        },
        gradient(
            LinearGradient::TO_RIGHT,
            [
                Color::srgba(0.91, 0.73, 0.47, 0.65),
                Color::srgba(0.63, 0.54, 0.77, 0.23),
                Color::srgba(0.63, 0.54, 0.77, 0.0),
            ],
        ),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
