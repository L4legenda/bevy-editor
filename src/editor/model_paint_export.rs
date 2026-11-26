//! Flat map export from the same layer snapshots that are saved in the GLB.
use super::*;
use crate::editor::model_document::{Document, MeshData, Object};
use crate::editor::model_paint_data::CompositeSample;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[path = "model_paint_export_templates.rs"]
mod templates;
pub(super) use templates::Template;
#[path = "model_paint_export_image.rs"]
mod image_output;
#[cfg(feature = "mcp")]
#[path = "model_paint_mcp_output.rs"]
mod mcp_output;
#[cfg(feature = "mcp")]
pub(in crate::editor) use mcp_output::{ExportArgs, McpOutput, OutputReply};
#[cfg(feature = "mcp")]
pub(super) use mcp_output::output_folder;
#[path = "model_paint_export_mesh.rs"]
mod mesh;
#[path = "model_paint_export_prefs.rs"]
pub(in crate::editor::ui::model_viewer) mod prefs;
#[path = "model_paint_export_custom.rs"]
pub(super) mod recipe;
use image_output::NormalOptions;
use image_output::Samples;
pub(super) use image_output::{Depth, Format, Options as FileOptions};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum Padding {
    #[default]
    Infinite,
    Fixed,
    Transparent,
    None,
}

impl Padding {
    pub(super) const ALL: [Self; 4] = [Self::Infinite, Self::Fixed, Self::Transparent, Self::None];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Infinite => "До края",
            Self::Fixed => "По ширине",
            Self::Transparent => "С прозрачностью",
            Self::None => "Без отступа",
        }
    }

    fn limit(self, width: u32) -> u32 {
        match self {
            Self::Infinite | Self::Transparent => u32::MAX,
            Self::Fixed => width,
            Self::None => 0,
        }
    }
}

const NO_SOURCE: u32 = u32::MAX;
const NEIGHBORS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
];

fn uv_coverage(mesh: &MeshData, slot: usize, offset: Vec2, size: u32) -> Vec<bool> {
    let faces = mesh.faces.iter().filter(|face| face.material == slot);
    mesh.uv_coverage(faces, offset, size).0
}

pub(super) fn nearest_sources(
    size: u32,
    limit: u32,
    covered: impl Iterator<Item = bool>,
) -> Vec<u32> {
    let mut sources: Vec<_> = covered
        .enumerate()
        .map(|(i, inside)| if inside { i as u32 } else { NO_SOURCE })
        .collect();
    if limit == 0 || !sources.contains(&NO_SOURCE) {
        return sources;
    }
    let mut frontier = Vec::new();
    let mut next = Vec::new();
    let grow = |index: u32, sources: &mut [u32], next: &mut Vec<u32>| {
        let x = (index % size) as i32;
        let y = (index / size) as i32;
        let source = sources[index as usize];
        let side = size as i32;
        for (dx, dy) in NEIGHBORS {
            let (nx, ny) = (x + dx, y + dy);
            if (0..side).contains(&nx) && (0..side).contains(&ny) {
                let neighbor = ny as u32 * size + nx as u32;
                if sources[neighbor as usize] == NO_SOURCE {
                    sources[neighbor as usize] = source;
                    next.push(neighbor);
                }
            }
        }
    };
    // Seed the first ring in UV raster order, without storing the whole island in the queue.
    for index in 0..sources.len() as u32 {
        if sources[index as usize] == index {
            grow(index, &mut sources, &mut frontier);
        }
    }
    for _ in 1..limit {
        if frontier.is_empty() {
            break;
        }
        for index in frontier.drain(..) {
            grow(index, &mut sources, &mut next);
        }
        std::mem::swap(&mut frontier, &mut next);
    }
    sources
}

fn pad<T: Copy + Default>(
    pixels: &mut [T],
    channels: usize,
    sources: &[u32],
    padding: Padding,
    baked: bool,
) {
    for (index, &source) in sources.iter().enumerate() {
        if source as usize == index {
            continue;
        }
        let start = index * channels;
        if source != NO_SOURCE {
            let from = source as usize * channels;
            pixels.copy_within(from..from + channels, start);
        } else if baked {
            pixels[start..start + channels].fill(T::default());
        }
        if channels == 4 && padding == Padding::Transparent {
            pixels[start + 3] = T::default();
        }
    }
}

pub(super) struct TextureSet {
    mesh: MeshData,
    slot: usize,
    uv_offset: Vec2,
    transform: Transform,
    model_bounds: (Vec3, Vec3),
    stem: String,
    tile: Option<u32>,
    source: engine::SurfaceSource,
    baked: Option<Arc<bake::Maps>>,
    custom_recipes: Arc<Vec<custom::Recipe>>,
}

#[derive(Default)]
struct GeometryMaps {
    position: Vec<[u16; 3]>,
    normal: Vec<[u16; 3]>,
}

impl GeometryMaps {
    fn bake(set: &TextureSet, recipe: &recipe::Recipe, size: u32) -> Self {
        let position = recipe.uses_source(recipe::Source::Position);
        let normal = recipe.uses_source(recipe::Source::WorldNormal);
        let mut maps = Self::default();
        if !position && !normal {
            return maps;
        }
        let len = (size as usize).pow(2);
        if position {
            maps.position.resize(len, [0; 3]);
        }
        if normal {
            maps.normal.resize(len, [32768, 32768, 65535]);
        }
        let world_positions: Vec<_> = set.mesh.positions.iter()
            .map(|&position| set.transform.transform_point(position))
            .collect();
        let (min, max) = set.model_bounds;
        let extent = (max - min).max(Vec3::splat(1e-4));
        let encode = |value: Vec3| value.to_array().map(|c| (c.clamp(0., 1.) * 65535.).round() as u16);
        let mut covered = vec![0; len];
        texture_fill::raster(
            &set.mesh, set.slot, size, set.uv_offset, set.transform, &world_positions, &mut covered,
            |texel, _, point, direction, _, _| {
                if position {
                    maps.position[texel] = encode((point - min) / extent);
                }
                if normal {
                    maps.normal[texel] = encode(direction * 0.5 + Vec3::splat(0.5));
                }
            },
        );
        maps
    }
}

#[derive(Clone)]
struct SetName {
    stem: String,
    tile: Option<u32>,
}

fn set_names(doc: &Document, selected: Option<(usize, usize)>) -> Vec<(&Object, usize, SetName)> {
    let mut names = HashSet::new();
    let mut result = Vec::new();
    for object in &doc.objects {
        let used: BTreeSet<_> = object
            .geometry
            .faces
            .iter()
            .map(|face| face.material)
            .collect();
        for (slot, material) in object
            .materials
            .iter()
            .enumerate()
            .filter(|(slot, _)| used.contains(slot))
        {
            let tile = material.udim().map(|tile| tile.number());
            let name = material.name(&doc.source.json).unwrap_or("Material");
            let root = format!("{}_{}", safe_stem(&object.name), safe_stem(name));
            let mut stem = root.clone();
            let mut suffix = 2;
            while !names.insert((stem.to_lowercase(), tile)) {
                stem = format!("{root}_{suffix}");
                suffix += 1;
            }
            if selected.is_none_or(|key| key == (object.id, slot)) {
                result.push((object, slot, SetName { stem, tile }));
            }
        }
    }
    result
}

