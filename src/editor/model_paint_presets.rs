//! Self-contained presets for reuse on a different mesh or in a different project.
use super::{
    Blend, Filter, Generator, Layer, LayerKind, MAX_FILTERS, MAX_LAYER_DEPTH, MAX_LAYERS,
    MAX_MASK_LAYERS, Mask, MaskLayer, material_graph, shelf_preview, texture_fill,
};
use crate::editor::model_paint_data::{CHANNEL_COUNT, mask::GraphMask, solid};
use material_graph::file::package;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[path = "model_paint_mpsmart.rs"]
mod mpsmart;
pub(in crate::editor) use mpsmart::{ProjectGraph, project_fill, project_graph, project_mask};

const EXTENSION: &str = "bevypaint";
const MAX_FILE: u64 = 194 * 1024 * 1024;
const MANIFEST: &str = "preset.json";
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_IMAGE: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MaskPreset {
    pixels: [u8; 4],
    affine: bool,
    pub generator: Option<Generator>,
    filters: Vec<Filter>,
    layers: Vec<MaskLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph: Option<MaskGraphPreset>,
}

impl MaskPreset {
    fn capture(mask: &Mask, images: &mut Vec<Arc<texture_fill::Bitmap>>) -> (Self, usize, bool) {
        let uniform = uniform(&mask.pixels);
        let pixels = uniform.unwrap_or(if mask.affine {
            [255, 0, 0, 255]
        } else {
            [255; 4]
        });
        let mut skipped = usize::from(uniform.is_none());
        let mut portable = |mut generator: Generator| {
            // Recipe IDs are local to a model. Copying them could bind a different map.
            for input in &mut generator.custom {
                if input.id != 0 {
                    *input = Default::default();
                    skipped += 1;
                }
            }
            generator
        };
        let generator = mask.generator.map(&mut portable);
        let layers = mask
            .layers
            .iter()
            .map(|layer| MaskLayer {
                generator: layer.generator.map(&mut portable),
                ..*layer
            })
            .collect();
        let graph = mask.graph.as_ref().map(|graph| MaskGraphPreset {
            payload: graph.payload.clone(),
            images: graph
                .images
                .iter()
                .map(|(key, image)| (key.clone(), capture_image(images, image)))
                .collect(),
            output: graph
                .output
                .as_ref()
                .map(|image| capture_image(images, image)),
        });
        (
            Self {
                pixels,
                affine: mask.affine,
                generator,
                filters: mask.filters.clone(),
                layers,
                graph,
            },
            skipped,
            uniform.is_none(),
        )
    }

    fn valid(&self) -> bool {
        self.filters.len() <= MAX_FILTERS
            && self.filters.iter().all(Filter::valid)
            && self.layers.len() <= MAX_MASK_LAYERS
            && self.layers.iter().all(|layer| layer.valid())
            && self.generator.is_none_or(|g| self.affine && g.valid())
            && self.graph.as_ref().is_none_or(|graph| {
                self.affine && graph.payload.is_object() && graph.images.len() <= 64
            })
    }

    pub fn build(&self, size: u32) -> Arc<Mask> {
        self.with_pixels(solid(size, self.pixels))
    }

    pub fn has_graph(&self) -> bool {
        self.graph.is_some()
    }

    fn load(
        &self,
        pixels: Arc<Vec<u8>>,
        bitmap: &mut impl FnMut(&ImageRef) -> Result<Arc<texture_fill::Bitmap>, String>,
    ) -> Result<Arc<Mask>, String> {
        let mut mask = self.with_pixels(pixels);
        if let Some(graph) = &self.graph {
            let images = graph
                .images
                .iter()
                .map(|(key, image)| Ok((key.clone(), bitmap(image)?)))
                .collect::<Result<texture_fill::GraphImages, String>>()?;
            let output = graph.output.as_ref().map(bitmap).transpose()?;
            Arc::make_mut(&mut mask).set_graph(Arc::new(GraphMask::new(
                graph.payload.clone(),
                Arc::new(images),
                output,
            )));
        }
        Ok(mask)
    }

    fn with_pixels(&self, pixels: Arc<Vec<u8>>) -> Arc<Mask> {
        let mut mask = Mask::new(pixels, self.filters.clone());
        mask.affine = self.affine;
        mask.generator = self.generator;
        *mask.edit_layers() = self.layers.clone();
        Arc::new(mask)
    }

    pub fn preview_generator(&self) -> Option<Generator> {
        self.generator
            .or_else(|| self.layers.iter().find_map(|layer| layer.generator))
    }

    pub fn brightness(&self) -> u8 {
        self.pixels[0]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ImageRef {
    file: String,
    name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MaskGraphPreset {
    payload: Arc<serde_json::Value>,
    images: BTreeMap<String, ImageRef>,
    output: Option<ImageRef>,
}

#[derive(Serialize, Deserialize)]
struct FillPreset {
    settings: texture_fill::Settings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph: Option<Arc<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    graph_images: BTreeMap<String, ImageRef>,
    images: [Option<ImageRef>; CHANNEL_COUNT],
}

#[derive(Serialize, Deserialize)]
struct FolderGraphPreset {
    payload: Arc<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    images: BTreeMap<String, ImageRef>,
}

#[derive(Serialize, Deserialize)]
struct LayerPreset {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder_graph: Option<FolderGraphPreset>,
    kind: LayerKind,
    visible: bool,
    #[serde(default)]
    collapsed: bool,
    opacity: f32,
    blend: Blend,
    depth: usize,
    channels: [Option<[u8; 4]>; CHANNEL_COUNT],
    mask: Option<MaskPreset>,
    fill: Option<FillPreset>,
}

#[derive(Serialize, Deserialize)]
enum Content {
    Material(Vec<LayerPreset>),
    Mask(MaskPreset),
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    name: String,
    content: Content,
}

#[derive(Clone)]
pub(super) enum Kind {
    Material {
        layers: usize,
        depth: usize,
        color: [u8; 3],
        roughness: f32,
        metallic: f32,
    },
    Mask(MaskPreset),
}

#[derive(Clone)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    pub thumbnail: Option<Arc<image::RgbaImage>>,
}

pub(super) enum Capture {
    Material { name: String, layers: Vec<Layer> },
    Mask { name: String, mask: Arc<Mask> },
}

pub(in crate::editor::ui::model_viewer) fn directory() -> Option<PathBuf> {
    #[cfg(test)]
    return None;
    #[cfg(not(test))]
    {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .or_else(|| std::env::home_dir().map(|home| home.join("AppData/Roaming")))
                .map(|base| base.join("BevyEditor"))
        } else if cfg!(target_os = "macos") {
            std::env::home_dir().map(|home| home.join("Library/Application Support/BevyEditor"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
                .map(|base| base.join("bevy-editor"))
        };
        base.map(|base| base.join("paint-library"))
    }
}

fn uniform(pixels: &[u8]) -> Option<[u8; 4]> {
    let value: [u8; 4] = pixels.get(..4)?.try_into().ok()?;
    pixels
        .chunks_exact(4)
        .all(|pixel| pixel == value)
        .then_some(value)
}

pub(super) fn list(directory: &Path) -> Result<Vec<Entry>, String> {
    let files = match fs::read_dir(directory) {
        Ok(files) => files,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut entries = files
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == EXTENSION || ext.eq_ignore_ascii_case(mpsmart::EXTENSION))
        })
        .filter_map(|path| entry_at(&path).ok())
        .collect::<Vec<_>>();
    entries.sort_by_cached_key(|entry| entry.name.to_lowercase());
    Ok(entries)
}

fn entry_at(path: &Path) -> Result<Entry, String> {
    let mut archive = open(path)?;
    let manifest = manifest_at(path, &mut archive)?;
    let mut result = entry(path.to_owned(), &manifest);
    result.thumbnail = thumbnail_from(&mut archive);
    Ok(result)
}

fn entry(path: PathBuf, manifest: &Manifest) -> Entry {
    let kind = match &manifest.content {
        Content::Mask(mask) => Kind::Mask(mask.clone()),
        Content::Material(layers) => {
            let plane = |channel: usize| layers.iter().find_map(|layer| layer.channels[channel]);
            let [r, g, b, _] = plane(0).unwrap_or([185, 185, 185, 255]);
            Kind::Material {
                layers: layers.len(),
                depth: layers.iter().map(|layer| layer.depth).max().unwrap_or(0),
                color: [r, g, b],
                roughness: f32::from(plane(1).map_or(127, |p| p[0])) / 255.,
                metallic: f32::from(plane(2).map_or(0, |p| p[0])) / 255.,
            }
        }
    };
    let number = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.rsplit_once(" ~"))
        .and_then(|(_, suffix)| suffix.parse::<usize>().ok());
    let name = number.map_or_else(
        || manifest.name.clone(),
        |number| format!("{} {number}", manifest.name),
    );
    Entry {
        path,
        name,
        kind,
        thumbnail: None,
    }
}

