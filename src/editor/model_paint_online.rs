//! Public PBR material catalogs and downloaded texture sets for the paint shelf.
use super::{
    Layer, LayerKind, presets,
    texture_fill::{Bitmap, Fill},
};
use crate::editor::hdri_catalog::{atomic_write, clean_name, get, json, strings};
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[path = "model_paint_online_previews.rs"]
mod previews;
pub(super) use previews::Previews;
#[path = "model_paint_community.rs"]
pub(super) mod community;

const IMAGE_LIMIT: u64 = 64 * 1024 * 1024;
const ARCHIVE_LIMIT: u64 = 192 * 1024 * 1024;
pub(super) const PAGE: usize = 24;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum Provider {
    #[default]
    PolyHaven,
    AmbientCg,
    Community,
}

impl Provider {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::PolyHaven => "Poly Haven",
            Self::AmbientCg => "ambientCG",
            Self::Community => "Сообщество",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::PolyHaven => "polyhaven",
            Self::AmbientCg => "ambientcg",
            Self::Community => "community",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Material {
    pub id: String,
    pub name: String,
    search: String,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    pub community: Option<community::Package>,
}

impl Material {
    fn new(id: &str, name: String, tags: Vec<String>) -> Self {
        let search = format!("{name} {id} {}", tags.join(" ")).to_lowercase();
        Self {
            id: id.into(),
            name,
            search,
            thumbnail: None,
            community: None,
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        query
            .split_whitespace()
            .all(|word| self.search.contains(word))
    }
}

#[derive(Default)]
pub(super) struct Shelf {
    pub provider: Provider,
    pub page: usize,
    pub resolution: u32,
    pub revision: u64,
    pub entries: Vec<Material>,
    pub error: Option<String>,
    pub job: Option<Task<Result<Vec<Material>, String>>>,
    loaded: bool,
}

impl Shelf {
    pub fn show(&mut self, provider: Provider, refresh: bool) {
        if self.job.is_some() || (provider == self.provider && !refresh && self.loaded) {
            return;
        }
        if provider != self.provider {
            self.entries.clear();
        }
        self.provider = provider;
        self.page = 0;
        self.loaded = false;
        self.error = None;
        self.revision += 1;
        self.job = Some(IoTaskPool::get().spawn(async move { catalog(provider, refresh) }));
    }

    pub fn update(&mut self) {
        let Some(result) = self.job.as_mut().and_then(|job| block_on(poll_once(job))) else {
            return;
        };
        self.job = None;
        self.loaded = true;
        match result {
            Ok(entries) => self.entries = entries,
            Err(error) => self.error = Some(error),
        }
        self.revision += 1;
    }

