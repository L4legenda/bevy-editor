//! Offline subprocess execution, job snapshots, and explicit project import.
use super::{browser_files, generation::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "generation_native.rs"]
mod native;
pub(super) use native::{Runtime, Service};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Request {
    pub model: PathBuf,
    pub name: String,
    pub steps: u32,
    pub guidance: f32,
    pub resolution: u32,
    pub chunks: u32,
    pub seed: u64,
    pub simplify: bool,
    pub target_faces: u32,
    pub obj: bool,
}
impl Request {
    pub fn from_draft(draft: &Workspace, model: PathBuf) -> Result<Self, String> {
        browser_files::validate_name(draft.name.trim()).map_err(str::to_owned)?;
        if !(1..=100).contains(&draft.steps)
            || !draft.guidance.is_finite()
            || !(1.0..=20.0).contains(&draft.guidance)
            || !(16..=512).contains(&draft.resolution)
            || !(1000..=5_000_000).contains(&draft.chunks)
            || !(100..=1_000_000).contains(&draft.target_faces)
        {
            return Err("Проверьте параметры генерации".into());
        }
        use std::hash::BuildHasher;
        let seed = if draft.random_seed {
            std::collections::hash_map::RandomState::new().hash_one(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            )
        } else {
            draft.seed
        };
        Ok(Self {
            model,
            name: draft.name.trim().into(),
            steps: draft.steps,
            guidance: draft.guidance,
            resolution: draft.resolution,
            chunks: draft.chunks,
            seed,
            simplify: draft.simplify,
            target_faces: draft.target_faces,
            obj: draft.obj,
        })
    }
}

#[derive(Clone, Default, Debug)]
pub(super) struct Status {
    pub stage: String,
    pub current: u32,
    pub total: u32,
    pub error: Option<String>,
}
#[derive(Default)]
pub(super) struct Progress {
    pub cancelled: AtomicBool,
    pub status: Mutex<Status>,
    session: Mutex<Option<std::sync::Weak<native::Session>>>,
}
impl Progress {
    pub fn shutdown(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(session) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.upgrade())
        {
            session.shutdown();
        }
    }
    pub fn snapshot(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn event(&self, line: &str) {
        let Some(line) = line.strip_prefix("BEVY_HUNYUAN ") else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let mut state = self.status.lock().unwrap_or_else(|e| e.into_inner());
        match value["event"].as_str() {
            Some("progress") => {
                state.stage = value["stage"].as_str().unwrap_or_default().into();
                state.current = value["current"].as_u64().unwrap_or(0) as u32;
                state.total = value["total"].as_u64().unwrap_or(0) as u32;
            }
            Some("error") => state.error = value["message"].as_str().map(str::to_owned),
            Some("complete") => {
                state.stage = "complete".into();
                state.current = 0;
                state.total = 0;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Deserialize, Debug)]
pub(super) struct ResultModel {
    #[serde(skip)]
    pub directory: PathBuf,
    pub name: String,
    pub seed: u64,
    pub steps: u32,
    pub guidance: f32,
    pub resolution: u32,
    pub vertices: usize,
    pub faces: usize,
    pub raw_faces: usize,
    pub seconds: f64,
    pub obj: bool,
}
impl ResultModel {
    pub fn glb(&self) -> PathBuf {
        self.directory.join("mesh.glb")
    }
}

pub(super) fn run(
    runtime: &Service,
    request: &Request,
    source: &Path,
    cache: &Path,
    progress: Arc<Progress>,
) -> Result<ResultModel, String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let id = format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    );
    let directory = cache.join(".jobs").join(id);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    // A fixed input snapshot makes the job independent of later image edits.
    let size = fs::metadata(source)
        .map_err(|e| format!("Не удалось прочитать изображение: {e}"))?
        .len();
    if size > 128 * 1024 * 1024 {
        return Err("Изображение превышает 128 МБ".into());
    }
    fs::copy(source, directory.join("input")).map_err(|e| e.to_string())?;
    fs::write(
        directory.join("request.json"),
        serde_json::to_vec_pretty(request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    runtime.generate(&directory, &progress)
}

pub(super) fn import(result: &ResultModel, root: &Path) -> Result<PathBuf, String> {
    browser_files::validate_name(&result.name).map_err(str::to_owned)?;
    super::model_document::glb::Glb::load(&result.glb())?;
    if result.obj && !result.directory.join("mesh.obj").is_file() {
        return Err("В результате отсутствует OBJ".into());
    }
    let parent = Path::new("models/generated");
    // Validate each existing ancestor before creating anything below it.
    let mut relative = PathBuf::new();
    for name in ["models", "generated"] {
        let current = browser_files::directory(root, &relative).map_err(|e| e.to_string())?;
        match fs::create_dir(current.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        relative.push(name);
    }
    let directory = browser_files::directory(root, parent).map_err(|e| e.to_string())?;
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            result.name.clone()
        } else {
            format!("{} ({suffix})", result.name)
        };
        let path = directory.join(format!("{name}.glb"));
        let mut target = match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        let obj_path = directory.join(format!("{name}.obj"));
        let mut obj_target = if result.obj {
            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&obj_path)
            {
                Ok(file) => Some(file),
                Err(error) => {
                    drop(target);
                    let _ = fs::remove_file(&path);
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(error.to_string());
                }
            }
        } else {
            None
        };
        let write = (|| {
            let mut source = fs::File::open(result.glb())?;
            std::io::copy(&mut source, &mut target)?;
            target.sync_all()?;
            if let Some(target) = obj_target.as_mut() {
                let mut source = fs::File::open(result.directory.join("mesh.obj"))?;
                std::io::copy(&mut source, target)?;
                target.sync_all()?;
            }
            Ok::<(), std::io::Error>(())
        })();
        if let Err(e) = write {
            drop(target);
            drop(obj_target);
            let _ = fs::remove_file(&path);
            if result.obj {
                let _ = fs::remove_file(&obj_path);
            }
            return Err(format!("Не удалось добавить модель: {e}"));
        }
        return Ok(parent.join(format!("{name}.glb")));
    }
    Err("Не удалось подобрать свободное имя модели".into())
}

#[cfg(test)]
#[path = "generation_backend_tests.rs"]
mod tests;
