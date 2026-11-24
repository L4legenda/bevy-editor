//! Instancing helpers usable without the editor UI feature.
use crate::PlacedModel;
use bevy::prelude::*;

#[cfg(feature = "game")]
pub(crate) mod fbx;
#[cfg(feature = "game")]
pub use fbx::FbxPlugin;

#[cfg(all(feature = "game", not(target_arch = "wasm32")))]
mod game;
#[cfg(all(feature = "game", not(target_arch = "wasm32")))]
pub use game::{
    GameCamera, GameLaunch, GameLevel, GameLevelPlugin, GameLevelStatus, GameModel, create_game_app,
};

#[cfg(feature = "lighting")]
pub use crate::HdriPlugin;
#[cfg(feature = "lighting")]
pub use crate::lighting::{apply_camera_lighting, apply_imported_light, apply_light, spawn_light};

/// Instantiate authored lights without changing host lighting or camera settings.
#[cfg(feature = "lighting")]
pub fn spawn_lights(commands: &mut Commands, lights: &[crate::SceneLight]) -> Vec<Entity> {
    lights
        .iter()
        .map(|light| spawn_light(commands, light))
        .collect()
}

/// Direct children of named groups, excluding host-owned scripted placements.
pub fn imported_nodes<'a>(
    root: Entity,
    groups: &'a [String],
    excluded: &'a [String],
    children: &'a Query<&Children>,
    names: &'a Query<&Name>,
) -> impl Iterator<Item = (Entity, &'a str)> + 'a {
    children
        .iter_descendants(root)
        .filter_map(move |entity| {
            groups
                .iter()
                .any(|group| group == names.get(entity).map(Name::as_str).unwrap_or(""))
                .then_some(entity)
        })
        .flat_map(move |group| {
            children
                .get(group)
                .into_iter()
                .flat_map(|nodes| nodes.iter())
                .filter_map(move |entity| {
                    let name = names.get(entity).ok()?.as_str();
                    (!excluded.iter().any(|excluded| excluded == name)).then_some((entity, name))
                })
        })
}

pub fn spawn_model(commands: &mut Commands, server: &AssetServer, model: &PlacedModel) -> Entity {
    commands
        .spawn((
            Name::new(
                model
                    .asset
                    .rsplit('/')
                    .next()
                    .unwrap_or(&model.asset)
                    .to_owned(),
            ),
            WorldAssetRoot(load_model(server, model)),
            model.pose.transform(),
        ))
        .id()
}

pub fn load_model(server: &AssetServer, model: &PlacedModel) -> Handle<WorldAsset> {
    match crate::data::ModelFormat::from_path(&model.asset) {
        Some(crate::data::ModelFormat::Fbx) => {
            match model.selector.as_deref().filter(|selector| *selector != "Scene0") {
                Some(selector) => server.load(format!("{}#{selector}", model.asset)),
                None => server.load(model.asset.clone()),
            }
        }
        _ => server.load(format!(
            "{}#{}",
            model.asset,
            model.selector.as_deref().unwrap_or("Scene0")
        )),
    }
}
