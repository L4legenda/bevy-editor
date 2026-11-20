//! Resolve a 3D file to its preview world. GLB previews retain their default scene.
use bevy::{
    gltf::{Gltf, GltfLoaderSettings},
    prelude::*,
};

use crate::data::ModelFormat;

pub(super) enum ModelSource {
    Glb(Handle<Gltf>),
    Fbx(Handle<WorldAsset>),
}

impl ModelSource {
    pub(super) fn load(server: &AssetServer, path: &str, lights: bool) -> Self {
        if ModelFormat::from_path(path) == Some(ModelFormat::Fbx) {
            return Self::Fbx(server.load(path.to_owned()));
        }
        Self::Glb(
            server
                .load_builder()
                .with_settings(move |settings: &mut GltfLoaderSettings| {
                    settings.load_cameras = false;
                    settings.load_lights = lights;
                    settings.load_animations = false;
                })
                .load(path.to_owned()),
        )
    }

    /// Wait for the model and all of its dependencies without blocking the frame.
    pub(super) fn world(
        &self,
        server: &AssetServer,
        gltfs: &Assets<Gltf>,
    ) -> Result<Option<Handle<WorldAsset>>, String> {
        match self {
            Self::Glb(handle) => {
                if let Some(error) = super::load_error(server, handle) {
                    return Err(error);
                }
                if !server.is_loaded_with_dependencies(handle.id()) {
                    return Ok(None);
                }
                let Some(gltf) = gltfs.get(handle) else {
                    return Ok(None);
                };
                gltf.default_scene
                    .as_ref()
                    .or_else(|| gltf.scenes.first())
                    .cloned()
                    .map(Some)
                    .ok_or_else(|| "В GLB нет доступной геометрии".into())
            }
            Self::Fbx(handle) => {
                if let Some(error) = super::load_error(server, handle) {
                    return Err(error);
                }
                Ok(server
                    .is_loaded_with_dependencies(handle.id())
                    .then(|| handle.clone()))
            }
        }
    }
}
