//! Authored cage offsets keyed by the GLB node and its surface positions.
use super::model_document::{MeshData, Object};
use bevy::prelude::Vec3;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

const FIELD: &str = "bevy_editor_cage_paint";
type Position = [u32; 3];
pub(super) type Edits = BTreeMap<usize, Edit>;

#[derive(Clone, Debug)]
pub(in crate::editor) struct Scales {
    pub values: Arc<Vec<f32>>,
}

impl PartialEq for Scales {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.values, &other.values)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub mesh: MeshData,
    pub scales: Scales,
}

impl PartialEq for Edit {
    fn eq(&self, other: &Self) -> bool {
        self.mesh == other.mesh
            && (self.scales == other.scales || self.scales.values == other.scales.values)
    }
}

impl Edit {
    pub fn matches(&self, mesh: &MeshData) -> bool {
        self.scales.values.len() == mesh.positions.len()
            && (self.mesh.shares_storage_with(mesh)
                || (self.mesh.positions == mesh.positions
                    && self.mesh.faces.len() == mesh.faces.len()
                    && self.mesh.faces.iter().zip(&mesh.faces).all(|(a, b)| {
                        a.corners
                            .iter()
                            .map(|corner| corner.vertex)
                            .eq(b.corners.iter().map(|corner| corner.vertex))
                    })))
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u8,
    surface: String,
    offsets: Vec<(Position, f32)>,
}

fn position(point: Vec3) -> Position {
    point
        .to_array()
        .map(|value| if value == 0. { 0 } else { value.to_bits() })
}

// GLB serialization groups polygons by material and welds exact positions on load.
// Canonicalize polygon order and starting corners while retaining winding.
fn surface(mesh: &MeshData) -> String {
    let mut polygons: Vec<_> = mesh
        .faces
        .iter()
        .map(|face| {
            let points: Vec<_> = face
                .corners
                .iter()
                .map(|corner| position(mesh.positions[corner.vertex]))
                .collect();
            let start = points
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| *p)
                .map_or(0, |(i, _)| i);
            let mut hash = blake3::Hasher::new();
            for point in points[start..].iter().chain(&points[..start]) {
                for component in point {
                    hash.update(&component.to_le_bytes());
                }
            }
            *hash.finalize().as_bytes()
        })
        .collect();
    polygons.sort_unstable();
    let mut hash = blake3::Hasher::new();
    for polygon in polygons {
        hash.update(&polygon);
    }
    hash.finalize().to_hex().to_string()
}

pub(super) fn read(node: &Value, object: &Object) -> Option<Edit> {
    let mesh = &object.geometry;
    let value = node["extras"].get(FIELD)?;
    let offsets = value["offsets"].as_array()?;
    if offsets.is_empty() || offsets.len() > mesh.positions.len() {
        return None;
    }
    let stored = Stored::deserialize(value).ok()?;
    if stored.version != 1 || stored.surface != surface(mesh) {
        return None;
    }
    let mut offsets = BTreeMap::new();
    for (position, scale) in stored.offsets {
        if !(0.05..=8.).contains(&scale) || offsets.insert(position, scale).is_some() {
            return None;
        }
    }
    let values: Vec<_> = mesh
        .positions
        .iter()
        .map(|&point| offsets.get(&position(point)).copied().unwrap_or(1.))
        .collect();
    values.iter().any(|&scale| scale != 1.).then(|| Edit {
        mesh: mesh.clone(),
        scales: Scales {
            values: Arc::new(values),
        },
    })
}

pub(super) fn write(
    node: &mut serde_json::Map<String, Value>,
    object: &Object,
    edit: Option<&Edit>,
) -> Result<(), String> {
    let mut offsets = BTreeMap::new();
    if let Some(edit) = edit.filter(|edit| edit.matches(&object.geometry)) {
        for (index, &scale) in edit.scales.values.iter().enumerate() {
            if scale != 1. {
                offsets.insert(position(object.geometry.positions[index]), scale);
            }
        }
    }
    if offsets.is_empty() {
        if let Some(extras) = node.get_mut("extras").and_then(Value::as_object_mut) {
            extras.remove(FIELD);
        }
    } else {
        let entry = node
            .entry("extras")
            .or_insert_with(|| serde_json::json!({}));
        if entry.is_null() {
            *entry = serde_json::json!({});
        }
        let extras = entry
            .as_object_mut()
            .ok_or("Некорректные дополнительные данные узла GLB")?;
        let value = serde_json::to_value(Stored {
            version: 1,
            surface: surface(&object.geometry),
            offsets: offsets.into_iter().collect(),
        })
        .map_err(|error| error.to_string())?;
        extras.insert(FIELD.into(), value);
    }
    Ok(())
}
