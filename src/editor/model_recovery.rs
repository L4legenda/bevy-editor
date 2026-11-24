//! Crash recovery for editable GLBs. Copies keep the source model untouched.
use super::{Document, Object, document::glb::Glb};
use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

const DEFAULT_EDITS: u16 = 10;
pub(in crate::editor) const MAX_EDITS: u16 = 500;
const INTERVAL: Duration = Duration::from_secs(15);
const MAX_GLB: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::editor) struct Settings {
    pub enabled: bool,
    pub edits: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            edits: DEFAULT_EDITS,
        }
    }
}

#[derive(Resource)]
pub(in crate::editor) struct Preferences {
    path: Option<PathBuf>,
    settings: Settings,
    can_publish: Arc<Mutex<bool>>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self::open(
            super::super::paint::presets::directory().and_then(|directory| {
                directory
                    .parent()
                    .map(|root| root.join("model-recovery.json"))
            }),
        )
    }
}

impl Preferences {
    fn open(path: Option<PathBuf>) -> Self {
        let mut settings = Settings::default();
        if let Some(file) = path.as_deref() {
            let load = || -> io::Result<Settings> {
                Ok(serde_json::from_slice(&bevy_scene_format::read_limited(
                    file, 4096,
                )?)?)
            };
            match load() {
                Ok(saved) => settings = saved,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => warn!("Could not read model recovery preferences: {error}"),
            }
        }
        settings.edits = settings.edits.clamp(1, MAX_EDITS);
        Self {
            path,
            settings,
            can_publish: Arc::new(Mutex::new(settings.enabled)),
        }
    }

    pub(in crate::editor) fn settings(&self) -> Settings {
        self.settings
    }

    pub(in crate::editor) fn save(&mut self, mut settings: Settings) -> io::Result<()> {
        settings.edits = settings.edits.clamp(1, MAX_EDITS);
        if settings == self.settings {
            return Ok(());
        }
        let mut can_publish = self
            .can_publish
            .lock()
            .map_err(|_| io::Error::other("Настройки автокопий недоступны"))?;
        if let Some(file) = &self.path {
            bevy_scene_format::atomic_write(
                file,
                &serde_json::to_vec(&settings)?,
                bevy_scene_format::Revision::read(file)?,
            )?;
        }
        *can_publish = settings.enabled;
        self.settings = settings;
        Ok(())
    }
}

fn directory(root: &Path) -> PathBuf {
    root.join(".bevy-editor/model-recovery")
}

fn snapshot_name(relative: &Path) -> String {
    let mut hash = blake3::Hasher::new();
    for component in relative.components() {
        let name = component.as_os_str().as_encoded_bytes();
        hash.update(&(name.len() as u64).to_le_bytes());
        hash.update(name);
    }
    format!("{}.glb", hash.finalize().to_hex())
}

struct Session {
    folder: PathBuf,
    lock: Option<File>,
}

impl Session {
    fn open(root: &Path) -> io::Result<Self> {
        let folder = directory(root).join(bevy::asset::uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&folder)?;
        let lock = File::create(folder.join("session.lock"))?;
        lock.try_lock().map_err(io::Error::from)?;
        Ok(Self {
            folder,
            lock: Some(lock),
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return;
        };
        if entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() != "session.lock")
        {
            return;
        }
        drop(self.lock.take());
        let _ = fs::remove_file(self.folder.join("session.lock"));
        let _ = fs::remove_dir(&self.folder);
    }
}

struct Snapshot {
    source: Glb,
    objects: Vec<Object>,
    original: Vec<Object>,
    custom: Arc<Vec<crate::editor::model_paint_data::custom::Recipe>>,
    settings: crate::editor::model_bake_settings::Settings,
    cage: Arc<crate::editor::model_cage_data::Edits>,
}

impl Snapshot {
    fn capture(doc: &Document) -> Self {
        Self {
            source: doc.source.clone(),
            objects: doc.objects.clone(),
            original: doc.original.clone(),
            custom: doc.custom_bakes.clone(),
            settings: doc.bake_settings.clone(),
            cage: doc.cage_edits.clone(),
        }
    }

