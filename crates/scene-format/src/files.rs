//! Same-directory atomic replacement and optimistic revisions. OS locks serialize
//! cooperating writers; revision checks also catch external edits before commit.
//! A non-cooperating writer can still race the final check/rename on some OSes.
use crate::{Scene, content_digest};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revision {
    Missing,
    Content([u8; 32]),
}
impl Revision {
    pub fn of(bytes: &[u8]) -> Self {
        Self::Content(content_digest(bytes))
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        match read_limited(path, 512 * 1024 * 1024) {
            Ok(bytes) => Ok(Self::of(&bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::Missing),
            Err(error) => Err(error),
        }
    }
}

pub fn read_limited(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Scene document exceeds its size limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Scene document exceeds its size limit",
        ));
    }
    Ok(bytes)
}

struct Staged(PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8], expected: Revision) -> io::Result<Revision> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let locks = parent.join(".bevy-editor/scene-locks");
    fs::create_dir_all(&locks)?;
    let lock_name = blake3::hash(
        path.file_name()
            .ok_or_else(|| io::Error::other("Missing scene filename"))?
            .to_string_lossy()
            .as_bytes(),
    )
    .to_hex();
    // Keep the lock inode permanently: deleting it can split a queued lock into
    // two independent locks. A process crash releases the OS lock automatically.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locks.join(format!("{lock_name}.lock")))?;
    lock.try_lock().map_err(io::Error::from)?;
    let changed = || io::Error::other("Scene changed on disk; reload before saving");
    if Revision::read(path)? != expected {
        return Err(changed());
    }
    let stage = Staged(parent.join(format!(".scene-{}.tmp", uuid::Uuid::new_v4())));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage.0)?;
    file.write_all(bytes)?;
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "The scene file is read-only",
            ));
        }
        file.set_permissions(metadata.permissions())?;
    }
    file.sync_all()?;
    drop(file);
    if Revision::read(path)? != expected {
        return Err(changed());
    }
    fs::rename(&stage.0, path)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(Revision::of(bytes))
}

pub struct SourceDocument {
    pub scene: Scene,
    pub revision: Revision,
    path: PathBuf,
}
impl SourceDocument {
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = read_limited(&path, crate::MAX_SOURCE_BYTES)?;
        Ok(Self {
            scene: Scene::from_slice(&bytes)?,
            revision: Revision::of(&bytes),
            path,
        })
    }
    pub fn save(&mut self) -> crate::Result<()> {
        let bytes = self.scene.to_source()?;
        self.revision = atomic_write(&self.path, &bytes, self.revision)?;
        Ok(())
    }
}
