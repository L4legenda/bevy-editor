//! Guarded moves inside an asset root, with editor-document reference updates.
//!
//! Relative glTF dependencies are checked before touching the filesystem. A
//! move requiring a URI rewrite is rejected, so embedded models and complete
//! model/texture folders can move without silently breaking external resources.

use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    Editor, RoomEdits,
    browser::{Browser, Entry},
};
use crate::asset_paths::{FILE as REDIRECT_FILE, Redirects};

struct Rewrite {
    before: PathBuf,
    after: PathBuf,
    original: Option<Vec<u8>>,
    updated: Vec<u8>,
    permissions: Option<fs::Permissions>,
}

struct MovePlan {
    from: PathBuf,
    to: PathBuf,
    companions: Vec<(PathBuf, PathBuf)>,
    rewrites: Vec<Rewrite>,
}

impl MovePlan {
    fn moves(&self) -> impl Iterator<Item = (&Path, &Path)> {
        std::iter::once((self.from.as_path(), self.to.as_path())).chain(
            self.companions
                .iter()
                .map(|(from, to)| (from.as_path(), to.as_path())),
        )
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn meta_path(path: &Path) -> PathBuf {
    let mut extension = path.extension().unwrap_or_default().to_os_string();
    if !extension.is_empty() {
        extension.push(".");
    }
    extension.push("meta");
    path.with_extension(extension)
}

fn destination_available(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "В папке назначения уже есть файл или папка с таким именем",
        )),
    }
}

fn remap(path: &Path, from: &Path, to: &Path) -> PathBuf {
    match path.strip_prefix(from) {
        Ok(suffix) if suffix.as_os_str().is_empty() => to.to_path_buf(),
        Ok(suffix) => to.join(suffix),
        Err(_) => path.to_path_buf(),
    }
}

fn remap_string(path: &mut String, from: &Path, to: &Path) {
    *path = remap(Path::new(path), from, to)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
}

fn remap_edits(edits: &mut RoomEdits, from: &Path, to: &Path) {
    for model in &mut edits.models {
        remap_string(&mut model.asset, from, to);
    }
}

fn validate_edit_paths(edits: &RoomEdits, from: &Path, to: &Path) -> io::Result<()> {
    for model in &edits.models {
        let mapped = remap(Path::new(&model.asset), from, to);
        if !mapped.to_str().is_some_and(|path| {
            crate::data::relative_path(&path.replace(std::path::MAIN_SEPARATOR, "/"), "glb")
        }) {
            return Err(invalid(
                "Путь размещённой модели после перемещения не поддерживается редактором",
            ));
        }
    }
    Ok(())
}

fn checked_path(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(invalid("Путь должен находиться внутри папки ассетов"));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(invalid(
                "Перемещение через символические ссылки не поддерживается",
            ));
        }
    }
    Ok(current)
}

fn portable_move_path(path: &Path) -> io::Result<()> {
    for component in path.components() {
        if let Component::Normal(name) = component {
            let name = name
                .to_str()
                .ok_or_else(|| invalid("Имя файла должно быть в UTF-8"))?;
            if name.contains(['\\', ':', '#']) {
                return Err(invalid(
                    "Имя файла содержит символы, несовместимые с переносимыми путями ассетов (\\, : или #)",
                ));
            }
        }
    }
    Ok(())
}

fn entry_path(editor: &Editor, browser: &Browser, entry: Entry) -> io::Result<PathBuf> {
    let path = match entry {
        Entry::Folder(folder) => browser
            .folders
            .get(folder)
            .map(|folder| folder.path.clone()),
        Entry::Model(model) => editor
            .project
            .catalog
            .get(model)
            .map(|model| PathBuf::from(&model.asset)),
        Entry::Scene { file, .. } | Entry::Image(file) | Entry::File(file) => {
            browser.files.get(file).map(|file| file.path.clone())
        }
    };
    path.ok_or_else(|| invalid("Файл больше не существует в текущем списке"))
}

fn all_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut pending = vec![PathBuf::new()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(root.join(&directory))? {
            let entry = entry?;
            let path = directory.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn reject_symlink_descendants(path: &Path) -> io::Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(invalid(
                    "Папка содержит символическую ссылку; перемещение отменено",
                ));
            }
            if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn glb_json(path: &Path) -> io::Result<serde_json::Value> {
    let mut file = fs::File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[..4] != b"glTF"
        || u32::from_le_bytes(header[4..8].try_into().map_err(io::Error::other)?) != 2
    {
        return Err(invalid("Поддерживается только glTF 2.0"));
    }
    let length = u32::from_le_bytes(header[8..12].try_into().map_err(io::Error::other)?) as u64;
    if file.metadata()?.len() != length {
        return Err(invalid("Повреждён заголовок GLB"));
    }
    let mut offset = 12;
    while offset < length {
        let mut chunk = [0; 8];
        file.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[..4].try_into().map_err(io::Error::other)?) as u64;
        if size % 4 != 0 || offset + 8 + size > length {
            return Err(invalid("Повреждён блок GLB"));
        }
        if &chunk[4..] == b"JSON" {
            if size > 32 * 1024 * 1024 {
                return Err(invalid(
                    "Блок JSON в GLB слишком велик для безопасной проверки",
                ));
            }
            let mut json = vec![0; size as usize];
            file.read_exact(&mut json)?;
            return serde_json::from_slice(&json).map_err(io::Error::other);
        }
        file.seek(SeekFrom::Current(size as i64))?;
        offset += 8 + size;
    }
    Err(invalid("В GLB отсутствует блок JSON"))
}

