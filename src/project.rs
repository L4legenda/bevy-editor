//! Project settings. Asset and document paths are relative to `asset_root`.

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{LevelEdits, data::relative_path};

/// Optional hooks let the host apply its own lighting and materials.
pub type SceneSetup = fn(&mut EntityCommands, &AssetServer, &str);
pub type CameraSetup = fn(&mut EntityCommands, &str);

pub const PROJECT_FILE_NAME: &str = "project.bevyeditor";

/// Accept a project folder, the standard manifest, or a legacy JSON manifest.
pub fn resolve_project_file(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();
    let file = if path.is_dir() {
        let standard = path.join(PROJECT_FILE_NAME);
        if standard.is_file() {
            standard
        } else {
            let legacy = path.join("project.json");
            if !legacy.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "В папке нет project.bevyeditor или прежнего project.json",
                ));
            }
            legacy
        }
    } else {
        path.to_path_buf()
    };
    if !file.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Файл проекта не найден",
        ));
    }
    file.canonicalize()
}

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
        let path = resolve_project_file(path)?;
        let mut project: Self =
            serde_json::from_slice(&std::fs::read(&path)?).map_err(io::Error::other)?;
        project.asset_root = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&project.asset_root)
            .canonicalize()?;
        crate::asset_paths::apply_project_paths(&mut project)?;
        for scene in &mut project.scenes {
            if scene.is_native() {
                let content = LevelEdits::<String>::read(&project.asset_root, &scene.file)?;
                let settings = content.scene.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Missing scene metadata")
                })?;
                scene.label = settings.label;
                scene.cameras = settings.cameras;
            }
        }
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
            if (!scene.is_native()
                && (!relative_path(&scene.asset, "glb")
                    || !self.asset_root.join(&scene.asset).is_file()))
                || !crate::data::document_path(&scene.file)
                || (relative_path(&scene.file, "scene") && !scene.asset.is_empty())
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

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDefinition {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub asset: String,
    pub file: String,
    /// Direct children of these named glTF groups become editable placements.
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub excluded_nodes: Vec<String>,
    #[serde(default)]
    pub node_policies: BTreeMap<String, NodePolicy>,
    #[serde(default)]
    pub cameras: Vec<CameraDefinition>,
    #[serde(default)]
    pub initial_selection: Option<String>,
    #[serde(default)]
    pub play: Option<PlayCommand>,
}

impl SceneDefinition {
    pub(crate) fn is_native(&self) -> bool {
        self.asset.is_empty() && relative_path(&self.file, "scene")
    }
    pub(crate) fn validate_edits(&self, edits: &LevelEdits) -> io::Result<()> {
        let cameras = edits
            .scene
            .as_ref()
            .map_or(&self.cameras, |settings| &settings.cameras);
        for (key, placement) in &edits.cameras {
            if cameras
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

impl Serialize for SceneDefinition {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("file", &self.file)?;
        if !self.is_native() {
            map.serialize_entry("label", &self.label)?;
            map.serialize_entry("asset", &self.asset)?;
            map.serialize_entry("groups", &self.groups)?;
            map.serialize_entry("excluded_nodes", &self.excluded_nodes)?;
            map.serialize_entry("node_policies", &self.node_policies)?;
            map.serialize_entry("cameras", &self.cameras)?;
            map.serialize_entry("initial_selection", &self.initial_selection)?;
        }
        if self.play.is_some() {
            map.serialize_entry("play", &self.play)?;
        }
        map.end()
    }
}

pub use crate::data::{CameraDefinition, LensKind};

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
