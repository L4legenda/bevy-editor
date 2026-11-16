//! Paint layers and flattened standard glTF materials travel in the same GLB.
use super::*;
use crate::editor::model_paint_data::{CHANNEL_COUNT, Layer, LayerKind, Stack, Surface};
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
        layers.push(json!({"name":layer.name,"kind":layer.kind,"visible":layer.visible,"opacity":layer.opacity,"channels":channels}));
    }
    primitive["extras"]["bevy_editor_paint"] = json!({"version":1,"size":surface.size,"base":base,"layers":layers,"selected":surface.stack.selected,"material":surface.material});
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
        for p in spec.chunks_exact_mut(4) {
            p[3] = p[0];
        }
        let view = png(root, bin, surface.size, &spec)?;
        let tex = texture(root, view);
        material["extensions"]["KHR_materials_specular"] =
            json!({"specularFactor":1.,"specularTexture":{"index":tex}});
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
        .filter(|v| v.len() <= 256)
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
        layers.push(Layer {
            name: row["name"].as_str().unwrap_or("Слой").into(),
            kind,
            visible: row["visible"].as_bool().unwrap_or(true),
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
    Ok(Some(Arc::new(Surface {
        size,
        base,
        stack: Stack { layers, selected },
        material: data["material"].clone(),
    })))
}
