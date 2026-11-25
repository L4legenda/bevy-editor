//! Import Material Painter's MPP manifest and compressed layer data into an editable GLB.
use super::{
    model_document::{MaterialSlot, Object, PAINT_SET_EXTRA, PREVIEW_SHADER_EXTRA, glb::Glb},
    model_mesh::{Corner, Face, Geometry},
    model_paint_data::{Blend, CHANNEL_COUNT, Layer, LayerKind, MAX_LAYERS, Stack, Surface, solid},
    model_paint_spp::{ImportDetails, project_name, publish_project},
    ui::model_viewer::paint::{
        ProjectGraph, project_fill, project_graph, project_mask, project_preview_shader,
    },
};
use bevy::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::model_paint_data::texture_fill::{Bitmap, OpacitySource};
use super::{
    model_paint_data::map_fixes::{BakeMap, Fixes, Plane},
    ui::model_viewer::paint::import_levels,
};

type Result<T> = std::result::Result<T, String>;
const MAGIC: &[u8; 8] = b"MPPROJ\x00\x01";
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BLOB: usize = 256 * 1024 * 1024;
const MAX_MANIFEST: usize = 32 * 1024 * 1024;
const MAX_TEXTURES: usize = 768 * 1024 * 1024;

#[path = "model_paint_mpp_bake.rs"]
mod bake_metadata;

struct Archive {
    file: BufReader<File>,
    blobs: Vec<Range<u64>>,
}

impl Archive {
    fn open(path: &Path) -> Result<(Self, Value)> {
        let mut file = BufReader::new(File::open(path).map_err(message)?);
        let end = file.get_ref().metadata().map_err(message)?.len();
        if !(24..=MAX_FILE).contains(&end) {
            return Err("Недопустимый размер проекта Material Painter".into());
        }
        let mut magic = [0; 8];
        file.read_exact(&mut magic).map_err(message)?;
        if &magic != MAGIC {
            return Err("Некорректный заголовок MPP".into());
        }
        let len = read_u64(&mut file)?;
        if len > MAX_MANIFEST as u64 || len > end - 24 {
            return Err("Недопустимый размер описания MPP".into());
        }
        let mut manifest = vec![0; len as usize];
        file.read_exact(&mut manifest).map_err(message)?;
        let manifest: Value = serde_json::from_slice(&manifest).map_err(message)?;
        if manifest["app"] != "Material Painter"
            || !manifest["version"]
                .as_u64()
                .is_some_and(|v| (1..=3).contains(&v))
        {
            return Err("Неподдерживаемая версия проекта Material Painter".into());
        }
        let count = read_u64(&mut file)?;
        if count > 100_000 || count > (end - len - 24) / 12 {
            return Err("Недопустимое число блоков MPP".into());
        }
        let mut blobs = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let len = read_u64(&mut file)?;
            let start = file.stream_position().map_err(message)?;
            if len < 4
                || len > (MAX_BLOB + MAX_BLOB / 255 + 32) as u64
                || len > end.saturating_sub(start)
            {
                return Err("Повреждённый блок MPP".into());
            }
            blobs.push(start..start + len);
            file.seek(SeekFrom::Start(start + len)).map_err(message)?;
        }
        if file.stream_position().map_err(message)? != end {
            return Err("Некорректная длина MPP".into());
        }
        Ok((Self { file, blobs }, manifest))
    }

    fn read(
        &mut self,
        reference: &Value,
        expected: Option<usize>,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let index = reference
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .ok_or("У блока MPP нет номера")?;
        let range = self.blobs.get(index).ok_or("Блок MPP отсутствует")?;
        self.file
            .seek(SeekFrom::Start(range.start))
            .map_err(message)?;
        let mut length = [0; 4];
        self.file.read_exact(&mut length).map_err(message)?;
        let length = u32::from_le_bytes(length) as usize;
        if length > limit || expected.is_some_and(|wanted| wanted != length) {
            return Err(format!("Неверный размер блока MPP {index}"));
        }
        let mut packed = vec![0; (range.end - range.start - 4) as usize];
        self.file.read_exact(&mut packed).map_err(message)?;
        let mut raw = vec![0; length];
        let written = lz4_flex::block::decompress_into(&packed, &mut raw).map_err(message)?;
        if written != length {
            return Err(format!("Неполный блок MPP {index}"));
        }
        Ok(raw)
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn read_u64(file: &mut impl Read) -> Result<u64> {
    let mut bytes = [0; 8];
    file.read_exact(&mut bytes).map_err(message)?;
    Ok(u64::from_le_bytes(bytes))
}

struct TextureSet<'a> {
    row: &'a Value,
    triangles: Range<usize>,
    tile: Option<u32>,
}

fn texture_sets(manifest: &Value, triangles: usize) -> Result<Vec<TextureSet<'_>>> {
    if let Some(sets) = manifest["texture_sets"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        let mut result = Vec::with_capacity(sets.len().min(1024));
        if sets.len() > 1024 {
            return Err("Слишком много наборов текстур MPP".into());
        }
        let mut previous = 0;
        for row in sets {
            let [start, end]: [usize; 2] =
                serde_json::from_value(row["tris"].clone()).map_err(message)?;
            if start != previous || end <= start || end > triangles {
                return Err("Некорректные диапазоны граней в наборах MPP".into());
            }
            previous = end;
            let tile = if row["udim"].is_null() {
                None
            } else {
                let tile = row["udim"]
                    .as_u64()
                    .filter(|tile| (1001..=2000).contains(tile))
                    .ok_or("Недопустимый номер UDIM в MPP")?;
                Some(tile as u32)
            };
            result.push(TextureSet {
                row,
                triangles: start..end,
                tile,
            });
        }
        if previous != triangles {
            return Err("Наборы MPP не покрывают сетку".into());
        }
        Ok(result)
    } else {
        Ok(vec![TextureSet {
            row: manifest,
            triangles: 0..triangles,
            tile: None,
        }])
    }
}

