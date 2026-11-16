//! Scene authoring documents and projections for model/camera/light tools.
use crate::data::{CameraDefinition, LensKind};
use crate::{CameraPlacement, LevelEdits, PlacedModel, Pose, SceneSettings};
use bevy_scene_format::{Entity, Resource, Scene};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    io,
};

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// Canonical files put the discriminant first. Other JSON orderings use a
/// complete lightweight probe; serde requires visitors to consume their map.
pub(crate) fn modern(bytes: &[u8]) -> io::Result<bool> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let first = bytes
        .trim_ascii_start()
        .strip_prefix(b"{")
        .map(<[u8]>::trim_ascii_start);
    if let Some(rest) = first.and_then(|value| value.strip_prefix(b"\"format\"")) {
        if let Some(value) = rest.trim_ascii_start().strip_prefix(b":") {
            if value.trim_ascii_start().starts_with(b"\"bevy_scene\"") {
                return Ok(true);
            }
        }
    }
    struct Probe;
    impl<'de> serde::de::Visitor<'de> for Probe {
        type Value = bool;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a scene object")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<bool, M::Error> {
            let mut modern = false;
            while let Some(key) = map.next_key::<String>()? {
                if key == "format" {
                    modern = map.next_value::<String>()? == "bevy_scene";
                } else {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(modern)
        }
    }
    use serde::de::Deserializer;
    serde_json::Deserializer::from_slice(bytes)
        .deserialize_map(Probe)
        .map_err(invalid)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Camera {
    pub(crate) id: String,
    pub(crate) projection: LensKind,
    pub(crate) lens: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reset: Option<CameraPlacement>,
}

pub(crate) fn transform(pose: Pose) -> bevy_scene_format::Transform {
    bevy_scene_format::Transform {
        translation: pose.position,
        rotation: pose.rotation,
        scale: pose.scale,
    }
}
pub(crate) fn pose(transform: bevy_scene_format::Transform) -> Pose {
    Pose {
        position: transform.translation,
        rotation: transform.rotation,
        scale: transform.scale,
    }
}
fn flat(scene: &Scene) -> io::Result<()> {
    if scene
        .entities
        .iter()
        .any(|entity| entity.parent.is_some() || entity.instance.is_some())
    {
        return Err(invalid(
            "Hierarchical documents require their authoring graph; stale flat edits cannot overwrite this scene",
        ));
    }
    if scene.entities.iter().any(|entity| {
        usize::from(entity.model.is_some())
            + usize::from(entity.components.contains_key("camera"))
            + usize::from(entity.components.contains_key("light"))
            > 1
    }) {
        return Err(invalid(
            "The current editor requires separate model, camera and light entities",
        ));
    }
    Ok(())
}
pub(crate) fn number(id: &str, kind: &str) -> u32 {
    id.strip_prefix(&format!("{kind}:"))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            u32::from_le_bytes(
                bevy_scene_format::content_digest(id.as_bytes())[..4]
                    .try_into()
                    .unwrap(),
            )
        })
}
fn camera_entity_id(id: &str) -> String {
    let digest = bevy_scene_format::content_digest(id.as_bytes());
    format!(
        "camera:{:016x}",
        u64::from_le_bytes(digest[..8].try_into().unwrap())
    )
}