type Archive<R = File> = zip::ZipArchive<R>;

fn open(path: &Path) -> Result<Archive, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_FILE {
        return Err("Файл пресета превышает допустимый размер".into());
    }
    let archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if archive.len() > 67 {
        return Err("Слишком много файлов в пресете".into());
    }
    Ok(archive)
}

fn read(
    archive: &mut Archive<impl Read + Seek>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let file = archive.by_name(name).map_err(|error| error.to_string())?;
    if file.size() > limit {
        return Err("Файл пресета превышает допустимый размер".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Файл пресета превышает допустимый размер".into());
    }
    Ok(bytes)
}

fn manifest(archive: &mut Archive) -> Result<Manifest, String> {
    let decoded: Manifest = serde_json::from_slice(&read(archive, MANIFEST, MAX_MANIFEST)?)
        .map_err(|error| error.to_string())?;
    if decoded.version != 1 || !valid(&decoded.content) {
        return Err("Некорректный пресет рисования".into());
    }
    Ok(decoded)
}

fn manifest_at(path: &Path, archive: &mut Archive) -> Result<Manifest, String> {
    if is_mpsmart(path) {
        mpsmart::decode(archive).map(|(manifest, _)| manifest)
    } else {
        manifest(archive)
    }
}

pub(super) fn is_mpsmart(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(mpsmart::EXTENSION))
}

pub(super) fn import_material(directory: &Path, path: &Path) -> Result<(Entry, usize), String> {
    if package::matches(path) {
        return import_material_package(directory, path);
    }
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(mpsmart::EXTENSION))
    {
        return Err("Выберите файл .mpsmart или ZIP-пакет Material Painter".into());
    }
    let mut archive = open(path)?;
    let (manifest, omitted) = mpsmart::decode(&mut archive)?;
    let thumbnail = thumbnail_from(&mut archive);
    let mut source = archive.into_inner();
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let path = publish(directory, &manifest, mpsmart::EXTENSION, |target| {
        let copied = std::io::copy(&mut source.take(MAX_FILE + 1), target)
            .map_err(|error| error.to_string())?;
        if copied > MAX_FILE {
            return Err("Файл пресета превышает допустимый размер".into());
        }
        Ok(())
    })?;
    let mut result = entry(path, &manifest);
    result.thumbnail = thumbnail;
    Ok((result, omitted))
}

fn import_material_package(directory: &Path, path: &Path) -> Result<(Entry, usize), String> {
    let mut package = package::Package::open(path, "smart", mpsmart::EXTENSION)?;
    let contents = package.read_main(MAX_FILE as usize)?;
    let sources = package.sources()?;
    let mut archive = Archive::new(Cursor::new(contents)).map_err(|error| error.to_string())?;
    if archive.len() > 67 {
        return Err("Слишком много файлов в пресете".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(&read(&mut archive, "smart.json", MAX_MANIFEST)?)
            .map_err(|error| format!("smart.json · {error}"))?;
    for record in value
        .get_mut("graphs")
        .and_then(|graphs| graphs.as_array_mut())
        .into_iter()
        .flatten()
    {
        let graph = record
            .get_mut("graph")
            .ok_or("В пресете нет данных графа")?;
        package::attach_sources(graph, &sources)?;
    }
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("Описание умного материала с WGSL превышает допустимый размер".into());
    }
    let (manifest, omitted) = mpsmart::decode_bytes(&mut archive, &bytes)?;
    let thumbnail = thumbnail_from(&mut archive);
    let path = publish(directory, &manifest, mpsmart::EXTENSION, |target| {
        let mut zip = zip::ZipWriter::new(target);
        package::add_entry(&mut zip, "smart.json", true, &bytes)?;
        let mut names = std::collections::HashSet::new();
        for index in 0..archive.len() {
            let file = archive
                .by_index_raw(index)
                .map_err(|error| error.to_string())?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().map_err(|error| error.to_string())?;
            if !package::relative(&name) || !names.insert(name.to_string()) {
                return Err("Недопустимый или повторяющийся путь в умном материале".into());
            }
            if name != "smart.json" {
                zip.raw_copy_file(file).map_err(|error| error.to_string())?;
            }
        }
        let target = zip.finish().map_err(|error| error.to_string())?;
        if target
            .stream_position()
            .map_err(|error| error.to_string())?
            > MAX_FILE
        {
            return Err("Файл пресета превышает допустимый размер".into());
        }
        Ok(())
    })?;
    let mut result = entry(path, &manifest);
    result.thumbnail = thumbnail;
    Ok((result, omitted))
}

fn valid(content: &Content) -> bool {
    match content {
        Content::Mask(mask) => mask.valid(),
        Content::Material(layers) => {
            if layers.is_empty() || layers.len() > MAX_LAYERS || layers.last().unwrap().depth != 0 {
                return false;
            }
            let mut folders = 0;
            let mut graph_ids = std::collections::HashSet::new();
            for layer in layers.iter().rev() {
                if layer.kind == LayerKind::Paint
                    || layer.depth > folders
                    || layer.depth > MAX_LAYER_DEPTH
                    || !(0.0..=100.0).contains(&layer.opacity)
                    || layer.mask.as_ref().is_some_and(|mask| !mask.valid())
                    || layer.graph_id.is_some_and(|id| id == 0 || !graph_ids.insert(id))
                    || layer.folder_graph.as_ref().is_some_and(|graph| {
                        layer.kind != LayerKind::Folder || layer.graph_id.is_none()
                            || !graph.payload.is_object() || graph.images.len() > 64
                    })
                    || layer.fill.as_ref().is_some_and(|fill| {
                        layer.kind != LayerKind::Fill
                            || !fill.settings.valid()
                            || fill.graph_images.len() > 64
                            || (fill.graph.is_none() && !fill.graph_images.is_empty())
                    })
                {
                    return false;
                }
                folders = layer.depth + usize::from(layer.kind == LayerKind::Folder);
            }
            true
        }
    }
}

pub(super) fn load_material(path: &Path, size: u32) -> Result<Vec<Layer>, String> {
    let mut archive = open(path)?;
    let Content::Material(rows) = manifest_at(path, &mut archive)?.content else {
        return Err("В файле сохранена маска".into());
    };
    build_layers(&rows, size, archive_images(&mut archive))
}

pub(super) fn load_mask(path: &Path, size: u32) -> Result<Arc<Mask>, String> {
    let mut archive = open(path)?;
    let Content::Mask(mask) = manifest_at(path, &mut archive)?.content else {
        return Err("В файле сохранён материал".into());
    };
    mask.load(solid(size, mask.pixels), &mut archive_images(&mut archive))
}

fn archive_images(
    archive: &mut Archive,
) -> impl FnMut(&ImageRef) -> Result<Arc<texture_fill::Bitmap>, String> + '_ {
    let mut images = HashMap::<String, Arc<texture_fill::Bitmap>>::new();
    let mut total_bytes = 0usize;
    let mut total_pixels = 0u64;
    move |source| {
        if let Some(bitmap) = images.get(&source.file) {
            return Ok(bitmap.clone());
        }
        let bytes = read(archive, &source.file, MAX_IMAGE)?;
        total_bytes += bytes.len();
        if total_bytes > 192 * 1024 * 1024 || images.len() >= 64 {
            return Err("Слишком много исходных изображений в пресете".into());
        }
        let reader = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|error| error.to_string())?;
        let (width, height) = reader
            .into_dimensions()
            .map_err(|error| error.to_string())?;
        if width > 8192 || height > 8192 {
            return Err("Изображение пресета превышает 8192 пикселей".into());
        }
        total_pixels += u64::from(width) * u64::from(height);
        if total_pixels > 128 * 1024 * 1024 {
            return Err("Пресет содержит слишком большие изображения".into());
        }
        let bitmap = Arc::new(texture_fill::Bitmap::decode(source.name.clone(), &bytes)?);
        images.insert(source.file.clone(), bitmap.clone());
        Ok(bitmap)
    }
}

