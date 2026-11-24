//! Lossless-container GLB updates: retain textures, materials, extras and untouched nodes.
use super::Object;
use bevy::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
#[path = "model_paint_io.rs"]
mod paint_io;

#[derive(Clone)]
pub(in crate::editor) struct Glb {
    pub path: PathBuf,
    pub json: Value,
    bin: Vec<u8>,
    extra: Vec<(u32, Vec<u8>)>,
    expected: Vec<u8>,
}
fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|v| v.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or("Повреждённый GLB: неполный заголовок".into())
}
impl Glb {
    pub fn new_asset(objects: &[Object]) -> Result<Vec<u8>, String> {
        Self::new_asset_with_pngs(objects, &[])
    }

    pub(in crate::editor) fn new_asset_with_pngs(objects: &[Object], pngs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
        let mut source = Self {
            path: PathBuf::new(),
            json: json!({
                "asset": {"version": "2.0", "generator": "Bevy Editor"},
                "scene": 0,
                "scenes": [{"nodes": []}],
                "nodes": [],
                "meshes": [],
                "materials": [{
                    "name": "Материал", "doubleSided": true,
                    "pbrMetallicRoughness": {
                        "baseColorFactor": [0.72, 0.74, 0.78, 1.0],
                        "metallicFactor": 0.0, "roughnessFactor": 0.75
                    }
                }],
                "buffers": [{"byteLength": 0}]
            }),
            bin: Vec::new(),
            extra: Vec::new(),
            expected: Vec::new(),
        };
        if !pngs.is_empty() {
            source.json["bufferViews"] = json!([]);
            let mut images = Vec::with_capacity(pngs.len());
            for png in pngs {
                align(&mut source.bin, 0);
                let start = source.bin.len();
                source.bin.extend_from_slice(png);
                let view = source.json["bufferViews"].as_array_mut().unwrap();
                images.push(json!({"bufferView":view.len(), "mimeType":"image/png"}));
                view.push(json!({"buffer":0, "byteOffset":start, "byteLength":png.len()}));
            }
            source.json["textures"] = json!((0..images.len()).map(|i| json!({"source":i})).collect::<Vec<_>>());
            source.json["images"] = json!(images);
        }
        source.encode(objects, &[])
    }

    pub fn data(&self) -> &[u8] {
        &self.expected
    }
    pub fn paint_surface(&self, primitive: &Value) -> Result<Option<std::sync::Arc<crate::editor::model_paint_data::Surface>>, String> {
        paint_io::decode(&self.json, &self.bin, primitive)
    }

    pub(in crate::editor) fn bake_fixes(&self, primitive: &Value) -> Result<std::sync::Arc<crate::editor::model_paint_data::map_fixes::Fixes>, String> {
        paint_io::decode_fixes(&self.json, &self.bin, primitive).map(std::sync::Arc::new)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("glb"))
        {
            return Err("Правка геометрии поддерживает файлы GLB 2.0".into());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        Self::parse(path.to_owned(), bytes)
    }
    fn parse(path: PathBuf, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > 512 * 1024 * 1024 {
            return Err("Для редактирования поддерживаются GLB до 512 МБ".into());
        }
        if word(&bytes, 0)? != 0x46546c67
            || word(&bytes, 4)? != 2
            || word(&bytes, 8)? as usize != bytes.len()
        {
            return Err("Ожидается корректный GLB версии 2".into());
        }
        let mut json = None;
        let mut bin = None;
        let mut extra = Vec::new();
        let mut at = 12;
        while at < bytes.len() {
            let length = word(&bytes, at)? as usize;
            let kind = word(&bytes, at + 4)?;
            let end = at
                .checked_add(8)
                .and_then(|n| n.checked_add(length))
                .ok_or("Слишком большой GLB-блок")?;
            let data = bytes
                .get(at + 8..end)
                .ok_or("Повреждённая длина GLB-блока")?;
            match kind {
                0x4e4f534a => {
                    if json.is_some() {
                        return Err("Повторный JSON-блок".into());
                    }
                    json = Some(serde_json::from_slice(data).map_err(|e| e.to_string())?);
                }
                0x004e4942 => {
                    if bin.is_some() {
                        return Err("Повторный BIN-блок".into());
                    }
                    bin = Some(data.to_vec());
                }
                _ => extra.push((kind, data.to_vec())),
            }
            at = end;
        }
        let json: Value = json.ok_or("GLB не содержит JSON")?;
        if json["buffers"]
            .as_array()
            .and_then(|v| v.first())
            .is_some_and(|b| !b["uri"].is_null())
        {
            return Err("Буфер 0 должен находиться внутри GLB".into());
        }
        Ok(Self {
            path,
            json,
            bin: bin.unwrap_or_default(),
            extra,
            expected: bytes,
        })
    }
    pub fn encode(&self, objects: &[Object], original: &[Object]) -> Result<Vec<u8>, String> {
        self.encode_with_cage(objects, original, None)
    }

