use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
pub struct Sandbox {
    pub path: PathBuf,
}
impl Sandbox {
    pub fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let name = format!(
            "bevy-cargo-cache-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let path = temp.join(name);
        fs::create_dir(&path).unwrap();
        Self { path }
    }
    pub fn file(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        // Every recursive test cleanup is confined to this explicitly created
        // absolute sandbox; no workspace, shared target, or user asset is removed.
        let Ok(temp) = std::env::temp_dir().canonicalize() else {
            return;
        };
        if self.path.parent() == Some(temp.as_path())
            && self
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("bevy-cargo-cache-")
            && self.path.canonicalize().ok().as_ref() == Some(&self.path)
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