    fn encode(mut self) -> Result<Vec<u8>, String> {
        super::write_model_metadata(&mut self.source, &self.custom, &self.settings)?;
        self.source
            .encode_with_cage(&self.objects, &self.original, Some(&self.cage))
    }
}

// The worker stages bytes before locking, so cancel never waits for encoding or disk writes.
struct CopySlot {
    path: PathBuf,
    relative: PathBuf,
    cancelled: Mutex<bool>,
}

impl CopySlot {
    fn publish(&self, bytes: &[u8], enabled: &Mutex<bool>) -> io::Result<bool> {
        if bytes.len() > MAX_GLB {
            return Err(io::Error::other(
                "Копия превышает предел редактируемого GLB",
            ));
        }
        let stage = TempFile::create(self.path.parent().unwrap())?;
        let mut file = fs::OpenOptions::new().write(true).open(&stage.0)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let cancelled = self
            .cancelled
            .lock()
            .map_err(|_| io::Error::other("Не удалось завершить запись автокопии"))?;
        let enabled = enabled
            .lock()
            .map_err(|_| io::Error::other("Настройки автокопий недоступны"))?;
        if !*cancelled && *enabled {
            let index = self.path.with_extension("json");
            if !index.exists() {
                let staged_index = TempFile::create(self.path.parent().unwrap())?;
                let mut file = fs::OpenOptions::new().write(true).open(&staged_index.0)?;
                serde_json::to_writer(&mut file, &self.relative)?;
                file.sync_all()?;
                drop(file);
                fs::rename(&staged_index.0, index)?;
            }
            fs::rename(&stage.0, &self.path)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn cancel(&self) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            *cancelled = true;
            for path in [&self.path, &self.path.with_extension("json")] {
                if let Err(error) = fs::remove_file(path)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    warn!("Could not remove model recovery copy: {error}");
                }
            }
        }
    }
}

struct TempFile(PathBuf);

