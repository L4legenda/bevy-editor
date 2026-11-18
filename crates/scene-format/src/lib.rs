//! Authoring documents are canonical UTF-8 JSON; runtime data is derived, checked
//! binary. Neither representation contains engine entity IDs or executable code.
mod files;
mod packed;
mod resolve;
pub use files::{Revision, SourceDocument, atomic_write, read_limited};
pub use packed::{EntityView, PackedScene, ResourceView, compress, cook, decompress};
pub use resolve::{
    Dependency, ResolveLimits, ResolvedScene, SceneLoader, resolve, resolve_with_limits,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt, io,
};

pub const VERSION: u32 = 1;
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ENTITIES: usize = 1_000_000;
pub const MAX_RESOURCES: usize = 100_000;
pub const MAX_INSTANCE_DEPTH: usize = 64;
pub fn content_digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error(message.into()))
    }
}

/// Defaults belong to format version 1, independently of engine defaults.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transform {
    #[serde(skip_serializing_if = "zero3")]
    pub translation: [f32; 3],
    #[serde(skip_serializing_if = "identity_rotation")]
    pub rotation: [f32; 4],
    #[serde(skip_serializing_if = "one3")]
    pub scale: [f32; 3],
}
fn zero3(value: &[f32; 3]) -> bool {
    *value == [0.0; 3]
}
fn one3(value: &[f32; 3]) -> bool {
    *value == [1.0; 3]
}
fn identity_rotation(value: &[f32; 4]) -> bool {
    *value == [0.0, 0.0, 0.0, 1.0]
}
fn identity(value: &Transform) -> bool {
    *value == Transform::default()
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}
impl Transform {
    pub fn validate(&self) -> Result<()> {
        let norm: f32 = self.rotation.iter().map(|value| value * value).sum();
        ensure(
            self.translation
                .iter()
                .chain(&self.rotation)
                .chain(&self.scale)
                .all(|v| v.is_finite())
                && (0.999..=1.001).contains(&norm)
                && self.scale.iter().all(|v| v.abs() >= 1.0e-6),
            "Non-finite, singular or non-unit transform",
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sharing {
    #[default]
    Shared,
    Local,
}
fn shared(value: &Sharing) -> bool {
    *value == Sharing::Shared
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// Stable, extensible resource type; the codec does not invoke a loader.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_unique_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "shared")]
    pub sharing: Sharing,
}
impl Resource {
    pub fn external(kind: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            path: Some(path.into()),
            uid: None,
            selector: None,
            data: None,
            sharing: Sharing::Shared,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EntityPatch {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    /// New parent ID in the document declaring the override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Move the node to the root of the document declaring the override.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub detach: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove_model: bool,
    /// Set replaces a whole component. JSON null is a value, never deletion.
    #[serde(
        deserialize_with = "unique_values",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub components: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remove_components: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub resource: String,
    #[serde(
        default,
        deserialize_with = "unique_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub overrides: BTreeMap<String, EntityPatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Sibling order is explicit and independent of canonical file ordering.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub order: i32,
    #[serde(default, skip_serializing_if = "identity")]
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<Instance>,
    #[serde(
        default,
        deserialize_with = "unique_values",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub components: BTreeMap<String, Value>,
}
fn is_zero(value: &i32) -> bool {
    *value == 0
}
impl Entity {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            parent: None,
            order: 0,
            transform: Transform::default(),
            model: None,
            instance: None,
            components: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(
        default,
        deserialize_with = "unique_values",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub settings: BTreeMap<String, Value>,
    #[serde(default, deserialize_with = "unique_map")]
    pub resources: BTreeMap<String, Resource>,
    pub entities: Vec<Entity>,
}
impl Scene {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: "bevy_scene".into(),
            version: VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            settings: BTreeMap::new(),
            resources: BTreeMap::new(),
            entities: Vec::new(),
        }
    }
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        ensure(
            bytes.len() <= MAX_SOURCE_BYTES,
            "Scene source exceeds 64 MiB",
        )?;
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let scene: Self = serde_json::from_slice(bytes)?;
        scene.validate()?;
        Ok(scene)
    }
    pub fn validate(&self) -> Result<()> {
        ensure(
            self.format == "bevy_scene" && self.version == VERSION,
            format!(
                "Unsupported scene format/version: {}/{}",
                self.format, self.version
            ),
        )?;
        ensure(
            uuid::Uuid::parse_str(&self.id).is_ok_and(|id| !id.is_nil()),
            "Scene ID must be a non-nil UUID",
        )?;
        ensure(label(&self.name), "Invalid scene name")?;
        let ids = validate_parts(&self.resources, &self.entities, false)?;
        for value in self.settings.values() {
            validate_value(value, &ids, &self.resources, false, 0)?;
        }
        Ok(())
    }
    /// Stable keys and separate property lines keep independent Git edits local.
    /// Only immutable format defaults are omitted. Component data is preserved.
    pub fn to_source(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = Vec::new();
        use io::Write;
        write!(
            &mut bytes,
            "{{\n  \"format\":\"bevy_scene\",\n  \"version\":{VERSION},\n  \"id\":{},\n  \"name\":{},\n  \"settings\":{},\n  \"resources\":{{",
            serde_json::to_string(&self.id)?,
            serde_json::to_string(&self.name)?,
            serde_json::to_string(&self.settings)?
        )?;
        for (index, (key, resource)) in self.resources.iter().enumerate() {
            if index > 0 {
                bytes.push(b',');
            }
            write!(&mut bytes, "\n    {}:", serde_json::to_string(key)?)?;
            serde_json::to_writer(&mut bytes, resource)?;
        }
        bytes.extend_from_slice(b"\n  },\n  \"entities\":[");
        let mut entities: Vec<_> = self.entities.iter().collect();
        entities.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        for (index, entity) in entities.iter().enumerate() {
            if index > 0 {
                bytes.push(b',');
            }
            bytes.extend_from_slice(b"\n    ");
            write_entity(&mut bytes, entity)?;
        }
        bytes.extend_from_slice(b"\n  ]\n}\n");
        ensure(
            bytes.len() <= MAX_SOURCE_BYTES,
            "Encoded scene exceeds 64 MiB",
        )?;
        Ok(bytes)
    }
}

fn spaces(out: &mut Vec<u8>, indent: usize) {
    out.extend(std::iter::repeat_n(b' ', indent));
}
fn entry(out: &mut Vec<u8>, first: &mut bool, indent: usize, key: &str) -> Result<()> {
    if !*first {
        out.push(b',');
    }
    *first = false;
    out.push(b'\n');
    spaces(out, indent);
    serde_json::to_writer(&mut *out, key)?;
    out.push(b':');
    Ok(())
}
fn field<T: Serialize>(
    out: &mut Vec<u8>,
    first: &mut bool,
    indent: usize,
    key: &str,
    value: &T,
) -> Result<()> {
    entry(out, first, indent, key)?;
    serde_json::to_writer(out, value)?;
    Ok(())
}
fn object_end(out: &mut Vec<u8>, indent: usize) {
    out.push(b'\n');
    spaces(out, indent);
    out.push(b'}');
}
fn write_value(out: &mut Vec<u8>, value: &Value, indent: usize) -> Result<()> {
    if let Value::Object(values) = value
        && !values.is_empty()
        && !values.contains_key("$entity")
        && !values.contains_key("$resource")
    {
        out.push(b'{');
        let mut first = true;
        for (key, value) in values {
            entry(out, &mut first, indent + 2, key)?;
            write_value(out, value, indent + 2)?;
        }
        object_end(out, indent);
        return Ok(());
    }
    serde_json::to_writer(out, value)?;
    Ok(())
}
fn write_entity(out: &mut Vec<u8>, entity: &Entity) -> Result<()> {
    out.push(b'{');
    let mut first = true;
    field(out, &mut first, 6, "id", &entity.id)?;
    if let Some(value) = &entity.name {
        field(out, &mut first, 6, "name", value)?;
    }
    if let Some(value) = &entity.parent {
        field(out, &mut first, 6, "parent", value)?;
    }
    if entity.order != 0 {
        field(out, &mut first, 6, "order", &entity.order)?;
    }
    if !identity(&entity.transform) {
        entry(out, &mut first, 6, "transform")?;
        out.push(b'{');
        let mut pose_first = true;
        if !zero3(&entity.transform.translation) {
            field(
                out,
                &mut pose_first,
                8,
                "translation",
                &entity.transform.translation,
            )?;
        }
        if !identity_rotation(&entity.transform.rotation) {
            field(
                out,
                &mut pose_first,
                8,
                "rotation",
                &entity.transform.rotation,
            )?;
        }
        if !one3(&entity.transform.scale) {
            field(out, &mut pose_first, 8, "scale", &entity.transform.scale)?;
        }
        object_end(out, 6);
    }
    if let Some(value) = &entity.model {
        field(out, &mut first, 6, "model", value)?;
    }
    if let Some(value) = &entity.instance {
        field(out, &mut first, 6, "instance", value)?;
    }
    if !entity.components.is_empty() {
        entry(out, &mut first, 6, "components")?;
        out.push(b'{');
        let mut component_first = true;
        for (key, value) in &entity.components {
            entry(out, &mut component_first, 8, key)?;
            write_value(out, value, 8)?;
        }
        object_end(out, 6);
    }
    object_end(out, 4);
    Ok(())
}

pub(crate) fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
pub(crate) fn identifier(value: &str, scoped: bool) -> bool {
    !value.is_empty()
        && value.len() <= if scoped { 8192 } else { 128 }
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '\\' | '#' | '"'))
        && if scoped {
            value
                .split('/')
                .all(|part| !part.is_empty() && part.len() <= 128)
        } else {
            !value.contains('/')
        }
}
pub fn asset_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value.contains(['\\', ':', '#', '\0'])
        && value.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.chars().any(char::is_control)
        })
}

