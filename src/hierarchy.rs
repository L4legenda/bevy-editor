//! Authoring graph shared by the editor, history, previews and game snapshots.
use crate::{CameraPlacement, LevelEdits, Pose, SceneSettings};
use bevy::{math::Affine3A, prelude::*};
use bevy_scene_format::{Entity as SceneEntity, EntityPatch, Instance, Resource, Scene};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    path::Path,
    sync::Arc,
};

pub(crate) const TREE_ORDER_SETTING: &str = "editor.scene_tree_order";

#[cfg(feature = "editor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeDrop<T> {
    Root,
    Inside(T),
    Before(T),
    After(T),
}

#[cfg(feature = "editor")]
impl<T> TreeDrop<T> {
    pub(crate) fn try_map<U>(self, map: impl FnOnce(T) -> Option<U>) -> Option<TreeDrop<U>> {
        Some(match self {
            Self::Root => TreeDrop::Root,
            Self::Inside(target) => TreeDrop::Inside(map(target)?),
            Self::Before(target) => TreeDrop::Before(map(target)?),
            Self::After(target) => TreeDrop::After(map(target)?),
        })
    }
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneNode {
    pub key: u32,
    pub entity: SceneEntity,
    /// Exact affine matrix: TRS decomposition would lose shear below scaled parents.
    pub world: [f32; 12],
    pub camera: Option<String>,
    pub instance: Option<String>,
}
impl SceneNode {
    pub fn affine(&self) -> Affine3A {
        Affine3A::from_cols_array(&self.world)
    }
    pub fn local(&self) -> Pose {
        crate::scene_io::pose(self.entity.transform)
    }
    pub fn pose(&self) -> Pose {
        Transform::from_matrix(Mat4::from(self.affine())).into()
    }
    pub fn inherited(&self) -> bool {
        self.entity.id.contains('/')
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneHierarchy {
    source: Scene,
    /// Immutable dependency snapshots make Undo independent of subsequent disk edits.
    dependencies: BTreeMap<String, Arc<Scene>>,
    pub nodes: Vec<SceneNode>,
    resources: BTreeMap<String, Resource>,
    by_key: HashMap<u32, usize>,
    by_id: HashMap<String, usize>,
    keys: BTreeMap<String, u32>,
}
// Snapshots contain authoring data only; derived matrices and indexes are rebuilt
// and validated instead of trusting or duplicating a serialized runtime graph.
impl Serialize for SceneHierarchy {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Snapshot<'a> {
            source: &'a Scene,
            dependencies: &'a BTreeMap<String, Arc<Scene>>,
            keys: &'a BTreeMap<String, u32>,
        }
        Snapshot {
            source: &self.source,
            dependencies: &self.dependencies,
            keys: &self.keys,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for SceneHierarchy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Snapshot {
            source: Scene,
            dependencies: BTreeMap<String, Arc<Scene>>,
            #[serde(default)]
            keys: BTreeMap<String, u32>,
        }
        let data = Snapshot::deserialize(deserializer)?;
        if data.keys.len() > bevy_scene_format::MAX_ENTITIES
            || data.keys.values().collect::<HashSet<_>>().len() != data.keys.len()
        {
            return Err(serde::de::Error::custom(
                "Invalid scene snapshot object keys",
            ));
        }
        let mut graph = Self {
            source: data.source,
            dependencies: data.dependencies,
            nodes: vec![],
            resources: BTreeMap::new(),
            by_key: HashMap::new(),
            by_id: HashMap::new(),
            keys: data.keys,
        };
        graph.resolve(None).map_err(serde::de::Error::custom)?;
        Ok(graph)
    }
}
impl SceneHierarchy {
    pub fn source(&self) -> &Scene {
        &self.source
    }
    #[cfg(feature = "editor")]
    pub(crate) fn remap_paths(&mut self, from: &Path, to: &Path) {
        let remap = |value: &mut String| {
            if let Ok(suffix) = Path::new(value).strip_prefix(from) {
                *value = if suffix.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(suffix)
                }
                .to_string_lossy()
                .replace('\\', "/");
            }
        };
        crate::scene_io::remap_scene_paths(&mut self.source, from, to);
        self.dependencies = std::mem::take(&mut self.dependencies)
            .into_iter()
            .map(|(mut path, mut source)| {
                remap(&mut path);
                crate::scene_io::remap_scene_paths(Arc::make_mut(&mut source), from, to);
                (path, source)
            })
            .collect();
        for resource in self.resources.values_mut() {
            if let Some(path) = &mut resource.path {
                remap(path);
            }
        }
        for node in &mut self.nodes {
            if let Some(path) = &mut node.instance {
                remap(path);
            }
        }
    }
    pub fn node(&self, key: u32) -> Option<&SceneNode> {
        self.by_key
            .get(&key)
            .and_then(|&index| self.nodes.get(index))
    }
    pub fn by_id(&self, id: &str) -> Option<&SceneNode> {
        self.by_id.get(id).and_then(|&index| self.nodes.get(index))
    }
    pub fn parent(&self, key: u32) -> Option<&SceneNode> {
        self.by_id(self.node(key)?.entity.parent.as_deref()?)
    }
    pub fn source_path(&self, key: u32) -> Option<&str> {
        let node = self.node(key)?;
        node.instance.as_deref().or_else(|| {
            node.entity
                .id
                .rsplit_once('/')
                .and_then(|(mount, _)| self.by_id(mount)?.instance.as_deref())
        })
    }
    pub fn overridden(&self, key: u32) -> bool {
        let Some(node) = self.node(key) else {
            return false;
        };
        if let Some((mount, child)) = node.entity.id.split_once('/') {
            return self
                .source
                .entities
                .iter()
                .find(|e| e.id == mount)
                .and_then(|e| e.instance.as_ref())
                .is_some_and(|i| i.overrides.contains_key(child));
        }
        self.source
            .entities
            .iter()
            .find(|e| e.id == node.entity.id)
            .and_then(|e| e.instance.as_ref())
            .is_some_and(|i| !i.overrides.is_empty())
    }
    pub fn load(source: Scene, root: &Path) -> io::Result<Self> {
        let mut result = Self {
            source,
            dependencies: BTreeMap::new(),
            nodes: vec![],
            resources: BTreeMap::new(),
            by_key: HashMap::new(),
            by_id: HashMap::new(),
            keys: BTreeMap::new(),
        };
        result.resolve(Some(root))?;
        Ok(result)
    }
    pub fn refresh(&mut self, root: &Path) -> io::Result<()> {
        let mut next = self.clone();
        next.dependencies.clear();
        next.resolve(Some(root))?;
        *self = next;
        Ok(())
    }
    pub(crate) fn from_resolved(mut scene: Scene) -> io::Result<Self> {
        for entity in &mut scene.entities {
            if let Some((prefix, _)) = entity.id.rsplit_once('/')
                && let Some(camera) = entity.components.get_mut("camera")
                && let Some(id) = camera.get("id").and_then(Value::as_str)
            {
                camera["id"] = format!("{prefix}/{id}").into();
            }
        }
        // Cooked IDs contain instance paths. Convert them to legal authoring IDs.
        let ids: BTreeMap<_, _> = scene
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), format!("node_{i}")))
            .collect();
        let resources: BTreeMap<_, _> = scene
            .resources
            .keys()
            .enumerate()
            .map(|(i, k)| (k.clone(), format!("resource_{i}")))
            .collect();
        remap_scene(&mut scene, &ids, &resources);
        Self::load(scene, Path::new("."))
    }
    fn resolve(&mut self, root: Option<&Path>) -> io::Result<()> {
        let dependencies = &mut self.dependencies;
        let resolved = bevy_scene_format::resolve(&self.source, &mut |resource: &Resource| {
            let path = resource.path.as_deref().ok_or_else(|| {
                bevy_scene_format::Error(
                    "Scene instances require a project-relative scene path".into(),
                )
            })?;
            if let Some(scene) = dependencies.get(path) {
                return Ok((**scene).clone());
            }
            let load = || -> io::Result<Scene> {
                let root = root.ok_or_else(|| {
                    invalid(format!("Dependency is missing from snapshot: {path}"))
                })?;
                if !crate::data::document_path(path) || path.ends_with(".sceneb") {
                    return Err(invalid(
                        "Instances require editable .scene or .json documents",
                    ));
                }
                let resolved = crate::asset_paths::resolve_asset_path(root, path)?;
                let bytes = bevy_scene_format::read_limited(
                    &root.join(resolved),
                    bevy_scene_format::MAX_SOURCE_BYTES,
                )
                .map_err(invalid)?;
                if crate::scene_io::modern(&bytes)? {
                    Scene::from_slice(&bytes).map_err(invalid)
                } else {
                    let edits: LevelEdits = serde_json::from_slice(&bytes).map_err(invalid)?;
                    Scene::from_slice(&crate::scene_io::encode(&edits, None)?).map_err(invalid)
                }
            };
            let scene = load().map_err(|e| bevy_scene_format::Error(format!("{path}: {e}")))?;
            dependencies.insert(path.to_owned(), Arc::new(scene.clone()));
            Ok(scene)
        })
        .map_err(invalid)?;
        let old_keys = &self.keys;
        let mut mounts = HashMap::new();
        let mut scopes = vec![(&self.source, String::new())];
        while let Some((scene, prefix)) = scopes.pop() {
            for entity in &scene.entities {
                if let Some(instance) = &entity.instance
                    && let Some(path) = scene
                        .resources
                        .get(&instance.resource)
                        .and_then(|r| r.path.as_ref())
                {
                    let id = format!("{prefix}{}", entity.id);
                    mounts.insert(id.clone(), path.clone());
                    if let Some(dependency) = self.dependencies.get(path) {
                        scopes.push((dependency, format!("{id}/")));
                    }
                }
            }
        }
        let mut used: HashSet<u32> = old_keys.values().copied().collect();
        let entities: HashMap<_, _> = resolved
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect();
        let mut pending: Vec<_> = resolved.entities.iter().collect();
        pending.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        let mut worlds: HashMap<String, Affine3A> = HashMap::new();
        let mut nodes = Vec::with_capacity(pending.len());
        // Iterative ancestor walks support deep scenes without recursive stack growth.
        for entity in pending {
            let mut ancestors = Vec::new();
            let mut cursor = Some(entity);
            while let Some(next) = cursor {
                if worlds.contains_key(&next.id) {
                    break;
                }
                ancestors.push(next);
                cursor = next
                    .parent
                    .as_deref()
                    .and_then(|id| entities.get(id).copied());
            }
            for next in ancestors.into_iter().rev() {
                let parent = next
                    .parent
                    .as_ref()
                    .and_then(|id| worlds.get(id))
                    .copied()
                    .unwrap_or(Affine3A::IDENTITY);
                worlds.insert(
                    next.id.clone(),
                    parent
                        * crate::scene_io::pose(next.transform)
                            .transform()
                            .compute_affine(),
                );
                if !worlds[&next.id].is_finite() {
                    return Err(invalid("Hierarchy transform overflow"));
                }
            }
            let key = if let Some(key) = old_keys.get(&entity.id) {
                *key
            } else {
                let mut key = u32::from_le_bytes(
                    bevy_scene_format::content_digest(entity.id.as_bytes())[..4]
                        .try_into()
                        .unwrap(),
                );
                while !used.insert(key) {
                    key = key.wrapping_add(1);
                }
                key
            };
            let camera = entity
                .components
                .get("camera")
                .map(|value| {
                    let camera: crate::scene_io::Camera =
                        serde_json::from_value(value.clone()).map_err(invalid)?;
                    if camera.id.is_empty()
                        || !camera.projection.valid_lens(camera.lens)
                        || camera.reset.is_some_and(|reset| {
                            !reset.pose.valid() || !camera.projection.valid_lens(reset.lens)
                        })
                    {
                        return Err(invalid("Invalid scene camera"));
                    }
                    Ok(camera.id)
                })
                .transpose()?
                .map(|id| {
                    if entity.id.contains('/') {
                        format!("{}/{}", entity.id.rsplit_once('/').unwrap().0, id)
                    } else {
                        id
                    }
                });
            let instance = mounts.get(&entity.id).cloned();
            nodes.push(SceneNode {
                key,
                entity: entity.clone(),
                world: worlds[&entity.id].to_cols_array(),
                camera,
                instance,
            });
        }
        self.resources = resolved.resources;
        self.nodes = nodes;
        self.by_key = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.key, i))
            .collect();
        self.by_id = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.entity.id.clone(), i))
            .collect();
        self.keys = self
            .nodes
            .iter()
            .map(|n| (n.entity.id.clone(), n.key))
            .collect();
        Ok(())
    }
    pub fn project<K: Ord + serde::de::DeserializeOwned>(self) -> io::Result<LevelEdits<K>> {
        Self::project_shared(Arc::new(self))
    }
    pub fn project_shared<K: Ord + serde::de::DeserializeOwned>(
        graph: Arc<Self>,
    ) -> io::Result<LevelEdits<K>> {
        let mut edits = graph.projected()?;
        edits.hierarchy = Some(graph);
        Ok(edits)
    }
    fn projected<K: Ord + serde::de::DeserializeOwned>(&self) -> io::Result<LevelEdits<K>> {
        let mut scene = Scene {
            format: self.source.format.clone(),
            version: self.source.version,
            id: self.source.id.clone(),
            name: self.source.name.clone(),
            settings: self.source.settings.clone(),
            resources: self.resources.clone(),
            entities: Vec::with_capacity(self.nodes.len()),
        };
        for node in &self.nodes {
            let mut e = node.entity.clone();
            e.id = format!("model:{}", node.key);
            // The flat bridge recognizes one numeric ID per object kind.
            if e.model.is_none() && e.components.contains_key("light") {
                e.id = format!("light:{}", node.key);
            }
            e.parent = None;
            e.instance = None;
            e.transform = crate::scene_io::transform(node.pose());
            if let Some(camera) = e.components.get_mut("camera") {
                camera["id"] = node.camera.as_ref().unwrap().clone().into();
                if let Some(reset) = camera.get_mut("reset") {
                    let mut placement: CameraPlacement =
                        serde_json::from_value(reset.clone()).map_err(invalid)?;
                    let parent = self
                        .parent(node.key)
                        .map_or(Affine3A::IDENTITY, |n| n.affine());
                    placement.pose = Transform::from_matrix(Mat4::from(
                        parent * placement.pose.transform().compute_affine(),
                    ))
                    .into();
                    *reset = serde_json::to_value(placement).map_err(invalid)?;
                }
            }
            // Split combined components only in the compatibility projection.
            let light = if e.model.is_some() {
                e.components.remove("light")
            } else {
                None
            };
            let camera = if e.model.is_some() || e.components.contains_key("light") {
                e.components.remove("camera")
            } else {
                None
            };
            if let Some(light) = light {
                let mut extra = e.clone();
                extra.id = format!("light:{}", node.key);
                extra.model = None;
                extra.components = BTreeMap::from([("light".into(), light)]);
                scene.entities.push(extra);
            }
            if let Some(camera) = camera {
                let mut extra = e.clone();
                extra.id = format!("camera:{}", node.key);
                extra.model = None;
                extra.components = BTreeMap::from([("camera".into(), camera)]);
                scene.entities.push(extra);
            }
            scene.entities.push(e);
        }
        let mut edits = crate::scene_io::decode_scene(scene)?;
        edits.models.sort_unstable_by_key(|model| model.id);
        edits.lights.sort_unstable_by_key(|light| light.id);
        if edits.scene.as_ref().unwrap().cameras.is_empty() {
            // Editor navigation needs a camera even for camera-less reusable scenes.
            // This preview camera is never written into the authoring graph.
            edits
                .scene
                .as_mut()
                .unwrap()
                .cameras
                .push(crate::data::CameraDefinition {
                    id: "__scene_preview".into(),
                    label: "Обзор сцены".into(),
                    projection: crate::data::LensKind::Perspective,
                    placement: CameraPlacement {
                        pose: Transform::from_xyz(6.0, 5.0, 8.0)
                            .looking_at(Vec3::ZERO, Vec3::Y)
                            .into(),
                        lens: 55.0,
                    },
                });
        }
        Ok(edits)
    }
    fn patch(&mut self, id: &str, patch: EntityPatch) -> io::Result<()> {
        if let Some(entity) = self.source.entities.iter_mut().find(|e| e.id == id) {
            if let Some(name) = patch.name {
                entity.name = Some(name);
            }
            if let Some(transform) = patch.transform {
                entity.transform = transform;
            }
            if patch.detach || patch.parent.is_some() {
                entity.parent = patch.parent;
            }
            if let Some(order) = patch.order {
                entity.order = order;
            }
            if patch.remove_model {
                entity.model = None;
            }
            if let Some(model) = patch.model {
                entity.model = Some(model);
            }
            for name in patch.remove_components {
                entity.components.remove(&name);
            }
            entity.components.extend(patch.components);
            return Ok(());
        }
        let (mount, child) = id
            .split_once('/')
            .ok_or_else(|| invalid("Object no longer exists"))?;
        let instance = self
            .source
            .entities
            .iter_mut()
            .find(|e| e.id == mount)
            .and_then(|e| e.instance.as_mut())
            .ok_or_else(|| invalid("Instance no longer exists"))?;
        let target = instance.overrides.entry(child.to_owned()).or_default();
        if patch.remove {
            *target = patch;
            return Ok(());
        }
        if patch.name.is_some() {
            target.name = patch.name;
        }
        if patch.transform.is_some() {
            target.transform = patch.transform;
        }
        if patch.detach || patch.parent.is_some() {
            target.parent = patch.parent;
            target.detach = patch.detach;
        }
        if patch.order.is_some() {
            target.order = patch.order;
        }
        if patch.model.is_some() || patch.remove_model {
            target.model = patch.model;
            target.remove_model = patch.remove_model;
        }
        for name in patch.remove_components {
            target.components.remove(&name);
            if !target.remove_components.contains(&name) {
                target.remove_components.push(name);
            }
        }
        for (name, value) in patch.components {
            target.remove_components.retain(|n| n != &name);
            target.components.insert(name, value);
        }
        Ok(())
    }
    pub fn set_local(&mut self, key: u32, mut pose: Pose) -> io::Result<()> {
        if !pose.valid() {
            return Err(invalid("Invalid local transform"));
        }
        let id = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?
            .entity
            .id
            .clone();
        let node = self.node(key).unwrap();
        let mut components = BTreeMap::new();
        if let Some(mut camera) = node.entity.components.get("camera").cloned() {
            if camera.get("reset").is_none() && !same_pose(node.local(), pose) {
                camera["reset"] = serde_json::to_value(CameraPlacement {
                    pose: node.local(),
                    lens: camera["lens"].as_f64().unwrap() as f32,
                })
                .map_err(invalid)?;
            }
            if let Some(original) = clear_camera_reset(&mut camera, pose)? {
                pose = original;
            }
            if node.entity.components.get("camera") != Some(&camera) {
                components.insert("camera".into(), camera);
            }
        }
        self.patch(
            &id,
            EntityPatch {
                transform: Some(crate::scene_io::transform(pose)),
                components: components.clone(),
                ..Default::default()
            },
        )?;
        self.nodes
            .iter_mut()
            .find(|n| n.key == key)
            .unwrap()
            .entity
            .transform = crate::scene_io::transform(pose);
        self.nodes[*self.by_key.get(&key).unwrap()]
            .entity
            .components
            .extend(components);
        // Recompute just matrices: transform drags never reload or expand dependencies.
        let mut worlds: HashMap<String, Affine3A> = HashMap::new();
        for node in &self.nodes {
            let mut ancestors = vec![];
            let mut cursor = Some(node);
            while let Some(next) = cursor {
                if worlds.contains_key(&next.entity.id) {
                    break;
                }
                ancestors.push(next);
                cursor = next.entity.parent.as_deref().and_then(|id| self.by_id(id));
            }
            for next in ancestors.into_iter().rev() {
                let parent = next
                    .entity
                    .parent
                    .as_ref()
                    .map_or(Affine3A::IDENTITY, |id| worlds[id]);
                let world = parent * next.local().transform().compute_affine();
                if !world.is_finite() {
                    return Err(invalid("Hierarchy transform overflow"));
                }
                worlds.insert(next.entity.id.clone(), world);
            }
        }
        for node in &mut self.nodes {
            node.world = worlds[&node.entity.id].to_cols_array();
        }
        Ok(())
    }
    pub fn set_world(&mut self, key: u32, pose: Pose) -> io::Result<()> {
        let node = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?;
        let parent = self.parent(key).map_or(Affine3A::IDENTITY, |n| n.affine());
        let current = node.pose();
        let local = if current
            .transform()
            .rotation
            .abs_diff_eq(pose.transform().rotation, 1e-5)
            && Vec3::from(current.scale).abs_diff_eq(pose.scale.into(), 1e-5)
        {
            Pose {
                position: parent
                    .inverse()
                    .transform_point3(pose.position.into())
                    .to_array(),
                ..node.local()
            }
        } else {
            affine_pose(parent.inverse() * pose.transform().compute_affine())?
        };
        self.set_local(key, local)
    }
    pub fn set_camera(&mut self, key: u32, placement: CameraPlacement) -> io::Result<()> {
        self.camera_placement(key, placement, false)
    }
    pub fn set_camera_local(&mut self, key: u32, placement: CameraPlacement) -> io::Result<()> {
        self.camera_placement(key, placement, true)
    }
    fn camera_placement(
        &mut self,
        key: u32,
        placement: CameraPlacement,
        local: bool,
    ) -> io::Result<()> {
        let original = self
            .node(key)
            .ok_or_else(|| invalid("Camera no longer exists"))?;
        let original_pose = original.local();
        let original_lens = original
            .entity
            .components
            .get("camera")
            .and_then(|c| c.get("lens"))
            .and_then(Value::as_f64)
            .ok_or_else(|| invalid("Invalid camera lens"))? as f32;
        if local {
            self.set_local(key, placement.pose)?;
        } else {
            self.set_world(key, placement.pose)?;
        }
        let node = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?;
        let id = node.entity.id.clone();
        let mut value = node
            .entity
            .components
            .get("camera")
            .ok_or_else(|| invalid("Camera no longer exists"))?
            .clone();
        if value.get("reset").is_none() && (placement.lens - original_lens).abs() > 1e-5 {
            value["reset"] = serde_json::to_value(CameraPlacement {
                pose: original_pose,
                lens: original_lens,
            })
            .map_err(invalid)?;
        }
        value["lens"] = placement.lens.into();
        if let Some(original) = clear_camera_reset(&mut value, node.local())? {
            self.set_local(key, original)?;
        }
        self.patch(
            &id,
            EntityPatch {
                components: BTreeMap::from([("camera".into(), value.clone())]),
                ..Default::default()
            },
        )?;
        self.nodes
            .iter_mut()
            .find(|n| n.key == key)
            .unwrap()
            .entity
            .components
            .insert("camera".into(), value);
        Ok(())
    }
    pub fn rename(&mut self, key: u32, name: &str) -> io::Result<()> {
        let id = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?
            .entity
            .id
            .clone();
        self.patch(
            &id,
            EntityPatch {
                name: Some(name.to_owned()),
                ..Default::default()
            },
        )?;
        self.resolve(None)
    }
    fn new_id(&self, prefix: &str) -> String {
        (1u32..)
            .map(|i| format!("{prefix}_{i}"))
            .find(|id| !self.source.entities.iter().any(|e| e.id == *id))
            .unwrap()
    }
    pub fn add_group(&mut self, parent: Option<u32>) -> io::Result<u32> {
        let id = self.new_id("group");
        let mut entity = SceneEntity::new(&id);
        entity.name = Some("Группа".into());
        entity.parent = parent
            .and_then(|k| self.node(k))
            .map(|n| n.entity.id.clone());
        self.source.entities.push(entity);
        self.resolve(None)?;
        Ok(self.by_id(&id).unwrap().key)
    }
    pub fn add_model(
        &mut self,
        model: crate::PlacedModel,
        name: Option<String>,
    ) -> io::Result<u32> {
        let id = self.new_id("model");
        let resource = unique_key(&self.source.resources, "model");
        self.source
            .resources
            .insert(resource.clone(), Resource::external("model", model.asset));
        let mut e = SceneEntity::new(&id);
        e.name = name;
        e.model = Some(resource);
        e.transform = crate::scene_io::transform(model.pose);
        if let Some(bounds) = model.bounds {
            e.components.insert(
                "bounds".into(),
                serde_json::to_value(bounds).map_err(invalid)?,
            );
        }
        self.source.entities.push(e);
        self.resolve(None)?;
        Ok(self.by_id(&id).unwrap().key)
    }
    pub fn add_light(&mut self, light: crate::SceneLight) -> io::Result<u32> {
        let id = self.new_id("light");
        let mut value = serde_json::to_value(&light).map_err(invalid)?;
        for key in ["id", "name", "pose"] {
            value.as_object_mut().unwrap().remove(key);
        }
        let mut e = SceneEntity::new(&id);
        e.name = Some(light.name);
        e.transform = crate::scene_io::transform(light.pose);
        e.components.insert("light".into(), value);
        self.source.entities.push(e);
        self.resolve(None)?;
        Ok(self.by_id(&id).unwrap().key)
    }
    pub fn insert_scene(
        &mut self,
        path: &str,
        name: &str,
        world: Pose,
        root: &Path,
    ) -> io::Result<u32> {
        let id = self.new_id("instance");
        let resource = unique_key(&self.source.resources, "scene");
        self.source
            .resources
            .insert(resource.clone(), Resource::external("scene", path));
        let mut entity = SceneEntity::new(&id);
        entity.name = Some(name.to_owned());
        entity.transform = crate::scene_io::transform(world);
        entity.instance = Some(Instance {
            resource,
            overrides: BTreeMap::new(),
        });
        self.source.entities.push(entity);
        self.resolve(Some(root))?;
        Ok(self.by_id(&id).unwrap().key)
    }
    pub fn reparent(&mut self, key: u32, parent: Option<u32>) -> io::Result<()> {
        let mut next = self.clone();
        if let Some(patch) = self.parent_patch(key, parent)? {
            next.patch(&self.node(key).unwrap().entity.id, patch)?;
            next.resolve(None)?;
            *self = next;
        }
        Ok(())
    }
    fn parent_patch(&self, key: u32, parent: Option<u32>) -> io::Result<Option<EntityPatch>> {
        let node = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?;
        let parent_node = parent
            .map(|k| {
                self.node(k)
                    .ok_or_else(|| invalid("Родитель больше не существует"))
            })
            .transpose()?;
        let parent_id = parent_node.map(|n| n.entity.id.clone());
        if node.entity.parent == parent_id {
            return Ok(None);
        }
        let mut cursor = parent_node;
        while let Some(ancestor) = cursor {
            if ancestor.key == key {
                return Err(invalid("Нельзя поместить объект внутрь самого себя"));
            }
            cursor = self.parent(ancestor.key);
        }
        let pose = affine_pose(
            parent_node
                .map_or(Affine3A::IDENTITY, |n| n.affine())
                .inverse()
                * node.affine(),
        )?;
        Ok(Some(EntityPatch {
            detach: parent_id.is_none(),
            parent: parent_id,
            transform: Some(crate::scene_io::transform(pose)),
            ..Default::default()
        }))
    }

    #[cfg(feature = "editor")]
    pub(crate) fn drop_node(&mut self, key: u32, destination: TreeDrop<u32>) -> io::Result<bool> {
        let Some(node) = self.node(key) else {
            return Err(invalid("Объект больше не существует"));
        };
        let (parent, anchor) = match destination {
            TreeDrop::Root => (None, None),
            TreeDrop::Inside(parent) => (Some(parent), None),
            TreeDrop::Before(target) | TreeDrop::After(target) => {
                if target == key {
                    return Ok(false);
                }
                self.node(target)
                    .ok_or_else(|| invalid("Целевой объект больше не существует"))?;
                (self.parent(target).map(|n| n.key), Some(target))
            }
        };
        let parent_patch = self.parent_patch(key, parent)?;
        let parent_id = parent
            .and_then(|key| self.node(key))
            .map(|n| n.entity.id.as_str());
        let mut siblings: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| n.entity.parent.as_deref() == parent_id)
            .collect();
        let old_index = siblings.iter().position(|n| n.key == key);
        if let Some(index) = old_index {
            siblings.remove(index);
        }
        let index = if let Some(anchor) = anchor {
            siblings.iter().position(|n| n.key == anchor).unwrap()
                + usize::from(matches!(destination, TreeDrop::After(_)))
        } else {
            siblings.len()
        };
        if parent_patch.is_none() && old_index == Some(index) {
            return Ok(false);
        }

        let mut next = self.clone();
        let mut patch = parent_patch.unwrap_or_default();
        let lower = index.checked_sub(1).map(|i| &siblings[i].entity);
        let upper = siblings.get(index).map(|n| &n.entity);
        // Use the existing ID tie-break if it gives the requested position.
        let min = lower.map_or(i32::MIN as i64, |n| {
            i64::from(n.order) + i64::from(n.id >= node.entity.id)
        });
        let max = upper.map_or(i32::MAX as i64, |n| {
            i64::from(n.order) - i64::from(n.id <= node.entity.id)
        });
        if min <= max && min <= i64::from(i32::MAX) && max >= i64::from(i32::MIN) {
            let order = i64::from(node.entity.order).clamp(min, max) as i32;
            if order != node.entity.order {
                patch.order = Some(order);
            }
        } else {
            // Tied orders may leave no slot. Only renumber the destination siblings.
            siblings.insert(index, node);
            for (order, sibling) in siblings.into_iter().enumerate() {
                let order = order as i32;
                if sibling.key == key {
                    patch.order = Some(order);
                } else if sibling.entity.order != order {
                    next.patch(
                        &sibling.entity.id,
                        EntityPatch {
                            order: Some(order),
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        next.patch(&node.entity.id, patch)?;
        next.source
            .settings
            .insert(TREE_ORDER_SETTING.into(), Value::Bool(true));
        next.resolve(None)?;
        *self = next;
        Ok(true)
    }
    pub fn sibling_offset(&self, key: u32, later: bool) -> Option<usize> {
        let node = self.node(key)?;
        let siblings = self
            .nodes
            .iter()
            .filter(|n| n.entity.parent == node.entity.parent);
        let count = siblings.clone().count();
        let index = siblings.into_iter().position(|n| n.key == key)?;
        if later {
            (index + 1 < count).then_some(index + 1)
        } else {
            index.checked_sub(1)
        }
    }
    pub fn move_sibling(&mut self, key: u32, later: bool) -> io::Result<()> {
        let Some(destination) = self.sibling_offset(key, later) else {
            return Ok(());
        };
        let node = self.node(key).unwrap();
        let mut siblings: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| n.entity.parent == node.entity.parent)
            .map(|n| (n.entity.id.clone(), n.entity.order, n.key))
            .collect();
        let index = siblings.iter().position(|n| n.2 == key).unwrap();
        let mut next = self.clone();
        let first = siblings[index].1;
        let second = siblings[destination].1;
        if first != second
            && siblings
                .iter()
                .filter(|n| n.1 == first || n.1 == second)
                .count()
                == 2
        {
            next.patch(
                &siblings[index].0,
                EntityPatch {
                    order: Some(second),
                    ..Default::default()
                },
            )?;
            next.patch(
                &siblings[destination].0,
                EntityPatch {
                    order: Some(first),
                    ..Default::default()
                },
            )?;
        } else {
            siblings.swap(index, destination);
            for (order, (id, previous, _)) in siblings.into_iter().enumerate() {
                let order = order as i32;
                if previous != order {
                    next.patch(
                        &id,
                        EntityPatch {
                            order: Some(order),
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        next.resolve(None)?;
        *self = next;
        Ok(())
    }
    pub fn subtree(&self, key: u32) -> BTreeSet<String> {
        let Some(root) = self.node(key) else {
            return BTreeSet::new();
        };
        let mut ids = BTreeSet::from([root.entity.id.clone()]);
        loop {
            let before = ids.len();
            for n in &self.nodes {
                if n.entity.parent.as_ref().is_some_and(|p| ids.contains(p)) {
                    ids.insert(n.entity.id.clone());
                }
            }
            if ids.len() == before {
                break;
            }
        }
        ids
    }
    pub fn delete(&mut self, keys: &[u32]) -> io::Result<()> {
        let mut next = self.clone();
        next.delete_nodes(keys)?;
        *self = next;
        Ok(())
    }
    fn delete_nodes(&mut self, keys: &[u32]) -> io::Result<()> {
        let mut ids = BTreeSet::new();
        let mut children = HashMap::<&str, Vec<&SceneNode>>::new();
        for node in &self.nodes {
            if let Some(parent) = node.entity.parent.as_deref() {
                children.entry(parent).or_default().push(node);
            }
            if let Some((owner, _)) = node.entity.id.rsplit_once('/') {
                children.entry(owner).or_default().push(node);
            }
        }
        let mut pending: Vec<_> = keys.iter().filter_map(|&key| self.node(key)).collect();
        while let Some(node) = pending.pop() {
            if ids.insert(node.entity.id.clone()) {
                pending.extend(
                    children
                        .get(node.entity.id.as_str())
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        self.source.entities.retain(|e| !ids.contains(&e.id));
        for id in &ids {
            if let Some((owner, _)) = id.split_once('/')
                && !ids.contains(owner)
            {
                // Dropping a parent override can move this node back outside the
                // deleted branch. Keep an explicit removal for each affected node.
                self.patch(
                    id,
                    EntityPatch {
                        remove: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        self.resolve(None)
    }
    pub fn reset_overrides(&mut self, key: u32) -> io::Result<()> {
        let mut next = self.clone();
        next.clear_overrides(key)?;
        *self = next;
        Ok(())
    }
    fn clear_overrides(&mut self, key: u32) -> io::Result<()> {
        let descendants = self.subtree(key);
        let id = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?
            .entity
            .id
            .clone();
        if let Some((mount, child)) = id.split_once('/') {
            if let Some(instance) = self
                .source
                .entities
                .iter_mut()
                .find(|e| e.id == mount)
                .and_then(|e| e.instance.as_mut())
            {
                instance.overrides.retain(|id, _| {
                    id != child
                        && !id.starts_with(&format!("{child}/"))
                        && !descendants.contains(&format!("{mount}/{id}"))
                });
            }
        } else if let Some(instance) = self
            .source
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .and_then(|e| e.instance.as_mut())
        {
            instance.overrides.clear();
        }
        self.resolve(None)
    }
    pub fn duplicate(&mut self, key: u32) -> io::Result<u32> {
        let node = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?;
        if node.inherited() {
            return Err(invalid(
                "Для копирования вложенного объекта распакуйте экземпляр",
            ));
        }
        let root_id = node.entity.id.clone();
        let ids = self.subtree(key);
        let originals: Vec<_> = self
            .source
            .entities
            .iter()
            .filter(|e| ids.contains(&e.id))
            .cloned()
            .collect();
        let mut map = BTreeMap::new();
        for original in &originals {
            let id = self.new_id("copy");
            map.insert(original.id.clone(), id.clone());
            self.source.entities.push(SceneEntity::new(id));
        }
        // Include inherited paths when remapping parent and typed references.
        for node in &self.nodes {
            let id = &node.entity.id;
            if !map.contains_key(id) {
                if let Some((prefix, rest)) = id.split_once('/') {
                    if let Some(copy) = map.get(prefix) {
                        map.insert(id.clone(), format!("{copy}/{rest}"));
                    }
                }
            }
        }
        for mut entity in originals {
            let old = entity.id.clone();
            remap_entity(&mut entity, &map, &BTreeMap::new());
            if let Some(camera) = entity.components.get_mut("camera") {
                camera["id"] = entity.id.clone().into();
            }
            if old == root_id {
                entity.transform.translation[0] += 0.5;
                entity.name = Some(format!(
                    "{} · копия",
                    entity
                        .name
                        .as_deref()
                        .unwrap_or("Объект")
                        .chars()
                        .take(115)
                        .collect::<String>()
                ));
            }
            let slot = self
                .source
                .entities
                .iter_mut()
                .find(|e| e.id == entity.id)
                .unwrap();
            *slot = entity;
        }
        self.resolve(None)?;
        Ok(self.by_id(&map[&root_id]).unwrap().key)
    }
    pub fn unpack(&mut self, key: u32) -> io::Result<()> {
        let node = self
            .node(key)
            .ok_or_else(|| invalid("Object no longer exists"))?;
        if node.inherited() || node.instance.is_none() {
            return Err(invalid("Выберите корень экземпляра вложенной сцены"));
        }
        let root = node.entity.id.clone();
        let prefix = format!("{root}/");
        let ids: BTreeSet<_> = self
            .nodes
            .iter()
            .filter(|n| n.entity.id == root || n.entity.id.starts_with(&prefix))
            .map(|n| n.entity.id.clone())
            .collect();
        let mut map = BTreeMap::new();
        for id in &ids {
            let mapped = if self.source.entities.iter().any(|e| e.id == *id) {
                id.clone()
            } else {
                let next = self.new_id("unpacked");
                self.source.entities.push(SceneEntity::new(&next));
                next
            };
            map.insert(id.clone(), mapped);
        }
        let mut resources = BTreeMap::new();
        for (key, resource) in &self.resources {
            if self.source.resources.contains_key(key) || !key.starts_with(&format!("{root}/")) {
                continue;
            }
            let new = unique_key(&self.source.resources, "unpacked_resource");
            self.source.resources.insert(new.clone(), resource.clone());
            resources.insert(key.clone(), new);
        }
        for node in self.nodes.iter().filter(|n| ids.contains(&n.entity.id)) {
            let mut entity = node.entity.clone();
            remap_entity(&mut entity, &map, &resources);
            if let Some(id) = &node.camera
                && let Some(camera) = entity.components.get_mut("camera")
            {
                camera["id"] = id.clone().into();
            }
            entity.instance = None;
            let slot = self
                .source
                .entities
                .iter_mut()
                .find(|e| e.id == entity.id)
                .unwrap();
            *slot = entity;
        }
        // Remap external attachments and component references to unpacked children too.
        for e in &mut self.source.entities {
            if !ids.contains(&e.id) || e.id == root {
                remap_entity(e, &map, &resources);
            }
        }
        for value in self.source.settings.values_mut() {
            remap_value(value, &map, &resources);
        }
        for resource in self.source.resources.values_mut() {
            if let Some(data) = &mut resource.data {
                remap_value(data, &map, &resources);
            }
        }
        self.resolve(None)
    }
    /// Reconcile inspector/property tools that operate on the compatibility view.
    pub fn reconcile<K: Ord + Serialize>(&mut self, edits: &LevelEdits<K>) -> io::Result<()> {
        let baseline: LevelEdits = self.projected()?;
        let models: HashMap<_, _> = edits.models.iter().map(|m| (m.id, m)).collect();
        let previous_models: HashMap<_, _> = baseline.models.iter().map(|m| (m.id, m)).collect();
        let lights: HashMap<_, _> = edits.lights.iter().map(|l| (l.id, l)).collect();
        let previous_lights: HashMap<_, _> = baseline.lights.iter().map(|l| (l.id, l)).collect();
        let cameras: HashMap<_, _> = baseline
            .scene
            .as_ref()
            .unwrap()
            .cameras
            .iter()
            .map(|c| (c.id.as_str(), c))
            .collect();
        let current_cameras = serde_json::to_value(&edits.cameras).map_err(invalid)?;
        let mut changed = false;
        let mut removed = vec![];
        for key in self.nodes.iter().map(|n| n.key).collect::<Vec<_>>() {
            let node = self.node(key).unwrap().clone();
            let mut patch = EntityPatch::default();
            if node.entity.model.is_some() {
                let Some(model) = models.get(&node.key) else {
                    removed.push(node.key);
                    continue;
                };
                let old = previous_models[&node.key];
                if model.pose != old.pose {
                    self.set_world(node.key, model.pose)?;
                    changed = true;
                }
                if model.asset != old.asset {
                    let key = unique_key(&self.source.resources, "model");
                    self.source
                        .resources
                        .insert(key.clone(), Resource::external("model", &model.asset));
                    patch.model = Some(key);
                }
                if model.bounds != old.bounds {
                    if let Some(bounds) = model.bounds {
                        patch.components.insert(
                            "bounds".into(),
                            serde_json::to_value(bounds).map_err(invalid)?,
                        );
                    } else {
                        patch.remove_components.push("bounds".into());
                    }
                }
                if edits.model_names.get(&node.key) != baseline.model_names.get(&node.key) {
                    patch.name = Some(
                        edits
                            .model_names
                            .get(&node.key)
                            .cloned()
                            .unwrap_or_else(|| "Объект".into()),
                    );
                }
            }
            if node.entity.components.contains_key("light") {
                let Some(light) = lights.get(&node.key) else {
                    removed.push(node.key);
                    continue;
                };
                let old = previous_lights[&node.key];
                if light.pose != old.pose {
                    self.set_world(node.key, light.pose)?;
                    changed = true;
                }
                if light.name != old.name {
                    patch.name = Some(light.name.clone());
                }
                let mut value = serde_json::to_value(light).map_err(invalid)?;
                for field in ["id", "name", "pose"] {
                    value.as_object_mut().unwrap().remove(field);
                }
                if node.entity.components.get("light") != Some(&value) {
                    patch.components.insert("light".into(), value);
                }
            }
            if let Some(id) = &node.camera {
                let definition = cameras[id.as_str()];
                let old = baseline
                    .cameras
                    .get(id)
                    .copied()
                    .unwrap_or(definition.placement);
                let current = current_cameras
                    .get(id)
                    .map(|v| serde_json::from_value::<CameraPlacement>(v.clone()).map_err(invalid))
                    .transpose()?
                    .unwrap_or(definition.placement);
                if current.pose != old.pose {
                    self.set_world(node.key, current.pose)?;
                    changed = true;
                }
                if current.lens != old.lens {
                    let mut value = node.entity.components["camera"].clone();
                    value["lens"] = current.lens.into();
                    patch.components.insert("camera".into(), value);
                }
            }
            if patch != EntityPatch::default() {
                self.patch(&node.entity.id, patch)?;
                changed = true;
            }
        }
        if !removed.is_empty() {
            self.delete(&removed)?;
            changed = true;
        }
        let mut added = LevelEdits::<String> {
            scene: Some(SceneSettings {
                label: self.source.name.clone(),
                cameras: vec![],
            }),
            ..Default::default()
        };
        added.models = edits
            .models
            .iter()
            .filter(|m| !previous_models.contains_key(&m.id))
            .cloned()
            .collect();
        added.model_names = edits.model_names.clone();
        added.lights = edits
            .lights
            .iter()
            .filter(|l| !previous_lights.contains_key(&l.id))
            .cloned()
            .collect();
        if !added.models.is_empty() || !added.lights.is_empty() {
            let mut extra =
                Scene::from_slice(&crate::scene_io::encode(&added, None)?).map_err(invalid)?;
            let mut ids = BTreeMap::new();
            for e in &extra.entities {
                let id = self.new_id("object");
                self.source.entities.push(SceneEntity::new(&id));
                ids.insert(e.id.clone(), id);
            }
            let mut resources = BTreeMap::new();
            for (key, resource) in &extra.resources {
                let id = unique_key(&self.source.resources, "model");
                self.source.resources.insert(id.clone(), resource.clone());
                resources.insert(key.clone(), id);
            }
            for e in &mut extra.entities {
                remap_entity(e, &ids, &resources);
                *self
                    .source
                    .entities
                    .iter_mut()
                    .find(|old| old.id == e.id)
                    .unwrap() = e.clone();
            }
            changed = true;
        }
        if let Some(scene) = &edits.scene {
            self.source.name = scene.label.clone();
        }
        if let Some(environment) = &edits.lighting {
            self.source.settings.insert(
                "environment".into(),
                serde_json::to_value(environment).map_err(invalid)?,
            );
        } else {
            self.source.settings.remove("environment");
        }
        if changed {
            self.resolve(None)?;
        }
        Ok(())
    }
    /// Spawn spatial nodes once and parent render components to these entities.
    #[cfg(feature = "lighting")]
    pub fn spawn(&self, commands: &mut Commands) -> HashMap<u32, Entity> {
        let entities: HashMap<_, _> = self
            .nodes
            .iter()
            .map(|n| {
                (
                    n.key,
                    commands
                        .spawn((
                            Name::new(n.entity.name.clone().unwrap_or_else(|| n.entity.id.clone())),
                            n.local().transform(),
                            GlobalTransform::default(),
                            Visibility::default(),
                        ))
                        .id(),
                )
            })
            .collect();
        for n in &self.nodes {
            if let Some(parent) = self.parent(n.key) {
                commands
                    .entity(entities[&n.key])
                    .insert(ChildOf(entities[&parent.key]));
            }
        }
        entities
    }
}
fn same_pose(a: Pose, b: Pose) -> bool {
    let a = a.transform();
    let b = b.transform();
    a.translation.abs_diff_eq(b.translation, 1e-5)
        && a.scale.abs_diff_eq(b.scale, 1e-5)
        && a.rotation.dot(b.rotation).abs() > 1.0 - 1e-6
}
fn clear_camera_reset(camera: &mut Value, pose: Pose) -> io::Result<Option<Pose>> {
    if let Some(reset) = camera.get("reset") {
        let reset: CameraPlacement = serde_json::from_value(reset.clone()).map_err(invalid)?;
        if same_pose(reset.pose, pose)
            && (camera["lens"].as_f64().unwrap() as f32 - reset.lens).abs() < 1e-5
        {
            camera.as_object_mut().unwrap().remove("reset");
            return Ok(Some(reset.pose));
        }
    }
    Ok(None)
}
fn affine_pose(affine: Affine3A) -> io::Result<Pose> {
    let transform = Transform::from_matrix(Mat4::from(affine));
    let rebuilt = transform.compute_affine();
    let error = affine
        .to_cols_array()
        .into_iter()
        .zip(rebuilt.to_cols_array())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let tolerance = affine.to_cols_array()[..9]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(1.0f32, f32::max)
        * 0.00001;
    if !transform.translation.is_finite() || error > tolerance || !Pose::from(transform).valid() {
        return Err(invalid(
            "Преобразование требует сдвига осей (shear). Используйте локальные координаты или сохраните текущего родителя",
        ));
    }
    Ok(transform.into())
}
fn unique_key<T>(map: &BTreeMap<String, T>, prefix: &str) -> String {
    (1u32..)
        .map(|i| format!("{prefix}_{i}"))
        .find(|key| !map.contains_key(key))
        .unwrap()
}
fn mapped(value: &mut String, map: &BTreeMap<String, String>) {
    if let Some(new) = map.get(value) {
        *value = new.clone();
    }
}
fn remap_value(
    value: &mut Value,
    ids: &BTreeMap<String, String>,
    resources: &BTreeMap<String, String>,
) {
    match value {
        Value::Object(values) => {
            if let Some(Value::String(id)) = values.get_mut("$entity") {
                mapped(id, ids);
            } else if let Some(Value::String(id)) = values.get_mut("$resource") {
                mapped(id, resources);
            } else {
                for value in values.values_mut() {
                    remap_value(value, ids, resources);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                remap_value(value, ids, resources);
            }
        }
        _ => {}
    }
}
fn remap_entity(
    e: &mut SceneEntity,
    ids: &BTreeMap<String, String>,
    resources: &BTreeMap<String, String>,
) {
    mapped(&mut e.id, ids);
    if let Some(p) = &mut e.parent {
        mapped(p, ids);
    }
    if let Some(m) = &mut e.model {
        mapped(m, resources);
    }
    for v in e.components.values_mut() {
        remap_value(v, ids, resources);
    }
    if let Some(i) = &mut e.instance {
        mapped(&mut i.resource, resources);
        for p in i.overrides.values_mut() {
            if let Some(parent) = &mut p.parent {
                mapped(parent, ids);
            }
            if let Some(m) = &mut p.model {
                mapped(m, resources);
            }
            for v in p.components.values_mut() {
                remap_value(v, ids, resources);
            }
        }
    }
}
fn remap_scene(
    scene: &mut Scene,
    ids: &BTreeMap<String, String>,
    resources: &BTreeMap<String, String>,
) {
    for e in &mut scene.entities {
        remap_entity(e, ids, resources);
    }
    scene.resources = std::mem::take(&mut scene.resources)
        .into_iter()
        .map(|(mut k, mut r)| {
            mapped(&mut k, resources);
            if let Some(v) = &mut r.data {
                remap_value(v, ids, resources);
            }
            (k, r)
        })
        .collect();
    for v in scene.settings.values_mut() {
        remap_value(v, ids, resources);
    }
}
