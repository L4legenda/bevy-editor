//! Configurable level editing with a small runtime document API.
//!
//! Disable default features in the game to use only [`LevelEdits`] and asset
//! instancing. Enable `editor` in a development executable for the native UI.

pub mod asset_paths;
pub mod data;
pub mod lighting;
pub mod runtime;

#[cfg(not(target_arch = "wasm32"))]
pub use asset_paths::AssetRedirectPlugin;

pub use data::{BoxBounds, CameraPlacement, LevelEdits, NodeEdit, PlacedModel, Pose};
pub use lighting::{
    FogSettings, LightKind, LightPreset, LightingPreset, SceneLight, SceneLighting, ToneMap,
};

#[cfg(feature = "editor")]
mod editor;
#[cfg(feature = "editor")]
pub mod project;

#[cfg(feature = "editor")]
pub use editor::{EditorPlugin, create_app};
#[cfg(feature = "editor")]
pub use project::{
    CameraDefinition, CatalogItem, EditorProject, LensKind, NodePolicy, PlayCommand,
    SceneDefinition,
};
