//! Paint layers and flattened standard glTF materials travel in the same GLB.
use super::*;
use crate::editor::model_paint_data::{custom, map_fixes};
use crate::editor::model_paint_data::{
    Blend, CHANNEL_COUNT, Layer, LayerKind, MAX_LAYERS, SPECULAR_MAX_F0, Stack, Surface,
    dielectric_f0,
    mask::{
        BakedInputs, Filter, Generator, GraphMask, INPUT_CHANNELS, MAX_FILTERS, MAX_MASK_LAYERS, Mask,
        MaskLayer, PartIds,
    },
    texture_fill,
};
use std::sync::Arc;

fn push(root: &mut Value, key: &str, value: Value) -> usize {
    if !root[key].is_array() {
        root[key] = json!([]);
    }
    let list = root[key].as_array_mut().unwrap();
    let id = list.len();
    list.push(value);
    id
}
fn png(root: &mut Value, bin: &mut Vec<u8>, size: u32, pixels: &[u8]) -> Result<usize, String> {
    let image = image::RgbaImage::from_raw(size, size, pixels.to_vec())
        .ok_or("Некорректная текстура слоя")?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    align(bin, 0);
    let start = bin.len();
    bin.extend_from_slice(bytes.get_ref());
    Ok(push(
        root,
        "bufferViews",
        json!({"buffer":0,"byteOffset":start,"byteLength":bytes.get_ref().len()}),
    ))
}
fn texture(root: &mut Value, view: usize) -> usize {
    let image = push(
        root,
        "images",
        json!({"bufferView":view,"mimeType":"image/png"}),
    );
    push(root, "textures", json!({"source":image}))
}

fn bake_view(root: &mut Value, bin: &mut Vec<u8>, pixels: &[u8]) -> usize {
    let bytes = lz4_flex::block::compress(pixels);
    align(bin, 0);
    let start = bin.len();
    bin.extend_from_slice(&bytes);
    push(
        root,
        "bufferViews",
        json!({"buffer":0,"byteOffset":start,"byteLength":bytes.len()}),
    )
}

pub(super) fn encode_fixes(
    root: &mut Value,
    bin: &mut Vec<u8>,
    primitive: &mut Value,
    fixes: &map_fixes::Fixes,
) -> Result<(), String> {
    let Some(data) = primitive.as_object_mut() else {
        return Err("Некорректный примитив GLB".into());
    };
    if fixes.is_empty() {
        if let Some(extras) = data.get_mut("extras").and_then(Value::as_object_mut)
            && extras.remove(map_fixes::EXTRAS).is_some()
            && extras.is_empty()
        {
            data.remove("extras");
        }
        return Ok(());
    }
    let extras = data.entry("extras").or_insert_with(|| json!({})).as_object_mut()
        .ok_or("Некорректные данные материала GLB")?;
    let mut entries = Vec::with_capacity(fixes.len());
    for (map, plane) in fixes {
        if !map.supports_fix() || map_fixes::Plane::byte_len(plane.size) != Some(plane.pixels.len()) {
            return Err("Некорректное исправление карты".into());
        }
        let view = bake_view(root, bin, &plane.pixels);
        entries.push(json!({"map": map, "size": plane.size, "view": view}));
    }
    extras.insert(map_fixes::EXTRAS.into(), json!({"version": 1, "maps": entries}));
    Ok(())
}