pub(crate) fn validate_parts<'a>(
    resources: &BTreeMap<String, Resource>,
    entities: &'a [Entity],
    resolved: bool,
) -> Result<HashSet<&'a str>> {
    ensure(
        entities.len() <= MAX_ENTITIES && resources.len() <= MAX_RESOURCES,
        "Scene entity/resource limit exceeded",
    )?;
    let mut ids = HashSet::with_capacity(entities.len());
    for entity in entities {
        ensure(
            identifier(&entity.id, resolved) && ids.insert(entity.id.as_str()),
            format!("Invalid or duplicate entity ID {}", entity.id),
        )?;
    }
    for (key, resource) in resources {
        ensure(
            identifier(key, resolved) && identifier(&resource.kind, false),
            format!("Invalid resource ID/type {key}"),
        )?;
        ensure(
            resource.path.is_some() != resource.data.is_some(),
            format!("Resource {key} needs either a path or inline data"),
        )?;
        if let Some(path) = &resource.path {
            ensure(asset_path(path), format!("Invalid asset path {path}"))?;
        }
        if let Some(uid) = &resource.uid {
            ensure(label(uid), format!("Invalid asset UID for {key}"))?;
        }
        if let Some(selector) = &resource.selector {
            ensure(
                label(selector),
                format!("Invalid subasset selector for {key}"),
            )?;
        }
        if let Some(data) = &resource.data {
            validate_value(data, &ids, resources, resolved, 0)?;
        }
    }
    for entity in entities {
        if let Some(name) = &entity.name {
            ensure(label(name), format!("Invalid name for {}", entity.id))?;
        }
        entity.transform.validate()?;
        if let Some(parent) = &entity.parent {
            ensure(
                parent != &entity.id
                    && (ids.contains(parent.as_str()) || (!resolved && parent.contains('/'))),
                format!("Missing or self parent {parent} for {}", entity.id),
            )?;
        }
        if let Some(model) = &entity.model {
            ensure(
                resources.get(model).is_some_and(|r| r.kind == "model"),
                format!("Unknown model resource {model}"),
            )?;
        }
        for (kind, value) in &entity.components {
            ensure(
                identifier(kind, false),
                format!("Invalid component type {kind}"),
            )?;
            validate_value(value, &ids, resources, resolved, 0)?;
        }
        if let Some(instance) = &entity.instance {
            ensure(
                !resolved
                    && resources
                        .get(&instance.resource)
                        .is_some_and(|r| r.kind == "scene" && r.path.is_some()),
                format!("Unknown external scene resource {}", instance.resource),
            )?;
            for (target, patch) in &instance.overrides {
                ensure(
                    identifier(target, true)
                        && target.split('/').all(|part| identifier(part, false)),
                    format!("Invalid override target {target}"),
                )?;
                ensure(
                    !(patch.remove_model && patch.model.is_some()),
                    "Cannot set and remove the same model",
                )?;
                ensure(
                    !(patch.detach && patch.parent.is_some()),
                    "Cannot set and detach the same parent",
                )?;
                if let Some(parent) = &patch.parent {
                    ensure(
                        parent.split('/').all(|part| identifier(part, false))
                            && identifier(parent, true)
                            && (ids.contains(parent.as_str()) || parent.contains('/'))
                            && parent != &format!("{}/{}", entity.id, target),
                        format!("Invalid override parent {parent}"),
                    )?;
                }
                ensure(
                    !patch.remove
                        || (patch.name.is_none()
                            && patch.transform.is_none()
                            && patch.parent.is_none()
                            && !patch.detach
                            && patch.order.is_none()
                            && patch.model.is_none()
                            && !patch.remove_model
                            && patch.components.is_empty()
                            && patch.remove_components.is_empty()),
                    "A removed entity cannot also be edited",
                )?;
                if let Some(name) = &patch.name {
                    ensure(label(name), "Invalid override name")?;
                }
                if let Some(transform) = &patch.transform {
                    transform.validate()?;
                }
                if let Some(model) = &patch.model {
                    ensure(
                        resources.get(model).is_some_and(|r| r.kind == "model"),
                        "Unknown override model",
                    )?;
                }
                let mut removals = HashSet::new();
                for kind in &patch.remove_components {
                    ensure(
                        identifier(kind, false)
                            && removals.insert(kind)
                            && !patch.components.contains_key(kind),
                        "Conflicting or duplicate component removal",
                    )?;
                }
                for (kind, value) in &patch.components {
                    ensure(identifier(kind, false), "Invalid override component type")?;
                    validate_value(value, &ids, resources, resolved, 0)?;
                }
            }
        }
    }
    if entities.iter().any(|entity| entity.parent.is_some()) {
        topological_order(entities, !resolved)?;
    }
    Ok(ids)
}

