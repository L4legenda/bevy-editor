//! Animated glass and plasma are drawn separately from the slider hit targets.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct CosmicMaterial {
    // Surface kind, interaction emphasis, normalized setting, and field seed.
    #[uniform(0)]
    state: Vec4,
    // Pixel density and the visual padding around the fixed energy hit target.
    #[uniform(1)]
    metrics: Vec4,
}

impl UiMaterial for CosmicMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("image_brush_rail_fx.wesl"))
                .with_source("embedded"),
        )
    }
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Glass,
    Energy,
    Flow,
}

#[derive(Component)]
struct Surface {
    kind: Kind,
    seed: f32,
    padding: Vec2,
    material: Option<Handle<CosmicMaterial>>,
}

#[derive(Component, Default, Clone, Copy, PartialEq)]
pub(super) struct Interaction {
    pub emphasis: f32,
    pub position: f32,
}

pub(super) fn attach(commands: &mut Commands, entity: Entity, kind: Kind, seed: f32) {
    commands.entity(entity).insert((
        Surface {
            kind,
            seed,
            padding: Vec2::ZERO,
            material: None,
        },
        Interaction::default(),
    ));
}

pub(super) fn energy(commands: &mut Commands, parent: Entity, seed: f32) -> Entity {
    // The halo can extend past the handle without changing picking or slider travel.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(-8),
                right: px(-8),
                top: px(-10),
                bottom: px(-10),
                ..default()
            },
            Pickable::IGNORE,
            Surface {
                kind: Kind::Energy,
                seed,
                padding: Vec2::new(8.0, 10.0),
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
        app.add_plugins(UiMaterialPlugin::<CosmicMaterial>::default())
            .add_systems(Update, synchronize.after(super::ui::refresh_photo_editor));
    }
}

fn synchronize(
    mut commands: Commands,
    mut surfaces: Query<(Entity, &mut Surface, &Interaction, &ComputedNode)>,
    mut materials: ResMut<Assets<CosmicMaterial>>,
) {
    for (entity, mut surface, interaction, node) in &mut surfaces {
        if node.is_empty() {
            continue;
        }
        let kind = match surface.kind {
            Kind::Glass => 0.0,
            Kind::Energy => 1.0,
            Kind::Flow => 2.0,
        };
        let state = Vec4::new(
            kind,
            interaction.emphasis,
            interaction.position,
            surface.seed,
        );
        let metrics = Vec4::new(
            node.inverse_scale_factor().recip(),
            surface.padding.x,
            surface.padding.y,
            0.0,
        );
        if let Some(handle) = &surface.material {
            if materials
                .get(handle)
                .is_some_and(|material| material.state != state || material.metrics != metrics)
                && let Some(mut material) = materials.get_mut(handle)
            {
                material.state = state;
                material.metrics = metrics;
            }
        } else {
            let handle = materials.add(CosmicMaterial { state, metrics });
            commands.entity(entity).insert(MaterialNode(handle.clone()));
            surface.material = Some(handle);
        }
    }
}
