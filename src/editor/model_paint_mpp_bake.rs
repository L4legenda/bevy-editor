//! Convert MPP's normalized bake data into Bevy Editor's document settings.
use super::{MPP_BAKE_MAPS, Result};
use crate::editor::{
    model_bake_settings::{self, Quality, Settings},
    model_cage_data::{Edit, Edits, Scales},
    model_document::Object,
    model_paint_data::custom::{self, Kind, Recipe},
    ui::model_viewer::paint::import_bake_map_bits,
};
use bevy::prelude::Vec3;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct Bake {
    settings: Settings,
    pub recipes: Vec<Recipe>,
    pub cage: Edits,
}

impl Bake {
    pub fn read(
        path: &Path,
        object: &Object,
        mesh: &Value,
        bake: &Value,
        warnings: &mut Vec<String>,
    ) -> Self {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for corner in object.geometry.faces.iter().flat_map(|face| &face.corners) {
            let point = object.geometry.positions[corner.vertex];
            lo = lo.min(point);
            hi = hi.max(point);
        }
        let extent = hi - lo;
        let normalized_scale = mesh["scale"].as_f64().unwrap_or(1.);
        let distance_factor = 100. / (normalized_scale * f64::from(extent.length().max(1e-5)));
        let mut rounded_distance = false;
        let mut distance = |key| {
            let percent = bake[key].as_f64().unwrap_or(0.05) * distance_factor;
            rounded_distance |= !(1. ..=100.).contains(&percent.round());
            percent.round().clamp(1., 100.) as u8
        };
        let front = distance("front");
        let rear = distance("rear");
        if rounded_distance {
            warnings.push("Дальности проекции ограничены диапазоном 1–100%".into());
        }
        let mask = bake["maps"].as_u64().unwrap_or(u64::MAX);
        let map_bits = import_bake_map_bits(
            MPP_BAKE_MAPS
                .iter()
                .copied()
                .take(7)
                .enumerate()
                .filter_map(|(i, map)| (mask & (1 << i) != 0).then_some(map)),
        );
        let settings = Settings {
            quality: match bake["quality"].as_str() {
                Some("Draft") => Quality::Draft,
                Some("High") => Quality::High,
                Some("Ultra") => Quality::Ultra,
                _ => Quality::Standard,
            },
            high_poly: source_file(path, &bake["high_poly"], "high-poly", warnings),
            cage: source_file(path, &bake["cage"], "cage", warnings),
            map_bits,
            front,
            rear,
            match_names: bake["match_by_name"].as_bool().unwrap_or(true),
            fill_misses: bake["fill_misses"].as_bool().unwrap_or(true),
            ..Settings::default()
        };
        if bake["average_normals"].as_bool() == Some(false)
            || bake["ignore_backfaces"].as_bool() == Some(false)
            || bake["id_source"]
                .as_str()
                .is_some_and(|id| id != "MeshParts")
        {
            warnings.push(
                "Для направления лучей и источника ID используются настройки Bevy Editor".into(),
            );
        }
        let unit = f64::from((extent.max_element() * 0.5).max(1e-6));
        let center =
            Vec3::from_array(serde_json::from_value(mesh["center"].clone()).unwrap_or([0.; 3]));
        let recipes = custom_maps(
            &bake["custom"],
            100. / (unit * normalized_scale),
            (center - (lo + hi) * 0.5)
                .to_array()
                .map(|axis| f64::from(axis) * 100. / unit),
            warnings,
        );
        Self {
            settings,
            recipes,
            cage: cage_edits(object, &bake["cage_paint"], warnings),
        }
    }

    pub fn write(&self, root: &mut Value, model: &Path) -> Result<()> {
        model_bake_settings::write(root, model, &self.settings)?;
        custom::write_document(root, &self.recipes)
    }
}

fn source_file(
    project: &Path,
    value: &Value,
    label: &str,
    warnings: &mut Vec<String>,
) -> Option<PathBuf> {
    let original = Path::new(value.as_str().filter(|path| !path.is_empty())?);
    let resolved = project.parent().unwrap_or(Path::new("")).join(original);
    if resolved.extension().is_some_and(|ext| {
        ["glb", "obj", "fbx"]
            .iter()
            .any(|format| ext.eq_ignore_ascii_case(format))
    }) && let Ok(path) = resolved.canonicalize()
        && path.is_file()
    {
        return Some(path);
    }
    warnings.push(format!(
        "{label} · исходный файл недоступен для запекания: {}",
        original.display()
    ));
    None
}

