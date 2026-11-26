//! Keep data referenced by the game and compact the embedded GLB buffer.

use serde_json::Value;
use std::collections::BTreeSet;

const EDITOR_EXTRAS: &[&str] = &[
    "bevy_editor_active_paint_set",
    "bevy_editor_bake_fixes",
    "bevy_editor_bake_levels",
    "bevy_editor_bake_settings",
    "bevy_editor_cage_paint",
    "bevy_editor_id_colors",
    "bevy_editor_model_camera",
    "bevy_editor_paint",
    "bevy_editor_paint_post",
    "bevy_editor_paint_preview",
    "bevy_editor_paint_uv",
    "bevy_editor_preview_shader",
    "bevy_editor_source_parts",
    "bevy_editor_topology",
    "bevy_editor_udim",
];

pub(super) fn strip_editor_extras(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(extras) = object.get_mut("extras").and_then(Value::as_object_mut) {
                for &key in EDITOR_EXTRAS {
                    extras.remove(key);
                }
                if extras.is_empty() {
                    object.remove("extras");
                }
            }
            for (key, child) in object {
                if key != "extras" {
                    strip_editor_extras(child);
                }
            }
        }
        Value::Array(children) => {
            for child in children {
                strip_editor_extras(child);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum Table {
    Mesh,
    Material,
    Texture,
    Image,
    Sampler,
    Accessor,
    View,
}

impl Table {
    // Each table can reference only tables that follow it in this order.
    const ORDER: [Self; 7] = [
        Self::Mesh,
        Self::Material,
        Self::Texture,
        Self::Image,
        Self::Sampler,
        Self::Accessor,
        Self::View,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Mesh => "meshes",
            Self::Material => "materials",
            Self::Texture => "textures",
            Self::Image => "images",
            Self::Sampler => "samplers",
            Self::Accessor => "accessors",
            Self::View => "bufferViews",
        }
    }
}

type Visit<'a> = dyn FnMut(Table, &mut Value) -> Result<(), String> + 'a;

pub(super) fn compact(root: &mut Value, bin: &[u8]) -> Result<Vec<u8>, String> {
    if !root.is_object() {
        return Err("Некорректное описание GLB".into());
    }
    strip_editor_extras(root);
    // Opaque extensions can refer to table indices, JSON pointers or raw buffer offsets.
    if !can_remap(root) {
        return Ok(bin.to_vec());
    }

    let mut live: [BTreeSet<usize>; Table::ORDER.len()] = std::array::from_fn(|_| BTreeSet::new());
    visit_roots(root, &mut |table, value| {
        live[table as usize].insert(index(value, table.key())?);
        Ok(())
    })?;
    let mut remap: [Vec<Option<usize>>; Table::ORDER.len()] = std::array::from_fn(|_| Vec::new());
    for table in Table::ORDER {
        let ids = std::mem::take(&mut live[table as usize]);
        let key = table.key();
        if !ids.is_empty() && root.get(key).is_none() {
            return Err(format!("В GLB отсутствует таблица {key}"));
        }
        let Some(entries) = root.get_mut(key) else {
            continue;
        };
        let entries = entries
            .as_array_mut()
            .ok_or_else(|| format!("Некорректная таблица GLB {key}"))?;
        if ids.last().is_some_and(|&id| id >= entries.len()) {
            return Err(format!("Некорректная ссылка на таблицу GLB {key}"));
        }
        let mut map = vec![None; entries.len()];
        let mut kept = Vec::with_capacity(ids.len());
        for id in ids {
            let mut item = entries[id].take();
            visit_table(table, &mut item, &mut |target, value| {
                live[target as usize].insert(index(value, target.key())?);
                Ok(())
            })?;
            map[id] = Some(kept.len());
            kept.push(item);
        }
        *entries = kept;
        remap[table as usize] = map;
    }

    let mut rewrite = |table: Table, value: &mut Value| {
        let key = table.key();
        let index = index(value, key)?;
        let new = remap[table as usize]
            .get(index)
            .and_then(|entry| *entry)
            .ok_or_else(|| format!("Некорректная ссылка на таблицу GLB {key}"))?;
        *value = Value::from(new);
        Ok(())
    };
    visit_roots(root, &mut rewrite)?;
    for table in Table::ORDER {
        for item in entries(root.get_mut(table.key())) {
            visit_table(table, item, &mut rewrite)?;
        }
    }
    for table in Table::ORDER {
        if root[table.key()].as_array().is_some_and(Vec::is_empty) {
            root.as_object_mut().unwrap().remove(table.key());
        }
    }
    compact_bin(root, bin)
}

fn index(value: &Value, name: &str) -> Result<usize, String> {
    value
        .as_u64()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| format!("Некорректный индекс или размер GLB {name}"))
}

