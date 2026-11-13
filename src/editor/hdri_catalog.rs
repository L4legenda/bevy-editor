//! Public HDRI catalogs, bounded downloads and portable project storage.
use crate::{HdriAsset, HdriProvider, lighting::valid_hdri_id};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{self, Cursor, Read},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_JSON: u64 = 16 * 1024 * 1024;
const USER_AGENT: &str = "BevyEditor/0.1 (+https://github.com/L4legenda/bevy-editor)";
const CACHE: &str = ".bevy-editor/hdri-catalog";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RemoteFile {
    pub resolution: u32,
    pub extension: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CatalogEntry {
    pub provider: HdriProvider,
    pub id: String,
    pub name: String,
    pub thumbnail: String,
    pub tags: Vec<String>,
    pub downloads: Vec<RemoteFile>,
    pub popularity: u64,
}

impl CatalogEntry {
    pub fn key(&self) -> String {
        format!("{}-{}", self.provider.key(), self.id)
    }
    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!("{} {} {}", self.id, self.name, self.tags.join(" ")).to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct LibraryRecord {
    pub environment: HdriAsset,
    pub preview: String,
    pub source_url: String,
    pub license: String,
}

#[derive(Default)]
pub(super) struct Progress {
    pub received: AtomicU64,
    pub total: AtomicU64,
    /// 0: metadata, 1: transfer, 2: archive, 3: validation, 4: saving.
    pub phase: AtomicU8,
    pub cancelled: AtomicBool,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("Загрузка отменена".into())
        } else {
            Ok(())
        }
    }
}

fn allowed_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some(
                "api.polyhaven.com"
                    | "cdn.polyhaven.com"
                    | "cdn.polyhaven.org"
                    | "dl.polyhaven.org"
                    | "ambientcg.com"
                    | "www.ambientcg.com"
                    | "acg-media.struffelproductions.com"
                    | "acg-download.struffelproductions.com"
            )
        )
}

fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent(USER_AGENT)
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 5 || !allowed_url(attempt.url()) {
                        attempt.error("Недопустимое перенаправление HDRI")
                    } else {
                        attempt.follow()
                    }
                }))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn bounded_read(
    mut reader: impl Read,
    limit: u64,
    progress: Option<&Progress>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if let Some(progress) = progress {
            progress.check()?;
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() as u64 + count as u64 > limit {
            return Err("Файл превышает допустимый размер".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(progress) = progress {
            progress
                .received
                .store(bytes.len() as u64, Ordering::Relaxed);
        }
    }
}

fn get(url: &str, limit: u64, progress: Option<&Progress>) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(url).map_err(|error| error.to_string())?;
    if !allowed_url(&url) {
        return Err("Неизвестный источник HDRI".into());
    }
    if let Some(progress) = progress {
        progress.check()?;
    }
    let response = client()?
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Не удалось получить данные: {error}"))?;
    if let Some(size) = response.content_length() {
        if size > limit {
            return Err("Выберите меньшее разрешение HDRI".into());
        }
        if let Some(progress) = progress {
            progress.total.store(size, Ordering::Relaxed);
        }
    }
    bounded_read(response, limit, progress)
}

fn json(url: &str) -> Result<Value, String> {
    serde_json::from_slice(&get(url, MAX_JSON, None)?)
        .map_err(|error| format!("Неверный ответ каталога: {error}"))
}

fn clean_name(value: Option<&Value>, fallback: &str) -> String {
    let name: String = value
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .chars()
        .filter(|ch| !ch.is_control())
        .take(128)
        .collect();
    if name.trim().is_empty() {
        fallback.into()
    } else {
        name.trim().into()
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(64)
        .map(|value| value.chars().take(128).collect())
        .collect()
}

pub(super) fn parse_polyhaven(value: &Value) -> Result<Vec<CatalogEntry>, String> {
    let assets = value.as_object().ok_or("Неверный каталог Poly Haven")?;
    let mut result = Vec::new();
    for (id, data) in assets {
        if !valid_hdri_id(id) || data.get("type").and_then(Value::as_u64) != Some(0) {
            continue;
        }
        let thumbnail = data.get("thumbnail_url").and_then(Value::as_str).map(str::to_owned)
            .unwrap_or_else(|| format!("https://cdn.polyhaven.com/asset_img/thumbs/{id}.png?width=256&height=256&format=webp"));
        let mut tags = strings(data.get("tags"));
        tags.extend(strings(data.get("categories")));
        result.push(CatalogEntry {
            provider: HdriProvider::PolyHaven,
            id: id.clone(),
            name: clean_name(data.get("name"), id),
            thumbnail,
            tags,
            downloads: Vec::new(),
            popularity: data
                .get("download_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        });
    }
    result.sort_by(|a, b| {
        b.popularity
            .cmp(&a.popularity)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(result)
}

fn ambient_files(data: &Value) -> Vec<RemoteFile> {
    data.get("downloads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            let attributes = file.get("attributes")?.as_str()?.to_ascii_uppercase();
            let resolution = [1024, 2048, 4096].into_iter().find(|size| {
                attributes == format!("{}K", size / 1024)
                    || attributes.starts_with(&format!("{}K-", size / 1024))
            })?;
            let extension = file.get("extension")?.as_str()?.to_ascii_lowercase();
            if !matches!(extension.as_str(), "hdr" | "exr" | "zip") {
                return None;
            }
            Some(RemoteFile {
                resolution,
                extension,
                url: file.get("url")?.as_str()?.to_owned(),
                size: file.get("size").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
}

pub(super) fn parse_ambient(value: &Value) -> Result<Vec<CatalogEntry>, String> {
    let assets = value
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("Неверный каталог ambientCG")?;
    let mut result = Vec::new();
    for data in assets {
        let Some(id) = data
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| valid_hdri_id(id))
        else {
            continue;
        };
        let thumbs = data.get("thumbnails");
        let thumbnail = ["256-WEBP", "256-PNG", "256-JPG-FFFFFF"]
            .into_iter()
            .find_map(|format| {
                thumbs
                    .and_then(|thumbs| thumbs.get(format))
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_owned();
        result.push(CatalogEntry {
            provider: HdriProvider::AmbientCg,
            id: id.into(),
            name: clean_name(data.get("title"), id),
            thumbnail,
            tags: strings(data.get("tags")),
            downloads: ambient_files(data),
            popularity: 0,
        });
    }
    Ok(result)
}

/// Resolve every directory before continuing, including pre-existing symlinks.
fn directory(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let mut current = root.clone();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err("Недопустимая папка HDRI".into());
        };
        let next = current.join(part);
        match fs::create_dir(&next) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        current = next.canonicalize().map_err(|error| error.to_string())?;
        if !current.starts_with(&root) || !current.is_dir() {
            return Err("Папка HDRI должна быть внутри проекта".into());
        }
    }
    Ok(current)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "part-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

pub(super) fn catalog(
    root: &Path,
    provider: HdriProvider,
    refresh: bool,
) -> Result<(Vec<CatalogEntry>, String), String> {
    let cache = directory(root, Path::new(CACHE))?.join(format!("{}.json", provider.key()));
    let stored = fs::read(&cache)
        .ok()
        .filter(|bytes| bytes.len() as u64 <= MAX_JSON)
        .and_then(|bytes| serde_json::from_slice::<Vec<CatalogEntry>>(&bytes).ok());
    let recent = fs::metadata(&cache)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < Duration::from_secs(24 * 3600));
    if !refresh
        && recent
        && let Some(stored) = stored.as_ref()
    {
        return Ok((stored.clone(), String::new()));
    }
    let fetch = || -> Result<Vec<CatalogEntry>, String> {
        match provider {
            HdriProvider::PolyHaven => {
                parse_polyhaven(&json("https://api.polyhaven.com/assets?t=hdris")?)
            }
            HdriProvider::AmbientCg => {
                let mut result = Vec::new();
                for offset in (0..2000).step_by(500) {
                    let data = json(&format!(
                        "https://ambientcg.com/api/v3/assets?type=hdri&sort=popular&limit=500&offset={offset}&include=title,tags,thumbnails,downloads"
                    ))?;
                    let page = parse_ambient(&data)?;
                    let count = page.len();
                    result.extend(page);
                    if count < 500 {
                        break;
                    }
                }
                Ok(result)
            }
        }
    };
    match fetch() {
        Ok(entries) if !entries.is_empty() => {
            if let Ok(bytes) = serde_json::to_vec(&entries) {
                let _ = atomic_write(&cache, &bytes);
            }
            Ok((entries, String::new()))
        }
        result => {
            let error = result.err().unwrap_or_else(|| "Каталог пока пуст".into());
            stored
                .map(|entries| (entries, format!("Сохранённый каталог · {error}")))
                .ok_or(error)
        }
    }
}

fn select_file(entry: &CatalogEntry, resolution: u32) -> Result<RemoteFile, String> {
    if !valid_hdri_id(&entry.id) || ![1024, 2048, 4096].contains(&resolution) {
        return Err("Неверная карта или разрешение".into());
    }
    match entry.provider {
        HdriProvider::PolyHaven => {
            let data = json(&format!("https://api.polyhaven.com/files/{}", entry.id))?;
            let file = data
                .get("hdri")
                .and_then(|data| data.get(format!("{}k", resolution / 1024)))
                .and_then(|data| data.get("hdr"))
                .ok_or("Такое разрешение HDRI недоступно")?;
            Ok(RemoteFile {
                resolution,
                extension: "hdr".into(),
                url: file
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("Нет ссылки HDRI")?
                    .into(),
                size: file.get("size").and_then(Value::as_u64).unwrap_or(0),
            })
        }
        HdriProvider::AmbientCg => {
            if let Some(file) = entry
                .downloads
                .iter()
                .find(|file| file.resolution == resolution)
            {
                return Ok(file.clone());
            }
            let data = json(&format!(
                "https://ambientcg.com/api/v3/assets?type=hdri&id={}&include=downloads,title&limit=1",
                entry.id
            ))?;
            data.get("assets")
                .and_then(Value::as_array)
                .and_then(|data| data.first())
                .and_then(|data| {
                    ambient_files(data)
                        .into_iter()
                        .find(|file| file.resolution == resolution)
                })
                .ok_or_else(|| "Такое разрешение HDRI недоступно".into())
        }
    }
}

fn unpack(
    bytes: Vec<u8>,
    extension: &str,
    progress: &Progress,
) -> Result<(Vec<u8>, String), String> {
    if extension != "zip" {
        return Ok((bytes, extension.into()));
    }
    progress.phase.store(2, Ordering::Relaxed);
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    if archive.len() > 256 {
        return Err("Слишком много файлов в архиве HDRI".into());
    }
    for wanted in ["hdr", "exr"] {
        for index in 0..archive.len() {
            progress.check()?;
            let file = archive.by_index(index).map_err(|error| error.to_string())?;
            if !file
                .name()
                .map_err(|error| error.to_string())?
                .to_ascii_lowercase()
                .ends_with(&format!(".{wanted}"))
            {
                continue;
            }
            if file.size() > MAX_FILE {
                return Err("Панорама в архиве слишком велика".into());
            }
            // Read only the panorama. Archive filenames are never used as filesystem paths.
            return Ok((bounded_read(file, MAX_FILE, Some(progress))?, wanted.into()));
        }
    }
    Err("В архиве нет панорамы HDR или EXR".into())
}

fn thumbnail_file(root: &Path, entry: &CatalogEntry) -> Result<PathBuf, String> {
    if !valid_hdri_id(&entry.id) {
        return Err("Неверный ID HDRI".into());
    }
    Ok(directory(root, Path::new(CACHE))?.join(format!("{}.png", entry.key())))
}

pub(super) fn thumbnail(
    root: &Path,
    entry: &CatalogEntry,
) -> Result<super::image_previews::DecodedImage, String> {
    let file = thumbnail_file(root, entry)?;
    if file.is_file() {
        if let Ok(decoded) =
            super::image_previews::decode_image(&file, bevy::prelude::UVec2::new(256, 160))
        {
            return Ok(decoded);
        }
    }
    let bytes = get(&entry.thumbnail, 2 * 1024 * 1024, None)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| error.to_string())?
        .thumbnail(256, 160);
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    atomic_write(&file, &png.into_inner())?;
    super::image_previews::decode_image(&file, bevy::prelude::UVec2::new(256, 160))
}

pub(super) fn download(
    root: &Path,
    entry: &CatalogEntry,
    resolution: u32,
    progress: Arc<Progress>,
) -> Result<LibraryRecord, String> {
    progress.check()?;
    let file = select_file(entry, resolution)?;
    if file.size > MAX_FILE {
        return Err("Выберите меньшее разрешение HDRI".into());
    }
    progress.total.store(file.size, Ordering::Relaxed);
    progress.phase.store(1, Ordering::Relaxed);
    let (bytes, extension) = unpack(
        get(&file.url, MAX_FILE, Some(&progress))?,
        &file.extension,
        &progress,
    )?;
    progress.check()?;
    progress.phase.store(3, Ordering::Relaxed);
    crate::hdri::validate_panorama(&bytes, &extension)
        .map_err(|error| format!("Не удалось прочитать HDRI: {error}"))?;
    progress.check()?;
    let relative = format!(
        "hdri/{}/{}/{}k",
        entry.provider.key(),
        entry.id,
        resolution / 1024
    );
    let folder = directory(root, Path::new(&relative))?;
    let asset = HdriAsset {
        provider: entry.provider,
        id: entry.id.clone(),
        name: entry.name.clone(),
        resolution,
        asset: format!("{relative}/environment.{extension}"),
    };
    asset.validate().map_err(|error| error.to_string())?;
    progress.phase.store(4, Ordering::Relaxed);
    atomic_write(&folder.join(format!("environment.{extension}")), &bytes)?;
    let preview = format!("{relative}/preview.png");
    if let Ok(cached) = thumbnail_file(root, entry)
        && cached.is_file()
    {
        let _ = fs::copy(cached, folder.join("preview.png"));
    }
    let record = LibraryRecord {
        environment: asset,
        preview,
        source_url: entry.provider.asset_url(&entry.id),
        license: "CC0-1.0".into(),
    };
    let notice = format!(
        "{}\n{}\nCC0 1.0: https://creativecommons.org/publicdomain/zero/1.0/\n",
        entry.provider.label(),
        record.source_url
    );
    atomic_write(&folder.join("LICENSE.txt"), notice.as_bytes())?;
    atomic_write(
        &folder.join("environment.json"),
        &serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?,
    )?;
    Ok(record)
}

pub(super) fn scan(root: &Path) -> Result<Vec<LibraryRecord>, String> {
    let mut records = Vec::new();
    for provider in HdriProvider::ALL {
        let base = root.join("hdri").join(provider.key());
        let Ok(directories) = fs::read_dir(base) else {
            continue;
        };
        for item in directories.take(2000).flatten() {
            let Some(id) = item
                .file_name()
                .to_str()
                .map(str::to_owned)
                .filter(|id| valid_hdri_id(id))
            else {
                continue;
            };
            for resolution in [1024, 2048, 4096] {
                let folder = item.path().join(format!("{}k", resolution / 1024));
                let Ok(bytes) = fs::read(folder.join("environment.json")) else {
                    continue;
                };
                if bytes.len() > 64 * 1024 {
                    continue;
                }
                let Ok(record) = serde_json::from_slice::<LibraryRecord>(&bytes) else {
                    continue;
                };
                if record.environment.provider == provider
                    && record.environment.id == id
                    && record.environment.resolution == resolution
                    && record.environment.validate().is_ok()
                    && root.join(&record.environment.asset).is_file()
                {
                    records.push(record);
                }
            }
        }
    }
    records.sort_by(|a, b| {
        a.environment
            .name
            .cmp(&b.environment.name)
            .then_with(|| a.environment.resolution.cmp(&b.environment.resolution))
    });
    Ok(records)
}

#[cfg(test)]
#[path = "hdri_catalog_tests.rs"]
mod tests;
