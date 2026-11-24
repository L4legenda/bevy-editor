//! Non-destructive levels applied over immutable baked map texels.
use super::{BakeMap, mesh};
use crate::editor::{
    model_document::MaterialSlot,
    model_paint_data::{map_fixes::{Fixes, Plane}, mask::BakedInputs},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{ops::Deref, sync::{Arc, OnceLock}};

const FIELD: &str = "bevy_editor_bake_levels";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Field {
    Black,
    White,
    Gamma,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor::ui::model_viewer::paint) struct Levels {
    pub black: u16,
    pub white: u16,
    pub gamma: u16,
}

impl Default for Levels {
    fn default() -> Self {
        Self { black: 0, white: 100, gamma: 100 }
    }
}

impl Levels {
    pub fn shift(&mut self, field: Field, amount: i8) {
        let (value, low, high) = match field {
            Field::Black => (&mut self.black, 0, 100),
            Field::White => (&mut self.white, 0, 100),
            Field::Gamma => (&mut self.gamma, 10, 400),
        };
        *value = (*value as i16 + i16::from(amount)).clamp(low, high) as u16;
    }

    fn table(self) -> [u8; 256] {
        let black = self.black as f32 / 100.;
        let white = self.white as f32 / 100.;
        let gamma = self.gamma as f32 / 100.;
        std::array::from_fn(|i| {
            let normalized = ((i as f32 / 255. - black) / (white - black).max(1.0e-4)).clamp(0., 1.);
            (normalized.powf(1. / gamma) * 255.).round() as u8
        })
    }

    fn valid(self) -> bool {
        self.black <= 100 && self.white <= 100 && (10..=400).contains(&self.gamma)
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn supports(map: BakeMap) -> bool {
    map.supports_fix() && map != BakeMap::ProjectedNormal
}

pub(super) fn metadata(slot: &MaterialSlot) -> Option<&Value> {
    slot.source["extras"].get(FIELD)
}

fn read(value: Option<&Value>) -> Vec<(BakeMap, Levels)> {
    value
        .and_then(Value::as_array)
        .filter(|entries| entries.len() <= BakeMap::ALL.len())
        .into_iter()
        .flatten()
        .filter_map(|entry| <(BakeMap, Levels)>::deserialize(entry).ok())
        .filter(|&(map, value)| supports(map) && value.valid() && value != Levels::default())
        .collect()
}

pub(in crate::editor::ui::model_viewer::paint) fn get(slot: &MaterialSlot, map: BakeMap) -> Levels {
    read(metadata(slot)).into_iter()
        .find_map(|(key, levels)| (key == map).then_some(levels))
        .unwrap_or_default()
}

pub(super) fn set(slot: &mut MaterialSlot, map: BakeMap, levels: Levels) -> Result<(), String> {
    let mut settings = read(metadata(slot));
    settings.retain(|(key, _)| *key != map);
    if levels != Levels::default() {
        settings.push((map, levels));
    }
    if settings.is_empty() {
        if let Some(source) = slot.source.as_object_mut()
            && let Some(extras) = source.get_mut("extras").and_then(Value::as_object_mut)
        {
            extras.remove(FIELD);
            if extras.is_empty() {
                source.remove("extras");
            }
        }
        return Ok(());
    }
    if slot.source["extras"].is_null() {
        slot.source["extras"] = serde_json::json!({});
    }
    let extras = slot.source["extras"].as_object_mut().ok_or("Некорректные данные материала GLB")?;
    extras.insert(FIELD.into(), serde_json::to_value(settings).map_err(|e| e.to_string())?);
    Ok(())
}

/// The disk cache stays raw. Every preview, graph and export uses this version.
pub(in crate::editor::ui::model_viewer::paint) struct Maps {
    pub(super) raw: Arc<mesh::Maps>,
    metadata: Option<Value>,
    tables: Vec<(BakeMap, [u8; 256])>,
    fixes: Option<Arc<Fixes>>,
    inputs: OnceLock<Arc<BakedInputs>>,
}

impl Deref for Maps {
    type Target = mesh::Maps;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl Maps {
    pub(super) fn new(raw: Arc<mesh::Maps>, material: Option<&MaterialSlot>) -> Self {
        let settings = material.and_then(metadata);
        Self {
            raw,
            metadata: settings.cloned(),
            tables: read(settings).into_iter().map(|(map, levels)| (map, levels.table())).collect(),
            fixes: material.map(|slot| slot.bake_fixes.clone()),
            inputs: OnceLock::new(),
        }
    }

    pub(super) fn matches(&self, material: Option<&MaterialSlot>) -> bool {
        self.metadata.as_ref() == material.and_then(metadata)
            && match (self.fixes.as_ref(), material) {
                (Some(fixes), Some(material)) => Arc::ptr_eq(fixes, &material.bake_fixes),
                (None, None) => true,
                _ => false,
            }
    }

    pub(super) fn has_fixes(&self, fixes: &Arc<Fixes>) -> bool {
        self.fixes.as_ref().is_some_and(|current| Arc::ptr_eq(current, fixes))
    }

    pub(in crate::editor::ui::model_viewer::paint) fn same_background(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.raw, &other.raw) && self.metadata == other.metadata
    }

    fn table(&self, map: BakeMap) -> Option<&[u8; 256]> {
        self.tables.iter().find_map(|(key, table)| (*key == map).then_some(table))
    }

    fn correction(&self, map: BakeMap) -> Correction<'_> {
        let fix = self.fixes.as_deref().into_iter().flatten()
            .find_map(|(key, plane)| (*key == map).then_some(plane.as_ref()));
        Correction { size: self.size, table: self.table(map), fix }
    }

    pub fn bytes(&self) -> usize {
        self.raw.bytes() + self.fixes.as_deref().into_iter().flatten()
            .map(|(_, plane)| plane.pixels.len()).sum::<usize>()
    }

    pub fn rgba(&self, map: BakeMap) -> Vec<u8> {
        self.resolve_rgba(map, self.correction(map))
    }

    pub(in crate::editor::ui::model_viewer::paint) fn rgba_before_fixes(&self, map: BakeMap) -> Vec<u8> {
        self.resolve_rgba(map, Correction { fix: None, ..self.correction(map) })
    }

    fn resolve_rgba(&self, map: BakeMap, correction: Correction<'_>) -> Vec<u8> {
        let mut pixels = self.raw.rgba(map);
        if correction.active() {
            for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
                if map == BakeMap::ProjectedNormal {
                    let normal = correction.normal(index, [pixel[0], pixel[1], pixel[2]]);
                    pixel[..3].copy_from_slice(&normal);
                } else {
                    let value = correction.scalar(index, pixel[0]);
                    pixel[..3].fill(value);
                }
            }
        }
        pixels
    }

    pub(in crate::editor::ui::model_viewer::paint) fn projected_tangent_normals(
        &self,
    ) -> Option<impl Fn(usize) -> Option<[u8; 3]> + '_> {
        let raw = self.raw.projected_tangent_normals()?;
        let correction = self.correction(BakeMap::ProjectedNormal);
        Some(move |index| raw(index).map(|normal| correction.normal(index, normal)))
    }

    pub(in crate::editor::ui::model_viewer::paint) fn graph_plane(&self, map: BakeMap) -> Option<impl Fn(usize) -> u8 + '_> {
        self.graph_component(map, 0)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn graph_component(
        &self,
        map: BakeMap,
        channel: usize,
    ) -> Option<impl Fn(usize) -> u8 + '_> {
        let raw = self.raw.graph_component(map, channel)?;
        let correction = self.correction(map);
        let normal = (map == BakeMap::ProjectedNormal && correction.active())
            .then(|| self.projected_tangent_normals()).flatten();
        Some(move |pixel| {
            if let Some(normal) = &normal {
                normal(pixel).map_or_else(|| raw(pixel), |value| value[channel])
            } else {
                correction.scalar(pixel, raw(pixel))
            }
        })
    }

    pub fn generator_inputs(&self) -> &Arc<BakedInputs> {
        let corrections = mesh::GENERATOR_MAPS.map(|map| map.map(|map| self.correction(map)));
        if !corrections.iter().flatten().any(Correction::active) {
            return self.raw.generator_inputs();
        }
        self.inputs.get_or_init(|| {
            self.raw.build_generator_inputs(|channel, pixel, value| {
                corrections[channel].map_or(value, |correction| correction.scalar(pixel, value))
            })
        })
    }

    pub fn has_generator_inputs(&self, existing: Option<&Arc<BakedInputs>>) -> bool {
        if let Some(current) = self.inputs.get() {
            return existing.is_some_and(|existing| Arc::ptr_eq(existing, current));
        }
        !mesh::GENERATOR_MAPS.into_iter().flatten().any(|map| self.correction(map).active())
            && self.raw.has_generator_inputs(existing)
    }
}