pub(super) fn decode_fixes(root: &Value, bin: &[u8], primitive: &Value) -> Result<map_fixes::Fixes, String> {
    let Some(metadata) = primitive["extras"].get(map_fixes::EXTRAS) else {
        return Ok(Vec::new());
    };
    let error = "Повреждены исправления запечённых карт";
    let entries = metadata["maps"].as_array()
        .filter(|entries| metadata["version"].as_u64() == Some(1) && entries.len() <= 11)
        .ok_or(error)?;
    let mut fixes = Vec::with_capacity(entries.len());
    for entry in entries {
        let map: map_fixes::BakeMap = serde_json::from_value(entry["map"].clone()).map_err(|_| error)?;
        if !map.supports_fix() || fixes.iter().any(|(existing, _)| *existing == map) {
            return Err(error.into());
        }
        let size = entry["size"].as_u64().and_then(|value| u32::try_from(value).ok()).ok_or(error)?;
        let mut pixels = vec![0; map_fixes::Plane::byte_len(size).ok_or(error)?];
        read_bake_view(root, bin, &entry["view"], &mut pixels).map_err(|_| error)?;
        fixes.push((map, Arc::new(map_fixes::Plane { size, pixels })));
    }
    Ok(fixes)
}

fn write_bitmap(root: &mut Value, bin: &mut Vec<u8>, image: &texture_fill::Bitmap) -> Value {
    align(bin, 0);
    let offset = bin.len();
    bin.extend_from_slice(&image.bytes);
    let view = push(root, "bufferViews", json!({
        "buffer": 0, "byteOffset": offset, "byteLength": image.bytes.len()
    }));
    json!({"name": image.name, "view": view})
}

fn read_bitmap(root: &Value, bin: &[u8], image: &Value) -> Result<Arc<texture_fill::Bitmap>, String> {
    let error = "Повреждённая текстура графа";
    let name = image["name"].as_str().ok_or(error)?;
    let id = image["view"].as_u64().ok_or(error)? as usize;
    let view = &root["bufferViews"][id];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let length = view["byteLength"].as_u64().ok_or(error)? as usize;
    let bytes = bin.get(start..start.checked_add(length).ok_or(error)?).ok_or(error)?;
    texture_fill::Bitmap::decode(name.into(), bytes).map(Arc::new)
}

fn write_graph_images(root: &mut Value, bin: &mut Vec<u8>, images: &texture_fill::GraphImages) -> Value {
    Value::Object(images.iter().map(|(key, image)| (key.clone(), write_bitmap(root, bin, image))).collect())
}

fn read_graph_images(root: &Value, bin: &[u8], images: &Value) -> Result<Arc<texture_fill::GraphImages>, String> {
    let mut decoded = texture_fill::GraphImages::new();
    if let Some(references) = images.as_object() {
        if references.len() > 64 {
            return Err("Слишком много изображений графа".into());
        }
        let mut bytes = 0;
        let mut texels = 0u64;
        for (key, reference) in references {
            let image = read_bitmap(root, bin, reference)?;
            bytes += image.bytes.len();
            texels += u64::from(image.pixels().width()) * u64::from(image.pixels().height());
            if bytes > 64 * 1024 * 1024 || texels > 64 * 1024 * 1024 {
                return Err("Текстуры графа слишком большие".into());
            }
            decoded.insert(key.clone(), image);
        }
    }
    Ok(Arc::new(decoded))
}

