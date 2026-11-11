//! A snapshot of the project's folders, files, and placeable GLB assets.
//!
//! Folder IDs and catalog indices stay stable while navigating. Filesystem
//! symlinks are deliberately omitted, so browsing cannot leave the asset root.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use bevy::prelude::Resource;

use crate::{
    data::relative_path,
    project::{CatalogItem, EditorProject},
};

#[derive(Debug)]
pub(super) struct Folder {
    pub label: String,
    /// Relative to `EditorProject::asset_root`; the root is an empty path.
    pub path: PathBuf,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Folder(usize),
    /// A physical file can refer to more than one configured scene.
    Scene {
        file: usize,
        room: usize,
    },
    Model(usize),
    Image(usize),
    File(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileKind {
    Scene,
    Image,
    Other,
}

#[derive(Debug)]
pub(super) struct BrowserFile {
    pub folder: usize,
    /// Relative to the asset root, preserving the actual filesystem spelling.
    pub path: PathBuf,
    pub filename: String,
    /// Original case, without the dot. Empty for extensionless files.
    pub extension: String,
    pub bytes: u64,
    pub kind: FileKind,
    scenes: Vec<SceneLink>,
}

#[derive(Clone, Debug)]
struct SceneLink {
    room: usize,
    label: String,
}

#[derive(Resource)]
pub(super) struct Browser {
    pub folders: Vec<Folder>,
    /// Non-model files; scene, image and generic entries address this list.
    pub files: Vec<BrowserFile>,
    pub current: usize,
    pub search: String,
    pub revision: u64,
    /// Folder-tree changes are separate from content, so expanding a branch
    /// does not recreate the grid or reset its scroll position.
    pub tree_revision: u64,
    /// File indices may change only after a filesystem rescan. Async preview
    /// jobs use this generation to reject results for old file indices.
    pub scan_revision: u64,
    /// An unreadable folder does not prevent browsing the rest of the project.
    pub error: Option<String>,
    models: Vec<(usize, usize)>,
    expanded: HashSet<usize>,
    back: Vec<usize>,
    forward: Vec<usize>,
}

impl Browser {
    /// A scene's source GLB can be placed as a model without opening the scene
    /// or adding a new catalog item. Images and folders are never placeable.
    pub fn model_asset(&self, catalog: &[CatalogItem], entry: Entry) -> Option<String> {
        let asset = match entry {
            Entry::Model(index) => catalog.get(index)?.asset.clone(),
            Entry::Scene { file, .. } | Entry::File(file) => self
                .files
                .get(file)?
                .path
                .to_str()?
                .replace(std::path::MAIN_SEPARATOR, "/"),
            Entry::Folder(_) | Entry::Image(_) => return None,
        };
        relative_path(&asset, "glb").then_some(asset)
    }

    /// Discover GLBs before allocating thumbnail textures. Existing catalog
    /// entries retain their indices and all host-provided placement settings.
    /// Both the source GLB and the saved JSON open their configured scene.
    pub fn new(project: &mut EditorProject) -> Self {
        let mut browser = Self {
            folders: vec![Folder {
                label: "Ассеты".into(),
                path: PathBuf::new(),
                parent: None,
                children: Vec::new(),
            }],
            files: Vec::new(),
            current: 0,
            search: String::new(),
            revision: 0,
            tree_revision: 0,
            scan_revision: 0,
            error: None,
            models: Vec::new(),
            expanded: HashSet::new(),
            back: Vec::new(),
            forward: Vec::new(),
        };
        // Several host-defined variants may share a GLB while retaining their
        // own labels, collision settings, and thumbnail directions.
        let mut catalog: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, model) in project.catalog.iter().enumerate() {
            catalog.entry(model.asset.clone()).or_default().push(index);
        }
        let mut scenes: HashMap<String, Vec<SceneLink>> = HashMap::new();
        for (room, scene) in project.scenes.iter().enumerate() {
            for path in [&scene.asset, &scene.file] {
                scenes.entry(path.clone()).or_default().push(SceneLink {
                    room,
                    label: scene.label.clone(),
                });
            }
        }
        let mut folder = 0;
        let mut discover = vec![true];
        while folder < browser.folders.len() {
            let directory = project.asset_root.join(&browser.folders[folder].path);
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) => {
                    browser.record_error(&directory, &error);
                    folder += 1;
                    continue;
                }
            };
            let mut entries: Vec<_> = entries
                .filter_map(|entry| match entry {
                    Ok(entry) => Some(entry),
                    Err(error) => {
                        browser.record_error(&directory, &error);
                        None
                    }
                })
                .collect();
            entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
            for entry in entries {
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let kind = match entry.file_type() {
                    Ok(kind) => kind,
                    Err(error) => {
                        browser.record_error(&entry.path(), &error);
                        continue;
                    }
                };
                let path = browser.folders[folder].path.join(&name);
                let hidden = name.starts_with('.') || ignored_directory(&name);
                if kind.is_dir() {
                    let allow_discovery = discover[folder] && !hidden;
                    if !allow_discovery
                        && !catalog
                            .keys()
                            .chain(scenes.keys())
                            .any(|asset| Path::new(asset).starts_with(&path))
                    {
                        continue;
                    }
                    let id = browser.folders.len();
                    browser.folders.push(Folder {
                        label: name,
                        path,
                        parent: Some(folder),
                        children: Vec::new(),
                    });
                    browser.folders[folder].children.push(id);
                    discover.push(allow_discovery);
                } else if kind.is_file() {
                    let Some(asset) = path
                        .to_str()
                        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
                    else {
                        continue;
                    };
                    let links = scenes.get(&asset);
                    if let Some(indices) = catalog.get(&asset).filter(|_| links.is_none()) {
                        browser
                            .models
                            .extend(indices.iter().map(|&index| (folder, index)));
                    } else {
                        if links.is_none() && (hidden || !discover[folder]) {
                            continue;
                        }
                        if links.is_none() && relative_path(&asset, "glb") {
                            let index = project.catalog.len();
                            project.catalog.push(CatalogItem {
                                label: Path::new(&name)
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .replace(['_', '-'], " "),
                                asset: asset.clone(),
                                category: browser.folders[folder].label.clone(),
                                solid: false,
                                preview_direction: [1.55, 1.3, 2.3],
                            });
                            catalog.insert(asset, vec![index]);
                            browser.models.push((folder, index));
                        } else {
                            let extension = path
                                .extension()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            let bytes = entry
                                .metadata()
                                .map(|metadata| metadata.len())
                                .unwrap_or_default();
                            browser.files.push(BrowserFile {
                                folder,
                                path,
                                filename: name,
                                kind: if links.is_some() {
                                    FileKind::Scene
                                } else {
                                    file_kind(&extension)
                                },
                                extension,
                                bytes,
                                scenes: links.cloned().unwrap_or_default(),
                            });
                        }
                    }
                }
            }
            folder += 1;
        }
        browser
    }

    /// Rescan after external file changes without resetting the user's location.
    /// If that folder was removed, fall back to its nearest surviving parent.
    pub fn refresh(&mut self, project: &mut EditorProject) {
        let current = self.folders[self.current].path.clone();
        let back: Vec<_> = self
            .back
            .iter()
            .map(|&folder| self.folders[folder].path.clone())
            .collect();
        let forward: Vec<_> = self
            .forward
            .iter()
            .map(|&folder| self.folders[folder].path.clone())
            .collect();
        let expanded: Vec<_> = self
            .expanded
            .iter()
            .map(|&folder| self.folders[folder].path.clone())
            .collect();
        let mut refreshed = Self::new(project);
        let folders: HashMap<_, _> = refreshed
            .folders
            .iter()
            .enumerate()
            .map(|(index, folder)| (folder.path.clone(), index))
            .collect();
        refreshed.current = current
            .ancestors()
            .find_map(|path| folders.get(path).copied())
            .unwrap_or(0);
        refreshed.back = back
            .iter()
            .filter_map(|path| folders.get(path).copied())
            .collect();
        refreshed.forward = forward
            .iter()
            .filter_map(|path| folders.get(path).copied())
            .collect();
        refreshed.expanded = expanded
            .iter()
            .filter_map(|path| folders.get(path).copied())
            .collect();
        refreshed.search = std::mem::take(&mut self.search);
        refreshed.revision = self.revision + 1;
        refreshed.tree_revision = self.tree_revision + 1;
        refreshed.scan_revision = self.scan_revision + 1;
        *self = refreshed;
    }

    /// Keep navigation and expansion attached to their directories when a
    /// filesystem move changes the prefix of an entire branch.
    pub fn refresh_after_move(&mut self, project: &mut EditorProject, from: &Path, to: &Path) {
        for folder in &mut self.folders {
            if let Ok(suffix) = folder.path.strip_prefix(from) {
                folder.path = to.join(suffix);
            }
        }
        self.refresh(project);
    }

    fn record_error(&mut self, directory: &Path, error: &std::io::Error) {
        self.error.get_or_insert_with(|| {
            format!("Не удалось прочитать {}: {error}", directory.display())
        });
    }

    pub fn navigate(&mut self, folder: usize) -> bool {
        if folder >= self.folders.len() {
            return false;
        }
        let revealed = self.expand_ancestors(folder);
        if folder == self.current {
            let changed = self.set_search(String::new());
            if revealed {
                self.tree_revision += 1;
            }
            return changed || revealed;
        }
        self.back.push(self.current);
        self.forward.clear();
        self.current = folder;
        self.search.clear();
        self.revision += 1;
        if revealed {
            self.tree_revision += 1;
        }
        true
    }

    pub fn is_expanded(&self, folder: usize) -> bool {
        folder == 0 || self.expanded.contains(&folder)
    }

    pub fn toggle_folder(&mut self, folder: usize) -> bool {
        // Keep the asset root open, with its top-level directories visible.
        if folder == 0 || folder >= self.folders.len() {
            return false;
        }
        if !self.expanded.remove(&folder) {
            self.expanded.insert(folder);
        }
        self.tree_revision += 1;
        true
    }

    fn expand_ancestors(&mut self, folder: usize) -> bool {
        let mut changed = false;
        let mut current = folder;
        while let Some(parent) = self.folders[current].parent {
            if parent != 0 {
                changed |= self.expanded.insert(parent);
            }
            current = parent;
        }
        changed
    }

    pub fn home(&mut self) -> bool {
        self.navigate(0)
    }

    pub fn up(&mut self) -> bool {
        self.folders[self.current]
            .parent
            .is_some_and(|parent| self.navigate(parent))
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn go_back(&mut self) -> bool {
        let Some(folder) = self.back.pop() else {
            return false;
        };
        self.forward.push(self.current);
        self.current = folder;
        if self.expand_ancestors(folder) {
            self.tree_revision += 1;
        }
        self.search.clear();
        self.revision += 1;
        true
    }

    pub fn go_forward(&mut self) -> bool {
        let Some(folder) = self.forward.pop() else {
            return false;
        };
        self.back.push(self.current);
        self.current = folder;
        if self.expand_ancestors(folder) {
            self.tree_revision += 1;
        }
        self.search.clear();
        self.revision += 1;
        true
    }

    pub fn set_search(&mut self, search: String) -> bool {
        if self.search == search {
            return false;
        }
        self.search = search;
        self.revision += 1;
        true
    }

    pub fn breadcrumbs(&self) -> Vec<usize> {
        let mut result = vec![self.current];
        let mut current = self.current;
        while let Some(parent) = self.folders[current].parent {
            result.push(parent);
            current = parent;
        }
        result.reverse();
        result
    }

    /// With an empty query, list direct children. Search includes descendants
    /// so an asset can be found without opening each nested folder in turn.
    pub fn entries(&self, catalog: &[CatalogItem]) -> Vec<Entry> {
        let query = self.search.trim().to_lowercase();
        let current = &self.folders[self.current];
        let mut entries: Vec<Entry> = if query.is_empty() {
            current
                .children
                .iter()
                .copied()
                .map(Entry::Folder)
                .collect()
        } else {
            self.folders
                .iter()
                .enumerate()
                .filter(|(index, folder)| {
                    *index != self.current
                        && folder.path.starts_with(&current.path)
                        && folder.label.to_lowercase().contains(&query)
                })
                .map(|(index, _)| Entry::Folder(index))
                .collect()
        };
        let mut models: Vec<_> = self
            .models
            .iter()
            .filter_map(|&(folder, index)| {
                let model = catalog.get(index)?;
                let matches = if query.is_empty() {
                    folder == self.current
                } else {
                    self.folders[folder].path.starts_with(&current.path)
                        && (model.label.to_lowercase().contains(&query)
                            || model.asset.to_lowercase().contains(&query)
                            || model.category.to_lowercase().contains(&query))
                };
                matches.then_some(index)
            })
            .collect();
        models.sort_by_key(|&index| catalog[index].label.to_lowercase());
        entries.extend(models.into_iter().map(Entry::Model));
        let mut files: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                if query.is_empty() {
                    file.folder == self.current
                } else {
                    self.folders[file.folder].path.starts_with(&current.path)
                        && (file.path.to_string_lossy().to_lowercase().contains(&query)
                            || file
                                .scenes
                                .iter()
                                .any(|scene| scene.label.to_lowercase().contains(&query)))
                }
            })
            .collect();
        files.sort_by_key(|(_, file)| file.filename.to_lowercase());
        for (index, file) in files {
            match file.kind {
                FileKind::Scene => {
                    let path_matches = query.is_empty()
                        || file.path.to_string_lossy().to_lowercase().contains(&query);
                    entries.extend(file.scenes.iter().filter_map(|scene| {
                        (path_matches || scene.label.to_lowercase().contains(&query)).then_some(
                            Entry::Scene {
                                file: index,
                                room: scene.room,
                            },
                        )
                    }));
                }
                FileKind::Image => entries.push(Entry::Image(index)),
                FileKind::Other => entries.push(Entry::File(index)),
            }
        }
        entries
    }
}