fn entries(value: Option<&mut Value>) -> impl Iterator<Item = &mut Value> {
    value.and_then(Value::as_array_mut).into_iter().flatten()
}

fn reference(value: Option<&mut Value>, table: Table, visit: &mut Visit<'_>) -> Result<(), String> {
    if let Some(value) = value {
        visit(table, value)?;
    }
    Ok(())
}

fn attributes(value: Option<&mut Value>, visit: &mut Visit<'_>) -> Result<(), String> {
    if let Some(values) = value.and_then(Value::as_object_mut) {
        for accessor in values.values_mut() {
            visit(Table::Accessor, accessor)?;
        }
    }
    Ok(())
}

fn visit_roots(root: &mut Value, visit: &mut Visit<'_>) -> Result<(), String> {
    for node in entries(root.get_mut("nodes")) {
        reference(node.get_mut("mesh"), Table::Mesh, visit)?;
        let instancing = node
            .get_mut("extensions")
            .and_then(|extensions| extensions.get_mut("EXT_mesh_gpu_instancing"));
        attributes(
            instancing.and_then(|instance| instance.get_mut("attributes")),
            visit,
        )?;
    }
    for skin in entries(root.get_mut("skins")) {
        reference(skin.get_mut("inverseBindMatrices"), Table::Accessor, visit)?;
    }
    for animation in entries(root.get_mut("animations")) {
        for sampler in entries(animation.get_mut("samplers")) {
            reference(sampler.get_mut("input"), Table::Accessor, visit)?;
            reference(sampler.get_mut("output"), Table::Accessor, visit)?;
        }
    }
    Ok(())
}

fn visit_table(table: Table, item: &mut Value, visit: &mut Visit<'_>) -> Result<(), String> {
    match table {
        Table::Mesh => {
            for primitive in entries(item.get_mut("primitives")) {
                reference(primitive.get_mut("material"), Table::Material, visit)?;
                reference(primitive.get_mut("indices"), Table::Accessor, visit)?;
                attributes(primitive.get_mut("attributes"), visit)?;
                for target in entries(primitive.get_mut("targets")) {
                    attributes(Some(target), visit)?;
                }
                if let Some(extensions) = primitive.get_mut("extensions") {
                    let draco = extensions.get_mut("KHR_draco_mesh_compression");
                    reference(
                        draco.and_then(|record| record.get_mut("bufferView")),
                        Table::View,
                        visit,
                    )?;
                    let variants = extensions.get_mut("KHR_materials_variants");
                    for mapping in entries(variants.and_then(|record| record.get_mut("mappings"))) {
                        reference(mapping.get_mut("material"), Table::Material, visit)?;
                    }
                }
            }
        }
        Table::Material => {
            texture_fields(item, visit)?;
            if let Some(pbr) = item.get_mut("pbrMetallicRoughness") {
                texture_fields(pbr, visit)?;
            }
            if let Some(extensions) = item.get_mut("extensions").and_then(Value::as_object_mut) {
                for extension in extensions.values_mut() {
                    texture_fields(extension, visit)?;
                }
            }
        }
        Table::Texture => {
            reference(item.get_mut("source"), Table::Image, visit)?;
            reference(item.get_mut("sampler"), Table::Sampler, visit)?;
            if let Some(extensions) = item.get_mut("extensions") {
                for key in [
                    "KHR_texture_basisu",
                    "EXT_texture_webp",
                    "EXT_texture_avif",
                    "MSFT_texture_dds",
                ] {
                    let extension = extensions.get_mut(key);
                    reference(
                        extension.and_then(|record| record.get_mut("source")),
                        Table::Image,
                        visit,
                    )?;
                }
            }
        }
        Table::Image => reference(item.get_mut("bufferView"), Table::View, visit)?,
        Table::Accessor => {
            reference(item.get_mut("bufferView"), Table::View, visit)?;
            if let Some(sparse) = item.get_mut("sparse") {
                for kind in ["indices", "values"] {
                    let data = sparse.get_mut(kind);
                    reference(
                        data.and_then(|record| record.get_mut("bufferView")),
                        Table::View,
                        visit,
                    )?;
                }
            }
        }
        Table::Sampler | Table::View => {}
    }
    Ok(())
}

