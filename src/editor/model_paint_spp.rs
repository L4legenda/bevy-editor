//! Import Substance Painter geometry and the cached appearance into an editable GLB.
use super::{
    model_document::{MaterialSlot, Object, glb::Glb, udim::Tile},
    model_mesh::{Corner, Face, Geometry},
    model_paint_data::{
        Layer, LayerKind, Stack, Surface,
        mask::{Generator, INPUT_CHANNELS},
        solid,
    },
    ui::model_viewer::paint::{PolygonReplay, Stencil, StencilCache, StrokeCoverage, import_generator_inputs},
};
use bevy::prelude::*;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use subcraft_spp::document::ActionKind;
use subcraft_spp::{
    FileReader,
    document::{TextureSetInfo, channel_bits},
    images::{CacheFormat, CacheImage},
};

#[path = "model_paint_spp_layers.rs"]
mod rebuild;

const CHANNELS: [(u64, &str, CacheFormat); 7] = [
    (channel_bits::BASE_COLOR, "цвет", CacheFormat::Srgb8Alpha8),
    (channel_bits::ROUGHNESS, "шероховатость", CacheFormat::Rg8),
    (channel_bits::METALLIC, "металличность", CacheFormat::Rg8),
    (channel_bits::HEIGHT, "высота", CacheFormat::Rg16f),
    (channel_bits::OPACITY, "непрозрачность", CacheFormat::Rg8),
    (channel_bits::EMISSIVE, "свечение", CacheFormat::Srgb8Alpha8),
    (channel_bits::NORMAL, "нормали", CacheFormat::Rgba16f),
];
const MAX_TEXTURE_BYTES: usize = 1024 * 1024 * 1024;
const MIN_TEXTURE_SIZE: u32 = 128;
const MAX_IMPORT_BAKE_SIZE: u32 = 512;
const BASE_COLORS: [[u8; 4]; 6] = [
    [184, 189, 199, 255],
    [191, 191, 191, 255],
    [0, 0, 0, 255],
    [128, 128, 128, 255],
    [255; 4],
    [128, 128, 255, 255],
];
const BASE_CHANNELS: [usize; 8] = [0, 1, 2, 3, 4, 2, 5, 3];

fn plane_bytes(size: u32) -> usize {
    size as usize * size as usize * 4
}

#[derive(Default)]
struct TextureCache {
    base: BTreeMap<u32, [Arc<Vec<u8>>; 8]>,
    height_patterns: rebuild::HeightPatterns,
}

impl TextureCache {
    fn base_plane_count(&self, size: u32) -> usize {
        if self.base.contains_key(&size) { 0 } else { BASE_COLORS.len() }
    }

    fn base(&mut self, size: u32) -> [Arc<Vec<u8>>; 8] {
        self.base.entry(size).or_insert_with(|| {
            let planes = BASE_COLORS.map(|color| solid(size, color));
            BASE_CHANNELS.map(|index| planes[index].clone())
        }).clone()
    }
}

fn generator_input_bytes(size: u32) -> usize {
    size as usize * size as usize * (INPUT_CHANNELS + 3)
}

#[derive(Default)]
struct Summary {
    meshes: usize,
    sets: usize,
    tiles: usize,
    channels: usize,
    groups: usize,
    fills: usize,
    masks: usize,
    polygon_fills: usize,
    stroke_actions: usize,
    mesh_masks: usize,
    mask_effects: usize,
    triplanar_mask_bitmaps: usize,
    tuned_mask_generators: usize,
    gradient_masks: usize,
    shape_masks: usize,
    tile_masks: usize,
    height_adjustments: usize,
    procedural_height_sources: usize,
    baked_mask_filters: usize,
    baked_mask_generators: usize,
    baked_generator_sets: usize,
    anchor_references: usize,
    unrestored: usize,
    partial: usize,
    notes: Vec<String>,
    layer_issues: Vec<String>,
}

pub(super) struct ImportDetails {
    pub label: String,
    pub overview: String,
    pub warnings: Vec<String>,
}

impl Summary {
    fn mask_generator(&mut self, generator: Generator) {
        self.tuned_mask_generators += usize::from(generator != Generator::new(generator.kind));
        self.gradient_masks += usize::from(generator.kind.is_gradient());
        self.shape_masks += usize::from(generator.kind.is_shape());
        self.tile_masks += usize::from(generator.kind == crate::editor::model_paint_data::mask::GeneratorKind::TilePattern);
    }

    fn overview(&self) -> String {
        let mut summary = format!(
            "SPP · сеток: {} · наборов: {} · карт: {}",
            self.meshes, self.sets, self.channels,
        );
        if self.tiles > 0 {
            summary.push_str(&format!(" · плиток UDIM: {}", self.tiles));
        }
        if self.groups + self.fills + self.masks > 0 {
            summary.push_str(&format!(
                " · групп: {} · заливок: {} · масок: {}",
                self.groups, self.fills, self.masks
            ));
        }
        if self.polygon_fills > 0 {
            summary.push_str(&format!(" · заливок по геометрии: {}", self.polygon_fills));
        }
        if self.stroke_actions > 0 {
            summary.push_str(&format!(
                " · действий кисти и ластика: {}",
                self.stroke_actions
            ));
        }
        if self.mesh_masks + self.mask_effects > 0 {
            summary.push_str(&format!(
                " · геометрических масок: {} · эффектов маски: {}",
                self.mesh_masks, self.mask_effects
            ));
        }
        if self.triplanar_mask_bitmaps > 0 {
            summary.push_str(&format!(" · трипланарных растровых масок: {}", self.triplanar_mask_bitmaps));
        }
        if self.tuned_mask_generators > 0 {
            summary.push_str(&format!(" · генераторов с параметрами: {}", self.tuned_mask_generators));
        }
        if self.gradient_masks > 0 {
            summary.push_str(&format!(" · градиентных масок: {}", self.gradient_masks));
        }
        if self.shape_masks > 0 {
            summary.push_str(&format!(" · масок с фигурами: {}", self.shape_masks));
        }
        if self.tile_masks > 0 {
            summary.push_str(&format!(" · плиточных масок: {}", self.tile_masks));
        }
        if self.height_adjustments > 0 {
            summary.push_str(&format!(" · поправок высоты: {}", self.height_adjustments));
        }
        if self.procedural_height_sources > 0 {
            summary.push_str(&format!(
                " · приближённых процедурных карт высоты: {}", self.procedural_height_sources,
            ));
        }
        if self.baked_mask_filters > 0 {
            summary.push_str(&format!(
                " · фильтров сведено в основу маски: {}",
                self.baked_mask_filters
            ));
        }
        if self.baked_mask_generators > 0 {
            summary.push_str(&format!(
                " · генераторов сведено в основу маски: {}",
                self.baked_mask_generators
            ));
        }
        if self.baked_generator_sets > 0 {
            summary.push_str(&format!(
                " · наборов с запечёнными картами: {}",
                self.baked_generator_sets
            ));
        }
        if self.anchor_references > 0 {
            summary.push_str(&format!(" · ссылок на якоря: {}", self.anchor_references));
        }
        if self.unrestored + self.partial > 0 {
            summary.push_str(&format!(
                " · слоёв частично: {} · без содержимого: {}",
                self.partial, self.unrestored
            ));
        }
        summary
    }

    fn details(mut self) -> ImportDetails {
        if self.triplanar_mask_bitmaps > 0 {
            self.notes.push("Положение трипланарной проекции масок приближённо восстановлено по сетке".into());
        }
        if self.gradient_masks > 0 {
            self.notes.push("Кривые градиентных масок и затухание объёмной проекции восстановлены приближённо".into());
        }
        if self.shape_masks > 0 {
            self.notes.push("Контуры, мягкость и затухание проекции фигур восстановлены приближённо".into());
        }
        if self.tile_masks > 0 {
            self.notes.push("Формы плиток, случайный разброс и затухание проекции восстановлены приближённо".into());
        }
        let overview = self.overview();
        let mut label = overview.clone();
        if let Some(note) = self.notes.first() {
            label.push_str(&format!(" · {note}"));
            if self.notes.len() > 1 {
                label.push_str(&format!(" (ещё {})", self.notes.len() - 1));
            }
        }
        ImportDetails {
            label,
            overview,
            warnings: self.notes.into_iter().chain(self.layer_issues).collect(),
        }
    }
}

pub(super) fn convert(source: &Path, target: &Path) -> io::Result<(PathBuf, ImportDetails)> {
    let name = project_name(source);
    let (model, summary) = import(source, &name)?;
    let bytes = Glb::new_asset(&[model]).map_err(io::Error::other)?;
    let file = publish_project(target, &name, "SPP", &bytes)?;
    Ok((file, summary.details()))
}

pub(super) fn project_name(source: &Path) -> String {
    let name = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Painter");
    let name: String = name
        .chars()
        .take(80)
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim();
    if name.is_empty() { "Painter" } else { name }.to_owned()
}

