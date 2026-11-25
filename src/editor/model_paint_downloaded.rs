//! Local material receipts and offline loading for the paint shelf.
use super::{Maps, Material, Provider, valid_id};
use crate::editor::{
    hdri_catalog::atomic_write,
    image_previews::{DecodedImage, decode_image},
    model_paint_data::{
        Layer, LayerKind, solid,
        texture_fill::{Bitmap, Fill, Projection, encode_png},
    },
};
use bevy::{
    prelude::UVec2,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAP_NAMES: [&str; 8] = [
    "color", "rough", "metal", "height", "opacity", "emission", "normal", "specular",
];

#[derive(Serialize, Deserialize)]
struct Metadata {
    name: String,
    source: String,
    id: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolution: Option<u32>,
    maps: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in super::super) struct Record {
    pub provider: Provider,
    pub asset: Material,
    pub resolution: u32,
}

impl Record {
    pub fn new(provider: Provider, asset: &Material, resolution: u32) -> Self {
        Self {
            provider,
            asset: asset.clone(),
            resolution,
        }
    }

    pub fn key(&self) -> String {
        format!(
            "{}-{}-{}k",
            self.provider.key(),
            self.asset.id,
            self.resolution
        )
    }

    pub fn directory(&self, root: &Path) -> PathBuf {
        root.join(self.key())
    }

    pub fn source_url(&self) -> Result<reqwest::Url, String> {
        match self.provider {
            Provider::PolyHaven => {
                reqwest::Url::parse(&format!("https://polyhaven.com/a/{}", self.asset.id))
            }
            Provider::AmbientCg => {
                reqwest::Url::parse(&format!("https://ambientcg.com/view?id={}", self.asset.id))
            }
            Provider::Community => return Err("Недоступна страница материала".into()),
        }
        .map_err(|error| error.to_string())
    }

    pub fn write(&self, directory: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        atomic_write(&directory.join("info.json"), &bytes)
    }

    pub fn layer(&self, root: &Path, size: u32) -> Result<Vec<Layer>, String> {
        let maps = read_maps(&self.directory(root))?;
        let mut layer = Layer::new(self.asset.name.clone(), LayerKind::Fill);
        if maps[2].is_none() {
            layer.channels[2] = Some(solid(size, [0, 0, 0, 255]));
        }
        let mut fill = Fill::default();
        fill.settings.projection = Projection::Triplanar;
        fill.settings.height_strength = 0.6;
        fill.images = maps;
        layer.texture_fill = Some(Arc::new(fill));
        Ok(vec![layer])
    }

    pub fn preview(&self, root: &Path, edge: u32) -> Result<DecodedImage, String> {
        let directory = self.directory(root);
        let bounds = UVec2::splat(edge);
        let path = directory.join("thumb.png");
        if let Ok(image) = decode_image(&path, bounds) {
            return Ok(image);
        }
        let paths = map_paths(&directory).ok_or("В кэше нет полного набора карт материала")?;
        let image = decode_image(paths[0].as_deref().unwrap(), bounds)?;
        if let Some(preview) =
            image::RgbaImage::from_raw(image.size.x, image.size.y, image.rgba.clone())
            && let Ok(bytes) = encode_png(&preview)
        {
            let _ = atomic_write(&path, &bytes);
        }
        Ok(image)
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, cap: u64) -> Option<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > cap {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn metadata(directory: &Path) -> Option<Metadata> {
    read_json(&directory.join("meta.json"), 32 * 1024)
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

fn mapped_files(directory: &Path, names: [Option<String>; 8]) -> Option<[Option<PathBuf>; 8]> {
    let mut paths = <[Option<PathBuf>; 8]>::default();
    for (index, name) in names.into_iter().enumerate() {
        if let Some(name) = name {
            if name.is_empty()
                || name.contains(['/', '\\', ':'])
                || matches!(name.as_str(), "." | "..")
            {
                return None;
            }
            let path = directory.join(name);
            if !regular_file(&path) {
                return None;
            }
            paths[index] = Some(path);
        }
    }
    paths[0].is_some().then_some(paths)
}

fn portable_paths(directory: &Path, metadata: &Metadata) -> Option<[Option<PathBuf>; 8]> {
    mapped_files(
        directory,
        MAP_NAMES.map(|name| metadata.maps.get(name).cloned()),
    )
}

fn map_paths(directory: &Path) -> Option<[Option<PathBuf>; 8]> {
    if let Some(meta) = metadata(directory) {
        return portable_paths(directory, &meta);
    }
    let slots: [bool; 8] = read_json(&directory.join("maps.json"), 1024)?;
    mapped_files(
        directory,
        std::array::from_fn(|index| slots[index].then(|| format!("{index}.image"))),
    )
}

fn load_maps(paths: [Option<PathBuf>; 8]) -> Result<Maps, String> {
    let mut maps = Maps::default();
    for (index, path) in paths.into_iter().enumerate() {
        if let Some(path) = path {
            maps[index] = Some(Arc::new(Bitmap::open(&path)?));
        }
    }
    Ok(maps)
}

pub(super) fn read_maps(directory: &Path) -> Result<Maps, String> {
    load_maps(map_paths(directory).ok_or("В кэше нет полного набора карт материала")?)
}

pub(super) fn write_maps(
    directory: &Path,
    record: &Record,
    maps: &Maps,
    license: &str,
) -> Result<(), String> {
    let mut meta = Metadata {
        name: record.asset.name.clone(),
        source: record.provider.name().into(),
        id: record.asset.id.clone(),
        author: record.asset.author.clone(),
        url: record.source_url()?.to_string(),
        license: license.into(),
        resolution: Some(record.resolution),
        maps: BTreeMap::new(),
    };
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    if !fs::symlink_metadata(directory).is_ok_and(|meta| meta.is_dir()) {
        return Err("Кэш материала должен быть обычной папкой".into());
    }
    for (name, bitmap) in MAP_NAMES.into_iter().zip(maps) {
        if let Some(bitmap) = bitmap {
            let format = image::guess_format(&bitmap.bytes).map_err(|error| error.to_string())?;
            let extension = format
                .extensions_str()
                .first()
                .ok_or("Неизвестный формат карты")?;
            let target = format!("{name}.{extension}");
            atomic_write(&directory.join(&target), &bitmap.bytes)?;
            meta.maps.insert(name.to_owned(), target);
        }
    }
    let manifest = serde_json::to_vec_pretty(&meta).map_err(|error| error.to_string())?;
    atomic_write(&directory.join("meta.json"), &manifest)
}

fn import_material(root: &Path, directory: &Path) -> Result<(), String> {
    let meta = metadata(directory).ok_or("В выбранной папке нет читаемого meta.json")?;
    let provider = match meta.source.as_str() {
        "Poly Haven" => Provider::PolyHaven,
        "ambientCG" => Provider::AmbientCg,
        _ => return Err("Источник материала должен быть Poly Haven или ambientCG".into()),
    };
    if !valid_id(&meta.id) {
        return Err("Некорректный идентификатор материала".into());
    }
    let maps = load_maps(
        portable_paths(directory, &meta)
            .ok_or("Карты отсутствуют или находятся вне папки материала")?,
    )?;
    let color = maps[0].as_ref().unwrap().pixels();
    let largest = color.width().max(color.height());
    let resolution = meta
        .resolution
        .filter(|size| matches!(size, 1 | 2 | 4))
        .unwrap_or_else(|| {
            if largest <= 1024 {
                1
            } else if largest <= 2048 {
                2
            } else {
                4
            }
        });
    let mut asset = Material::new(&meta.id, meta.name, vec![]);
    asset.author = meta.author;
    let record = Record::new(provider, &asset, resolution);
    let target = record.directory(root);
    if map_paths(&target).is_some() {
        return Err("Это разрешение материала уже есть в «Скачанных»".into());
    }
    write_maps(&target, &record, &maps, &meta.license)?;
    record.write(&target)?;
    let thumb = directory.join("thumb.png");
    if regular_file(&thumb)
        && let Ok(image) = decode_image(&thumb, UVec2::splat(128))
        && let Some(pixels) = image::RgbaImage::from_raw(image.size.x, image.size.y, image.rgba)
        && let Ok(bytes) = encode_png(&pixels)
    {
        let _ = atomic_write(&target.join("thumb.png"), &bytes);
    }
    Ok(())
}

fn parse_key(key: &str) -> Option<(Provider, &str, u32)> {
    let (provider, rest) = [Provider::PolyHaven, Provider::AmbientCg]
        .into_iter()
        .find_map(|provider| {
            key.strip_prefix(provider.key())?
                .strip_prefix('-')
                .map(|rest| (provider, rest))
        })?;
    let (id, size) = rest.rsplit_once('-')?;
    if !valid_id(id) {
        return None;
    }
    let resolution = match size {
        "1k" => 1,
        "2k" => 2,
        "4k" => 4,
        _ => return None,
    };
    Some((provider, id, resolution))
}

fn scan(root: &Path) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    let mut legacy_catalogs = HashMap::new();
    let directories = match fs::read_dir(root) {
        Ok(directories) => directories,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
        Err(error) => return Err(error.to_string()),
    };
    for dir in directories.flatten() {
        let key = dir.file_name();
        let Some(key) = key.to_str() else { continue };
        let Some((provider, id, resolution)) = parse_key(key) else {
            continue;
        };
        let path = dir.path();
        if !dir.file_type().is_ok_and(|kind| kind.is_dir()) || map_paths(&path).is_none() {
            continue;
        }
        let record = read_json::<Record>(&path.join("info.json"), 32 * 1024)
            .filter(|record| {
                record.provider == provider
                    && record.resolution == resolution
                    && record.asset.id == id
            })
            .unwrap_or_else(|| {
                let catalog = legacy_catalogs.entry(provider.key()).or_insert_with(|| {
                    read_json::<Vec<Material>>(
                        &root.join(format!("{}.json", provider.key())),
                        16 * 1024 * 1024,
                    )
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entry| (entry.id.clone(), entry))
                    .collect::<HashMap<_, _>>()
                });
                let asset = catalog.get(id).cloned().unwrap_or_else(|| {
                    let meta = metadata(&path).filter(|meta| meta.id == id);
                    let name = meta
                        .as_ref()
                        .map_or_else(|| id.replace(['_', '-'], " "), |meta| meta.name.clone());
                    let mut asset = Material::new(id, name, Vec::new());
                    if let Some(meta) = meta {
                        asset.author = meta.author;
                    }
                    asset
                });
                let record = Record::new(provider, &asset, resolution);
                let _ = record.write(&path);
                record
            });
        records.push(record);
    }
    records.sort_by(|a, b| {
        a.asset
            .name
            .to_lowercase()
            .cmp(&b.asset.name.to_lowercase())
            .then(a.key().cmp(&b.key()))
    });
    Ok(records)
}

#[derive(Default)]
pub(in super::super) struct Library {
    pub root: Option<PathBuf>,
    records: Vec<Option<Record>>,
    pub menu: Option<usize>,
    pub revision: u64,
    pub page: usize,
    pub error: Option<String>,
    pub scan: Option<Task<Result<Vec<Record>, String>>>,
    job: Option<(Operation, Task<Result<bool, String>>)>,
    initialized: bool,
    rescan: bool,
}

#[derive(Clone, Copy)]
pub(in super::super) enum Operation {
    Download,
    Delete,
    Import,
}

impl Library {
    pub fn records(&self) -> impl Iterator<Item = (usize, &Record)> {
        self.records
            .iter()
            .enumerate()
            .filter_map(|(id, record)| record.as_ref().map(|record| (id, record)))
    }

    pub fn record(&self, id: usize) -> Option<&Record> {
        self.records.get(id)?.as_ref()
    }

    fn replace_records(&mut self, records: Vec<Record>) {
        // Slots stay stable while a card or drag payload refers to them.
        let mut current: HashMap<_, _> = self
            .records()
            .map(|(id, record)| (record.key(), id))
            .collect();
        for record in records {
            if let Some(id) = current.remove(&record.key()) {
                self.records[id] = Some(record);
            } else {
                self.records.push(Some(record));
            }
        }
        for id in current.into_values() {
            self.records[id] = None;
            if self.menu == Some(id) {
                self.menu = None;
            }
        }
    }

    pub fn ensure(&mut self, root: Option<PathBuf>) {
        if self.initialized {
            return;
        }
        self.root = root;
        self.initialized = true;
        self.refresh();
    }

    pub fn refresh(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.scan.is_some() {
            self.rescan = true;
            return;
        }
        self.scan = Some(IoTaskPool::get().spawn(async move { scan(&root) }));
        self.revision += 1;
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn operation(&self) -> Option<Operation> {
        self.job.as_ref().map(|(operation, _)| *operation)
    }

    pub fn start(
        &mut self,
        provider: Provider,
        asset: Material,
        resolution: u32,
    ) -> Result<(), String> {
        let root = self
            .root
            .clone()
            .ok_or("Локальный кэш материалов недоступен")?;
        if self.busy() {
            return Err("Пожалуйста, дождитесь загрузки материала".into());
        }
        let task = IoTaskPool::get().spawn(async move {
            super::texture_maps(Some(root), provider, &asset, resolution).map(|_| true)
        });
        self.job = Some((Operation::Download, task));
        self.revision += 1;
        Ok(())
    }

    pub fn start_import(
        &mut self,
        choose: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static,
    ) -> Result<(), String> {
        let root = self
            .root
            .clone()
            .ok_or("Папка скачанных материалов недоступна")?;
        if self.busy() {
            return Err("Дождитесь завершения операции с материалом".into());
        }
        let task = IoTaskPool::get().spawn(async move {
            let Some(directory) = choose()? else {
                return Ok(false);
            };
            import_material(&root, &directory).map(|()| true)
        });
        self.job = Some((Operation::Import, task));
        self.revision += 1;
        Ok(())
    }

    pub fn delete(&mut self, id: usize) -> Result<(), String> {
        if self.busy() {
            return Err("Дождитесь завершения операции с материалом".into());
        }
        let root = self
            .root
            .as_deref()
            .ok_or("Папка скачанных материалов недоступна")?;
        let directory = self
            .record(id)
            .ok_or("Скачанный материал не найден")?
            .directory(root);
        let task = IoTaskPool::get().spawn(async move {
            let metadata = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("Папка материала недоступна".into());
            }
            fs::remove_dir_all(directory)
                .map(|()| true)
                .map_err(|error| error.to_string())
        });
        self.job = Some((Operation::Delete, task));
        self.menu = None;
        self.revision += 1;
        Ok(())
    }

    pub fn update(&mut self) -> Option<(Operation, Result<(), String>)> {
        if let Some(result) = self.scan.as_mut().and_then(|job| block_on(poll_once(job))) {
            self.scan = None;
            match result {
                Ok(records) => {
                    self.error = None;
                    self.replace_records(records);
                }
                Err(error) => self.error = Some(error),
            }
            self.revision += 1;
            if std::mem::take(&mut self.rescan) {
                self.refresh();
            }
        }
        let (operation, task) = self.job.as_mut()?;
        let operation = *operation;
        let result = block_on(poll_once(task))?;
        self.job = None;
        self.revision += 1;
        if result == Ok(true) {
            self.refresh();
        }
        match result {
            Ok(false) => None,
            Ok(true) => Some((operation, Ok(()))),
            Err(error) => Some((operation, Err(error))),
        }
    }
}

#[cfg(test)]
pub(in super::super) fn fixture(root: &Path, provider: Provider, id: &str, name: &str) -> Record {
    let asset = Material::new(id, name.into(), vec!["terrain".into()]);
    let record = Record::new(provider, &asset, 1);
    let mut maps: Maps = Default::default();
    for (slot, color) in [(0, [132, 68, 44, 255]), (3, [180, 180, 180, 255])] {
        let pixels = image::RgbaImage::from_pixel(160, 128, image::Rgba(color));
        maps[slot] = Some(Arc::new(Bitmap::from_rgba(name.into(), pixels).unwrap()));
    }
    write_maps(&record.directory(root), &record, &maps, "CC0").unwrap();
    record.write(&record.directory(root)).unwrap();
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloaded_material_migrates_legacy_cache_and_loads_without_network() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let original = fixture(root, Provider::PolyHaven, "red-rock", "Red Rock");
        let folder = original.directory(root);
        fs::remove_file(folder.join("info.json")).unwrap();
        fs::remove_file(folder.join("meta.json")).unwrap();
        fs::rename(folder.join("color.png"), folder.join("0.image")).unwrap();
        fs::rename(folder.join("height.png"), folder.join("3.image")).unwrap();
        fs::write(
            folder.join("maps.json"),
            b"[true,false,false,true,false,false,false,false]",
        )
        .unwrap();
        fs::write(
            root.join("polyhaven.json"),
            serde_json::to_vec(&[original.asset]).unwrap(),
        )
        .unwrap();
        let incomplete = fixture(root, Provider::AmbientCg, "broken", "Broken");
        fs::remove_file(incomplete.directory(root).join("height.png")).unwrap();

        let records = scan(root).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert!(record.asset.matches("rock terrain"));
        assert!(folder.join("info.json").is_file());

        let preview = record.preview(root, 128).unwrap();
        assert_eq!(preview.size, UVec2::new(128, 102));
        assert_eq!(&preview.rgba[..4], &[132, 68, 44, 255]);
        assert!(folder.join("thumb.png").is_file());

        let layers = record.layer(root, 64).unwrap();
        let fill = layers[0].texture_fill.as_ref().unwrap();
        assert_eq!(fill.settings.projection, Projection::Triplanar);
        assert_eq!(fill.settings.height_strength, 0.6);
        assert!(fill.images[0].is_some() && fill.images[3].is_some());
        assert_eq!(&layers[0].channel(2).unwrap()[..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn downloaded_material_keeps_card_ids_valid_when_records_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        let a = fixture(temp.path(), Provider::PolyHaven, "a", "A");
        let b = fixture(temp.path(), Provider::AmbientCg, "b", "B");
        let mut library = Library::default();
        library.replace_records(vec![a.clone(), b.clone()]);
        library.menu = Some(0);
        library.replace_records(vec![b]);
        assert!(library.record(0).is_none());
        assert!(library.menu.is_none());
        library.replace_records(vec![library.record(1).unwrap().clone(), a]);
        assert!(library.record(0).is_none());
        assert_eq!(library.record(1).unwrap().asset.name, "B");
        assert_eq!(library.record(2).unwrap().asset.name, "A");
    }

    #[test]
    fn downloaded_material_imports_material_painter_folder_with_original_image_bytes() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let color = image::RgbImage::from_pixel(1024, 64, image::Rgb([138, 205, 178]));
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpg)
            .encode_image(&color)
            .unwrap();
        fs::write(source.path().join("base.jpg"), &jpg).unwrap();
        let normal = image::RgbaImage::from_pixel(8, 8, image::Rgba([128, 128, 255, 255]));
        let png = encode_png(&normal).unwrap();
        fs::write(source.path().join("normal-map.png"), &png).unwrap();
        fs::write(source.path().join("thumb.png"), &png).unwrap();
        let mut manifest = serde_json::json!({
            "name": "Limestone",
            "source": "Poly Haven",
            "id": "limestone",
            "author": "Test Author",
            "url": "https://polyhaven.com/a/limestone",
            "license": "CC0",
            "maps": { "color": "base.jpg", "normal": "normal-map.png" }
        });
        let manifest_path = source.path().join("meta.json");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        import_material(output.path(), source.path()).unwrap();
        let records = scan(output.path()).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.asset.author, "Test Author");
        let directory = record.directory(output.path());
        let meta = metadata(&directory).unwrap();
        assert_eq!(meta.source, "Poly Haven");
        assert_eq!(meta.license, "CC0");
        assert_eq!(meta.url, "https://polyhaven.com/a/limestone");
        assert_eq!(meta.resolution, Some(1));
        for (channel, reference, format) in [
            ("color", &jpg, image::ImageFormat::Jpeg),
            ("normal", &png, image::ImageFormat::Png),
        ] {
            let map = directory.join(&meta.maps[channel]);
            assert!(
                format
                    .extensions_str()
                    .contains(&map.extension().unwrap().to_str().unwrap())
            );
            assert_eq!(&fs::read(map).unwrap(), reference);
        }
        assert_eq!(
            &read_maps(&directory).unwrap()[6].as_ref().unwrap().bytes,
            &png
        );
        assert!(directory.join("thumb.png").is_file());
        assert!(import_material(output.path(), source.path()).is_err());
        manifest["maps"]["color"] = serde_json::json!("../other.jpg");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            import_material(output.path(), source.path())
                .unwrap_err()
                .contains("вне папки")
        );
    }
}