fn build_layers(
    rows: &[LayerPreset],
    size: u32,
    mut bitmap: impl FnMut(&ImageRef) -> Result<Arc<texture_fill::Bitmap>, String>,
) -> Result<Vec<Layer>, String> {
    let mut planes = HashMap::new();
    rows.iter()
        .map(|row| {
            let mut layer = Layer::new(row.name.clone(), row.kind);
            layer.graph_id = row.graph_id;
            if let Some(graph) = &row.folder_graph {
                layer.folder_graph = Some(graph.payload.clone());
                let (mut bytes, mut texels) = (0, 0);
                for (path, source) in &graph.images {
                    let image = bitmap(source)?;
                    material_graph::file::check_size(&mut bytes, &mut texels, graph.images.len(), &image)?;
                    Arc::make_mut(&mut layer.folder_images).insert(path.clone(), image);
                }
            }
            layer.visible = row.visible;
            layer.collapsed = row.kind == LayerKind::Folder && row.collapsed;
            layer.depth = row.depth;
            layer.opacity = row.opacity;
            layer.blend = row.blend;
            layer.channels = row.channels.map(|plane| {
                plane.map(|rgba| {
                    planes
                        .entry(rgba)
                        .or_insert_with(|| solid(size, rgba))
                        .clone()
                })
            });
            layer.mask = row
                .mask
                .as_ref()
                .map(|mask| {
                    let pixels = planes
                        .entry(mask.pixels)
                        .or_insert_with(|| solid(size, mask.pixels))
                        .clone();
                    mask.load(pixels, &mut bitmap)
                })
                .transpose()?;
            if let Some(payload) = &row.fill {
                let mut fill = texture_fill::Fill::default();
                fill.settings = payload.settings;
                fill.graph = payload.graph.clone();
                for (target, source) in fill.images.iter_mut().zip(&payload.images) {
                    if let Some(source) = source {
                        *target = Some(bitmap(source)?);
                    }
                }
                for (path, source) in &payload.graph_images {
                    Arc::get_mut(&mut fill.graph_images)
                        .unwrap()
                        .insert(path.clone(), bitmap(source)?);
                }
                layer.texture_fill = Some(Arc::new(fill));
            }
            Ok(layer)
        })
        .collect()
}

struct Captured {
    manifest: Manifest,
    images: Vec<Arc<texture_fill::Bitmap>>,
    skipped: usize,
}

fn capture_image(
    images: &mut Vec<Arc<texture_fill::Bitmap>>,
    image: &Arc<texture_fill::Bitmap>,
) -> ImageRef {
    let index = images
        .iter()
        .position(|other| Arc::ptr_eq(other, image))
        .unwrap_or_else(|| {
            images.push(image.clone());
            images.len() - 1
        });
    ImageRef {
        file: format!("images/{index}"),
        name: image.name.clone(),
    }
}

fn capture(capture: Capture, include_folder_graphs: bool) -> Result<Captured, String> {
    let mut skipped = 0;
    let mut images = Vec::<Arc<texture_fill::Bitmap>>::new();
    let (name, content) = match capture {
        Capture::Mask { name, mask } => {
            let (mask, omitted, _) = MaskPreset::capture(&mask, &mut images);
            skipped += omitted;
            (name, Content::Mask(mask))
        }
        Capture::Material { name, layers } => {
            let depth = layers.last().ok_or("Выберите слой")?.depth;
            let mut rows = Vec::new();
            for layer in layers {
                if layer.kind == LayerKind::Paint {
                    skipped += 1;
                    continue;
                }
                let mask = layer.mask.as_ref().and_then(|mask| {
                    let (mask, omitted, painted) = MaskPreset::capture(mask, &mut images);
                    skipped += omitted;
                    (!painted
                        || mask.generator.is_some()
                        || mask.graph.is_some()
                        || !mask.layers.is_empty()
                        || !mask.filters.is_empty())
                    .then_some(mask)
                });
                let mut channels = [None; CHANNEL_COUNT];
                for (target, source) in channels.iter_mut().zip(&layer.channels) {
                    if let Some(source) = source {
                        *target =
                            Some(uniform(source).ok_or("Заливка содержит данные UV-развёртки")?);
                    }
                }
                let fill = layer.texture_fill.as_ref().map(|fill| FillPreset {
                    settings: fill.settings,
                    graph: fill.graph.clone(),
                    images: std::array::from_fn(|channel| {
                        fill.images[channel]
                            .as_ref()
                            .map(|image| capture_image(&mut images, image))
                    }),
                    graph_images: fill
                        .graph_images
                        .iter()
                        .map(|(key, image)| (key.clone(), capture_image(&mut images, image)))
                        .collect(),
                });
                rows.push(LayerPreset {
                    name: layer.name,
                    graph_id: layer.graph_id.filter(|_| include_folder_graphs),
                    folder_graph: layer.folder_graph.filter(|_| include_folder_graphs).map(|payload| {
                        FolderGraphPreset {
                            payload,
                            images: layer.folder_images.iter().map(|(key, image)| {
                                (key.clone(), capture_image(&mut images, image))
                            }).collect(),
                        }
                    }),
                    kind: layer.kind,
                    visible: layer.visible,
                    collapsed: layer.collapsed,
                    opacity: layer.opacity,
                    blend: layer.blend,
                    depth: layer.depth - depth,
                    channels,
                    mask,
                    fill,
                });
            }
            if !rows.iter().any(|row| row.kind == LayerKind::Fill) {
                return Err("Выберите заливку или группу с заливками".into());
            }
            (name, Content::Material(rows))
        }
    };
    let manifest = Manifest {
        version: 1,
        name,
        content,
    };
    if !valid(&manifest.content) {
        return Err("Некорректная структура пресета".into());
    }
    if images.len() > 64
        || images.iter().map(|image| image.bytes.len()).sum::<usize>() > 192 * 1024 * 1024
    {
        return Err("Слишком много исходных изображений в пресете".into());
    }
    Ok(Captured {
        manifest,
        images,
        skipped,
    })
}