fn custom_maps(
    value: &Value,
    linear_factor: f64,
    origin: [f64; 3],
    warnings: &mut Vec<String>,
) -> Vec<Recipe> {
    let mut recipes = Vec::new();
    for (index, row) in value.as_array().into_iter().flatten().enumerate() {
        if row.is_null() {
            continue;
        }
        let kind = match row["kind"].as_str() {
            Some("Flow") => Kind::Flow,
            Some("Exposure") => Kind::Exposure,
            Some("Deposit") => Kind::Deposit,
            Some("Contact") => Kind::Contact,
            Some("Point") => Kind::Point,
            _ => {
                warnings.push(format!(
                    "Неизвестный тип пользовательской карты {}",
                    index + 1
                ));
                continue;
            }
        };
        let mut recipe = Recipe::new(kind, &recipes);
        recipe.id = row["id"].as_u64().unwrap_or(index as u64 + 1);
        if recipe.id == 0
            || recipes.len() == custom::MAX_MAPS
            || recipes.iter().any(|existing| existing.id == recipe.id)
        {
            warnings.push(format!("Пропущена пользовательская карта {}", index + 1));
            continue;
        }
        if let Some(name) = row["name"].as_str().filter(|name| !name.trim().is_empty()) {
            recipe.name = name[..name.floor_char_boundary(128)].to_owned();
        }
        let mut limited = false;
        let mut quantize = |value: f64, min: i16, max: i16| {
            let value = value.round();
            limited |= !(f64::from(min)..=f64::from(max)).contains(&value);
            value.clamp(f64::from(min), f64::from(max)) as i16
        };
        if let Some(distance) = row["distance"].as_f64() {
            recipe.distance = quantize(distance * linear_factor, 1, 800);
        }
        if let Some(azimuth) = row["azimuth"].as_f64() {
            recipe.azimuth = ((azimuth + 180.).rem_euclid(360.) - 180.).round() as i16;
        }
        for (key, value, min, max, factor) in [
            ("elevation", &mut recipe.elevation, -90, 90, 1.),
            ("spread", &mut recipe.spread, 0, 90, 1.),
            ("strength", &mut recipe.strength, 1, 800, 100.),
            ("wander", &mut recipe.wander, 0, 90, 1.),
        ] {
            if let Some(source) = row[key].as_f64() {
                *value = quantize(source * factor, min, max);
            }
        }
        recipe.point = std::array::from_fn(|axis| {
            let point = row["point"][axis].as_f64().unwrap_or(0.);
            quantize(point * linear_factor + origin[axis], -200, 200)
        });
        if limited {
            warnings.push(format!(
                "«{}» · значения карты ограничены диапазоном редактора",
                recipe.name
            ));
        }
        recipes.push(recipe);
    }
    recipes
}

fn cage_edits(object: &Object, value: &Value, warnings: &mut Vec<String>) -> Edits {
    let Some(scales) = value.as_array() else {
        return Edits::new();
    };
    let mesh = &object.geometry;
    if scales.len() != mesh.positions.len() || scales.iter().any(|scale| scale.as_f64().is_none()) {
        warnings.push("Правки cage не соответствуют вершинам сетки MPP".into());
        return Edits::new();
    }
    // GLB welds coincident vertices at UV seams, so store one multiplier per position.
    let key = |point: Vec3| {
        point
            .to_array()
            .map(|x| if x == 0. { 0 } else { x.to_bits() })
    };
    let mut groups = HashMap::new();
    for (&point, scale) in mesh.positions.iter().zip(scales) {
        let group = groups.entry(key(point)).or_insert((0., 0u32));
        group.0 += scale.as_f64().unwrap_or(1.).clamp(0.05, 8.);
        group.1 += 1;
    }
    let values: Vec<_> = mesh
        .positions
        .iter()
        .map(|&point| {
            let (sum, count) = groups[&key(point)];
            (sum / f64::from(count)) as f32
        })
        .collect();
    if values.iter().all(|&scale| scale == 1.) {
        return Edits::new();
    }
    Edits::from([(
        object.id,
        Edit {
            mesh: mesh.clone(),
            scales: Scales {
                values: Arc::new(values),
            },
        },
    )])
}