fn validate_value(
    value: &Value,
    entities: &HashSet<&str>,
    resources: &BTreeMap<String, Resource>,
    resolved: bool,
    depth: usize,
) -> Result<()> {
    ensure(
        depth <= 64,
        "Component/resource data nesting exceeds 64 levels",
    )?;
    match value {
        Value::Object(fields) => {
            if fields.contains_key("$entity") || fields.contains_key("$resource") {
                ensure(
                    fields.len() == 1,
                    "A typed reference cannot contain additional fields",
                )?;
                if let Some(reference) = fields.get("$entity") {
                    let key = reference
                        .as_str()
                        .ok_or_else(|| Error("Entity reference must be a string".into()))?;
                    ensure(
                        entities.contains(key) || (!resolved && key.contains('/')),
                        format!("Dangling entity reference {key}"),
                    )?;
                } else {
                    let key = fields["$resource"]
                        .as_str()
                        .ok_or_else(|| Error("Resource reference must be a string".into()))?;
                    ensure(
                        resources.contains_key(key),
                        format!("Dangling resource reference {key}"),
                    )?;
                }
            } else {
                for value in fields.values() {
                    validate_value(value, entities, resources, resolved, depth + 1)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_value(value, entities, resources, resolved, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Iterative traversal: deeply nested files cannot overflow the call stack.
pub(crate) fn topological_order(entities: &[Entity], allow_deferred: bool) -> Result<Vec<usize>> {
    if entities.iter().all(|entity| entity.parent.is_none()) {
        let mut indices: Vec<_> = (0..entities.len()).collect();
        indices.sort_unstable_by(|a, b| {
            entities[*a]
                .order
                .cmp(&entities[*b].order)
                .then_with(|| entities[*a].id.cmp(&entities[*b].id))
        });
        return Ok(indices);
    }
    let index: HashMap<_, _> = entities
        .iter()
        .enumerate()
        .map(|(i, entity)| (entity.id.as_str(), i))
        .collect();
    let mut children = vec![Vec::new(); entities.len()];
    let mut roots = Vec::new();
    for (i, entity) in entities.iter().enumerate() {
        match entity.parent.as_deref() {
            Some(parent) if index.contains_key(parent) => children[index[parent]].push(i),
            Some(parent) if !allow_deferred => {
                return Err(Error(format!("Missing parent {parent}")));
            }
            _ => roots.push(i),
        }
    }
    let compare = |a: &usize, b: &usize| {
        entities[*a]
            .order
            .cmp(&entities[*b].order)
            .then_with(|| entities[*a].id.cmp(&entities[*b].id))
    };
    roots.sort_unstable_by(compare);
    for group in &mut children {
        group.sort_unstable_by(compare);
    }
    let mut pending: VecDeque<_> = roots.into();
    let mut ordered = Vec::with_capacity(entities.len());
    while let Some(i) = pending.pop_front() {
        ordered.push(i);
        pending.extend(children[i].iter().copied());
    }
    ensure(ordered.len() == entities.len(), "Cyclic entity hierarchy")?;
    Ok(ordered)
}

/// serde's normal map decoder silently overwrites duplicate resource IDs.
fn unique_map<'de, D, T>(deserializer: D) -> std::result::Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a map with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                if result.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom(format!("Duplicate key {key}")));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}

struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON data with unique object keys")
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("Non-finite number"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = access.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "Duplicate nested key {key}"
                        )));
                    }
                    values.insert(key, access.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        d.deserialize_any(Visitor)
    }
}
fn unique_values<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, Value>, D::Error> {
    Ok(unique_map::<D, UniqueValue>(d)?
        .into_iter()
        .map(|(key, value)| (key, value.0))
        .collect())
}
fn optional_unique_value<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Ok(Some(UniqueValue::deserialize(d)?.0))
}