#[derive(Clone, Copy)]
struct Correction<'a> {
    size: u32,
    table: Option<&'a [u8; 256]>,
    fix: Option<&'a Plane>,
}

impl Correction<'_> {
    fn active(&self) -> bool {
        self.table.is_some() || self.fix.is_some()
    }

    fn scalar(self, index: usize, value: u8) -> u8 {
        let value = self.table.map_or(value, |table| table[value as usize]);
        self.fix.map_or(value, |fix| {
            let (paint, coverage) = fix.sample(index, self.size);
            (value as f32 * (1. - coverage) + paint).round() as u8
        })
    }

    fn normal(self, index: usize, normal: [u8; 3]) -> [u8; 3] {
        let Some(fix) = self.fix else { return normal };
        let (_, coverage) = fix.sample(index, self.size);
        if coverage == 0. {
            return normal;
        }
        let [x, y, _] = normal.map(|v| (v as f32 / 255. * 2. - 1.) * (1. - coverage));
        let z = (1. - x * x - y * y).max(0.).sqrt();
        [x, y, z].map(|v| ((v * 0.5 + 0.5) * 255.).round() as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    #[test]
    fn mesh_map_levels_resolve_preview_graph_and_generator_from_the_same_texels() {
        let mut slot = MaterialSlot {
            bake_fixes: Default::default(),
            material: Default::default(),
            source: serde_json::json!({"extras":{"keep":true}}),
            paint: None,
            authored: None,
        };
        let map = BakeMap::Occlusion;
        let settings = Levels { black: 20, white: 80, gamma: 200 };
        set(&mut slot, map, settings).unwrap();
        assert_eq!(get(&slot, map), settings);
        let mut sample = [100; mesh::COMPONENTS];
        sample[0] = 128;
        let raw = Arc::new(mesh::Maps {
            size: 1,
            filled: 1,
            values: vec![sample],
            covered: vec![1],
            projection: Some(vec![[204, 128, 229, 64, 255]]),
            hits: 0,
            filled_misses: 0,
            cage_fit: None,
            gpu_rays: false,
            gpu_projection: false,
            gpu_denoise: false,
            gpu_padding: false,
            selection: super::super::MapSelection::ALL,
            edge_reach: 1.,
            inputs: OnceLock::new(),
            custom: vec![],
        });
        let corrected = Maps::new(raw.clone(), Some(&slot));
        let value = settings.table()[128];
        assert!(value > 175 && value < 185);
        assert_eq!(corrected.rgba(map), [value, value, value, 255]);
        assert_eq!(corrected.graph_component(map, 2).unwrap()(0), value);
        assert_eq!(corrected.generator_inputs().texels[0][0], value);
        assert_eq!(corrected.generator_inputs().texels[0][1], 100);
        assert_eq!(raw.rgba(map), [128, 128, 128, 255]);

        let plane = Arc::new(Plane { size: 1, pixels: vec![16, 128] });
        slot.bake_fixes = Arc::new(vec![(map, plane), (
            BakeMap::ProjectedNormal, Arc::new(Plane { size: 1, pixels: vec![0, 255] }),
        )]);
        assert!(!corrected.matches(Some(&slot)));
        let painted = Maps::new(raw.clone(), Some(&slot));
        let mixed = (value as f32 * (127. / 255.) + 16.).round() as u8;
        assert_eq!(painted.rgba(map), [mixed, mixed, mixed, 255]);
        assert_eq!(painted.generator_inputs().texels[0][0], mixed);
        assert_eq!(painted.graph_plane(map).unwrap()(0), mixed);
        assert_eq!(painted.rgba(BakeMap::ProjectedNormal), [128, 128, 255, 255]);
        assert_eq!(painted.projected_tangent_normals().unwrap()(0), Some([128, 128, 255]));
        assert_eq!(painted.graph_component(BakeMap::ProjectedNormal, 2).unwrap()(0), 255);
        assert_eq!(raw.rgba(BakeMap::ProjectedNormal), [204, 128, 229, 255]);

        slot.bake_fixes = Default::default();
        set(&mut slot, map, Levels::default()).unwrap();
        assert!(metadata(&slot).is_none());
        assert_eq!(slot.source["extras"]["keep"], true);
        let reset = Maps::new(raw.clone(), Some(&slot));
        assert!(Arc::ptr_eq(reset.generator_inputs(), raw.generator_inputs()));
    }
}
