//! Material Painter community index, portable packages and installed graph records.
use super::{Material, valid_id};
use crate::editor::model_paint_data::texture_fill::{Bitmap, GraphImages};
use crate::editor::{
    hdri_catalog::{atomic_write, clean_name, get, strings},
    ui::model_viewer::paint::{material_graph, node_library, presets},
};
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) const BASE: &str =
    "https://raw.githubusercontent.com/alla2001/material-painter-community/main/";
const MAX_PACKAGE: u64 = 194 * 1024 * 1024;
const MAX_FILES: usize = material_graph::file::package::ENTRY_LIMIT - 1;

pub(in super::super) enum Draft {
    Graph {
        payload: Arc<Value>,
        images: Arc<GraphImages>,
        preview: Option<Arc<Bitmap>>,
    },
    Smart(presets::Capture),
    SavedSmart(presets::Entry),
    SavedGraph(super::super::graph_shelf::Entry),
}

pub(in super::super) struct Submission {
    pub path: PathBuf,
    pub name: String,
    pub omitted: usize,
    pub adjusted: usize,
}

pub(in super::super) fn prepare(root: &Path, draft: Draft) -> Result<Submission, String> {
    let directory = registry(root).join("submissions");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    match draft {
        Draft::Graph {
            payload,
            images,
            preview,
        } => {
            let thumbnail = super::super::graph_shelf::thumbnail(preview.as_deref())?;
            let name = clean_name(payload.get("name"), "Граф материала");
            let path = material_graph::file::package::export_submission(
                &directory,
                &payload,
                &images,
                thumbnail.as_deref(),
            )?;
            Ok(Submission {
                path,
                name,
                omitted: 0,
                adjusted: 0,
            })
        }
        Draft::Smart(capture) => {
            let presets::Capture::Material { name, .. } = &capture else {
                return Err("Выберите материал для отправки".into());
            };
            let name = name.clone();
            let report = presets::export_community_package(&directory, capture)?;
            Ok(Submission {
                path: report.path,
                name,
                omitted: report.omitted,
                adjusted: report.adjusted,
            })
        }
        Draft::SavedSmart(entry) => {
            let name = entry.name.clone();
            let report = presets::export_saved_community_package(&directory, entry)?;
            Ok(Submission { path: report.path, name, omitted: report.omitted, adjusted: report.adjusted })
        }
        Draft::SavedGraph(entry) => {
            let (graph, images) = material_graph::file::load(&entry.path)?;
            let thumbnail = super::super::graph_shelf::thumbnail(entry.thumbnail.as_deref())?;
            let path = material_graph::file::package::export_submission(
                &directory, &material_graph::encode(&graph), &images, thumbnail.as_deref(),
            )?;
            Ok(Submission { path, name: graph.name, omitted: 0, adjusted: 0 })
        }
    }
}

impl Submission {
    pub fn submit_url(&self) -> reqwest::Url {
        let mut url = reqwest::Url::parse(
            "https://github.com/alla2001/material-painter-community/issues/new",
        )
        .unwrap();
        url.query_pairs_mut()
            .append_pair("template", "submit.yml")
            .append_pair("title", &format!("Submit: {}", self.name));
        url
    }
}