fn file_kind(extension: &str) -> FileKind {
    match extension.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tga" | "gif" | "ico" => FileKind::Image,
        _ => FileKind::Other,
    }
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name,
        "target" | "node_modules" | "build" | "dist" | "__pycache__"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bevy-editor-browser-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }

        fn file(&self, path: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, []).unwrap();
        }

        fn project(&self) -> EditorProject {
            EditorProject {
                title: "Test".into(),
                asset_root: self.0.clone(),
                scenes: Vec::new(),
                catalog: Vec::new(),
                scene_setup: None,
                camera_setup: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scene(label: &str, asset: &str, file: &str) -> crate::project::SceneDefinition {
        crate::project::SceneDefinition {
            label: label.into(),
            asset: asset.into(),
            file: file.into(),
            groups: Vec::new(),
            excluded_nodes: Vec::new(),
            node_policies: Default::default(),
            cameras: Vec::new(),
            initial_selection: None,
            play: None,
        }
    }

    #[test]
    fn discovery_preserves_catalog_metadata_and_shows_regular_files_without_cataloging_them() {
        let files = Fixture::new();
        for path in [
            "models/chair.glb",
            "models/new_table.glb",
            "models/script.rs",
            "models/readme.md",
            "models/unsupported.GLB",
            ".git/hidden.glb",
            "target/cache.glb",
            "app.js",
            "scenes/room.glb",
        ] {
            files.file(path);
        }
        let mut project = files.project();
        project
            .scenes
            .push(scene("Room", "scenes/room.glb", "scenes/room.json"));
        project.catalog.push(CatalogItem {
            label: "Кресло".into(),
            asset: "models/chair.glb".into(),
            category: "Мебель".into(),
            solid: true,
            preview_direction: [-1.0, 1.0, 2.0],
        });
        let mut browser = Browser::new(&mut project);
        assert_eq!(project.catalog.len(), 2);
        assert_eq!(project.catalog[0].label, "Кресло");
        assert_eq!(project.catalog[0].category, "Мебель");
        assert!(project.catalog[0].solid);
        assert_eq!(project.catalog[0].preview_direction, [-1.0, 1.0, 2.0]);
        assert_eq!(project.catalog[1].asset, "models/new_table.glb");
        let root = browser.entries(&project.catalog);
        assert!(root.contains(&Entry::Folder(1)));
        assert!(root.contains(&Entry::Folder(2)));
        assert!(root.iter().any(|entry| match *entry {
            Entry::File(index) => browser.files[index].filename == "app.js",
            _ => false,
        }));
        browser.navigate(1);
        let entries = browser.entries(&project.catalog);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, Entry::Model(_)))
                .count(),
            2
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, Entry::File(_)))
                .count(),
            3
        );
        assert!(
            browser
                .files
                .iter()
                .all(|file| !file.path.starts_with(".git") && !file.path.starts_with("target"))
        );
        assert!(browser.error.is_none());
    }

    #[test]
    fn source_and_saved_scene_files_open_the_same_scene_without_becoming_models() {
        let files = Fixture::new();
        for path in [
            "scenes/Workshop.glb",
            "levels/workshop.json",
            "levels/settings.json",
        ] {
            files.file(path);
        }
        fs::write(files.0.join("scenes/Workshop.glb"), [1, 2, 3]).unwrap();
        let mut project = files.project();
        project.scenes.push(scene(
            "Мастерская",
            "scenes/Workshop.glb",
            "levels/workshop.json",
        ));
        project.catalog.push(CatalogItem {
            label: "Former scene model".into(),
            asset: "scenes/Workshop.glb".into(),
            category: "Configured".into(),
            solid: false,
            preview_direction: [1.55, 1.3, 2.3],
        });
        let mut browser = Browser::new(&mut project);
        let source = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("scenes/Workshop.glb"))
            .unwrap();
        let saved = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("levels/workshop.json"))
            .unwrap();
        assert_eq!(browser.files[source].filename, "Workshop.glb");
        assert_eq!(browser.files[source].bytes, 3);
        assert_eq!(browser.files[source].kind, FileKind::Scene);
        assert_eq!(
            browser.model_asset(
                &project.catalog,
                Entry::Scene {
                    file: source,
                    room: 0
                }
            ),
            Some("scenes/Workshop.glb".into())
        );
        assert_eq!(
            browser.model_asset(&project.catalog, Entry::File(source)),
            Some("scenes/Workshop.glb".into())
        );
        assert_eq!(
            browser.model_asset(
                &project.catalog,
                Entry::Scene {
                    file: saved,
                    room: 0
                }
            ),
            None
        );
        assert_eq!(
            browser.model_asset(&project.catalog, Entry::Folder(0)),
            None
        );
        browser.navigate(browser.files[source].folder);
        assert_eq!(
            browser.entries(&project.catalog),
            vec![Entry::Scene {
                file: source,
                room: 0
            }]
        );
        browser.home();
        browser.set_search("мастерская".into());
        let found = browser.entries(&project.catalog);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&Entry::Scene {
            file: source,
            room: 0
        }));
        assert!(found.contains(&Entry::Scene {
            file: saved,
            room: 0
        }));
        browser.set_search(".json".into());
        let found = browser.entries(&project.catalog);
        assert!(found.contains(&Entry::Scene {
            file: saved,
            room: 0
        }));
        assert!(found.iter().any(|entry| matches!(entry, Entry::File(_))));
        assert!(!found.iter().any(|entry| matches!(entry, Entry::Model(_))));
        assert_eq!(
            project.catalog.len(),
            1,
            "scene files are never automatically cataloged"
        );
    }

    #[test]
    fn scenes_sharing_a_source_remain_distinct_and_searchable_by_scene_label() {
        let files = Fixture::new();
        files.file("scenes/shared.glb");
        files.file("levels/day.json");
        files.file("levels/night.json");
        let mut project = files.project();
        project.scenes.extend([
            scene("День", "scenes/shared.glb", "levels/day.json"),
            scene("Ночь", "scenes/shared.glb", "levels/night.json"),
        ]);
        let mut browser = Browser::new(&mut project);
        let source = browser
            .files
            .iter()
            .position(|file| file.path == Path::new("scenes/shared.glb"))
            .unwrap();
        browser.navigate(browser.files[source].folder);
        assert_eq!(
            browser.entries(&project.catalog),
            vec![
                Entry::Scene {
                    file: source,
                    room: 0
                },
                Entry::Scene {
                    file: source,
                    room: 1
                },
            ]
        );
        browser.set_search("ночь".into());
        assert_eq!(
            browser.entries(&project.catalog),
            vec![Entry::Scene {
                file: source,
                room: 1
            }]
        );
        browser.home();
        browser.set_search("день".into());
        let found = browser.entries(&project.catalog);
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .all(|entry| matches!(entry, Entry::Scene { room: 0, .. }))
        );
        assert!(project.catalog.is_empty());
    }

    #[test]
    fn refresh_discovers_saved_scene_documents_and_keeps_their_scene_identity() {
        let files = Fixture::new();
        files.file("scenes/room.glb");
        let mut project = files.project();
        project
            .scenes
            .push(scene("Room", "scenes/room.glb", "levels/room.json"));
        let mut browser = Browser::new(&mut project);
        browser.set_search("room".into());
        assert!(matches!(
            browser.entries(&project.catalog).as_slice(),
            [Entry::Scene { room: 0, .. }]
        ));
        files.file("levels/room.json");
        files.file("scenes/00-notes.txt");
        browser.refresh(&mut project);
        let found = browser.entries(&project.catalog);
        assert_eq!(found.len(), 2);
        let paths: HashSet<_> = found
            .into_iter()
            .map(|entry| {
                let Entry::Scene { file, room } = entry else {
                    panic!("expected scene file")
                };
                assert_eq!(room, 0);
                browser.files[file].path.clone()
            })
            .collect();
        assert_eq!(
            paths,
            HashSet::from([
                PathBuf::from("scenes/room.glb"),
                PathBuf::from("levels/room.json")
            ])
        );
        assert!(project.catalog.is_empty());
    }

    #[test]
    fn explicitly_configured_scenes_remain_visible_under_ignored_and_hidden_paths() {
        let files = Fixture::new();
        let configured = [
            "target/.hidden/room.glb",
            "target/.hidden/room.json",
            ".hidden_scene.glb",
            ".hidden_scene.json",
        ];
        for path in configured.into_iter().chain([
            "target/.hidden/unlisted.glb",
            "target/.hidden/unlisted.json",
            "target/cache.glb",
            ".hidden_other.glb",
        ]) {
            files.file(path);
        }
        let mut project = files.project();
        project.scenes.extend([
            scene("Nested", configured[0], configured[1]),
            scene("Hidden filename", configured[2], configured[3]),
        ]);
        let mut browser = Browser::new(&mut project);
        let visible: HashSet<_> = browser
            .files
            .iter()
            .map(|file| file.path.as_path())
            .collect();
        let expected: HashSet<_> = configured.into_iter().map(Path::new).collect();
        assert_eq!(visible, expected);
        assert!(
            browser
                .files
                .iter()
                .all(|file| file.kind == FileKind::Scene)
        );
        assert!(
            project.catalog.is_empty(),
            "unlisted GLBs in hidden paths stay excluded"
        );
        let folder = browser
            .folders
            .iter()
            .position(|folder| folder.path == Path::new("target/.hidden"))
            .unwrap();
        browser.navigate(folder);
        let entries = browser.entries(&project.catalog);
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| matches!(entry, Entry::Scene { room: 0, .. }))
        );
        browser.home();
        let entries = browser.entries(&project.catalog);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, Entry::Scene { room: 1, .. }))
                .count(),
            2
        );
    }

    #[test]
    fn image_and_generic_entries_preserve_real_filenames_extensions_and_metadata() {
        let files = Fixture::new();
        for path in [
            "textures/Brick_Wall.Albedo.PNG",
            "textures/Фото 01.JPG",
            "notes/material_settings.json",
            "LICENSE",
            "props/blue_block.glb",
        ] {
            files.file(path);
        }
        fs::write(files.0.join("textures/Brick_Wall.Albedo.PNG"), [1, 2, 3, 4]).unwrap();
        let mut project = files.project();
        let mut browser = Browser::new(&mut project);
        assert_eq!(project.catalog.len(), 1);
        assert!(
            browser
                .files
                .iter()
                .all(|file| file.path != Path::new("props/blue_block.glb")),
            "placeable models must not be listed twice"
        );
        let png = browser
            .files
            .iter()
            .position(|file| file.filename == "Brick_Wall.Albedo.PNG")
            .unwrap();
        assert_eq!(browser.files[png].extension, "PNG");
        assert_eq!(
            browser.files[png].path,
            Path::new("textures/Brick_Wall.Albedo.PNG")
        );
        assert_eq!(browser.files[png].bytes, 4);
        assert_eq!(browser.files[png].kind, FileKind::Image);
        let license = browser
            .files
            .iter()
            .find(|file| file.filename == "LICENSE")
            .unwrap();
        assert!(license.extension.is_empty());
        assert_eq!(license.kind, FileKind::Other);
        browser.set_search(".PNG".into());
        assert_eq!(browser.entries(&project.catalog), vec![Entry::Image(png)]);
        browser.set_search("фото 01".into());
        assert!(matches!(
            browser.entries(&project.catalog).as_slice(),
            [Entry::Image(_)]
        ));
        browser.set_search("material_settings.json".into());
        assert!(matches!(
            browser.entries(&project.catalog).as_slice(),
            [Entry::File(_)]
        ));
    }

    #[test]
    fn root_models_and_explicit_catalog_variants_remain_visible_in_ignored_directories() {
        let files = Fixture::new();
        for path in [
            "shared.glb",
            "other.glb",
            ".models/chair.glb",
            ".models/unlisted.glb",
        ] {
            files.file(path);
        }
        let mut project = files.project();
        for (label, asset) in [
            ("Decorative", "shared.glb"),
            ("Solid", "shared.glb"),
            ("Configured hidden model", ".models/chair.glb"),
        ] {
            project.catalog.push(CatalogItem {
                label: label.into(),
                asset: asset.into(),
                category: "Configured".into(),
                solid: label == "Solid",
                preview_direction: [1.55, 1.3, 2.3],
            });
        }
        let mut browser = Browser::new(&mut project);
        assert_eq!(
            project.catalog.len(),
            4,
            "do not discover unlisted files in a hidden directory"
        );
        let root = browser.entries(&project.catalog);
        assert!(root.contains(&Entry::Model(0)));
        assert!(root.contains(&Entry::Model(1)));
        assert!(root.contains(&Entry::Model(3)));
        assert!(project.catalog[1].solid);
        let hidden = browser
            .folders
            .iter()
            .position(|folder| folder.label == ".models")
            .unwrap();
        browser.navigate(hidden);
        assert_eq!(browser.entries(&project.catalog), vec![Entry::Model(2)]);
    }

    #[test]
    fn folder_history_breadcrumbs_and_recursive_search_stay_inside_the_project() {
        let files = Fixture::new();
        files.file("props/furniture/кресло.glb");
        files.file("props/lamp.glb");
        files.file("other/hidden.glb");
        let mut project = files.project();
        let mut browser = Browser::new(&mut project);
        let props = browser
            .folders
            .iter()
            .position(|folder| folder.label == "props")
            .unwrap();
        let furniture = browser
            .folders
            .iter()
            .position(|folder| folder.label == "furniture")
            .unwrap();
        assert!(browser.navigate(props));
        browser.set_search("КРЕСЛО".into());
        let chair = project
            .catalog
            .iter()
            .position(|model| model.asset.ends_with("кресло.glb"))
            .unwrap();
        assert_eq!(browser.entries(&project.catalog), vec![Entry::Model(chair)]);
        assert!(browser.navigate(furniture));
        assert!(browser.search.is_empty());
        assert_eq!(browser.breadcrumbs(), vec![0, props, furniture]);
        assert!(browser.go_back());
        assert_eq!(browser.current, props);
        assert!(browser.can_go_forward());
        assert!(browser.go_forward());
        assert!(browser.up());
        assert_eq!(browser.current, props);
        assert!(!browser.can_go_forward());
        assert!(!browser.navigate(usize::MAX));
        assert_eq!(browser.current, props);
        assert!(browser.home());
        assert!(!browser.up());
    }

    #[test]
    fn revealed_folder_navigation_and_history_update_content_without_rebuilding_the_tree() {
        let files = Fixture::new();
        files.file("props/chairs/wood/seat.glb");
        files.file("props/chairs/metal/stool.glb");
        let mut project = files.project();
        let mut browser = Browser::new(&mut project);
        let wood = browser
            .folders
            .iter()
            .position(|folder| folder.label == "wood")
            .unwrap();
        let metal = browser
            .folders
            .iter()
            .position(|folder| folder.label == "metal")
            .unwrap();
        assert!(browser.navigate(wood));
        let tree = browser.tree_revision;
        let mut content = browser.revision;
        let wood_entries = browser.entries(&project.catalog);

        assert!(browser.navigate(metal));
        content += 1;
        assert_eq!(browser.revision, content);
        assert_eq!(browser.tree_revision, tree);
        assert_ne!(browser.entries(&project.catalog), wood_entries);
        assert!(browser.go_back());
        content += 1;
        assert_eq!(browser.current, wood);
        assert_eq!(browser.revision, content);
        assert_eq!(browser.tree_revision, tree);
        assert_eq!(browser.entries(&project.catalog), wood_entries);
        assert!(browser.go_forward());
        content += 1;
        assert_eq!(browser.current, metal);
        assert_eq!(browser.revision, content);
        assert_eq!(browser.tree_revision, tree);

        assert!(browser.set_search("stool".into()));
        content += 1;
        assert!(browser.navigate(metal));
        content += 1;
        assert!(browser.search.is_empty());
        assert_eq!(browser.revision, content);
        assert_eq!(browser.tree_revision, tree);
        assert!(!browser.navigate(metal));
        assert_eq!(browser.revision, content);
        assert_eq!(browser.tree_revision, tree);
    }

    #[test]
    fn navigation_and_history_rebuild_the_tree_only_when_hidden_ancestors_are_revealed() {
        let files = Fixture::new();
        files.file("props/chairs/wood/seat.glb");
        files.file("other/table.glb");
        let mut project = files.project();
        let mut browser = Browser::new(&mut project);
        let find = |label: &str| {
            browser
                .folders
                .iter()
                .position(|folder| folder.label == label)
                .unwrap()
        };
        let props = find("props");
        let chairs = find("chairs");
        let wood = find("wood");
        let other = find("other");

        assert!(browser.navigate(wood));
        assert_eq!(browser.tree_revision, 1);
        assert!(browser.is_expanded(props));
        assert!(browser.is_expanded(chairs));
        assert!(browser.navigate(other));
        assert_eq!(browser.tree_revision, 1);
        assert!(browser.toggle_folder(props));
        let tree = browser.tree_revision;
        let content = browser.revision;
        assert!(browser.go_back());
        assert_eq!(browser.current, wood);
        assert!(browser.is_expanded(props));
        assert_eq!(browser.tree_revision, tree + 1);
        assert_eq!(browser.revision, content + 1);

        assert!(browser.go_back());
        assert_eq!(browser.current, 0);
        assert_eq!(browser.tree_revision, tree + 1);
        assert!(browser.toggle_folder(props));
        let tree = browser.tree_revision;
        let content = browser.revision;
        assert!(browser.go_forward());
        assert_eq!(browser.current, wood);
        assert!(browser.is_expanded(props));
        assert_eq!(browser.tree_revision, tree + 1);
        assert_eq!(browser.revision, content + 1);
    }

    #[test]
    fn refresh_discovers_new_models_and_falls_back_when_the_current_folder_is_removed() {
        let files = Fixture::new();
        files.file("props/chairs/seat.glb");
        let mut project = files.project();
        let mut browser = Browser::new(&mut project);
        let chairs = browser
            .folders
            .iter()
            .position(|folder| folder.label == "chairs")
            .unwrap();
        browser.navigate(chairs);
        browser.set_search("seat".into());
        fs::remove_dir_all(files.0.join("props/chairs")).unwrap();
        files.file("props/table.glb");
        let revision = browser.revision;
        let scan_revision = browser.scan_revision;
        browser.refresh(&mut project);
        assert_eq!(browser.folders[browser.current].path, Path::new("props"));
        assert_eq!(browser.search, "seat");
        assert_eq!(browser.revision, revision + 1);
        assert_eq!(browser.scan_revision, scan_revision + 1);
        assert_eq!(project.catalog.len(), 2);
        browser.set_search(String::new());
        assert_eq!(browser.entries(&project.catalog), vec![Entry::Model(1)]);
        assert_eq!(project.catalog[1].asset, "props/table.glb");
    }

    #[test]
    fn tree_starts_collapsed_and_navigation_reveals_only_the_destination_ancestors() {
        let files = Fixture::new();
        files.file("props/chairs/wood/seat.glb");
        files.file("other/table.glb");
        let mut project = files.project();
        let mut browser = Browser::new(&mut project);
        let find = |browser: &Browser, name: &str| {
            browser
                .folders
                .iter()
                .position(|folder| folder.label == name)
                .unwrap()
        };
        let props = find(&browser, "props");
        let chairs = find(&browser, "chairs");
        let wood = find(&browser, "wood");
        let other = find(&browser, "other");
        assert!(browser.is_expanded(0));
        assert!(browser.expanded.is_empty());
        assert!(!browser.is_expanded(props));
        assert!(!browser.toggle_folder(0));
        browser.navigate(wood);
        assert!(browser.is_expanded(props));
        assert!(browser.is_expanded(chairs));
        assert!(!browser.is_expanded(wood));
        assert!(!browser.is_expanded(other));
        let content = browser.revision;
        let tree = browser.tree_revision;
        let scan = browser.scan_revision;
        assert!(browser.toggle_folder(props));
        assert!(!browser.is_expanded(props));
        assert_eq!(
            browser.revision, content,
            "collapsing must not reset grid content"
        );
        assert_eq!(browser.tree_revision, tree + 1);
        assert_eq!(browser.scan_revision, scan);
        browser.navigate(wood);
        assert!(
            browser.is_expanded(props),
            "revealing the current folder reopens its ancestors"
        );

        // New alphabetically earlier paths shift folder IDs, but a refresh
        // preserves expanded branches by their paths rather than stale IDs.
        files.file("aaa/new.glb");
        browser.refresh(&mut project);
        assert!(browser.is_expanded(find(&browser, "props")));
        assert!(browser.is_expanded(find(&browser, "chairs")));
        assert!(!browser.is_expanded(find(&browser, "other")));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_and_folders_are_not_scanned() {
        let files = Fixture::new();
        let outside = Fixture::new();
        outside.file("outside.glb");
        std::os::unix::fs::symlink(&outside.0, files.0.join("escape")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("outside.glb"), files.0.join("linked.glb"))
            .unwrap();
        let mut project = files.project();
        let browser = Browser::new(&mut project);
        assert_eq!(browser.folders.len(), 1);
        assert!(project.catalog.is_empty());
        assert!(browser.entries(&project.catalog).is_empty());
    }
}
