//! Upload only a paint batch's damage, preserving the canvas's GPU texture and bind groups.
use bevy::{
    asset::AssetId,
    prelude::*,
    render::{
        ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{Extent3d, Origin3d, TexelCopyBufferLayout},
        renderer::RenderQueue,
        texture::GpuImage,
    },
};

enum Upload {
    Reset(AssetId<Image>),
    Patch {
        id: AssetId<Image>,
        origin: UVec2,
        size: UVec2,
        rgba: Vec<u8>,
    },
}

#[derive(Resource, Default)]
pub(super) struct TextureUploads(Vec<Upload>);

impl TextureUploads {
    pub fn reset(&mut self, id: AssetId<Image>) {
        self.0.push(Upload::Reset(id));
    }

    pub fn patch(&mut self, id: AssetId<Image>, origin: UVec2, size: UVec2, rgba: Vec<u8>) {
        self.0.push(Upload::Patch {
            id,
            origin,
            size,
            rgba,
        });
    }
}

#[derive(Resource, Default)]
pub(in crate::editor) struct PendingUploads(Vec<Upload>);

pub(super) struct PaintTexturePlugin;

impl Plugin for PaintTexturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TextureUploads>();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<PendingUploads>()
                .add_systems(ExtractSchedule, extract)
                .add_systems(
                    Render,
                    upload
                        .after(RenderSystems::PrepareAssets)
                        .before(RenderSystems::Queue),
                );
        }
    }
}

fn extract(mut main: ResMut<MainWorld>, mut pending: ResMut<PendingUploads>) {
    let updates = std::mem::take(&mut main.resource_mut::<TextureUploads>().0);
    for update in updates {
        if let Upload::Reset(id) = update {
            // A full asset upload supersedes patches queued before undo, a layer edit or a tab switch.
            pending
                .0
                .retain(|patch| !matches!(patch, Upload::Patch { id: old, .. } if *old == id));
        } else {
            pending.0.push(update);
        }
    }
    let images = main.resource::<Assets<Image>>();
    pending
        .0
        .retain(|patch| matches!(patch, Upload::Patch { id, .. } if images.contains(*id)));
}

pub(in crate::editor) fn upload(
    mut pending: ResMut<PendingUploads>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
) {
    pending.0.retain(|patch| {
        let Upload::Patch {
            id,
            origin,
            size,
            rgba,
        } = patch
        else {
            return false;
        };
        let Some(image) = images.get(*id) else {
            return true;
        };
        if origin.x + size.x > image.texture_descriptor.size.width
            || origin.y + size.y > image.texture_descriptor.size.height
        {
            return false;
        }
        let mut target = image.texture.as_image_copy();
        target.origin = Origin3d {
            x: origin.x,
            y: origin.y,
            z: 0,
        };
        queue.write_texture(
            target,
            rgba,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.x * 4),
                rows_per_image: Some(size.y),
            },
            Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
        );
        false
    });
}