fn thumbnail_from(archive: &mut Archive<impl Read + Seek>) -> Option<Arc<image::RgbaImage>> {
    let bytes = read(archive, "thumb.png", 512 * 1024).ok()?;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    Some(Arc::new(reader.decode().ok()?.into_rgba8()))
}

fn render_thumbnail(
    manifest: &Manifest,
    images: &[Arc<texture_fill::Bitmap>],
) -> Option<image::RgbaImage> {
    let mut bitmap = |source: &ImageRef| {
        source
            .file
            .strip_prefix("images/")
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| images.get(i))
            .cloned()
            .ok_or_else(|| "Отсутствует исходная текстура".into())
    };
    let size = shelf_preview::SIZE;
    match &manifest.content {
        Content::Material(rows) => {
            shelf_preview::render(build_layers(rows, size, bitmap).ok()?).ok()
        }
        Content::Mask(mask) if mask.has_graph() => {
            let mask = mask.load(solid(size, mask.pixels), &mut bitmap).ok()?;
            image::RgbaImage::from_raw(size, size, mask.effective().to_vec())
        }
        Content::Mask(_) => None,
    }
}

pub(super) fn save(directory: &Path, capture: Capture) -> Result<(Entry, usize), String> {
    let Captured {
        manifest,
        images,
        skipped,
    } = self::capture(capture, true)?;
    let bytes = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    let thumbnail = render_thumbnail(&manifest, &images);
    let png = thumbnail
        .as_ref()
        .map(texture_fill::encode_png)
        .transpose()?;
    let path = write_archive(
        directory,
        &manifest,
        EXTENSION,
        MANIFEST,
        &bytes,
        images.iter().map(|image| image.bytes.as_slice()),
        png.as_deref(),
    )?;
    let mut result = entry(path, &manifest);
    result.thumbnail = thumbnail.map(Arc::new);
    Ok((result, skipped))
}

pub(super) struct Export {
    pub path: PathBuf,
    pub omitted: usize,
    pub adjusted: usize,
}

pub(super) fn export_material(directory: &Path, capture: Capture) -> Result<Export, String> {
    export_material_as(directory, capture, ExportFormat::Material)
}

pub(super) fn export_material_package(
    directory: &Path,
    capture: Capture,
) -> Result<Export, String> {
    export_material_as(directory, capture, ExportFormat::Package)
}

pub(super) fn export_community_package(directory: &Path, capture: Capture) -> Result<Export, String> {
    export_material_as(directory, capture, ExportFormat::Community)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Material,
    Package,
    Community,
}

fn export_material_as(directory: &Path, capture: Capture, format: ExportFormat) -> Result<Export, String> {
    let package = format != ExportFormat::Material;
    let Captured {
        manifest,
        images,
        skipped,
    } = self::capture(capture, false)?;
    let thumbnail = render_thumbnail(&manifest, &images)
        .as_ref()
        .map(texture_fill::encode_png)
        .transpose()?;
    let images = mpsmart::prepare_images(&images)?;
    let (mut contents, adjusted) = mpsmart::encode(&manifest, &images)?;
    let mut sources = BTreeMap::new();
    if package {
        for record in contents["graphs"].as_array_mut().unwrap() {
            let graph = material_graph::read(&record["graph"])?;
            if !graph.nodes.is_empty() {
                graph.validate_texture_graph(&material_graph::library())?;
            }
            let mut payload = material_graph::encode(&graph);
            package::split_sources(&mut payload, &mut sources);
            record["graph"] = payload;
        }
        package::check_sources(&sources)?;
    }
    let bytes = serde_json::to_vec(&contents).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("Описание пресета Material Painter превышает допустимый размер".into());
    }
    let path = if package {
        let mut inner = Cursor::new(Vec::new());
        encode_archive(
            &mut inner,
            "smart.json",
            &bytes,
            images.iter().map(|image| image.bytes.as_ref()),
            thumbnail.as_deref(),
        )?;
        let inner = inner.into_inner();
        if inner.len() as u64 > MAX_FILE {
            return Err("Файл пресета превышает допустимый размер".into());
        }
        let id = if format == ExportFormat::Community {
            package::submission_id("smart", &manifest.name)
        } else {
            package::safe_name(&manifest.name)
        };
        let main = format!("{id}.mpsmart");
        let files: Vec<_> = std::iter::once(main.as_str())
            .chain(sources.keys().map(String::as_str))
            .collect();
        let mut info = serde_json::json!({
            "id": id, "name": manifest.name, "kind": "smart", "path": format!("items/{id}"),
            "files": files, "author": "", "description": "", "tags": [],
            "thumb": thumbnail.as_ref().map(|_| "thumb.png"),
        });
        if format == ExportFormat::Community {
            info["license"] = serde_json::json!("CC0");
        }
        publish(directory, &manifest, "zip", |file| {
            let mut zip = zip::ZipWriter::new(file);
            package::add_entry(
                &mut zip,
                &format!("{id}/info.json"),
                false,
                &serde_json::to_vec_pretty(&info).map_err(|error| error.to_string())?,
            )?;
            package::add_entry(&mut zip, &format!("{id}/{main}"), true, &inner)?;
            for (name, source) in &sources {
                package::add_entry(&mut zip, &format!("{id}/{name}"), false, source.as_bytes())?;
            }
            if let Some(png) = &thumbnail {
                package::add_entry(&mut zip, &format!("{id}/thumb.png"), true, png)?;
            }
            zip.finish().map_err(|error| error.to_string())?;
            Ok(())
        })?
    } else {
        write_archive(
            directory,
            &manifest,
            mpsmart::EXTENSION,
            "smart.json",
            &bytes,
            images.iter().map(|image| image.bytes.as_ref()),
            thumbnail.as_deref(),
        )?
    };
    Ok(Export {
        path,
        omitted: skipped,
        adjusted,
    })
}

pub(super) fn sphere(color: [u8; 3], roughness: f32, metal: f32) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(72, 72);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let nx = (x as f32 - 35.5) / 32.;
        let ny = (y as f32 - 35.5) / 32.;
        let r = nx * nx + ny * ny;
        if r > 1. {
            continue;
        }
        let z = (1. - r).sqrt();
        let light = (nx * -0.4 + ny * -0.55 + z * 0.73).max(0.);
        let gloss = light.powf(8. + (100. - roughness) * 0.6) * (0.4 + metal * 0.005);
        let shade = 0.18 + light * 0.78;
        *pixel = image::Rgba([
            (color[0] as f32 * shade + gloss * 100.).min(255.) as u8,
            (color[1] as f32 * shade + gloss * 100.).min(255.) as u8,
            (color[2] as f32 * shade + gloss * 100.).min(255.) as u8,
            255,
        ]);
    }
    image
}

