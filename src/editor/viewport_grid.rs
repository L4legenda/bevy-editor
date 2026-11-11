//! A world-aligned, anti-aliased ground grid visible only in the editor view.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    camera::visibility::RenderLayers,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

use super::{Editor, viewport};

const HEIGHT: f32 = 0.008;
const EXTENT: f32 = 2000.0;
const RECENTER_STEP: f32 = 64.0;

#[derive(Component)]
pub(super) struct Grid;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct GridMaterial {
    // Minor spacing, major spacing, distance fade start/end, in metres.
    #[uniform(0)]
    parameters: Vec4,
}

impl Material for GridMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("viewport_grid.wesl")).with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub(super) struct GridPlugin;

impl Plugin for GridPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "viewport_grid.wesl");
        app.add_plugins(MaterialPlugin::<GridMaterial>::default())
            .add_systems(Startup, setup);
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GridMaterial>>,
) {
    commands.spawn((
        Name::new("Editor ground grid"),
        Grid,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(EXTENT, EXTENT))),
        MeshMaterial3d(materials.add(GridMaterial {
            parameters: Vec4::new(0.5, 2.0, 28.0, 72.0),
        })),
        Transform::from_xyz(0.0, HEIGHT, 0.0),
        Visibility::Hidden,
        RenderLayers::layer(viewport::EDITOR_GIZMO_LAYER),
        Pickable::IGNORE,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// Run after camera navigation. The mesh follows the view, while the shader's
/// world coordinates keep all half-metre divisions anchored to the origin.
pub(super) fn sync(
    editor: Res<Editor>,
    camera: Query<(&Camera, &Transform), (With<viewport::EditorCamera>, Without<Grid>)>,
    mut grids: Query<(&mut Transform, &mut Visibility), With<Grid>>,
) {
    let view = camera.single().ok().filter(|(camera, pose)| {
        editor.ready
            && editor.has_scene()
            && !editor.pilot
            && camera.is_active
            && pose.translation.is_finite()
    });
    for (mut transform, mut visibility) in &mut grids {
        let Some((_, pose)) = view else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        visibility.set_if_neq(Visibility::Visible);
        let center = Vec3::new(
            (pose.translation.x / RECENTER_STEP).floor() * RECENTER_STEP,
            HEIGHT,
            (pose.translation.z / RECENTER_STEP).floor() * RECENTER_STEP,
        );
        if transform.translation != center {
            transform.translation = center;
        }
    }
}
