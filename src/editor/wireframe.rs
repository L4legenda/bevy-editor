//! Depth-tested line overlay without optional polygon-mode GPU features.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(super) struct WireMaterial {
    #[uniform(0)]
    tint: Vec4,
    sorting_bias: f32,
}

impl Default for WireMaterial {
    fn default() -> Self {
        Self {
            tint: Vec4::new(0.035, 0.027, 0.016, 1.0),
            sorting_bias: 0.0,
        }
    }
}

impl WireMaterial {
    pub(super) fn colored(tint: Vec4) -> Self {
        Self { tint, sorting_bias: 0.0 }
    }

    pub(super) fn after_other_lines(mut self) -> Self {
        // Transparent meshes are sorted back to front. The depth test still uses the mesh's clip position.
        self.sorting_bias = f32::INFINITY;
        self
    }
}

impl Material for WireMaterial {
    fn vertex_shader() -> ShaderRef {
        Self::fragment_shader()
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("wireframe.wesl")).with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        // Queue after the opaque surface so its depth test can hide rear edges.
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn depth_bias(&self) -> f32 {
        self.sorting_bias
    }
    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "wireframe.wesl");
    app.add_plugins(MaterialPlugin::<WireMaterial>::default());
}