fn write_archive<'a>(
    directory: &Path,
    manifest: &Manifest,
    extension: &str,
    manifest_file: &str,
    bytes: &[u8],
    images: impl IntoIterator<Item = &'a [u8]>,
    thumbnail: Option<&[u8]>,
) -> Result<PathBuf, String> {
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("Пресет слишком большой".into());
    }
    publish(directory, manifest, extension, |file| {
        encode_archive(file, manifest_file, bytes, images, thumbnail)
    })
}

fn encode_archive<'a>(
    file: impl Write + Seek,
    manifest_file: &str,
    bytes: &[u8],
    images: impl IntoIterator<Item = &'a [u8]>,
    thumbnail: Option<&[u8]>,
) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(manifest_file, options)
        .map_err(|error| error.to_string())?;
    zip.write_all(bytes).map_err(|error| error.to_string())?;
    for (index, image) in images.into_iter().enumerate() {
        zip.start_file(format!("images/{index}"), options)
            .map_err(|error| error.to_string())?;
        zip.write_all(image).map_err(|error| error.to_string())?;
    }
    if let Some(thumbnail) = thumbnail {
        zip.start_file("thumb.png", options)
            .map_err(|error| error.to_string())?;
        zip.write_all(thumbnail)
            .map_err(|error| error.to_string())?;
    }
    zip.finish().map_err(|error| error.to_string())?;
    Ok(())
}

fn publish(
    directory: &Path,
    manifest: &Manifest,
    extension: &str,
    write: impl FnOnce(&mut File) -> Result<(), String>,
) -> Result<PathBuf, String> {
    publish_named(directory, "Пресет", &manifest.name, extension, write)
}