pub(crate) fn decode<K: Ord + serde::de::DeserializeOwned>(
    bytes: &[u8],
    root: &std::path::Path,
) -> io::Result<LevelEdits<K>> {
    let scene = Scene::from_slice(bytes).map_err(invalid)?;
    if scene.entities.iter().any(|entity| {
        entity.parent.is_some()
            || entity.instance.is_some()
            || (entity.model.is_none()
                && !entity.components.contains_key("camera")
                && !entity.components.contains_key("light"))
            || usize::from(entity.model.is_some())
                + usize::from(entity.components.contains_key("camera"))
                + usize::from(entity.components.contains_key("light"))
                > 1
    }) || !scene
        .entities
        .iter()
        .any(|entity| entity.components.contains_key("camera"))
    {
        crate::hierarchy::SceneHierarchy::load(scene, root)?.project()
    } else {
        decode_scene(scene)
    }
}
pub(crate) fn decode_packed<K: Ord + serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> io::Result<LevelEdits<K>> {
    let bytes = bevy_scene_format::decompress(bytes).map_err(invalid)?;
    let resolved = bevy_scene_format::PackedScene::open(&bytes, None)
        .map_err(invalid)?
        .unpack()
        .map_err(invalid)?;
    let scene = Scene {
        format: "bevy_scene".into(),
        version: bevy_scene_format::VERSION,
        id: resolved.id,
        name: resolved.name,
        settings: resolved.settings,
        resources: resolved.resources,
        entities: resolved.entities,
    };
    if flat(&scene).is_ok()
        && scene
            .entities
            .iter()
            .any(|e| e.components.contains_key("camera"))
        && scene.entities.iter().all(|e| {
            e.model.is_some()
                || e.components.contains_key("camera")
                || e.components.contains_key("light")
        })
    {
        return decode_scene(scene);
    }
    crate::hierarchy::SceneHierarchy::from_resolved(scene)?.project()
}
pub(crate) fn decode_scene<K: Ord + serde::de::DeserializeOwned>(
    scene: Scene,
) -> io::Result<LevelEdits<K>> {
    flat(&scene)?;
    let mut edits = LevelEdits::default();
    let mut cameras = Vec::new();
    let mut entities: Vec<_> = scene.entities.iter().collect();
    entities.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    let mut model_ids = HashSet::new();
    let mut light_ids = HashSet::new();
    for entity in entities {
        if let Some(model) = &entity.model {
            let resource = &scene.resources[model];
            if resource
                .selector
                .as_deref()
                .is_some_and(|selector| selector != "Scene0")
            {
                return Err(invalid("The current editor supports GLB Scene0 models"));
            }
            let id = number(&entity.id, "model");
            if !model_ids.insert(id) {
                return Err(invalid("Model ID collision in editor bridge"));
            }
            edits.models.push(PlacedModel {
                id,
                asset: resource
                    .path
                    .clone()
                    .ok_or_else(|| invalid("An editor model requires an external GLB"))?,
                pose: pose(entity.transform),
                bounds: entity
                    .components
                    .get("bounds")
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()
                    .map_err(invalid)?,
            });
            if let Some(name) = &entity.name {
                edits.model_names.insert(id, name.clone());
            }
        }
        if let Some(value) = entity.components.get("camera") {
            let camera: Camera = serde_json::from_value(value.clone()).map_err(invalid)?;
            let current = CameraPlacement {
                pose: pose(entity.transform),
                lens: camera.lens,
            };
            let original = camera.reset.unwrap_or(current);
            if camera.reset.is_some() {
                let key =
                    serde_json::from_value(Value::String(camera.id.clone())).map_err(invalid)?;
                edits.cameras.insert(key, current);
            }
            cameras.push(CameraDefinition {
                label: entity.name.clone().unwrap_or_else(|| camera.id.clone()),
                id: camera.id,
                projection: camera.projection,
                placement: original,
            });
        }
        if let Some(value) = entity.components.get("light") {
            let id = number(&entity.id, "light");
            if !light_ids.insert(id) {
                return Err(invalid("Light ID collision in editor bridge"));
            }
            let mut value = value.clone();
            let fields = value
                .as_object_mut()
                .ok_or_else(|| invalid("Light component must be an object"))?;
            fields.insert("id".into(), id.into());
            fields.insert(
                "name".into(),
                entity.name.as_deref().unwrap_or("Light").into(),
            );
            fields.insert(
                "pose".into(),
                serde_json::to_value(pose(entity.transform)).map_err(invalid)?,
            );
            edits
                .lights
                .push(serde_json::from_value(value).map_err(invalid)?);
        }
    }
    edits.scene = Some(SceneSettings {
        label: scene.name,
        cameras,
    });
    edits.lighting = scene
        .settings
        .get("environment")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(invalid)?;
    Ok(edits)
}

