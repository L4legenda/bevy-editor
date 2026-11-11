//! Static orbital artwork built from native UI geometry and gradients.
use bevy::{
    prelude::*,
    ui::{
        BackgroundGradient, BorderGradient, ColorStop, CornerRadius, LinearGradient,
        RadialGradient, RadialGradientShape, UiPosition, UiTransform, Val2,
    },
};

const GOLD: Color = Color::srgb(0.91, 0.72, 0.43);
const IVORY: Color = Color::srgb(1.0, 0.95, 0.82);

fn part(
    commands: &mut Commands,
    parent: Entity,
    width: f32,
    height: f32,
    offset: Vec2,
    angle: f32,
) -> Entity {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: percent(50),
                width: px(width),
                height: px(height),
                margin: UiRect {
                    left: px(offset.x),
                    top: px(offset.y),
                    ..default()
                },
                border_radius: BorderRadius::all(CornerRadius::MAX_ELLIPTICAL),
                ..default()
            },
            UiTransform {
                translation: Val2::percent(-50, -50),
                rotation: Rot2::degrees(angle),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

fn radial(commands: &mut Commands, entity: Entity, stops: Vec<ColorStop>) {
    commands
        .entity(entity)
        .insert(BackgroundGradient::from(RadialGradient::new(
            UiPosition::CENTER,
            RadialGradientShape::ClosestCorner,
            stops,
        )));
}

fn ring(
    commands: &mut Commands,
    parent: Entity,
    size: Vec2,
    angle: f32,
    stroke: f32,
    colors: [Color; 3],
) -> Entity {
    let entity = part(commands, parent, size.x, size.y, Vec2::ZERO, angle);
    commands.entity(entity).insert((
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            width: px(size.x),
            height: px(size.y),
            border: UiRect::all(px(stroke)),
            border_radius: BorderRadius::all(CornerRadius::MAX_ELLIPTICAL),
            ..default()
        },
        BorderGradient::from(LinearGradient::new(
            LinearGradient::TO_BOTTOM_RIGHT,
            colors.into_iter().map(ColorStop::auto).collect(),
        )),
    ));
    entity
}