    fn encode_with_cage(
        &self,
        objects: &[Object],
        original: &[Object],
        cage_edits: Option<&crate::editor::model_cage_data::Edits>,
    ) -> Result<Vec<u8>, String> {
        let mut root = self.json.clone();
        let mut bin = self.bin.clone();
        for name in ["nodes", "meshes", "bufferViews", "accessors"] {
            if root[name].is_null() {
                root[name] = json!([]);
            }
            if !root[name].is_array() {
                return Err(format!("Некорректная таблица {name}"));
            }
        }
        let node_count = root["nodes"].as_array().unwrap().len();
        let mut parents = vec![None; node_count];
        for (i, node) in self.json["nodes"].as_array().unwrap().iter().enumerate() {
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    if let Some(child) = child
                        .as_u64()
                        .map(|v| v as usize)
                        .filter(|&n| n < node_count)
                    {
                        parents[child] = Some(i);
                    }
                }
            }
        }
        let desired: HashMap<_, _> = objects
            .iter()
            .filter_map(|o| o.source_node.map(|node| (node, o.transform.to_matrix())))
            .collect();
        fn world(
            index: usize,
            parents: &[Option<usize>],
            nodes: &Value,
            desired: &HashMap<usize, Mat4>,
            depth: usize,
        ) -> Result<Mat4, String> {
            if depth > 256 {
                return Err("Слишком глубокая иерархия GLB".into());
            }
            if let Some(m) = desired.get(&index) {
                return Ok(*m);
            }
            let local = node_matrix(&nodes[index])?;
            Ok(if let Some(parent) = parents[index] {
                world(parent, parents, nodes, desired, depth + 1)? * local
            } else {
                local
            })
        }
        for old in original {
            if let Some(node) = old.source_node {
                if !objects.iter().any(|o| o.source_node == Some(node)) {
                    root["nodes"][node].as_object_mut().unwrap().remove("mesh");
                }
            }
        }
        for object in objects {
            let node = if let Some(node) = object.source_node {
                node
            } else {
                let node = root["nodes"].as_array().unwrap().len();
                root["nodes"].as_array_mut().unwrap().push(json!({}));
                if root["scenes"].as_array().is_none_or(|v| v.is_empty()) {
                    root["scenes"] = json!([{"nodes":[]}]);
                    root["scene"] = json!(0);
                }
                let scene = root["scene"].as_u64().unwrap_or(0) as usize;
                if root["scenes"][scene]["nodes"].is_null() {
                    root["scenes"][scene]["nodes"] = json!([]);
                }
                root["scenes"][scene]["nodes"]
                    .as_array_mut()
                    .ok_or("Некорректный список узлов сцены")?
                    .push(json!(node));
                node
            };
            let parent = object
                .source_node
                .and_then(|n| parents[n])
                .map(|p| world(p, &parents, &self.json["nodes"], &desired, 0))
                .transpose()?
                .unwrap_or(Mat4::IDENTITY);
            let local = parent.inverse() * object.transform.to_matrix();
            if !local.is_finite() {
                return Err("Невозможно сохранить вырожденную матрицу родителя".into());
            }
            let transform = Transform::from_matrix(local);
            let difference = transform
                .to_matrix()
                .to_cols_array()
                .into_iter()
                .zip(local.to_cols_array())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            let node_value = &mut root["nodes"][node];
            let map = node_value.as_object_mut().ok_or("Некорректный узел GLB")?;
            if let Some(edits) = cage_edits {
                crate::editor::model_cage_data::write(map, object, edits.get(&object.id))?;
            }
            map.insert("name".into(), json!(object.name));
            map.remove("matrix");
            map.remove("translation");
            map.remove("rotation");
            map.remove("scale");
            if difference < 0.0001 {
                map.insert(
                    "translation".into(),
                    json!(transform.translation.to_array()),
                );
                map.insert("rotation".into(), json!(transform.rotation.to_array()));
                map.insert("scale".into(), json!(transform.scale.to_array()));
            } else {
                map.insert("matrix".into(), json!(local.to_cols_array()));
            }
            if object.geometry.faces.is_empty() && object.geometry.loose_edges.is_empty() {
                map.remove("mesh");
                continue;
            }
            if object.source_node.is_some()
                && original.iter().any(|old| {
                    old.source_node == object.source_node
                        && old.geometry == object.geometry
                        && old.materials == object.materials
                })
            {
                continue;
            }
            let mut primitives = Vec::new();
            let mut topology = Vec::new();
            for (slot, mut buffers) in object.geometry.buffers() {
                if buffers.indices.is_empty() {
                    continue;
                }
                buffers.localize_uv(object.uv_offset(slot));
                let position =
                    append_vec::<3>(&mut root, &mut bin, &buffers.positions, "VEC3", true);
                let normal = append_vec::<3>(&mut root, &mut bin, &buffers.normals, "VEC3", false);
                let uv = append_vec::<2>(&mut root, &mut bin, &buffers.uv, "VEC2", false);
                let uv1 = append_vec::<2>(&mut root, &mut bin, &buffers.uv1, "VEC2", false);
                let colors = append_vec::<4>(&mut root, &mut bin, &buffers.colors, "VEC4", false);
                align(&mut bin, 0);
                let start = bin.len();
                for index in &buffers.indices {
                    bin.extend_from_slice(&index.to_le_bytes());
                }
                let view = append_view(&mut root, start, bin.len() - start, 34963);
                let indices = append_accessor(
                    &mut root,
                    json!({"bufferView":view,"componentType":5125,"count":buffers.indices.len(),"type":"SCALAR"}),
                );
                let mut primitive = object
                    .materials
                    .get(slot)
                    .map_or_else(|| json!({}), |slot| slot.source.clone());
                if let Some(definition) = object.materials.get(slot).and_then(|m| m.authored.as_deref()) {
                    if root["materials"].is_null() {
                        root["materials"] = json!([]);
                    }
                    let materials = root["materials"].as_array_mut()
                        .ok_or("Некорректная таблица материалов GLB")?;
                    let index = materials.iter().position(|material| material == definition)
                        .unwrap_or_else(|| {
                            materials.push(definition.clone());
                            materials.len() - 1
                        });
                    primitive["material"] = json!(index);
                }
                if let Some(surface) = object.materials.get(slot).and_then(|m| m.paint.as_deref()) {
                    let mut surface = surface.clone();
                    surface.bind_texture_fills(&object.geometry, slot, object.uv_offset(slot), object.transform);
                    surface.bind_generators(&object.geometry, slot, object.uv_offset(slot), object.transform);
                    paint_io::encode(&mut root, &mut bin, &mut primitive, &surface)?;
                }
                if let Some(material) = object.materials.get(slot) {
                    paint_io::encode_fixes(&mut root, &mut bin, &mut primitive, &material.bake_fixes)?;
                }
                let map = primitive
                    .as_object_mut()
                    .ok_or("Некорректный примитив GLB")?;
                map.remove("targets");
                if let Some(extensions) = map.get_mut("extensions").and_then(Value::as_object_mut) {
                    extensions.remove("KHR_draco_mesh_compression");
                }
                map.insert("attributes".into(),json!({"POSITION":position,"NORMAL":normal,"TEXCOORD_0":uv,"TEXCOORD_1":uv1,"COLOR_0":colors}));
                map.insert("indices".into(), json!(indices));
                map.insert("mode".into(), json!(4));
                primitives.push(primitive);
                topology.push(buffers.polygons);
            }
            if !object.geometry.loose_edges.is_empty() {
                let lines = object.geometry.line_positions();
                let position = append_vec::<3>(&mut root, &mut bin, &lines, "VEC3", true);
                primitives.push(json!({"attributes":{"POSITION":position},"mode":1}));
                topology.push(Vec::new());
            }
            if primitives.is_empty() {
                root["nodes"][node].as_object_mut().unwrap().remove("mesh");
                continue;
            }
            let mesh = root["meshes"].as_array().unwrap().len();
            let mut saved_topology = json!({"version":1,"primitives":topology});
            if !object.geometry.uv_seams.is_empty() {
                saved_topology["uv_seams"] = json!(object.geometry.uv_seam_positions());
            }
            root["meshes"].as_array_mut().unwrap().push(json!({
                "name":object.name,"primitives":primitives,
                "extras":{"bevy_editor_topology":saved_topology}
            }));
            root["nodes"][node]["mesh"] = json!(mesh);
        }
        if root["buffers"].as_array().is_none_or(|v| v.is_empty()) {
            root["buffers"] = json!([{}]);
        }
        root["buffers"][0]["byteLength"] = json!(bin.len());
        let mut json = serde_json::to_vec(&root).map_err(|e| e.to_string())?;
        align(&mut json, b' ');
        align(&mut bin, 0);
        let length = 12
            + 8
            + json.len()
            + 8
            + bin.len()
            + self.extra.iter().map(|(_, b)| 8 + b.len()).sum::<usize>();
        if length > u32::MAX as usize {
            return Err("GLB превышает 4 ГБ".into());
        }
        let mut output = Vec::with_capacity(length);
        for v in [0x46546c67, 2, length as u32] {
            output.extend_from_slice(&v.to_le_bytes());
        }
        chunk(&mut output, 0x4e4f534a, &json);
        chunk(&mut output, 0x004e4942, &bin);
        for (kind, bytes) in &self.extra {
            chunk(&mut output, *kind, bytes);
        }
        Ok(output)
    }
    #[cfg(test)]
    pub fn save(&mut self, objects: &[Object], original: &[Object]) -> Result<(), String> {
        self.save_with_cage(objects, original, None)
    }

    pub fn save_with_cage(
        &mut self,
        objects: &[Object],
        original: &[Object],
        cage_edits: Option<&crate::editor::model_cage_data::Edits>,
    ) -> Result<(), String> {
        if fs::read(&self.path).map_err(|e| e.to_string())? != self.expected {
            return Err("GLB изменён другой программой. Сохранение отменено; ваши правки остались в памяти.".into());
        }
        let bytes = self.encode_with_cage(objects, original, cage_edits)?;
        let parent = self.path.parent().ok_or("У файла нет родительской папки")?;
        let path = parent.join(format!(
            ".bevy-editor-model-{}.tmp",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            if fs::read(&self.path).map_err(|e| e.to_string())? != self.expected {
                return Err("Файл изменился во время сохранения".into());
            }
            fs::rename(&path, &self.path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        result?;
        self.expected = bytes;
        Ok(())
    }
}
pub(in crate::editor) fn node_matrix(value: &Value) -> Result<Mat4, String> {
    fn array<const N: usize>(v: &Value, default: [f32; N]) -> Result<[f32; N], String> {
        if v.is_null() {
            return Ok(default);
        }
        let values = v
            .as_array()
            .filter(|v| v.len() == N)
            .ok_or("Некорректная матрица или TRS GLB")?;
        let mut result = default;
        for (i, v) in values.iter().enumerate() {
            result[i] = v.as_f64().ok_or("Недопустимое значение TRS")? as f32;
        }
        Ok(result)
    }
    if !value["matrix"].is_null() {
        return Ok(Mat4::from_cols_array(&array(
            &value["matrix"],
            Mat4::IDENTITY.to_cols_array(),
        )?));
    }
    let p = Vec3::from_array(array(&value["translation"], [0.0; 3])?);
    let q = Quat::from_array(array(&value["rotation"], [0.0, 0.0, 0.0, 1.0])?);
    let s = Vec3::from_array(array(&value["scale"], [1.0; 3])?);
    Ok(Mat4::from_scale_rotation_translation(s, q, p))
}
fn align(bytes: &mut Vec<u8>, padding: u8) {
    while bytes.len() % 4 != 0 {
        bytes.push(padding);
    }
}
fn chunk(out: &mut Vec<u8>, kind: u32, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(bytes);
}
fn append_view(root: &mut Value, offset: usize, length: usize, target: u32) -> usize {
    let views = root["bufferViews"].as_array_mut().unwrap();
    let index = views.len();
    views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":length,"target":target}));
    index
}
fn append_accessor(root: &mut Value, value: Value) -> usize {
    let accessors = root["accessors"].as_array_mut().unwrap();
    let index = accessors.len();
    accessors.push(value);
    index
}
fn append_vec<const N: usize>(
    root: &mut Value,
    bin: &mut Vec<u8>,
    values: &[[f32; N]],
    kind: &str,
    bounds: bool,
) -> usize {
    align(bin, 0);
    let start = bin.len();
    for value in values {
        for v in value {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let view = append_view(root, start, bin.len() - start, 34962);
    let mut value =
        json!({"bufferView":view,"componentType":5126,"count":values.len(),"type":kind});
    if bounds {
        let mut min = [f32::INFINITY; N];
        let mut max = [f32::NEG_INFINITY; N];
        for p in values {
            for i in 0..N {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        value["min"] = json!(min.to_vec());
        value["max"] = json!(max.to_vec());
    }
    append_accessor(root, value)
}

#[cfg(test)]
#[path = "model_glb_tests.rs"]
mod tests;
