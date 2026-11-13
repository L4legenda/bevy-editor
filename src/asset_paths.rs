//! Persistent aliases for assets moved inside an editor project.
//!
//! Add [`AssetRedirectPlugin`] before Bevy's `DefaultPlugins` in native hosts.
//! Existing logical asset names then keep working without rewriting game code.

use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

pub const FILE: &str = ".bevy-editor-asset-moves.json";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Normalize portable asset paths without allowing an absolute path or escape
/// above the asset root. Both slash styles are accepted on every host platform.
fn relative(path: &Path) -> io::Result<PathBuf> {
    let value = path
        .to_str()
        .ok_or_else(|| invalid("Asset paths must be valid UTF-8"))?
        .replace('\\', "/");
    if value.starts_with('/') || value.contains(':') || value.contains('\0') {
        return Err(invalid("Asset paths must be relative to the asset root"));
    }
    let mut result = PathBuf::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !result.pop() {
                    return Err(invalid("Asset paths cannot leave the asset root"));
                }
            }
            part => result.push(part),
        }
    }
    Ok(result)
}

fn replace_prefix(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|tail| to.join(tail))
}

/// Serialized move history. Alias targets are composed whenever their current
/// physical location moves again. `origins` retains stable callback IDs even
/// when an asset is moved back and an identity alias is removed.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Redirects {
    moves: BTreeMap<PathBuf, PathBuf>,
    origins: BTreeMap<PathBuf, PathBuf>,
}

