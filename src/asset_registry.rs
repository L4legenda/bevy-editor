//! Portable asset identities. Only the editor discovers files. Runtime lookups use the published snapshot.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, RwLock},
};

#[cfg(feature = "editor")]
pub(crate) mod scan;

pub(crate) const FILE: &str = ".bevy-editor-assets.json";
type Uid = String;

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Record {
    path: String,
    size: u64,
    hash: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    aliases: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    missing: bool,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    assets: BTreeMap<Uid, Record>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    linked: BTreeMap<Uid, Uid>,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            assets: BTreeMap::new(),
            linked: BTreeMap::new(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Index {
    manifest: Manifest,
    paths: HashMap<String, Uid>,
    historical: HashMap<String, Option<Uid>>,
}
fn invalid(s: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.into())
}
pub(crate) fn supported(path: &str) -> bool {
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    if path.contains('#') && matches!(ext.as_deref(), Some("glb" | "fbx" | "hdr" | "exr")) {
        return false;
    }
    matches!(
        ext.as_deref(),
        Some(
            "glb"
                | "fbx"
                | "png"
                | "jpg"
                | "jpeg"
                | "webp"
                | "bmp"
                | "tga"
                | "gif"
                | "ico"
                | "svg"
                | "hdr"
                | "exr"
                | "ktx2"
        )
    )
}
fn portable(path: &str) -> bool {
    !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
impl Index {
    fn build(manifest: Manifest) -> io::Result<Self> {
        if manifest.version != 1 {
            return Err(invalid("Unsupported asset registry version"));
        }
        let mut paths = HashMap::new();
        let mut aliases = HashMap::new();
        let mut historical = HashMap::new();
        for (uid, record) in &manifest.assets {
            if bevy::asset::uuid::Uuid::parse_str(uid).is_err()
                || !portable(&record.path)
                || !supported(&record.path)
                || record.hash.len() != 64
                || !record.hash.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(invalid("Invalid asset registry entry"));
            }
            if !record.missing && paths.insert(record.path.clone(), uid.clone()).is_some() {
                return Err(invalid("Duplicate physical asset path"));
            }
            for path in record.aliases.iter().chain(std::iter::once(&record.path)) {
                if !portable(path) {
                    return Err(invalid("Invalid asset alias"));
                }
                aliases
                    .entry(path.clone())
                    .and_modify(|v: &mut Option<Uid>| *v = None)
                    .or_insert_with(|| Some(uid.clone()));
            }
            for path in &record.aliases {
                historical
                    .entry(path.clone())
                    .and_modify(|v| *v = None)
                    .or_insert_with(|| Some(uid.clone()));
            }
            paths.insert(missing_path(uid, &record.path), uid.clone());
        }
        for (path, uid) in aliases {
            if let Some(uid) = uid {
                paths.entry(path).or_insert(uid);
            }
        }
        for (from, to) in &manifest.linked {
            if bevy::asset::uuid::Uuid::parse_str(from).is_err()
                || manifest.assets.contains_key(from)
                || !manifest.assets.contains_key(to)
            {
                return Err(invalid("Invalid linked asset UID"));
            }
        }
        Ok(Self {
            manifest,
            paths,
            historical,
        })
    }
    #[cfg(feature = "editor")]
    pub(crate) fn previous_name(&self, path: &str, name: &str) -> bool {
        self.uid(path)
            .and_then(|id| self.manifest.assets.get(id))
            .is_some_and(|r| {
                !r.missing
                    && r.path == path
                    && r.aliases.iter().any(|old| {
                        Path::new(old)
                            .file_name()
                            .and_then(|x| x.to_str())
                            .is_some_and(|n| n.eq_ignore_ascii_case(name))
                    })
            })
    }
    #[cfg(feature = "editor")]
    pub(crate) fn known(&self, id: &str) -> bool {
        self.manifest.assets.contains_key(id) || self.manifest.linked.contains_key(id)
    }
    pub(crate) fn uid(&self, path: &str) -> Option<&str> {
        self.paths.get(path).map(String::as_str)
    }
    pub(crate) fn resolve(&self, path: &str, uid: Option<&str>) -> String {
        if !portable(path) {
            return path.into();
        }
        let Some(uid) = uid.or_else(|| self.uid(path)) else {
            return path.to_owned();
        };
        let id = self
            .manifest
            .linked
            .get(uid)
            .map(String::as_str)
            .unwrap_or(uid);
        match self.manifest.assets.get(id) {
            Some(r) if !r.missing => r.path.clone(),
            Some(r) => missing_path(id, &r.path),
            None => path.to_owned(), // Foreign/legacy resource UIDs use their path fallback.
        }
    }
    pub(crate) fn bind(&self, path: &mut String, uid: &mut Option<String>) {
        if uid.is_none() {
            *uid = self.uid(path).map(str::to_owned);
        }
        *path = self.resolve(path, uid.as_deref());
    }
    pub(crate) fn read_ref(&self, path: &mut String, uid: &mut Option<String>) {
        if uid.is_none() {
            *uid = self.historical.get(path).cloned().flatten();
        }
        self.bind(path, uid);
    }
    pub(crate) fn save_ref(&self, path: &mut String, uid: &mut Option<String>) {
        self.bind(path, uid);
        if let Some(id) = uid.as_deref() {
            let id = self
                .manifest
                .linked
                .get(id)
                .map(String::as_str)
                .unwrap_or(id);
            if let Some(record) = self.manifest.assets.get(id) {
                path.clone_from(&record.path);
            }
        }
    }
    pub(crate) fn bind_json(&self, obj: &mut serde_json::Value) {
        let Some(mut path) = obj.get("asset").and_then(|v| v.as_str()).map(str::to_owned) else {
            return;
        };
        let mut uid = obj.get("uid").and_then(|v| v.as_str()).map(str::to_owned);
        self.save_ref(&mut path, &mut uid);
        obj["asset"] = path.into();
        if let Some(uid) = uid {
            obj["uid"] = uid.into();
        }
    }
    pub(crate) fn scene(&self, scene: &mut bevy_scene_format::Scene, writing: bool) {
        if writing
            && let Some(hdri) = scene
                .settings
                .get_mut("environment")
                .and_then(|v| v.get_mut("hdri_asset"))
        {
            self.bind_json(hdri);
        }
        for resource in scene.resources.values_mut() {
            if let Some(path) = &mut resource.path
                && supported(path)
            {
                // The scene codec reserves '#' for selectors. Keep its portable hint
                // when an image gets a literal '#' through the filesystem.
                let hint = path.clone();
                if writing {
                    self.save_ref(path, &mut resource.uid);
                } else {
                    self.read_ref(path, &mut resource.uid);
                }
                if path.contains('#') {
                    *path = hint;
                }
            }
        }
    }
    #[cfg(feature = "editor")]
    pub(crate) fn missing(&self) -> impl Iterator<Item = (&str, &str)> {
        self.manifest
            .assets
            .iter()
            .filter(|(_, r)| r.missing)
            .map(|(id, r)| (id.as_str(), r.path.as_str()))
    }
}
fn missing_path(uid: &str, path: &str) -> String {
    // Even malformed foreign UIDs cannot inject path components.
    let uid = bevy::asset::uuid::Uuid::parse_str(uid).unwrap_or_default();
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("asset");
    format!(".bevy-editor/missing/{uid}.{ext}")
}
type Snapshots = HashMap<PathBuf, Arc<Index>>;
static INDICES: LazyLock<RwLock<Snapshots>> = LazyLock::new(Default::default);
static LOAD: Mutex<()> = Mutex::new(());

fn read_manifest(root: &Path) -> io::Result<(Manifest, bevy_scene_format::Revision)> {
    let path = root.join(FILE);
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err(invalid("Asset registry must be a regular file"));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok((Manifest::default(), bevy_scene_format::Revision::Missing));
        }
        Err(e) => return Err(e),
        _ => {}
    }
    let bytes = bevy_scene_format::read_limited(&path, 128 << 20)?;
    Ok((
        serde_json::from_slice(&bytes).map_err(io::Error::other)?,
        bevy_scene_format::Revision::of(&bytes),
    ))
}
fn publish(root: PathBuf, snapshot: Arc<Index>) {
    // Inactive project snapshots can be reconstructed from their portable manifest.
    let mut indices = INDICES.write().unwrap();
    if indices.len() >= 16
        && !indices.contains_key(&root)
        && let Some(old) = indices.keys().next().cloned()
    {
        indices.remove(&old);
    }
    indices.insert(root, snapshot);
}
#[cfg(feature = "editor")]
pub(crate) fn reopen(root: &Path) -> io::Result<Arc<Index>> {
    let key = root.canonicalize()?;
    let _load = LOAD.lock().unwrap();
    let snapshot = Arc::new(Index::build(read_manifest(&key)?.0)?);
    publish(key, snapshot.clone());
    Ok(snapshot)
}
pub(crate) fn index(root: &Path) -> io::Result<Arc<Index>> {
    if let Some(index) = INDICES.read().unwrap().get(root) {
        return Ok(index.clone());
    }
    let key = root.canonicalize()?;
    let _load = LOAD.lock().unwrap();
    if let Some(index) = INDICES.read().unwrap().get(&key) {
        return Ok(index.clone());
    }
    let snapshot = Arc::new(Index::build(read_manifest(&key)?.0)?);
    publish(key, snapshot.clone());
    Ok(snapshot)
}

#[cfg(all(test, feature = "editor"))]
mod tests;
