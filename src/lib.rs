//! Configurable level editing with a small runtime document API.
//!
//! Disable default features in the game to use only [`LevelEdits`] and asset
//! instancing. Enable `editor` in a development executable for the native UI.

pub mod asset_paths;
pub mod data;
pub mod lighting;
pub mod runtime;

#[cfg(feature = "lighting")]
mod hdri;
#[cfg(feature = "lighting")]
pub use hdri::HdriPlugin;

#[cfg(not(target_arch = "wasm32"))]
pub use asset_paths::AssetRedirectPlugin;

pub use data::{
    BoxBounds, CameraPlacement, LevelEdits, NodeEdit, PlacedModel, Pose, SceneSettings,
};
pub use lighting::{
    EnvironmentPreset, FogSettings, HdriAsset, HdriProvider, LightKind, LightPreset,
    LightingPreset, SceneLight, SceneLighting, ToneMap,
};

#[cfg(feature = "editor")]
mod app_icon;
#[cfg(feature = "editor")]
mod editor;
#[cfg(feature = "editor")]
pub mod project;
#[cfg(all(feature = "editor", not(target_arch = "wasm32")))]
pub mod project_manager;
#[cfg(feature = "editor")]
mod ui_theme;
#[cfg(feature = "editor")]
mod window_chrome;

#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub use editor::mcp;
#[cfg(feature = "editor")]
pub use editor::{EditorPlugin, create_app};
#[cfg(feature = "editor")]
pub use project::{
    CameraDefinition, CatalogItem, EditorProject, LensKind, NodePolicy, PlayCommand,
    SceneDefinition,
};
