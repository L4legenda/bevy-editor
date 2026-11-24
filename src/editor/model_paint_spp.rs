//! Import Substance Painter geometry and the cached appearance into an editable GLB.
use super::{
    model_document::{MaterialSlot, MeshData, Object, glb::Glb, udim::Tile},
    model_mesh::{Corner, Face, Geometry},
    model_paint_data::{Layer, LayerKind, Stack, Surface, solid},
    ui::model_viewer::paint::{PolygonReplay, Stencil, StrokeCoverage},
};
use bevy::prelude::*;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
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
    baked_mask_filters: usize,
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
        if self.baked_mask_filters > 0 {
            summary.push_str(&format!(
                " · фильтров сведено в основу маски: {}",
                self.baked_mask_filters
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

    fn details(self) -> ImportDetails {
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
    use bevy_scene_format::{Revision, atomic_write};
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
    let name = if name.is_empty() { "Painter" } else { name };
    let (model, summary) = import(source, name)?;
    let bytes = Glb::new_asset(&[model]).map_err(io::Error::other)?;
    for number in 1.. {
        let file = if number == 1 {
            format!("{name}_SPP.glb")
        } else {
            format!("{name}_SPP_{number}.glb")
        };
        let path = target.join(&file);
        if path.exists() {
            continue;
        }
        match atomic_write(&path, &bytes, Revision::Missing) {
            Ok(_) => return Ok((PathBuf::from(file), summary.details())),
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
    let mut shared_surfaces = BTreeMap::new();
    for (slot, (material, set)) in object.materials.iter_mut().zip(matched_sets).enumerate() {
        if let Some(set) = set {
            let tile = material.udim();
            if set.use_uv_tiles && tile.is_none() {
                continue;
            }
            let per_material = needs_replay(&set.layers);
            material.paint = shared_surfaces
                .entry((set.uid, tile, per_material.then_some(slot)))
                .or_insert_with(|| {
                    decode_surface(
                        &mut file,
                        set,
                        material.authored.as_deref().unwrap(),
                        &mut remaining,
                        &mut summary,
                        (&object.geometry, slot, polygon_replay.as_ref()),
                        tile,
                    )
                    .map(Arc::new)
                })
                .clone();
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
    authored: &serde_json::Value,
    remaining: &mut usize,
    summary: &mut Summary,
    geometry: (&MeshData, usize, Option<&PolygonReplay>),
    tile: Option<Tile>,
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
    let budget = |size: u32| size as usize * size as usize * 4 * (8 + CHANNELS.len());
    let planes = rebuild::estimated_planes(&set.layers);
    let working = |size: u32| {
        budget(size).saturating_add((size as usize * size as usize * 4).saturating_mul(planes))
    };
    while size > 128 && working(size) > *remaining {
        size /= 2;
    }
    if budget(size) > *remaining {
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
    *remaining -= budget(size);
    let mut layers = Vec::new();
    if has_cache {
        layers.push(layer);
    }
    rebuild::add(
        file,
        &set.layers,
        size,
        has_cache,
        remaining,
        summary,
        &mut layers,
        geometry,
        tile,
        &scope,
    );
    let base = [
        [184, 189, 199, 255],
        [191, 191, 191, 255],
        [0, 0, 0, 255],
        [128, 128, 128, 255],
        [255; 4],
        [0, 0, 0, 255],
        [128, 128, 255, 255],
        [128, 128, 128, 255],
    ]
    .map(|value| solid(size, value));
    Some(Surface {
        size,
        base,
        stack: Stack {
            selected: if has_cache { 0 } else { layers.len() - 1 },
            layers,
        },
        material: authored.clone(),
        baked: None,
    })
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
        let details = report.spp_details().unwrap();
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
        assert_eq!(imported.generator.unwrap().kind, GeneratorKind::Dirt);
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
                TipStroke::Splatter { .. } => unreachable!(),
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
        let source = root.join("UVStencil.spp");
        let key = "1234567890123456789012345678901234567890.image";
        fs::write(
            &source,
            fixture::hdf5_file(&[
                ("paint/scene.assbin", fixture::assbin_quad()),
                (
                    "paint/document.bin",
                    fixture::document_with_uv_stencil(&format!("Stencil?version={key}")),
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
        let alpha = |x, y| pixels[((y * surface.size + x) * 4 + 3) as usize];
        let direct = alpha(190, 190);
        assert!(direct > 140, "{direct}");
        assert_eq!(alpha(65, 190), direct);
        assert_eq!(alpha(193, 190), 0);
        assert_eq!(alpha(62, 190), 0);
        assert_eq!(alpha(190, 193), 0);
        assert_eq!(alpha(65, 193), 0);
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
            let path = root.join(output);
            let glb = Glb::load(&path).unwrap();
            let primitives = glb.json["meshes"][0]["primitives"].as_array().unwrap();
            assert_eq!(primitives.len(), primitive_count);
            let mut table_tiles = BTreeMap::new();
            let mut source_file = FileReader::open(&source).unwrap();
            let metadata = source_file.project().unwrap();
            for primitive in primitives {
                let surface = glb.paint_surface(primitive).unwrap().unwrap();
                assert!(surface.stack.valid_hierarchy());
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
