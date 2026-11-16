//! Shared black-hole artwork. Ray bending and accretion flow run on the GPU.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct BlackHoleMaterial {
    // Exposure, flow speed, phase offset, reserved.
    #[uniform(0)]
    settings: Vec4,
}

impl UiMaterial for BlackHoleMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("orbital.wesl")).with_source("embedded"),
        )
    }
}

#[derive(Resource)]
struct Artwork(Handle<BlackHoleMaterial>);

impl FromWorld for Artwork {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource_mut::<Assets<BlackHoleMaterial>>()
                .add(BlackHoleMaterial {
                    settings: Vec4::new(1.15, 1.0, 0.0, 0.0),
                }),
        )
    }
}

#[derive(Component)]
struct BlackHole;

pub(crate) struct OrbitalPlugin;

impl Plugin for OrbitalPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "orbital.wesl");
        app.add_plugins(UiMaterialPlugin::<BlackHoleMaterial>::default())
            .init_resource::<Artwork>()
            .add_systems(Update, attach);
    }
}

fn attach(
    mut commands: Commands,
    artwork: Res<Artwork>,
    stages: Query<Entity, (With<BlackHole>, Without<MaterialNode<BlackHoleMaterial>>)>,
) {
    for entity in &stages {
        commands
            .entity(entity)
            .try_insert(MaterialNode(artwork.0.clone()));
    }
}

pub(crate) fn brand(commands: &mut Commands, parent: Entity) -> Entity {
    crate::app_icon::brand(commands, parent)
}

pub(crate) fn illustration(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            BlackHole,
            Node {
                width: px(440),
                max_width: percent(100),
                height: px(240),
                flex_shrink: 0.0,
                align_self: AlignSelf::Center,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "orbital_render_tests.rs"]
mod render_tests;
