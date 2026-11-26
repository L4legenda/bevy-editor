use super::*;
use bevy_scene_format::{Revision, atomic_write};
use std::{collections::HashSet, time::UNIX_EPOCH};

pub(super) const CACHE: &str = ".bevy-editor/asset-stamps.json";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    modified: u128,
    identity: Option<String>,
}
#[cfg(windows)]
fn file_key(path: &Path, _meta: &fs::Metadata) -> io::Result<Option<String>> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_READ_ATTRIBUTES, GetFileInformationByHandle,
    };
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .open(path)?;
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The owned handle outlives the call and the OS initializes info on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let i = unsafe { info.assume_init() };
    Ok(((i.nFileIndexHigh, i.nFileIndexLow) != (0, 0)).then(|| {
        format!(
            "{}:{}:{}:{}:{}",
            i.dwVolumeSerialNumber,
            i.nFileIndexHigh,
            i.nFileIndexLow,
            i.ftCreationTime.dwHighDateTime,
            i.ftCreationTime.dwLowDateTime
        )
    }))
}
#[cfg(unix)]
fn file_key(_path: &Path, m: &fs::Metadata) -> io::Result<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    Ok(m.created()
        .ok()
        .map(|birth| format!("{}:{}:{birth:?}", m.dev(), m.ino())))
}
#[cfg(not(any(windows, unix)))]
fn file_key(_path: &Path, _meta: &fs::Metadata) -> io::Result<Option<String>> {
    Ok(None)
}
fn stamp(path: &Path, m: &fs::Metadata) -> io::Result<Stamp> {
    Ok(Stamp {
        size: m.len(),
        modified: m
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        identity: file_key(path, m)?,
    })
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Seen {
    stamp: Stamp,
    hash: String,
}
type Inventory = BTreeMap<String, Seen>;

#[derive(Default)]
pub(crate) struct Changes {
    pub modified: Vec<String>,
    pub moves: Vec<(String, String)>,
    pub companions: Vec<(String, String)>,
    pub refresh: bool,
    pub hashed: u64,
}
fn skipped(path: &Path) -> bool {
    path.components().any(|part| {
        let n = part.as_os_str().to_string_lossy();
        n.starts_with('.')
            || matches!(
                n.as_ref(),
                "target" | "build" | "node_modules" | "dist" | "__pycache__"
            )
    })
}
pub(crate) fn relevant(relative: &Path) -> bool {
    !skipped(relative) && !relative.as_os_str().is_empty()
}
fn format(path: &str) -> String {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "jpeg" { "jpg".into() } else { ext }
}
fn inventory(
    root: &Path,
    old: &Inventory,
    scopes: Option<&BTreeSet<PathBuf>>,
    initial: bool,
    stats: &mut Changes,
) -> io::Result<Inventory> {
    let mut found = if scopes.is_some() && !initial {
        old.clone()
    } else {
        Inventory::new()
    };
    let mut pending = vec![];
    if let Some(paths) = scopes {
        for rel in paths {
            if !relevant(rel) {
                continue;
            }
            // A directory event covers its descendants. Removing by ordered prefix avoids
            // walking every unrelated record for every newly created file in a batch.
            if rel.ancestors().skip(1).any(|p| paths.contains(p)) {
                continue;
            }
            if initial && !supported(&rel.to_string_lossy()) {
                continue;
            }
            if !initial {
                let name = rel
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                found.remove(&name);
                let prefix = format!("{name}/");
                let children: Vec<_> = found
                    .range(prefix.clone()..)
                    .take_while(|(p, _)| p.starts_with(&prefix))
                    .map(|(p, _)| p.clone())
                    .collect();
                for child in children {
                    found.remove(&child);
                }
            }
            if let Ok(physical) = root.join(rel).canonicalize()
                && !physical.starts_with(root)
            {
                continue;
            }
            pending.push(rel.clone());
        }
    } else {
        pending.push(PathBuf::new())
    }
    let mut keys: HashMap<&str, Option<&Seen>> = HashMap::new();
    for s in old.values() {
        if let Some(k) = s.stamp.identity.as_deref() {
            keys.entry(k).and_modify(|v| *v = None).or_insert(Some(s));
        }
    }
    while let Some(rel) = pending.pop() {
        let abs = root.join(&rel);
        let m = match fs::symlink_metadata(&abs) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if m.file_type().is_symlink() {
            continue;
        }
        if m.is_dir() {
            for entry in fs::read_dir(abs)? {
                let path = rel.join(entry?.file_name());
                if !skipped(&path) {
                    pending.push(path)
                }
            }
            continue;
        }
        let Some(name) = rel.to_str().map(|p| p.replace('\\', "/")) else {
            continue;
        };
        if !m.is_file() || !portable(&name) || !supported(&name) {
            continue;
        }
        let stamp_before = stamp(&abs, &m)?;
        let cached = old
            .get(&name)
            .filter(|p| p.stamp == stamp_before)
            .or_else(|| {
                keys.get(stamp_before.identity.as_deref()?)
                    .copied()
                    .flatten()
                    .filter(|p| p.stamp == stamp_before)
            });
        let item = if let Some(cached) = cached {
            cached.clone()
        } else {
            let mut hasher = blake3::Hasher::new();
            let bytes = io::copy(&mut fs::File::open(&abs)?, &mut hasher)?;
            if bytes != stamp_before.size || stamp(&abs, &fs::metadata(&abs)?)? != stamp_before {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Asset changed during indexing",
                ));
            }
            stats.hashed += bytes;
            Seen {
                stamp: stamp_before,
                hash: hasher.finalize().to_hex().to_string(),
            }
        };
        found.insert(name, item);
    }
    Ok(found)
}
fn unique_by<K: std::hash::Hash + Eq>(
    pairs: impl Iterator<Item = (K, String)>,
) -> HashMap<K, Option<String>> {
    let mut map = HashMap::new();
    for (k, v) in pairs {
        map.entry(k).and_modify(|v| *v = None).or_insert(Some(v));
    }
    map
}
fn assign(
    manifest: &Manifest,
    old: &Inventory,
    now: &Inventory,
    hints: &[(String, String)],
) -> HashMap<String, String> {
    let mut assignments = HashMap::new();
    let mut used = HashSet::new();
    // Keep an unchanged occupant first (also disambiguates hard links and equal copies).
    for (id, r) in &manifest.assets {
        if !r.missing
            && let Some(seen) = now.get(&r.path)
            && old.get(&r.path).is_some_and(|s| {
                s.stamp.identity == seen.stamp.identity
                    && s.hash == r.hash
                    && s.stamp.identity.is_some()
            })
        {
            assignments.insert(r.path.clone(), id.clone());
            used.insert(id.clone());
        }
    }
    // Replay only paired rename notifications. Atomic-save temporary names never enter
    // this map. Unpaired create/remove notifications require identity or digest evidence.
    if !hints.is_empty() {
        let mut routes: HashMap<&str, &str> = manifest
            .assets
            .iter()
            .filter(|(_, r)| !r.missing)
            .map(|(id, r)| (r.path.as_str(), id.as_str()))
            .collect();
        for (from, to) in hints {
            if format(from) == format(to)
                && let Some(id) = routes.remove(from.as_str())
            {
                routes.insert(to, id);
            }
        }
        for (path, id) in routes {
            if manifest.assets[id].path != path
                && now.contains_key(path)
                && !used.contains(id)
                && !assignments.contains_key(path)
            {
                assignments.insert(path.into(), id.into());
                used.insert(id.into());
            }
        }
    }
    // Identity on this volume takes precedence over names. Without a cache only
    // a one-to-one full digest match may infer a move.
    for by_identity in [true, false] {
        let before = unique_by(
            manifest
                .assets
                .iter()
                .filter(|(id, _)| !used.contains(*id))
                .filter_map(|(id, r)| {
                    let key = if by_identity {
                        let s = old.get(&r.path)?;
                        if r.missing || s.hash != r.hash {
                            return None;
                        }
                        s.stamp.identity.clone()?
                    } else {
                        format!("{}:{}", r.size, r.hash)
                    };
                    Some(((format(&r.path), key), id.clone()))
                }),
        );
        let after = unique_by(
            now.iter()
                .filter(|(path, _)| !assignments.contains_key(*path))
                .filter_map(|(path, s)| {
                    let key = if by_identity {
                        s.stamp.identity.clone()?
                    } else {
                        format!("{}:{}", s.stamp.size, s.hash)
                    };
                    Some(((format(path), key), path.clone()))
                }),
        );
        for (key, path) in after {
            if let (Some(path), Some(Some(id))) = (path, before.get(&key)) {
                assignments.insert(path, id.clone());
                used.insert(id.clone());
            }
        }
        if by_identity {
            // The current path and digest survive a clean checkout, including identical copies.
            for (id, r) in &manifest.assets {
                if !r.missing
                    && !used.contains(id)
                    && !assignments.contains_key(&r.path)
                    && now.get(&r.path).is_some_and(|s| s.hash == r.hash)
                {
                    assignments.insert(r.path.clone(), id.clone());
                    used.insert(id.clone());
                }
            }
        }
    }
    for (id, r) in &manifest.assets {
        if !r.missing
            && !used.contains(id)
            && now.contains_key(&r.path)
            && !assignments.contains_key(&r.path)
        {
            assignments.insert(r.path.clone(), id.clone());
        }
    }
    assignments
}

