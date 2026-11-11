//! Decorative star motion runs entirely in the UI shader, below editor controls.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct StarfieldMaterial {
    // Opacity, drift speed, seed, and padding; vec4 preserves uniform alignment.
    #[uniform(0)]
    parameters: Vec4,
}

impl UiMaterial for StarfieldMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("starfield.wesl")).with_source("embedded"),
        )
    }
}

#[derive(Resource)]
pub(super) struct Starfield {
    pub chrome: Handle<StarfieldMaterial>,
    pub space: Handle<StarfieldMaterial>,
}

impl FromWorld for Starfield {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<StarfieldMaterial>>();
        Self {
            chrome: materials.add(StarfieldMaterial {
                parameters: Vec4::new(0.26, 0.7, 37.0, 0.0),
            }),
            space: materials.add(StarfieldMaterial {
                parameters: Vec4::new(0.70, 1.0, 5.0, 0.0),
            }),
        }
    }
}

pub(super) struct StarfieldPlugin;

impl Plugin for StarfieldPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "starfield.wesl");
        app.add_plugins(UiMaterialPlugin::<StarfieldMaterial>::default())
            .init_resource::<Starfield>();
    }
}

pub(super) fn background(
    commands: &mut Commands,
    parent: Entity,
    material: &Handle<StarfieldMaterial>,
) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            ..default()
        },
        MaterialNode(material.clone()),
        ZIndex(-1),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
