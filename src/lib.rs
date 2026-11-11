//! Configurable level editing with a small runtime document API.
//!
//! Disable default features in the game to use only [`LevelEdits`] and asset
//! instancing. Enable `editor` in a development executable for the native UI.

pub mod data;
pub mod runtime;

pub use data::{BoxBounds, CameraPlacement, LevelEdits, NodeEdit, PlacedModel, Pose};

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