impl Redirects {
    /// A project without a sidecar has no redirects.
    pub fn read(root: impl AsRef<Path>) -> io::Result<Self> {
        let path = root.as_ref().join(FILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid("The asset move sidecar must not be a symlink"));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
            _ => {}
        }
        let parsed: Self = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        let mut normalized = Self::default();
        for (source, target) in parsed.moves {
            let source = relative(&source)?;
            let target = relative(&target)?;
            if let Some(previous) = normalized.moves.insert(source, target.clone())
                && previous != target
            {
                return Err(invalid("Conflicting asset move aliases"));
            }
        }
        for (physical, logical) in parsed.origins {
            let physical = relative(&physical)?;
            let logical = relative(&logical)?;
            if let Some(previous) = normalized.origins.insert(physical, logical.clone())
                && previous != logical
            {
                return Err(invalid("Conflicting original asset names"));
            }
        }
        normalized.moves.retain(|source, target| source != target);
        normalized.validate()?;
        Ok(normalized)
    }

    /// Follow the most specific matching file or folder alias to its current
    /// physical relative path. Repeated application is idempotent.
    pub fn resolve(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let mut current = relative(path.as_ref())?;
        let mut visited = HashSet::new();
        for _ in 0..1024 {
            if !visited.insert(current.clone()) {
                return Err(invalid("Cyclic asset move aliases"));
            }
            let mapping = self
                .moves
                .iter()
                .filter(|(from, _)| current.starts_with(from))
                .max_by_key(|(from, _)| from.components().count());
            let Some((from, to)) = mapping else {
                return Ok(current);
            };
            current = relative(
                &replace_prefix(&current, from, to)
                    .ok_or_else(|| invalid("Invalid asset move prefix"))?,
            )?;
            if current.as_os_str().len() > 8192 {
                return Err(invalid("Expanding asset move cycle"));
            }
        }
        Err(invalid("Asset move alias chain is too long or cyclic"))
    }

    /// Return a stable logical ID for scene/material hooks. The physical path
    /// is resolved first, so this also accepts an older alias.
    pub fn original(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let physical = self.resolve(path)?;
        if let Some((current, original)) = self
            .origins
            .iter()
            .filter(|(current, _)| physical.starts_with(current))
            .max_by_key(|(current, _)| current.components().count())
        {
            return relative(
                &replace_prefix(&physical, current, original)
                    .ok_or_else(|| invalid("Invalid original asset prefix"))?,
            );
        }
        // Accept move-only sidecars too. New records always persist explicit
        // origins, avoiding ambiguity between several aliases of one asset.
        let mut original = physical;
        let mut visited = HashSet::new();
        for _ in 0..1024 {
            if !visited.insert(original.clone()) || original.as_os_str().len() > 8192 {
                return Err(invalid("Cyclic original asset aliases"));
            }
            let mapping = self
                .moves
                .iter()
                .filter(|(_, to)| original.starts_with(to))
                .max_by(|(from_a, to_a), (from_b, to_b)| {
                    to_a.components()
                        .count()
                        .cmp(&to_b.components().count())
                        .then_with(|| from_b.cmp(from_a))
                });
            let Some((from, to)) = mapping else {
                return Ok(original);
            };
            original = relative(
                &replace_prefix(&original, to, from)
                    .ok_or_else(|| invalid("Invalid reverse asset prefix"))?,
            )?;
        }
        Err(invalid("Original asset alias chain is too long"))
    }

    /// Prepare the sidecar change for a filesystem transaction. This method
    /// performs no writes and leaves the previous state intact on rejection.
    pub fn record_move(&mut self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let from = relative(from.as_ref())?;
        let to = relative(to.as_ref())?;
        if from.as_os_str().is_empty() || to.as_os_str().is_empty() {
            return Err(invalid("The asset root cannot be moved"));
        }
        if from == Path::new(FILE) || to == Path::new(FILE) {
            return Err(invalid("The asset move sidecar cannot be moved"));
        }
        if from == to {
            return Ok(());
        }
        if from.starts_with(&to) || to.starts_with(&from) {
            return Err(invalid("A folder cannot move into itself or an ancestor"));
        }
        if self.resolve(&from)? != from {
            return Err(invalid("The source path is reserved for a moved asset"));
        }
        let resolved_to = self.resolve(&to)?;
        let returning = resolved_to == from;
        if (resolved_to != to && !returning)
            || (!returning
                && self
                    .moves
                    .keys()
                    .any(|alias| alias.starts_with(&to) || to.starts_with(alias)))
        {
            return Err(invalid(
                "The destination is reserved for another moved asset",
            ));
        }
        let original = self.original(&from)?;
        let mut next = self.clone();
        for target in next.moves.values_mut() {
            if let Some(relocated) = replace_prefix(target, &from, &to) {
                *target = relocated;
            }
        }
        next.moves.retain(|source, target| source != target);
        // A moved folder can contain aliases for children moved out earlier.
        // Rebase those aliases too, preserving older logical parent names.
        let nested: Vec<_> = next
            .moves
            .iter()
            .filter_map(|(source, target)| {
                replace_prefix(source, &from, &to).map(|source| (source, target.clone()))
            })
            .collect();
        for (source, target) in nested {
            if let Some(previous) = next.moves.insert(source, target.clone())
                && previous != target
                && next.resolve(&previous)? != next.resolve(&target)?
            {
                return Err(invalid(
                    "Moving the folder would shadow another asset alias",
                ));
            }
        }
        next.moves.insert(from.clone(), to.clone());
        next.moves.retain(|source, target| source != target);
        let mut origins = BTreeMap::new();
        for (physical, logical) in &self.origins {
            let physical = replace_prefix(physical, &from, &to).unwrap_or_else(|| physical.clone());
            if let Some(previous) = origins.insert(physical, logical.clone())
                && previous != *logical
            {
                return Err(invalid(
                    "Moving the folder would shadow an original asset ID",
                ));
            }
        }
        origins.insert(to.clone(), original);
        next.origins = origins;
        next.validate()?;
        if next.resolve(&to)? != to {
            return Err(invalid("The destination would remain an asset alias"));
        }
        *self = next;
        Ok(())
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn validate(&self) -> io::Result<()> {
        for (source, target) in &self.moves {
            if relative(source)? != *source
                || relative(target)? != *target
                || source.as_os_str().is_empty()
                || target.as_os_str().is_empty()
                || source.starts_with(target)
                || target.starts_with(source)
            {
                return Err(invalid("Invalid or cyclic asset move prefix"));
            }
            self.resolve(source)?;
        }
        for (physical, original) in &self.origins {
            if relative(physical)? != *physical
                || relative(original)? != *original
                || physical.as_os_str().is_empty()
                || original.as_os_str().is_empty()
                || self.resolve(physical)? != *physical
            {
                return Err(invalid("Invalid original asset prefix"));
            }
        }
        Ok(())
    }
}

