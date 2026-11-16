//! Procedural artwork for the empty result. Its clock stops with the pane.
use super::*;
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    window::RequestRedraw,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct WaitingMaterial {
    // Visible animation time, generation activity, reserved.
    #[uniform(0)]
    motion: Vec4,
}

impl UiMaterial for WaitingMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("generation_waiting.wesl"))
                .with_source("embedded"),
        )
    }
}

#[derive(Resource)]
struct Artwork(Handle<WaitingMaterial>);

impl FromWorld for Artwork {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource_mut::<Assets<WaitingMaterial>>()
                .add(WaitingMaterial { motion: Vec4::ZERO }),
        )
    }
}

#[derive(Component)]
struct Waiting;

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "generation_waiting.wesl");
    app.add_plugins(UiMaterialPlugin::<WaitingMaterial>::default())
        .init_resource::<Artwork>()
        .add_systems(Update, animate.after(run::refresh));
}

pub(super) fn illustration(commands: &mut Commands, parent: Entity) {
    commands.spawn((
        Waiting,
        Node {
            width: px(340),
            max_width: percent(100),
            height: px(270),
            flex_shrink: 1.0,
            min_height: px(100),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}

fn animate(
    mut commands: Commands,
    editor: Res<Editor>,
    time: Res<Time<Real>>,
    artwork: Res<Artwork>,
    mut materials: ResMut<Assets<WaitingMaterial>>,
    nodes: Query<
        (
            Entity,
            &ComputedNode,
            Option<&MaterialNode<WaitingMaterial>>,
        ),
        With<Waiting>,
    >,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let mut visible = false;
    for (entity, node, material) in &nodes {
        if material.is_none() {
            commands
                .entity(entity)
                .insert(MaterialNode(artwork.0.clone()));
        }
        visible |= !node.is_empty();
    }
    // Display::None on a parent removes the material from Bevy's render queue.
    // Do not invalidate assets or wake the window while another tab/result is up.
    if !visible || !editor.generation.active || editor.generation.has_result {
        return;
    }
    if let Some(mut material) = materials.get_mut(&artwork.0) {
        let dt = time.delta_secs().min(0.1);
        material.motion.x += dt;
        let activity = if editor.generation.running { 1.0 } else { 0.0 };
        material.motion.y += (activity - material.motion.y) * (1.0 - (-dt * 3.0).exp());
        redraw.write(RequestRedraw);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "generation_waiting_tests.rs"]
mod tests;
