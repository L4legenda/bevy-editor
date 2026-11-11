//! Version 0.1 level documents. Models are references to shared GLB assets.
use bevy::{math::Affine3A, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
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
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, NodeEdit>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub cameras: BTreeMap<K, CameraPlacement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<PlacedModel>,
}

impl<K: Ord> Default for LevelEdits<K> {
    fn default() -> Self {
        Self {
            nodes: BTreeMap::new(),
            cameras: BTreeMap::new(),
            models: Vec::new(),
        }
    }
}

impl<K: Ord> LevelEdits<K> {
    pub fn read(root: &Path, file: &str) -> io::Result<Self>
    where
        K: serde::de::DeserializeOwned,
    {
        if !relative_path(file, "json") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid document path",
            ));
        }
        let path = root.join(file);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let edits: Self = serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", path.display()),
            )
        })?;
        edits.validate(root)?;
        Ok(edits)
    }

    pub fn validate(&self, root: &Path) -> io::Result<()> {
        let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
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
        for model in &self.models {
            if !relative_path(&model.asset, "glb")
                || !root.join(&model.asset).is_file()
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
        use std::io::Write;

        self.validate(root)?;
        if !relative_path(file, "json") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid document path",
            ));
        }
        let path = root.join(file);
        fs::create_dir_all(path.parent().expect("room file has a parent"))?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let written = (|| {
            serde_json::to_writer_pretty(&mut file, self)?;
            file.write_all(b"\n")?;
            file.sync_all()
        })();
        drop(file);
        let result = written.and_then(|()| fs::rename(&temp, &path));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
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
