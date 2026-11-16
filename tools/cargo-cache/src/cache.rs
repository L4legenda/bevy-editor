use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const INDEX: &str = ".bevy-editor-cache.json";

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not just symbolic links.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

pub struct Root {
    pub path: PathBuf,
}
impl Root {
    pub fn new(path: &Path) -> io::Result<Self> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if linked(&metadata) => {
                return Err(io::Error::other(
                    "Refusing to maintain a linked target directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir_all(path)?;
        let path = path.canonicalize()?;
        if path.parent().is_none() {
            return Err(io::Error::other(
                "A filesystem root cannot be a build cache",
            ));
        }
        Ok(Self { path })
    }
    fn check(&self) -> io::Result<()> {
        if linked(&fs::symlink_metadata(&self.path)?) || self.path.canonicalize()? != self.path {
            return Err(io::Error::other(
                "The target directory has changed; cleanup skipped",
            ));
        }
        Ok(())
    }
    pub fn local_file(&self, name: &Path) -> Option<PathBuf> {
        let path = if name.is_absolute() {
            name.to_owned()
        } else {
            self.path.join(name)
        };
        if !fs::symlink_metadata(&path).ok()?.is_file() {
            return None;
        }
        // Reject linked files and parents before resolving. Unlinking always uses
        // the individually verified absolute path, never a shell or glob.
        for ancestor in path.ancestors() {
            if linked(&fs::symlink_metadata(ancestor).ok()?) {
                return None;
            }
        }
        let resolved = path.canonicalize().ok()?;
        if resolved == self.path || !resolved.starts_with(&self.path) {
            return None;
        }
        Some(resolved)
    }
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.path)
            .expect("verified local artifact")
            .to_string_lossy()
            .replace('\\', "/")
    }
    fn size(&self) -> io::Result<u64> {
        self.check()?;
        let mut directories = vec![self.path.clone()];
        let mut total = 0u64;
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if linked(&metadata) {
                    continue;
                }
                if metadata.is_dir() {
                    directories.push(entry.path());
                } else if metadata.is_file() {
                    total = total.saturating_add(metadata.len());
                }
            }
        }
        Ok(total)
    }
}

pub struct BuildLock(File);
impl BuildLock {
    pub fn acquire(target: &Root) -> io::Result<Self> {
        let path = target
            .path
            .parent()
            .unwrap()
            .join(".bevy-editor-build.lock");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| linked(&metadata)) {
            return Err(io::Error::other("Refusing to use a linked build lock"));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock().map_err(|error| io::Error::other(format!(
            "Another editor build is active. Wait for it to finish before starting another. ({error})"
        )))?;
        Ok(Self(file))
    }
}
impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct State {
    pub version: u32,
    pub snapshots: BTreeMap<String, BTreeSet<String>>,
    pub known: BTreeSet<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            snapshots: BTreeMap::new(),
            known: BTreeSet::new(),
        }
    }
}
impl State {
    pub fn load(root: &Root) -> io::Result<Self> {
        let path = root.path.join(INDEX);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
            Ok(metadata) if linked(&metadata) => {
                return Err(io::Error::other("Linked cache index; cleanup skipped"));
            }
            Ok(_) => {}
        }
        let mut state: Self = serde_json::from_slice(&fs::read(path)?)?;
        if state.version != 1 {
            return Err(io::Error::other(
                "Unrecognised cache index; cleanup skipped",
            ));
        }
        // Python's json.dumps inserts spaces in keys. Normalize old keys instead
        // of leaving a second permanent snapshot protecting superseded artifacts.
        let mut normalized = BTreeMap::<String, BTreeSet<String>>::new();
        for (key, files) in state.snapshots {
            let key = match serde_json::from_str::<[String; 3]>(&key) {
                Ok(parts) => serde_json::to_string(&parts)?,
                Err(_) => key,
            };
            normalized.entry(key).or_default().extend(files);
        }
        state.snapshots = normalized;
        Ok(state)
    }
    pub fn save(&self, root: &Root) -> io::Result<()> {
        root.check()?;
        let index = root.path.join(INDEX);
        if fs::symlink_metadata(&index).is_ok_and(|metadata| linked(&metadata)) {
            return Err(io::Error::other("Linked cache index; update skipped"));
        }
        let temp = root
            .path
            .join(format!("{INDEX}.{}.tmp", std::process::id()));
        let mut created = false;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            created = true;
            serde_json::to_writer_pretty(&mut file, self)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, index)
        })();
        if result.is_err()
            && created
            && fs::symlink_metadata(&temp).is_ok_and(|m| m.is_file() && !linked(&m))
        {
            let _ = fs::remove_file(temp);
        }
        result
    }
}

fn internal(root: &Root, file: &Path) -> bool {
    let relative = file.strip_prefix(&root.path).unwrap();
    relative.components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        name.starts_with('.') || matches!(name.as_ref(), "build" | "incremental" | "CACHEDIR.TAG")
    })
}

#[derive(Debug)]
pub struct Report {
    pub removed: u64,
    pub remaining: u64,
}

pub fn prune(root: &Root, state: &mut State, limit: u64) -> io::Result<Report> {
    // Finish the inventory before deleting anything. An unreadable subtree must
    // not turn a partial inventory into permission to remove unknown data.
    let mut total = root.size()?;
    let active: BTreeSet<_> = state
        .snapshots
        .values()
        .flatten()
        .filter_map(|name| {
            let name = Path::new(name);
            let path = if name.is_absolute() {
                name.to_owned()
            } else {
                root.path.join(name)
            };
            // Aliases in an old index may protect a file, but never grant deletion rights.
            let path = path.canonicalize().ok()?;
            (path.starts_with(&root.path) && path.is_file()).then_some(path)
        })
        .collect();
    let executable = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    let mut removed = 0;
    let mut seen = BTreeSet::new();
    for name in &state.known {
        if total <= limit {
            break;
        }
        let Some(file) = root.local_file(Path::new(name)) else {
            continue;
        };
        if active.contains(&file)
            || internal(root, &file)
            || executable.as_ref() == Some(&file)
            || !seen.insert(file.clone())
        {
            continue;
        }
        root.check()?;
        // Revalidate immediately before a file-only deletion, including containment.
        if root.local_file(&file).as_ref() != Some(&file) {
            continue;
        }
        let size = fs::metadata(&file)?.len();
        if let Err(error) = fs::remove_file(&file) {
            eprintln!("Cache: keeping {name}: {error}");
            continue;
        }
        total = total.saturating_sub(size);
        removed += size;
    }
    state
        .known
        .retain(|name| root.local_file(Path::new(name)).is_some());
    Ok(Report {
        removed,
        remaining: total,
    })
}