pub(super) fn capture(
    doc: &Document,
    selected: Option<(usize, usize)>,
    images: &Assets<Image>,
    materials: &Assets<StandardMaterial>,
    baked: Option<(&bake::State, usize)>,
) -> Vec<TextureSet> {
    let mut images_by_id = HashMap::new();
    let mut sets = Vec::new();
    let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for object in &doc.objects {
        for &position in &object.geometry.positions {
            let point = object.transform.transform_point(position);
            min = min.min(point);
            max = max.max(point);
        }
    }
    for (object, slot, SetName { stem, tile }) in set_names(doc, selected) {
        let material = &object.materials[slot];
        sets.push(TextureSet {
            mesh: object.geometry.clone(),
            slot,
            uv_offset: object.uv_offset(slot),
            transform: object.transform,
            model_bounds: (min, max),
            stem,
            tile,
            custom_recipes: doc.custom_bakes.clone(),
            source: engine::surface_source(
                doc, object, slot, images, materials, &mut images_by_id,
            ),
            baked: baked
                .and_then(|(cache, model)| {
                    cache.maps(model, doc, (object.id, slot), doc.paint_size(material))
                })
                .cloned(),
        });
    }
    sets
}

fn safe_stem(name: &str) -> String {
    let stem: String = name
        .trim()
        .chars()
        .take(48)
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if stem.is_empty() {
        "Texture".into()
    } else {
        stem
    }
}

fn file_name(stem: &str, format: Format) -> String {
    format!("{stem}.{}", format.extension())
}

fn udim_suffix(tile: Option<u32>) -> String {
    tile.map_or_else(String::new, |tile| format!(".{tile}"))
}

fn template_stem(stem: &str, template: Template) -> String {
    if template == Template::Unreal { format!("T_{stem}") } else { stem.to_owned() }
}

fn custom_bake_stem(recipe: &custom::Recipe) -> String {
    format!("{}_{:016x}", safe_stem(&recipe.name), recipe.id)
}

pub(super) struct PreviewFile {
    pub name: String,
    pub depth: Depth,
    pub layout: &'static str,
    pub size: u32,
    pub source_size: u32,
    pub conditional: bool,
}

pub(super) const PREVIEW_PAGE_SIZE: usize = 32;

fn has_distinct_stems<'a>(mut stems: impl Iterator<Item = &'a str>) -> bool {
    stems.next().is_some_and(|first| stems.any(|stem| stem != first))
}

pub(super) fn preview_files(
    doc: &Document,
    session: &Session,
    project: &str,
    custom: Option<&recipe::Recipe>,
    baked: (&bake::State, usize),
) -> Vec<PreviewFile> {
    let selected = if session.export_preview_all {
        None
    } else {
        let Some(binding) = session.binding else { return Vec::new() };
        Some(binding)
    };
    let sets = set_names(doc, selected);
    let multiple_sets = has_distinct_stems(sets.iter().map(|(_, _, set)| set.stem.as_str()));
    sets.iter()
        .flat_map(|(object, slot, set)| {
            let (cache, model) = baked;
            let size = doc.paint_size(&object.materials[*slot]);
            let maps = cache.maps(model, doc, (object.id, *slot), size);
            let names = recipe::Names {
                mesh: texture_model_name(doc, session),
                project,
                set: &set.stem,
                tile: set.tile,
                multiple_sets,
            };
            preview_set(doc, names, size, session.export_settings, custom, maps.map(Arc::as_ref))
        })
        .skip(session.export_preview_page.saturating_mul(PREVIEW_PAGE_SIZE))
        .take(PREVIEW_PAGE_SIZE + 1)
        .collect()
}

fn preview_set(
    doc: &Document,
    names: recipe::Names<'_>,
    material_size: u32,
    settings: prefs::Settings,
    custom: Option<&recipe::Recipe>,
    baked: Option<&bake::Maps>,
) -> Vec<PreviewFile> {
    let stem = names.set;
    let tile = names.tile;
    let template = settings.template;
    let format = settings.file.format;
    let item = |stem: String, samples, channels, optional_alpha, source_size, conditional| {
        let layout = match (format.encoded_channels(channels), optional_alpha) {
            (1, _) => recipe::Layout::Grayscale.label(),
            (3, _) => recipe::Layout::Rgb.label(),
            (4, true) => "RGB или RGBA",
            (4, false) => recipe::Layout::Rgba.label(),
            _ => unreachable!(),
        };
        PreviewFile {
            name: file_name(&stem, format),
            depth: image_output::output_depth(samples, settings.file),
            layout,
            size: settings.file.size.unwrap_or(source_size),
            source_size,
            conditional,
        }
    };
    if let Some(custom) = custom {
        return custom.maps.iter().map(|map| {
            let components = map.components();
            let output = map.export_map(&components);
            item(names.stem(&map.name), Samples::Packed { srgb: 0, normal: false, force_16: map.force_16 },
                components.len(), output.can_drop_alpha(settings.padding, true), material_size,
                !output.skip_if_default.is_empty())
        }).collect();
    }
    let suffix = udim_suffix(tile);
    let mut files = template.maps().iter().map(|map| item(
        format!("{}_{name}{suffix}", template_stem(stem, template), name = map.name),
        map.samples(template), map.components.len(), map.can_drop_alpha(settings.padding, template != Template::Separate),
        material_size, !map.skip_if_default.is_empty(),
    )).collect::<Vec<_>>();
    if matches!(template, Template::Separate | Template::Baked) && let Some(baked) = baked {
        for map in bake::BakeMap::ALL.into_iter().filter(|&map| baked.has_map(map)) {
            files.push(item(format!("{stem}_{}{suffix}", map.export_name()), Samples::Data, 4, false, baked.size, false));
        }
        for recipe in doc.custom_bakes.iter().filter(|recipe| baked.has_map(bake::BakeMap::Custom(recipe.id))) {
            files.push(item(format!("{stem}_{}{suffix}", custom_bake_stem(recipe)), Samples::Data, 4, false, baked.size, false));
        }
    }
    files
}

fn model_name(doc: &Document) -> &str {
    doc.source.path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("Model")
}

pub(super) fn texture_model_name<'a>(doc: &'a Document, session: &'a Session) -> &'a str {
    session.export_mesh_name.as_deref().unwrap_or_else(|| model_name(doc))
}

struct ExportDirectory {
    path: PathBuf,
    files: Vec<PathBuf>,
    complete: bool,
    options: FileOptions,
}

