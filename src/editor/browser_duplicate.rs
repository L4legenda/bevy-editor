//! Copies saved assets and their companions without replacing existing files.
use crate::editor::{browser, browser_files, file_moves};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        for name in ["file", "meta", "layers"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn copy_saved(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = fs::File::open(source)?;
    let before = input.metadata()?;
    if !before.is_file() {
        return Err(invalid("Для дублирования выберите обычный файл"));
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    let after = fs::metadata(source)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(invalid("Исходный файл изменился. Повторите дублирование"));
    }
    Ok(())
}

fn candidate(source: &Path, number: usize) -> io::Result<PathBuf> {
    let extension = source
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let suffix = if number == 1 {
        " — копия".into()
    } else {
        format!(" — копия ({number})")
    };
    let available = 200usize
        .checked_sub(suffix.encode_utf16().count() + extension.encode_utf16().count())
        .ok_or_else(|| invalid("Расширение файла слишком длинное"))?;
    let mut stem = String::new();
    let mut length = 0;
    for ch in source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
    {
        length += ch.len_utf16();
        if length > available {
            break;
        }
        stem.push(ch);
    }
    let filename = format!("{stem}{suffix}{extension}");
    browser_files::validate_name(&filename).map_err(invalid)?;
    Ok(source.with_file_name(filename))
}

fn optional_file(root: &Path, relative: &Path) -> io::Result<Option<PathBuf>> {
    match fs::symlink_metadata(root.join(relative)) {
        Ok(_) => {
            let path = file_moves::checked_path(root, relative)?;
            if !path.is_file() {
                return Err(invalid("Сопутствующий файл должен быть обычным файлом"));
            }
            Ok(Some(path))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn duplicate(root: &Path, source: &Path) -> Result<PathBuf, String> {
    duplicate_inner(root, source).map_err(|error| error.to_string())
}

fn duplicate_inner(root: &Path, source: &Path) -> io::Result<PathBuf> {
    let root = root.canonicalize()?;
    let absolute = file_moves::checked_path(&root, source)?;
    if !absolute.is_file() {
        return Err(invalid("Для дублирования выберите файл"));
    }
    let parent = source
        .parent()
        .ok_or_else(|| invalid("У файла нет папки"))?;
    let stage = Staging(root.join(parent).join(format!(
        ".bevy-editor-copy-{}",
        bevy::asset::uuid::Uuid::new_v4()
    )));
    fs::create_dir(&stage.0)?;
    copy_saved(&absolute, &stage.0.join("file"))?;
    let redirects = crate::asset_paths::Redirects::read(&root)?;
    let meta = optional_file(&root, &redirects.resolve(file_moves::meta_path(source))?)?;
    if let Some(path) = &meta {
        copy_saved(path, &stage.0.join("meta"))?;
    }
    let layers = if browser::is_image(source) {
        optional_file(&root, &source.with_extension("pcraft"))?
    } else {
        None
    };
    let native = if let Some(path) = &layers {
        if fs::metadata(path)?.len() > 512 << 20 {
            return Err(invalid("Файл слоёв превышает 512 МБ"));
        }
        Some(fs::read(path)?)
    } else {
        None
    };
    for number in 1..10_000 {
        let to = candidate(source, number)?;
        let mut destinations = vec![];
        if meta.is_some() {
            destinations.push(("meta", file_moves::meta_path(&to)));
        }
        if native.is_some() {
            destinations.push(("layers", to.with_extension("pcraft")));
        }
        destinations.push(("file", to.clone()));
        // Reserve sidecar names even if the source does not currently have one.
        let mut reserved = vec![to.clone(), file_moves::meta_path(&to)];
        if browser::is_image(source) {
            reserved.push(to.with_extension("pcraft"));
        }
        let mut occupied = false;
        for path in reserved {
            if redirects.resolve(&path)? != path {
                occupied = true;
            }
            match fs::symlink_metadata(root.join(path)) {
                Ok(_) => occupied = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if occupied {
            continue;
        }
        if let Some(native) = &native {
            let bytes = file_moves::image_bundle_for_name(native, &to)?;
            let mut output = fs::File::create(stage.0.join("layers"))?;
            output.write_all(&bytes)?;
            output.sync_all()?;
        }
        let mut published: Vec<(&str, PathBuf)> = Vec::new();
        let mut collision = false;
        for (name, path) in &destinations {
            if let Err(error) = file_moves::move_no_replace(&stage.0.join(name), &root.join(path)) {
                for &(old_name, ref old_path) in published.iter().rev() {
                    file_moves::move_no_replace(&root.join(old_path), &stage.0.join(old_name))
                        .map_err(|rollback| io::Error::other(format!("Не удалось завершить копирование: {error}. Ошибка отката: {rollback}")))?;
                }
                if error.kind() == io::ErrorKind::AlreadyExists {
                    collision = true;
                    break;
                }
                return Err(error);
            }
            published.push((*name, path.clone()));
        }
        if !collision {
            return Ok(to);
        }
    }
    Err(invalid("Не удалось подобрать свободное имя копии"))
}

#[cfg(test)]
#[path = "browser_duplicate_tests.rs"]
mod tests;