impl TempFile {
    fn create(folder: &Path) -> io::Result<Self> {
        let path = folder.join(format!(
            ".bevy-editor-recovery-{}.tmp",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        File::create_new(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct Entry {
    published: u64,
    observed: u64,
    last_edit: Instant,
    slot: Arc<CopySlot>,
}

struct Job {
    source: PathBuf,
    edits: u64,
    task: Task<Result<bool, String>>,
}

#[derive(Default)]
pub(super) struct Store {
    session: Option<Session>,
    entries: HashMap<PathBuf, Entry>,
    job: Option<Job>,
    next_poll: Option<Instant>,
    #[cfg(test)]
    enabled: bool,
}

impl Store {
    pub(super) fn due(&self) -> bool {
        #[cfg(test)]
        if !self.enabled {
            return false;
        }
        self.next_poll.is_none_or(|next| next <= Instant::now())
    }

    pub(super) fn clear(&mut self, source: &Path) {
        if let Some(entry) = self.entries.remove(source) {
            entry.slot.cancel();
        }
    }

    pub(super) fn clear_session(&mut self) {
        for (_, entry) in self.entries.drain() {
            entry.slot.cancel();
        }
    }

    pub(super) fn update<'a>(
        &mut self,
        root: &Path,
        preferences: &Preferences,
        documents: impl Iterator<Item = &'a Document>,
    ) {
        if !self.due() {
            return;
        }
        let now = Instant::now();
        self.next_poll = Some(now + Duration::from_secs(1));
        if let Some(job) = &mut self.job {
            let Some(result) = block_on(poll_once(&mut job.task)) else {
                return;
            };
            match result {
                Ok(true) => {
                    if let Some(entry) = self.entries.get_mut(&job.source) {
                        entry.published = job.edits;
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    warn!("Could not save model recovery copy: {error}");
                    self.next_poll = Some(now + INTERVAL);
                    self.job = None;
                    return;
                }
            }
            self.job = None;
        }
        for doc in documents.filter(|doc| !doc.paint_busy && !doc.paint_pending) {
            let path = &doc.source.path;
            if !doc.dirty() {
                self.clear(path);
                continue;
            }
            if !preferences.settings.enabled {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if self.session.is_none() {
                match Session::open(root) {
                    Ok(session) => self.session = Some(session),
                    Err(error) => {
                        warn!("Could not open model recovery session: {error}");
                        self.next_poll = Some(now + INTERVAL);
                        return;
                    }
                }
            }
            let folder = &self.session.as_ref().unwrap().folder;
            let entry = self.entries.entry(path.clone()).or_insert_with(|| Entry {
                published: doc.saved_recovery_edits,
                observed: doc.recovery_edits,
                last_edit: now,
                slot: Arc::new(CopySlot {
                    path: folder.join(snapshot_name(relative)),
                    relative: relative.to_owned(),
                    cancelled: Mutex::new(false),
                }),
            });
            if entry.observed != doc.recovery_edits {
                entry.observed = doc.recovery_edits;
                entry.last_edit = now;
            }
            let edits = doc
                .recovery_edits
                .saturating_sub(entry.published.max(doc.saved_recovery_edits));
            if edits == 0
                || (edits < u64::from(preferences.settings.edits)
                    && now - entry.last_edit < INTERVAL)
            {
                continue;
            }
            let snapshot = Snapshot::capture(doc);
            let slot = entry.slot.clone();
            let enabled = preferences.can_publish.clone();
            self.job = Some(Job {
                source: path.clone(),
                edits: doc.recovery_edits,
                task: AsyncComputeTaskPool::get().spawn(async move {
                    slot.publish(&snapshot.encode()?, &enabled)
                        .map_err(|error| error.to_string())
                }),
            });
            break;
        }
    }
}

#[cfg(test)]
pub(in crate::editor) fn seed_abandoned_copy(root: &Path, relative: &Path, bytes: &[u8]) {
    let session = Session::open(root).unwrap();
    let slot = CopySlot {
        path: session.folder.join(snapshot_name(relative)),
        relative: relative.to_owned(),
        cancelled: Mutex::new(false),
    };
    slot.publish(bytes, &Mutex::new(true)).unwrap();
}

fn candidates(root: &Path, relative: &Path) -> Vec<(SystemTime, PathBuf)> {
    let name = snapshot_name(relative);
    let mut copies: Vec<_> = fs::read_dir(directory(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let path = entry.path().join(&name);
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    copies.sort_unstable_by(|a, b| b.cmp(a));
    copies
}

fn try_session_lock(folder: &Path) -> io::Result<File> {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(folder.join("session.lock"))?;
    lock.try_lock().map_err(io::Error::from)?;
    Ok(lock)
}

#[derive(Debug)]
pub(in crate::editor) struct Offer {
    pub relative: PathBuf,
    snapshot: PathBuf,
}

impl Offer {
    pub(in crate::editor) fn restore(self, root: &Path) -> Result<PathBuf, String> {
        let _lock =
            try_session_lock(self.snapshot.parent().unwrap()).map_err(|error| error.to_string())?;
        let restored = restore_copy(root, &self.relative, &self.snapshot)?;
        if let Err(error) = self.delete_copy() {
            warn!("Could not consume restored model snapshot: {error}");
        }
        Ok(restored)
    }

    pub(in crate::editor) fn discard(self) -> io::Result<()> {
        let _lock = try_session_lock(self.snapshot.parent().unwrap())?;
        self.delete_copy()
    }

    fn delete_copy(&self) -> io::Result<()> {
        fs::remove_file(&self.snapshot)?;
        let _ = fs::remove_file(self.snapshot.with_extension("json"));
        Ok(())
    }
}

pub(in crate::editor) fn startup_offer(root: &Path, known_models: &[PathBuf]) -> Option<Offer> {
    let known: HashMap<_, _> = known_models
        .iter()
        .map(|relative| (snapshot_name(relative), relative))
        .collect();
    let mut newest = None;
    for session in fs::read_dir(directory(root)).ok()?.flatten() {
        if !session.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let folder = session.path();
        let Ok(_lock) = try_session_lock(&folder) else {
            continue;
        };
        let Ok(files) = fs::read_dir(&folder) else {
            continue;
        };
        for file in files
            .flatten()
            .filter(|file| file.path().extension().is_some_and(|ext| ext == "glb"))
        {
            let snapshot = file.path();
            let index = snapshot.with_extension("json");
            let relative = match bevy_scene_format::read_limited(&index, 16 * 1024) {
                Ok(bytes) => serde_json::from_slice::<PathBuf>(&bytes).ok(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => snapshot
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| known.get(name))
                    .map(|path| (*path).clone()),
                Err(_) => None,
            };
            let Some(relative) = relative else { continue };
            if !relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
                || !relative
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
                || !snapshot
                    .file_name()
                    .is_some_and(|name| name == snapshot_name(&relative).as_str())
            {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&snapshot) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_GLB as u64 {
                continue;
            }
            let Ok(time) = metadata.modified() else {
                continue;
            };
            if newest.as_ref().is_none_or(|(previous, _)| time > *previous) {
                newest = Some((time, Offer { relative, snapshot }));
            }
        }
    }
    newest.map(|(_, offer)| offer)
}

pub(in crate::editor) fn restore(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    for (_, copy) in candidates(root, relative) {
        if let Ok(_lock) = try_session_lock(copy.parent().unwrap()) {
            return restore_copy(root, relative, &copy);
        }
    }
    Err("Нет копии из завершившегося сеанса".into())
}

fn restore_copy(root: &Path, relative: &Path, copy: &Path) -> Result<PathBuf, String> {
    use crate::editor::{browser_files, file_moves};
    if !relative
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
    {
        return Err("Черновики восстановления поддерживают модели GLB".into());
    }
    let folder = relative.parent().ok_or("У модели нет папки")?;
    let parent = file_moves::checked_path(root, folder).map_err(|error| error.to_string())?;
    let mut stem = String::new();
    let mut length = 0;
    for ch in relative
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
    {
        length += ch.len_utf16();
        if length > 140 {
            break;
        }
        stem.push(ch);
    }
    let stage = TempFile::create(&parent).map_err(|error| error.to_string())?;
    fs::copy(copy, &stage.0).map_err(|error| error.to_string())?;
    fs::OpenOptions::new()
        .write(true)
        .open(&stage.0)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())?;
    for number in 1..=1000 {
        let suffix = if number == 1 {
            String::new()
        } else {
            format!(" ({number})")
        };
        let name = format!("{stem} — восстановлено{suffix}.glb");
        browser_files::validate_name(&name)?;
        match file_moves::move_no_replace(&stage.0, &parent.join(&name)) {
            Ok(()) => return Ok(relative.with_file_name(name)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Не удалось подобрать имя для восстановленной модели".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        model_document::MaterialSlot,
        model_paint_data::{Stack, Surface, solid},
        model_primitives::Primitive,
    };

    #[test]
    fn model_recovery_reopens_committed_layers_from_an_abandoned_session() {
        AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let root = std::env::temp_dir().join(format!(
            "bevy-recovery-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let name = Path::new("sample.glb");
        let original_path = root.join(name);
        let mut object = Object {
            id: 0,
            source_node: None,
            name: "Sample".into(),
            geometry: Primitive::Cube
                .geometry(&Primitive::Cube.defaults())
                .unwrap()
                .into(),
            transform: Transform::IDENTITY,
            materials: vec![MaterialSlot {
                paint: None,
                authored: None,
                material: default(),
                bake_fixes: default(),
                source: serde_json::json!({"material":0}),
            }],
            hidden: false,
        };
        let original = Glb::new_asset(std::slice::from_ref(&object)).unwrap();
        fs::write(&original_path, &original).unwrap();
        object.source_node = Some(0);
        let mut doc = Document::new(Glb::load(&original_path).unwrap(), vec![object]);
        let mut store = Store {
            enabled: true,
            ..default()
        };
        let mut preferences = Preferences::open(Some(root.join("preferences.json")));
        assert!(!doc.dirty());
        let mut stack = Stack::default();
        stack.layers[0].name = "Несохранённый мазок".into();
        stack.layers[0].channels[0] = Some(solid(16, [210, 72, 23, 255]));
        doc.objects[0].materials[0].paint = Some(Arc::new(Surface {
            size: 16,
            base: std::array::from_fn(|_| solid(16, [128; 4])),
            stack,
            material: serde_json::Value::Null,
            baked: None,
        }));
        doc.recovery_edits = u64::from(DEFAULT_EDITS - 1);
        doc.revision = 1000;
        doc.paint_busy = true;
        store.update(&root, &preferences, std::iter::once(&doc));
        assert!(store.job.is_none());
        doc.paint_busy = false;
        store.next_poll = None;
        store.update(&root, &preferences, std::iter::once(&doc));
        assert!(store.job.is_none(), "preview revisions are not edits");
        preferences
            .save(Settings {
                enabled: false,
                edits: DEFAULT_EDITS - 1,
            })
            .unwrap();
        store.next_poll = None;
        store.update(&root, &preferences, std::iter::once(&doc));
        assert!(store.job.is_none());
        assert_eq!(
            Preferences::open(Some(root.join("preferences.json"))).settings(),
            preferences.settings()
        );
        preferences
            .save(Settings {
                enabled: true,
                ..preferences.settings()
            })
            .unwrap();
        store.next_poll = None;
        store.update(&root, &preferences, std::iter::once(&doc));
        let job = store.job.take().unwrap();
        assert!(block_on(job.task).unwrap());
        let existing = &store.entries[&original_path].slot;
        let bytes = fs::read(&existing.path).unwrap();
        preferences
            .save(Settings {
                enabled: false,
                ..preferences.settings()
            })
            .unwrap();
        assert!(
            !existing
                .publish(b"queued snapshot", &preferences.can_publish)
                .unwrap()
        );
        assert_eq!(fs::read(&existing.path).unwrap(), bytes);
        assert!(
            restore(&root, name).is_err(),
            "a live session owns its copy"
        );
        assert!(startup_offer(&root, &[]).is_none());
        assert_eq!(fs::read(&original_path).unwrap(), original);
        drop(store);

        let result = restore(&root, name).unwrap();
        let glb = Glb::load(&root.join(&result)).unwrap();
        let primitive = glb.json["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
            .find_map(|primitive| glb.paint_surface(primitive).unwrap())
            .unwrap();
        assert_eq!(primitive.stack.layers[0].name, "Несохранённый мазок");
        assert_eq!(
            &primitive.stack.layers[0].channels[0].as_ref().unwrap()[..4],
            &[210, 72, 23, 255]
        );
        assert_ne!(restore(&root, name).unwrap(), result);
        assert_eq!(fs::read(&original_path).unwrap(), original);

        let mut live = Store {
            enabled: true,
            ..default()
        };
        preferences
            .save(Settings {
                enabled: true,
                edits: MAX_EDITS,
            })
            .unwrap();
        live.update(&root, &preferences, std::iter::once(&doc));
        assert!(live.job.is_none());
        live.entries.get_mut(&original_path).unwrap().last_edit = Instant::now() - INTERVAL;
        live.next_poll = None;
        live.update(&root, &preferences, std::iter::once(&doc));
        let latest = live.entries[&original_path].slot.clone();
        live.clear(&original_path);
        assert!(!block_on(live.job.take().unwrap().task).unwrap());
        assert!(!latest.path.exists(), "discard cancels a background write");
        drop(live);

        let offer = startup_offer(&root, &[]).unwrap();
        assert_eq!(offer.relative, name);
        fs::remove_file(&original_path).unwrap();
        let restored = offer.restore(&root).unwrap();
        assert!(root.join(restored).is_file());
        assert!(!original_path.exists());
        assert!(startup_offer(&root, &[]).is_none());
        seed_abandoned_copy(&root, name, &original);
        startup_offer(&root, &[]).unwrap().discard().unwrap();
        assert!(startup_offer(&root, &[]).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
