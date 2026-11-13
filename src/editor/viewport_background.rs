//! An opaque far-depth backdrop, isolated from scene documents and game cameras.
use bevy::{
    asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path},
    camera::visibility::{NoFrustumCulling, RenderLayers},
    light::{NotShadowCaster, NotShadowReceiver, Skybox},
    material::OpaqueRendererMethod,
    mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

use super::{Editor, viewport};

#[derive(Component)]
pub(super) struct Background;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct BackgroundMaterial {
    #[uniform(0)]
    top: LinearRgba,
    #[uniform(1)]
    bottom: LinearRgba,
    #[uniform(2)]
    violet: LinearRgba,
    #[uniform(3)]
    gold: LinearRgba,
}

impl Material for BackgroundMaterial {
    fn vertex_shader() -> ShaderRef {
        Self::fragment_shader()
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("viewport_background.wesl"))
                .with_source("embedded"),
        )
    }

    fn opaque_render_method(&self) -> OpaqueRendererMethod {
        OpaqueRendererMethod::Forward
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
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

pub(super) struct BackgroundPlugin;

impl Plugin for BackgroundPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "viewport_background.wesl");
        app.add_plugins(MaterialPlugin::<BackgroundMaterial>::default())
            .add_systems(Startup, setup);
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BackgroundMaterial>>,
) {
    // Explicit positions avoid depending on the mesh allocator's vertex offset.
    // The custom vertex stage emits these directly in clip space at far depth.
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-1.0, 1.0, -1.0], [-1.0, -3.0, 1.0], [3.0, 1.0, -1.0]],
    );
    commands.spawn((
        Name::new("Editor gradient backdrop"),
        Background,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(BackgroundMaterial {
            top: Color::srgb(0.030, 0.039, 0.061).to_linear(),
            bottom: Color::srgb(0.070, 0.080, 0.106).to_linear(),
            violet: Color::srgb(0.145, 0.122, 0.190).to_linear(),
            gold: Color::srgb(0.160, 0.117, 0.075).to_linear(),
        })),
        Transform::IDENTITY,
        Visibility::Hidden,
        RenderLayers::layer(viewport::EDITOR_GIZMO_LAYER),
        NoFrustumCulling,
        Pickable::IGNORE,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

pub(super) fn sync(
    editor: Res<Editor>,
    camera: Query<
        (&Camera, &Transform, Option<&Skybox>),
        (With<viewport::EditorCamera>, Without<Background>),
    >,
    images: Res<Assets<Image>>,
    mut backgrounds: Query<(&mut Visibility, &mut Transform), With<Background>>,
) {
    let view = camera.single().ok();
    let panorama_loaded = view
        .and_then(|(_, _, skybox)| skybox)
        .and_then(|skybox| skybox.image.as_ref())
        .is_some_and(|image| images.contains(image));
    let visible = editor.has_scene()
        && editor.ready
        && !editor.pilot
        && !panorama_loaded
        && view.is_some_and(|(camera, _, _)| camera.is_active);
    for (mut visibility, mut transform) in &mut backgrounds {
        visibility.set_if_neq(if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        // Keep the dummy bounds around the camera for optional GPU occlusion
        // culling. The vertex shader itself ignores the world transform.
        if let Some((_, pose, _)) = view
            && transform.translation != pose.translation
        {
            transform.translation = pose.translation;
        }
    }
}
