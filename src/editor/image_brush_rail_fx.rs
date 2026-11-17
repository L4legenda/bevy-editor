//! Quiet, theme-matched rail surfaces drawn independently of the slider hit targets.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct RailMaterial {
    // Surface kind, interaction emphasis, pixel density, reveal opacity.
    #[uniform(0)]
    state: Vec4,
    // Theme color in linear RGB; opaque panel fill matches the tools sidebar.
    #[uniform(1)]
    tint: Vec4,
}

impl UiMaterial for RailMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("image_brush_rail_fx.wesl"))
                .with_source("embedded"),
        )
    }
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Panel,
    Handle,
    Fill,
}

#[derive(Component)]
struct Surface {
    kind: Kind,
    material: Option<Handle<RailMaterial>>,
}

#[derive(Component, Default, Clone, Copy, PartialEq)]
pub(super) struct Interaction {
    pub emphasis: f32,
}

#[derive(Component, Default)]
pub(super) struct Reveal {
    progress: f32,
}

#[derive(Component)]
pub(super) struct RevealTarget(pub bool);

#[derive(Component)]
pub(super) struct Track;

fn animate(
    time: Res<Time>,
    mut panels: Query<(
        &mut Reveal,
        &RevealTarget,
        &mut Node,
        &mut UiTransform,
        &mut BoxShadow,
    )>,
    mut tracks: Query<(&mut BackgroundColor, &mut BorderColor), With<Track>>,
) {
    for (mut reveal, target, mut node, mut transform, mut shadow) in &mut panels {
        let step = time.delta_secs().min(0.05) / 0.18;
        reveal.progress = (reveal.progress + if target.0 { step } else { -step }).clamp(0.0, 1.0);
        let p = reveal.progress * reveal.progress * (3.0 - 2.0 * reveal.progress);
        node.display = if reveal.progress > 0.0 {
            Display::Flex
        } else {
            Display::None
        };
        transform.translation = Val2 {
            x: px(-12.0 * (1.0 - p)),
            y: percent(-50),
        };
        transform.scale = Vec2::splat(0.86 + 0.14 * p);
        for style in &mut shadow.0 {
            style.color = Color::srgba(0.0, 0.0, 0.0, 0.45 * p);
        }
        for (mut background, mut border) in &mut tracks {
            background.0 = crate::ui_theme::WELL.with_alpha(p);
            *border = BorderColor::all(crate::ui_theme::EDGE.with_alpha(p));
        }
    }
}

pub(super) fn attach(commands: &mut Commands, entity: Entity, kind: Kind) {
    commands.entity(entity).insert((
        Surface {
            kind,
            material: None,
        },
        Interaction::default(),
    ));
}

pub(super) fn handle(commands: &mut Commands, parent: Entity) -> Entity {
    // A compact visual handle retains the larger, comfortable picking area.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(4),
                right: px(4),
                top: px(6),
                bottom: px(6),
                ..default()
            },
            Pickable::IGNORE,
            Surface {
                kind: Kind::Handle,
                material: None,
            },
            Interaction::default(),
            ChildOf(parent),
        ))
        .id()
}

pub(super) struct BrushRailFxPlugin;

impl Plugin for BrushRailFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "image_brush_rail_fx.wesl");
        app.add_plugins(UiMaterialPlugin::<RailMaterial>::default())
            .add_systems(
                Update,
                (animate, synchronize)
                    .chain()
                    .after(super::ui::refresh_photo_editor),
            );
    }
}

fn synchronize(
    mut commands: Commands,
    mut surfaces: Query<(Entity, &mut Surface, &Interaction, &ComputedNode)>,
    mut materials: ResMut<Assets<RailMaterial>>,
    parents: Query<&ChildOf>,
    reveals: Query<&Reveal>,
) {
    for (entity, mut surface, interaction, node) in &mut surfaces {
        if node.is_empty() {
            continue;
        }
        let (kind, color) = match surface.kind {
            Kind::Panel => (0.0, crate::ui_theme::CHROME),
            Kind::Handle => (1.0, Color::srgb(0.56, 0.57, 0.59)),
            Kind::Fill => (2.0, Color::srgb(0.29, 0.29, 0.31)),
        };
        let mut ancestor = entity;
        let mut opacity = 1.0;
        loop {
            if let Ok(reveal) = reveals.get(ancestor) {
                let p = reveal.progress;
                opacity = p * p * (3.0 - 2.0 * p);
                break;
            }
            let Ok(parent) = parents.get(ancestor) else {
                break;
            };
            ancestor = parent.parent();
        }
        let state = Vec4::new(
            kind,
            interaction.emphasis,
            node.inverse_scale_factor().recip(),
            opacity,
        );
        let color = color.to_linear();
        let tint = Vec4::new(color.red, color.green, color.blue, 1.0);
        if let Some(handle) = &surface.material {
            if materials
                .get(handle)
                .is_some_and(|material| material.state != state || material.tint != tint)
                && let Some(mut material) = materials.get_mut(handle)
            {
                material.state = state;
                material.tint = tint;
            }
        } else {
            let handle = materials.add(RailMaterial { state, tint });
            commands.entity(entity).insert(MaterialNode(handle.clone()));
            surface.material = Some(handle);
        }
    }
}