#[cfg(windows)]
pub(in super::super) fn open_form(url: &reqwest::Url) -> std::io::Result<()> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let target: Vec<_> = url.as_str().encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            null(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as usize <= 32 {
        Err(std::io::Error::other(format!(
            "Код открытия браузера · {}",
            result as usize
        )))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(in super::super) fn open_form(url: &reqwest::Url) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let opener = "/usr/bin/open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let mut child = std::process::Command::new(opener)
        .arg(url.as_str())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(in super::super) enum Kind {
    Graph,
    Node,
    Smart,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Node => "node",
            Self::Smart => "smart",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Graph => "mpgraph",
            Self::Node => "wgsl",
            Self::Smart => "mpsmart",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Graph => "Граф",
            Self::Node => "Узел WGSL",
            Self::Smart => "Умный материал",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in super::super) struct Package {
    pub kind: Kind,
    path: String,
    files: Vec<String>,
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && material_graph::file::package::relative(path)
        && !path.chars().any(char::is_control)
}

fn url(base: &str, file: &str) -> Option<String> {
    if !relative(base) || !relative(file) {
        return None;
    }
    let mut url = reqwest::Url::parse(BASE).ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend(base.split('/').chain(file.split('/')));
    Some(url.into())
}

fn format(path: &str) -> Option<&str> {
    Path::new(path).extension()?.to_str()
}

impl Package {
    fn valid(&self) -> bool {
        if !relative(&self.path) || self.files.is_empty() || self.files.len() > MAX_FILES {
            return false;
        }
        let mut seen = HashSet::new();
        self.files.iter().all(|file| {
            relative(file)
                && seen.insert(file.to_ascii_lowercase())
                && file
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| !name.eq_ignore_ascii_case("info.json"))
        }) && self
            .files
            .iter()
            .filter(|file| format(file) == Some(self.kind.extension()))
            .count()
            == 1
    }
}

pub(in super::super) fn catalog(value: &Value) -> Result<Vec<Material>, String> {
    let items = value["items"]
        .as_array()
        .filter(|_| value["version"] == 1)
        .ok_or("Неверный индекс сообщества Material Painter")?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str().filter(|id| valid_id(id))?;
            let path = item["path"].as_str()?.trim_end_matches('/').to_owned();
            let kind: Kind = serde_json::from_value(item["kind"].clone()).ok()?;
            let files = item["files"]
                .as_array()?
                .iter()
                .map(|file| file.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?;
            let package = Package { kind, path, files };
            if !package.valid() {
                return None;
            }
            let mut terms = strings(item.get("tags"));
            terms.push(clean_name(item.get("author"), ""));
            terms.push(kind.label().into());
            let mut entry = Material::new(id, clean_name(item.get("name"), id), terms);
            entry.thumbnail = item["thumb"]
                .as_str()
                .and_then(|file| url(&package.path, file));
            entry.community = Some(package);
            Some(entry)
        })
        .collect())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in super::super) struct Record {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    file: String,
}

impl Record {
    pub fn path(&self, root: &Path) -> Option<PathBuf> {
        if !valid_id(&self.id) || !relative(&self.file) || self.file.contains('/') {
            return None;
        }
        match self.kind {
            Kind::Graph if self.file == format!("{}.zip", self.id) => {
                Some(registry(root).join(&self.file))
            }
            Kind::Node if format(&self.file) == Some("wgsl") => {
                Some(root.join("nodes").join(&self.file))
            }
            Kind::Smart if format(&self.file) == Some("mpsmart") => Some(root.join(&self.file)),
            _ => None,
        }
    }
}

fn registry(root: &Path) -> PathBuf {
    root.join("online/community")
}

fn read_record(root: &Path, path: &Path) -> Option<Record> {
    let bytes = material_graph::file::read_limited(path, 16 * 1024).ok()?;
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    record.path(root).filter(|file| file.is_file())?;
    Some(record)
}

pub(in super::super) struct Installed {
    pub record: Record,
    pub smart: Option<(presets::Entry, usize)>,
}

#[derive(Default)]
pub(in super::super) struct Library {
    pub records: Vec<Record>,
    pub revision: u64,
    scan: Option<Task<Vec<Record>>>,
    initialized: bool,
    install: Option<Task<Result<Installed, String>>>,
}

impl Library {
    pub fn ensure(&mut self, root: Option<PathBuf>) {
        if self.initialized {
            return;
        }
        let Some(root) = root else { return };
        self.initialized = true;
        self.scan = Some(IoTaskPool::get().spawn(async move {
            let mut rows: Vec<_> = fs::read_dir(registry(&root))
                .into_iter()
                .flatten()
                .flatten()
                .filter(|file| format(&file.file_name().to_string_lossy()) == Some("json"))
                .filter_map(|file| read_record(&root, &file.path()))
                .collect();
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            rows
        }));
    }

    pub fn installed(&self, id: &str) -> bool {
        self.records.iter().any(|record| record.id == id)
    }

    pub fn busy(&self) -> bool {
        self.install.is_some()
    }

    pub fn start(&mut self, root: PathBuf, entry: Material) {
        if self.busy() || self.installed(&entry.id) {
            return;
        }
        self.install = Some(
            IoTaskPool::get()
                .spawn(async move { install(&root, &entry, |url, limit| get(url, limit, None)) }),
        );
        self.revision += 1;
    }