pub(super) fn publish_project(target: &Path, name: &str, format: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    use bevy_scene_format::{Revision, atomic_write};
    for number in 1.. {
        let file = if number == 1 {
            format!("{name}_{format}.glb")
        } else {
            format!("{name}_{format}_{number}.glb")
        };
        let path = target.join(&file);
        if path.exists() {
            continue;
        }
        match atomic_write(&path, bytes, Revision::Missing) {
            Ok(_) => return Ok(PathBuf::from(file)),
            Err(_) if path.exists() => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn import(path: &Path, name: &str) -> io::Result<(Object, Summary)> {
    let mut file = FileReader::open(path).map_err(io::Error::other)?;
    let project = file.project().map_err(io::Error::other)?;
    let (meshes, material_names) = file.meshes().map_err(io::Error::other)?;
    let mut summary = Summary {
        meshes: meshes.len(),
        ..Summary::default()
    };
    let mut geometry = Geometry::default();
    let mut slots = BTreeMap::new();
    let mut materials = Vec::new();
    let mut assigned = HashSet::new();
    let mut matched_sets = Vec::new();
    let mut parts = Vec::new();
    let mut remaining = MAX_TEXTURE_BYTES;
    let mut textures = TextureCache::default();
    for mesh in meshes {
        if mesh.triangles.is_empty() {
            continue;
        }
        let slot = if let Some(&slot) = slots.get(&mesh.material) {
            slot
        } else {
            let index = mesh.material as usize;
            let material = material_names.get(index).map(String::as_str);
            let matched = match material {
                Some(name) => project.texture_sets.iter().find(|set| set.name == name),
                None => project.texture_sets.get(index),
            }
            .or_else(|| (project.texture_sets.len() == 1).then(|| &project.texture_sets[0]));
            let fallback = format!("Материал {}", materials.len() + 1);
            let name = matched
                .map(|set| set.name.as_str())
                .or(material)
                .unwrap_or(&fallback);
            let authored = json!({
                "name": name, "doubleSided": true,
                "pbrMetallicRoughness": {"baseColorFactor": [0.72, 0.74, 0.78, 1.], "metallicFactor": 0., "roughnessFactor": 0.75}
            });
            if let Some(set) = matched {
                assigned.insert(set.uid);
            }
            if matched.is_none() {
                summary.notes.push(format!("нет набора текстур «{name}»"));
            }
            let slot = materials.len();
            materials.push(MaterialSlot {
                bake_fixes: Default::default(),
                material: Handle::default(),
                source: json!({"name": name}),
                paint: None,
                authored: Some(Arc::new(authored)),
            });
            matched_sets.push(matched);
            slots.insert(mesh.material, slot);
            slot
        };
        let start = geometry.positions.len();
        let first_face = geometry.faces.len();
        geometry.positions.extend(
            mesh.positions
                .iter()
                .map(|p| Vec3::from_array(p.to_array())),
        );
        for triangle in &mesh.triangles {
            let corners = triangle
                .iter()
                .map(|&vertex| {
                    let index = vertex as usize;
                    let mut corner = Corner::new(start + index);
                    if let Some(uv) = mesh.uvs.get(index) {
                        corner.uv = Vec2::new(uv.x, 1. - uv.y);
                    }
                    if let Some(n) = mesh.normals.get(index) {
                        corner.source_normal = Vec3::from_array(n.to_array()).try_normalize();
                    }
                    corner
                })
                .collect();
            geometry.faces.push(Face {
                corners,
                material: slot,
                smooth: false,
            });
        }
        parts.push((mesh.name, first_face..geometry.faces.len()));
    }
    if geometry.faces.is_empty() {
        return Err(io::Error::other("В проекте SPP нет сетки"));
    }
    geometry.validate().map_err(io::Error::other)?;
    split_tiles(
        &mut geometry,
        &mut materials,
        &mut matched_sets,
        &mut summary,
    );
    summary.sets = assigned.len();
    if assigned.len() != project.texture_sets.len() {
        summary.notes.push(format!(
            "наборов без соответствующей сетки: {}",
            project.texture_sets.len() - assigned.len()
        ));
    }
    let mut object = Object {
        id: 0,
        source_node: None,
        name: name.to_owned(),
        geometry: geometry.into(),
        transform: Transform::IDENTITY,
        materials,
        hidden: false,
    };
    let polygon_replay = matched_sets
        .iter()
        .flatten()
        .any(|set| needs_replay(&set.layers))
        .then(|| PolygonReplay::new(&object, parts));
    let surfaces: Vec<_> = object
        .materials
        .iter()
        .zip(&matched_sets)
        .enumerate()
        .map(|(slot, (material, set))| {
            let set = (*set)?;
            let tile = material.udim();
            if set.use_uv_tiles && tile.is_none() {
                return None;
            }
            let mut generators = Vec::new();
            rebuild::mask_generators(&set.layers, &mut generators);
            let per_material = !generators.is_empty() || needs_replay(&set.layers);
            generators.retain(|generator| generator.needs_bake());
            Some(((set.uid, tile, per_material.then_some(slot)), generators))
        })
        .collect();
    let mut pending: BTreeSet<_> = surfaces.iter().flatten().map(|(key, _)| *key).collect();
    let mut shared_surfaces = BTreeMap::new();
    for (slot, (set, surface)) in matched_sets.into_iter().zip(surfaces).enumerate() {
        if let (Some(set), Some((key, generators))) = (set, surface) {
            let paint = shared_surfaces
                .entry(key)
                .or_insert_with(|| {
                    pending.remove(&key);
                    // Leave space for at least the smallest surface in each later texture set.
                    let minimum = plane_bytes(MIN_TEXTURE_SIZE) * (BASE_COLORS.len() + CHANNELS.len())
                        + generator_input_bytes(MIN_TEXTURE_SIZE);
                    let reserved = pending
                        .len()
                        .saturating_mul(minimum)
                        .min(remaining.saturating_sub(minimum));
                    let mut available = remaining - reserved;
                    let surface = decode_surface(
                        &mut file,
                        set,
                        (&mut available, &mut textures),
                        &mut summary,
                        (&object, slot, polygon_replay.as_ref()),
                        key.1,
                        &generators,
                    );
                    remaining = available + reserved;
                    surface.map(Arc::new)
                })
                .clone();
            object.materials[slot].paint = paint;
        }
    }
    Ok((object, summary))
}

fn split_tiles<'a>(
    geometry: &mut Geometry,
    materials: &mut Vec<MaterialSlot>,
    sets: &mut Vec<Option<&'a TextureSetInfo>>,
    summary: &mut Summary,
) {
    let mut assignments = vec![BTreeMap::<Option<Tile>, Vec<usize>>::new(); materials.len()];
    for (index, face) in geometry.faces.iter().enumerate() {
        let Some(set) = sets[face.material].filter(|set| set.use_uv_tiles) else {
            continue;
        };
        let center =
            face.corners.iter().map(|corner| corner.uv).sum::<Vec2>() / face.corners.len() as f32;
        let fits = |tile: &Tile| face.corners.iter().all(|corner| tile.contains(corner.uv));
        let declared = |tile: &Tile| {
            set.uv_tiles.is_empty()
                || set
                    .uv_tiles
                    .iter()
                    .any(|info| info.indices == tile.indices())
        };
        let tile = Tile::at_uv(center)
            .filter(fits)
            .filter(declared)
            .or_else(|| {
                // The centroid can lie on a tile boundary for a UV-degenerate face.
                set.uv_tiles
                    .iter()
                    .filter_map(|info| {
                        let [u, v] = info.indices;
                        Tile::at_uv(Vec2::new(u as f32 + 0.5, 0.5 - v as f32))
                    })
                    .find(fits)
            });
        assignments[face.material]
            .entry(tile)
            .or_default()
            .push(index);
    }
    for (slot, groups) in assignments.into_iter().enumerate() {
        let source = materials[slot].clone();
        for (number, (tile, faces)) in groups.into_iter().enumerate() {
            let mut material = source.clone();
            if let Some(tile) = tile {
                material.source["extras"]["bevy_editor_udim"] = json!(tile.number());
                summary.tiles += 1;
            } else {
                summary.notes.push(format!(
                    "«{}» · граней вне плиток UDIM: {}",
                    source.source["name"].as_str().unwrap_or_default(),
                    faces.len(),
                ));
            }
            let new_slot = if number == 0 {
                materials[slot] = material;
                slot
            } else {
                let index = materials.len();
                materials.push(material);
                sets.push(sets[slot]);
                index
            };
            for face in faces {
                geometry.faces[face].material = new_slot;
            }
        }
    }
}

fn needs_replay(layers: &[subcraft_spp::document::LayerInfo]) -> bool {
    layers.iter().any(|layer| {
        let mask = layer
            .mask_enabled
            .then_some(layer.mask.as_slice())
            .into_iter()
            .flatten();
        layer.mesh_filter.is_some()
            || layer.content.iter().chain(mask).any(|action| {
                action.enabled
                    && matches!(
                        action.kind,
                        ActionKind::Decal { .. }
                            | ActionKind::Paint { .. }
                            | ActionKind::Eraser { .. }
                    )
            })
            || needs_replay(&layer.children)
    })
}

fn decode_surface(
    file: &mut FileReader,
    set: &TextureSetInfo,
    (remaining, textures): (&mut usize, &mut TextureCache),
    summary: &mut Summary,
    (object, slot, replay): (&Object, usize, Option<&PolygonReplay>),
    tile: Option<Tile>,
    generators: &[Generator],
) -> Option<Surface> {
    let scope = match tile {
        Some(tile) => format!("«{}» · UDIM {}", set.name, tile.number()),
        None => format!("«{}»", set.name),
    };
    let tile_info = tile.and_then(|tile| {
        set.uv_tiles
            .iter()
            .find(|info| info.indices == tile.indices())
    });
    let [width, height] = tile_info
        .and_then(|info| info.resolution_log2)
        .unwrap_or([set.width_log2, set.height_log2]);
    let mut size = 1u32 << width.max(height).clamp(7, 12);
    let planes = rebuild::estimated_planes(&set.layers);
    let surface_bytes = |size| plane_bytes(size) * (textures.base_plane_count(size) + CHANNELS.len());
    let working = |size: u32| {
        surface_bytes(size)
            .saturating_add(plane_bytes(size).saturating_mul(planes))
            .saturating_add(if generators.is_empty() {
                0
            } else {
                generator_input_bytes(size.min(MAX_IMPORT_BAKE_SIZE))
            })
    };
    while size > MIN_TEXTURE_SIZE && working(size) > *remaining {
        size /= 2;
    }
    if surface_bytes(size) > *remaining {
        summary.unrestored += rebuild::count(&set.layers);
        summary
            .notes
            .push(format!("{scope} · исчерпан объём текстур при импорте"));
        return None;
    }
    if width.max(height) > size.ilog2() as i32 {
        summary
            .notes
            .push(format!("{scope} · разрешение уменьшено до {size} × {size}"));
    }
    let mut layer = Layer::new("Результат Substance Painter".into(), LayerKind::Paint);
    for (channel, (bits, label, format)) in CHANNELS.into_iter().enumerate() {
        let cache = match tile {
            Some(tile) => file.tile_channel(set, tile.indices(), bits, size),
            None => file.channel(set, bits, size),
        };
        match cache {
            Ok(Some(cache)) if cache.format == format => {
                layer.channels[channel] = Some(Arc::new(channel_pixels(&cache, size, channel)));
                summary.channels += 1;
            }
            Ok(Some(_)) | Err(_) => summary
                .notes
                .push(format!("{scope} · карта «{label}» не прочитана")),
            Ok(None) if set.channels.iter().any(|channel| channel.bits == bits) => {
                summary
                    .notes
                    .push(format!("{scope} · карта «{label}» отсутствует в кэше",));
            }
            Ok(None) => {}
        }
    }
    let has_cache = layer.channels.iter().any(Option::is_some);
    if !has_cache && set.layers.is_empty() {
        return None;
    }
    let cached_channels = layer.channels.iter().flatten().count();
    *remaining -= plane_bytes(size) * (textures.base_plane_count(size) + cached_channels);
    let baked = if generators.is_empty() {
        None
    } else {
        let mut bake_size = size.min(MAX_IMPORT_BAKE_SIZE);
        while bake_size > MIN_TEXTURE_SIZE && generator_input_bytes(bake_size) > *remaining {
            bake_size /= 2;
        }
        let bytes = generator_input_bytes(bake_size);
        let result = if bytes <= *remaining {
            import_generator_inputs(object, slot, bake_size, generators)
        } else {
            Err("исчерпан объём текстур при импорте".into())
        };
        match result {
            Ok(inputs) => {
                *remaining -= bytes;
                summary.baked_generator_sets += 1;
                Some(inputs)
            }
            Err(error) => {
                summary.notes.push(format!("{scope} · карты генераторов: {error}"));
                None
            }
        }
    };
    let mut layers = Vec::new();
    if has_cache {
        layers.push(layer);
    }
    rebuild::add(
        file,
        &set.layers,
        size,
        has_cache,
        (remaining, &mut textures.height_patterns),
        summary,
        &mut layers,
        (&object.geometry, slot, replay),
        tile,
        &scope,
        baked.as_ref(),
    );
    let mut surface = Surface {
        size,
        base: textures.base(size),
        stack: Stack {
            selected: if has_cache { 0 } else { layers.len() - 1 },
            layers,
        },
        material: object.materials[slot].authored.as_deref().unwrap().clone(),
        baked,
    };
    if !has_cache {
        surface.bind_generators(
            &object.geometry,
            slot,
            object.uv_offset(slot),
            object.transform,
        );
    }
    Some(surface)
}

fn channel_pixels(cache: &CacheImage, size: u32, channel: usize) -> Vec<u8> {
    let source = &cache.plane;
    let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    let mut pixels = Vec::with_capacity(size as usize * size as usize * 4);
    for y in 0..size {
        for x in 0..size {
            // SPP stores the cache in OpenGL row order. Bevy's UV rows start at the top.
            let data = source.pixel(
                x * source.width / size,
                source.height - 1 - y * source.height / size,
            );
            let rgb = match channel {
                0 | 5 | 6 => [byte(data[0]), byte(data[1]), byte(data[2])],
                3 => [byte(data[0] * 0.5 + 0.5); 3],
                _ => [byte(data[0]); 3],
            };
            pixels.extend([rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    pixels
}

#[cfg(test)]
#[path = "model_paint_spp_fixture.rs"]
pub(in crate::editor) mod fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn spp_import_creates_editable_glb_and_restores_cache_row_order() {
        let directory = std::env::temp_dir().join(format!(
            "bevy-import-spp-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let assets = directory.join("assets");
        fs::create_dir_all(&assets).unwrap();
        let source = directory.join("Куб.spp");
        let pixels: Vec<u8> = (0..8).flat_map(|i| [i * 10, 255]).collect();
        let cache = format!("texture-cache/4242/1/{}", channel_bits::ROUGHNESS);
        let project = fixture::hdf5_file(&[
            ("paint/scene.assbin", fixture::assbin_quad()),
            ("paint/document.bin", fixture::document(0)),
            (&cache, fixture::cache_blob(4, 2, &pixels)),
        ]);
        fs::write(&source, project).unwrap();

        let report = crate::editor::browser_files::import_files(
            &assets,
            Path::new(""),
            vec![source.clone(), source.clone()],
            Default::default(),
        );
        assert!(report.errors.is_empty(), "{}", report.status());
        assert_eq!(report.skipped, 1);
        assert_eq!(report.imported, [Path::new("Куб_SPP.glb")]);
        assert!(report.status().contains("наборов: 1 · карт: 1"));
        assert!(
            report.status().contains("групп: 1 · заливок: 1 · масок: 1"),
            "{}",
            report.status()
        );
        let file = assets.join(&report.imported[0]);
        let glb = Glb::load(&file).unwrap();
        let primitive = &glb.json["meshes"][0]["primitives"][0];
        let surface = glb.paint_surface(primitive).unwrap().unwrap();
        assert_eq!(surface.size, 256);
        assert_eq!(surface.stack.layers[0].name, "Результат Substance Painter");
        assert!(surface.stack.valid_hierarchy());
        assert_eq!(
            surface.stack.layers.last().unwrap().name,
            "Слои Substance Painter"
        );
        assert!(!surface.stack.layers.last().unwrap().visible);
        let fill = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap();
        assert_eq!(fill.kind, LayerKind::Fill);
        assert_eq!(fill.depth, 1);
        assert_eq!(
            fill.channels[0].as_ref().unwrap()[..4],
            [137, 188, 225, 255]
        );
        let roughness = surface.composite(1);
        assert_eq!(roughness[0], 40);
        assert_eq!(
            roughness[((surface.size - 1) * surface.size * 4) as usize],
            0
        );
        assert_eq!(roughness[((surface.size - 1) * 4) as usize], 70);
        let model = bevy::gltf::gltf::Gltf::open(&file).unwrap();
        let primitive = model.meshes().next().unwrap().primitives().next().unwrap();
        let reader = primitive.reader(|_| model.blob.as_deref());
        assert_eq!(reader.read_indices().unwrap().into_u32().count(), 6);
        let uv: Vec<_> = reader.read_tex_coords(0).unwrap().into_f32().collect();
        assert!(uv.contains(&[0., 1.]) && uv.contains(&[1., 0.]));
        assert!(source.is_file());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn spp_import_restores_bitmap_and_uniform_fills_even_without_a_texture_cache() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-bitmap-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Painter.spp");
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                (
                    "paint/document.bin",
                    fixture::document_with_bitmap(0, Some(&format!("Grain?version={key}"))),
                ),
                (
                    &format!("resources/{key}"),
                    fixture::bitmap_blob([204, 45, 19, 255]),
                ),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(
            status.contains("карт: 0 · групп: 1 · заливок: 1 · масок: 1"),
            "{status}"
        );
        let glb = Glb::load(&root.join(filename)).unwrap();
        let mut surface = (*glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap())
        .clone();
        assert!(surface.stack.valid_hierarchy());
        assert!(surface.stack.layers.last().unwrap().visible);
        assert!(
            !surface
                .stack
                .layers
                .iter()
                .any(|layer| layer.name == "Результат Substance Painter")
        );
        let fill = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap();
        let image = fill.texture_fill.as_ref().unwrap().images[1]
            .as_ref()
            .unwrap();
        assert_eq!(image.pixels().get_pixel(0, 0).0, [204, 45, 19, 255]);
        let (mesh, _) = import(&source, "Painter").unwrap();
        surface.bind_texture_fills(&mesh.geometry, 0, Vec2::ZERO, Transform::IDENTITY);
        assert_eq!(&surface.composite(0)[..3], &[137, 188, 225]);
        assert_eq!(&surface.composite(1)[..3], &[204; 3]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_replays_uv_click_and_3d_rectangle_into_editable_polygon_mask() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-polygon-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Polygons.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", fixture::document_with_polygon_mask()),
            ]),
        )
        .unwrap();

        let (filename, report) = convert(&source, &root).unwrap();

        let status = report.label;
        assert!(status.contains("заливок по геометрии: 1"), "{status}");
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let mask = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap()
            .mask
            .as_ref()
            .unwrap();
        let index = |x, y| ((y * surface.size + x) * 4) as usize;
        assert_eq!(mask.pixels[index(192, 192)], 64);
        assert_eq!(mask.pixels[index(64, 64)], 191);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_applies_polygon_fill_over_a_generator_layer() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-procedural-polygon-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Procedural.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                (
                    "paint/document.bin",
                    fixture::document_with_generator_and_polygon_mask(
                        "starter/fractal_noise/fractal_noise",
                    ),
                ),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        assert!(report.overview.contains("заливок по геометрии: 1"));
        assert!(
            report
                .overview
                .contains("генераторов сведено в основу маски: 1"),
            "{}",
            report.overview
        );
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let mask = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap()
            .mask
            .as_ref()
            .unwrap();
        assert!(mask.layers.is_empty());
        let value = |x, y| mask.pixels[((y * surface.size + x) * 4) as usize];
        assert_eq!(value(192, 192), 64);
        let remaining: std::collections::HashSet<_> = (32..96)
            .map(|x| value(x, 64))
            .collect();
        assert!(remaining.len() > 2 && remaining.iter().all(|&v| v <= 128));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_bakes_dirt_before_replaying_a_polygon_fill() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-dirt-polygon-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Dirt.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                (
                    "paint/document.bin",
                    fixture::document_with_generator_and_polygon_mask("starter/mg_dirt/mg_dirt"),
                ),
            ]),
        )
        .unwrap();
        let (object, summary) = import(&source, "Dirt").unwrap();
        assert_eq!(summary.baked_generator_sets, 1);
        assert_eq!(summary.baked_mask_generators, 1);
        assert_eq!(summary.polygon_fills, 1);
        let surface = object.materials[0].paint.as_ref().unwrap();
        let mask = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap()
            .mask
            .as_ref()
            .unwrap();
        assert!(mask.layers.is_empty());
        assert_eq!(mask.pixels[((192 * surface.size + 192) * 4) as usize], 64);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_replays_brush_and_eraser_in_both_views_and_mask() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-strokes-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Strokes.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", fixture::document_with_strokes()),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(status.contains("действий кисти и ластика: 5"), "{status}");
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let find = |name| {
            surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == name)
                .unwrap()
        };
        let index = |x, y| ((y * surface.size + x) * 4) as usize;
        let color = find("Scratches").channels[0].as_ref().unwrap();
        let red = &color[index(192, 192)..index(192, 192) + 4];
        let green = &color[index(64, 64)..index(64, 64) + 4];
        assert_eq!(&red[..3], &[255, 0, 0]);
        assert!((125..=132).contains(&red[3]), "{red:?}");
        assert_eq!(&green[..3], &[0, 255, 0]);
        assert!(green[3] > 250);
        assert_eq!(color[index(128, 128) + 3], 0);
        let mask = &find("Rust").mask.as_ref().unwrap().pixels;
        assert!((125..=132).contains(&mask[index(192, 192)]));
        assert_eq!(mask[index(128, 128)], 255);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_limits_uniform_paint_fills_to_uv_islands_and_padding() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-paint-fill-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("PaintFill.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad_with_margin()),
                ("paint/document.bin", fixture::document_with_paint_fill()),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        assert!(
            report.label.contains("заливок по геометрии: 1"),
            "{}",
            report.label
        );
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let layer = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Scratches")
            .unwrap();
        assert_eq!(layer.kind, LayerKind::Paint);
        let color = layer.channels[0].as_ref().unwrap();
        let pixel = |x, y| {
            let index = ((y * surface.size + x) * 4) as usize;
            &color[index..index + 4]
        };
        let edge = surface.size / 4;
        let y = surface.size / 2;
        let inside = pixel(edge + 6, y);
        assert_eq!(inside, [137, 137, 137, 255]);
        assert_eq!(pixel(edge - 1, y), inside);
        assert_eq!(pixel(edge - 3, y), [0, 0, 0, 0]);
        assert!(color.chunks_exact(4).any(|pixel| {
            pixel[3] > 0 && pixel[1] > pixel[0].saturating_add(80)
        }));
        assert!(layer.channels[1].is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_reads_version_zero_layers_and_rejects_truncated_documents() {
        let root = std::env::temp_dir()
            .join(format!("bevy-spp-v0-{}", bevy::asset::uuid::Uuid::new_v4(),));
        let assets = root.join("assets");
        fs::create_dir_all(&assets).unwrap();
        let old = root.join("Old.spp");
        let broken = root.join("Broken.spp");
        let document = fixture::document_v0();
        let cache = format!("texture-cache/4242/1/{}", channel_bits::ROUGHNESS);
        for (input, payload) in [
            (&old, document.as_slice()),
            (&broken, &document[..document.len() - 1]),
        ] {
            fs::write(
                input,
                fixture::hdf5_file(&[
                    ("paint/scene.assbin", fixture::assbin_quad()),
                    ("paint/document.bin", payload.to_vec()),
                    (&cache, fixture::cache_blob(2, 2, &[100, 255].repeat(4))),
                ]),
            )
            .unwrap();
        }
        let report =
            crate::editor::browser_files::import_files(&assets, Path::new(""), vec![old, broken], Default::default());
        assert_eq!(
            report.imported,
            [Path::new("Old_SPP.glb")],
            "{}",
            report.status()
        );
        assert_eq!(report.errors.len(), 1, "{}", report.status());
        assert!(!assets.join("Broken_SPP.glb").exists());
        let details = report.paint_project_details().unwrap();
        assert!(details.contains("Old_SPP.glb"), "{details}");
        assert!(
            details.contains("карта «цвет» отсутствует в кэше"),
            "{details}"
        );
        assert!(details.contains("Broken.spp"), "{details}");
        let glb = Glb::load(&assets.join(&report.imported[0])).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        assert_eq!(surface.size, 256);
        assert_eq!(surface.composite(1)[0], 100);
        assert!(surface.stack.valid_hierarchy());
        let group = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Old group")
            .unwrap();
        let fill = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap();
        assert_eq!((group.kind, group.depth), (LayerKind::Folder, 1));
        assert_eq!((fill.kind, fill.depth), (LayerKind::Fill, 2));
        assert_eq!(
            fill.channels[0].as_ref().unwrap()[..4],
            [137, 188, 225, 255]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_projects_overlapping_charts_onto_the_last_rasterized_surface() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-overlap-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        for (name, front_last) in [("FrontLast", true), ("BackLast", false)] {
            let source = root.join(format!("{name}.spp"));
            fs::write(
                &source,
                fixture::hdf5_file(&[
                    (
                        "paint/scene.assbin",
                        fixture::assbin_overlapping_charts(front_last),
                    ),
                    ("paint/document.bin", fixture::document_with_strokes()),
                ]),
            )
            .unwrap();
            let (filename, _) = convert(&source, &root).unwrap();
            let glb = Glb::load(&root.join(filename)).unwrap();
            let surface = glb
                .paint_surface(&glb.json["meshes"][0]["primitives"][0])
                .unwrap()
                .unwrap();
            let color = surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == "Scratches")
                .unwrap()
                .channels[0]
                .as_ref()
                .unwrap();
            let index = |x, y| ((y * surface.size + x) * 4) as usize;
            let red = &color[index(192, 192)..][..4];
            let green = &color[index(64, 64)..][..4];
            assert_eq!(&red[..3], &[255, 0, 0]);
            assert!((125..=132).contains(&red[3]), "{name}: {red:?}");
            if front_last {
                assert_eq!(&green[..3], &[0, 255, 0]);
                assert!(green[3] > 250, "{name}: {green:?}");
            } else {
                assert_eq!(green[3], 0, "{name}: {green:?}");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_preserves_mask_effects_and_restricts_named_mesh_parts() {
        use crate::editor::model_paint_data::mask::{FilterKind, GeneratorKind};
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-mask-effects-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Masks.spp");
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_two_parts()),
                (
                    "paint/document.bin",
                    fixture::document_with_mask_effects(&format!("Gray?version={key}")),
                ),
                (
                    &format!("resources/{key}"),
                    fixture::bitmap_blob([128, 128, 128, 255]),
                ),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(
            status.contains("геометрических масок: 1 · эффектов маски: 6"),
            "{status}"
        );
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let find = |name| {
            surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == name)
                .unwrap()
        };
        let mask = find("Rust").mask.as_ref().unwrap();
        assert_eq!(mask.filters[0].kind, FilterKind::Levels);
        assert_eq!(mask.filters[0].values, [0., 100., 200.]);
        assert_eq!(mask.filters[1].kind, FilterKind::Blur);
        assert_eq!(mask.filters[1].values[0], 3.);
        assert!(mask.filters[1].wrap);
        assert_eq!(mask.filters[2].kind, FilterKind::Sharpen);
        assert_eq!(mask.filters[2].values[0], 0.25);
        assert_eq!(mask.filters.len(), 4); // Custom-noise Warp remains outside the supported subset.
        assert_eq!(mask.filters[3].kind, FilterKind::Warp);
        assert!((mask.filters[3].values[0] - 3.9).abs() < 0.001);
        assert_eq!(&mask.filters[3].values[1..], &[4., 7.]);
        let left = (surface.size * 64 + 64) as usize;
        let right = (surface.size * 64 + 192) as usize;
        assert_eq!(mask.pixels[left * 4], 128);
        assert_eq!(mask.effective()[left * 4], 181);
        assert_eq!(mask.effective()[right * 4], 0);
        let resized = mask.with_pixels(solid(128, [128; 4]));
        let coverage = resized.geometry_coverage().unwrap();
        assert_eq!(coverage.len(), 128 * 128);
        assert_eq!((coverage[32], coverage[96]), (255, 0));
        let imported = find("Scratches").mask.as_ref().unwrap();
        assert!(imported.affine);
        let generator = imported.generator.unwrap();
        assert_eq!(generator.kind, GeneratorKind::Dirt);
        assert!(status.contains("наборов с запечёнными картами: 1"), "{status}");
        let baked = surface.baked.as_ref().unwrap();
        assert_eq!(baked.size, surface.size);
        assert_eq!(baked.texels.len(), (baked.size * baked.size) as usize);
        assert!(!generator.missing_bake_inputs(Some(baked)));
        assert!(Generator::new(GeneratorKind::ThinAreas).missing_bake_inputs(Some(baked)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_keeps_mask_outline_parameters_editable_in_glb() {
        use crate::editor::model_paint_data::mask::FilterKind;
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-mask-outline-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Outline.spp");
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_two_parts()),
                (
                    "paint/document.bin",
                    fixture::document_with_mask_outline(&format!("Gray?version={key}")),
                ),
                (
                    &format!("resources/{key}"),
                    fixture::bitmap_blob([128, 128, 128, 255]),
                ),
            ]),
        )
        .unwrap();
        let (filename, _) = convert(&source, &root).unwrap();
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let outline = surface
            .stack
            .layers
            .iter()
            .find_map(|layer| {
                layer
                    .mask
                    .as_ref()?
                    .filters
                    .iter()
                    .find(|f| f.kind == FilterKind::Outline)
            })
            .unwrap();
        assert_eq!(
            std::array::from_fn::<_, 6, _>(|slot| outline.value(slot)),
            [75., 2.5, 1., 1., -25., 1.]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_mirror_mask_retains_controls_and_reflects_the_input() {
        use crate::editor::model_paint_data::mask::FilterKind;
        let root = std::env::temp_dir().join(format!("bevy-spp-mirror-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("Mirror.spp");
        let key = "1234567890123456789012345678901234567890.image";
        let mut row = [0, 0, 0, 255].repeat(8);
        row.extend([255; 32]);
        fs::write(&source, fixture::hdf5_file(&[
            ("paint/scene.assbin", fixture::assbin_quad()),
            ("paint/document.bin", fixture::document_with_mirror_mask(&format!("Mask?version={key}"))),
            (&format!("resources/{key}"), fixture::bitmap_row(&row)),
        ])).unwrap();
        let (name, report) = convert(&source, &root).unwrap();
        assert!(report.overview.contains("эффектов маски: 2"), "{}", report.overview);
        let glb = Glb::load(&root.join(name)).unwrap();
        let surface = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
        let mask = surface.stack.layers.iter().find(|layer| layer.name == "Rust")
            .unwrap().mask.as_deref().unwrap();
        let filter = mask.filters[0];
        assert_eq!(filter.kind, FilterKind::Mirror);
        assert_eq!(std::array::from_fn::<_, 6, _>(|slot| filter.value(slot)), [0., 37.5, 20., 1., 1., 3.]);
        let size = surface.size as usize;
        let pixel = |x| (size * (size / 2) + x) * 4;
        assert_eq!(mask.pixels[pixel(size / 8)], 0);
        assert_eq!(mask.effective()[pixel(size / 8)], 255);
        assert_eq!(mask.effective()[pixel(size * 3 / 8)], 0);
        assert_eq!(mask.effective()[pixel(size * 7 / 8)], 255);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_projects_triplanar_bitmap_masks_through_the_mesh() {
        let root = std::env::temp_dir().join(format!("bevy-spp-mask-projection-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let key = "1234567890123456789012345678901234567890.image";
        let mut colors = [0, 0, 0, 255].repeat(4);
        colors.extend([255; 16]);
        let mut right_side = Vec::new();
        for (projection, name) in [(0, "UV"), (2, "Triplanar")] {
            let source = root.join(format!("{name}.spp"));
            fs::write(&source, fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_compressed_uv_quad()),
                ("paint/document.bin", fixture::document_with_bitmap_mask_projection(&format!("Mask?version={key}"), projection)),
                (&format!("resources/{key}"), fixture::bitmap_row(&colors)),
            ])).unwrap();
            let (file, report) = convert(&source, &root).unwrap();
            assert_eq!(report.overview.contains("трипланарных растровых масок: 1"), projection == 2);
            let glb = Glb::load(&root.join(file)).unwrap();
            let surface = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
            let layer = surface.stack.layers.iter().find(|layer| layer.name == "Rust").unwrap();
            let mask = layer.mask.as_ref().unwrap();
            let size = surface.size as usize;
            let sample = |x| mask.pixels[(size * (size / 2) + x) * 4];
            right_side.push(sample(size * 3 / 8));
            if projection == 2 {
                assert!(report.warnings.iter().any(|warning| warning.contains("приближённо восстановлено по сетке")));
                assert_eq!(sample(size / 8), 0);
                assert_eq!(sample(size / 2 + 1), sample(size / 2 - 1));
                assert_eq!(sample(size / 2 + 2), 255);
            }
        }
        assert_eq!(right_side, [0, 255]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_keeps_procedural_mask_settings_editable_in_glb() {
        use crate::editor::model_paint_data::mask::GeneratorInput;
        use fixture::GeneratorTweak::{Bool, Float, Int};

        let root = std::env::temp_dir().join(format!(
            "bevy-spp-generator-settings-{}", bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        for (name, tweaks, values, weights, inverted) in [
            (
                "mg_dirt",
                vec![Float("dirt_level", 0.75), Float("dirt_contrast", 0.8),
                    Float("Grunge_Scale", 6.), Float("grunge_amount", 0.45),
                    Int("$randomseed", 431), Float("edges_masking", 0.4), Bool("invert", true)],
                [25., 10., 6., 45., 27.],
                vec![(GeneratorInput::Occlusion, 100), (GeneratorInput::Convexity, -40)], true,
            ),
            (
                "mg_mask_editor",
                vec![Float("global_balance", 0.625), Float("global_contrast", 0.5),
                    Float("AO", 0.7), Bool("invert_ao", false), Float("curv", 0.25),
                    Float("thick", 0.2), Bool("thick_invert", true), Float("Position_Gradient", 0.1),
                    Float("tex", 0.5), Float("scale_tex", 2.), Float("tex2", 0.25), Float("tex2_scale", 9.)],
                [-32.5, 25., 2., 50., 0.],
                vec![(GeneratorInput::Occlusion, -70), (GeneratorInput::Convexity, 25),
                    (GeneratorInput::Cavity, 0), (GeneratorInput::Concavity, 0),
                    (GeneratorInput::Thin, 20), (GeneratorInput::Height, 10)], false,
            ),
            (
                "noise_clouds_2",
                vec![Float("balance", 0.75), Float("contrast", 1.), Float("scale", 80.), Int("$randomseed", -1)],
                [25., 0.1, 64., 0., 100.], vec![(GeneratorInput::Noise, 100)], false,
            ),
        ] {
            let source = root.join(format!("{name}.spp"));
            fs::write(&source, fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad_with_margin()),
                ("paint/document.bin", fixture::document_with_generator_tweaks(&format!("starter/{name}/{name}"), &tweaks)),
            ])).unwrap();
            let (filename, report) = convert(&source, &root).unwrap();
            assert!(report.overview.contains("генераторов с параметрами: 1"), "{}", report.overview);
            let model = Glb::load(&root.join(filename)).unwrap();
            let surface = model.paint_surface(&model.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
            let layer = surface.stack.layers.iter().find(|layer| layer.name == "Rust").unwrap();
            let generator = layer.mask.as_ref().unwrap().generator.unwrap();
            assert!(generator.valid());
            for (actual, expected) in generator.values.into_iter().zip(values) {
                assert!((actual - expected).abs() < 1e-3, "{name}: {actual} != {expected}");
            }
            for (input, expected) in weights {
                assert_eq!(generator.weight(input), expected, "{name}, {input:?}");
            }
            assert_eq!(generator.invert, inverted);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_reopens_editable_gradients_with_uv_and_planar_coverage() {
        use crate::editor::model_paint_data::mask::{GeneratorKind, GeneratorSurface};
        use fixture::GeneratorTweak::{Bool, Float, Int};

        let root = std::env::temp_dir().join(format!("bevy-spp-gradient-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for (name, kind, projection, tweaks) in [
            ("gradient_linear_1", GeneratorKind::GradientLinear, 1, vec![Float("Position", 0.5)]),
            ("gradient_linear_2", GeneratorKind::GradientMirror, 0, vec![Int("Tiling", 2), Int("rotation", 1)]),
            ("gradient_linear_3", GeneratorKind::GradientPeak, 0, vec![Float("Position", 0.49), Float("position", 0.25), Bool("invert", true)]),
        ] {
            let source = root.join(format!("{name}.spp"));
            let document = if kind == GeneratorKind::GradientLinear {
                fixture::document_with_projected_generator(&format!("starter/{name}/{name}"), &tweaks, projection)
            } else {
                fixture::document_with_generator_tweaks(&format!("starter/{name}/{name}"), &tweaks)
            };
            let geometry = fixture::assbin_quad();
            fs::write(&source, fixture::hdf5_file(&[
                ("paint/scene.assbin", geometry),
                ("paint/document.bin", document),
            ])).unwrap();
            let (filename, report) = convert(&source, &root).unwrap();
            assert!(report.overview.contains("градиентных масок: 1"), "{}", report.overview);
            let model = Glb::load(&root.join(filename)).unwrap();
            let primitive = &model.json["meshes"][0]["primitives"][0];
            let surface = model.paint_surface(primitive).unwrap().unwrap();
            let layer = surface.stack.layers.iter().find(|layer| layer.name == "Rust").unwrap();
            let mut mask = (**layer.mask.as_ref().unwrap()).clone();
            let generator = mask.generator.or_else(|| mask.layers.last()?.generator).unwrap();
            assert_eq!(generator.kind, kind);
            let (object, _) = import(&source, name).unwrap();
            mask.bind(Arc::new(GeneratorSurface::new(
                object.geometry, 0, surface.size, Vec2::ZERO, Transform::IDENTITY, None,
            )));
            let size = surface.size as usize;
            let value = |x, y| mask.effective()[(y * size + x) * 4];
            match kind {
                GeneratorKind::GradientLinear => {
                    assert_eq!(mask.layers.len(), 2);
                    let mapping = generator.gradient_mapping.unwrap();
                    assert_eq!(mapping.repeat, [false, true]);
                    assert_eq!(mapping.volume.unwrap().size, [0.5, 1., 0.4]);
                    assert_eq!(value(size / 8, size / 4), 64);
                    assert!(value(size / 2, size / 4).abs_diff(191) <= 2);
                    assert!(value(size / 2, 3 * size / 4).abs_diff(64) <= 2);
                }
                GeneratorKind::GradientMirror => {
                    assert_eq!(&generator.values[2..4], &[2., 90.]);
                    assert!(value(size / 4, size / 3) > 250);
                    assert!(value(3 * size / 4, size / 3) > 250);
                    assert!(value(size / 2, size / 3) < 8);
                }
                GeneratorKind::GradientPeak => {
                    assert_eq!(generator.values[4], 25.);
                    assert_eq!(generator.values[0], 49.);
                    assert!(value(size / 2, 3 * size / 4) < 8);
                    assert!(value(size / 2, 0) > 250);
                }
                _ => unreachable!(),
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_reopens_editable_shapes_with_uv_and_planar_coverage() {
        use crate::editor::model_paint_data::mask::{GeneratorKind, GeneratorSurface};
        use fixture::GeneratorTweak::{Bool, Float};

        let root = std::env::temp_dir().join(format!("bevy-spp-shapes-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for (name, kind, projection, tweaks, values) in [
            ("shape", GeneratorKind::ShapeDisc, None,
                vec![Float("scale", 0.5), Float("hardness", 1.)], [50., 100., 0.]),
            ("shape_cone", GeneratorKind::ShapeCone, None,
                vec![Float("hardness", 1.)], [100., 100., 0.]),
            ("shape_border_square", GeneratorKind::ShapeSquareBorder, Some(1),
                vec![Float("border_size", 0.25), Float("hardness", 1.)], [100., 100., 25.]),
            ("shape_border_circle", GeneratorKind::ShapeCircleBorder, None,
                vec![Float("width", 0.25), Bool("invert", true)], [100., 100., 25.]),
        ] {
            let source = root.join(format!("{name}.spp"));
            let url = format!("starter/{name}/{name}");
            let document = match projection {
                Some(projection) => fixture::document_with_projected_generator(&url, &tweaks, projection),
                None => fixture::document_with_generator_tweaks(&url, &tweaks),
            };
            fs::write(&source, fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", document),
            ])).unwrap();
            let (filename, report) = convert(&source, &root).unwrap();
            assert!(report.overview.contains("масок с фигурами: 1"), "{}", report.overview);
            let model = Glb::load(&root.join(filename)).unwrap();
            let surface = model.paint_surface(&model.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
            let mut mask = (**surface.stack.layers.iter().find(|layer| layer.name == "Rust")
                .unwrap().mask.as_ref().unwrap()).clone();
            let generator = mask.generator.or_else(|| mask.layers.last()?.generator).unwrap();
            assert_eq!(generator.kind, kind);
            assert_eq!(&generator.values[..3], &values);
            let (object, _) = import(&source, name).unwrap();
            mask.bind(Arc::new(GeneratorSurface::new(
                object.geometry, 0, surface.size, Vec2::ZERO, Transform::IDENTITY, None,
            )));
            let size = surface.size as usize;
            let value = |x, y| mask.effective()[(y * size + x) * 4];
            match kind {
                GeneratorKind::ShapeDisc => {
                    assert_eq!(value(size / 2, size / 2), 255);
                    assert_eq!(value(size / 8, size / 2), 0);
                }
                GeneratorKind::ShapeCone => {
                    assert!(value(size / 2, size / 2) > 250);
                    assert!(value(size / 4, size / 2).abs_diff(128) < 3);
                }
                GeneratorKind::ShapeSquareBorder => {
                    assert_eq!(generator.gradient_mapping.unwrap().volume.unwrap().size, [0.5, 1., 0.4]);
                    assert_eq!(value(size * 7 / 8, size / 2), 64);
                    assert_eq!(value(size / 2, size / 2), 0);
                    assert_eq!(value(size * 23 / 32, size / 2), 255);
                }
                GeneratorKind::ShapeCircleBorder => {
                    assert_eq!(value(size / 2, size / 2), 255);
                    assert_eq!(value(size * 15 / 16, size / 2), 0);
                    assert_eq!(value(size * 15 / 16, size * 15 / 16), 255);
                }
                _ => unreachable!(),
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_reopens_editable_tile_pattern_with_planar_coverage() {
        use crate::editor::model_paint_data::mask::{GeneratorKind, GeneratorSurface};
        use fixture::GeneratorTweak::{Float, Int};
        let root = std::env::temp_dir().join(format!("bevy-spp-tiles-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("Stripes.spp");
        let document = fixture::document_with_projected_generator(
            "starter/tile_generator/tile_generator",
            &[Int("Pattern_Type", 2), Int("Number_X", 4), Int("Number_Y", 1),
                Float("Scale", 0.5), Float("Background_Color", 0.125)],
            1,
        );
        fs::write(&source, fixture::hdf5_file(&[
            ("paint/scene.assbin", fixture::assbin_quad()),
            ("paint/document.bin", document),
        ])).unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        assert!(report.overview.contains("плиточных масок: 1"), "{}", report.overview);
        let model = Glb::load(&root.join(filename)).unwrap();
        let surface = model.paint_surface(&model.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
        let mut mask = (**surface.stack.layers.iter().find(|layer| layer.name == "Rust")
            .unwrap().mask.as_ref().unwrap()).clone();
        let generator = mask.layers.last().unwrap().generator.unwrap();
        assert_eq!(generator.kind, GeneratorKind::TilePattern);
        assert_eq!(generator.values, [4., 1., 100., 100., 50.]);
        assert_eq!(generator.value(5), 0.);
        assert_eq!(generator.value(19), 12.5);
        let (object, _) = import(&source, "Stripes").unwrap();
        mask.bind(Arc::new(GeneratorSurface::new(
            object.geometry, 0, surface.size, Vec2::ZERO, Transform::IDENTITY, None,
        )));
        let size = surface.size as usize;
        let value = |x| mask.effective()[(size / 2 * size + x) * 4];
        assert_eq!(value(size / 8), 64);
        assert_eq!(value(size * 5 / 16), 255);
        assert_eq!(value(size * 3 / 8), 32);
        assert_eq!(value(size * 7 / 16), 255);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_restores_ordered_height_adjustments_on_layers_and_masks() {
        use crate::editor::model_paint_data::mask::FilterKind;
        let root = std::env::temp_dir().join(format!("bevy-spp-height-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let key = "1234567890123456789012345678901234567890.image";
        let url = format!("Gray?version={key}");
        for (name, bitmap) in [("Solid", None), ("Bitmap", Some(url.as_str()))] {
            let source = root.join(format!("{name}.spp"));
            fs::write(&source, fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", fixture::document_with_height_adjust(bitmap)),
                (&format!("resources/{key}"), fixture::bitmap_blob([159, 159, 159, 255])),
            ])).unwrap();
            let (filename, report) = convert(&source, &root).unwrap();
            assert!(report.overview.contains("поправок высоты: 3"), "{}", report.overview);
            let glb = Glb::load(&root.join(filename)).unwrap();
            let mut surface = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
            let (object, _) = import(&source, name).unwrap();
            std::sync::Arc::make_mut(&mut surface)
                .bind_texture_fills(&object.geometry, 0, Vec2::ZERO, Transform::IDENTITY);
            let layer = surface.stack.layers.iter().find(|layer| layer.name == "Rust").unwrap();
            if bitmap.is_some() {
                let fill = layer.texture_fill.as_ref().unwrap();
                assert!(fill.images[3].is_some());
                assert_eq!(fill.settings.height_strength, -0.75);
                assert_eq!(fill.settings.height_offset, -0.0625);
            } else {
                assert!(layer.texture_fill.is_none());
            }
            let middle = (surface.size as usize * (surface.size as usize / 2) + surface.size as usize / 2) * 4;
            assert_eq!(&layer.channel(3).unwrap()[middle..middle + 4], &[96, 96, 96, 255]);
            let mask = layer.mask.as_ref().unwrap();
            assert_eq!(mask.filters[0].kind, FilterKind::HeightAdjust);
            assert_eq!(mask.filters[0].values, [2., 0.125, 1.]);
            assert_eq!(mask.effective()[middle], 63);
            assert_eq!(layer.channel(0).unwrap()[middle], 137);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_bakes_procedural_height_before_applying_adjustments() {
        let root = std::env::temp_dir().join(format!("bevy-spp-procedural-height-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for graph in ["gradient_checker", "noise_gaussian_noise", "noise_directional_scratches"] {
            let source = root.join(format!("{graph}.spp"));
            let url = format!("starter/{graph}/{graph}");
            fs::write(&source, fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", fixture::document_with_procedural_height(&url)),
            ])).unwrap();
            let (filename, report) = convert(&source, &root).unwrap();
            assert!(report.overview.contains("поправок высоты: 3"), "{}", report.overview);
            assert!(report.overview.contains("приближённых процедурных карт высоты: 1"), "{}", report.overview);
            let glb = Glb::load(&root.join(filename)).unwrap();
            let surface = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0]).unwrap().unwrap();
            let layer = surface.stack.layers.iter().find(|layer| layer.name == "Rust").unwrap();
            let fill = layer.texture_fill.as_ref().unwrap();
            assert_eq!(fill.settings.height_strength, -0.75);
            assert_eq!(fill.settings.height_offset, -0.0625);
            let image = fill.images[3].as_ref().unwrap().pixels();
            let levels: std::collections::HashSet<_> = image.pixels().map(|p| p[0]).collect();
            assert!(levels.len() > 16, "{graph} only generated {} shades", levels.len());
            if graph == "gradient_checker" {
                let gray = |x, y| image.get_pixel(x, y)[0];
                assert_eq!(gray(32, 16), gray(32, 80));
                assert!(gray(32, 16) < gray(96, 16));
                assert_eq!(gray(144, 16), gray(208, 16));
                assert!(gray(144, 16) < gray(144, 80));
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_preserves_tonal_mask_filters_in_glb() {
        use crate::editor::model_paint_data::mask::FilterKind;
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-tonal-mask-{}", bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Tonal.spp");
        fs::write(&source, fixture::hdf5_file(&[
            ("paint/scene.assbin", fixture::assbin_quad()),
            ("paint/document.bin", fixture::document_with_tonal_mask_filters()),
        ])).unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        assert!(report.overview.contains("слоёв частично: 1"));
        assert!(report.warnings.iter().any(|note| note.contains("Rust")));
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap().unwrap();
        let mask = surface.stack.layers.iter().find(|layer| layer.name == "Rust")
            .unwrap().mask.as_deref().unwrap();
        assert_eq!(mask.filters.iter().map(|f| (f.kind, f.values)).collect::<Vec<_>>(), [
            (FilterKind::ContrastLuminosity, [50., 37.5, 0.]),
            (FilterKind::HistogramScan, [62.5, 50., 1.]),
            (FilterKind::Threshold, [37.5, 1., 0.]),
            (FilterKind::DirectionalBlur, [4., 45., 0.]),
            (FilterKind::Clamp, [12.5, 56.25, 0.]),
            (FilterKind::Smoothstep, [25., 75., 0.]),
            (FilterKind::Posterize, [4., 0., 0.]),
            (FilterKind::Pixelate, [24., 0., 0.]),
        ]);
        assert_eq!(mask.effective()[0], 170);
        for (count, expected) in [(5, 143), (4, 255), (2, 128), (1, 96)] {
            let mut prefix = mask.clone();
            prefix.filters.truncate(count);
            assert!(prefix.effective()[0].abs_diff(expected) <= 2);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_preserves_filters_before_and_after_mask_fills() {
        use crate::editor::model_paint_data::mask::{FilterKind, MaskBlend};
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-ordered-mask-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Ordered.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                (
                    "paint/document.bin",
                    fixture::document_with_ordered_mask_filters(),
                ),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        assert!(
            report
                .overview
                .contains("фильтров сведено в основу маски: 1")
        );
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let mask = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Rust")
            .unwrap()
            .mask
            .as_ref()
            .unwrap();
        assert_eq!(mask.pixels[0], 191); // Invert the initial 25% fill.
        assert_eq!(mask.layers.len(), 1);
        assert_eq!(mask.layers[0].blend, MaskBlend::Multiply);
        assert_eq!(mask.layers[0].value, 50);
        assert_eq!(mask.filters.len(), 1);
        assert_eq!(mask.filters[0].kind, FilterKind::Levels);
        assert!((mask.filters[0].output[0] - 20.).abs() < 0.001);
        assert!((mask.filters[0].output[1] - 60.).abs() < 0.001);
        assert_eq!(mask.effective()[0], 114); // 0.2 + 0.4 * sqrt(0.75 * 0.5), at eight bits.
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_resolves_mask_and_material_anchors_at_the_recorded_action() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-anchors-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Anchors.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", fixture::document_with_anchors()),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(status.contains("ссылок на якоря: 2"), "{status}");
        assert!(status.contains("слоёв частично: 1"), "{status}");
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let find = |name| {
            surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == name)
                .unwrap()
        };
        let source = find("Rust");
        let dependent = find("Scratches");
        assert_eq!(source.mask.as_ref().unwrap().effective()[0], 191);
        assert_eq!(dependent.mask.as_ref().unwrap().effective()[0], 128);
        assert_eq!(&source.channels[0].as_ref().unwrap()[..4], &[0, 0, 0, 255]);
        assert_eq!(
            &dependent.channels[0].as_ref().unwrap()[..4],
            &[137, 188, 225, 255]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_keeps_mouse_taps_and_aligns_timed_pressure_samples() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-pressure-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Pressure.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                ("paint/document.bin", fixture::document_with_pressure()),
            ]),
        )
        .unwrap();
        let metadata = FileReader::open(&source).unwrap().project().unwrap();
        let mut tracks = Vec::new();
        subcraft_spp::document::walk_layers(
            &metadata.texture_sets[0].layers,
            &mut |layer, _| {
                for action in &layer.content {
                    if let ActionKind::Paint { strokes, .. } = &action.kind {
                        tracks.extend(strokes.iter().map(|stroke| stroke.pressure.clone()));
                    }
                }
            },
            0,
        );
        assert_eq!(tracks, [vec![], vec![0.], vec![0., 0.25, 1.]]);
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(status.contains("действий кисти и ластика: 1"), "{status}");
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let layer = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Scratches")
            .unwrap();
        let pixels = layer.channels[0].as_ref().unwrap();
        let alpha = |x| pixels[((64 * surface.size + x) * 4 + 3) as usize];
        assert!(
            alpha(64) > 250,
            "a tap without a pressure track uses the nominal brush"
        );
        assert_eq!(alpha(192), 0, "explicit zero pressure remains transparent");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_preserves_an_asymmetric_bitmap_brush_alpha() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-tip-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Tip.spp");
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                (
                    "paint/document.bin",
                    fixture::document_with_bitmap_tip(&format!("Tip?version={key}")),
                ),
                (
                    &format!("resources/{key}"),
                    fixture::bitmap_row(&[0, 0, 0, 0, 0, 0, 0, 255]),
                ),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(status.contains("действий кисти и ластика: 1"), "{status}");
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let layer = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Scratches")
            .unwrap();
        let pixels = layer.channels[0].as_ref().unwrap();
        let alpha = |x| pixels[((64 * surface.size + x) * 4 + 3) as usize];
        assert_eq!(alpha(55), 0);
        assert_eq!(alpha(73), 255);
        assert_eq!(alpha(80), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_places_uv_projected_3d_stamps_in_texture_space() {
        use fixture::TipStroke;
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-uv-projection-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        for (name, projection, expected) in [
            ("Surface", TipStroke::Plain, [0, 255]),
            ("UV", TipStroke::UvProjection, [255, 0]),
        ] {
            let alpha = imported_stamp_alpha(
                &root, name, fixture::assbin_quad_with_narrow_uv(), projection,
            );
            assert_eq!(alpha(16, 64), 255, "{name}");
            assert_eq!([alpha(21, 64), alpha(16, 73)], expected, "{name}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_camera_projection_distinguishes_backfaces_and_occlusion() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-camera-projection-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        for (name, culling, paint_through, normal, angle, front_alpha, rear_alpha) in [
            ("Visible", true, false, 1., 90, 255, 0),
            ("Culled", true, false, -1., 90, 0, 0),
            ("Unculled", false, false, -1., 90, 255, 0),
            ("Through", true, true, 1., 90, 255, 255),
            ("ThroughCulled", true, true, -1., 90, 0, 255),
            ("Tight", true, false, 0.5, 48, 0, 0),
            ("Wide", true, false, 0.5, 90, 255, 0),
            ("FullAngle", true, false, -1., 180, 255, 0),
        ] {
            let alpha = imported_stamp_alpha(
                &root, name, fixture::assbin_projection_layers(-0.5, normal),
                fixture::TipStroke::Camera { culling, paint_through, angle },
            );
            assert_eq!(alpha(64, 192), front_alpha, "{name}");
            assert_eq!(alpha(96, 32), rear_alpha, "{name}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_restores_size_and_flow_jitter_independently() {
        use fixture::TipStroke;
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-size-flow-jitter-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let alpha = |name, projection| imported_stamp_alpha(&root, name, fixture::assbin_quad(), projection);
        let plain = alpha("Plain", TipStroke::Plain);
        let size = alpha("Size", TipStroke::SizeJitter);
        let flow = alpha("Flow", TipStroke::FlowJitter);
        let repeat = alpha("Repeat", TipStroke::FlowJitter);
        assert_eq!([plain(64, 64), plain(74, 64)], [255, 255]);
        assert_eq!([size(64, 64), size(74, 64)], [255, 0]);
        assert!((150..180).contains(&flow(64, 64)));
        assert_eq!(flow(64, 64), flow(74, 64));
        assert_eq!(flow(64, 64), repeat(64, 64));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn spp_import_planar_stamp_fades_away_from_the_tangent_plane() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-tangent-projection-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let layers = || fixture::assbin_projection_layers(-0.03, 1.);
        let wrap = imported_stamp_alpha(&root, "Wrap", layers(), fixture::TipStroke::Plain);
        let planar = imported_stamp_alpha(&root, "Planar", layers(), fixture::TipStroke::PlanarProjection);
        assert_eq!(wrap(64, 160), 255);
        assert_eq!(planar(64, 160), 255);
        assert_eq!(wrap(64, 32), 255);
        assert!((70..160).contains(&planar(64, 32)));
        fs::remove_dir_all(&root).unwrap();
    }

    fn imported_stamp_alpha(
        root: &Path,
        name: &str,
        model: Vec<u8>,
        projection: fixture::TipStroke,
    ) -> impl Fn(u32, u32) -> u8 {
        let source = root.join(format!("{name}.spp"));
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", model),
                (
                    "paint/document.bin",
                    fixture::document_with_directed_tip(&format!("White?version={key}"), projection),
                ),
                (&format!("resources/{key}"), fixture::bitmap_blob([255; 4])),
            ]),
        )
        .unwrap();
        let (filename, _) = convert(&source, root).unwrap();
        let glb = Glb::load(&root.join(filename)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let pixels = surface.stack.layers.iter()
            .find(|layer| layer.name == "Scratches")
            .unwrap().channels[0].clone();
        let size = surface.size;
        move |x, y| pixels.as_ref().map_or(0, |pixels| pixels[((y * size + x) * 4 + 3) as usize])
    }

    #[test]
    fn spp_import_rotates_bitmap_brush_and_follows_recorded_path() {
        use fixture::TipStroke;
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-directed-tip-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let key = "1234567890123456789012345678901234567890.image";
        let mut plain = None;
        let mut jittered = None;
        for (name, stroke) in [
            ("Plain", TipStroke::Plain),
            ("Rotated", TipStroke::Rotated),
            ("Follow", TipStroke::Follow),
            ("Tap", TipStroke::FollowTap),
            ("JitterA", TipStroke::Jittered),
            ("JitterB", TipStroke::Jittered),
        ] {
            let source = root.join(format!("{name}.spp"));
            fs::write(
                &source,
                fixture::hdf5_file(&[
                    ("paint/scene.assbin", fixture::assbin_quad()),
                    (
                        "paint/document.bin",
                        fixture::document_with_directed_tip(&format!("Tip?version={key}"), stroke),
                    ),
                    (
                        &format!("resources/{key}"),
                        fixture::bitmap_row(&[0, 0, 0, 0, 0, 0, 0, 255]),
                    ),
                ]),
            )
            .unwrap();
            let (filename, _) = convert(&source, &root).unwrap();
            let glb = Glb::load(&root.join(filename)).unwrap();
            let surface = glb
                .paint_surface(&glb.json["meshes"][0]["primitives"][0])
                .unwrap()
                .unwrap();
            let pixels = surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == "Scratches")
                .and_then(|layer| layer.channels[0].as_ref());
            let alpha = |x: u32, y: u32| {
                pixels.map_or(0, |pixels| {
                    pixels[((y * surface.size + x) * 4 + 3) as usize]
                })
            };
            match stroke {
                TipStroke::Rotated => {
                    assert!(alpha(64, 55) > 240);
                    assert!(alpha(73, 64) < 150);
                    assert_eq!(alpha(64, 73), 0);
                }
                TipStroke::Follow => {
                    assert!(alpha(192, 123) > 240);
                    assert_eq!(alpha(192, 121), 0);
                }
                TipStroke::FollowTap => assert!(
                    pixels
                        .is_none_or(|pixels| { pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) })
                ),
                TipStroke::Plain => plain = pixels.cloned(),
                TipStroke::Jittered => {
                    let pixels = pixels.unwrap();
                    assert_ne!(Some(pixels), plain.as_ref());
                    if let Some(previous) = &jittered {
                        assert_eq!(pixels, previous);
                    } else {
                        jittered = Some(pixels.clone());
                    }
                }
                TipStroke::Splatter { .. } | TipStroke::UvProjection | TipStroke::Camera { .. }
                | TipStroke::PlanarProjection | TipStroke::SizeJitter | TipStroke::FlowJitter => unreachable!(),
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_projects_bitmap_stencil_in_uv_and_mirrors_its_coverage() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-uv-stencil-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let key = "1234567890123456789012345678901234567890.image";
        for (wrap, horizontal, vertical) in [
            (None, false, false),
            (Some(1), true, false),
            (Some(2), false, true),
            (Some(3), true, true),
        ] {
            let source = root.join(format!("UVStencil{}.spp", wrap.unwrap_or(0)));
            fs::write(
                &source,
                fixture::hdf5_file(&[
                    ("paint/scene.assbin", fixture::assbin_quad()),
                    (
                        "paint/document.bin",
                        fixture::document_with_uv_stencil_wrap(&format!("Stencil?version={key}"), wrap),
                    ),
                    (&format!("resources/{key}"), fixture::bitmap_blob([255; 4])),
                ]),
            )
            .unwrap();
            let (path, report) = convert(&source, &root).unwrap();
            let status = report.label;
            assert!(status.contains("действий кисти и ластика: 1"), "{status}");
            let glb = Glb::load(&root.join(path)).unwrap();
            let surface = glb
                .paint_surface(&glb.json["meshes"][0]["primitives"][0])
                .unwrap()
                .unwrap();
            let layer = surface.stack.layers.iter()
                .find(|layer| layer.name == "Scratches")
                .unwrap();
            let pixels = layer.channels[0].as_ref().unwrap();
            let alpha = |x, y| pixels[((y * surface.size + x) * 4 + 3) as usize];
            let direct = alpha(190, 190);
            assert!(direct > 140, "{direct}");
            assert_eq!(alpha(65, 190), direct);
            assert_eq!(alpha(193, 190) > 140, horizontal);
            assert_eq!(alpha(62, 190), alpha(193, 190));
            assert_eq!(alpha(190, 193) > 140, vertical);
            assert_eq!(alpha(65, 193), alpha(190, 193));
            assert_eq!(alpha(193, 193) > 140, horizontal && vertical);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_projects_stencil_and_pads_the_painted_uv_boundary() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-stencil-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Stencil.spp");
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad_with_margin()),
                (
                    "paint/document.bin",
                    fixture::document_with_stencil(&format!("Stencil?version={key}")),
                ),
                (&format!("resources/{key}"), fixture::bitmap_blob([255; 4])),
            ]),
        )
        .unwrap();
        let (path, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(status.contains("действий кисти и ластика: 1"), "{status}");
        let glb = Glb::load(&root.join(path)).unwrap();
        let surface = glb
            .paint_surface(&glb.json["meshes"][0]["primitives"][0])
            .unwrap()
            .unwrap();
        let layer = surface
            .stack
            .layers
            .iter()
            .find(|layer| layer.name == "Scratches")
            .unwrap();
        let pixels = layer.channels[0].as_ref().unwrap();
        let alpha = |x| pixels[((64 * surface.size + x) * 4 + 3) as usize];
        assert!(alpha(66) > 220);
        assert_eq!(
            alpha(73),
            0,
            "the 3D stencil clips the right side of the dab"
        );
        assert!(
            alpha(62) > 220,
            "padding extends two texels outside the UV edge"
        );
        assert_eq!(alpha(61), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_uses_splatter_as_a_local_brush_alpha() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-splatter-tip-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let url = "resource://starter_assets/drop_splatter/drop_splatter?version=tip.sbsar";
        for uv in [false, true] {
            let mut normal = None;
            for (inverted, horizontal) in [(false, false), (true, false), (false, true)] {
                let source = root.join(format!("Tip-{uv}-{inverted}-{horizontal}.spp"));
                let stroke = fixture::TipStroke::Splatter {
                    uv,
                    inverted,
                    horizontal,
                };
                fs::write(
                    &source,
                    fixture::hdf5_file(&[
                        ("paint/scene.assbin", fixture::assbin_quad()),
                        (
                            "paint/document.bin",
                            fixture::document_with_directed_tip(url, stroke),
                        ),
                    ]),
                )
                .unwrap();
                let (path, report) = convert(&source, &root).unwrap();
                assert!(
                    report.label.contains("слоёв частично: 1"),
                    "{}",
                    report.label
                );
                let glb = Glb::load(&root.join(path)).unwrap();
                let surface = glb
                    .paint_surface(&glb.json["meshes"][0]["primitives"][0])
                    .unwrap()
                    .unwrap();
                let pixels = surface
                    .stack
                    .layers
                    .iter()
                    .find(|layer| layer.name == "Scratches")
                    .and_then(|layer| layer.channels[0].as_ref())
                    .unwrap()
                    .clone();
                let alpha = |x: u32, y: u32| pixels[((y * surface.size + x) * 4 + 3) as usize];
                let (center, diagonal) = if uv { (192, 4) } else { (64, 10) };
                if inverted {
                    assert_eq!(alpha(center, center), 0);
                    assert!(alpha(center + diagonal, center + diagonal) > 240);
                } else {
                    assert!(alpha(center, center) > 240);
                    assert_eq!(alpha(center + diagonal, center + diagonal), 0);
                }
                if horizontal {
                    let original: &Arc<Vec<u8>> = normal.as_ref().unwrap();
                    let mut differed = false;
                    for y in center - diagonal..center + diagonal {
                        for x in center - diagonal..center + diagonal {
                            let mirror = 2 * center - 1 - x;
                            let index = |x| ((y * surface.size + x) * 4 + 3) as usize;
                            assert!(alpha(x, y).abs_diff(original[index(mirror)]) <= 2);
                            differed |= alpha(x, y) != original[index(x)];
                        }
                    }
                    assert!(differed);
                } else if !inverted {
                    normal = Some(pixels);
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_projects_procedural_splatter_in_3d_and_uv() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-splatter-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        for (uv, inverted) in [(false, false), (false, true), (true, false), (true, true)] {
            let source = root.join(format!("Splatter-{uv}-{inverted}.spp"));
            fs::write(
                &source,
                fixture::hdf5_file(&[
                    ("paint/scene.assbin", fixture::assbin_quad()),
                    (
                        "paint/document.bin",
                        fixture::document_with_procedural_stencil(uv, inverted),
                    ),
                ]),
            )
            .unwrap();
            let (path, report) = convert(&source, &root).unwrap();
            assert!(
                report.label.contains("слоёв частично: 1"),
                "{}",
                report.label
            );
            let glb = Glb::load(&root.join(path)).unwrap();
            let surface = glb
                .paint_surface(&glb.json["meshes"][0]["primitives"][0])
                .unwrap()
                .unwrap();
            let pixels = surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == "Scratches")
                .and_then(|layer| layer.channels[0].as_ref())
                .unwrap();
            let alpha = |x, y| pixels[((y * surface.size + x) * 4 + 3) as usize];
            let (center, outside) = if uv {
                (alpha(191, 191), alpha(181, 191))
            } else {
                (alpha(64, 64), alpha(80, 64))
            };
            assert_eq!(outside, 0);
            if inverted {
                assert_eq!(center, 0);
                assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0));
            } else {
                assert!(center > 200, "{center}");
            }
            if uv {
                assert_eq!(alpha(64, 191), center);
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_reconstructs_the_perlin_stencil_and_its_inverse() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-perlin-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        for uv in [false, true] {
            let import_alpha = |inverted| {
                let source = root.join(format!("Perlin-{uv}-{inverted}.spp"));
                fs::write(
                    &source,
                    fixture::hdf5_file(&[
                        ("paint/scene.assbin", fixture::assbin_quad()),
                        ("paint/document.bin", fixture::document_with_perlin_stencil(uv, inverted)),
                    ]),
                )
                .unwrap();
                let (path, report) = convert(&source, &root).unwrap();
                assert!(report.overview.contains("слоёв частично: 1"));
                let glb = Glb::load(&root.join(path)).unwrap();
                let surface = glb.paint_surface(&glb.json["meshes"][0]["primitives"][0])
                    .unwrap().unwrap();
                let pixels = surface.stack.layers.iter().find(|layer| layer.name == "Scratches")
                    .and_then(|layer| layer.channels[0].clone()).unwrap();
                move |x, y| pixels[((y * surface.size + x) * 4 + 3) as usize]
            };
            let normal = import_alpha(false);
            let inverse = import_alpha(true);
            let center = if uv { 191 } else { 64 };
            let (a, b) = (normal(center, center), inverse(center, center));
            assert!((20..235).contains(&a) && (20..235).contains(&b), "{a}, {b}");
            assert!(u16::from(a) + u16::from(b) > 200);
            assert_eq!(normal(center + 20, center), 0);
            if uv {
                assert_eq!(normal(64, center), a);
                assert_eq!(inverse(64, center), b);
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_reuses_default_planes_across_same_size_surfaces() {
        let mut textures = TextureCache::default();
        assert_eq!(textures.base_plane_count(128), 6);
        let first = textures.base(128);
        assert_eq!(textures.base_plane_count(128), 0);
        let second = textures.base(128);
        assert!(first.iter().zip(&second).all(|(a, b)| Arc::ptr_eq(a, b)));
        assert!(Arc::ptr_eq(&first[2], &first[5]));
        assert!(Arc::ptr_eq(&first[3], &first[7]));
        assert_eq!(textures.base_plane_count(256), 6);
        assert_eq!(textures.base(256)[0].len(), plane_bytes(256));
    }

    #[test]
    fn spp_import_uses_the_exact_cache_for_each_udim_tile() {
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-udim-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("Tiles.spp");
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_udim()),
                ("paint/document.bin", fixture::document_with_udim()),
                (
                    "texture-cache/4242/13537199649261162080/128",
                    fixture::cache_blob(1, 1, &[222, 255]),
                ),
                (
                    "texture-cache/4242/3864292196/128",
                    fixture::cache_blob(1, 1, &[31, 255]),
                ),
            ]),
        )
        .unwrap();
        let (filename, report) = convert(&source, &root).unwrap();
        let status = report.label;
        assert!(
            status.contains("наборов: 1 · карт: 2 · плиток UDIM: 2"),
            "{status}"
        );
        let path = root.join(filename);
        let glb = Glb::load(&path).unwrap();
        let primitives = glb.json["meshes"][0]["primitives"].as_array().unwrap();
        let model = bevy::gltf::gltf::Gltf::open(&path).unwrap();
        assert_eq!(primitives.len(), 2);
        for ((primitive, geometry), (tile, size, value, visible)) in primitives
            .iter()
            .zip(model.meshes().next().unwrap().primitives())
            .zip([(1001, 256, 31, true), (1011, 128, 222, false)])
        {
            assert_eq!(primitive["extras"]["bevy_editor_udim"], tile);
            let surface = glb.paint_surface(primitive).unwrap().unwrap();
            assert_eq!(surface.size, size);
            assert_eq!(surface.composite(1)[0], value);
            let fill = surface
                .stack
                .layers
                .iter()
                .find(|layer| layer.name == "Rust")
                .unwrap();
            assert_eq!(fill.visible, visible);
            let reader = geometry.reader(|_| model.blob.as_deref());
            assert!(
                reader
                    .read_tex_coords(0)
                    .unwrap()
                    .into_f32()
                    .all(|[u, v]| (0. ..=1.).contains(&u) && (0. ..=1.).contains(&v))
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires the locally installed Adobe Painter sample projects"]
    fn spp_import_adobe_sample_projects() {
        let samples = std::env::var_os("BEVY_SPP_SAMPLE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("ProgramFiles").unwrap())
                    .join("Adobe/Adobe Substance 3D Painter/resources/samples")
            });
        let root = std::env::temp_dir().join(format!(
            "bevy-spp-adobe-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        for (name, primitive_count) in [
            ("JadeToad", 1),
            ("MeetMat", 3),
            ("FrenchRestaurantTable", 7),
        ] {
            let source = samples.join(format!("{name}.spp"));
            let (output, report) = convert(&source, &root).unwrap();
            let status = report.label;
            println!("{name}: {status}");
            if name == "JadeToad" {
                assert!(status.contains("действий кисти и ластика:"), "{status}");
            }
            if name == "FrenchRestaurantTable" {
                assert!(status.contains("градиентных масок:"), "{status}");
                let tile_masks = status.split(" · ")
                    .find_map(|part| part.strip_prefix("плиточных масок: "))
                    .and_then(|count| count.parse::<usize>().ok());
                assert!(tile_masks.is_some_and(|count| count >= 20), "{status}");
                assert!(status.contains("приближённых процедурных карт высоты:"), "{status}");
            }
            let path = root.join(output);
            let glb = Glb::load(&path).unwrap();
            let primitives = glb.json["meshes"][0]["primitives"].as_array().unwrap();
            assert_eq!(primitives.len(), primitive_count);
            let mut table_tiles = BTreeMap::new();
            let mut adjusted_height_sources = HashSet::new();
            let mut source_file = FileReader::open(&source).unwrap();
            let metadata = source_file.project().unwrap();
            for primitive in primitives {
                let surface = glb.paint_surface(primitive).unwrap().unwrap();
                assert!(surface.stack.valid_hierarchy());
                for layer in &surface.stack.layers {
                    if let Some(fill) = &layer.texture_fill
                        && let Some(height) = &fill.images[3]
                        && [0.2_f32, 0.035, 0.65].contains(&fill.settings.height_strength)
                    {
                        adjusted_height_sources.insert(height.name.clone());
                    }
                }
                if primitive["name"] == "Table" {
                    let tile = primitive["extras"]["bevy_editor_udim"].as_u64().unwrap() as u32;
                    let index = tile - 1001;
                    let set = metadata
                        .texture_sets
                        .iter()
                        .find(|set| set.name == "Table")
                        .unwrap();
                    let cache = source_file
                        .tile_channel(set, [index % 10, index / 10], 1, surface.size)
                        .unwrap()
                        .unwrap();
                    let expected = channel_pixels(&cache, surface.size, 0);
                    assert_eq!(surface.composite(0), expected, "tile {tile}");
                    table_tiles.insert(tile, blake3::hash(&expected));
                }
            }
            if name == "FrenchRestaurantTable" {
                assert_eq!(adjusted_height_sources, HashSet::from([
                    "gradient_checker (SPP).png".to_owned(),
                    "noise_gaussian_noise (SPP).png".to_owned(),
                    "noise_directional_scratches (SPP).png".to_owned(),
                ]));
                assert_eq!(
                    table_tiles.keys().copied().collect::<Vec<_>>(),
                    [1001, 1002, 1011, 1012]
                );
                assert_eq!(
                    table_tiles.values().copied().collect::<HashSet<_>>().len(),
                    4
                );
            }
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn spp_import_rejects_truncated_container_without_publishing_an_asset() {
        let directory = std::env::temp_dir().join(format!(
            "bevy-bad-spp-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("truncated.spp");
        fs::write(&source, b"invalid").unwrap();
        assert!(convert(&source, &directory).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