fn geometry(
    mesh: &Value,
    vertices: &[u8],
    indices: &[u8],
    sets: &[TextureSet<'_>],
    materials: &[MaterialSlot],
    colors: Option<&[u8]>,
) -> Result<(Geometry, usize)> {
    let center =
        Vec3::from_array(serde_json::from_value(mesh["center"].clone()).unwrap_or([0.; 3]));
    let scale = mesh["scale"].as_f64().unwrap_or(1.) as f32;
    if !center.is_finite() || !scale.is_finite() || scale <= 0. {
        return Err("Некорректный масштаб сетки MPP".into());
    }
    let mut mesh = Geometry::default();
    let mut attributes = Vec::new();
    // The 48-byte vertex stores position, normal, UV and tangent in little-endian f32.
    for vertex in vertices.chunks_exact(48) {
        let value = |i: usize| f32::from_le_bytes(vertex[i * 4..i * 4 + 4].try_into().unwrap());
        let position = Vec3::new(value(0), value(1), value(2)) / scale + center;
        let normal = Vec3::new(value(3), value(4), value(5));
        let uv = Vec2::new(value(6), value(7));
        if !position.is_finite() || !normal.is_finite() || !uv.is_finite() {
            return Err("Недопустимые координаты сетки MPP".into());
        }
        mesh.positions.push(position);
        attributes.push((normal.try_normalize(), uv));
    }
    for (material, set) in sets.iter().enumerate() {
        let first = mesh.faces.len();
        let offset = materials[material]
            .udim()
            .map_or(Vec2::ZERO, |tile| tile.offset());
        for (ordinal, triangle) in indices[set.triangles.start * 12..set.triangles.end * 12]
            .chunks_exact(12)
            .enumerate()
        {
            let offset_bytes = (set.triangles.start + ordinal) * 3;
            let id_color = colors.map(|colors| {
                Vec3::from_array(std::array::from_fn(|channel| {
                    f32::from(colors[offset_bytes + channel]) / 255.
                }))
            });
            let mut corners = Vec::with_capacity(3);
            for index in triangle.chunks_exact(4) {
                let index = u32::from_le_bytes(index.try_into().unwrap()) as usize;
                let &(normal, uv) = attributes
                    .get(index)
                    .ok_or("Индекс вершины MPP вне сетки")?;
                let mut corner = Corner::new(index);
                corner.uv = uv + offset;
                corner.source_normal = normal;
                corner.id_color = id_color;
                corners.push(corner);
            }
            let face = Face {
                corners,
                material,
                smooth: false,
            };
            // Painter primitives can include collapsed triangles at UV poles.
            if mesh.has_face_area(&face) {
                mesh.faces.push(face);
            }
        }
        if mesh.faces.len() == first {
            return Err(format!(
                "В наборе MPP «{}» нет невырожденных граней",
                materials[material].source["name"]
                    .as_str()
                    .unwrap_or("Материал")
            ));
        }
    }
    mesh.validate().map_err(message)?;
    let skipped = indices.len() / 12 - mesh.faces.len();
    Ok((mesh, skipped))
}

fn reserve(remaining: &mut usize, bytes: usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or("Текстуры MPP превышают доступный бюджет импорта")?;
    Ok(())
}

// These indices are part of Material Painter's project format, independent of Bevy's map order.
const MPP_BAKE_MAPS: [BakeMap; 13] = [
    BakeMap::Occlusion,
    BakeMap::Curvature,
    BakeMap::Cavity,
    BakeMap::Thickness,
    BakeMap::Sky,
    BakeMap::EdgeDistance,
    BakeMap::CreaseDistance,
    BakeMap::PartRandom,
    BakeMap::PartGradient,
    BakeMap::BentNormal,
    BakeMap::ProjectedNormal,
    BakeMap::ProjectedHeight,
    BakeMap::PartId,
];

fn map_fixes(
    archive: &mut Archive,
    set: &Value,
    remaining: &mut usize,
    warnings: &mut Vec<String>,
) -> Result<Fixes> {
    let mut fixes = Vec::new();
    let name = set["name"].as_str().unwrap_or("Материал");
    for row in set["map_fixes"].as_array().into_iter().flatten() {
        let index = row["map"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or("Нет номера исправляемой карты MPP")?;
        let Some(map) = MPP_BAKE_MAPS
            .get(index)
            .copied()
            .filter(|map| map.supports_fix())
        else {
            warnings.push(format!(
                "«{name}» · исправление карты {index} не поддерживается"
            ));
            continue;
        };
        if fixes.iter().any(|(existing, _)| *existing == map) {
            return Err(format!("«{name}» · повторное исправление карты {index}"));
        }
        let texture = &row["texture"];
        let size = texture["width"]
            .as_u64()
            .and_then(|size| u32::try_from(size).ok())
            .filter(|&size| texture["height"].as_u64() == Some(u64::from(size)))
            .ok_or("Исправление карты MPP должно быть квадратным")?;
        let len = Plane::byte_len(size).ok_or("Недопустимое разрешение исправления карты MPP")?;
        if texture["format"] != "Rg8Unorm" {
            return Err("Неизвестный формат исправления карты MPP".into());
        }
        reserve(remaining, len)?;
        let mut pixels = archive.read(&texture["blob"], Some(len), MAX_BLOB)?;
        for pixel in pixels.chunks_exact_mut(2) {
            // Painter saves (value, coverage); Bevy blends premultiplied paint.
            pixel[0] = if map == BakeMap::ProjectedNormal {
                0
            } else {
                ((u16::from(pixel[0]) * u16::from(pixel[1]) + 127) / 255) as u8
            };
        }
        fixes.push((map, Arc::new(Plane { size, pixels })));
    }
    Ok(fixes)
}

fn map_levels(
    set: &Value,
    material: &mut MaterialSlot,
    legacy_gamma: bool,
    warnings: &mut Vec<String>,
) -> Result<usize> {
    let name = set["name"].as_str().unwrap_or("Материал");
    let mut count = 0;
    for (index, row) in set["map_levels"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let [black, white, gamma]: [f64; 3] =
            serde_json::from_value(row.clone()).map_err(message)?;
        if [black, white, gamma] == [0., 1., 1.] {
            continue;
        }
        let Some(&map) = MPP_BAKE_MAPS
            .get(index)
            .filter(|map| map.supports_fix() && **map != BakeMap::ProjectedNormal)
        else {
            warnings.push(format!("«{name}» · уровни карты {index} не поддерживаются"));
            continue;
        };
        // Material Painter 1 stores the exponent rather than its reciprocal.
        let gamma = if legacy_gamma {
            1. / gamma.max(0.001)
        } else {
            gamma
        };
        if !import_levels(material, map, [black, white, gamma])? {
            warnings.push(format!(
                "«{name}» · уровни карты {index} ограничены диапазоном редактора"
            ));
        }
        count += 1;
    }
    Ok(count)
}

fn pack(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

fn texture(
    archive: &mut Archive,
    row: &Value,
    size: u32,
    channel: Option<usize>,
) -> Result<Arc<Vec<u8>>> {
    let format = row["format"].as_str().ok_or("Нет формата текстуры MPP")?;
    let stride = match (channel, format) {
        (None, "R8Unorm") => 1,
        (None | Some(1 | 2 | 4 | 7), "Rg8Unorm") => 2,
        (Some(3), "Rg16Float") | (Some(0 | 5 | 6), "Rgba8Unorm") => 4,
        _ => return Err(format!("Неподдерживаемый формат слоя MPP «{format}»")),
    };
    if row["width"].as_u64() != Some(size.into()) || row["height"].as_u64() != Some(size.into()) {
        return Err("Разрешение слоя MPP отличается от разрешения набора".into());
    }
    let bytes = size as usize * size as usize * stride;
    let raw = archive.read(&row["blob"], Some(bytes), MAX_BLOB)?;
    match channel {
        None => Ok(Arc::new(
            raw.chunks_exact(stride)
                .flat_map(|p| [p[0], if stride == 2 { p[1] } else { 0 }, 0, 255])
                .collect(),
        )),
        Some(3) => Ok(Arc::new(
            raw.chunks_exact(4)
                .flat_map(|p| {
                    let half = |i| half::f16::from_le_bytes([p[i], p[i + 1]]).to_f32();
                    let value = pack(0.5 + 0.5 * half(0));
                    [value, value, value, pack(half(2))]
                })
                .collect(),
        )),
        Some(_) if stride == 2 => Ok(Arc::new(
            raw.chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
        )),
        _ => Ok(Arc::new(raw)),
    }
}

fn split_graph_mask_cache(
    size: u32,
    pixels: &mut Arc<Vec<u8>>,
    remaining: &mut usize,
) -> Result<Arc<Bitmap>> {
    // Material Painter saves the last graph output in the multiplier channel.
    // Keep the additive channel on the editable Bevy mask.
    let gray: Vec<u8> = Arc::make_mut(pixels)
        .chunks_exact_mut(4)
        .flat_map(|pixel| {
            let value = std::mem::replace(&mut pixel[0], 255);
            [value, value, value, 255]
        })
        .collect();
    let rgba = image::RgbaImage::from_raw(size, size, gray)
        .ok_or("Некорректный размер растрового кэша маски MPP")?;
    let bitmap = Bitmap::from_rgba("Маска MPP.png".into(), rgba)?;
    reserve(
        remaining,
        bitmap.bytes.len() + bitmap.pixels().as_raw().len(),
    )?;
    Ok(Arc::new(bitmap))
}

fn blend(value: &Value, height: bool) -> Result<Blend> {
    Ok(match value.as_str().unwrap_or("Normal") {
        "Normal" => Blend::Normal,
        "Multiply" => Blend::Multiply,
        "Screen" => Blend::Screen,
        "Overlay" => Blend::Overlay,
        "Linear Dodge (Add)" if height => Blend::AddHeight,
        "Linear Dodge (Add)" => Blend::Add,
        "Subtract" => Blend::Subtract,
        "Darken" => Blend::Darken,
        "Lighten" => Blend::Lighten,
        name => return Err(format!("Неизвестный режим смешивания «{name}»")),
    })
}

struct Graphs<'a> {
    directory: &'a Path,
    entries: HashMap<u64, &'a Value>,
    imported: HashMap<u64, Option<Arc<ProjectGraph>>>,
}

struct Layers<'a, 'b> {
    archive: &'a mut Archive,
    remaining: &'a mut usize,
    graphs: &'a mut Graphs<'b>,
    warnings: &'a mut Vec<String>,
    recipes: &'a [super::model_paint_data::custom::Recipe],
    size: u32,
    planes: HashMap<[u8; 4], Arc<Vec<u8>>>,
}

impl Layers<'_, '_> {
    fn read_graph(&mut self, reference: &Value, layer: &str) -> Option<Arc<ProjectGraph>> {
        let id = reference.as_u64()?;
        if let Some(cached) = self.graphs.imported.get(&id) {
            let graph = cached.as_ref()?;
            // GLB stores and decodes a separate bitmap for each layer reference.
            let size = graph
                .images
                .values()
                .map(|image| image.bytes.len() + image.pixels().as_raw().len())
                .sum();
            if let Err(error) = reserve(self.remaining, size) {
                self.warnings
                    .push(format!("«{layer}» · граф {id}: {error}"));
                return None;
            }
            return Some(graph.clone());
        }
        let result = self
            .graphs
            .entries
            .get(&id)
            .ok_or_else(|| "Нет данных графа".to_owned())
            .and_then(|data| {
                project_graph(
                    data,
                    self.graphs.directory,
                    id,
                    |bytes| reserve(self.remaining, bytes),
                    self.warnings,
                )
            });
        let imported = match result {
            Ok(graph) => Some(Arc::new(graph)),
            Err(error) => {
                self.warnings
                    .push(format!("«{layer}» · граф {id}: {error}"));
                None
            }
        };
        self.graphs.imported.insert(id, imported.clone());
        imported
    }

    fn read(&mut self, set: &Value) -> Result<Stack> {
        let rows = set["layers_bottom_to_top"]
            .as_array()
            .ok_or("Нет списка слоёв MPP")?;
        if rows.len() > MAX_LAYERS {
            return Err(format!("В наборе MPP больше {MAX_LAYERS} слоёв"));
        }
        let mut layers = Vec::new();
        let mut ancestors = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for row in rows.iter().rev() {
            let id = row["id"].as_u64().ok_or("У слоя MPP нет идентификатора")?;
            if !ids.insert(id) {
                return Err("Повторяющийся идентификатор слоя MPP".into());
            }
            let depth = if let Some(parent) = row["parent"].as_u64() {
                ancestors
                    .iter()
                    .position(|&id| id == parent)
                    .ok_or("Некорректная вложенность слоёв MPP")?
                    + 1
            } else {
                0
            };
            ancestors.truncate(depth);
            let content = &row["content"];
            let kind = match content["kind"].as_str() {
                Some("folder") => LayerKind::Folder,
                Some("fill") => LayerKind::Fill,
                Some("paint") => LayerKind::Paint,
                _ => return Err("Неизвестный тип слоя MPP".into()),
            };
            let name = row["name"].as_str().unwrap_or("Слой");
            let mut layer = Layer::new(name.to_owned(), kind);
            layer.graph_id = Some(id);
            layer.depth = depth;
            layer.visible = row["visible"].as_bool().unwrap_or(true);
            layer.opacity = (row["opacity"].as_f64().unwrap_or(1.).clamp(0., 1.) * 100.) as f32;
            let layer_bytes = self.size as usize * self.size as usize * 4;
            match kind {
                LayerKind::Folder => {
                    ancestors.push(id);
                    layer.collapsed = !content["open"].as_bool().unwrap_or(true);
                    if let Some(graph) = self.read_graph(&content["graph"], name) {
                        layer.folder_graph = Some(graph.payload.clone());
                        layer.folder_images = graph.images.clone();
                    }
                }
                LayerKind::Fill => {
                    let mut images = std::array::from_fn(|_| None);
                    let mut opacity = OpacitySource::Auto;
                    for (channel, row) in row["images"].as_array().into_iter().flatten().enumerate()
                    {
                        if row.is_null() {
                            continue;
                        }
                        let slot = images
                            .get_mut(channel)
                            .ok_or("Неизвестный канал изображения MPP")?;
                        let encoded = self.archive.read(&row["data"], None, 64 * 1024 * 1024)?;
                        let bitmap = Bitmap::decode(
                            row["name"].as_str().unwrap_or("Текстура").to_owned(),
                            &encoded,
                        )?;
                        reserve(
                            self.remaining,
                            encoded.len() + bitmap.pixels().as_raw().len(),
                        )?;
                        *slot = Some(Arc::new(bitmap));
                        if channel == 4 {
                            opacity = match row["source"].as_str() {
                                Some("Alpha") => OpacitySource::Alpha,
                                Some("Brightness") => OpacitySource::Brightness,
                                Some("Red") => OpacitySource::Red,
                                Some("Green") => OpacitySource::Green,
                                Some("Blue") => OpacitySource::Blue,
                                _ => OpacitySource::Auto,
                            };
                        }
                    }
                    let value = &content["fill"];
                    let graph = self.read_graph(&value["graph"], name);
                    project_fill(
                        &mut layer,
                        value,
                        images,
                        opacity,
                        graph.as_deref(),
                        |rgba| {
                            if let Some(existing) = self.planes.get(&rgba) {
                                return Ok(existing.clone());
                            }
                            reserve(self.remaining, layer_bytes)?;
                            let pixels = solid(self.size, rgba);
                            self.planes.insert(rgba, pixels.clone());
                            Ok(pixels)
                        },
                    )?;
                }
                LayerKind::Paint => {
                    for (channel, row) in content["channels"]
                        .as_array()
                        .ok_or("Нет каналов слоя MPP")?
                        .iter()
                        .enumerate()
                    {
                        if row.is_null() {
                            continue;
                        }
                        reserve(self.remaining, layer_bytes)?;
                        let slot = layer
                            .channels
                            .get_mut(channel)
                            .ok_or("Неизвестный канал слоя MPP")?;
                        *slot = Some(texture(self.archive, row, self.size, Some(channel))?);
                    }
                }
            }
            layer.blend = blend(&row["blend"], layer.channels[3].is_some())?;
            if let Some(mask) = row.get("mask").filter(|v| !v.is_null()) {
                reserve(self.remaining, layer_bytes)?;
                let cached = !mask["paint"].is_null();
                let mut pixels = if cached {
                    texture(self.archive, &mask["paint"], self.size, None)?
                } else {
                    solid(self.size, [255, 0, 0, 255])
                };
                let graph = self.read_graph(&mask["graph"], name);
                let output = (cached && graph.is_some())
                    .then(|| split_graph_mask_cache(self.size, &mut pixels, self.remaining))
                    .transpose()?;
                let mut omitted = 0;
                layer.mask = Some(project_mask(
                    mask,
                    pixels,
                    graph.as_deref(),
                    output,
                    self.recipes,
                    &mut omitted,
                )?);
                if omitted != 0 {
                    self.warnings.push(format!(
                        "«{name}» · пропущено параметров генератора: {omitted}"
                    ));
                }
            }
            layers.push(layer);
        }
        if layers.is_empty() {
            layers.push(Layer::new("Слой".into(), LayerKind::Paint));
        }
        layers.reverse();
        let selected = set["selected"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .unwrap_or(0)
            .min(layers.len() - 1);
        let stack = Stack { selected, layers };
        if !stack.valid_hierarchy() {
            return Err("Вложенность слоёв MPP выходит за пределы редактора".into());
        }
        Ok(stack)
    }
}

struct Imported {
    object: Object,
    bake: bake_metadata::Bake,
    preview: Option<Value>,
    active_set: usize,
    report: ImportDetails,
}

fn import(path: &Path, name: &str) -> Result<Imported> {
    let (mut archive, manifest) = Archive::open(path)?;
    let mesh = &manifest["mesh"];
    let vertices = archive.read(&mesh["vertices"], None, MAX_BLOB)?;
    let indices = archive.read(&mesh["indices"], None, MAX_BLOB)?;
    if vertices.is_empty()
        || !vertices.len().is_multiple_of(48)
        || indices.is_empty()
        || !indices.len().is_multiple_of(12)
    {
        return Err("Некорректные вершины или треугольники MPP".into());
    }
    let sets = texture_sets(&manifest, indices.len() / 12)?;
    let active_set = manifest["active_set"]
        .as_u64()
        .unwrap_or(0)
        .min(sets.len().saturating_sub(1) as u64) as usize;
    let materials: Vec<_> = sets.iter().map(|set| {
        let name = set.row["name"].as_str().unwrap_or("Материал");
        let mut source = json!({"name": name});
        if let Some(tile) = set.tile {
            source["extras"]["bevy_editor_udim"] = json!(tile);
        }
        MaterialSlot {
            material: Handle::default(), source, paint: None,
            authored: Some(Arc::new(json!({
                "name": name,
                "pbrMetallicRoughness": {"baseColorFactor": [0.8, 0.8, 0.8, 1.], "metallicFactor": 0., "roughnessFactor": 0.5},
            }))),
            bake_fixes: Default::default(),
        }
    }).collect();
    let colors = (!mesh["tri_color"].is_null())
        .then(|| archive.read(&mesh["tri_color"], Some(indices.len() / 4), MAX_BLOB))
        .transpose()?;
    let (geometry, skipped) = geometry(
        mesh,
        &vertices,
        &indices,
        &sets,
        &materials,
        colors.as_deref(),
    )?;
    let mut object = Object {
        id: 0,
        source_node: None,
        name: mesh["name"].as_str().unwrap_or(name).to_owned(),
        geometry: geometry.into(),
        transform: Transform::IDENTITY,
        materials,
        hidden: false,
    };
    let mut graphs = Graphs {
        directory: path.parent().unwrap_or_else(|| Path::new(".")),
        entries: manifest["graphs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| Some((entry["id"].as_u64()?, &entry["graph"])))
            .collect(),
        imported: HashMap::new(),
    };
    let mut remaining = MAX_TEXTURES;
    let mut warnings = Vec::new();
    if skipped != 0 {
        warnings.push(format!("Пропущено вырожденных треугольников: {skipped}"));
    }
    if mesh["objects"]
        .as_array()
        .is_some_and(|parts| parts.len() > 1)
    {
        warnings.push(
            "Части исходной сетки объединены в одну модель для сохранения общих наборов текстур"
                .into(),
        );
    }
    let preview_ref = &manifest["view"]["preview_shader"];
    let preview = if preview_ref.is_null() {
        None
    } else {
        let shader = preview_ref
            .as_u64()
            .and_then(|id| graphs.entries.get(&id))
            .ok_or_else(|| "Данные графа отсутствуют".to_owned())
            .and_then(|data| project_preview_shader(data));
        match shader {
            Ok(shader) => Some(shader),
            Err(error) => {
                warnings.push(format!("Шейдер предпросмотра · {error}"));
                None
            }
        }
    };
    let bake = bake_metadata::Bake::read(path, &object, mesh, &manifest["bake"], &mut warnings);
    let mut layer_count = 0;
    let mut fix_count = 0;
    let mut level_count = 0;
    let legacy_gamma = manifest["version"].as_u64() == Some(1);
    for (slot, set) in sets.iter().enumerate() {
        let size = set.row["resolution"].as_u64().unwrap_or(2048);
        if !(128..=4096).contains(&size) || !size.is_power_of_two() {
            return Err(format!("Неподдерживаемое разрешение набора MPP: {size}"));
        }
        let size = size as u32;
        reserve(
            &mut remaining,
            size as usize * size as usize * 4 * CHANNEL_COUNT,
        )?;
        let stack = Layers {
            archive: &mut archive,
            remaining: &mut remaining,
            graphs: &mut graphs,
            warnings: &mut warnings,
            recipes: &bake.recipes,
            size,
            planes: HashMap::new(),
        }
        .read(set.row)?;
        layer_count += stack.layers.len();
        let fixes = map_fixes(&mut archive, set.row, &mut remaining, &mut warnings)?;
        fix_count += fixes.len();
        object.materials[slot].bake_fixes = Arc::new(fixes);
        level_count += map_levels(
            set.row,
            &mut object.materials[slot],
            legacy_gamma,
            &mut warnings,
        )?;
        let mut surface = Surface {
            size,
            stack,
            base: [
                [204, 204, 204, 255],
                [128, 128, 128, 255],
                [0, 0, 0, 255],
                [128, 128, 128, 255],
                [255; 4],
                [0, 0, 0, 255],
                [128, 128, 255, 255],
                [128, 128, 128, 255],
            ]
            .map(|rgba| solid(size, rgba)),
            material: object.materials[slot].authored.as_deref().unwrap().clone(),
            baked: None,
        };
        let offset = object.uv_offset(slot);
        surface.bind_texture_fills(&object.geometry, slot, offset, object.transform);
        surface.bind_generators(&object.geometry, slot, offset, object.transform);
        object.materials[slot].paint = Some(Arc::new(surface));
    }
    let mut overview = format!("MPP · наборов: {} · слоёв: {layer_count}", sets.len());
    if fix_count + level_count != 0 {
        overview.push_str(&format!(
            " · исправлений карт: {fix_count} · настроек уровней: {level_count}"
        ));
        warnings.push("Правки будут видны после запекания карт в Bevy Editor".into());
    }
    let label = if warnings.is_empty() {
        overview.clone()
    } else {
        format!("{overview} · замечаний: {}", warnings.len())
    };
    Ok(Imported {
        object,
        bake,
        preview,
        active_set,
        report: ImportDetails {
            label,
            overview,
            warnings,
        },
    })
}

pub(super) fn convert(source: &Path, target: &Path) -> io::Result<(PathBuf, ImportDetails)> {
    let name = project_name(source);
    let Imported {
        object,
        bake,
        preview,
        active_set,
        report,
    } = import(source, &name).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let model = target.join("MPP.glb");
    let bytes = Glb::new_asset_configured(
        &[object],
        &[],
        |root| {
            root["extras"][PAINT_SET_EXTRA] = json!(active_set);
            if let Some(preview) = preview {
                root["extras"][PREVIEW_SHADER_EXTRA] = preview;
            }
            bake.write(root, &model)
        },
        Some(&bake.cage),
    )
    .map_err(io::Error::other)?;
    Ok((publish_project(target, &name, "MPP", &bytes)?, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::browser_files;

    fn native_file(manifest: Value, raw_blobs: &[Vec<u8>]) -> Vec<u8> {
        let text = serde_json::to_vec(&manifest).unwrap();
        let mut file = MAGIC.to_vec();
        file.extend((text.len() as u64).to_le_bytes());
        file.extend(text);
        file.extend((raw_blobs.len() as u64).to_le_bytes());
        for raw in raw_blobs {
            let blob = lz4_flex::block::compress_prepend_size(raw);
            file.extend((blob.len() as u64).to_le_bytes());
            file.extend(blob);
        }
        file
    }

    #[test]
    fn mpp_browser_import_restores_sets_and_editable_layers() {
        use material_painter_graph::{Graph, GraphParam, Library, Value as ParamValue};

        let directory =
            std::env::temp_dir().join(format!("bevy-mpp-{}", bevy::asset::uuid::Uuid::new_v4()));
        let source = directory.join("Материал.mpp");
        let assets = directory.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(directory.join("high.obj"), "o high\n").unwrap();
        let set_size = 128;
        let texels = (set_size * set_size) as usize;
        let vertices: Vec<_> = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [2., 0., 0.],
            [3., 0., 0.],
            [2., 1., 0.],
        ]
        .into_iter()
        .enumerate()
        .flat_map(|(i, [x, y, z])| {
            let [u, v] = [[0., 0.], [1., 0.], [0., 1.]][i % 3];
            [x, y, z, 0., 0., 1., u, v, 1., 0., 0., 1.]
                .into_iter()
                .flat_map(f32::to_le_bytes)
        })
        .collect();
        let indices: Vec<_> = (0..6u32).flat_map(u32::to_le_bytes).collect();
        let height: Vec<_> = [half::f16::from_f32(-0.5), half::f16::from_f32(0.5)]
            .into_iter()
            .flat_map(half::f16::to_le_bytes)
            .collect();
        let bitmap = Bitmap::from_rgba(
            "Tint.png".into(),
            image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 25, 50, 255])),
        )
        .unwrap();
        let texture = directory.join("Graph.png");
        std::fs::write(&texture, &bitmap.bytes).unwrap();
        let library = Library::load(None);
        let mut graph = Graph::new("Tint graph");
        let node = graph.add("bitmap", [0., 0.]);
        let image_ref = ParamValue::Image(Some("Graph.png".into()));
        graph
            .node_mut(node)
            .unwrap()
            .params
            .insert("image".into(), image_ref.clone());
        graph.params.push(GraphParam {
            name: "Tint bitmap".into(),
            group: "Textures".into(),
            binds: vec![(node, "image".into())],
            default: Some(image_ref),
        });
        let missing_node = graph.add("bitmap", [0., 120.]);
        graph.node_mut(missing_node).unwrap().params.insert(
            "image".into(),
            ParamValue::Image(Some("Missing.png".into())),
        );
        let output = graph.add("output", [300., 0.]);
        assert!(graph.connect(node, output, "input"));
        let graph = graph.to_json(&library);
        let preview = material_painter_graph::shading::starter("Project look");
        let preview = preview.to_json(&library);
        let texture = |blob, format| json!({"blob": blob, "format": format, "width": set_size, "height": set_size});
        let manifest = json!({
            "app": "Material Painter", "version": 3,
            "active_set": 1,
            "graphs": [{"id": 4, "graph": graph}, {"id": 5, "graph": preview}],
            "view": {"preview_shader": 5},
            "mesh": {"name": "Panels", "vertices": 0, "indices": 1, "center": [1, 2, 3], "scale": 2, "tri_color": 7},
            "bake": {
                "quality": "High", "high_poly": "high.obj", "cage": "missing.fbx",
                "front": 0.2, "rear": 0.3, "maps": 547,
                "fill_misses": false, "match_by_name": false,
                "ignore_backfaces": false, "average_normals": false,
                "id_source": "VertexColors",
                "cage_paint": [0.8, 1, 1, 1, 1, 1],
                "custom": [{"id": 20, "name": "Wear", "kind": "Point", "distance": 0.75, "point": [0.3, 0.6, 0.0]}],
            },
            "texture_sets": [
                {
                    "name": "Panel.1001", "tris": [0, 1], "udim": 1001, "resolution": set_size,
                    "selected": 1,
                    "map_fixes": [
                        {"map": 0, "texture": texture(6, "Rg8Unorm")},
                        {"map": 10, "texture": texture(6, "Rg8Unorm")},
                    ],
                    "map_levels": [[0.1, 0.8, 1.5], [0.2, 0.9, 0.75]],
                    "layers_bottom_to_top": [
                        {"id": 10, "name": "Tint", "content": {"kind": "fill", "fill": {"color": [1,1,1], "graph": 4}},
                            "images": [{"name": "Tint.png", "data": 5, "source": "Auto"}]},
                        {"id": 11, "parent": 20, "name": "Strokes", "opacity": 0.75,
                            "content": {"kind":"paint", "channels": [null, texture(2, "Rg8Unorm"), null, texture(3, "Rg16Float")]},
                            "mask": {"paint": texture(4, "Rg8Unorm"), "filters": [{"kind": "invert"}]}},
                        {"id": 20, "name": "Details", "content": {"kind":"folder", "open": false, "graph": 4}},
                    ],
                },
                {"name": "Panel.1013", "tris": [1, 2], "udim": 1013, "resolution": set_size,
                    "layers_bottom_to_top": [{"id": 1, "name": "Base", "content": {"kind":"paint", "channels": []},
                        "mask": {"graph": 4, "paint": texture(4, "Rg8Unorm"),
                            "generator": {"kind": "Dust", "custom": [{"id": 20, "weight": 0.5}]}}}]},
            ],
        });
        let broken = directory.join("Broken.mpp");
        let bytes = native_file(
            manifest,
            &[
                vertices,
                indices,
                [105, 240].repeat(texels),
                height.repeat(texels),
                [128, 32].repeat(texels),
                bitmap.bytes,
                [200, 128].repeat(texels),
                vec![127, 10, 35, 25, 150, 220],
            ],
        );
        std::fs::write(&source, &bytes).unwrap();
        std::fs::write(&broken, &bytes[..bytes.len() - 1]).unwrap();
        let report = browser_files::import_files(
            &assets,
            Path::new(""),
            vec![source.clone(), broken],
            Default::default(),
        );
        assert_eq!(
            report.imported,
            [Path::new("Материал_MPP.glb")],
            "{}",
            report.status()
        );
        assert_eq!(report.errors.len(), 1);
        assert!(
            report
                .paint_project_details()
                .unwrap()
                .contains("MPP · наборов: 2 · слоёв: 4")
        );
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        let Imported {
            object,
            report: imported,
            ..
        } = import(&source, "Fallback").unwrap();
        assert_eq!(
            imported
                .warnings
                .iter()
                .filter(|warning| warning.contains("Missing.png"))
                .count(),
            1
        );
        std::fs::remove_file(directory.join("Graph.png")).unwrap();
        assert_eq!(object.geometry.positions[0], Vec3::new(1., 2., 3.));
        assert_eq!(object.geometry.faces[1].corners[0].uv, Vec2::new(2., -1.));
        assert_eq!(object.materials[1].udim().unwrap().number(), 1013);

        let glb = Glb::load(&assets.join(&report.imported[0])).unwrap();
        assert_eq!(glb.json["extras"][PAINT_SET_EXTRA], 1);
        let preview = &glb.json["extras"][PREVIEW_SHADER_EXTRA];
        assert_eq!(preview["name"], "Project look");
        let shader = Graph::from_json(preview, &library).unwrap();
        assert!(material_painter_graph::shading::compile(&shader, &library).is_ok());
        let settings = super::super::model_bake_settings::read(&glb.json, &glb.path);
        assert_eq!(
            settings.quality,
            super::super::model_bake_settings::Quality::High
        );
        assert_eq!(
            (settings.front, settings.rear, settings.map_bits),
            (6, 9, 49)
        );
        assert!(!settings.fill_misses && !settings.match_names);
        assert!(!settings.prefer_front_faces && !settings.average_normals);
        assert_eq!(
            settings.id_source,
            super::super::model_bake_settings::IdSource::VertexColors
        );
        let scene = bevy::gltf::gltf::Gltf::from_slice(glb.data()).unwrap();
        let bin = scene.blob.as_deref().unwrap();
        let mesh = scene.meshes().next().unwrap();
        for (primitive, rgb) in mesh.primitives().zip([[127u8, 10, 35], [25, 150, 220]]) {
            let colors = super::super::model_document::glb::read_id_colors(
                &scene.document,
                &primitive,
                bin,
                3,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                colors,
                vec![Some(Vec3::from_array(rgb.map(|v| f32::from(v) / 255.))); 3]
            );
            let reader = primitive.reader(|_| Some(bin));
            assert!(
                reader
                    .read_colors(0)
                    .unwrap()
                    .into_rgba_f32()
                    .all(|rgba| rgba == [1.; 4])
            );
        }
        assert_eq!(
            settings.high_poly.unwrap(),
            directory.join("high.obj").canonicalize().unwrap()
        );
        assert!(settings.cage.is_none());
        assert!(
            report
                .paint_project_details()
                .unwrap()
                .contains("missing.fbx")
        );
        let cage = super::super::model_cage_data::read(&glb.json["nodes"][0], &object).unwrap();
        assert!((cage.scales.values[0] - 0.8).abs() < 0.0001);
        let recipes = super::super::model_paint_data::custom::read_document(&glb.json);
        assert_eq!(recipes.len(), 1);
        assert_eq!(
            (recipes[0].id, recipes[0].distance, recipes[0].point),
            (20, 50, [-80, 7, 0])
        );
        let primitives = glb.json["meshes"][0]["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(), 2);
        assert_eq!(
            primitives[0]["extras"]["bevy_editor_bake_levels"],
            json!([
                ["occlusion", {"black": 10, "white": 80, "gamma": 150}],
                ["curvature", {"black": 20, "white": 90, "gamma": 75}],
            ])
        );
        let fixes = glb.bake_fixes(&primitives[0]).unwrap();
        assert_eq!(fixes.len(), 2);
        assert_eq!(fixes[0].0, BakeMap::Occlusion);
        assert_eq!(fixes[0].1.size, set_size);
        assert_eq!(&fixes[0].1.pixels[..2], &[100, 128]);
        assert_eq!(fixes[1].0, BakeMap::ProjectedNormal);
        assert_eq!(&fixes[1].1.pixels[..2], &[0, 128]);
        assert!(glb.bake_fixes(&primitives[1]).unwrap().is_empty());
        assert!(primitives[1]["extras"]["bevy_editor_bake_levels"].is_null());
        let first = glb.paint_surface(&primitives[0]).unwrap().unwrap();
        assert_eq!(first.stack.selected, 1);
        let rows = &first.stack.layers;
        assert_eq!(
            rows.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["Tint", "Strokes", "Details"]
        );
        assert_eq!(
            rows[0].texture_fill.as_ref().unwrap().images[0]
                .as_ref()
                .unwrap()
                .pixels()
                .get_pixel(0, 0)
                .0,
            [200, 25, 50, 255]
        );
        assert!(rows[2].collapsed);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].opacity, 75.);
        assert_eq!(
            &rows[1].channels[1].as_ref().unwrap()[..4],
            &[105, 105, 105, 240]
        );
        assert_eq!(
            &rows[1].channels[3].as_ref().unwrap()[..4],
            &[64, 64, 64, 128]
        );
        let mask = rows[1].mask.as_ref().unwrap();
        assert_eq!(&mask.pixels[..4], &[128, 32, 0, 255]);
        assert_eq!(mask.effective()[0], 95);
        let second = glb.paint_surface(&primitives[1]).unwrap().unwrap();
        assert_eq!(second.stack.layers[0].name, "Base");
        let mask = second.stack.layers[0].mask.as_ref().unwrap();
        let generator = mask.generator.unwrap();
        let mask_graph = mask.graph.as_ref().unwrap();
        assert_eq!(&mask.pixels[..4], &[255, 32, 0, 255]);
        assert_eq!(
            mask_graph
                .output
                .as_ref()
                .unwrap()
                .pixels()
                .get_pixel(0, 0)
                .0,
            [128, 128, 128, 255]
        );
        assert_eq!(mask.effective()[0], 160);
        assert_eq!(mask_graph.payload["bevy_mpp_graph_keep_cache"], true);
        let fill = rows[0].texture_fill.as_ref().unwrap();
        let key = "graph-images/mpp-4/1.png";
        for (payload, images) in [
            (fill.graph.as_ref().unwrap(), &fill.graph_images),
            (
                rows[2].folder_graph.as_ref().unwrap(),
                &rows[2].folder_images,
            ),
            (&mask_graph.payload, &mask_graph.images),
        ] {
            assert_eq!(images.len(), 1);
            assert_eq!(images[key].pixels().get_pixel(0, 0).0, [200, 25, 50, 255]);
            let reopened = Graph::from_json(payload, &library).unwrap();
            assert_eq!(
                reopened.node(node).unwrap().params["image"],
                ParamValue::Image(Some(key.into()))
            );
            assert_eq!(
                reopened.params[0].default,
                Some(ParamValue::Image(Some(key.into())))
            );
            assert_eq!(
                reopened.node(missing_node).unwrap().params["image"],
                ParamValue::Image(None)
            );
        }
        assert_eq!(
            (generator.custom[0].id, generator.custom[0].weight),
            (20, 50)
        );
        let mut object = object;
        object.source_node = Some(0);
        let path = glb.path.clone();
        let mut doc = super::super::model_document::Document::new(glb, vec![object]);
        assert_eq!(doc.paint_set_index(), 1);
        assert!(!doc.dirty());
        Arc::make_mut(doc.preview_shader.as_mut().unwrap())["name"] = json!("Updated look");
        assert!(doc.dirty());
        doc.save().unwrap();
        assert!(!doc.dirty());
        let reopened = Glb::load(&path).unwrap();
        assert_eq!(
            reopened.json["extras"][PREVIEW_SHADER_EXTRA]["name"],
            "Updated look"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn old_mpp_restores_the_single_set_and_legacy_grayscale_mask() {
        let path = std::env::temp_dir().join(format!(
            "bevy-mpp-v1-{}.mpp",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let vertices: Vec<_> = [[0., 0.], [1., 0.], [0., 1.]]
            .into_iter()
            .flat_map(|[x, y]| {
                [x, y, 0., 0., 0., 1., x, y, 1., 0., 0., 1.]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
            })
            .collect();
        let manifest = json!({
            "app": "Material Painter", "version": 1, "active_set": u64::MAX,
            "view": {"preview_shader": 42},
            "mesh": {"vertices": 0, "indices": 1},
            "resolution": 128,
            "map_levels": [[0.05, 0.9, 0.5]],
            "layers_bottom_to_top": [{
                "id": 1, "name": "Legacy", "content": {"kind": "fill", "fill": {"enabled": [true, true, true, false]}},
                "mask": {"paint": {"blob": 2, "format": "R8Unorm", "width": 128, "height": 128}}
            }]
        });
        std::fs::write(
            &path,
            native_file(
                manifest,
                &[
                    vertices,
                    [0u32, 1, 2, 0, 0, 1]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect(),
                    vec![96; 128 * 128],
                ],
            ),
        )
        .unwrap();
        let Imported {
            object,
            report,
            active_set,
            ..
        } = import(&path, "Legacy").unwrap();
        assert_eq!(active_set, 0);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("Данные графа отсутствуют"))
        );
        assert_eq!(object.geometry.faces.len(), 1);
        assert_eq!(
            object.materials[0].source["extras"]["bevy_editor_bake_levels"],
            json!([
                ["occlusion", {"black": 5, "white": 90, "gamma": 200}],
            ])
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("треугольников: 1"))
        );
        let mask = object.materials[0].paint.as_ref().unwrap().stack.layers[0]
            .mask
            .as_ref()
            .unwrap();
        assert!(mask.affine);
        assert_eq!(&mask.pixels[..4], &[96, 0, 0, 255]);
        assert_eq!(mask.effective()[0], 96);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires an MPP file saved by Material Painter"]
    fn mpp_import_native_sample_file() {
        let source =
            PathBuf::from(std::env::var_os("BEVY_MPP_SAMPLE").expect("set BEVY_MPP_SAMPLE"));
        let directory = std::env::temp_dir().join(format!(
            "bevy-mpp-native-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        let Imported { object, .. } = import(&source, "Native").unwrap();
        if source
            .file_stem()
            .is_some_and(|stem| stem == "folder_graph")
        {
            assert!(
                object
                    .materials
                    .iter()
                    .filter_map(|slot| slot.paint.as_ref())
                    .flat_map(|surface| &surface.stack.layers)
                    .any(|layer| layer.folder_graph.is_some())
            );
        }
        let (file, report) = convert(&source, &directory).unwrap();
        println!("{}", report.label);
        let glb = Glb::load(&directory.join(file)).unwrap();
        let primitives = glb.json["meshes"][0]["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(), object.materials.len());
        for (primitive, slot) in primitives.iter().zip(&object.materials) {
            let reopened = glb.paint_surface(primitive).unwrap().unwrap();
            let original = slot.paint.as_ref().unwrap();
            assert_eq!(reopened.size, original.size);
            assert_eq!(
                reopened
                    .stack
                    .layers
                    .iter()
                    .map(|l| &l.name)
                    .collect::<Vec<_>>(),
                original
                    .stack
                    .layers
                    .iter()
                    .map(|l| &l.name)
                    .collect::<Vec<_>>()
            );
            assert!(reopened.stack.valid_hierarchy());
            for (layer, previous) in reopened.stack.layers.iter().zip(&original.stack.layers) {
                assert_eq!(layer.graph_id, previous.graph_id);
                assert_eq!(layer.folder_graph, previous.folder_graph);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
