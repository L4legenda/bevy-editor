//! Tool-independent canvas sampling and a GPU pixel grid anchored to document texels.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path, uuid_handle},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

pub(super) const HANDLE: Handle<CanvasMaterial> =
    uuid_handle!("78d51f7a-a2c0-4a2b-bbcd-e7a1eb55b2ce");

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub(in crate::editor) struct CanvasMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub image: Option<Handle<Image>>,
    // Original document size, full-resolution texture flag, physical pixels per document pixel.
    #[uniform(2)]
    pub parameters: Vec4,
    #[texture(3)]
    #[sampler(4)]
    pub selection: Option<Handle<Image>>,
    #[uniform(5)]
    pub selection_style: Vec4,
    #[uniform(6)]
    pub preview_rect: Vec4,
    #[texture(7)]
    #[sampler(8)]
    pub transform_under: Option<Handle<Image>>,
    #[texture(9)]
    #[sampler(10)]
    pub transform_moving: Option<Handle<Image>>,
    #[texture(11)]
    #[sampler(12)]
    pub transform_above: Option<Handle<Image>>,
    #[uniform(13)]
    pub transform_rows: [Vec4; 3],
    #[uniform(14)]
    pub transform_source: Vec4,
    #[uniform(15)]
    pub transform_style: Vec4,
    #[uniform(16)]
    pub transform_frame: Vec4,
    #[texture(17)]
    #[sampler(18)]
    pub transform_selection: Option<Handle<Image>>,
    #[texture(19)]
    #[sampler(20)]
    pub pencil_mask: Option<Handle<Image>>,
    #[uniform(21)]
    pub pencil_rect: Vec4,
    #[uniform(22)]
    pub pencil_color: Vec4,
}

impl UiMaterial for CanvasMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("image_canvas.wesl")).with_source("embedded"),
        )
    }
}

pub(in crate::editor) struct ImageCanvasPlugin;

impl Plugin for ImageCanvasPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "image_canvas.wesl");
        app.add_plugins(UiMaterialPlugin::<CanvasMaterial>::default());
        app.add_systems(
            Update,
            super::tool_cursor::pencil::refresh
                .after(super::tool_cursor::cursor)
                .after(super::refresh),
        );
        // UUID handles have no generation and cannot fail insertion.
        let _ = app
            .world_mut()
            .resource_mut::<Assets<CanvasMaterial>>()
            .insert(HANDLE.id(), CanvasMaterial::default());
    }
}
