//! UDIM numbering uses U rightward and V upward. Mesh UVs are texture-down.
use super::{MaterialSlot, Object};
use crate::editor::model_mesh::Face;
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[path = "model_udim_merge.rs"]
mod merge;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::editor) struct Tile(u32);

impl Tile {
    pub fn indices(self) -> [u32; 2] {
        let index = self.0 - 1001;
        [index % 10, index / 10]
    }

    pub fn at_uv(uv: Vec2) -> Option<Self> {
        let (u, v) = (uv.x.floor(), (1. - uv.y).floor());
        ((0. ..10.).contains(&u) && (0. ..100.).contains(&v))
            .then(|| Self(1001 + u as u32 + 10 * v as u32))
    }

    pub fn number(self) -> u32 {
        self.0
    }

    pub fn offset(self) -> Vec2 {
        let index = self.0 - 1001;
        Vec2::new((index % 10) as f32, -((index / 10) as f32))
    }

    pub fn contains(self, uv: Vec2) -> bool {
        let local = uv - self.offset();
        local.is_finite() && local.min_element() >= -0.001 && local.max_element() <= 1.001
    }

    fn from_face(face: &Face) -> Option<Self> {
        let origin = face.corners.first()?.uv;
        let area = face
            .corners
            .iter()
            .zip(face.corners.iter().cycle().skip(1))
            .map(|(a, b)| (a.uv - origin).perp_dot(b.uv - origin))
            .sum::<f32>();
        if !area.is_finite() || area.abs() <= 1e-8 {
            return None;
        }
        let center =
            face.corners.iter().map(|corner| corner.uv).sum::<Vec2>() / face.corners.len() as f32;
        let tile = Self::at_uv(center)?;
        face.corners
            .iter()
            .all(|corner| tile.contains(corner.uv))
            .then_some(tile)
    }
}

impl MaterialSlot {
    pub fn udim(&self) -> Option<Tile> {
        let number = self.source["extras"]["bevy_editor_udim"].as_u64()?;
        (1001..=2000)
            .contains(&number)
            .then_some(Tile(number as u32))
    }
}

impl Object {
    pub fn uv_offset(&self, slot: usize) -> Vec2 {
        self.materials
            .get(slot)
            .and_then(MaterialSlot::udim)
            .map_or(Vec2::ZERO, Tile::offset)
    }

    pub fn udim_tiles(&self) -> impl Iterator<Item = Tile> + '_ {
        self.materials.iter().filter_map(MaterialSlot::udim)
    }

    pub fn preserves_udim_assignment(&self, mesh: &crate::editor::model_mesh::Geometry) -> bool {
        if self.udim_tiles().next().is_none() {
            return true;
        }
        let assigned = self
            .materials
            .iter()
            .map(MaterialSlot::udim)
            .collect::<Vec<_>>();
        mesh.faces.iter().all(|face| {
            assigned
                .get(face.material)
                .copied()
                .flatten()
                .is_none_or(|tile| face.corners.iter().all(|corner| tile.contains(corner.uv)))
        })
    }
}