fn normalize(path: &Path) -> io::Result<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => result.push(component),
            Component::CurDir => {}
            Component::ParentDir if result.pop() => {}
            _ => return Err(invalid("Внешняя зависимость glTF выходит за папку ассетов")),
        }
    }
    Ok(result)
}

fn decode_uri(uri: &str) -> io::Result<Option<PathBuf>> {
    if uri.starts_with("data:") || uri.starts_with("https://") || uri.starts_with("http://") {
        return Ok(None);
    }
    if uri.contains(['\\', ':', '?', '#']) || uri.starts_with('/') {
        return Err(invalid(format!(
            "Неподдерживаемая внешняя ссылка glTF: {uri}"
        )));
    }
    let mut decoded = Vec::with_capacity(uri.len());
    let mut bytes = uri.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|value| (value as char).to_digit(16));
            let low = bytes.next().and_then(|value| (value as char).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else {
                return Err(invalid("Повреждён URI внешней зависимости glTF"));
            };
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).map_err(io::Error::other)?;
    if decoded.contains(['\\', ':', '\0']) || decoded.starts_with('/') {
        return Err(invalid("Неподдерживаемый путь внешней зависимости glTF"));
    }
    Ok(Some(PathBuf::from(decoded)))
}

fn check_uris(
    value: &serde_json::Value,
    root: &Path,
    source: &Path,
    from: &Path,
    to: &Path,
) -> io::Result<()> {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(uri) = object.get("uri").and_then(serde_json::Value::as_str)
                && let Some(relative) = decode_uri(uri)?
            {
                let parent = source.parent().unwrap_or(Path::new(""));
                let original_target = normalize(&parent.join(&relative))?;
                checked_path(root, &original_target)?;
                let new_source = remap(source, from, to);
                let new_parent = new_source.parent().unwrap_or(Path::new(""));
                let retained_target = normalize(&new_parent.join(&relative))?;
                if retained_target != remap(&original_target, from, to) {
                    return Err(invalid(format!(
                        "Перемещение изменит внешнюю ссылку «{uri}» в {}. Переместите модель вместе с её зависимостями в общей папке.",
                        source.display(),
                    )));
                }
            }
            for child in object.values() {
                check_uris(child, root, source, from, to)?;
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                check_uris(child, root, source, from, to)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn prepare(
    editor: &Editor,
    browser: &Browser,
    entry: Entry,
    target: usize,
) -> io::Result<Option<MovePlan>> {
    let from = entry_path(editor, browser, entry)?;
    if from.as_os_str().is_empty() {
        return Err(invalid("Корневую папку ассетов нельзя переместить"));
    }
    let target = &browser
        .folders
        .get(target)
        .ok_or_else(|| invalid("Папка назначения больше не существует"))?
        .path;
    let source = checked_path(&editor.root, &from)?;
    let destination = checked_path(&editor.root, target)?;
    if !destination.is_dir() {
        return Err(invalid("Назначение должно быть папкой"));
    }
    if from.parent() == Some(target.as_path()) {
        return Ok(None);
    }
    if source.is_dir() && target.starts_with(&from) {
        return Err(invalid("Папку нельзя переместить внутрь самой себя"));
    }
    let filename = from
        .file_name()
        .ok_or_else(|| invalid("У файла нет имени"))?;
    let to = target.join(filename);
    portable_move_path(&from)?;
    portable_move_path(&to)?;
    destination_available(&editor.root.join(&to))?;
    let redirect_path = PathBuf::from(REDIRECT_FILE);
    let original_redirects = read_optional(&editor.root.join(&redirect_path))?;
    if original_redirects.is_some() {
        checked_path(&editor.root, &redirect_path)?;
    }
    let mut redirects = Redirects::read(&editor.root)?;
    let mut companions = Vec::new();
    if matches!(entry, Entry::Image(_)) {
        let native = from.with_extension("pcraft");
        match fs::symlink_metadata(editor.root.join(&native)) {
            Ok(metadata) => {
                checked_path(&editor.root, &native)?;
                if !metadata.is_file() {
                    return Err(invalid("Сопутствующий .pcraft должен быть обычным файлом"));
                }
                let target = to.with_extension("pcraft");
                destination_available(&editor.root.join(&target))?;
                companions.push((native, target));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if source.is_file() {
        // Metadata may have been moved separately earlier. Follow its retained
        // alias rather than silently dropping those settings on the next move.
        let original_meta = redirects.resolve(meta_path(&from))?;
        match fs::symlink_metadata(editor.root.join(&original_meta)) {
            Ok(metadata) => {
                checked_path(&editor.root, &original_meta)?;
                if !metadata.is_file() {
                    return Err(invalid("Сопутствующий .meta должен быть обычным файлом"));
                }
                let moved_meta = meta_path(&to);
                if original_meta != moved_meta {
                    destination_available(&editor.root.join(&moved_meta))?;
                    companions.push((original_meta, moved_meta));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    reject_symlink_descendants(&source)?;
    for edits in std::iter::once(&editor.edits)
        .chain(std::iter::once(&editor.saved))
        .chain(&editor.undo)
        .chain(&editor.redo)
        .chain(&editor.camera_navigation)
    {
        validate_edit_paths(edits, &from, &to)?;
    }
    for document in editor.documents.values() {
        for edits in std::iter::once(&document.edits)
            .chain(std::iter::once(&document.saved))
            .chain(&document.undo)
            .chain(&document.redo)
        {
            validate_edit_paths(edits, &from, &to)?;
        }
    }

    // Read only JSON chunks, not the potentially huge embedded BIN payloads.
    for file in all_files(&editor.root)? {
        let extension = file
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let json = match extension.as_str() {
            "glb" => glb_json(&editor.root.join(&file)),
            "gltf" => fs::read(editor.root.join(&file))
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other)),
            _ => continue,
        }
        .map_err(|error| {
            invalid(format!(
                "Не удалось проверить зависимости {}: {error}",
                file.display()
            ))
        })?;
        check_uris(&json, &editor.root, &file, &from, &to)?;
    }

    let mut updated = editor.project.clone();
    for model in &mut updated.catalog {
        remap_string(&mut model.asset, &from, &to);
        if !crate::data::relative_path(&model.asset, "glb") {
            return Err(invalid(
                "Путь модели после перемещения не поддерживается редактором",
            ));
        }
    }
    let mut rewrites = Vec::new();
    for (index, scene) in updated.scenes.iter_mut().enumerate() {
        remap_string(&mut scene.asset, &from, &to);
        remap_string(&mut scene.file, &from, &to);
        if (!scene.is_native() && !crate::data::relative_path(&scene.asset, "glb"))
            || !crate::data::document_path(&scene.file)
        {
            return Err(invalid(
                "Путь сцены после перемещения не поддерживается редактором",
            ));
        }
        let original_path = PathBuf::from(&editor.project.scenes[index].file);
        let absolute = editor.root.join(&original_path);
        if !absolute.try_exists()? {
            continue;
        }
        checked_path(&editor.root, &original_path)?;
        let edits = editor.project.read_scene(index)?;
        validate_edit_paths(&edits, &from, &to)?;
        let mut new_edits = edits.clone();
        remap_edits(&mut new_edits, &from, &to);
        if new_edits == edits {
            continue;
        }
        let original = fs::read(&absolute)?;
        let mut updated = serde_json::to_vec_pretty(&new_edits).map_err(io::Error::other)?;
        updated.push(b'\n');
        rewrites.push(Rewrite {
            before: original_path,
            after: PathBuf::from(&scene.file),
            original: Some(original),
            updated,
            permissions: Some(fs::metadata(absolute)?.permissions()),
        });
    }
    redirects.record_move(&from, &to)?;
    for (from, to) in &companions {
        redirects.record_move(from, to)?;
    }
    rewrites.push(Rewrite {
        before: redirect_path.clone(),
        after: redirect_path,
        original: original_redirects,
        updated: redirects.encode()?,
        permissions: fs::metadata(editor.root.join(REDIRECT_FILE))
            .ok()
            .map(|metadata| metadata.permissions()),
    });
    Ok(Some(MovePlan {
        from,
        to,
        companions,
        rewrites,
    }))
}

/// Reserve the destination before moving, so a pre-existing or concurrently
/// created file is never silently overwritten. Source and target share a root.
fn move_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    if fs::symlink_metadata(source)?.is_dir() {
        // Windows rejects existing directory destinations. Unix can replace
        // empty directories, so reserve one first on those platforms.
        #[cfg(windows)]
        fs::rename(source, destination)?;
        #[cfg(not(windows))]
        {
            fs::create_dir(destination)?;
            if let Err(error) = fs::rename(source, destination) {
                let _ = fs::remove_dir(destination);
                return Err(error);
            }
        }
    } else {
        let reservation = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        drop(reservation);
        if let Err(error) = fs::rename(source, destination) {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
    }
    Ok(())
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(invalid(
            "Файл карты перемещений не должен быть символической ссылкой",
        )),
        Ok(_) => fs::read(path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_staged(path: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    Ok(())
}

fn commit(root: &Path, plan: &MovePlan) -> io::Result<()> {
    let moves: Vec<_> = plan.moves().collect();
    for &(from, to) in &moves {
        checked_path(root, from)?;
        checked_path(root, to.parent().unwrap_or(Path::new("")))?;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let staging = root.join(format!(
        ".bevy-editor-move-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&staging)?;
    let staged = (|| {
        for (index, rewrite) in plan.rewrites.iter().enumerate() {
            if read_optional(&root.join(&rewrite.before))? != rewrite.original {
                return Err(invalid(
                    "Документ изменился во время подготовки перемещения",
                ));
            }
            if let Some(original) = &rewrite.original {
                write_staged(
                    &staging.join(format!("{index}.original")),
                    original,
                    rewrite.permissions.clone(),
                )?;
            }
            write_staged(
                &staging.join(format!("{index}.updated")),
                &rewrite.updated,
                rewrite.permissions.clone(),
            )?;
        }
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    for (index, &(from, to)) in moves.iter().enumerate() {
        if let Err(error) = move_no_replace(&root.join(from), &root.join(to)) {
            if let Some(rollback_error) = rollback_moves(root, &moves[..index]) {
                return Err(io::Error::other(format!(
                    "Перемещение не завершено: {error}. Ошибка отката: {rollback_error}. Резервные документы: {}",
                    staging.display()
                )));
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    }
    for (index, rewrite) in plan.rewrites.iter().enumerate() {
        let replace = (|| {
            if read_optional(&root.join(&rewrite.after))? != rewrite.original {
                return Err(invalid("Документ изменился во время перемещения"));
            }
            fs::rename(
                staging.join(format!("{index}.updated")),
                root.join(&rewrite.after),
            )
        })();
        if let Err(error) = replace {
            let mut rollback_error = None;
            for (applied, previous) in plan.rewrites[..index].iter().enumerate().rev() {
                let restored = if previous.original.is_some() {
                    fs::rename(
                        staging.join(format!("{applied}.original")),
                        root.join(&previous.after),
                    )
                } else {
                    fs::remove_file(root.join(&previous.after))
                };
                if let Err(error) = restored {
                    rollback_error = Some(error);
                }
            }
            if let Some(error) = rollback_moves(root, &moves) {
                rollback_error = Some(error);
            }
            if let Some(rollback_error) = rollback_error {
                return Err(io::Error::other(format!(
                    "Перемещение не завершено: {error}. Ошибка отката: {rollback_error}. Резервные документы: {}",
                    staging.display()
                )));
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(())
}

fn rollback_moves(root: &Path, moves: &[(&Path, &Path)]) -> Option<io::Error> {
    let mut failed = None;
    for &(from, to) in moves.iter().rev() {
        if let Err(error) = move_no_replace(&root.join(to), &root.join(from)) {
            failed = Some(error);
        }
    }
    failed
}

/// Move one browser entry without changing scene-edit dirty state or history.
/// Returns `false` when the entry is already in the requested parent folder.
pub(super) fn move_entry(
    editor: &mut Editor,
    browser: &mut Browser,
    entry: Entry,
    target_folder: usize,
) -> io::Result<bool> {
    let Some(plan) = prepare(editor, browser, entry, target_folder)? else {
        return Ok(false);
    };
    commit(&editor.root, &plan)?;
    for model in &mut editor.project.catalog {
        remap_string(&mut model.asset, &plan.from, &plan.to);
    }
    for scene in &mut editor.project.scenes {
        remap_string(&mut scene.asset, &plan.from, &plan.to);
        remap_string(&mut scene.file, &plan.from, &plan.to);
    }
    remap_edits(&mut editor.edits, &plan.from, &plan.to);
    remap_edits(&mut editor.saved, &plan.from, &plan.to);
    for edits in editor
        .undo
        .iter_mut()
        .chain(&mut editor.redo)
        .chain(&mut editor.camera_navigation)
    {
        remap_edits(edits, &plan.from, &plan.to);
    }
    for document in editor.documents.values_mut() {
        for edits in std::iter::once(&mut document.edits)
            .chain(std::iter::once(&mut document.saved))
            .chain(&mut document.undo)
            .chain(&mut document.redo)
        {
            remap_edits(edits, &plan.from, &plan.to);
        }
    }
    for tab in &mut editor.image_tabs {
        tab.path = remap(&tab.path, &plan.from, &plan.to);
    }
    browser.refresh_after_move(&mut editor.project, &plan.from, &plan.to);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{PlacedModel, Pose},
        project::{CameraDefinition, EditorProject, SceneDefinition},
    };
    use bevy::prelude::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "bevy-editor-moves-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, path: &str, bytes: &[u8]) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn glb(&self, path: &str, external: Option<&str>) {
            let mut json = serde_json::json!({"asset":{"version":"2.0"}, "scenes":[{}]});
            if let Some(uri) = external {
                json["images"] = serde_json::json!([{"uri":uri}]);
            }
            let mut chunk = serde_json::to_vec(&json).unwrap();
            while chunk.len() % 4 != 0 {
                chunk.push(b' ');
            }
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"glTF");
            bytes.extend_from_slice(&2_u32.to_le_bytes());
            bytes.extend_from_slice(&((20 + chunk.len()) as u32).to_le_bytes());
            bytes.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            bytes.extend_from_slice(b"JSON");
            bytes.extend_from_slice(&chunk);
            self.file(path, &bytes);
        }

        fn editor(&self) -> (Editor, Browser) {
            self.glb("scenes/room.glb", None);
            let scene = SceneDefinition {
                label: "Room".into(),
                asset: "scenes/room.glb".into(),
                file: "levels/room.json".into(),
                groups: Vec::new(),
                excluded_nodes: Vec::new(),
                node_policies: Default::default(),
                cameras: vec![CameraDefinition {
                    id: "main".into(),
                    label: "Main".into(),
                    placement: crate::CameraPlacement {
                        pose: Pose::from(Transform::IDENTITY),
                        lens: 60.0,
                    },
                    projection: Default::default(),
                }],
                initial_selection: None,
                play: None,
            };
            let mut editor = Editor::new(
                EditorProject {
                    title: "Test".into(),
                    asset_root: self.0.clone(),
                    scenes: vec![scene],
                    catalog: Vec::new(),
                    scene_setup: None,
                    camera_setup: None,
                },
                RoomEdits::default(),
            );
            let browser = Browser::new(&mut editor.project);
            (editor, browser)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn folder(browser: &Browser, path: &str) -> usize {
        browser
            .folders
            .iter()
            .position(|folder| folder.path == Path::new(path))
            .unwrap()
    }

    fn model(editor: &Editor, path: &str) -> usize {
        editor
            .project
            .catalog
            .iter()
            .position(|model| model.asset == path)
            .unwrap()
    }

    #[test]
    fn moving_an_open_image_updates_its_tab_path_without_changing_its_identity() {
        let files = Fixture::new();
        files.file("images/photo.png", b"image");
        files.file("images/photo.pcraft", b"native layers");
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        editor.open_image_tab(PathBuf::from("images/photo.png"));
        let tab = editor.viewing_image.unwrap();
        let image = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("images/photo.png"))
            .unwrap();
        let destination = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::Image(image), destination).unwrap());
        assert_eq!(editor.viewing_image, Some(tab));
        assert_eq!(
            editor.image_tab(tab).unwrap().path,
            PathBuf::from("archive/photo.png")
        );
        assert!(files.0.join("archive/photo.png").exists());
        assert_eq!(
            fs::read(files.0.join("archive/photo.pcraft")).unwrap(),
            b"native layers"
        );
        assert!(!files.0.join("images/photo.pcraft").exists());
    }

    fn placements(asset: &str) -> RoomEdits {
        RoomEdits {
            models: vec![PlacedModel {
                id: 1,
                asset: asset.into(),
                pose: Pose::from(Transform::IDENTITY),
                bounds: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn moving_native_scene_models_rewrites_the_single_scene_document() {
        let files = Fixture::new();
        files.glb("models/chair.glb", None);
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, _) = files.editor();
        editor.project.scenes[0].asset.clear();
        editor.project.scenes[0].file = "scenes/room.scene".into();
        let mut edits = placements("models/chair.glb");
        edits.scene = Some(crate::SceneSettings {
            label: "Room".into(),
            cameras: editor.project.scenes[0].cameras.clone(),
        });
        edits.write(&files.0, "scenes/room.scene").unwrap();
        editor.edits = edits.clone();
        editor.saved = edits;
        fs::write(
            files.0.join(crate::project::PROJECT_FILE_NAME),
            serde_json::to_vec_pretty(&editor.project).unwrap(),
        )
        .unwrap();
        let mut browser = Browser::new(&mut editor.project);
        let index = model(&editor, "models/chair.glb");
        let target = folder(&browser, "archive");
        move_entry(&mut editor, &mut browser, Entry::Model(index), target).unwrap();
        let saved = editor.project.read_scene(0).unwrap();
        assert_eq!(saved.models[0].asset, "archive/chair.glb");
        assert!(saved.scene.is_some());
        let file = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("scenes/room.scene"))
            .unwrap();
        let target = folder(&browser, "archive");
        move_entry(
            &mut editor,
            &mut browser,
            Entry::Scene { file, room: 0 },
            target,
        )
        .unwrap();
        assert_eq!(editor.project.scenes[0].file, "archive/room.scene");
        let reopened = EditorProject::load(&files.0).unwrap();
        assert!(reopened.scenes[0].is_native());
        assert_eq!(reopened.read_scene(0).unwrap(), saved);
    }

    #[test]
    fn moves_models_and_updates_saved_documents_and_history_without_losing_unsaved_edits() {
        let files = Fixture::new();
        files.glb("models/chair.glb", None);
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        editor.saved = placements("models/chair.glb");
        editor.saved.write(&files.0, "levels/room.json").unwrap();
        let mut second = editor.project.scenes[0].clone();
        second.file = "levels/other.json".into();
        editor.project.scenes.push(second);
        editor.saved.write(&files.0, "levels/other.json").unwrap();
        editor.edits = editor.saved.clone();
        editor.edits.models[0].pose.position[0] = 7.0;
        editor.undo.push(editor.saved.clone());
        editor.redo.push(editor.edits.clone());
        assert!(editor.open_document(1));
        editor.edits.models[0].pose.position[0] = 8.0;
        editor.undo.push(editor.saved.clone());
        editor.redo.push(editor.edits.clone());
        editor.camera_navigation = Some(editor.edits.clone());
        let dirty = editor.dirty();
        let index = model(&editor, "models/chair.glb");
        let target = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::Model(index), target).unwrap());
        assert!(!files.0.join("models/chair.glb").exists());
        assert!(files.0.join("archive/chair.glb").is_file());
        assert_eq!(editor.project.catalog[index].asset, "archive/chair.glb");
        assert_eq!(editor.dirty(), dirty);
        assert_eq!(editor.edits.models[0].pose.position[0], 8.0);
        for edits in [
            &editor.edits,
            &editor.saved,
            &editor.undo[0],
            &editor.redo[0],
            editor.camera_navigation.as_ref().unwrap(),
        ] {
            assert_eq!(edits.models[0].asset, "archive/chair.glb");
        }
        let inactive = &editor.documents[&0];
        assert_eq!(inactive.edits.models[0].pose.position[0], 7.0);
        assert!(editor.document_dirty(0));
        for edits in [
            &inactive.edits,
            &inactive.saved,
            &inactive.undo[0],
            &inactive.redo[0],
        ] {
            assert_eq!(edits.models[0].asset, "archive/chair.glb");
        }
        for path in ["levels/room.json", "levels/other.json"] {
            let saved: RoomEdits = RoomEdits::read(&files.0, path).unwrap();
            assert_eq!(saved.models[0].asset, "archive/chair.glb");
            assert_eq!(
                saved.models[0].pose.position[0], 0.0,
                "file moves must not save unrelated current edits"
            );
        }
    }

    #[test]
    fn moves_whole_dependency_folders_and_keeps_navigation_attached_to_the_moved_branch() {
        let files = Fixture::new();
        files.glb("packs/chair/model.glb", Some("textures/wood%20albedo.png"));
        files.file("packs/chair/textures/wood albedo.png", b"texture");
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        let source = folder(&browser, "packs/chair");
        let textures = folder(&browser, "packs/chair/textures");
        let target = folder(&browser, "archive");
        browser.navigate(textures);
        assert!(move_entry(&mut editor, &mut browser, Entry::Folder(source), target).unwrap());
        assert_eq!(
            browser.folders[browser.current].path,
            Path::new("archive/chair/textures")
        );
        assert!(files.0.join("archive/chair/model.glb").is_file());
        assert!(
            files
                .0
                .join("archive/chair/textures/wood albedo.png")
                .is_file()
        );
        assert!(
            editor
                .project
                .catalog
                .iter()
                .any(|item| item.asset == "archive/chair/model.glb")
        );
    }

    #[test]
    fn rejects_moves_that_break_external_dependencies_before_any_mutation() {
        let files = Fixture::new();
        files.glb("models/chair.glb", Some("../textures/wood.png"));
        files.file("textures/wood.png", b"texture");
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        let image = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("textures/wood.png"))
            .unwrap();
        let target = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::Image(image), target).is_err());
        assert!(files.0.join("textures/wood.png").exists());
        assert!(!files.0.join("archive/wood.png").exists());
        assert_eq!(editor.project.catalog[0].asset, "models/chair.glb");
        assert_eq!(browser.scan_revision, 0);
        assert!(!files.0.join(REDIRECT_FILE).exists());
    }

    #[test]
    fn moving_configured_scene_and_document_folders_persists_restart_paths() {
        let files = Fixture::new();
        files.glb("content/scene.glb", None);
        files.glb("content/models/chair.glb", None);
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, _) = files.editor();
        editor.project.scenes[0].asset = "content/scene.glb".into();
        editor.project.scenes[0].file = "content/room.json".into();
        editor.saved = placements("content/models/chair.glb");
        editor.edits = editor.saved.clone();
        editor.saved.write(&files.0, "content/room.json").unwrap();
        let original = editor.project.clone();
        let mut browser = Browser::new(&mut editor.project);
        let source = folder(&browser, "content");
        let target = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::Folder(source), target).unwrap());
        assert_eq!(editor.project.scenes[0].asset, "archive/content/scene.glb");
        assert_eq!(editor.project.scenes[0].file, "archive/content/room.json");
        assert!(!editor.dirty());
        let redirects = Redirects::read(&files.0).unwrap();
        assert_eq!(
            redirects
                .resolve(Path::new(&original.scenes[0].asset))
                .unwrap(),
            Path::new("archive/content/scene.glb")
        );
        assert_eq!(
            redirects
                .resolve(Path::new(&original.scenes[0].file))
                .unwrap(),
            Path::new("archive/content/room.json")
        );
        let document: RoomEdits = RoomEdits::read(&files.0, "archive/content/room.json").unwrap();
        assert_eq!(document.models[0].asset, "archive/content/models/chair.glb");
        assert!(!files.0.join("content").exists());
        assert!(files.0.join(REDIRECT_FILE).is_file());
    }

    #[test]
    fn failed_reference_commit_rolls_back_the_move_and_exact_document_bytes() {
        let files = Fixture::new();
        files.glb("models/chair.glb", None);
        files.file("models/chair.glb.meta", b"original metadata settings\n");
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, browser) = files.editor();
        editor.saved = placements("models/chair.glb");
        editor.edits = editor.saved.clone();
        editor.saved.write(&files.0, "levels/room.json").unwrap();
        let original = fs::read(files.0.join("levels/room.json")).unwrap();
        let index = model(&editor, "models/chair.glb");
        let target = folder(&browser, "archive");
        let mut plan = prepare(&editor, &browser, Entry::Model(index), target)
            .unwrap()
            .unwrap();
        // Simulate a destination disappearing after staging: the level rewrite
        // succeeds, then the final redirect-map replacement cannot be committed.
        plan.rewrites.last_mut().unwrap().after = PathBuf::from("missing/redirects.json");
        assert!(commit(&files.0, &plan).is_err());
        assert!(files.0.join("models/chair.glb").is_file());
        assert!(!files.0.join("archive/chair.glb").exists());
        assert_eq!(
            fs::read(files.0.join("models/chair.glb.meta")).unwrap(),
            b"original metadata settings\n"
        );
        assert!(!files.0.join("archive/chair.glb.meta").exists());
        assert_eq!(
            fs::read(files.0.join("levels/room.json")).unwrap(),
            original
        );
        assert!(!files.0.join(REDIRECT_FILE).exists());
        assert!(fs::read_dir(&files.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".bevy-editor-move-")
        }));
    }

    #[test]
    fn individual_asset_moves_carry_exact_metadata_settings_and_redirect_both_paths() {
        let files = Fixture::new();
        files.glb("models/chair.glb", None);
        let settings = b"(meta_format_version: \"1.0\", custom_loader_settings: [1, 2, 3])\n";
        files.file("models/chair.glb.meta", settings);
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        let index = model(&editor, "models/chair.glb");
        let target = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::Model(index), target).unwrap());
        assert!(!files.0.join("models/chair.glb").exists());
        assert!(!files.0.join("models/chair.glb.meta").exists());
        assert_eq!(
            fs::read(files.0.join("archive/chair.glb.meta")).unwrap(),
            settings
        );
        let redirects = Redirects::read(&files.0).unwrap();
        assert_eq!(
            redirects.resolve("models/chair.glb").unwrap(),
            Path::new("archive/chair.glb")
        );
        assert_eq!(
            redirects.resolve("models/chair.glb.meta").unwrap(),
            Path::new("archive/chair.glb.meta")
        );
    }

    #[test]
    fn metadata_collisions_prevent_both_the_asset_and_metadata_move() {
        let files = Fixture::new();
        files.glb("models/chair.glb", None);
        files.file("models/chair.glb.meta", b"source settings");
        files.file("archive/chair.glb.meta", b"existing destination settings");
        let original_model = fs::read(files.0.join("models/chair.glb")).unwrap();
        let (mut editor, mut browser) = files.editor();
        let index = model(&editor, "models/chair.glb");
        let target = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::Model(index), target).is_err());
        assert_eq!(
            fs::read(files.0.join("models/chair.glb")).unwrap(),
            original_model
        );
        assert_eq!(
            fs::read(files.0.join("models/chair.glb.meta")).unwrap(),
            b"source settings"
        );
        assert!(!files.0.join("archive/chair.glb").exists());
        assert_eq!(
            fs::read(files.0.join("archive/chair.glb.meta")).unwrap(),
            b"existing destination settings"
        );
        assert!(!files.0.join(REDIRECT_FILE).exists());
    }

    #[test]
    fn collisions_descendants_root_and_same_parent_do_not_change_files() {
        let files = Fixture::new();
        files.file("items/readme.txt", b"source");
        files.file("items/report#1.txt", b"unambiguous filename required");
        files.file("destination/readme.txt", b"destination");
        fs::create_dir(files.0.join("items/child")).unwrap();
        let (mut editor, mut browser) = files.editor();
        let file = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("items/readme.txt"))
            .unwrap();
        let target = folder(&browser, "destination");
        let items = folder(&browser, "items");
        let child = folder(&browser, "items/child");
        assert!(move_entry(&mut editor, &mut browser, Entry::File(file), target).is_err());
        assert!(move_entry(&mut editor, &mut browser, Entry::Folder(items), child).is_err());
        assert!(move_entry(&mut editor, &mut browser, Entry::Folder(0), target).is_err());
        let reserved = browser
            .files
            .iter()
            .position(|file| file.filename == "report#1.txt")
            .unwrap();
        assert!(move_entry(&mut editor, &mut browser, Entry::File(reserved), target).is_err());
        assert!(files.0.join("items/report#1.txt").is_file());
        assert!(!move_entry(&mut editor, &mut browser, Entry::File(file), items).unwrap());
        assert_eq!(
            fs::read(files.0.join("items/readme.txt")).unwrap(),
            b"source"
        );
        assert_eq!(
            fs::read(files.0.join("destination/readme.txt")).unwrap(),
            b"destination"
        );
    }

    #[test]
    fn regular_files_can_move_after_all_scene_tabs_are_closed() {
        let files = Fixture::new();
        files.file("notes/readme.txt", b"notes");
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        editor.close_document(0);
        let file = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("notes/readme.txt"))
            .unwrap();
        let target = folder(&browser, "archive");
        assert!(move_entry(&mut editor, &mut browser, Entry::File(file), target).unwrap());
        assert_eq!(
            fs::read(files.0.join("archive/readme.txt")).unwrap(),
            b"notes"
        );
        assert!(!editor.has_scene());
        assert!(!editor.dirty());
    }

    #[test]
    fn the_same_model_can_move_back_without_creating_duplicate_catalog_entries() {
        let files = Fixture::new();
        files.glb("models/chair.glb", None);
        fs::create_dir(files.0.join("archive")).unwrap();
        let (mut editor, mut browser) = files.editor();
        editor.saved = placements("models/chair.glb");
        editor.edits = editor.saved.clone();
        editor.saved.write(&files.0, "levels/room.json").unwrap();
        let index = model(&editor, "models/chair.glb");
        let target = folder(&browser, "archive");
        move_entry(&mut editor, &mut browser, Entry::Model(index), target).unwrap();
        let original_parent = folder(&browser, "models");
        move_entry(
            &mut editor,
            &mut browser,
            Entry::Model(index),
            original_parent,
        )
        .unwrap();
        assert_eq!(editor.project.catalog.len(), 2);
        assert_eq!(editor.project.catalog[index].asset, "models/chair.glb");
        assert_eq!(editor.edits.models[0].asset, "models/chair.glb");
        assert!(files.0.join("models/chair.glb").is_file());
        assert!(!files.0.join("archive/chair.glb").exists());
        let redirects = Redirects::read(&files.0).unwrap();
        assert_eq!(
            redirects.resolve("models/chair.glb").unwrap(),
            Path::new("models/chair.glb")
        );
        assert_eq!(
            redirects.resolve("archive/chair.glb").unwrap(),
            Path::new("models/chair.glb")
        );
        assert!(!editor.dirty());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_source_folders_with_symlinks_and_destinations_replaced_by_symlinks() {
        let files = Fixture::new();
        let outside = Fixture::new();
        files.file("items/file.txt", b"source");
        fs::create_dir(files.0.join("destination")).unwrap();
        let (mut editor, mut browser) = files.editor();
        let source = folder(&browser, "items");
        let target = folder(&browser, "destination");
        std::os::unix::fs::symlink(&outside.0, files.0.join("items/escape")).unwrap();
        assert!(move_entry(&mut editor, &mut browser, Entry::Folder(source), target).is_err());
        fs::remove_dir(files.0.join("destination")).unwrap();
        std::os::unix::fs::symlink(&outside.0, files.0.join("destination")).unwrap();
        assert!(move_entry(&mut editor, &mut browser, Entry::Folder(source), target).is_err());
        assert!(files.0.join("items/file.txt").exists());
        assert!(fs::read_dir(&outside.0).unwrap().next().is_none());
    }
}
