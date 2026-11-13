//! Filesystem operations for importing images and creating asset folders.
use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

use super::browser::is_image;

pub(super) fn directory(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Недопустимый путь папки",
        ));
    }
    let root = root.canonicalize()?;
    let target = root.join(relative).canonicalize()?;
    if !target.starts_with(&root) || !target.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Папка должна находиться внутри ассетов проекта",
        ));
    }
    Ok(target)
}

pub(super) fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("Введите название папки");
    }
    if name.starts_with('.') || name != name.trim() || name.ends_with('.') {
        return Err(
            "Название не должно начинаться с точки, содержать пробелы по краям или заканчиваться точкой",
        );
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || "\\/:*?\"<>|".contains(ch))
    {
        return Err("В названии нельзя использовать \\ / : * ? \" < > |");
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        })
    {
        return Err("Это имя зарезервировано системой. Выберите другое");
    }
    if name.encode_utf16().count() > 200 {
        return Err("Название слишком длинное (до 200 символов)");
    }
    Ok(())
}

pub(super) fn create_folder(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name).map_err(str::to_owned)?;
    let directory =
        directory(root, parent).map_err(|error| format!("Не удалось открыть папку: {error}"))?;
    fs::create_dir(directory.join(name)).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "Файл или папка с таким названием уже существует".into()
        } else {
            format!("Не удалось создать папку: {error}")
        }
    })?;
    Ok(parent.join(name))
}

pub(super) fn suggested_name(root: &Path, parent: &Path) -> String {
    let directory = root.join(parent);
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            "Новая папка".into()
        } else {
            format!("Новая папка ({suffix})")
        };
        if !directory.join(&name).exists() {
            return name;
        }
    }
    "Новая папка".into()
}

pub(super) fn validate_image_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("Ширина и высота должны быть от 1 до 8192 пикселей.".into());
    }
    if u64::from(width) * u64::from(height) > super::photo_document::MAX_PIXELS {
        return Err("Максимум 16 Мп — например, 4096 × 4096 пикселей.".into());
    }
    Ok(())
}

pub(super) fn image_stem(name: &str) -> Result<String, String> {
    let mut stem = name.to_owned();
    if stem.to_ascii_lowercase().ends_with(".png") {
        stem.truncate(stem.len() - 4);
    }
    if stem.trim().is_empty() {
        return Err("Введите название изображения.".into());
    }
    validate_name(&stem).map_err(str::to_owned)?;
    Ok(stem)
}

pub(super) fn suggested_image_name(root: &Path, parent: &Path) -> String {
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            "Новое изображение".into()
        } else {
            format!("Новое изображение ({suffix})")
        };
        if !root.join(parent).join(format!("{name}.png")).exists() {
            return name;
        }
    }
    "Новое изображение".into()
}

/// Called on an I/O worker. Exclusively create the file so existing artwork is
/// never overwritten, including when two queued creations share the same name.
pub(super) fn create_image(
    root: &Path,
    parent: &Path,
    name: &str,
    width: u32,
    height: u32,
) -> Result<PathBuf, String> {
    use image::ImageEncoder;
    validate_image_size(width, height)?;
    let stem = image_stem(name)?;
    let target =
        directory(root, parent).map_err(|error| format!("Не удалось открыть папку: {error}"))?;
    let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    for suffix in 1..10_000 {
        let filename = if suffix == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem} ({suffix}).png")
        };
        let destination = target.join(&filename);
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Не удалось создать изображение: {error}")),
        };
        let result = image::codecs::png::PngEncoder::new_with_quality(
            &mut file,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        )
        .write_image(
            pixels.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())
        .and_then(|_| file.flush().map_err(|error| error.to_string()));
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&destination);
            return Err(format!("Не удалось записать PNG: {error}"));
        }
        return Ok(parent.join(filename));
    }
    Err("Не удалось подобрать свободное имя изображения.".into())
}

#[derive(Default)]
pub(super) struct ImportReport {
    pub imported: Vec<PathBuf>,
    pub skipped: usize,
    pub errors: Vec<String>,
}

impl ImportReport {
    pub fn status(&self) -> String {
        let mut status = if self.imported.is_empty() {
            "Не добавлено новых изображений".into()
        } else {
            format!("Добавлено изображений: {}", self.imported.len())
        };
        if self.skipped != 0 {
            status.push_str(&format!(" · Уже в папке: {}", self.skipped));
        }
        if let Some(error) = self.errors.first() {
            status.push_str(&format!(" · {error}"));
            if self.errors.len() > 1 {
                status.push_str(&format!(" (и ещё {})", self.errors.len() - 1));
            }
        }
        status
    }
}

pub(super) fn import_images(root: &Path, parent: &Path, sources: Vec<PathBuf>) -> ImportReport {
    let mut report = ImportReport::default();
    let target = match directory(root, parent) {
        Ok(target) => target,
        Err(error) => {
            report
                .errors
                .push(format!("Не удалось открыть папку: {error}"));
            return report;
        }
    };
    let mut seen = HashSet::new();
    for source in sources {
        let label = source.file_name().unwrap_or_default().to_string_lossy();
        match import_image(&source, &target, &mut seen) {
            Ok(Some(name)) => report.imported.push(parent.join(name)),
            Ok(None) => report.skipped += 1,
            Err(error) => report.errors.push(format!("{label}: {error}")),
        }
    }
    report
}

fn import_image(
    source: &Path,
    target: &Path,
    seen: &mut HashSet<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    if !is_image(source) {
        return Err("Поддерживаются PNG, JPEG, WebP, BMP, TGA, GIF, ICO и SVG".into());
    }
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    if !source.is_file() {
        return Err("Перетащите файл изображения".into());
    }
    if !seen.insert(source.clone()) || source.parent() == Some(target) {
        return Ok(None);
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Неподдерживаемое имя файла")?;
    if name.starts_with('.') {
        return Err("Уберите точку в начале имени файла перед импортом".into());
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("image");
    let ext = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let mut input = fs::File::open(&source).map_err(|error| error.to_string())?;
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            name.to_owned()
        } else {
            format!("{stem} ({suffix}).{ext}")
        };
        let destination = target.join(&name);
        let mut output = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        // Only remove the file exclusively created by this import on a failed copy.
        if let Err(error) = io::copy(&mut input, &mut output).and_then(|_| output.flush()) {
            drop(output);
            let _ = fs::remove_file(&destination);
            return Err(error.to_string());
        }
        return Ok(Some(PathBuf::from(name)));
    }
    Err("Не удалось подобрать свободное имя файла".into())
}

use io::Write;

#[cfg(test)]
#[path = "browser_files_tests.rs"]
mod tests;