pub(super) fn publish_named(
    directory: &Path,
    prefix: &str,
    name: &str,
    extension: &str,
    write: impl FnOnce(&mut File) -> Result<(), String>,
) -> Result<PathBuf, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let scratch = directory.join(format!(
        ".preset-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&scratch)
        .map_err(|error| error.to_string())?;
    let result = (|| -> Result<PathBuf, String> {
        write(&mut file)?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        let stem: String = name
            .chars()
            .take(60)
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        for n in 1..=10_000 {
            let suffix = if n == 1 {
                String::new()
            } else {
                format!(" ~{n}")
            };
            let path = directory.join(format!("{prefix} {}{suffix}.{extension}", stem.trim()));
            match fs::hard_link(&scratch, &path) {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Слишком много одноимённых пресетов".into())
    })();
    let _ = fs::remove_file(&scratch);
    result
}

#[derive(Default)]
pub(super) struct Library {
    pub root: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub revision: u64,
    initialized: bool,
    scan: Option<bevy::tasks::Task<Result<Vec<Entry>, String>>>,
    write: Option<bevy::tasks::Task<Result<Option<(Entry, usize)>, String>>>,
    export: Option<bevy::tasks::Task<Result<Option<Export>, String>>>,
}

impl Library {
    pub fn ensure(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        if self.root.is_none() {
            self.root = directory();
        }
        if let Some(root) = self.root.clone() {
            self.scan = Some(bevy::tasks::IoTaskPool::get().spawn(async move { list(&root) }));
        }
    }

    pub fn start_save(&mut self, capture: Capture) -> Result<(), String> {
        self.ensure();
        let root = self.root.clone().ok_or("Папка библиотеки недоступна")?;
        if self.write.is_some() || self.export.is_some() {
            return Err("Дождитесь завершения предыдущего сохранения".into());
        }
        self.write = Some(
            bevy::tasks::IoTaskPool::get().spawn(async move { save(&root, capture).map(Some) }),
        );
        Ok(())
    }

    pub fn start_import(
        &mut self,
        choose: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<(), String> {
        self.ensure();
        let root = self.root.clone().ok_or("Папка библиотеки недоступна")?;
        if self.write.is_some() || self.export.is_some() {
            return Err("Дождитесь завершения операции с библиотекой".into());
        }
        self.write = Some(bevy::tasks::IoTaskPool::get().spawn(async move {
            choose()?
                .map(|path| import_material(&root, &path))
                .transpose()
        }));
        Ok(())
    }

    pub fn start_export(
        &mut self,
        capture: Capture,
        package: bool,
        choose: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.write.is_some() || self.export.is_some() {
            return Err("Дождитесь завершения операции с библиотекой".into());
        }
        self.export = Some(bevy::tasks::IoTaskPool::get().spawn(async move {
            choose()?
                .map(|directory| {
                    if package {
                        export_material_package(&directory, capture)
                    } else {
                        export_material(&directory, capture)
                    }
                })
                .transpose()
        }));
        Ok(())
    }

    pub fn poll_export(&mut self) -> Option<Result<Export, String>> {
        use bevy::tasks::{block_on, poll_once};
        let result = self
            .export
            .as_mut()
            .and_then(|job| block_on(poll_once(job)))?;
        self.export = None;
        result.transpose()
    }

    pub fn poll(&mut self) -> Option<Result<(Entry, usize), String>> {
        use bevy::tasks::{block_on, poll_once};
        if let Some(result) = self.scan.as_mut().and_then(|job| block_on(poll_once(job))) {
            self.scan = None;
            match result {
                Ok(entries) => {
                    for entry in entries {
                        if !self.entries.iter().any(|known| known.path == entry.path) {
                            self.upsert(entry);
                        }
                    }
                }
                Err(error) => return Some(Err(error)),
            }
        }
        let result = self
            .write
            .as_mut()
            .and_then(|job| block_on(poll_once(job)))?;
        self.write = None;
        match result {
            Ok(Some((entry, omitted))) => {
                self.upsert(entry.clone());
                Some(Ok((entry, omitted)))
            }
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    pub(super) fn upsert(&mut self, entry: Entry) {
        // Keep indices stable for controls and drag payloads during asynchronous refresh.
        if let Some(old) = self.entries.iter_mut().find(|old| old.path == entry.path) {
            *old = entry;
        } else {
            self.entries.push(entry);
        }
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn paint_library_presets_reopen_at_a_different_resolution_without_uv_paint() {
        let directory =
            std::env::temp_dir().join(format!("bevy-paint-library-{}", std::process::id()));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[200, 50, 10, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let bitmap = Arc::new(texture_fill::Bitmap::decode("цвет.png".into(), &png).unwrap());
        let mut fill = texture_fill::Fill::default();
        fill.images[0] = Some(bitmap);
        fill.settings.projection = texture_fill::Projection::Triplanar;
        fill.settings.height_strength = -0.25;
        let mut mask = Mask::new(
            Arc::new([255, 0, 0, 255, 33, 98, 0, 255].repeat(8)),
            Vec::new(),
        );
        mask.affine = true;
        let mut generator = Generator::new(super::super::GeneratorKind::Clouds);
        generator.custom[0] = super::super::custom::Input { id: 88, weight: 75 };
        mask.generator = Some(generator);
        let mask = Arc::new(mask);
        let mut layer = Layer::new("Облицовка".into(), LayerKind::Fill);
        layer.depth = 2;
        layer.opacity = 85.;
        layer.channels[1] = Some(solid(4, [110, 110, 110, 255]));
        layer.texture_fill = Some(Arc::new(fill));
        layer.mask = Some(mask.clone());
        let mut nested = Layer::new("Внутренняя группа".into(), LayerKind::Folder);
        nested.depth = 1;
        let mut painted = Layer::new("Мазки".into(), LayerKind::Paint);
        painted.depth = 1;
        let root = Layer::new("Материал".into(), LayerKind::Folder);
        let (material, omitted) = save(
            &directory,
            Capture::Material {
                name: "Материал".into(),
                layers: vec![layer, nested, painted, root],
            },
        )
        .unwrap();
        assert_eq!(omitted, 3);
        let (saved_mask, omitted) = save(
            &directory,
            Capture::Mask {
                name: "Облака".into(),
                mask: mask.clone(),
            },
        )
        .unwrap();
        assert_eq!(omitted, 2);
        let (copy, _) = save(
            &directory,
            Capture::Mask {
                name: "Облака".into(),
                mask,
            },
        )
        .unwrap();
        assert_ne!(saved_mask.path, copy.path);
        assert_eq!(copy.name, "Облака 2");
        let entries = list(&directory).unwrap();
        assert_eq!(entries.len(), 3);
        let material = entries
            .iter()
            .find(|entry| entry.path == material.path)
            .unwrap();
        assert_eq!(material.thumbnail.as_ref().unwrap().width(), 144);
        let layers = load_material(&material.path, 16).unwrap();
        assert_eq!(
            layers.iter().map(|layer| layer.depth).collect::<Vec<_>>(),
            [2, 1, 0]
        );
        assert_eq!(layers[0].opacity, 85.);
        assert_eq!(layers[0].channels[1].as_ref().unwrap().len(), 16 * 16 * 4);
        let fill = layers[0].texture_fill.as_ref().unwrap();
        assert_eq!(fill.settings.height_strength, -0.25);
        assert_eq!(fill.images[0].as_ref().unwrap().bytes, png);
        let mask = layers[0].mask.as_ref().unwrap();
        assert_eq!(&mask.pixels[..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(mask.generator.unwrap().custom[0].id, 0);
        let mask = entries
            .iter()
            .find(|entry| entry.path == saved_mask.path)
            .unwrap();
        assert!(matches!(&mask.kind, Kind::Mask(mask) if mask.build(8).pixels.len() == 8 * 8 * 4));
        for entry in entries {
            fs::remove_file(entry.path).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }
}

#[cfg(test)]
pub(super) fn mpsmart_fixture(path: &Path, with_graph: bool) {
    use image::ImageEncoder;
    use serde_json::json;
    let graph = with_graph.then_some(33);
    let manifest = json!({
        "format": "material-painter-smart-material",
        "version": 1,
        "name": "MP Patinated brass",
        "layers_bottom_to_top": [
            {
                "id": 1, "parent": 2, "name": "Brass coating", "opacity": 0.75,
                "blend": "Linear Dodge (Add)",
                "content": {"kind":"fill", "fill":{
                    "enabled":[true,false,true,true,true,true,false,true],
                    "color":[0.6,0.4,0.2], "metallic":1., "height":0.2,
                    "opacity":0.5, "emission":[0.5,0.4,0.3], "specular":0.8,
                    "part_variation":0.25, "projection":"Triplanar",
                    "tiling":3., "rotation":25., "offset":[0.15,-0.15], "graph": graph
                }},
                "images":[{"file":"images/pattern.png","name":"pattern.png"},null,null,
                    {"file":"images/pattern.png","name":"pattern.png"},
                    {"file":"images/pattern.png","name":"pattern.png","source":"Green"},
                    {"file":"images/pattern.png","name":"pattern.png"},null,
                    {"file":"images/pattern.png","name":"pattern.png"}],
                "mask":{
                    "generator":{"kind":"Edge Wear","level":0.6,"contrast":0.15,"scale":3.,
                        "edge_width":0.034,"weights":[0.5,0.3],"custom":[{"id":7,"weight":1.}]},
                    "filters":[{"kind":"levels","a":0.1,"b":0.9,"c":0.8}],
                    "layers":[{"source":{"fill":0.4},"blend":"multiply","opacity":0.5,"enabled":true}]
                }
            },
            {"id":2,"parent":3,"name":"Weathering","content":{"kind":"folder"}},
            {"id":3,"parent":99,"name":"MP Patinated brass","content":{"kind":"folder"}}
        ],
        "graphs": [],
    });
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[180, 204, 12, 64], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    zip.start_file("smart.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    zip.start_file(
        "images/pattern.png",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&png).unwrap();
    zip.finish().unwrap();
}

#[cfg(test)]
pub(super) fn mpsmart_graph_fixture(path: &Path) {
    use super::material_graph;
    use material_painter_graph::{Graph, MASK_USAGE, Value};
    use serde_json::json;

    let library = material_graph::library();
    let library = library.as_ref();
    let mut fill = Graph::new("Imported Bitmap");
    fill.res = 64;
    let bitmap = fill.add("bitmap", [10., 20.]);
    fill.node_mut(bitmap).unwrap().params.insert(
        "image".into(),
        Value::Image(Some("images/green.png".into())),
    );
    let base = fill.add("output", [250., 20.]);
    let roughness = fill.add("output", [250., 200.]);
    fill.node_mut(roughness)
        .unwrap()
        .params
        .insert("usage".into(), Value::Enum(1));
    assert!(fill.connect(bitmap, base, "input"));
    assert!(fill.connect(bitmap, roughness, "input"));

    let mut mask = Graph::new("Imported Facing Up");
    mask.res = 64;
    let node = mask.add("smart_mask", [10., 20.]);
    let values = &mut mask.node_mut(node).unwrap().params;
    values.insert("input_1".into(), Value::Enum(7));
    values.insert("noise".into(), Value::Float(0.));
    let output = mask.add("output", [250., 20.]);
    mask.node_mut(output)
        .unwrap()
        .params
        .insert("usage".into(), Value::Enum(MASK_USAGE));
    assert!(mask.connect(node, output, "input"));

    let manifest = json!({
        "format": "material-painter-smart-material", "version":1, "name":"MP Graphs",
        "graphs":[{"id":19, "graph":fill.to_json(library)}, {"id":42, "graph":mask.to_json(library)}],
        "layers_bottom_to_top":[{
            "id": 4, "parent": null, "name": "Upward green",
            "content": {"kind":"fill", "fill": {
                "graph":19, "enabled":[true,false,false,false,false,false,false,false]
            }},
            "mask":{"graph":42,"layers":[],"filters":[]},
            "images":[],
        }]
    });
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]));
    let png = texture_fill::encode_png(&image).unwrap();
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    zip.start_file("smart.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    zip.start_file("images/green.png", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&png).unwrap();
    zip.finish().unwrap();
}

#[test]
fn mpsmart_round_trip_keeps_standard_graph_links_and_embedded_sources() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-mpsmart-graph-{}",
        bevy::asset::uuid::Uuid::new_v4(),
    ));
    fs::create_dir_all(&directory).unwrap();
    let original = directory.join("foreign.mpsmart");
    mpsmart_graph_fixture(&original);
    let layers = load_material(&original, 64).unwrap();
    let fill = layers[0].texture_fill.as_ref().unwrap();
    assert!(fill.graph.is_some());
    assert_eq!(
        fill.graph_images["images/green.png"]
            .pixels()
            .get_pixel(0, 0)
            .0,
        [0, 255, 0, 255]
    );
    let mask = layers[0].mask.as_ref().unwrap().graph.as_ref().unwrap();
    assert_eq!(mask.payload["name"], "Imported Facing Up");
    let report = export_material(
        &directory,
        Capture::Material {
            name: "Bevy Graphs".into(),
            layers: layers.clone(),
        },
    )
    .unwrap();
    assert_eq!(report.omitted, 0);
    assert_eq!(report.adjusted, 0);
    let mut archive = open(&report.path).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&read(&mut archive, "smart.json", MAX_MANIFEST).unwrap()).unwrap();
    let row = &manifest["layers_bottom_to_top"][0];
    let graphs = manifest["graphs"].as_array().unwrap();
    let find = |id: &serde_json::Value| graphs.iter().find(|graph| graph["id"] == *id).unwrap();
    assert_eq!(
        find(&row["content"]["fill"]["graph"])["graph"],
        *fill.graph.as_ref().unwrap().as_ref()
    );
    assert_eq!(find(&row["mask"]["graph"])["graph"], *mask.payload);
    let reread = load_material(&report.path, 64).unwrap();
    let restored = reread[0].texture_fill.as_ref().unwrap();
    assert_eq!(restored.graph_images, fill.graph_images);
    assert_eq!(
        restored.settings.graph_channels,
        fill.settings.graph_channels
    );
    assert_eq!(
        reread[0].mask.as_ref().unwrap().graph,
        layers[0].mask.as_ref().unwrap().graph
    );
    drop(archive);
    if let Some(directory) = std::env::var_os("BEVY_MPSMART_GRAPH_QA_DIR") {
        // Stock Material Painter can evaluate portable procedural graphs directly.
        let mut portable = layers;
        let fill = Arc::make_mut(portable[0].texture_fill.as_mut().unwrap());
        let mut graph = super::material_graph::read(fill.graph.as_ref().unwrap()).unwrap();
        let source = &mut graph.nodes[0];
        source.def = "color".into();
        source.params.clear();
        source.params.insert(
            "color".into(),
            material_painter_graph::Value::Color([0., 1., 0.]),
        );
        fill.graph = Some(Arc::new(graph.to_json(&super::material_graph::library())));
        fill.graph_images = Arc::default();
        let report = export_material(
            &PathBuf::from(directory),
            Capture::Material {
                name: "Bevy Procedural Graphs".into(),
                layers: portable,
            },
        )
        .unwrap();
        println!("Material Painter graph archive: {}", report.path.display());
    }
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(test)]
#[test]
fn smart_zip_round_trip_keeps_external_nodes_masks_and_bitmap() {
    use serde_json::{Value, json};
    use zip::ZipWriter;

    let directory = std::env::temp_dir().join(format!(
        "bevy-smart-zip-{}",
        bevy::asset::uuid::Uuid::new_v4(),
    ));
    fs::create_dir(&directory).unwrap();
    let native = directory.join("fixture.mpsmart");
    mpsmart_graph_fixture(&native);
    let mut source = open(&native).unwrap();
    let bitmap = read(&mut source, "images/green.png", MAX_IMAGE).unwrap();
    let mut manifest: Value =
        serde_json::from_slice(&read(&mut source, "smart.json", MAX_MANIFEST).unwrap()).unwrap();
    manifest["layers_bottom_to_top"][0]["opacity"] = json!(0.7);
    let definitions = [
        ("package-color", "color"), ("package-mask", "gray"),
    ].map(|(stem, kind)| (stem, format!(
        "// @name {stem}\n// @output {kind}\n// @input source: {kind} = 1\nfn node(uv: vec2<f32>) -> vec4<f32> {{ return in_source(uv); }}\n",
    )));
    for (record, (stem, _)) in manifest["graphs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(&definitions)
    {
        let graph = &mut record["graph"];
        let nodes = graph["nodes"].as_array_mut().unwrap();
        let output = nodes
            .iter_mut()
            .find(|node| node["def"] == "output")
            .unwrap();
        let input = output["links"]["input"].clone();
        output["links"]["input"] = json!(50);
        nodes.push(json!({
            "id": 50, "def": format!("user/{stem}"), "links": {"source": input},
        }));
    }
    let mut inner = ZipWriter::new(Cursor::new(Vec::new()));
    package::add_entry(
        &mut inner,
        "smart.json",
        true,
        &serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    package::add_entry(&mut inner, "images/green.png", true, &bitmap).unwrap();
    let inner = inner.finish().unwrap().into_inner();
    let file = directory.join("material-painter.zip");
    let mut archive = ZipWriter::new(File::create(&file).unwrap());
    let info = json!({"id": "smart", "kind": "smart", "files": [
        "smart.mpsmart", "nodes/package-color.wgsl", "nodes/package-mask.wgsl",
    ]});
    package::add_entry(
        &mut archive,
        "smart/info.json",
        false,
        &serde_json::to_vec(&info).unwrap(),
    )
    .unwrap();
    package::add_entry(&mut archive, "smart/smart.mpsmart", true, &inner).unwrap();
    for (stem, code) in &definitions {
        package::add_entry(
            &mut archive,
            &format!("smart/nodes/{stem}.wgsl"),
            false,
            code.as_bytes(),
        )
        .unwrap();
    }
    drop(archive.finish().unwrap());
    let (entry, skipped) = import_material(&directory.join("shelf"), &file).unwrap();
    assert_eq!(skipped, 0);
    assert!(is_mpsmart(&entry.path));
    fs::remove_file(&file).unwrap();
    let layers = load_material(&entry.path, 64).unwrap();
    let verify = |layers: &[Layer]| {
        let layer = &layers[0];
        assert_eq!(layer.opacity, 70.);
        let fill = layer.texture_fill.as_ref().unwrap();
        let mask = layer.mask.as_ref().unwrap().graph.as_ref().unwrap();
        assert_eq!(
            fill.graph_images
                .values()
                .next()
                .unwrap()
                .pixels()
                .get_pixel(0, 0)
                .0,
            [0, 255, 0, 255]
        );
        for (payload, (_, code)) in [fill.graph.as_ref().unwrap(), &mask.payload]
            .into_iter()
            .zip(&definitions)
        {
            let graph = material_graph::read(payload).unwrap();
            let node = graph.node(50).unwrap();
            assert!(graph.node(node.links["source"]).is_some());
            assert_eq!(
                &material_graph::library().find(&node.def).unwrap().source,
                code
            );
        }
    };
    verify(&layers);
    let report = export_material_package(
        &directory,
        Capture::Material {
            name: "Packaged graphs".into(),
            layers,
        },
    )
    .unwrap();
    let mut exported = package::Package::open(&report.path, "smart", "mpsmart").unwrap();
    assert_eq!(exported.sources().unwrap().len(), 2);
    let (reread, _) = import_material(&directory.join("reopened"), &report.path).unwrap();
    verify(&load_material(&reread.path, 64).unwrap());
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(test)]
#[test]
fn paint_library_imports_layered_mpsmart_and_survives_reloading() {
    let temp = std::env::temp_dir().join(format!("bevy-import-mpsmart-{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    let source = temp.join("MaterialPainter.mpsmart");
    let bad = temp.join("WithGraph.mpsmart");
    let shelf = temp.join("shelf");
    mpsmart_fixture(&source, false);
    mpsmart_fixture(&bad, true);
    let (entry, omitted) = import_material(&shelf, &source).unwrap();
    assert_eq!(omitted, 1);
    assert_eq!(fs::read(&entry.path).unwrap(), fs::read(&source).unwrap());
    assert!(matches!(
        &entry.kind,
        Kind::Material {
            layers: 3,
            depth: 2,
            ..
        }
    ));
    assert!(
        import_material(&shelf, &bad)
            .err()
            .unwrap()
            .contains("граф")
    );
    let catalog = list(&shelf).unwrap();
    assert_eq!(catalog.len(), 1);
    let reopened = load_material(&catalog[0].path, 16).unwrap();
    assert_eq!(
        reopened.iter().map(|layer| layer.depth).collect::<Vec<_>>(),
        [2, 1, 0]
    );
    let layer = &reopened[0];
    assert_eq!(layer.opacity, 75.);
    assert_eq!(layer.blend, Blend::AddHeight);
    assert!(layer.channels[1].is_none());
    let fill = layer.texture_fill.as_ref().unwrap();
    assert_eq!(
        fill.settings.projection,
        texture_fill::Projection::Triplanar
    );
    assert_eq!(fill.settings.height_strength, 0.4);
    assert_eq!(fill.settings.opacity_strength, 0.5);
    assert_eq!(fill.settings.emission_tint, [0.5, 0.4, 0.3]);
    assert_eq!(fill.settings.specular_strength, 1.6);
    assert_eq!(
        fill.settings.opacity_source,
        texture_fill::OpacitySource::Green
    );
    assert_eq!(fill.settings.variation_channels, [true, false, true]);
    assert!(!fill.settings.image_alpha_coverage);
    assert!(Arc::ptr_eq(
        fill.images[0].as_ref().unwrap(),
        fill.images[4].as_ref().unwrap()
    ));
    let mut sampled = (**fill).clone();
    sampled.settings.projection = texture_fill::Projection::Uv;
    sampled.bind(
        &crate::editor::model_mesh::Geometry::default().into(),
        0,
        4,
        bevy::prelude::Vec2::ZERO,
        bevy::prelude::Transform::IDENTITY,
        &layer.channels,
    );
    assert_eq!(
        &sampled.sampled[4].as_ref().unwrap()[..4],
        &[102, 102, 102, 255]
    );
    assert_eq!(
        &sampled.sampled[5].as_ref().unwrap()[..4],
        &[90, 82, 4, 255]
    );
    assert_eq!(
        &sampled.sampled[7].as_ref().unwrap()[..4],
        &[255, 255, 255, 255]
    );
    assert!(!sampled.sampled[1].is_some());
    let mask = layer.mask.as_ref().unwrap();
    assert_eq!(mask.filters[0].values, [10., 90., 80.]);
    assert_eq!(mask.layers[0].value, 40);
    assert_eq!(mask.layers[0].opacity, 50);
    assert_eq!(mask.generator.unwrap().edge_width, 34);
    assert_eq!(
        mask.generator.unwrap().weight(super::GeneratorInput::Edges),
        50
    );
    fs::remove_file(&entry.path).unwrap();
    fs::remove_dir(shelf).unwrap();
    fs::remove_file(source).unwrap();
    fs::remove_file(bad).unwrap();
    fs::remove_dir(temp).unwrap();
}

#[cfg(test)]
#[test]
fn paint_library_exports_mpsmart_with_embedded_images_and_round_trips() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-export-mpsmart-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.mpsmart");
    mpsmart_fixture(&source, false);
    let mut layers = load_material(&source, 8).unwrap();
    let layer = &mut layers[0];
    let source_texture = layer.texture_fill.as_ref().unwrap().images[0]
        .as_ref()
        .unwrap()
        .bytes
        .clone();
    let mask = Arc::make_mut(layer.mask.as_mut().unwrap());
    mask.pixels = solid(8, [173, 29, 0, 255]);
    let bottom = *mask.layers.first().unwrap();
    let filters = mask.filters.clone();
    mask.edit_layers().resize(MAX_MASK_LAYERS, bottom);
    let generator = mask.generator.unwrap();
    let settings = layer.texture_fill.as_ref().unwrap().settings;
    let capture = || Capture::Material {
        name: "Латунь".into(),
        layers: layers.clone(),
    };

    let report = export_material(&directory, capture()).unwrap();
    assert_eq!(report.path.extension().unwrap(), "mpsmart");
    assert_eq!((report.omitted, report.adjusted), (0, 0));
    let another = export_material(&directory, capture()).unwrap();
    assert_ne!(report.path, another.path);
    let mut archive = open(&report.path).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&read(&mut archive, "smart.json", MAX_MANIFEST).unwrap()).unwrap();
    assert_eq!(manifest["format"], "material-painter-smart-material");
    let rows = manifest["layers_bottom_to_top"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["parent"], rows[1]["id"]);
    assert_eq!(rows[1]["parent"], rows[2]["id"]);
    assert_eq!(rows[0]["images"][4]["source"], "Green");
    assert_eq!(rows[0]["blend"], "Linear Dodge (Add)");
    let file = rows[0]["images"][0]["file"].as_str().unwrap();
    assert_eq!(read(&mut archive, file, MAX_IMAGE).unwrap(), source_texture);
    let thumb = read(&mut archive, "thumb.png", MAX_IMAGE).unwrap();
    assert_eq!(image::load_from_memory(&thumb).unwrap().width(), 144);
    assert_eq!(rows[0]["mask"]["layers"][0]["blend"], "multiply");
    assert_eq!(rows[0]["mask"]["layers"][1]["blend"], "add");
    assert_eq!(
        rows[0]["mask"]["layers"].as_array().unwrap().len(),
        MAX_MASK_LAYERS + 2
    );
    drop(archive);

    let restored = load_material(&report.path, 16).unwrap();
    assert_eq!(
        restored.iter().map(|layer| layer.depth).collect::<Vec<_>>(),
        [2, 1, 0]
    );
    assert_eq!(restored[0].opacity, 75.);
    let fill = restored[0].texture_fill.as_ref().unwrap();
    assert_eq!(fill.settings, settings);
    assert_eq!(fill.images[0].as_ref().unwrap().bytes, source_texture);
    let mask = restored[0].mask.as_ref().unwrap();
    assert_eq!(&mask.pixels[..4], &[173, 29, 0, 255]);
    assert_eq!(mask.layers.as_slice(), &[bottom; MAX_MASK_LAYERS]);
    assert_eq!(mask.filters, filters);
    assert_eq!(mask.generator, Some(generator));
    if let Some(path) = std::env::var_os("BEVY_MPSMART_QA_ARCHIVE") {
        fs::copy(&report.path, path).unwrap();
    }
    layers[0].blend = Blend::Normal;
    let fill = Arc::make_mut(layers[0].texture_fill.as_mut().unwrap());
    fill.images[3] = fill.images[0].clone();
    fill.settings.height_strength = -3.;
    fill.settings.height_offset = 0.375;
    let report = export_material(&directory, Capture::Material {
        name: "Высота".into(),
        layers,
    }).unwrap();
    assert_eq!(report.adjusted, 1);
    let restored = load_material(&report.path, 16).unwrap();
    let settings = restored[0].texture_fill.as_ref().unwrap().settings;
    assert_eq!(settings.height_strength, -3.);
    assert_eq!(settings.height_offset, 0.375);
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(test)]
#[test]
fn paint_library_exports_webp_as_lossless_png_for_material_painter() {
    let pixels = image::RgbaImage::from_raw(2, 1, vec![200, 8, 61, 100, 0, 55, 74, 255]).unwrap();
    let mut webp = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp),
        pixels.as_raw(),
        2,
        1,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    let bitmap = Arc::new(texture_fill::Bitmap::decode("Decal.webp".into(), &webp).unwrap());
    let images = [bitmap];
    let compatible = mpsmart::prepare_images(&images).unwrap();
    assert_eq!(
        image::guess_format(&compatible[0].bytes).unwrap(),
        image::ImageFormat::Png
    );
    assert_eq!(
        image::load_from_memory(&compatible[0].bytes)
            .unwrap()
            .into_rgba8(),
        pixels
    );
}