impl ExportDirectory {
    fn create(parent: &Path, name: &str, suffix: &str, options: FileOptions) -> io::Result<Self> {
        let prefix = format!("{}_{suffix}", safe_stem(name));
        for suffix in 1.. {
            let name = if suffix == 1 {
                prefix.clone()
            } else {
                format!("{prefix}_{suffix}")
            };
            let path = parent.join(name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        files: Vec::new(),
                        complete: false,
                        options,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    fn image(
        &mut self,
        name: String,
        size: u32,
        pixels: Vec<u8>,
        channels: usize,
        samples: Samples,
    ) -> io::Result<()> {
        // Append the extension so a UDIM suffix remains part of the stem.
        let name = file_name(&name, self.options.format);
        let mut writer = self.file(&name)?;
        image_output::write(&mut writer, size, pixels, channels, samples, self.options)
            .map_err(io::Error::other)?;
        writer.flush()
    }

    fn float_image(
        &mut self,
        name: String,
        size: u32,
        pixels: Vec<f32>,
        channels: usize,
        samples: Samples,
    ) -> io::Result<()> {
        let name = file_name(&name, self.options.format);
        let mut writer = self.file(&name)?;
        image_output::write_float(&mut writer, size, pixels, channels, samples, self.options)
            .map_err(io::Error::other)?;
        writer.flush()
    }

    fn file(&mut self, name: &str) -> io::Result<BufWriter<File>> {
        let path = self.path.join(name);
        let file = File::create_new(&path)?;
        self.files.push(path);
        Ok(BufWriter::new(file))
    }

    fn finish(mut self, kind: ExportKind) -> Report {
        self.complete = true;
        Report {
            path: self.path.clone(),
            files: self.files.len(),
            #[cfg(feature = "mcp")]
            paths: std::mem::take(&mut self.files),
            kind,
        }
    }
}

impl Drop for ExportDirectory {
    fn drop(&mut self) {
        if !self.complete {
            for path in &self.files {
                let _ = fs::remove_file(path);
            }
            let _ = fs::remove_dir(&self.path);
        }
    }
}

fn material_normal(
    surface: &Surface,
    painted: Vec<u8>,
    heights: &[u8],
    baked: Option<&bake::Maps>,
    options: NormalOptions,
) -> Vec<u8> {
    let mut normal = painted;
    if options.use_baked
        && let Some(baked) = baked
        && let Some(sample) = baked.projected_tangent_normals()
    {
        mix_projected_normals(&mut normal, surface.size, baked.size, sample);
    }
    let scale = options.height_strength * surface.size as f32 / 256.;
    surface.normal_with_height_scaled(normal, heights, scale)
}

fn mix_projected_normals(
    normals: &mut [u8],
    size: u32,
    bake_size: u32,
    baked_at: impl Fn(usize) -> Option<[u8; 3]>,
) {
    mix_projected_normal_rows(normals, size, 0, bake_size, baked_at);
}

fn mix_projected_normal_rows<T: CompositeSample>(
    normals: &mut [T],
    size: u32,
    first_row: u32,
    bake_size: u32,
    baked_at: impl Fn(usize) -> Option<[u8; 3]>,
) {
    let size = size as usize;
    let bake_size = bake_size as usize;
    let flat = [128, 128, 255].map(T::from_byte);
    for (index, pixel) in normals.chunks_exact_mut(4).enumerate() {
        let sample = |coord| (coord * 2 + 1) * bake_size / (size * 2);
        let baked_index =
            sample(index / size + first_row as usize) * bake_size + sample(index % size);
        let Some(base) = baked_at(baked_index) else {
            continue;
        };
        let paint = [pixel[0], pixel[1], pixel[2]];
        if paint == flat {
            pixel[..3].copy_from_slice(&base.map(T::from_byte));
            continue;
        }
        let base = Vec3::from_array(base.map(|v| v as f32 / 127.5 - 1.)).normalize_or(Vec3::Z);
        let paint = Vec3::from_array(paint.map(|v| v.unit() * 2. - 1.)).normalize_or(Vec3::Z);
        let mixed =
            Vec3::new(base.x + paint.x, base.y + paint.y, base.z * paint.z).normalize_or(Vec3::Z);
        pixel[..3].copy_from_slice(&mixed.to_array().map(|v| T::from_unit(v * 0.5 + 0.5)));
    }
}

fn material_normal_rows(
    surface: &Surface,
    rect: URect,
    baked: Option<&bake::Maps>,
    options: NormalOptions,
) -> Vec<f32> {
    let mut normal = surface.composite_samples(6, rect);
    if options.use_baked
        && let Some(baked) = baked
        && let Some(sample) = baked.projected_tangent_normals()
    {
        mix_projected_normal_rows(&mut normal, surface.size, rect.min.y, baked.size, sample);
    }
    if options.height_strength == 0. || !surface.has_channel(3) {
        return normal;
    }
    let size = surface.size;
    let first = rect.min.y.saturating_sub(1);
    let heights: Vec<f32> = surface.composite_samples(
        3,
        URect {
            min: UVec2::new(0, first),
            max: UVec2::new(size, (rect.max.y + 1).min(size)),
        },
    );
    let height = |x, y| heights[((y - first) * size + x) as usize * 4];
    let scale = options.height_strength * size as f32 / 256.;
    for (index, pixel) in normal.chunks_exact_mut(4).enumerate() {
        let x = index as u32 % size;
        let y = rect.min.y + index as u32 / size;
        let dx = (height((x + 1).min(size - 1), y) - height(x.saturating_sub(1), y)) * scale;
        let dy = (height(x, (y + 1).min(size - 1)) - height(x, y.saturating_sub(1))) * scale;
        let vector = Vec3::new(
            pixel[0] * 2. - 1. - dx,
            pixel[1] * 2. - 1. + dy,
            (pixel[2] * 2. - 1.).max(0.01),
        )
        .normalize();
        pixel[..3].copy_from_slice(&(vector * 0.5 + Vec3::splat(0.5)).to_array());
    }
    normal
}

pub(in crate::editor::ui::model_viewer) struct Report {
    pub path: PathBuf,
    pub files: usize,
    #[cfg(feature = "mcp")]
    paths: Vec<PathBuf>,
    kind: ExportKind,
}

#[derive(Clone, Copy)]
enum ExportKind {
    Maps,
    Mesh,
}

pub(super) fn write_graph_outputs(
    parent: &Path,
    name: &str,
    outputs: [Option<image::DynamicImage>; material_painter_graph::USAGES.len()],
) -> io::Result<Report> {
    let mut directory = ExportDirectory::create(parent, name, "outputs", FileOptions::default())?;
    let stem = safe_stem(name);
    for (usage, image) in material_painter_graph::USAGES.into_iter().zip(outputs) {
        if let Some(image) = image {
            let filename = format!("{stem}_{}.png", usage.replace(' ', ""));
            let mut writer = directory.file(&filename)?;
            image.write_to(&mut writer, image::ImageFormat::Png).map_err(io::Error::other)?;
            writer.flush()?;
        }
    }
    Ok(directory.finish(ExportKind::Maps))
}

#[cfg(test)]
pub(super) fn write(
    parent: &Path,
    model: &str,
    sets: Vec<TextureSet>,
    padding: Padding,
    width: u32,
) -> io::Result<Report> {
    write_with_template(parent, model, sets, padding, width, Template::Separate)
}

#[cfg(test)]
pub(super) fn write_with_template(
    parent: &Path,
    model: &str,
    sets: Vec<TextureSet>,
    padding: Padding,
    width: u32,
    template: Template,
) -> io::Result<Report> {
    write_with_options(
        parent,
        model,
        sets,
        padding,
        width,
        template,
        FileOptions::default(),
    )
}

#[cfg(test)]
pub(super) fn write_with_options(
    parent: &Path,
    model: &str,
    sets: Vec<TextureSet>,
    padding: Padding,
    width: u32,
    template: Template,
    options: FileOptions,
) -> io::Result<Report> {
    write_textures(
        parent,
        model,
        model,
        sets,
        prefs::Settings {
            padding,
            width,
            template,
            file: options,
            ..prefs::Settings::default()
        },
        None,
    )
}

fn write_textures(
    parent: &Path,
    model: &str,
    project: &str,
    sets: Vec<TextureSet>,
    settings: prefs::Settings,
    custom: Option<&recipe::Recipe>,
) -> io::Result<Report> {
    if sets.is_empty() {
        return Err(io::Error::other(
            "У модели нет наборов текстур для экспорта",
        ));
    }
    let prefs::Settings {
        padding,
        width,
        template,
        file: options,
        ..
    } = settings;
    if matches!(template, Template::Custom(_)) && custom.is_none() {
        return Err(io::Error::other("Пользовательский шаблон не найден"));
    }
    let multiple_sets = has_distinct_stems(sets.iter().map(|set| set.stem.as_str()));
    let mut output = ExportDirectory::create(parent, model, "textures", options)?;
    for set in sets {
        let geometry = custom.map(|recipe| GeometryMaps::bake(&set, recipe, set.source.size()));
        let mut surface = set.source.build();
        if surface.needs_bake()
            && let Some(maps) = &set.baked
        {
            surface.baked = Some(maps.generator_inputs().clone());
        }
        surface.bind_generators(&set.mesh, set.slot, set.uv_offset, set.transform);
        surface.bind_texture_fills(&set.mesh, set.slot, set.uv_offset, set.transform);
        let suffix = udim_suffix(set.tile);
        let limit = padding.limit(width);
        let sources = if limit == 0 || template == Template::Baked {
            Vec::new()
        } else {
            nearest_sources(
                surface.size,
                limit,
                uv_coverage(&set.mesh, set.slot, set.uv_offset, surface.size).into_iter(),
            )
        };
        if let Some((custom, geometry)) = custom.zip(geometry.as_ref()) {
            recipe::write(
                custom,
                &mut output,
                recipe::Names {
                    mesh: model,
                    project,
                    set: &set.stem,
                    tile: set.tile,
                    multiple_sets,
                },
                &surface,
                set.baked.as_deref(),
                geometry,
                &sources,
                padding,
            )?;
            continue;
        }
        if template != Template::Baked {
            let stem = template_stem(&set.stem, template);
            templates::write(
                &mut output,
                &stem,
                set.baked.as_deref(),
                &surface,
                &suffix,
                &sources,
                padding,
                template,
            )?;
            if template != Template::Separate {
                continue;
            }
        }
        drop(sources);
        if let Some(maps) = set.baked {
            let sources = nearest_sources(maps.size, limit, maps.island_texels());
            let mut write_map = |channel: &str, map| {
                let mut pixels = maps.rgba(map);
                pad(&mut pixels, 4, &sources, padding, true);
                let samples = match map {
                    bake::BakeMap::BentNormal
                    | bake::BakeMap::WorldNormal
                    | bake::BakeMap::ProjectedNormal => Samples::Normal,
                    _ => Samples::Data,
                };
                output.image(
                    format!("{}_{channel}{suffix}", set.stem),
                    maps.size,
                    pixels,
                    4,
                    samples,
                )
            };
            for map in bake::BakeMap::ALL
                .into_iter()
                .filter(|&map| maps.has_map(map))
            {
                write_map(map.export_name(), map)?;
            }
            for recipe in set.custom_recipes.iter() {
                let map = bake::BakeMap::Custom(recipe.id);
                if maps.has_map(map) {
                    let label = custom_bake_stem(recipe);
                    write_map(&label, map)?;
                }
            }
        }
    }
    if output.files.is_empty() {
        let message = if template == Template::Baked {
            "Сначала запеките карты выбранных наборов"
        } else {
            "В выбранных наборах нет карт, которые создаёт этот шаблон"
        };
        return Err(io::Error::other(message));
    }
    Ok(output.finish(ExportKind::Maps))
}

enum Outcome {
    Written(Report),
    Imported(Vec<recipe::Recipe>),
    RecipeFile(PathBuf),
    Destination(PathBuf),
}

type ExportResult = io::Result<Option<Outcome>>;

#[derive(Default)]
pub(in crate::editor::ui::model_viewer) struct Pending(Option<Receiver<ExportResult>>);

#[derive(Default)]
pub(super) enum ActivityState {
    #[default]
    Idle,
    Exporting,
    Finished {
        message: String,
        folder: Option<PathBuf>,
        failed: bool,
    },
}

#[derive(Default, Resource)]
pub(in crate::editor::ui::model_viewer) struct Activity {
    pub(super) state: ActivityState,
    pub(super) revision: usize,
}

impl Activity {
    pub(super) fn exporting(&self) -> bool {
        matches!(self.state, ActivityState::Exporting)
    }

    fn set(&mut self, state: ActivityState) {
        self.state = state;
        self.revision = self.revision.wrapping_add(1);
    }

    fn finish(&mut self, message: &str, folder: Option<PathBuf>, failed: bool) {
        self.set(ActivityState::Finished { message: message.into(), folder, failed });
    }
}

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct RecipeName {
    pub recipe: usize,
    pub output: Option<usize>,
}

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct MeshExportName;

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct DestinationField;

fn parse_destination(input: &str) -> io::Result<Option<PathBuf>> {
    let input = input.trim();
    let input = input
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .unwrap_or(input);
    if input.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(input);
    if !path.is_absolute() || input.contains('\0') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Укажите полный путь к папке"));
    }
    Ok(Some(path))
}

enum Job {
    Maps(
        Vec<TextureSet>,
        prefs::Settings,
        Option<Arc<recipe::Recipe>>,
    ),
    Mesh(mesh::Snapshot),
}

impl Job {
    fn write(self, parent: &Path, model: &str, project: &str) -> io::Result<Report> {
        match self {
            Self::Maps(sets, settings, custom) => {
                write_textures(parent, model, project, sets, settings, custom.as_deref())
            }
            Self::Mesh(snapshot) => snapshot.write(parent, model),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    #[test]
    fn export_padding_writes_all_four_modes_on_udim() {
        let size = 8;
        let uv_offset = Vec2::new(2., 1.);
        let mesh = MeshData::from(Geometry {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            faces: vec![Face {
                corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
                    .into_iter()
                    .enumerate()
                    .map(|(vertex, uv)| {
                        let mut corner = Corner::new(vertex);
                        corner.uv = uv_offset + (Vec2::splat(3.) + uv * 2.) / size as f32;
                        corner
                    })
                    .collect(),
                material: 0,
                smooth: false,
            }],
            ..default()
        });
        let coverage = uv_coverage(&mesh, 0, uv_offset, size);
        assert_eq!(coverage.iter().filter(|&&inside| inside).count(), 4);
        let plane = |inside: [u8; 4], outside: [u8; 4]| {
            Arc::new(
                coverage
                    .iter()
                    .flat_map(|&covered| if covered { inside } else { outside })
                    .collect(),
            )
        };
        let base = std::array::from_fn(|channel| match channel {
            0 => plane([215, 117, 33, 255], [5, 10, 15, 255]),
            4 => plane([128; 4], [48; 4]),
            _ => plane([128; 4], [0; 4]),
        });
        let source = Arc::new(Surface {
            size,
            base,
            stack: Default::default(),
            material: serde_json::Value::Null,
            baked: None,
        });
        let root =
            std::env::temp_dir().join(format!("bevy-export-{}", bevy::asset::uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let inside = [215, 117, 33, 128];
        let outside = [5, 10, 15, 48];
        for (mode, near, far, opacity) in [
            (Padding::None, outside, outside, 48),
            (Padding::Fixed, inside, outside, 48),
            (Padding::Infinite, inside, inside, 128),
            (
                Padding::Transparent,
                [215, 117, 33, 0],
                [215, 117, 33, 0],
                128,
            ),
        ] {
            let report = write(
                &root,
                "Atlas",
                vec![TextureSet {
                    mesh: mesh.clone(),
                    slot: 0,
                    uv_offset,
                    transform: Transform::IDENTITY,
                    model_bounds: (Vec3::ZERO, Vec3::ONE),
                    stem: "Island".into(),
                    tile: Some(1013),
                    source: engine::SurfaceSource::Saved(source.clone()),
                    baked: None,
                    custom_recipes: Arc::default(),
                }],
                mode,
                1,
            )
            .unwrap();
            let rgba = image::open(report.path.join("Island_BaseColor.1013.png"))
                .unwrap()
                .into_rgba8();
            assert_eq!(rgba.get_pixel(3, 3).0, inside, "{mode:?}");
            assert_eq!(rgba.get_pixel(2, 2).0, near, "{mode:?}");
            assert_eq!(rgba.get_pixel(0, 0).0, far, "{mode:?}");
            let alpha = image::open(report.path.join("Island_Opacity.1013.png"))
                .unwrap()
                .into_luma8();
            assert_eq!(alpha.get_pixel(0, 0).0, [opacity], "{mode:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_padding_uses_islands_and_discards_previous_bake_padding() {
        let coverage = [1, 2, 0, 0, 0, 0, 0, 2, 1];
        let sources =
            |limit| nearest_sources(3, limit, coverage.into_iter().map(|value| value == 1));
        assert_eq!(sources(1), [0, 0, NO_SOURCE, 0, 0, 8, NO_SOURCE, 8, 8]);
        let mut rgba = [128; 36];
        pad(&mut rgba, 4, &sources(0), Padding::None, true);
        assert_eq!(&rgba[4..32], &[0; 28]);
        assert_eq!(&rgba[32..], &[128; 4]);
    }

    #[test]
    fn export_normal_mixes_projected_tangents_and_keeps_uncovered_texels() {
        let tilted = [128, 204, 230, 192];
        let flat = [128, 128, 255, 96];
        let mut normal = [flat, tilted, flat, tilted].concat();
        let baked = [Some([204, 128, 230]), None, Some([51, 128, 230]), None];
        mix_projected_normals(&mut normal, 2, 2, |index| baked[index]);
        assert_eq!(
            normal,
            [
                204, 128, 230, 96, 128, 204, 230, 192, 51, 128, 230, 96, 128, 204, 230, 192
            ]
        );
        mix_projected_normals(&mut normal, 2, 1, |_| Some([204, 128, 230]));
        let mixed =
            Vec3::from_array([normal[4], normal[5], normal[6]].map(|v| v as f32 / 127.5 - 1.));
        assert!(mixed.x > 0.5 && mixed.y > 0.5);
        assert!((mixed.length() - 1.).abs() < 0.01);
        assert_eq!(normal[7], 192);
    }

    #[test]
    fn export_normal_height_strength_varies_relief_without_changing_the_height_map() {
        let normals = [128, 128, 255, 255].repeat(4);
        let height = [[0, 0, 0, 255], [255; 4]].concat().repeat(2);
        let mut layer = Layer::new("Height".into(), LayerKind::Paint);
        layer.channels[3] = Some(Arc::new(height.clone()));
        let surface = Surface {
            size: 2,
            base: std::array::from_fn(|_| Arc::new([0, 0, 0, 255].repeat(4))),
            stack: Stack {
                layers: vec![layer],
                selected: 0,
            },
            material: serde_json::Value::Null,
            baked: None,
        };
        let options = |height_strength| NormalOptions {
            height_strength,
            ..NormalOptions::default()
        };
        assert_eq!(
            material_normal(&surface, normals.clone(), &height, None, options(0.)),
            normals
        );
        let normal = material_normal(&surface, normals.clone(), &height, None, options(1.));
        let strong = material_normal(&surface, normals, &height, None, options(8.));
        assert!(strong[0] < normal[0]);
        assert_eq!(surface.composite(3), height);
    }

    #[test]
    fn export_imported_material_painter_recipe_packs_files_and_expands_udim_names() {
        let root = std::env::temp_dir().join(format!(
            "bevy-export-recipe-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("export_templates.json");
        fs::write(
            &source,
            br#"[{
          "name":"Packed Mask",
          "maps":[
            {"name":"$project_$mesh_$set_mask.$udim","layout":"RGBA","force_16":true,"channels":[
              {"src":"Metallic","comp":0},{"src":"mesh:AmbientOcclusion","comp":0},
              {"src":"Zero","comp":0},{"src":"Roughness","comp":0,"invert":true}]},
            {"name":"Height","layout":"Grayscale","channels":[
              {"src":"Height","comp":0},{"src":"Zero"},{"src":"Zero"},{"src":"One"}]},
            {"name":"BodyColor","layout":"RGBA","channels":[
              {"src":"BaseColor","comp":0},{"src":"BaseColor","comp":1},
              {"src":"BaseColor","comp":2},{"src":"Opacity","comp":0}]},
            {"name":"OptionalEffects","layout":"RGB","channels":[
              {"src":"Emission","comp":0},{"src":"SpecularLevel","comp":0},{"src":"One"}]},
            {"name":"Glow","layout":"Grayscale","channels":[{"src":"Emission","comp":0}]},
            {"name":"Constant","layout":"Grayscale","channels":[{"src":"One"}]}
          ]
        }]"#,
        )
        .unwrap();
        let mut items = recipe::load(&source).unwrap();
        let template = items.pop().unwrap();
        assert_eq!(template.name, "Packed Mask");
        let surface = Surface {
            size: 2,
            base: std::array::from_fn(|channel| {
                let value = [235, 64, 192, 121, 255, 0, 128, 128][channel];
                Arc::new([value; 4].repeat(4))
            }),
            stack: Default::default(),
            material: serde_json::Value::Null,
            baked: None,
        };
        let sets = [("Hull", 1002), ("Glass", 1013)]
            .into_iter()
            .map(|(stem, tile)| {
                let mut surface = surface.clone();
                if stem == "Glass" {
                    surface.base[4] = Arc::new([128; 4].repeat(4));
                    surface.base[7] = Arc::new([154; 4].repeat(4));
                }
                TextureSet {
                    mesh: MeshData::from(Geometry::default()),
                    slot: 0,
                    uv_offset: Vec2::ZERO,
                    transform: Transform::IDENTITY,
                    model_bounds: (Vec3::ZERO, Vec3::ONE),
                    stem: stem.into(),
                    tile: Some(tile),
                    source: engine::SurfaceSource::Saved(Arc::new(surface)),
                    baked: None,
                    custom_recipes: Arc::default(),
                }
            })
            .collect();
        let report = write_textures(
            &root,
            "Ship",
            "Demo",
            sets,
            prefs::Settings {
                padding: Padding::None,
                template: Template::Custom(0),
                ..prefs::Settings::default()
            },
            Some(&template),
        )
        .unwrap();
        assert_eq!(report.files, 9);
        for (set, tile) in [("Hull", 1002), ("Glass", 1013)] {
            let packed =
                image::open(report.path.join(format!("Demo_Ship_{set}_mask.{tile}.png"))).unwrap();
            assert_eq!(packed.color(), image::ColorType::Rgba16);
            assert_eq!(
                packed.to_rgba16().get_pixel(0, 0).0,
                [192u16, 255, 0, 191].map(|v| v * 257)
            );
            let height = image::open(report.path.join(format!("{set}_Height.{tile}.png"))).unwrap();
            assert_eq!(height.to_luma8().get_pixel(0, 0).0, [121]);
            assert!(!report.path.join(format!("{set}_Glow.{tile}.png")).exists());
            assert!(report.path.join(format!("{set}_Constant.{tile}.png")).is_file());
        }
        assert!(!report.path.join("Hull_OptionalEffects.1002.png").exists());
        let effects = image::open(report.path.join("Glass_OptionalEffects.1013.png")).unwrap();
        assert_eq!(effects.to_rgb8().get_pixel(0, 0).0, [0, 154, 255]);
        let hull = image::open(report.path.join("Hull_BodyColor.1002.png")).unwrap();
        let glass = image::open(report.path.join("Glass_BodyColor.1013.png")).unwrap();
        assert_eq!(hull.color(), image::ColorType::Rgb8);
        assert_eq!(glass.color(), image::ColorType::Rgba8);
        assert_eq!(glass.to_rgba8().get_pixel(0, 0).0[3], 128);
        fs::write(&source, br#"{"name":"Escape","maps":[{"name":"../outside","layout":"Grayscale","channels":[{"src":"One"}]}]}"#).unwrap();
        assert!(recipe::load(&source).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_geometry_sources_use_the_transformed_udim_without_mesh_bakes() {
        let root = std::env::temp_dir().join(format!(
            "bevy-export-geometry-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let recipe: recipe::Recipe = serde_json::from_value(serde_json::json!({
            "name": "Geometry",
            "maps": [
                { "name": "Position", "layout": "RGB", "channels": [
                    { "src": "Position", "comp": 0 }, { "src": "Position", "comp": 1 }, { "src": "Position", "comp": 2 }
                ] },
                { "name": "Normal", "layout": "RGB", "channels": [
                    { "src": "WorldNormal", "comp": 0 }, { "src": "WorldNormal", "comp": 1 }, { "src": "WorldNormal", "comp": 2 }
                ] }
            ]
        })).unwrap();
        let offset = Vec2::new(1., -1.);
        let mut geometry = Geometry::default();
        geometry.positions = vec![Vec3::ZERO, Vec3::X * 2., Vec3::new(2., 4., 0.), Vec3::Y * 4.];
        geometry.faces.push(Face {
            corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y].into_iter().enumerate()
                .map(|(vertex, uv)| {
                    let mut corner = Corner::new(vertex);
                    corner.uv = uv + offset;
                    corner
                }).collect(),
            material: 0,
            smooth: false,
        });
        let set = TextureSet {
            mesh: geometry.into(),
            slot: 0,
            uv_offset: offset,
            transform: Transform::from_translation(Vec3::new(1., 2., 5.))
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
            model_bounds: (Vec3::ZERO, Vec3::new(2., 8., 10.)),
            stem: "Hull".into(),
            tile: Some(1012),
            source: engine::SurfaceSource::Saved(Arc::new(Surface {
                size: 2,
                base: std::array::from_fn(|_| Arc::new([255; 16].to_vec())),
                stack: Default::default(),
                material: serde_json::Value::Null,
                baked: None,
            })),
            baked: None,
            custom_recipes: Arc::default(),
        };
        let report = write_textures(
            &root, "Hull", "Test", vec![set],
            prefs::Settings {
                template: Template::Custom(0),
                padding: Padding::None,
                file: FileOptions { depth: Depth::U16, ..FileOptions::default() },
                ..prefs::Settings::default()
            },
            Some(&recipe),
        ).unwrap();
        for (file, expected) in [
            ("Position.1012.png", [32768, 24576, 29491]),
            ("Normal.1012.png", [65535, 32768, 32768]),
        ] {
            let actual = image::open(report.path.join(file)).unwrap().into_rgb16().get_pixel(0, 0).0;
            assert!(actual.into_iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "{file}: {actual:?}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_high_depth_preserves_sub_byte_layer_blending() {
        let size = 128;
        let mut layer = Layer::new("Sub-byte stroke".into(), LayerKind::Paint);
        layer.opacity = 25.;
        // Paint from the last row of the first stripe through the second stripe.
        let mut paint = vec![0; (size * size * 4) as usize];
        paint[(63 * size * 4) as usize..]
            .chunks_exact_mut(4)
            .for_each(|pixel| pixel.copy_from_slice(&[129, 129, 129, 255]));
        let paint = Arc::new(paint);
        layer.channels[1] = Some(paint.clone());
        layer.channels[3] = Some(paint);
        let surface = Arc::new(Surface {
            size,
            base: std::array::from_fn(|c| {
                let rgba = if c == 6 {
                    [128, 128, 255, 255]
                } else {
                    [128, 128, 128, 255]
                };
                Arc::new(rgba.repeat((size * size) as usize))
            }),
            stack: Stack {
                layers: vec![layer],
                selected: 0,
            },
            material: serde_json::Value::Null,
            baked: None,
        });
        let sets = || {
            vec![TextureSet {
                mesh: MeshData::from(Geometry::default()),
                slot: 0,
                uv_offset: Vec2::ZERO,
                transform: Transform::IDENTITY,
                model_bounds: (Vec3::ZERO, Vec3::ONE),
                stem: "Surface".into(),
                tile: None,
                source: engine::SurfaceSource::Saved(surface.clone()),
                baked: None,
                custom_recipes: Arc::default(),
            }]
        };
        let root = std::env::temp_dir().join(format!(
            "bevy-export-precision-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let recipe_path = root.join("Recipe.json");
        fs::write(&recipe_path, br#"{"name":"Precision","maps":[{"name":"Fine","layout":"Grayscale","force_16":true,"channels":[{"src":"Roughness"}]}]}"#).unwrap();
        let recipe = recipe::load(&recipe_path).unwrap().pop().unwrap();
        for (template, format, depth, name, component) in [
            (
                Template::Separate,
                Format::Png,
                Depth::U8,
                "Surface_Roughness.png",
                0,
            ),
            (
                Template::Separate,
                Format::Png,
                Depth::U16,
                "Surface_Roughness.png",
                0,
            ),
            (
                Template::Separate,
                Format::Exr,
                Depth::Float,
                "Surface_Roughness.exr",
                0,
            ),
            (
                Template::Godot,
                Format::Tiff,
                Depth::U16,
                "Surface_ORM.tif",
                1,
            ),
            (Template::Custom(0), Format::Png, Depth::U8, "Fine.png", 0),
            (
                Template::Custom(0),
                Format::Exr,
                Depth::Float,
                "Fine.exr",
                0,
            ),
        ] {
            let custom = (template == Template::Custom(0)).then_some(&recipe);
            let report = write_textures(
                &root,
                "Model",
                "Project",
                sets(),
                prefs::Settings {
                    padding: Padding::None,
                    template,
                    file: FileOptions {
                        format,
                        depth,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                custom,
            )
            .unwrap();
            let written = image::open(report.path.join(name)).unwrap();
            for (y, sample) in [(62, 128.), (63, 128.25), (64, 128.25), (127, 128.25)] {
                if depth == Depth::Float {
                    let pixel = written.to_rgba32f().get_pixel(0, y)[component];
                    assert!((pixel - sample / 255.).abs() < 1e-6);
                } else if depth == Depth::U16 || custom.is_some() {
                    assert_eq!(
                        written.to_rgba16().get_pixel(0, y)[component],
                        (sample * 257_f32).round() as u16
                    );
                } else {
                    assert_eq!(written.to_rgba8().get_pixel(0, y)[component], 128);
                }
            }
            if depth == Depth::Float && template == Template::Separate {
                let normals = image::open(report.path.join("Surface_Normal.exr"))
                    .unwrap()
                    .into_rgb32f();
                let flat = normals.get_pixel(0, 61)[1];
                let border = normals.get_pixel(0, 63)[1];
                assert!(border - flat > 0.0002, "{flat} -> {border}");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn engine_export_templates_pack_channels_and_preserve_udim_names() {
        let base = [
            [215, 117, 33, 255],
            [64; 4],
            [192; 4],
            [128; 4],
            [172; 4],
            [0, 0, 0, 255],
            [143, 157, 230, 255],
            [128; 4],
        ];
        let surface = Arc::new(Surface {
            size: 2,
            base: base.map(|rgba| Arc::new(rgba.repeat(4))),
            stack: Default::default(),
            material: serde_json::Value::Null,
            baked: None,
        });
        let sets = || {
            vec![TextureSet {
                mesh: MeshData::from(Geometry::default()),
                slot: 0,
                uv_offset: Vec2::X,
                transform: Transform::IDENTITY,
                model_bounds: (Vec3::ZERO, Vec3::ONE),
                stem: "Set".into(),
                tile: Some(1002),
                source: engine::SurfaceSource::Saved(surface.clone()),
                baked: None,
                custom_recipes: Arc::default(),
            }]
        };
        let root = std::env::temp_dir().join(format!(
            "bevy-export-presets-{}",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        fs::create_dir(&root).unwrap();
        for (template, checks, count) in [
            (
                Template::Unreal,
                &[
                    ("T_Set_OcclusionRoughnessMetallic", &[255, 64, 192, 255][..]),
                    ("T_Set_Normal", &[143, 98, 230, 255]),
                    ("T_Set_BaseColor", &[215, 117, 33, 172]),
                ][..],
                3,
            ),
            (
                Template::UnityUrp,
                &[("Set_MaskMap", &[192, 255, 0, 191][..])],
                4,
            ),
            (
                Template::UnityHdrp,
                &[("Set_MaskMap", &[192, 255, 255, 191][..])],
                4,
            ),
            (
                Template::UnityStandard,
                &[("Set_MetallicSmoothness", &[192, 192, 192, 191][..])],
                5,
            ),
            (Template::Godot, &[("Set_ORM", &[255, 64, 192, 255][..])], 4),
            (
                Template::Gltf,
                &[("Set_occlusionRoughnessMetallic", &[255, 64, 192, 255][..])],
                3,
            ),
            (
                Template::Roblox,
                &[("Set_Metalness", &[192, 192, 192, 255][..])],
                4,
            ),
            (
                Template::Blender,
                &[("Set_Normal", &[143, 157, 230, 255][..])],
                5,
            ),
            (
                Template::MetallicRoughness,
                &[("Set_AmbientOcclusion", &[255; 4][..])],
                6,
            ),
            (
                Template::SpecularGlossiness,
                &[("Set_Glossiness", &[191, 191, 191, 255][..])],
                6,
            ),
        ] {
            let report =
                write_with_template(&root, "Model", sets(), Padding::None, 8, template).unwrap();
            assert_eq!(report.files, count, "{template:?}");
            for (name, pixel) in checks {
                let image = image::open(report.path.join(format!("{name}.1002.png")))
                    .unwrap()
                    .into_rgba8();
                assert_eq!(&image.get_pixel(0, 0).0, pixel, "{template:?} {name}");
            }
        }
        assert!(
            write_with_template(&root, "Baked", sets(), Padding::None, 8, Template::Baked).is_err()
        );
        assert!(!root.join("Baked_textures").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

pub(in crate::editor::ui::model_viewer) fn actions(
    mut pending: Local<Pending>,
    mut messages: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    editing: Res<editing::State>,
    mut paint: ResMut<PaintState>,
    baked: Res<bake::State>,
    images: Res<Assets<Image>>,
    materials: Res<Assets<StandardMaterial>>,
    mut preferences: ResMut<prefs::Store>,
    mut activity: ResMut<Activity>,
    names: Query<(&RecipeName, &bevy::text::EditableText)>,
    mesh_names: Query<&bevy::text::EditableText, With<MeshExportName>>,
    destinations: Query<&bevy::text::EditableText, With<DestinationField>>,
) {
    let pending = &mut pending.0;
    if let Some(index) = editor.viewing_model
        && editor.viewing_image.is_none()
        && editor.pending.is_none()
        && editing
            .documents
            .get(&index)
            .is_some_and(|document| document.mode == Mode::Paint)
        && let Some(session) = paint.sessions.get(&index)
    {
        preferences.set_settings(session.export_settings);
    }
    preferences.schedule_save();
    if let Some(task) = pending.as_ref() {
        let result = match task.try_recv() {
            Err(TryRecvError::Empty) => {
                messages.clear();
                return;
            }
            Err(TryRecvError::Disconnected) => Err(io::Error::other(
                "Задача экспорта завершилась без результата",
            )),
            Ok(result) => result,
        };
        *pending = None;
        let folder = match &result {
            Ok(Some(Outcome::Written(report))) if activity.exporting() => Some(report.path.clone()),
            _ => None,
        };
        let failed = result.is_err();
        editor.status = match result {
            Ok(Some(Outcome::Written(report))) => {
                if let Some(parent) = report.path.parent() {
                    preferences.set_folder(parent);
                }
                let mut status = match report.kind {
                    ExportKind::Maps => format!(
                        "Экспортировано карт: {} · {}",
                        report.files,
                        report.path.display()
                    ),
                    ExportKind::Mesh => format!(
                        "Сетка экспортирована в OBJ и MTL · {}",
                        report.path.display()
                    ),
                };
                if preferences.settings().reveal_folder
                    && let Err(error) = crate::project_manager::reveal_folder(&report.path)
                {
                    status.push_str(&format!(" · Не удалось открыть папку: {error}"));
                }
                status
            }
            Ok(Some(Outcome::Imported(recipes))) => match preferences.import(recipes) {
                Ok(0) => "Шаблон уже в списке экспорта".into(),
                Ok(added) => format!("Добавлено шаблонов экспорта: {added}"),
                Err(error) => format!("Не удалось добавить шаблоны · {error}"),
            },
            Ok(Some(Outcome::RecipeFile(path))) => format!("Шаблон сохранён · {}", path.display()),
            Ok(Some(Outcome::Destination(path))) => {
                let status = format!("Папка экспорта · {}", path.display());
                preferences.set_destination(Some(path));
                status
            }
            Ok(None) => "Операция отменена".into(),
            Err(error) => format!("Не удалось завершить операцию · {error}"),
        };
        if activity.exporting() {
            activity.finish(&editor.status, folder, failed);
        }
    }
    for action in messages.read() {
        let Action::ModelView(ModelViewAction::Paint(
            action @ (PaintAction::ExportMaps(_)
            | PaintAction::ExportMesh
            | PaintAction::ApplyExportMeshName
            | PaintAction::ApplyExportDestination
            | PaintAction::BrowseExportDestination
            | PaintAction::RevealExportResult
            | PaintAction::ImportExportTemplate
            | PaintAction::CopyExportTemplate
            | PaintAction::SaveExportTemplate
            | PaintAction::EditExportTemplate { .. }),
        )) = *action
        else {
            continue;
        };
        let Some(index) = editor.viewing_model else {
            continue;
        };
        let Some(doc) = editing
            .documents
            .get(&index)
            .filter(|doc| doc.mode == Mode::Paint)
        else {
            continue;
        };
        if editor.pending.is_some() || editor.viewing_image.is_some() {
            continue;
        }
        if action == PaintAction::RevealExportResult {
            if let ActivityState::Finished { folder: Some(folder), .. } = &activity.state
                && let Err(error) = crate::project_manager::reveal_folder(folder)
            {
                editor.status = format!("Не удалось открыть папку · {error}");
            }
            continue;
        }
        if action == PaintAction::BrowseExportDestination {
            let folder = preferences.destination().or_else(|| preferences.folder()).map(Path::to_owned);
            let (sender, receiver) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("paint-choose-export-folder".into())
                .spawn(move || {
                    let result = crate::project_manager::choose_folder_in(
                        "Bevy Editor: папка для экспорта карт",
                        folder.as_deref(),
                    ).map(|path| path.map(Outcome::Destination));
                    let _ = sender.send(result);
                });
            match worker {
                Ok(_) => {
                    *pending = Some(receiver);
                    editor.status = "Выбор папки экспорта…".into();
                }
                Err(error) => editor.status = format!("Не удалось открыть выбор папки · {error}"),
            }
            messages.clear();
            break;
        }
        if matches!(action, PaintAction::ApplyExportDestination | PaintAction::ExportMaps(_))
            && let Ok(input) = destinations.single()
        {
            let current = preferences.destination().map(Path::to_string_lossy).unwrap_or_default();
            if input.value() != current.as_ref() {
                match parse_destination(&input.value().to_string()) {
                    Ok(destination) => preferences.set_destination(destination),
                    Err(error) => {
                        editor.status = format!("Папка экспорта · {error}");
                        if matches!(action, PaintAction::ExportMaps(_)) {
                            activity.finish(&editor.status, None, true);
                        }
                        continue;
                    }
                }
            }
        }
        if action == PaintAction::ApplyExportDestination {
            continue;
        }
        if matches!(action, PaintAction::ApplyExportMeshName | PaintAction::ExportMaps(_))
            && let (Ok(input), Some(session)) = (mesh_names.single(), paint.sessions.get_mut(&index))
            && input.value() != texture_model_name(doc, session)
        {
            let value = input.value().to_string();
            let value = value.trim();
            session.export_mesh_name =
                (!value.is_empty() && value != model_name(doc)).then(|| safe_stem(value));
            session.revision += 1;
        }
        if action == PaintAction::ApplyExportMeshName {
            continue;
        }
        if action == PaintAction::CopyExportTemplate {
            editor.status = match preferences.duplicate() {
                Ok(()) => "Добавлена копия шаблона".into(),
                Err(error) => format!("Не удалось скопировать шаблон · {error}"),
            };
            continue;
        }
        if let PaintAction::EditExportTemplate {
            recipe,
            output,
            channel,
            edit,
        } = action
        {
            if preferences.settings().template != Template::Custom(recipe) {
                continue;
            }
            let text = match edit {
                recipe::Edit::RenameTemplate | recipe::Edit::RenameOutput => {
                    let output = (edit == recipe::Edit::RenameOutput).then_some(output);
                    names
                        .iter()
                        .find(|(field, _)| field.recipe == recipe && field.output == output)
                        .map(|(_, input)| input.value().to_string())
                }
                _ => None,
            };
            match preferences.edit(recipe, output, channel, edit, text.as_deref()) {
                Ok(()) => {
                    if let Some(session) = paint.sessions.get_mut(&index)
                        && let Some(updated) = preferences.recipe(recipe)
                    {
                        session.export_output = match edit {
                            recipe::Edit::Add => updated.maps.len() - 1,
                            recipe::Edit::Move(true) => output - 1,
                            recipe::Edit::Move(false) => output + 1,
                            _ => session.export_output.min(updated.maps.len() - 1),
                        };
                        if matches!(edit, recipe::Edit::SetSource(_)) {
                            session.export_source_menu = false;
                        }
                        session.export_component = session
                            .export_component
                            .min(updated.maps[session.export_output].layout.len() - 1);
                        session.revision += 1;
                    }
                }
                Err(error) => editor.status = format!("Не удалось изменить шаблон · {error}"),
            }
            continue;
        }
        if action == PaintAction::SaveExportTemplate {
            let settings = preferences.settings();
            let selected = preferences
                .selected_recipe()
                .or_else(|| recipe::Recipe::from_builtin(settings.template).map(Arc::new));
            let Some(recipe) = selected else { continue };
            let folder = preferences.folder().map(Path::to_owned);
            let (sender, receiver) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("paint-save-export-template".into())
                .spawn(move || {
                    let result = crate::project_manager::choose_folder_in(
                        "Bevy Editor: папка для шаблона JSON",
                        folder.as_deref(),
                    )
                    .and_then(|path| {
                        path.map(|path| recipe.write_json(&path).map(Outcome::RecipeFile))
                            .transpose()
                    });
                    let _ = sender.send(result);
                });
            match worker {
                Ok(_) => {
                    *pending = Some(receiver);
                    editor.status = "Сохранение шаблона…".into();
                }
                Err(error) => editor.status = format!("Не удалось запустить сохранение · {error}"),
            }
            messages.clear();
            break;
        }
        if action == PaintAction::ImportExportTemplate {
            let root = editor.project.asset_root.clone();
            let (sender, receiver) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("paint-export-template".into())
                .spawn(move || {
                    let result =
                        crate::editor::asset_monitor::choose_file_path(&root).and_then(|path| {
                            path.map(|path| recipe::load(&path).map(Outcome::Imported))
                                .transpose()
                        });
                    let _ = sender.send(result);
                });
            match worker {
                Ok(_) => {
                    *pending = Some(receiver);
                    editor.status = "Импорт шаблонов экспорта…".into();
                }
                Err(error) => editor.status = format!("Не удалось запустить импорт · {error}"),
            }
            messages.clear();
            break;
        }
        if doc.paint_busy {
            editor.status = "Дождитесь завершения мазка перед экспортом".into();
            continue;
        }
        let mesh = action == PaintAction::ExportMesh;
        let job = if let PaintAction::ExportMaps(all) = action {
            let selected = if all {
                None
            } else {
                let Some(key) = paint
                    .sessions
                    .get(&index)
                    .and_then(|session| session.binding)
                else {
                    continue;
                };
                Some(key)
            };
            let sets = capture(doc, selected, &images, &materials, Some((&baked, index)));
            if sets.is_empty() {
                editor.status = "У модели нет наборов текстур для экспорта".into();
                activity.finish(&editor.status, None, true);
                continue;
            }
            Job::Maps(sets, preferences.settings(), preferences.selected_recipe())
        } else {
            Job::Mesh(mesh::Snapshot::capture(doc))
        };
        let model = paint.sessions.get(&index)
            .filter(|_| !mesh)
            .map_or_else(|| model_name(doc), |session| texture_model_name(doc, session))
            .to_owned();
        let start_directory = preferences.folder().map(Path::to_owned);
        let destination = preferences.destination().filter(|_| !mesh).map(Path::to_owned);
        let project = editor.project.title.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("paint-export".into())
            .spawn(move || {
                let parent = if let Some(path) = destination {
                    fs::create_dir_all(&path).map(|()| Some(path))
                } else {
                    crate::project_manager::choose_folder_in(
                        if mesh {
                            "Bevy Editor: папка для экспорта OBJ"
                        } else {
                            "Bevy Editor: папка для экспорта карт"
                        },
                        start_directory.as_deref(),
                    )
                };
                let result = parent.and_then(|parent| {
                    parent
                        .map(|parent| job.write(&parent, &model, &project).map(Outcome::Written))
                        .transpose()
                });
                let _ = sender.send(result);
            });
        match worker {
            Ok(_) => {
                *pending = Some(receiver);
                if !mesh {
                    activity.set(ActivityState::Exporting);
                }
                editor.status = if mesh {
                    "Экспорт сетки OBJ…"
                } else {
                    "Экспорт карт материала…"
                }
                .into();
            }
            Err(error) => {
                editor.status = format!("Не удалось запустить экспорт · {error}");
                if !mesh {
                    activity.finish(&editor.status, None, true);
                }
            }
        }
        messages.clear();
        break;
    }
}