    pub fn poll(&mut self) -> Option<Result<Installed, String>> {
        if let Some(records) = self.scan.as_mut().and_then(|job| block_on(poll_once(job))) {
            self.scan = None;
            for record in records {
                self.upsert(record);
            }
        }
        let result = self
            .install
            .as_mut()
            .and_then(|job| block_on(poll_once(job)))?;
        self.install = None;
        if let Ok(installed) = &result {
            self.upsert(installed.record.clone());
        }
        self.revision += 1;
        Some(result)
    }

    fn upsert(&mut self, record: Record) {
        if let Some(old) = self.records.iter_mut().find(|old| old.id == record.id) {
            *old = record;
        } else {
            self.records.push(record);
        }
        self.revision += 1;
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(in super::super) fn install(
    root: &Path,
    entry: &Material,
    fetch: impl Fn(&str, u64) -> Result<Vec<u8>, String>,
) -> Result<Installed, String> {
    let package = entry
        .community
        .as_ref()
        .filter(|package| valid_id(&entry.id) && package.valid())
        .ok_or("Некорректный пакет сообщества")?;
    let registry = registry(root);
    fs::create_dir_all(&registry).map_err(|error| error.to_string())?;
    let manifest = registry.join(format!("{}.json", entry.id));
    if let Some(record) = read_record(root, &manifest) {
        return Ok(Installed {
            record,
            smart: None,
        });
    }
    let mut smart = None;
    let file = match package.kind {
        Kind::Node => {
            let main = package
                .files
                .iter()
                .find(|file| format(file) == Some("wgsl"))
                .unwrap();
            let bytes = fetch(
                &url(&package.path, main).ok_or("Некорректный путь пакета")?,
                node_library::SOURCE_LIMIT as u64,
            )?;
            if bytes.len() > node_library::SOURCE_LIMIT {
                return Err("Исходник WGSL слишком большой".into());
            }
            let source =
                std::str::from_utf8(&bytes).map_err(|_| "Ожидался исходник WGSL в UTF-8")?;
            let directory = root.join("nodes");
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let path = node_library::publish_named(&directory, Path::new(main), source)?;
            path.file_name().unwrap().to_string_lossy().into_owned()
        }
        Kind::Graph | Kind::Smart => {
            let scratch = Scratch(registry.join(format!(
                "download-{}.zip",
                bevy::asset::uuid::Uuid::new_v4()
            )));
            atomic_write(&scratch.0, &bundle(package, fetch)?)?;
            if package.kind == Kind::Graph {
                material_graph::file::load(&scratch.0)?;
                let file = format!("{}.zip", entry.id);
                fs::rename(&scratch.0, registry.join(&file)).map_err(|error| error.to_string())?;
                file
            } else {
                let imported = presets::import_material(root, &scratch.0)?;
                let file = imported
                    .0
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                smart = Some(imported);
                file
            }
        }
    };
    let record = Record {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: package.kind,
        file,
    };
    atomic_write(
        &manifest,
        &serde_json::to_vec(&record).map_err(|error| error.to_string())?,
    )?;
    Ok(Installed { record, smart })
}

fn bundle(
    package: &Package,
    fetch: impl Fn(&str, u64) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    use material_graph::file::package::add_entry;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let info = serde_json::json!({"kind": package.kind.key(), "files": package.files});
    add_entry(
        &mut zip,
        "info.json",
        true,
        &serde_json::to_vec(&info).map_err(|error| error.to_string())?,
    )?;
    let mut remaining = MAX_PACKAGE;
    for file in &package.files {
        let limit = if format(file) == Some("wgsl") {
            remaining.min(node_library::SOURCE_LIMIT as u64)
        } else {
            remaining
        };
        let bytes = fetch(
            &url(&package.path, file).ok_or("Некорректный путь пакета")?,
            limit,
        )?;
        remaining = remaining
            .checked_sub(bytes.len() as u64)
            .filter(|_| bytes.len() as u64 <= limit)
            .ok_or("Пакет сообщества слишком большой")?;
        add_entry(&mut zip, file, true, &bytes)?;
    }
    zip.finish()
        .map(Cursor::into_inner)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const WGSL: &str = "// @name Community test tint\n// @output color\n// @input source: color = 1\nfn node(uv: vec2<f32>) -> vec4<f32> { return in_source(uv) * vec4<f32>(0.15, 0.25, 0.8, 1.0); }\n";

    #[test]
    fn community_submission_packages_keep_preview_dependencies_and_cc0_metadata() {
        let root = std::env::temp_dir().join(format!(
            "bevy-submissions-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let directory = registry(&root).join("submissions");
        fs::create_dir_all(&directory).unwrap();
        let name = "Синий граф & камень";
        let graph = json!({"name": name, "res": 64, "next_id": 4, "nodes": [
            {"id": 1, "def": "bitmap", "pos": [0, 0], "params": {"image": "images/tile.png"}},
            {"id": 2, "def": "user/share-tint", "pos": [0, 0], "links": {"source": 1}},
            {"id": 3, "def": "output", "pos": [200, 0], "links": {"input": 2}}
        ], "bevy_wgsl_nodes": {"user/share-tint": WGSL}});
        let preview = Arc::new(
            Bitmap::from_rgba(
                "Preview.png".into(),
                image::RgbaImage::from_pixel(288, 144, image::Rgba([11, 83, 170, 255])),
            )
            .unwrap(),
        );
        let inputs = || Draft::Graph {
            payload: Arc::new(graph.clone()),
            images: Arc::new(GraphImages::from([(
                "images/tile.png".into(),
                preview.clone(),
            )])),
            preview: Some(preview.clone()),
        };
        let graph = prepare(&root, inputs()).unwrap();
        let repeated = prepare(&root, inputs()).unwrap();
        assert_ne!(graph.path, repeated.path);
        let query: std::collections::HashMap<_, _> =
            graph.submit_url().query_pairs().into_owned().collect();
        assert_eq!(query["template"], "submit.yml");
        assert_eq!(query["title"], format!("Submit: {name}"));
        let (loaded, images) = material_graph::file::load(&graph.path).unwrap();
        assert!(node_library::is_embedded(&loaded.node(2).unwrap().def));
        assert_eq!(images.values().next().unwrap().pixels().width(), 288);

        let source = root.join("source.mpsmart");
        presets::mpsmart_fixture(&source, false);
        let smart = prepare(
            &root,
            Draft::Smart(presets::Capture::Material {
                name: "Синий материал".into(),
                layers: presets::load_material(&source, 16).unwrap(),
            }),
        )
        .unwrap();
        let mut ids = Vec::new();
        for (path, kind) in [(&graph.path, "graph"), (&smart.path, "smart")] {
            use std::io::Read;
            let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
            let manifest = archive
                .file_names()
                .filter_map(Result::ok)
                .find(|name| name.ends_with("/info.json"))
                .unwrap()
                .into_owned();
            let mut json = Vec::new();
            archive
                .by_name(&manifest)
                .unwrap()
                .read_to_end(&mut json)
                .unwrap();
            let info: Value = serde_json::from_slice(&json).unwrap();
            assert_eq!(info["license"], "CC0");
            assert_eq!(info["kind"], kind);
            assert_eq!(info["thumb"], "thumb.png");
            let id = info["id"].as_str().unwrap();
            assert_eq!(info["path"], format!("items/{id}"));
            let mut thumbnail = Vec::new();
            archive
                .by_name(&format!("{id}/thumb.png"))
                .unwrap()
                .read_to_end(&mut thumbnail)
                .unwrap();
            let preview = image::load_from_memory(&thumbnail).unwrap();
            if kind == "graph" {
                assert_eq!([preview.width(), preview.height()], [144, 72]);
            }
            assert!(preview.width() > 0);
            ids.push(id.to_owned());
        }
        assert_ne!(ids[0], ids[1]);
        let (installed, _) = presets::import_material(&root, &smart.path).unwrap();
        assert!(
            !presets::load_material(&installed.path, 16)
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn community_catalog_keeps_supported_resources_and_rejects_ambiguous_paths() {
        let entries = catalog(&json!({"version": 1, "items": [
            {"id": "wall", "name": "Wall Pattern", "author": "Test Artist", "tags": ["brick"],
                "kind": "graph", "path": "items/wall", "files": ["Wall pattern.mpgraph", "nodes/tint.wgsl"], "thumb": "thumb.png"},
            {"id": "escape", "kind": "node", "path": "../elsewhere", "files": ["test.wgsl"]},
            {"id": "duplicate", "kind": "graph", "path": "items/duplicate", "files": ["a.mpgraph", "A.mpgraph"]},
            {"id": "ambiguous", "kind": "smart", "path": "items/ambiguous", "files": ["a.mpsmart", "b.mpsmart"]},
            {"id": "unknown", "kind": "binary", "path": "items/unknown", "files": ["test.exe"]}
        ]})).unwrap();
        assert_eq!(entries.len(), 1);
        let asset = &entries[0];
        assert!(asset.matches("artist brick граф"));
        assert_eq!(
            asset.thumbnail.as_deref(),
            Some(concat!(
                "https://raw.githubusercontent.com/alla2001/material-painter-community/main/",
                "items/wall/thumb.png"
            ))
        );
        assert!(
            url("items/wall", "Wall pattern.mpgraph")
                .unwrap()
                .ends_with("Wall%20pattern.mpgraph")
        );
        assert!(
            catalog(&json!({"version": 1, "items": []}))
                .unwrap()
                .is_empty()
        );
        assert!(catalog(&json!({"version": 2, "items": []})).is_err());
    }

    #[test]
    fn community_installs_graph_with_portable_dependencies_smart_material_and_node() {
        let root = std::env::temp_dir().join(format!(
            "bevy-community-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let entries = catalog(&json!({"version": 1, "items": [
            {"id": "wall", "kind": "graph", "path": "items/wall",
                "files": ["wall.mpgraph", "nodes/community-tint.wgsl", "images/tile.png"]},
            {"id": "brass", "kind": "smart", "path": "items/brass", "files": ["brass.mpsmart"]},
            {"id": "tint", "kind": "node", "path": "items/tint", "files": ["community-demo-tint.wgsl"]}
        ]}))
        .unwrap();
        let graph = json!({"name": "Community wall", "res": 64, "next_id": 4, "nodes": [
            {"id": 1, "def": "bitmap", "pos": [0, 0], "params": {"image": "images/tile.png"}},
            {"id": 2, "def": "user/community-tint", "pos": [0, 0], "links": {"source": 1}},
            {"id": 3, "def": "output", "pos": [200, 0], "links": {"input": 2}}
        ]});
        let pbr_path = root.join("fixture.mpsmart");
        presets::mpsmart_fixture(&pbr_path, false);
        let color = [61, 120, 230, 255];
        let png = super::super::super::texture_fill::encode_png(&image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba(color),
        ))
        .unwrap();
        let fetch = |url: &str, _: u64| -> Result<Vec<u8>, String> {
            if url.ends_with("wall.mpgraph") {
                return Ok(serde_json::to_vec(&graph).unwrap());
            }
            if url.ends_with(".wgsl") {
                return Ok(WGSL.as_bytes().to_vec());
            }
            if url.ends_with("tile.png") {
                return Ok(png.clone());
            }
            if url.ends_with("brass.mpsmart") {
                return fs::read(&pbr_path).map_err(|error| error.to_string());
            }
            Err(format!("Unexpected path {url}"))
        };
        let graph = install(&root, &entries[0], fetch).unwrap();
        let graph_file = graph.record.path(&root).unwrap();
        let (loaded, images) = material_graph::file::load(&graph_file).unwrap();
        let definition = loaded.node(2).unwrap().def.clone();
        assert!(node_library::is_embedded(&definition));
        assert_eq!(
            node_library::library().find(&definition).unwrap().source,
            WGSL
        );
        assert_eq!(
            images.values().next().unwrap().pixels().get_pixel(0, 0).0,
            color
        );
        let smart = install(&root, &entries[1], fetch).unwrap();
        let (preset, _) = smart.smart.unwrap();
        assert_eq!(preset.path, smart.record.path(&root).unwrap());
        assert!(!presets::load_material(&preset.path, 8).unwrap().is_empty());
        let node = install(&root, &entries[2], fetch).unwrap();
        let path = node.record.path(&root).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), WGSL);
        assert_eq!(
            node_library::node_id(&path).unwrap(),
            "user/community-demo-tint"
        );
        assert!(
            node_library::library()
                .find(&node_library::node_id(&path).unwrap())
                .is_some()
        );
        for entry in &entries {
            let restored =
                install(&root, entry, |_, _| panic!("Installed items are reused")).unwrap();
            assert!(restored.record.path(&root).unwrap().is_file());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
