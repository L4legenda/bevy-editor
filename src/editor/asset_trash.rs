//! Same-volume session trash, recoverable until the editor exits.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const TRASH: &str = ".bevy-editor-trash";

#[derive(Debug)]
pub(super) struct TrashedFile {
    pub original: PathBuf,
    stored: PathBuf,
}

fn relative_file(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || path.starts_with(TRASH)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Недопустимый путь файла",
        ));
    }
    Ok(())
}

fn checked_file(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    relative_file(relative)?;
    let path = root.join(relative);
    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Ссылки не удаляются",
        ));
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Файл должен находиться внутри ассетов проекта",
        ));
    }
    Ok(canonical)
}

pub(super) fn remove(root: &Path, relative: &Path) -> io::Result<TrashedFile> {
    let root = root.canonicalize()?;
    let source = checked_file(&root, relative)?;
    let trash = root.join(TRASH);
    match fs::symlink_metadata(&trash) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Недопустимая папка корзины",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&trash)?,
        Err(error) => return Err(error),
    }
    if !trash.canonicalize()?.starts_with(&root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Корзина должна находиться внутри проекта",
        ));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let slot = (0..1000)
        .find_map(|index| {
            let slot = trash.join(format!("{stamp}-{}-{index}", std::process::id()));
            match fs::create_dir(&slot) {
                Ok(()) => Some(Ok(slot)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .unwrap_or_else(|| Err(io::Error::other("Не удалось создать запись корзины")))?;
    // Preserve the original path for recovery after an interrupted session.
    fs::write(
        slot.join("original-path.txt"),
        relative.to_string_lossy().as_bytes(),
    )?;
    let stored = slot.join("payload");
    fs::rename(&source, &stored)?;
    Ok(TrashedFile {
        original: relative.into(),
        stored: stored.strip_prefix(root).unwrap().into(),
    })
}

pub(super) fn restore(root: &Path, file: &TrashedFile) -> io::Result<()> {
    let root = root.canonicalize()?;
    relative_file(&file.original)?;
    let stored = root.join(&file.stored);
    let source = stored.canonicalize()?;
    let trash = root.join(TRASH).canonicalize()?;
    if fs::symlink_metadata(&stored)?.file_type().is_symlink()
        || !trash.starts_with(&root)
        || !source.starts_with(&trash)
        || !source.is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Недопустимая запись корзины",
        ));
    }
    let parent = crate::editor::browser_files::directory(&root, file.original.parent().unwrap())?;
    let destination = parent.join(file.original.file_name().unwrap());
    // An atomic hard link refuses to overwrite an existing file on every platform.
    fs::hard_link(&source, &destination)?;
    // The destination already owns the bytes. A locked trash link may be
    // cleaned later; it must not turn a successful restore into an error.
    let _ = fs::remove_file(&source);
    Ok(())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes directory junctions, which must never be traversed.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Remove only the dedicated trash directory under this asset root.
pub(super) fn empty(root: &Path) -> io::Result<()> {
    let root = root.canonicalize()?;
    let path = root.join(TRASH);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Недопустимая папка корзины",
        ));
    }
    let resolved = path.canonicalize()?;
    if resolved.parent() != Some(root.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Корзина должна находиться внутри ассетов проекта",
        ));
    }
    empty_directory(&resolved, &resolved)
}

fn empty_directory(path: &Path, boundary: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let resolved = path.canonicalize()?;
    if is_link(&metadata) || !metadata.is_dir() || !resolved.starts_with(boundary) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Путь выходит за пределы корзины",
        ));
    }
    for entry in fs::read_dir(&resolved)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if is_link(&metadata) {
            // Unlink junctions/symlinks themselves, never their targets.
            #[cfg(windows)]
            let directory_link = {
                use std::os::windows::fs::MetadataExt;
                // FileType::is_dir() excludes junctions; use their directory attribute.
                metadata.file_attributes() & 0x10 != 0
            };
            #[cfg(not(windows))]
            let directory_link = false;
            if directory_link {
                fs::remove_dir(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        } else if metadata.is_dir() {
            empty_directory(&path, boundary)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    fs::remove_dir(&resolved)
}
