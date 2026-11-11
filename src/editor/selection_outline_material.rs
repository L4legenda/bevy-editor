//! Screen-space selection contour, composited only over the editor viewport.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct OutlineMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub mask: Handle<Image>,
    #[uniform(2)]
    pub color: LinearRgba,
    // A vec4 keeps this binding aligned on all renderer backends. X is the
    // contour's physical pixel width; zero disables stale masks immediately.
    #[uniform(3)]
    width: Vec4,
}

impl OutlineMaterial {
    pub fn new(mask: Handle<Image>) -> Self {
        Self {
            mask,
            color: Color::srgb(1.0, 0.81, 0.38).to_linear(),
            width: Vec4::new(2.5, 0.0, 0.0, 0.0),
        }
    }

    pub fn set_width(&mut self, physical_pixels: f32) {
        self.width.x = if physical_pixels.is_finite() {
            physical_pixels.clamp(0.0, 32.0)
        } else {
            0.0
        };
    }
}

impl UiMaterial for OutlineMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("selection_outline.wesl"))
                .with_source("embedded"),
        )
    }
}

/// The mask pass must ignore source vertex colors, textures, and lighting.
/// Reusing the PBR vertex shader retains skinning and morph target support.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct OutlineMaskMaterial {
    #[uniform(0)]
    pub color: LinearRgba,
}

impl OutlineMaskMaterial {
    pub fn new(color: LinearRgba) -> Self {
        Self { color }
    }
}

impl Material for OutlineMaskMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("selection_outline_mask.wesl"))
                .with_source("embedded"),
        )
    }

    fn enable_shadows() -> bool {
        false
    }

    fn enable_prepass() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub(crate) struct OutlineMaterialPlugin;

impl Plugin for OutlineMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "selection_outline.wesl");
        embedded_asset!(app, "selection_outline_mask.wesl");
        app.add_plugins((
            UiMaterialPlugin::<OutlineMaterial>::default(),
            MaterialPlugin::<OutlineMaskMaterial>::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_width_hides_the_mask_and_invalid_widths_stay_finite() {
        let mut material = OutlineMaterial::new(Handle::default());
        material.set_width(0.0);
        assert_eq!(material.width.x, 0.0);
        material.set_width(f32::NAN);
        assert_eq!(material.width.x, 0.0);
        material.set_width(5.0);
        assert_eq!(material.width.x, 5.0);
    }
}