/// Translate selected faces by whole tiles, preserving their texture-local coordinates.
/// Prefer an existing compatible set, otherwise keep the original texture data separate.
pub(in crate::editor) fn relocate(
    original: &Object,
    islands: &[usize],
    selected: &BTreeSet<usize>,
    delta: Vec2,
) -> Result<Object, String> {
    if original.geometry.faces.len() != islands.len()
        || !delta.is_finite()
        || delta != delta.round()
    {
        return Err("Некорректный перенос UV-островов".into());
    }
    let mut moves = BTreeMap::<usize, Vec<usize>>::new();
    let mut remaining = vec![0; original.materials.len()];
    for (index, (face, island)) in original.geometry.faces.iter().zip(islands).enumerate() {
        if selected.contains(island) {
            moves.entry(face.material).or_default().push(index);
        } else if let Some(count) = remaining.get_mut(face.material) {
            *count += 1;
        }
    }
    let stationary: Vec<_> = (0..original.materials.len())
        .filter(|slot| remaining[*slot] > 0 && !moves.contains_key(slot))
        .collect();
    let mut retired = BTreeSet::new();
    let mut object = original.clone();
    for (slot, faces) in moves {
        let tile = original
            .materials
            .get(slot)
            .and_then(MaterialSlot::udim)
            .ok_or("Перенос между плитками доступен для наборов UDIM")?;
        let destination = Tile::at_uv(tile.offset() + delta + Vec2::splat(0.5))
            .ok_or("Плитка назначения выходит за пределы UDIM")?;
        if destination == tile {
            continue;
        }
        if faces.iter().any(|&index| {
            original.geometry.faces[index]
                .corners
                .iter()
                .any(|corner| !tile.contains(corner.uv))
        }) {
            return Err("Часть острова находится за пределами исходной плитки".into());
        }
        let merged = stationary
            .iter()
            .copied()
            .filter(|&target| object.materials[target].udim() == Some(destination))
            .find_map(|target| {
                merge::surfaces(original, &object, slot, target, &faces)
                    .map(|surface| (target, surface))
            });
        let new_slot = if let Some((target, surface)) = merged {
            object.materials[target].paint = surface;
            if remaining[slot] == 0 {
                retired.insert(slot);
            }
            target
        } else if remaining[slot] == 0 {
            slot
        } else {
            let new_slot = object.materials.len();
            object.materials.push(original.materials[slot].clone());
            new_slot
        };
        object.materials[new_slot].source["extras"]["bevy_editor_udim"] =
            serde_json::json!(destination.number());
        for index in faces {
            let face = &mut object.geometry.faces[index];
            face.material = new_slot;
            for corner in &mut face.corners {
                corner.uv = corner.uv - tile.offset() + destination.offset();
            }
        }
    }
    if !retired.is_empty() {
        let mut remap = Vec::with_capacity(object.materials.len());
        let mut index = 0;
        object.materials.retain(|_| {
            let keep = !retired.contains(&remap.len());
            remap.push(index);
            index += usize::from(keep);
            keep
        });
        for face in &mut object.geometry.faces {
            if let Some(&slot) = remap.get(face.material) {
                face.material = slot;
            }
        }
    }
    Ok(object)
}

/// Restore saved tile metadata and optionally detect tiles in an external mesh.
pub(in crate::editor) fn import(object: &mut Object, detect_tiles: bool) {
    let original_slots = object.materials.len();
    let mut faces_by_slot = vec![Vec::new(); original_slots];
    for (index, face) in object.geometry.faces.iter().enumerate() {
        if let Some(faces) = faces_by_slot.get_mut(face.material) {
            faces.push(index);
        }
    }
    let mut detected = Vec::with_capacity(original_slots);
    for (slot, faces) in faces_by_slot.iter().enumerate() {
        let material = &object.materials[slot];
        if let Some(tile) = material.udim() {
            for &index in faces {
                for corner in &mut object.geometry.faces[index].corners {
                    corner.uv += tile.offset();
                }
            }
            detected.push(None);
        } else if !detect_tiles
            || material.paint.is_some()
            || material.source["extras"].as_object().is_none()
                && !material.source["extras"].is_null()
        {
            detected.push(None);
        } else {
            detected.push(
                faces
                    .iter()
                    .map(|&index| Tile::from_face(&object.geometry.faces[index]))
                    .collect::<Option<Vec<_>>>(),
            );
        }
    }
    if !detected
        .iter()
        .flatten()
        .flatten()
        .any(|tile| tile.number() != 1001)
        && !object
            .materials
            .iter()
            .any(|material| material.udim().is_some())
    {
        return;
    }
    for (slot, tiles) in detected.into_iter().enumerate() {
        let Some(tiles) = tiles else { continue };
        let mut groups: BTreeMap<Tile, Vec<usize>> = BTreeMap::new();
        for (index, tile) in faces_by_slot[slot].iter().copied().zip(tiles) {
            groups.entry(tile).or_default().push(index);
        }
        let source = object.materials[slot].clone();
        for (ordinal, (tile, faces)) in groups.into_iter().enumerate() {
            let mut material = source.clone();
            material.source["extras"]["bevy_editor_udim"] = serde_json::json!(tile.number());
            let new_slot = if ordinal == 0 {
                object.materials[slot] = material;
                slot
            } else {
                let index = object.materials.len();
                object.materials.push(material);
                index
            };
            for index in faces {
                object.geometry.faces[index].material = new_slot;
            }
        }
    }
}