fn asset_string(redirects: &Redirects, path: &str, original: bool) -> io::Result<String> {
    let (path, label) = path
        .split_once('#')
        .map_or((path, None), |(path, label)| (path, Some(label)));
    let path = if original {
        redirects.original(path)?
    } else {
        redirects.resolve(path)?
    };
    let mut result = path
        .to_str()
        .ok_or_else(|| invalid("Asset paths must be valid UTF-8"))?
        .replace('\\', "/");
    if let Some(label) = label {
        result.push('#');
        result.push_str(label);
    }
    Ok(result)
}

pub fn resolve_asset_path(root: impl AsRef<Path>, path: &str) -> io::Result<String> {
    let root = root.as_ref().canonicalize()?;
    resolved_string(&Redirects::read(&root)?, &root, path)
}

pub fn original_asset_path(root: impl AsRef<Path>, path: &str) -> io::Result<String> {
    asset_string(&Redirects::read(root)?, path, true)
}

fn contained(root: &Path, physical: &Path) -> io::Result<()> {
    let full = root.join(physical);
    for ancestor in full.ancestors() {
        match ancestor.canonicalize() {
            Ok(canonical) if canonical.starts_with(root) => return Ok(()),
            Ok(_) => {
                return Err(invalid(
                    "Asset path leaves the asset root through a symlink",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
    }
    Err(invalid("Asset path has no valid root"))
}

fn resolved_string(redirects: &Redirects, root: &Path, path: &str) -> io::Result<String> {
    let result = asset_string(redirects, path, false)?;
    contained(
        root,
        Path::new(result.split('#').next().unwrap_or_default()),
    )?;
    Ok(result)
}

/// Apply saved moves before validating a project loaded from an unchanged host
/// manifest. Repeated calls leave physical paths unchanged.
#[cfg(feature = "editor")]
pub fn apply_project_paths(project: &mut crate::project::EditorProject) -> io::Result<()> {
    let root = project.asset_root.canonicalize()?;
    let redirects = Redirects::read(&root)?;
    let scenes: Vec<_> = project
        .scenes
        .iter()
        .map(|scene| {
            Ok((
                if scene.asset.is_empty() {
                    String::new()
                } else {
                    resolved_string(&redirects, &root, &scene.asset)?
                },
                resolved_string(&redirects, &root, &scene.file)?,
            ))
        })
        .collect::<io::Result<_>>()?;
    let catalog: Vec<_> = project
        .catalog
        .iter()
        .map(|item| resolved_string(&redirects, &root, &item.asset))
        .collect::<io::Result<_>>()?;
    for (scene, (asset, file)) in project.scenes.iter_mut().zip(scenes) {
        scene.asset = asset;
        scene.file = file;
    }
    for (item, asset) in project.catalog.iter_mut().zip(catalog) {
        item.asset = asset;
    }
    Ok(())
}

/// Shared by all native asset readers. Reload only after the move transaction
/// has committed, so a failed transaction never changes live asset routing.
#[derive(Clone, Resource)]
pub struct AssetRedirects {
    root: PathBuf,
    redirects: Arc<RwLock<Redirects>>,
}

impl AssetRedirects {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(invalid("Asset root must be a directory"));
        }
        let redirects = Redirects::read(&root)?;
        Ok(Self {
            root,
            redirects: Arc::new(RwLock::new(redirects)),
        })
    }

    pub fn reload(&self) -> io::Result<()> {
        let updated = Redirects::read(&self.root)?;
        *self
            .redirects
            .write()
            .map_err(|_| io::Error::other("Asset redirect lock was poisoned"))? = updated;
        Ok(())
    }

    pub fn resolve(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let physical = self
            .redirects
            .read()
            .map_err(|_| io::Error::other("Asset redirect lock was poisoned"))?
            .resolve(path)?;
        // Lexical normalization also protects missing paths. For existing
        // paths, resolve the nearest ancestor to reject escaping symlinks.
        contained(&self.root, &physical)?;
        Ok(physical)
    }

    pub fn original(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        self.redirects
            .read()
            .map_err(|_| io::Error::other("Asset redirect lock was poisoned"))?
            .original(path)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{
        io::{Read, Seek, SeekFrom},
        pin::Pin,
        task::{Context, Poll},
    };

    use bevy::{
        asset::{
            AssetApp,
            io::{
                AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream,
                Reader, ReaderNotSeekableError, STACK_FUTURE_SIZE, SeekableReader, StackFuture,
                file::FileAssetReader,
            },
        },
        prelude::{App, Plugin},
        tasks::futures_lite::{AsyncRead, AsyncSeek, StreamExt},
    };

    use super::*;

    /// Native hosts must install this before `DefaultPlugins` / `AssetPlugin`.
    pub struct AssetRedirectPlugin {
        redirects: AssetRedirects,
    }

    impl AssetRedirectPlugin {
        pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
            Ok(Self {
                redirects: AssetRedirects::new(root)?,
            })
        }
    }

    impl Plugin for AssetRedirectPlugin {
        fn build(&self, app: &mut App) {
            let shared = self.redirects.clone();
            let reader = shared.clone();
            let root = shared.root.to_string_lossy().into_owned();
            app.register_asset_source(
                AssetSourceId::Default,
                AssetSourceBuilder::platform_default(&root, None)
                    .with_reader(move || Box::new(RedirectReader::new(reader.clone()))),
            )
            .insert_resource(shared);
        }
    }

    struct RedirectReader {
        file: FileAssetReader,
        redirects: AssetRedirects,
    }

    impl RedirectReader {
        fn new(redirects: AssetRedirects) -> Self {
            Self {
                file: FileAssetReader::new(&redirects.root),
                redirects,
            }
        }

        fn open(&self, path: &Path, meta: bool) -> Result<FileReader, AssetReaderError> {
            let physical = self.redirects.resolve(path)?;
            let physical = if meta {
                let mut meta = physical.clone();
                let mut extension = physical.extension().unwrap_or_default().to_os_string();
                if !extension.is_empty() {
                    extension.push(".");
                }
                extension.push("meta");
                meta.set_extension(extension);
                self.redirects.resolve(meta)?
            } else {
                physical
            };
            let full = self.file.root_path().join(physical);
            fs::File::open(&full).map(FileReader).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    AssetReaderError::NotFound(full)
                } else {
                    error.into()
                }
            })
        }
    }

    // An owned streaming reader avoids returning a borrow of a temporary mapped
    // path, and avoids buffering each full GLB twice just to apply a redirect.
    struct FileReader(fs::File);

    impl AsyncRead for FileReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(self.get_mut().0.read(buffer))
        }
    }

    impl AsyncSeek for FileReader {
        fn poll_seek(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            position: SeekFrom,
        ) -> Poll<io::Result<u64>> {
            Poll::Ready(self.get_mut().0.seek(position))
        }
    }

    impl Reader for FileReader {
        fn read_to_end<'a>(
            &'a mut self,
            buffer: &'a mut Vec<u8>,
        ) -> StackFuture<'a, io::Result<usize>, STACK_FUTURE_SIZE> {
            StackFuture::from(async move { self.0.read_to_end(buffer) })
        }

        fn seekable(&mut self) -> Result<&mut dyn SeekableReader, ReaderNotSeekableError> {
            Ok(self)
        }
    }

    impl AssetReader for RedirectReader {
        async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
            self.open(path, false)
        }

        async fn read_meta<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<impl Reader + 'a, AssetReaderError> {
            self.open(path, true)
        }

        async fn read_directory<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<Box<PathStream>, AssetReaderError> {
            let physical = self.redirects.resolve(path)?;
            let logical = relative(path)?;
            let directory = self.file.read_directory(&physical).await?;
            Ok(Box::new(directory.map(move |entry| {
                replace_prefix(&entry, &physical, &logical).unwrap_or(entry)
            })))
        }

        async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
            let physical = self.redirects.resolve(path)?;
            self.file.is_directory(&physical).await
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::asset_paths::tests::Directory;

        #[test]
        fn reader_preserves_logical_paths_and_reloads_after_a_second_move() {
            let directory = Directory::new();
            fs::create_dir(directory.0.join("Current")).unwrap();
            fs::write(directory.0.join("Current/model.bin"), b"model bytes").unwrap();
            fs::write(directory.0.join("Current/model.bin.meta"), b"meta bytes").unwrap();
            let mut redirects = Redirects::default();
            redirects.record_move("Original", "Current").unwrap();
            fs::write(directory.0.join(FILE), redirects.encode().unwrap()).unwrap();
            let shared = AssetRedirects::new(&directory.0).unwrap();
            let reader = RedirectReader::new(shared.clone());
            bevy::tasks::block_on(async {
                let mut asset = reader.read(Path::new("Original/model.bin")).await.unwrap();
                let mut bytes = Vec::new();
                asset.read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, b"model bytes");
                assert!(asset.seekable().is_ok());
                let mut meta = reader
                    .read_meta(Path::new("Original/model.bin"))
                    .await
                    .unwrap();
                let mut bytes = Vec::new();
                meta.read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, b"meta bytes");
                assert!(reader.is_directory(Path::new("Original")).await.unwrap());
                let mut entries = reader.read_directory(Path::new("Original")).await.unwrap();
                assert_eq!(
                    entries.next().await,
                    Some(PathBuf::from("Original/model.bin"))
                );
            });
            fs::rename(directory.0.join("Current"), directory.0.join("Final")).unwrap();
            redirects.record_move("Current", "Final").unwrap();
            fs::write(directory.0.join(FILE), redirects.encode().unwrap()).unwrap();
            shared.reload().unwrap();
            bevy::tasks::block_on(async {
                let mut asset = reader.read(Path::new("Original/model.bin")).await.unwrap();
                let mut bytes = Vec::new();
                asset.read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, b"model bytes");
            });
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::AssetRedirectPlugin;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    pub(super) struct Directory(pub PathBuf);

    impl Directory {
        pub(super) fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "bevy-editor-asset-paths-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn chained_moves_keep_current_paths_idempotent_and_original_ids_stable() {
        let mut redirects = Redirects::default();
        redirects
            .record_move("Models/A.glb", "Props/B.glb")
            .unwrap();
        redirects
            .record_move("Props/B.glb", "Shared/C.glb")
            .unwrap();
        for path in ["Models/A.glb", "Props/B.glb", "Shared/C.glb"] {
            assert_eq!(redirects.resolve(path).unwrap(), Path::new("Shared/C.glb"));
            assert_eq!(redirects.original(path).unwrap(), Path::new("Models/A.glb"));
        }
    }

    #[test]
    fn moving_back_removes_the_identity_alias_without_losing_the_original_id() {
        let mut redirects = Redirects::default();
        redirects.record_move("A.glb", "B.glb").unwrap();
        redirects.record_move("B.glb", "A.glb").unwrap();
        assert_eq!(redirects.resolve("A.glb").unwrap(), Path::new("A.glb"));
        assert_eq!(redirects.resolve("B.glb").unwrap(), Path::new("A.glb"));
        assert_eq!(redirects.original("A.glb").unwrap(), Path::new("A.glb"));
        assert_eq!(redirects.original("B.glb").unwrap(), Path::new("A.glb"));
    }

    #[test]
    fn moving_parent_folders_preserves_children_moved_out_earlier() {
        let mut redirects = Redirects::default();
        redirects.record_move("A", "B").unwrap();
        redirects.record_move("B/child", "Shared/child").unwrap();
        redirects.record_move("B", "D").unwrap();
        assert_eq!(
            redirects.resolve("A/other.glb").unwrap(),
            Path::new("D/other.glb")
        );
        for path in [
            "A/child/model.glb",
            "B/child/model.glb",
            "D/child/model.glb",
        ] {
            assert_eq!(
                redirects.resolve(path).unwrap(),
                Path::new("Shared/child/model.glb")
            );
            assert_eq!(
                redirects.original(path).unwrap(),
                Path::new("A/child/model.glb")
            );
        }
        redirects.record_move("D", "A").unwrap();
        assert_eq!(
            redirects.resolve("A/other.glb").unwrap(),
            Path::new("A/other.glb")
        );
        assert_eq!(
            redirects.original("A/other.glb").unwrap(),
            Path::new("A/other.glb")
        );
        assert_eq!(
            redirects.resolve("A/child/model.glb").unwrap(),
            Path::new("Shared/child/model.glb")
        );
    }

    #[test]
    fn a_renamed_child_keeps_its_original_id_when_its_parent_moves_back() {
        let mut redirects = Redirects::default();
        redirects.record_move("A/child", "A/renamed").unwrap();
        redirects.record_move("A", "B").unwrap();
        redirects.record_move("B", "A").unwrap();
        assert_eq!(
            redirects.resolve("A/child/file.glb").unwrap(),
            Path::new("A/renamed/file.glb")
        );
        assert_eq!(
            redirects.original("A/renamed/file.glb").unwrap(),
            Path::new("A/child/file.glb")
        );
    }

    #[test]
    fn rejected_alias_reuse_and_cycles_leave_the_previous_map_unchanged() {
        let mut redirects = Redirects::default();
        redirects.record_move("B.glb", "C.glb").unwrap();
        let previous = redirects.encode().unwrap();
        assert!(redirects.record_move("A.glb", "B.glb").is_err());
        assert_eq!(redirects.encode().unwrap(), previous);
        assert!(redirects.record_move("Folder", "Folder/Nested").is_err());
        let cycle = Redirects {
            moves: BTreeMap::from([
                (PathBuf::from("A"), PathBuf::from("B")),
                (PathBuf::from("B"), PathBuf::from("A")),
            ]),
            origins: BTreeMap::new(),
        };
        assert!(cycle.resolve("A/file.glb").is_err());
        assert!(cycle.encode().is_err());
    }

    #[test]
    fn portable_slashes_and_subasset_labels_survive_persistence() {
        let directory = Directory::new();
        let mut redirects = Redirects::read(&directory.0).unwrap();
        redirects
            .record_move("Models\\Chair.glb", "Props\\Seat.glb")
            .unwrap();
        fs::write(directory.0.join(FILE), redirects.encode().unwrap()).unwrap();
        let reopened = Redirects::read(&directory.0).unwrap();
        assert_eq!(
            reopened.resolve("Models/Chair.glb").unwrap(),
            Path::new("Props/Seat.glb")
        );
        assert_eq!(
            resolve_asset_path(&directory.0, "Models\\Chair.glb#Scene0").unwrap(),
            "Props/Seat.glb#Scene0"
        );
        assert_eq!(
            original_asset_path(&directory.0, "Props/Seat.glb#Scene0").unwrap(),
            "Models/Chair.glb#Scene0"
        );
        for path in [
            "../outside",
            "A/../../outside",
            "/absolute",
            "C:\\outside",
            "\\\\server\\share",
        ] {
            assert!(reopened.resolve(path).is_err(), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_routing_rejects_a_symlink_leading_outside_the_asset_root() {
        let root = Directory::new();
        let outside = Directory::new();
        fs::write(outside.0.join("image.png"), b"outside").unwrap();
        std::os::unix::fs::symlink(&outside.0, root.0.join("escape")).unwrap();
        let shared = AssetRedirects::new(&root.0).unwrap();
        assert!(shared.resolve("escape/image.png").is_err());
    }
}