pub(super) fn brand(commands: &mut Commands, parent: Entity) -> Entity {
    let mark = commands
        .spawn((
            Node {
                width: px(31),
                height: px(31),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundGradient::from(LinearGradient::new(
                LinearGradient::TO_BOTTOM_RIGHT,
                vec![
                    ColorStop::auto(Color::srgb(0.16, 0.14, 0.18)),
                    ColorStop::auto(Color::srgb(0.045, 0.048, 0.065)),
                ],
            )),
            BorderColor::all(Color::srgba(0.79, 0.65, 0.44, 0.36)),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id();
    let glow = part(commands, mark, 28.0, 28.0, Vec2::ZERO, 0.0);
    radial(
        commands,
        glow,
        vec![
            ColorStop::percent(Color::srgba(0.9, 0.67, 0.37, 0.28), 0),
            ColorStop::percent(Color::srgba(0.9, 0.67, 0.37, 0.0), 100),
        ],
    );
    let core = ring(
        commands,
        mark,
        Vec2::splat(16.0),
        0.0,
        1.2,
        [GOLD, IVORY, Color::srgb(0.65, 0.49, 0.35)],
    );
    commands
        .entity(core)
        .insert(BackgroundColor(Color::srgb(0.035, 0.036, 0.048)));
    ring(
        commands,
        mark,
        Vec2::new(28.0, 9.0),
        -24.0,
        0.8,
        [
            Color::srgba(0.92, 0.77, 0.53, 0.28),
            IVORY,
            Color::srgba(0.92, 0.77, 0.53, 0.36),
        ],
    );
    let star = part(commands, mark, 2.0, 2.0, Vec2::new(9.0, -8.0), 0.0);
    commands.entity(star).insert(BackgroundColor(IVORY));
    mark
}

pub(super) fn illustration(commands: &mut Commands, parent: Entity) -> Entity {
    let stage = commands
        .spawn((
            Node {
                width: px(400),
                max_width: percent(100),
                height: px(200),
                flex_shrink: 0.0,
                align_self: AlignSelf::Center,
                overflow: Overflow::clip(),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id();

    let cool_glow = part(commands, stage, 360.0, 198.0, Vec2::ZERO, 0.0);
    radial(
        commands,
        cool_glow,
        vec![
            ColorStop::percent(Color::srgba(0.40, 0.35, 0.62, 0.19), 0),
            ColorStop::percent(Color::srgba(0.34, 0.31, 0.54, 0.07), 55),
            ColorStop::percent(Color::srgba(0.34, 0.31, 0.54, 0.0), 100),
        ],
    );

    // Thin, slightly different inclinations keep the orbits readable at UI scale.
    for (width, height, angle, opacity) in [
        (366.0, 75.0, -11.0, 0.13),
        (328.0, 94.0, -16.0, 0.22),
        (294.0, 113.0, -23.0, 0.15),
        (252.0, 152.0, -31.0, 0.10),
    ] {
        ring(
            commands,
            stage,
            Vec2::new(width, height),
            angle,
            0.65,
            [
                Color::srgba(0.67, 0.62, 0.76, opacity * 0.45),
                Color::srgba(0.89, 0.76, 0.56, opacity),
                Color::srgba(0.65, 0.60, 0.77, opacity * 0.35),
            ],
        );
    }

    let warm_glow = part(commands, stage, 190.0, 176.0, Vec2::new(0.0, 5.0), 0.0);
    radial(
        commands,
        warm_glow,
        vec![
            ColorStop::percent(Color::srgba(0.96, 0.72, 0.36, 0.0), 38),
            ColorStop::percent(Color::srgba(0.96, 0.72, 0.36, 0.18), 66),
            ColorStop::percent(Color::srgba(0.96, 0.72, 0.36, 0.0), 100),
        ],
    );
    ring(
        commands,
        stage,
        Vec2::splat(138.0),
        0.0,
        0.65,
        [
            Color::srgba(0.67, 0.60, 0.82, 0.13),
            Color::srgba(0.93, 0.78, 0.52, 0.34),
            Color::srgba(0.93, 0.78, 0.52, 0.09),
        ],
    );
    ring(
        commands,
        stage,
        Vec2::splat(130.0),
        0.0,
        0.8,
        [
            Color::srgba(0.88, 0.79, 0.70, 0.30),
            Color::srgba(1.0, 0.86, 0.61, 0.59),
            Color::srgba(0.88, 0.79, 0.70, 0.17),
        ],
    );
    let eclipse = ring(
        commands,
        stage,
        Vec2::splat(121.0),
        0.0,
        1.6,
        [Color::srgb(0.43, 0.42, 0.54), GOLD, IVORY],
    );
    commands.entity(eclipse).insert((
        BackgroundGradient::from(LinearGradient::new(
            LinearGradient::TO_BOTTOM_RIGHT,
            vec![
                ColorStop::auto(Color::srgb(0.052, 0.051, 0.072)),
                ColorStop::auto(Color::srgb(0.014, 0.018, 0.026)),
            ],
        )),
        BoxShadow::new(
            Color::srgba(0.98, 0.77, 0.43, 0.18),
            px(0),
            px(0),
            px(1),
            px(16),
        ),
    ));

    // A soft accretion band passes in front of the dark centre.
    let beam_glow = part(commands, stage, 398.0, 36.0, Vec2::new(0.0, 15.0), 0.0);
    radial(
        commands,
        beam_glow,
        vec![
            ColorStop::percent(Color::srgba(1.0, 0.86, 0.61, 0.21), 0),
            ColorStop::percent(Color::srgba(1.0, 0.86, 0.61, 0.0), 100),
        ],
    );
    let beam = part(commands, stage, 392.0, 1.0, Vec2::new(0.0, 15.0), 0.0);
    commands
        .entity(beam)
        .insert(BackgroundGradient::from(LinearGradient::to_right(vec![
            ColorStop::percent(Color::srgba(0.95, 0.79, 0.53, 0.0), 0),
            ColorStop::percent(Color::srgba(0.95, 0.79, 0.53, 0.32), 22),
            ColorStop::percent(IVORY, 50),
            ColorStop::percent(Color::srgba(0.95, 0.79, 0.53, 0.32), 78),
            ColorStop::percent(Color::srgba(0.95, 0.79, 0.53, 0.0), 100),
        ])));

    for (x, y, size, opacity) in [
        (-133.0, -38.0, 2.0, 0.50),
        (116.0, -65.0, 2.5, 0.67),
        (158.0, 33.0, 1.5, 0.35),
        (-90.0, 64.0, 1.5, 0.30),
    ] {
        let star = part(commands, stage, size, size, Vec2::new(x, y), 0.0);
        commands
            .entity(star)
            .insert(BackgroundColor(Color::srgba(0.98, 0.88, 0.70, opacity)));
    }
    stage
}