    pub fn resolution(&self) -> u32 {
        self.resolution.max(1)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

fn cache() -> Option<PathBuf> {
    presets::directory().map(|root| root.join("online"))
}

fn catalog(provider: Provider, refresh: bool) -> Result<Vec<Material>, String> {
    let path = cache().map(|cache| cache.join(format!("{}.json", provider.key())));
    let cached = || {
        let bytes = fs::read(path.as_ref()?).ok()?;
        serde_json::from_slice::<Vec<Material>>(&bytes).ok()
    };
    if !refresh
        && path
            .as_ref()
            .and_then(|path| path.metadata().ok())
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age < Duration::from_secs(24 * 60 * 60))
        && let Some(entries) = cached()
        && (provider == Provider::Community
            || entries.iter().any(|entry| entry.thumbnail.is_some()))
    {
        return Ok(entries);
    }
    let entries = match fetch_catalog(provider) {
        Ok(entries) => entries,
        Err(error) if !refresh => return cached().ok_or(error),
        Err(error) => return Err(error),
    };
    if let Some(path) = path
        && let Ok(bytes) = serde_json::to_vec(&entries)
        && fs::create_dir_all(path.parent().unwrap()).is_ok()
    {
        let _ = atomic_write(&path, &bytes);
    }
    Ok(entries)
}

fn poly_catalog(value: &Value) -> Result<Vec<Material>, String> {
    let assets = value.as_object().ok_or("Неверный каталог Poly Haven")?;
    Ok(assets
        .iter()
        .filter(|(id, _)| valid_id(id))
        .map(|(id, data)| {
            let mut tags = strings(data.get("tags"));
            tags.extend(strings(data.get("categories")));
            let mut entry = Material::new(id, clean_name(data.get("name"), id), tags);
            entry.thumbnail = data["thumbnail_url"].as_str().filter(|url| !url.is_empty()).map(str::to_owned)
                .or_else(|| Some(format!("https://cdn.polyhaven.com/asset_img/thumbs/{id}.png?width=256&height=256&format=webp")));
            entry
        })
        .collect())
}

fn ambient_catalog(value: &Value) -> Result<(usize, Vec<Material>), String> {
    let assets = value["assets"]
        .as_array()
        .ok_or("Неверный каталог ambientCG")?;
    Ok((
        assets.len(),
        assets
            .iter()
            .filter_map(|data| {
                let id = data["id"].as_str().filter(|id| valid_id(id))?;
                let mut entry = Material::new(
                    id,
                    clean_name(data.get("title"), id),
                    strings(data.get("tags")),
                );
                entry.thumbnail = ["256-WEBP", "256-PNG", "256-JPG-FFFFFF"]
                    .into_iter()
                    .find_map(|format| data["thumbnails"][format].as_str())
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned);
                Some(entry)
            })
            .collect(),
    ))
}

fn fetch_catalog(provider: Provider) -> Result<Vec<Material>, String> {
    let mut entries = match provider {
        Provider::PolyHaven => poly_catalog(&json("https://api.polyhaven.com/assets?t=textures")?)?,
        Provider::AmbientCg => {
            let mut entries = Vec::new();
            for offset in (0..10_000).step_by(500) {
                let data = json(&format!(
                    "https://ambientcg.com/api/v3/assets?type=material&sort=alphabet&limit=500&offset={offset}&include=title,tags,thumbnails"
                ))?;
                let (count, page) = ambient_catalog(&data)?;
                entries.extend(page);
                if count < 500 {
                    break;
                }
            }
            entries
        }
        Provider::Community => {
            community::catalog(&json(&format!("{}index.json", community::BASE))?)?
        }
    };
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    entries.dedup_by(|a, b| a.id == b.id);
    Ok(entries)
}

type Maps = [Option<Arc<Bitmap>>; 8];

pub(super) fn material(
    provider: Provider,
    asset: &Material,
    resolution: u32,
) -> Result<Vec<Layer>, String> {
    material_in(cache(), provider, asset, resolution)
}

fn material_in(
    cache: Option<PathBuf>,
    provider: Provider,
    asset: &Material,
    resolution: u32,
) -> Result<Vec<Layer>, String> {
    if provider == Provider::Community || !valid_id(&asset.id) || !matches!(resolution, 1 | 2 | 4) {
        return Err("Некорректный материал каталога".into());
    }
    let directory =
        cache.map(|root| root.join(format!("{}-{}-{resolution}k", provider.key(), asset.id)));
    let maps = if let Some(maps) = directory.as_deref().and_then(|path| read_maps(path).ok()) {
        maps
    } else {
        let maps = match provider {
            Provider::PolyHaven => poly_maps(asset, resolution)?,
            Provider::AmbientCg => ambient_maps(asset, resolution)?,
            Provider::Community => return Err("Выберите пакет сообщества".into()),
        };
        if maps[0].is_none() {
            return Err("У материала отсутствует карта цвета".into());
        }
        if let Some(directory) = &directory {
            write_maps(directory, &maps)?;
        }
        maps
    };
    let mut layer = Layer::new(asset.name.clone(), LayerKind::Fill);
    let mut fill = Fill::default();
    fill.images = maps;
    layer.texture_fill = Some(Arc::new(fill));
    Ok(vec![layer])
}

fn read_maps(directory: &Path) -> Result<Maps, String> {
    let mut bytes = Vec::new();
    fs::File::open(directory.join("maps.json"))
        .and_then(|file| file.take(1025).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    let slots: [bool; 8] = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if bytes.len() > 1024 || !slots[0] {
        return Err("В кэше нет описания карт материала".into());
    }
    let mut maps: Maps = Default::default();
    for (index, used) in slots.into_iter().enumerate() {
        if used {
            maps[index] = Some(Arc::new(Bitmap::open(
                &directory.join(format!("{index}.image")),
            )?));
        }
    }
    Ok(maps)
}

fn write_maps(directory: &Path, maps: &Maps) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| format!("Кэш материалов · {error}"))?;
    for (index, image) in maps.iter().enumerate() {
        if let Some(image) = image {
            atomic_write(&directory.join(format!("{index}.image")), &image.bytes)?;
        }
    }
    let slots = maps.each_ref().map(Option::is_some);
    let manifest = serde_json::to_vec(&slots).map_err(|error| error.to_string())?;
    atomic_write(&directory.join("maps.json"), &manifest)
}

fn poly_maps(asset: &Material, resolution: u32) -> Result<Maps, String> {
    let data = json(&format!("https://api.polyhaven.com/files/{}", asset.id))?;
    let resolution = format!("{resolution}k");
    let source = |map: &str| {
        let formats = if matches!(map, "Displacement" | "arm") {
            ["png", "jpg"]
        } else {
            ["jpg", "png"]
        };
        formats
            .into_iter()
            .find_map(|extension| data[map][&resolution][extension]["url"].as_str())
    };
    let mut maps: Maps = Default::default();
    for (index, map) in [
        (0, "Diffuse"),
        (1, "Rough"),
        (2, "Metal"),
        (3, "Displacement"),
        (6, "nor_gl"),
    ] {
        if let Some(url) = source(map) {
            let bytes = get(url, IMAGE_LIMIT, None)?;
            maps[index] = Some(Arc::new(Bitmap::decode(
                format!("{} {map}", asset.name),
                &bytes,
            )?));
        }
    }
    if maps[2].is_none()
        && let Some(url) = source("arm")
    {
        let bytes = get(url, IMAGE_LIMIT, None)?;
        let packed = Bitmap::decode("ARM".into(), &bytes)?;
        let source = packed.pixels();
        if source.pixels().any(|pixel| pixel[2] > 12) {
            let metal = image::RgbaImage::from_fn(source.width(), source.height(), |x, y| {
                let v = source.get_pixel(x, y)[2];
                image::Rgba([v, v, v, 255])
            });
            maps[2] = Some(Arc::new(Bitmap::from_rgba(
                format!("{} Metal", asset.name),
                metal,
            )?));
        }
    }
    Ok(maps)
}

fn ambient_maps(asset: &Material, resolution: u32) -> Result<Maps, String> {
    let data = json(&format!(
        "https://ambientcg.com/api/v3/assets?type=material&id={}&limit=1&include=downloads",
        asset.id
    ))?;
    let options = data["assets"][0]["downloads"]
        .as_array()
        .ok_or("В каталоге нет файлов материала")?;
    let format = format!("{resolution}K-JPG");
    let url = options
        .iter()
        .find(|entry| entry["attributes"].as_str() == Some(&format) && entry["extension"] == "zip")
        .and_then(|entry| entry["url"].as_str())
        .ok_or("Материал недоступен в этом разрешении")?;
    decode_ambient_zip(get(url, ARCHIVE_LIMIT, None)?)
}

fn decode_ambient_zip(bytes: Vec<u8>) -> Result<Maps, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let mut maps: Maps = Default::default();
    let mut remaining = ARCHIVE_LIMIT;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|error| error.to_string())?;
        let Some(name) = file
            .enclosed_name()
            .and_then(|path| path.file_name()?.to_str().map(str::to_owned))
        else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let Some((stem, ext)) = lower.rsplit_once('.') else {
            continue;
        };
        if !matches!(ext, "jpg" | "jpeg" | "png") {
            continue;
        }
        let index = match stem.rsplit('_').next() {
            Some("color") => 0,
            Some("roughness") => 1,
            Some("metalness") => 2,
            Some("displacement") => 3,
            Some("opacity") => 4,
            Some("emission") => 5,
            Some("normalgl") => 6,
            _ => continue,
        };
        if !file.is_file() || maps[index].is_some() {
            continue;
        }
        let limit = IMAGE_LIMIT.min(remaining);
        if file.size() > limit {
            return Err("Слишком большой архив текстур".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("Слишком большой архив текстур".into());
        }
        remaining -= bytes.len() as u64;
        maps[index] = Some(Arc::new(Bitmap::decode(name, &bytes)?));
    }
    Ok(maps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn online_material_catalogs_filter_and_search_metadata() {
        let poly = poly_catalog(&serde_json::json!({
            "brick_wall": {"name": "Brick Wall", "categories": ["Brown Stone"]},
            "../invalid": {"name": "Ignore"}
        }))
        .unwrap();
        assert_eq!(poly.len(), 1);
        assert!(poly[0].matches("brick brown"));
        assert!(!poly[0].matches("brick metal"));
        let (count, ambient) = ambient_catalog(&serde_json::json!({"assets": [
            {"id": "Bricks001", "title": "Old Wall", "tags": ["masonry"],
                "thumbnails": {"256-WEBP": "https://acg-media.struffelproductions.com/Bricks001.webp"}},
            {"id": "bad/id"}
        ]}))
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(ambient.len(), 1);
        assert!(ambient[0].matches("wall masonry"));
        assert!(
            ambient[0]
                .thumbnail
                .as_deref()
                .unwrap()
                .ends_with("Bricks001.webp")
        );
    }

    #[test]
    fn online_material_archive_keeps_opengl_normals_and_standard_channels() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, color) in [
            ("nested/Bricks_Color.png", [155, 56, 18, 255]),
            ("Bricks_Displacement.png", [42, 42, 42, 255]),
            ("Bricks_NormalDX.png", [128, 185, 255, 255]),
            ("Bricks_NormalGL.png", [128, 71, 255, 255]),
            ("../Other_Opacity.png", [0, 0, 0, 255]),
        ] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            let bitmap = Bitmap::from_rgba(
                name.into(),
                image::RgbaImage::from_pixel(2, 2, image::Rgba(color)),
            )
            .unwrap();
            archive.write_all(&bitmap.bytes).unwrap();
        }
        let maps = decode_ambient_zip(archive.finish().unwrap().into_inner()).unwrap();
        let pixel = |i: usize| maps[i].as_ref().unwrap().pixels().get_pixel(0, 0).0;
        assert_eq!(pixel(0), [155, 56, 18, 255]);
        assert_eq!(pixel(3), [42, 42, 42, 255]);
        assert_eq!(pixel(6), [128, 71, 255, 255]);
        assert!(maps[4].is_none());
    }

    #[test]
    #[ignore = "downloads public texture materials from two live providers"]
    fn online_material_downloads_real_pbr_maps_from_both_providers() {
        let root = std::env::temp_dir().join(format!(
            "bevy-material-catalog-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        for (provider, id) in [
            (Provider::PolyHaven, "brick_wall_001"),
            (Provider::AmbientCg, "Bricks001"),
        ] {
            let entries = fetch_catalog(provider).unwrap();
            assert!(entries.len() > 500);
            let entry = entries.iter().find(|entry| entry.id == id).unwrap();
            let (preview, file) = previews::live_thumbnail(&root, provider, entry).unwrap();
            assert!(preview.size.x.max(preview.size.y) <= 128);
            assert!(file.is_file());
            let cached_preview = crate::editor::hdri_catalog::public_thumbnail(
                "not a URL",
                Some(&file),
                bevy::prelude::UVec2::splat(128),
            )
            .unwrap();
            assert_eq!(cached_preview.rgba, preview.rgba);
            let layers = material_in(Some(root.clone()), provider, entry, 1).unwrap();
            let fill = layers[0].texture_fill.as_ref().unwrap();
            for slot in [0, 1, 3, 6] {
                assert_eq!(fill.images[slot].as_ref().unwrap().pixels().width(), 1024);
            }
            let manifest = root.join(format!("{}-{id}-1k/maps.json", provider.key()));
            assert!(manifest.is_file());
            let cached = material_in(Some(root.clone()), provider, entry, 1).unwrap();
            assert_eq!(
                cached[0].texture_fill.as_ref().unwrap().images[0]
                    .as_ref()
                    .unwrap()
                    .bytes,
                fill.images[0].as_ref().unwrap().bytes
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