pub(super) fn encode(
    root: &mut Value,
    bin: &mut Vec<u8>,
    primitive: &mut Value,
    surface: &Surface,
) -> Result<(), String> {
    let base = surface
        .base
        .iter()
        .map(|pixels| png(root, bin, surface.size, pixels).map(|i| json!(i)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut layers = Vec::new();
    for layer in &surface.stack.layers {
        let mut channels = Vec::new();
        for pixels in &layer.channels {
            channels.push(match pixels {
                Some(pixels) => json!(png(root, bin, surface.size, pixels)?),
                None => Value::Null,
            });
        }
        let mask = layer
            .mask
            .as_ref()
            .map(|mask| png(root, bin, surface.size, &mask.pixels))
            .transpose()?;
        let filters = layer
            .mask
            .as_ref()
            .map_or(&[][..], |mask| mask.filters.as_slice());
        let mask_layers = layer
            .mask
            .as_ref()
            .map_or(&[][..], |mask| mask.layers.as_slice());
        let generator = layer.mask.as_ref().and_then(|mask| mask.generator);
        let affine = layer.mask.as_ref().is_some_and(|mask| mask.affine);
        layers.push(json!({"name":layer.name,"kind":layer.kind,"visible":layer.visible,"opacity":layer.opacity,"blend":layer.blend,"depth":layer.depth,"channels":channels,"mask":mask,"mask_filters":filters,"mask_layers":mask_layers,"mask_generator":generator,"mask_affine":affine}));
        let row = layers.last_mut().unwrap();
        if let Some(id) = layer.graph_id {
            row["graph_id"] = json!(id);
        }
        if let Some(graph) = &layer.folder_graph {
            row["folder_graph"] = graph.as_ref().clone();
            if !layer.folder_images.is_empty() {
                row["folder_images"] = write_graph_images(root, bin, &layer.folder_images);
            }
        }
        if layer.kind == LayerKind::Folder && layer.collapsed {
            row["collapsed"] = json!(true);
        }
        if let Some(coverage) = layer.mask.as_ref().and_then(|mask| mask.geometry_coverage()) {
            row["mask_geometry"] = json!(bake_view(root, bin, coverage));
        }
        if let Some(graph) = layer.mask.as_ref().and_then(|mask| mask.graph.as_ref()) {
            row["mask_graph"] = json!({
                "graph": graph.payload.as_ref(),
                "graph_images": write_graph_images(root, bin, &graph.images),
                "output": graph.output.as_ref().map(|image| write_bitmap(root, bin, image)),
            });
        }
        if let Some(fill) = &layer.texture_fill {
            let images: Vec<_> = fill.images.iter()
                .map(|image| image.as_ref().map(|image| write_bitmap(root, bin, image))).collect();
            row["texture_fill"] = json!({
                "settings": fill.settings, "images": images,
            });
            if let Some(graph) = &fill.graph {
                row["texture_fill"]["graph"] = (**graph).clone();
                if !fill.graph_images.is_empty() {
                    row["texture_fill"]["graph_images"] = write_graph_images(root, bin, &fill.graph_images);
                }
            }
        }
    }
    primitive["extras"]["bevy_editor_paint"] = json!({"version":1,"size":surface.size,"base":base,"layers":layers,"selected":surface.stack.selected,"material":surface.material});
    if surface.needs_bake()
        && let Some(inputs) = &surface.baked
    {
        let view = bake_view(root, bin, inputs.texels.as_flattened());
        let mut data = json!({"size":inputs.size,"edge_reach":inputs.edge_reach,"view":view});
        if inputs.disabled_inputs != 0 {
            data["disabled_inputs"] = json!(inputs.disabled_inputs);
        }
        if let Some(ids) = &inputs.part_ids {
            data["part_ids"] = json!(bake_view(root, bin, ids.colors.as_flattened()));
        }
        if !inputs.custom.is_empty() {
            data["custom"] = Value::Array(inputs.custom.iter().map(|plane| {
                json!({"recipe":plane.recipe,"view":bake_view(root, bin, &plane.values)})
            }).collect());
        }
        primitive["extras"]["bevy_editor_paint"]["baked_inputs"] = data;
    }
    let maps = surface.composites();
    let mut color = maps[0].clone();
    for (p, a) in color.chunks_exact_mut(4).zip(maps[4].chunks_exact(4)) {
        p[3] = a[0];
    }
    let color = png(root, bin, surface.size, &color)?;
    let color = texture(root, color);
    let mut rm = maps[1].clone();
    for (p, metal) in rm.chunks_exact_mut(4).zip(maps[2].chunks_exact(4)) {
        p[1] = p[0];
        p[0] = 255;
        p[2] = metal[0];
        p[3] = 255;
    }
    let rm = png(root, bin, surface.size, &rm)?;
    let rm = texture(root, rm);
    let mut material = surface.material.clone();
    if !material.is_object() {
        material = json!({});
    }
    material["pbrMetallicRoughness"]["baseColorFactor"] = json!([1., 1., 1., 1.]);
    material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":color});
    material["pbrMetallicRoughness"]["metallicFactor"] = json!(1.);
    material["pbrMetallicRoughness"]["roughnessFactor"] = json!(1.);
    material["pbrMetallicRoughness"]["metallicRoughnessTexture"] = json!({"index":rm});
    if color_has_alpha(&maps[4]) {
        material["alphaMode"] = json!("BLEND");
    }
    if surface.has_channel(5) {
        let view = png(root, bin, surface.size, &maps[5])?;
        let tex = texture(root, view);
        material["emissiveTexture"] = json!({"index":tex});
        material["emissiveFactor"] = json!([1., 1., 1.]);
    }
    if surface.has_channel(6) || surface.has_channel(3) {
        let normal = surface.normal_map(&maps);
        let view = png(root, bin, surface.size, &normal)?;
        let tex = texture(root, view);
        material["normalTexture"] = json!({"index":tex});
    }
    if surface.has_channel(7) {
        let mut spec = maps[7].clone();
        let scale = dielectric_f0(&material);
        for p in spec.chunks_exact_mut(4) {
            p[3] = if scale > 0. {
                (p[0] as f32 * SPECULAR_MAX_F0 / scale)
                    .clamp(0., 255.)
                    .round() as u8
            } else {
                0
            };
        }
        let view = png(root, bin, surface.size, &spec)?;
        let tex = texture(root, view);
        let extension = &mut material["extensions"]["KHR_materials_specular"];
        extension["specularFactor"] = json!(1.);
        extension["specularTexture"] = json!({"index":tex});
        if !root["extensionsUsed"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "KHR_materials_specular"))
        {
            push(root, "extensionsUsed", json!("KHR_materials_specular"));
        }
    }
    let id = push(root, "materials", material);
    primitive["material"] = json!(id);
    Ok(())
}
pub(super) fn color_has_alpha(opacity: &[u8]) -> bool {
    opacity.chunks_exact(4).any(|p| p[0] < 255)
}
pub(super) fn decode(
    root: &Value,
    bin: &[u8],
    primitive: &Value,
) -> Result<Option<Arc<Surface>>, String> {
    let data = &primitive["extras"]["bevy_editor_paint"];
    if data.is_null() {
        return Ok(None);
    }
    if data["version"].as_u64() != Some(1) {
        return Err("Неподдерживаемая версия слоёв рисования".into());
    }
    let size = data["size"]
        .as_u64()
        .filter(|s| (16..=4096).contains(s))
        .ok_or("Некорректное разрешение слоёв")? as u32;
    let read = |value: &Value| -> Result<Arc<Vec<u8>>, String> {
        let id = value.as_u64().ok_or("Нет текстуры слоя")? as usize;
        let view = &root["bufferViews"][id];
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = view["byteLength"].as_u64().ok_or("Нет данных слоя")? as usize;
        let bytes = bin
            .get(start..start.checked_add(len).ok_or("Повреждённый слой")?)
            .ok_or("Повреждённая текстура слоя")?;
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        if image.width() != size || image.height() != size {
            return Err("Размер текстуры слоя не совпадает".into());
        }
        Ok(Arc::new(image.to_rgba8().into_raw()))
    };
    let base = data["base"]
        .as_array()
        .filter(|v| v.len() == CHANNEL_COUNT)
        .ok_or("Нет основы материала")?;
    let base = base
        .iter()
        .map(read)
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .unwrap();
    let rows = data["layers"]
        .as_array()
        .filter(|v| v.len() <= MAX_LAYERS)
        .ok_or("Слишком много слоёв")?;
    let mut layers = Vec::new();
    for row in rows {
        let kind: LayerKind =
            serde_json::from_value(row["kind"].clone()).map_err(|e| e.to_string())?;
        let channels = row["channels"]
            .as_array()
            .filter(|v| v.len() == CHANNEL_COUNT)
            .ok_or("Нет каналов слоя")?;
        let channels = channels
            .iter()
            .map(|v| {
                if v.is_null() {
                    Ok(None)
                } else {
                    read(v).map(Some)
                }
            })
            .collect::<Result<Vec<_>, String>>()?
            .try_into()
            .unwrap();
        let mask = if let Some(value) = row.get("mask").filter(|value| !value.is_null()) {
            let filters: Vec<Filter> = row
                .get("mask_filters")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            if filters.len() > MAX_FILTERS || filters.iter().any(|filter| !filter.valid()) {
                return Err("Некорректные параметры фильтров маски".into());
            }
            let layers: Vec<MaskLayer> = row
                .get("mask_layers")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            if layers.len() > MAX_MASK_LAYERS || layers.iter().any(|layer| !layer.valid()) {
                return Err("Некорректные параметры слоёв маски".into());
            }
            let mut mask = Mask::new(read(value)?, filters);
            if let Some(view) = row.get("mask_geometry") {
                let mut coverage = vec![0; (size * size) as usize];
                read_bake_view(root, bin, view, &mut coverage)?;
                mask.set_geometry_coverage(coverage);
            }
            *mask.edit_layers() = layers;
            let generator: Option<Generator> = row
                .get("mask_generator")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .flatten();
            let affine = row["mask_affine"].as_bool().unwrap_or(false);
            if generator.is_some_and(|g| !g.valid() || !affine) {
                return Err("Некорректные параметры генератора маски".into());
            }
            mask.generator = generator;
            mask.affine = affine;
            if let Some(graph) = row.get("mask_graph") {
                if !affine || !graph["graph"].is_object() {
                    return Err("Повреждённый граф маски".into());
                }
                let output = graph.get("output").filter(|value| !value.is_null())
                    .map(|value| read_bitmap(root, bin, value)).transpose()?;
                mask.set_graph(Arc::new(GraphMask::new(
                    Arc::new(graph["graph"].clone()),
                    read_graph_images(root, bin, &graph["graph_images"])?,
                    output,
                )));
            }
            Some(Arc::new(mask))
        } else {
            None
        };
        let texture_fill = row.get("texture_fill").map(|value| {
            let error = "Повреждённая текстурная заливка";
            if kind != LayerKind::Fill {
                return Err(error.to_owned());
            }
            let settings: texture_fill::Settings =
                serde_json::from_value(value["settings"].clone()).map_err(|_| error.to_owned())?;
            if !settings.valid() {
                return Err(error.to_owned());
            }
            let images = value["images"].as_array().filter(|images| images.len() == CHANNEL_COUNT)
                .ok_or(error)?;
            let mut fill = texture_fill::Fill::default();
            fill.settings = settings;
            fill.graph = value.get("graph").filter(|graph| graph.is_object()).map(|graph| Arc::new(graph.clone()));
            for (target, image) in fill.images.iter_mut().zip(images) {
                if !image.is_null() {
                    *target = Some(read_bitmap(root, bin, image)?);
                }
            }
            fill.graph_images = read_graph_images(root, bin, &value["graph_images"])?;
            Ok(Arc::new(fill))
        }).transpose()?;
        layers.push(Layer {
            mask,
            texture_fill,
            name: row["name"].as_str().unwrap_or("Слой").into(),
            graph_id: row["graph_id"].as_u64().filter(|&id| id != 0),
            folder_graph: (kind == LayerKind::Folder)
                .then(|| row["folder_graph"].as_object().map(|_| Arc::new(row["folder_graph"].clone())))
                .flatten(),
            folder_images: if kind == LayerKind::Folder && row["folder_graph"].is_object() {
                read_graph_images(root, bin, &row["folder_images"])?
            } else {
                Arc::default()
            },
            kind,
            depth: row
                .get("depth")
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or("Некорректная вложенность слоёв")
                })
                .transpose()?
                .unwrap_or_default(),
            blend: row
                .get("blend")
                .map(|mode| serde_json::from_value::<Blend>(mode.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default(),
            visible: row["visible"].as_bool().unwrap_or(true),
            collapsed: kind == LayerKind::Folder && row["collapsed"].as_bool().unwrap_or(false),
            opacity: row["opacity"]
                .as_f64()
                .filter(|v| v.is_finite())
                .unwrap_or(100.)
                .clamp(0., 100.) as f32,
            channels,
        });
    }
    let selected =
        (data["selected"].as_u64().unwrap_or(0) as usize).min(layers.len().saturating_sub(1));
    let stack = Stack { layers, selected };
    if !stack.valid_hierarchy() {
        return Err("Некорректная вложенность слоёв".into());
    }
    let baked = data
        .get("baked_inputs")
        .map(|input| -> Result<_, &'static str> {
            let error = "Повреждённые карты генераторов масок";
            let size = input["size"]
                .as_u64()
                .filter(|s| (16..=4096).contains(s))
                .ok_or(error)? as u32;
            let edge_reach = input["edge_reach"]
                .as_f64()
                .map(|v| v as f32)
                .filter(|v| v.is_finite() && *v > 0.)
                .ok_or(error)?;
            let count = (size * size) as usize;
            let mut texels = vec![[0; INPUT_CHANNELS]; count];
            read_bake_view(root, bin, &input["view"], texels.as_flattened_mut())?;
            let part_ids = if let Some(view) = input.get("part_ids") {
                let mut colors = vec![[0; 3]; count];
                read_bake_view(root, bin, view, colors.as_flattened_mut())?;
                Some(PartIds::new(colors))
            } else {
                None
            };
            Ok(Arc::new(BakedInputs {
                size,
                edge_reach,
                disabled_inputs: input["disabled_inputs"].as_u64().and_then(|bits| u32::try_from(bits).ok()).unwrap_or(0),
                texels,
                part_ids,
                custom: read_custom_planes(root, bin, input.get("custom"), count)?,
            }))
        })
        .transpose()?;
    Ok(Some(Arc::new(Surface {
        size,
        base,
        stack,
        material: data["material"].clone(),
        baked,
    })))
}