/// Atomic image/GLB saves replace the filesystem object. Persist the new evidence before
/// reporting the save complete, even if Explorer moves the result before the watcher wakes.
pub(crate) fn record_save(root: &Path, absolute: &Path) -> io::Result<()> {
    let relative = absolute
        .strip_prefix(root)
        .map_err(|_| invalid("Saved asset outside project"))?;
    let path = relative
        .to_str()
        .ok_or_else(|| invalid("Invalid asset path"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let index = super::index(root)?;
    let Some(uid) = index.uid(&path) else {
        return Ok(());
    };
    let files = inventory(
        root,
        &Inventory::new(),
        Some(&[relative.to_owned()].into_iter().collect()),
        true,
        &mut Changes::default(),
    )?;
    let seen = files
        .get(&path)
        .ok_or_else(|| invalid("Saved asset disappeared before indexing"))?;
    let mut scanner = Scanner::new(root)?;
    for attempt in 0..4 {
        let (mut manifest, rev) = read_manifest(root)?;
        let Some(record) = manifest.assets.get_mut(uid) else {
            return Ok(());
        };
        if record.path != path || record.missing {
            return Ok(());
        }
        let dirty = record.size != seen.stamp.size || record.hash != seen.hash;
        record.size = seen.stamp.size;
        record.hash.clone_from(&seen.hash);
        match scanner.publish(manifest, rev, dirty) {
            Ok(()) => {
                // The portable hash is sufficient without the cache. An unchanged copy can
                // also be distinguished locally by the newly saved filesystem identity.
                let cache = root.join(CACHE);
                let bytes = bevy_scene_format::read_limited(&cache, 128 << 20).unwrap_or_default();
                let mut stamps: Inventory = serde_json::from_slice(&bytes).unwrap_or_default();
                stamps.insert(path.clone(), seen.clone());
                let expected = if bytes.is_empty() {
                    Revision::Missing
                } else {
                    Revision::of(&bytes)
                };
                let _ = atomic_write(&cache, &serde_json::to_vec(&stamps)?, expected);
                return Ok(());
            }
            Err(_) if attempt < 3 => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

pub(crate) struct Scanner {
    root: PathBuf,
    old: Inventory,
    initialized: bool,
    published: Arc<Index>,
}
impl Scanner {
    pub(crate) fn new(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let old = bevy_scene_format::read_limited(&root.join(CACHE), 128 << 20)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let published = index(&root)?;
        Ok(Self {
            root,
            published,
            old,
            initialized: false,
        })
    }
    pub(crate) fn initial(&mut self, paths: &BTreeSet<PathBuf>) -> io::Result<Changes> {
        self.reconcile(Some(paths), true, false, &[])
    }
    pub(crate) fn run(&mut self, scopes: Option<&BTreeSet<PathBuf>>) -> io::Result<Changes> {
        self.reconcile(scopes.filter(|_| self.initialized), false, false, &[])
    }
    pub(crate) fn first_import(&mut self, paths: &BTreeSet<PathBuf>) -> io::Result<Changes> {
        self.reconcile(Some(paths), true, true, &[])
    }
    pub(crate) fn events(
        &mut self,
        scopes: Option<&BTreeSet<PathBuf>>,
        moves: &[(String, String)],
    ) -> io::Result<Changes> {
        self.reconcile(scopes.filter(|_| self.initialized), false, false, moves)
    }
    fn reconcile(
        &mut self,
        scopes: Option<&BTreeSet<PathBuf>>,
        initial: bool,
        only_empty: bool,
        hints: &[(String, String)],
    ) -> io::Result<Changes> {
        let mut changes = Changes::default();
        let (mut manifest, rev) = read_manifest(&self.root)?;
        if only_empty && rev != Revision::Missing {
            return Ok(changes);
        }
        let changed_on_disk = manifest != self.published.manifest;
        if changed_on_disk {
            Index::build(manifest.clone())?;
            let cached = Scanner::new(&self.root)?.old;
            for (path, seen) in cached {
                if self.old.get(&path) != Some(&seen) {
                    self.old.insert(path, seen);
                }
            }
        }
        // A concurrent editor can change paths outside our local notification batch.
        let scopes = if changed_on_disk { None } else { scopes };
        let now = inventory(&self.root, &self.old, scopes, initial, &mut changes)?;
        if self.initialized && self.old == now && manifest == self.published.manifest {
            return Ok(changes);
        }
        let assigned = assign(&manifest, &self.old, &now, hints);
        let mut absent: HashSet<_> = manifest.assets.keys().cloned().collect();
        let mut dirty = false;
        for (path, s) in &now {
            let id = assigned
                .get(path)
                .cloned()
                .unwrap_or_else(|| bevy::asset::uuid::Uuid::new_v4().to_string());
            absent.remove(&id);
            let r = manifest.assets.entry(id.clone()).or_insert_with(|| {
                dirty = true;
                changes.refresh |= !initial;
                Record {
                    path: path.clone(),
                    size: s.stamp.size,
                    hash: s.hash.clone(),
                    aliases: Default::default(),
                    missing: false,
                }
            });
            if r.path != *path || r.missing {
                if r.path != *path {
                    r.aliases
                        .insert(std::mem::replace(&mut r.path, path.clone()));
                }
                r.missing = false;
                changes.refresh = true;
                dirty = true;
            }
            if r.hash != s.hash || r.size != s.stamp.size {
                r.hash = s.hash.clone();
                r.size = s.stamp.size;
                dirty = true;
            }
        }
        for uid in absent {
            let r = manifest.assets.get_mut(&uid).unwrap();
            if !r.missing {
                r.missing = true;
                dirty = true;
                changes.refresh = true;
            }
        }
        // Compare against this window's last snapshot even if another editor already
        // committed the move. Both windows must update their own tabs and undo history.
        for (id, previous) in &self.published.manifest.assets {
            if let Some(current) = manifest.assets.get(id)
                && (current.path != previous.path || current.missing != previous.missing)
            {
                let location = |r: &Record| {
                    if r.missing {
                        missing_path(id, &r.path)
                    } else {
                        r.path.clone()
                    }
                };
                changes.moves.push((location(previous), location(current)));
                if !current.missing && previous.path != current.path {
                    changes
                        .companions
                        .push((previous.path.clone(), current.path.clone()));
                }
                changes.refresh = true;
            }
        }
        for (id, current) in &manifest.assets {
            match self.published.manifest.assets.get(id) {
                Some(previous) if !current.missing && (current.hash != previous.hash || previous.missing) => {
                    changes.modified.push(current.path.clone());
                }
                None if self.initialized && !current.missing => changes.modified.push(current.path.clone()),
                _ => {}
            }
        }
        self.publish(manifest, rev, dirty)?;
        if now != self.old {
            let path = self.root.join(CACHE);
            let save = || atomic_write(&path, &serde_json::to_vec(&now)?, Revision::read(&path)?);
            if let Err(e) = save() {
                bevy::log::warn!("Could not save local asset index cache: {e}");
            }
        }
        self.old = now;
        self.initialized = true;
        Ok(changes)
    }
    fn publish(&mut self, manifest: Manifest, revision: Revision, dirty: bool) -> io::Result<()> {
        let snapshot = Arc::new(Index::build(manifest)?);
        let path = self.root.join(FILE);
        if dirty {
            let mut bytes = serde_json::to_vec_pretty(&snapshot.manifest)?;
            bytes.push(b'\n');
            atomic_write(&path, &bytes, revision)?;
        } else if Revision::read(&path)? != revision {
            return Err(invalid("Asset registry changed during scan"));
        }
        let _load = LOAD.lock().unwrap();
        self.published = snapshot.clone();
        super::publish(self.root.clone(), snapshot);
        Ok(())
    }
    pub(crate) fn relink(&mut self, uid: &str, path: &str) -> io::Result<Changes> {
        let (mut m, rev) = read_manifest(&self.root)?;
        let idx = Index::build(m.clone())?;
        let missing = m
            .assets
            .get(uid)
            .filter(|r| r.missing)
            .ok_or_else(|| invalid("Этот ресурс уже восстановлен"))?;
        if !portable(path) || format(&missing.path) != format(path) {
            return Err(invalid("Выберите файл того же формата внутри проекта"));
        }
        let target = idx
            .uid(path)
            .filter(|id| {
                m.assets
                    .get(*id)
                    .is_some_and(|r| !r.missing && r.path == path)
            })
            .ok_or_else(|| invalid("Файл ещё не проиндексирован"))?
            .to_owned();
        let selected = m.assets.remove(&target).unwrap();
        let record = m.assets.get_mut(uid).unwrap();
        let previous = std::mem::replace(record, selected);
        record.aliases.extend(previous.aliases);
        record.aliases.insert(previous.path.clone());
        for to in m.linked.values_mut() {
            if *to == target {
                *to = uid.to_owned();
            }
        }
        m.linked.insert(target, uid.to_owned());
        self.publish(m, rev, true)?;
        Ok(Changes {
            moves: vec![(missing_path(uid, &previous.path), path.into())],
            companions: vec![(previous.path, path.into())],
            refresh: true,
            ..Default::default()
        })
    }
}