pub(crate) fn encode<K: Ord + Serialize>(
    edits: &LevelEdits<K>,
    previous: Option<&[u8]>,
) -> io::Result<Vec<u8>> {
    if let Some(hierarchy) = &edits.hierarchy {
        let mut hierarchy = (**hierarchy).clone();
        hierarchy.reconcile(edits)?;
        return hierarchy.source().to_source().map_err(invalid);
    }
    let settings = edits
        .scene
        .as_ref()
        .ok_or_else(|| invalid("Scene metadata is missing"))?;
    if !edits.nodes.is_empty() || !edits.node_names.is_empty() || !edits.deleted_nodes.is_empty() {
        return Err(invalid(
            "Native scenes cannot contain legacy source-node edits",
        ));
    }
    let mut scene = match previous {
        Some(bytes) if modern(bytes)? => Scene::from_slice(bytes).map_err(invalid)?,
        _ => Scene::new(&settings.label),
    };
    flat(&scene)?;
    scene.name = settings.label.clone();
    let previous_entities = std::mem::take(&mut scene.entities);
    let previous_models: BTreeMap<_, _> = previous_entities
        .iter()
        .filter(|entity| entity.model.is_some())
        .map(|entity| (number(&entity.id, "model"), entity))
        .collect();
    let previous_lights: BTreeMap<_, _> = previous_entities
        .iter()
        .filter(|entity| entity.components.contains_key("light"))
        .map(|entity| (number(&entity.id, "light"), entity))
        .collect();
    let previous_cameras: BTreeMap<_, _> = previous_entities
        .iter()
        .filter_map(|entity| {
            entity
                .components
                .get("camera")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .map(|id| (id.to_owned(), entity))
        })
        .collect();
    let mut assets: BTreeMap<_, _> = scene
        .resources
        .iter()
        .filter_map(|(key, resource)| {
            (resource.kind == "model")
                .then(|| {
                    resource
                        .path
                        .as_ref()
                        .map(|path| (path.clone(), key.clone()))
                })
                .flatten()
        })
        .collect();
    for (index, model) in edits.models.iter().enumerate() {
        let resource = if let Some(key) = assets.get(&model.asset) {
            key.clone()
        } else {
            let digest = bevy_scene_format::content_digest(model.asset.as_bytes());
            let stem = format!(
                "model_{:016x}",
                u64::from_le_bytes(digest[..8].try_into().unwrap())
            );
            let mut key = stem.clone();
            let mut suffix = 0u32;
            while scene.resources.contains_key(&key) {
                suffix += 1;
                key = format!("{stem}_{suffix}");
            }
            scene
                .resources
                .insert(key.clone(), Resource::external("model", &model.asset));
            assets.insert(model.asset.clone(), key.clone());
            key
        };
        let mut entity = previous_models
            .get(&model.id)
            .map(|value| (**value).clone())
            .unwrap_or_else(|| Entity::new(format!("model:{}", model.id)));
        entity.name = edits.model_names.get(&model.id).cloned();
        entity.transform = transform(model.pose);
        entity.order = index as i32;
        entity.model = Some(resource);
        if let Some(bounds) = model.bounds {
            entity.components.insert(
                "bounds".into(),
                serde_json::to_value(bounds).map_err(invalid)?,
            );
        } else {
            entity.components.remove("bounds");
        }
        scene.entities.push(entity);
    }
    let camera_edits = serde_json::to_value(&edits.cameras).map_err(invalid)?;
    if camera_edits.as_object().is_none_or(|cameras| {
        cameras
            .keys()
            .any(|key| !settings.cameras.iter().any(|camera| camera.id == *key))
    }) {
        return Err(invalid("Unknown native scene camera override"));
    }
    for (index, definition) in settings.cameras.iter().enumerate() {
        let override_placement = camera_edits
            .get(&definition.id)
            .map(|value| serde_json::from_value::<CameraPlacement>(value.clone()))
            .transpose()
            .map_err(invalid)?;
        let current = override_placement.unwrap_or(definition.placement);
        let mut entity = previous_cameras
            .get(&definition.id)
            .map(|value| (**value).clone())
            .unwrap_or_else(|| Entity::new(camera_entity_id(&definition.id)));
        entity.name = Some(definition.label.clone());
        entity.order = index as i32;
        entity.transform = transform(current.pose);
        entity.components.insert(
            "camera".into(),
            serde_json::to_value(Camera {
                id: definition.id.clone(),
                projection: definition.projection,
                lens: current.lens,
                reset: override_placement.map(|_| definition.placement),
            })
            .map_err(invalid)?,
        );
        scene.entities.push(entity);
    }
    for (index, light) in edits.lights.iter().enumerate() {
        let mut entity = previous_lights
            .get(&light.id)
            .map(|value| (**value).clone())
            .unwrap_or_else(|| Entity::new(format!("light:{}", light.id)));
        entity.name = Some(light.name.clone());
        entity.order = index as i32;
        entity.transform = transform(light.pose);
        let mut value = serde_json::to_value(light).map_err(invalid)?;
        let fields = value.as_object_mut().unwrap();
        for key in ["id", "name", "pose"] {
            fields.remove(key);
        }
        entity.components.insert("light".into(), value);
        scene.entities.push(entity);
    }
    // Unregistered components and non-render entities remain authoring data.
    scene
        .entities
        .extend(previous_entities.into_iter().filter(|entity| {
            entity.model.is_none()
                && !entity.components.contains_key("camera")
                && !entity.components.contains_key("light")
        }));
    if let Some(environment) = &edits.lighting {
        scene.settings.insert(
            "environment".into(),
            serde_json::to_value(environment).map_err(invalid)?,
        );
    } else {
        scene.settings.remove("environment");
    }
    scene.to_source().map_err(invalid)
}

#[cfg(feature = "editor")]
pub(crate) fn remap_paths(
    bytes: &[u8],
    from: &std::path::Path,
    to: &std::path::Path,
) -> io::Result<Vec<u8>> {
    let mut scene = Scene::from_slice(bytes).map_err(invalid)?;
    remap_scene_paths(&mut scene, from, to);
    scene.to_source().map_err(invalid)
}

#[cfg(feature = "editor")]
pub(crate) fn remap_scene_paths(scene: &mut Scene, from: &std::path::Path, to: &std::path::Path) {
    let remap = |value: &mut String| {
        if let Ok(suffix) = std::path::Path::new(value).strip_prefix(from) {
            let mapped = if suffix.as_os_str().is_empty() {
                to.to_path_buf()
            } else {
                to.join(suffix)
            };
            *value = mapped.to_string_lossy().replace('\\', "/");
        }
    };
    for resource in scene.resources.values_mut() {
        if let Some(path) = &mut resource.path {
            remap(path);
        }
    }
    if let Some(environment) = scene.settings.get_mut("environment") {
        if let Some(path) = environment
            .pointer_mut("/hdri_asset/asset")
            .and_then(|value| value.as_str().map(str::to_owned))
        {
            let mut mapped = path;
            remap(&mut mapped);
            environment["hdri_asset"]["asset"] = mapped.into();
        }
    }
}