fn read_custom_planes(
    root: &Value,
    bin: &[u8],
    data: Option<&Value>,
    count: usize,
) -> Result<Vec<custom::Plane>, &'static str> {
    let Some(data) = data else {
        return Ok(Vec::new());
    };
    let error = "Повреждённые пользовательские карты";
    let rows = data
        .as_array()
        .filter(|rows| rows.len() <= custom::MAX_MAPS)
        .ok_or(error)?;
    let recipes = rows
        .iter()
        .map(|row| serde_json::from_value(row["recipe"].clone()).map_err(|_| error))
        .collect::<Result<Vec<custom::Recipe>, _>>()?;
    if !custom::valid_recipes(&recipes) {
        return Err(error);
    }
    rows.iter()
        .zip(recipes)
        .map(|(row, recipe)| {
            let mut values = vec![0; count];
            read_bake_view(root, bin, &row["view"], &mut values)?;
            Ok(custom::Plane {
                recipe,
                values: Arc::new(values),
            })
        })
        .collect()
}

fn read_bake_view(
    root: &Value,
    bin: &[u8],
    index: &Value,
    output: &mut [u8],
) -> Result<(), &'static str> {
    let error = "Повреждённые карты генераторов масок";
    let id = index
        .as_u64()
        .and_then(|id| usize::try_from(id).ok())
        .ok_or(error)?;
    let view = &root["bufferViews"][id];
    let start = view["byteOffset"].as_u64().unwrap_or(0);
    let len = view["byteLength"].as_u64().ok_or(error)?;
    let end = start
        .checked_add(len)
        .filter(|&end| end <= bin.len() as u64)
        .ok_or(error)?;
    let written = lz4_flex::block::decompress_into(&bin[start as usize..end as usize], output)
        .map_err(|_| error)?;
    if written != output.len() {
        return Err(error);
    }
    Ok(())
}
