//! Versioned native scene documents and legacy GLB edit compatibility.
use bevy::{math::Affine3A, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs, io,
    path::{Component as PathComponent, Path},
};

pub(crate) fn relative_path(path: &str, extension: &str) -> bool {
    let relative = Path::new(path);
    !path.is_empty()
        && !path.contains(['#', '\\', ':'])
        && relative
            .components()
            .all(|part| matches!(part, PathComponent::Normal(_)))
        && relative.extension().is_some_and(|value| value == extension)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl From<Transform> for Pose {
    fn from(transform: Transform) -> Self {
        Self {
            position: transform.translation.to_array(),
            rotation: transform.rotation.normalize().to_array(),
            scale: transform.scale.to_array(),
        }
    }
}

impl Pose {
    pub fn transform(self) -> Transform {
        Transform {
            translation: self.position.into(),
            rotation: Quat::from_array(self.rotation).normalize(),
            scale: self.scale.into(),
        }
    }

    pub(crate) fn valid(self) -> bool {
        let length = Quat::from_array(self.rotation).length_squared();
        Vec3::from(self.position)
            .abs()
            .cmple(Vec3::splat(100_000.0))
            .all()
            && (0.99..=1.01).contains(&length)
            && self
                .scale
                .iter()
                .all(|scale| (0.0001..=1000.0).contains(&scale.abs()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeEdit {
    pub original: Pose,
    pub pose: Pose,
}

impl NodeEdit {
    pub fn delta(self) -> Affine3A {
        self.pose.transform().compute_affine()
            * self.original.transform().compute_affine().inverse()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraPlacement {
    pub pose: Pose,
    /// Degrees for perspective cameras, vertical coverage in metres for orthographic cameras.
    pub lens: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraDefinition {
    /// Stable ID used as the JSON camera key.
    pub id: String,
    pub label: String,
    pub placement: CameraPlacement,
    #[serde(default)]
    pub projection: LensKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSettings {
    pub label: String,
    pub cameras: Vec<CameraDefinition>,
}

pub(crate) fn document_path(path: &str) -> bool {
    relative_path(path, "scene") || relative_path(path, "json") || relative_path(path, "sceneb")
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxBounds {
    pub center: [f32; 3],
    pub half_size: [f32; 3],
}

impl BoxBounds {
    pub(crate) fn valid(self) -> bool {
        Vec3::from(self.center).is_finite()
            && self
                .half_size
                .iter()
                .all(|half| (0.001..=1000.0).contains(half))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacedModel {
    pub id: u32,
    pub asset: String,
    pub pose: Pose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoxBounds>,
}

#[derive(Clone, Debug, PartialEq, Resource, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[serde(bound(deserialize = "K: Ord + Deserialize<'de>"))]
pub struct LevelEdits<K: Ord = String> {
    /// Authoring graph and dependency snapshots. Shared by document history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<std::sync::Arc<crate::hierarchy::SceneHierarchy>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneSettings>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, NodeEdit>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub cameras: BTreeMap<K, CameraPlacement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<PlacedModel>,
    /// Per-instance labels; shared model assets keep their original names.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub model_names: BTreeMap<u32, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub node_names: BTreeMap<String, String>,
    /// Removed source placements retain stable GLB names for undo and reloading.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub deleted_nodes: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<crate::lighting::SceneLight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<crate::lighting::SceneLighting>,
}

impl<K: Ord> Default for LevelEdits<K> {
    fn default() -> Self {
        Self {
            hierarchy: None,
            scene: None,
            nodes: BTreeMap::new(),
            cameras: BTreeMap::new(),
            models: Vec::new(),
            model_names: BTreeMap::new(),
            node_names: BTreeMap::new(),
            deleted_nodes: BTreeSet::new(),
            lights: Vec::new(),
            lighting: None,
        }
    }
}

impl<K: Ord> LevelEdits<K> {
    pub fn read(root: &Path, file: &str) -> io::Result<Self>
    where
        K: serde::de::DeserializeOwned,
    {
        if !document_path(file) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid document path",
            ));
        }
        let resolved = match crate::asset_paths::resolve_asset_path(root, file) {
            Ok(path) => path,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && relative_path(file, "json") =>
            {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let path = root.join(resolved);
        let bytes = match read_document_bytes(&path, relative_path(file, "sceneb")) {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && relative_path(file, "json") =>
            {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let decoded = if relative_path(file, "sceneb") {
            crate::scene_io::decode_packed(&bytes)
        } else if relative_path(file, "scene") && crate::scene_io::modern(&bytes)? {
            crate::scene_io::decode(&bytes, root)
        } else {
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .map_err(io::Error::other)
        };
        let edits: Self = decoded.map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", path.display()),
            )
        })?;
        if relative_path(file, "scene") && edits.scene.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Scene metadata is missing",
            ));
        }
        edits.validate(root)?;
        Ok(edits)
    }

    pub fn validate(&self, root: &Path) -> io::Result<()> {
        let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
        let model_ids: HashSet<_> = self.models.iter().map(|model| model.id).collect();
        if self
            .model_names
            .values()
            .chain(self.node_names.values())
            .any(|name| {
                name.trim().is_empty()
                    || name.chars().count() > 128
                    || name.chars().any(char::is_control)
            })
            || self
                .node_names
                .keys()
                .chain(self.deleted_nodes.iter())
                .any(String::is_empty)
            || self.model_names.keys().any(|id| !model_ids.contains(id))
        {
            return Err(invalid("Invalid scene object name".into()));
        }
        if let Some(scene) = &self.scene {
            if !self.nodes.is_empty() {
                return Err(invalid(
                    "Standalone scenes must store model instances instead of source-node edits"
                        .into(),
                ));
            }
            let mut ids = HashSet::new();
            if scene.label.trim().is_empty() || scene.cameras.is_empty() {
                return Err(invalid("A scene needs a name and a camera".into()));
            }
            for camera in &scene.cameras {
                if camera.id.is_empty()
                    || !ids.insert(&camera.id)
                    || !camera.placement.pose.valid()
                    || !camera.projection.valid_lens(camera.placement.lens)
                {
                    return Err(invalid("Invalid scene camera".into()));
                }
            }
        }
        let redirects = crate::asset_paths::Redirects::read(root)?;
        if let Some(settings) = &self.lighting {
            settings.validate()?;
        }
        let mut light_ids = HashSet::new();
        for light in &self.lights {
            light.validate()?;
            if !light_ids.insert(light.id) {
                return Err(invalid(format!("Duplicate light ID #{}", light.id)));
            }
        }
        for (name, edit) in &self.nodes {
            if name.is_empty() || !edit.original.valid() || !edit.pose.valid() {
                return Err(invalid(format!("Invalid transform for {name}")));
            }
        }
        for camera in self.cameras.values() {
            if !camera.pose.valid() || !(0.0001..=180.0).contains(&camera.lens) {
                return Err(invalid("Invalid camera placement".to_owned()));
            }
        }
        let mut ids = HashSet::new();
        let mut asset_exists = BTreeMap::new();
        for model in &self.models {
            if !relative_path(&model.asset, "glb")
                || !*asset_exists.entry(&model.asset).or_insert_with(|| {
                    redirects
                        .resolve(&model.asset)
                        .is_ok_and(|path| root.join(path).is_file())
                })
                || !ids.insert(model.id)
                || !model.pose.valid()
                || model.bounds.is_some_and(|bounds| !bounds.valid())
            {
                return Err(invalid(format!(
                    "Invalid model #{}: {}",
                    model.id, model.asset
                )));
            }
        }
        Ok(())
    }

    pub fn write(&self, root: &Path, file: &str) -> io::Result<()>
    where
        K: Serialize,
    {
        self.validate(root)?;
        if relative_path(file, "scene") && self.scene.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Scene metadata is missing",
            ));
        }
        if !relative_path(file, "scene") && !relative_path(file, "json") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid document path",
            ));
        }
        fs::create_dir_all(root)?;
        let path = root.join(crate::asset_paths::resolve_asset_path(root, file)?);
        let previous = match read_document_bytes(&path, false) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let bytes = self.encode_document(file, previous.as_deref())?;
        let expected = previous.as_deref().map_or(
            bevy_scene_format::Revision::Missing,
            bevy_scene_format::Revision::of,
        );
        bevy_scene_format::atomic_write(&path, &bytes, expected).map(|_| ())
    }

    pub(crate) fn encode_document(&self, file: &str, previous: Option<&[u8]>) -> io::Result<Vec<u8>>
    where
        K: Serialize,
    {
        if relative_path(file, "scene") {
            crate::scene_io::encode(self, previous)
        } else {
            let mut bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
            bytes.push(b'\n');
            Ok(bytes)
        }
    }

    pub fn anchor(&self, node: &str, point: Vec3) -> Vec3 {
        self.nodes
            .get(node)
            .map_or(point, |edit| edit.delta().transform_point3(point))
    }

    pub fn seat_yaw(&self, node: &str, yaw: f32) -> f32 {
        self.nodes.get(node).map_or(yaw, |edit| {
            let rotation =
                edit.pose.transform().rotation * edit.original.transform().rotation.inverse();
            let facing = rotation * (Quat::from_rotation_y(yaw) * Vec3::Z);
            facing.x.atan2(facing.z)
        })
    }
}

fn read_document_bytes(path: &Path, packed: bool) -> io::Result<Vec<u8>> {
    let limit = if packed {
        512 * 1024 * 1024
    } else {
        bevy_scene_format::MAX_SOURCE_BYTES
    };
    bevy_scene_format::read_limited(path, limit)
}