fn texture_fields(item: &mut Value, visit: &mut Visit<'_>) -> Result<(), String> {
    if let Some(properties) = item.as_object_mut() {
        for (key, value) in properties {
            if key.ends_with("Texture") {
                reference(value.get_mut("index"), Table::Texture, visit)?;
            }
        }
    }
    Ok(())
}

fn supported_extension(key: &str) -> bool {
    matches!(
        key,
        "EXT_mesh_gpu_instancing"
            | "EXT_texture_avif"
            | "EXT_texture_webp"
            | "KHR_draco_mesh_compression"
            | "KHR_lights_punctual"
            | "KHR_materials_anisotropy"
            | "KHR_materials_clearcoat"
            | "KHR_materials_diffuse_transmission"
            | "KHR_materials_dispersion"
            | "KHR_materials_emissive_strength"
            | "KHR_materials_ior"
            | "KHR_materials_iridescence"
            | "KHR_materials_pbrSpecularGlossiness"
            | "KHR_materials_sheen"
            | "KHR_materials_specular"
            | "KHR_materials_transmission"
            | "KHR_materials_unlit"
            | "KHR_materials_variants"
            | "KHR_materials_volume"
            | "KHR_mesh_quantization"
            | "KHR_texture_basisu"
            | "KHR_texture_transform"
            | "KHR_xmp_json_ld"
            | "MSFT_texture_dds"
    )
}

fn can_remap(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().all(|(key, child)| match key.as_str() {
            "extras" => true,
            "extensions" => child.as_object().is_some_and(|ext| {
                ext.keys().all(|name| supported_extension(name)) && ext.values().all(can_remap)
            }),
            "extensionsUsed" | "extensionsRequired" => child.as_array().is_some_and(|ext| {
                ext.iter()
                    .all(|name| name.as_str().is_some_and(supported_extension))
            }),
            _ => can_remap(child),
        }),
        Value::Array(children) => children.iter().all(can_remap),
        _ => true,
    }
}

