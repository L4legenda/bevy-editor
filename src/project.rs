//! Project settings. Asset and document paths are relative to `asset_root`.

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{CameraPlacement, LevelEdits, data::relative_path};

/// Optional hooks let the host apply its own lighting and materials.
pub type SceneSetup = fn(&mut EntityCommands, &AssetServer, &str);
pub type CameraSetup = fn(&mut EntityCommands, &str);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorProject {
    pub title: String,
    pub asset_root: PathBuf,
    pub scenes: Vec<SceneDefinition>,
    #[serde(default)]
    pub catalog: Vec<CatalogItem>,
    #[serde(skip)]
    pub scene_setup: Option<SceneSetup>,
    #[serde(skip)]
    pub camera_setup: Option<CameraSetup>,
}

impl EditorProject {
    /// Resolve the asset root against the project file, independent of the CWD.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let mut project: Self =
            serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
        project.asset_root = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&project.asset_root)
            .canonicalize()?;
        project.validate()?;
        Ok(project)
    }

    pub fn validate(&self) -> io::Result<()> {
        let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
        if self.scenes.is_empty() || !self.asset_root.is_dir() {
            return Err(invalid(
                "A project needs an asset directory and at least one scene",
            ));
        }
        let mut files = HashSet::new();
        for scene in &self.scenes {
            if !relative_path(&scene.asset, "glb")
                || !self.asset_root.join(&scene.asset).is_file()
                || !relative_path(&scene.file, "json")
                || !files.insert(&scene.file)
                || scene.cameras.is_empty()
            {
                return Err(invalid("Invalid scene asset, document path or camera list"));
            }
            let mut keys = HashSet::new();
            for camera in &scene.cameras {
                if camera.id.is_empty()
                    || !keys.insert(&camera.id)
                    || !camera.placement.pose.valid()
                    || !camera.projection.valid_lens(camera.placement.lens)
                {
                    return Err(invalid("Camera IDs must be unique and placements valid"));
                }
            }
        }
        for item in &self.catalog {
            let direction = Vec3::from(item.preview_direction);
            if !relative_path(&item.asset, "glb")
                || !self.asset_root.join(&item.asset).is_file()
                || !direction.is_finite()
                || direction.length_squared() < 1e-6
                || direction.normalize().dot(Vec3::Y).abs() > 0.999
            {
                return Err(invalid("Invalid catalog asset or preview direction"));
            }
        }
        Ok(())
    }

    pub(crate) fn read_scene(&self, index: usize) -> io::Result<LevelEdits> {
        let scene = &self.scenes[index];
        let edits = LevelEdits::read(&self.asset_root, &scene.file)?;
        scene.validate_edits(&edits)?;
        Ok(edits)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDefinition {
    pub label: String,
    pub asset: String,
    pub file: String,
    /// Direct children of these named glTF groups become editable placements.
    pub groups: Vec<String>,
    #[serde(default)]
    pub excluded_nodes: Vec<String>,
    #[serde(default)]
    pub node_policies: BTreeMap<String, NodePolicy>,
    pub cameras: Vec<CameraDefinition>,
    #[serde(default)]
    pub initial_selection: Option<String>,
    #[serde(default)]
    pub play: Option<PlayCommand>,
}

impl SceneDefinition {
    pub(crate) fn validate_edits(&self, edits: &LevelEdits) -> io::Result<()> {
        for (key, placement) in &edits.cameras {
            if self
                .cameras
                .iter()
                .find(|camera| camera.id == *key)
                .is_none_or(|camera| !camera.projection.valid_lens(placement.lens))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unknown or invalid camera {key}"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraDefinition {
    /// Stable ID used as the JSON camera key.
    pub id: String,
    pub label: String,
    pub placement: CameraPlacement,
    #[serde(default)]
    pub projection: LensKind,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensKind {
    #[default]
    Perspective,
    Orthographic,
}

impl LensKind {
    pub fn valid_lens(self, value: f32) -> bool {
        match self {
            Self::Perspective => (5.0..=145.0).contains(&value),
            Self::Orthographic => (0.05..=20.0).contains(&value),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodePolicy {
    pub label: Option<String>,
    pub translation: [bool; 3],
    pub rotation: [bool; 3],
    pub scale: [bool; 3],
}

impl Default for NodePolicy {
    fn default() -> Self {
        Self {
            label: None,
            translation: [true; 3],
            rotation: [true; 3],
            scale: [true; 3],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogItem {
    pub label: String,
    pub asset: String,
    pub category: String,
    #[serde(default)]
    pub solid: bool,
    /// View direction from the model centre. Useful for wall-mounted meshes.
    #[serde(default = "preview_direction")]
    pub preview_direction: [f32; 3],
}

fn preview_direction() -> [f32; 3] {
    [1.55, 1.3, 2.3]
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayCommand {
    /// `None` launches the host executable with `args`.
    #[serde(default)]
    pub program: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
}

impl PlayCommand {
    pub(crate) fn spawn(&self) -> io::Result<std::process::Child> {
        let program = match &self.program {
            Some(program) => program.clone(),
            None => std::env::current_exe()?,
        };
        let mut command = Command::new(program);
        command.args(&self.args);
        if let Some(directory) = &self.working_directory {
            command.current_dir(directory);
        }
        command.spawn()
    }
}