fn compact_bin(root: &mut Value, bin: &[u8]) -> Result<Vec<u8>, String> {
    let mut ranges = Vec::new();
    let limit = root["buffers"][0]
        .get("byteLength")
        .map(|value| index(value, "buffers.byteLength"))
        .transpose()?
        .unwrap_or_default();
    if limit > bin.len() {
        return Err("BIN-блок короче объявленной длины".into());
    }
    let count = root["buffers"].as_array().map_or(0, Vec::len);
    for (id, view) in entries(root.get_mut("bufferViews")).enumerate() {
        let buffer = index(&view["buffer"], "bufferViews.buffer")?;
        if buffer >= count {
            return Err("Отсутствует буфер GLB".into());
        }
        if buffer != 0 {
            continue;
        }
        let start = view
            .get("byteOffset")
            .map(|value| index(value, "bufferViews.byteOffset"))
            .transpose()?
            .unwrap_or_default();
        let size = index(&view["byteLength"], "bufferViews.byteLength")?;
        let end = start
            .checked_add(size)
            .filter(|&end| end <= limit)
            .ok_or("Срез GLB выходит за пределы BIN-блока")?;
        ranges.push((start, end, id));
    }

    // Overlapping views share bytes. Keep the original offset modulo four for accessor alignment.
    ranges.sort_unstable();
    let mut output = Vec::new();
    let mut old_start = 0;
    let mut old_end = 0;
    let mut new_start = 0;
    for (start, end, id) in ranges {
        if start > old_end {
            output.resize(output.len() + (start % 4 + 4 - output.len() % 4) % 4, 0);
            new_start = output.len();
            old_start = start;
            old_end = start;
        }
        if end > old_end {
            output.extend_from_slice(&bin[old_end..end]);
            old_end = end;
        }
        root["bufferViews"][id]["byteOffset"] = Value::from(new_start + (start - old_start));
    }
    if output.is_empty() {
        if count > 1 {
            // Keep the original buffer indices for assets with external buffers.
            return Ok(bin.to_vec());
        }
        root.as_object_mut().unwrap().remove("buffers");
        return Ok(output);
    }
    if count != 0 {
        root["buffers"][0]["byteLength"] = Value::from(output.len());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn game_buffer_compaction_keeps_overlapping_views_and_alignment() {
        let mut root = json!({
            "buffers": [{"byteLength": 32}],
            "bufferViews": [
                {"buffer": 0, "byteLength": 12},
                {"buffer": 0, "byteOffset": 13, "byteLength": 4},
                {"buffer": 0, "byteOffset": 16, "byteLength": 7},
                {"buffer": 0, "byteOffset": 27, "byteLength": 3}
            ],
            "accessors": [{"bufferView": 1}, {"bufferView": 2}, {"bufferView": 3}],
            "meshes": [{"primitives": [{"attributes": {"_CHECK": 0, "_CHECK2": 1, "_CHECK3": 2}}]}],
            "nodes": [{"mesh": 0}]
        });
        let source = (0..32).collect::<Vec<u8>>();
        let bin = compact(&mut root, &source).unwrap();
        let views = root["bufferViews"].as_array().unwrap();
        assert_eq!(views.len(), 3);
        assert_eq!(views[0]["byteOffset"], 1);
        assert_eq!(views[1]["byteOffset"], 4);
        assert_eq!(views[2]["byteOffset"], 11);
        assert_eq!(&bin[1..5], &source[13..17]);
        assert_eq!(&bin[4..11], &source[16..23]);
        assert_eq!(&bin[11..14], &source[27..30]);
        assert_eq!(root["buffers"][0]["byteLength"], 14);
    }

    #[test]
    fn opaque_extension_keeps_original_buffer_indices_and_offsets() {
        let mut root = json!({
            "extras": {"bevy_editor_paint": {"layers": []}, "my_metadata": "keep"},
            "extensionsUsed": ["KHR_animation_pointer"],
            "animations": [{"channels": [{"sampler": 0, "target": {
                "path": "pointer", "extensions": {"KHR_animation_pointer": {
                    "pointer": "/materials/1/pbrMetallicRoughness/roughnessFactor"
                }}
            }}], "samplers": [{"input": 0, "output": 1}]}],
            "buffers": [{"byteLength": 8}],
            "materials": [{"name": "first"}, {"name": "second"}],
        });
        let animations = root["animations"].clone();
        let source = [0, 1, 2, 3, 4, 5, 6, 7];
        assert_eq!(compact(&mut root, &source).unwrap(), source);
        assert_eq!(root["materials"][1]["name"], "second");
        assert_eq!(root["animations"], animations);
        assert_eq!(root["extras"], json!({"my_metadata": "keep"}));
    }

    #[test]
    fn compact_game_glb_rejects_out_of_bounds_live_references() {
        let root = json!({
            "buffers": [{"byteLength": 8}],
            "bufferViews": [{"buffer": 0, "byteOffset": 8, "byteLength": 4}],
            "accessors": [{"bufferView": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "nodes": [{"mesh": 0}]
        });
        let bin = [0; 8];
        let mut invalid_accessor = root.clone();
        invalid_accessor["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(3);
        assert!(compact(&mut invalid_accessor, &bin).is_err());
        assert!(compact(&mut root.clone(), &bin).is_err());
    }
}
