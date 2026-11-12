//! Filesystem operations for the project manager. Project assets are never
//! removed when an entry is forgotten from the recent-project list.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    CameraDefinition, CatalogItem, EditorProject, LensKind, LevelEdits, SceneDefinition,
    project::{PROJECT_FILE_NAME, resolve_project_file},
};

pub(super) fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs())
}

pub(super) fn config_file() -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = home()?.join("Library/Application Support/BevyEditor");
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or(home()?.join("AppData/Roaming"))
        .join("BevyEditor");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".config"))
        .join("bevy-editor");
    Ok(base.join("recent-projects.json"))
}

pub(super) fn home() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Не найдена домашняя папка пользователя"))
}

pub(super) fn input_path(value: &str) -> io::Result<PathBuf> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if value.is_empty() {
        return Err(io::Error::other("Укажите папку проекта"));
    }
    if value == "~" {
        return home();
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return Ok(home()?.join(rest));
    }
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RecentProject {
    pub file: PathBuf,
    pub title: String,
    #[serde(default)]
    pub last_opened: u64,
    #[serde(default)]
    pub favorite: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct RecentProjects {
    pub projects: Vec<RecentProject>,
}

impl RecentProjects {
    pub fn read(file: &Path) -> io::Result<Self> {
        match fs::read(file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, file: &Path) -> io::Result<()> {
        let parent = file
            .parent()
            .ok_or_else(|| io::Error::other("Некорректный путь истории"))?;
        fs::create_dir_all(parent)?;
        // Preserve an unreadable history before replacing it with a valid list.
        if let Ok(bytes) = fs::read(file)
            && serde_json::from_slice::<Self>(&bytes).is_err()
        {
            let backup = parent.join(format!("recent-projects-{}.backup.json", timestamp()));
            if !backup.exists() {
                fs::write(backup, bytes)?;
            }
        }
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let temp = parent.join(format!(
            ".recent-{}-{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            serde_json::to_writer_pretty(&mut output, self).map_err(io::Error::other)?;
            output.write_all(b"\n")?;
            output.sync_all()?;
            drop(output);
            fs::rename(&temp, file)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }

    pub fn remember(&mut self, file: &Path, title: &str) {
        if let Some(project) = self
            .projects
            .iter_mut()
            .find(|project| project.file == file)
        {
            project.title = title.into();
            project.last_opened = timestamp();
        } else {
            self.projects.push(RecentProject {
                file: file.to_owned(),
                title: title.into(),
                last_opened: timestamp(),
                favorite: false,
            });
        }
        self.projects
            .sort_by_key(|project| std::cmp::Reverse((project.favorite, project.last_opened)));
        self.projects.truncate(100);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Template {
    Empty,
    #[default]
    Starter,
}

pub(super) fn folder_name(title: &str) -> String {
    let name: String = title
        .trim()
        .chars()
        .map(|value| {
            if value.is_control() || "<>:\"/\\|?*".contains(value) {
                '-'
            } else {
                value
            }
        })
        .take(80)
        .collect();
    let name = name.trim_matches(['.', ' ']);
    if name.is_empty() {
        "New Project".into()
    } else {
        name.into()
    }
}

/// Create only in an absent or empty directory. A manifest is published last;
/// rollback removes only files and directories created by this operation.
pub(super) fn create_project(
    directory: &Path,
    title: &str,
    template: Template,
) -> io::Result<PathBuf> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 128 || title.chars().any(char::is_control) {
        return Err(io::Error::other(
            "Название должно содержать от 1 до 128 символов",
        ));
    }
    let created_root = match fs::create_dir(directory) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = directory.parent().ok_or(error)?;
            fs::create_dir_all(parent)?;
            fs::create_dir(directory)?;
            true
        }
        Err(error) => return Err(error),
    };
    if !directory.is_dir() || fs::read_dir(directory)?.next().is_some() {
        return Err(io::Error::other(
            "Выберите пустую папку или создайте новую подпапку: существующие файлы проекта не перезаписываются",
        ));
    }
    let mut files = Vec::new();
    let mut folders = Vec::new();
    let result = (|| {
        for folder in ["assets", "assets/scenes", "assets/models", "assets/levels"] {
            let path = directory.join(folder);
            fs::create_dir(&path)?;
            folders.push(path);
        }
        let mut write = |relative: &str, bytes: &[u8]| -> io::Result<()> {
            let path = directory.join(relative);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            files.push(path);
            file.write_all(bytes)?;
            file.sync_all()
        };
        write("assets/scenes/main.glb", &scene_glb(template))?;
        write("assets/models/cube.glb", &cube_glb())?;
        let view = Transform::from_xyz(5.0, 4.0, 7.0).looking_at(Vec3::ZERO, Vec3::Y);
        let project = EditorProject {
            title: title.into(),
            asset_root: "assets".into(),
            scenes: vec![SceneDefinition {
                label: "Главная сцена".into(),
                asset: "scenes/main.glb".into(),
                file: "levels/main.json".into(),
                groups: vec!["Objects".into()],
                excluded_nodes: Vec::new(),
                node_policies: Default::default(),
                cameras: vec![CameraDefinition {
                    id: "main".into(),
                    label: "Главная камера".into(),
                    placement: crate::CameraPlacement {
                        pose: view.into(),
                        lens: 50.0,
                    },
                    projection: LensKind::Perspective,
                }],
                initial_selection: (template == Template::Starter).then(|| "Cube".into()),
                play: None,
            }],
            catalog: vec![CatalogItem {
                label: "Куб".into(),
                asset: "models/cube.glb".into(),
                category: "Примитивы".into(),
                solid: true,
                preview_direction: [1.55, 1.3, 2.3],
            }],
            scene_setup: None,
            camera_setup: None,
        };
        let mut edits = LevelEdits::<String>::default();
        if template == Template::Starter {
            let (environment, lights) = crate::LightingPreset::Daylight.create(1);
            edits.lighting = Some(environment);
            edits.lights = lights;
        }
        write(
            "assets/levels/main.json",
            &serde_json::to_vec_pretty(&edits).map_err(io::Error::other)?,
        )?;
        write(
            PROJECT_FILE_NAME,
            &serde_json::to_vec_pretty(&project).map_err(io::Error::other)?,
        )?;
        let file = resolve_project_file(directory)?;
        EditorProject::load(&file)?;
        Ok(file)
    })();
    if result.is_err() {
        for file in files.iter().rev() {
            let _ = fs::remove_file(file);
        }
        for folder in folders.iter().rev() {
            let _ = fs::remove_dir(folder);
        }
        if created_root {
            let _ = fs::remove_dir(directory);
        }
    }
    result
}

fn scene_glb(template: Template) -> Vec<u8> {
    if template == Template::Empty {
        return encode_glb(
            serde_json::json!({"asset":{"version":"2.0","generator":"Bevy Editor"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"name":"Objects"}]}),
            Vec::new(),
        );
    }
    geometry_glb(false)
}

fn cube_glb() -> Vec<u8> {
    geometry_glb(true)
}

fn geometry_glb(cube_only: bool) -> Vec<u8> {
    let mut positions = Vec::<f32>::new();
    let mut normals = Vec::<f32>::new();
    let mut indices = Vec::<u16>::new();
    for (normal, right, up) in [
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, -Vec3::X, Vec3::Y),
        (Vec3::X, -Vec3::Z, Vec3::Y),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, -Vec3::Z),
        (-Vec3::Y, Vec3::X, Vec3::Z),
    ] {
        let base = (positions.len() / 3) as u16;
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.extend_from_slice(&((normal + right * x + up * y) * 0.5).to_array());
            normals.extend_from_slice(&normal.to_array());
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut bytes = Vec::new();
    for value in &positions {
        bytes.extend(value.to_le_bytes());
    }
    let normal_offset = bytes.len();
    for value in &normals {
        bytes.extend(value.to_le_bytes());
    }
    let index_offset = bytes.len();
    for value in &indices {
        bytes.extend(value.to_le_bytes());
    }
    let primitive = |material: u32| serde_json::json!({"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":material});
    let mut json = serde_json::json!({
        "asset":{"version":"2.0","generator":"Bevy Editor"}, "scene":0,
        "scenes":[{"nodes":[0]}], "nodes":[{"name":"Cube","mesh":0}],
        "buffers":[{"byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":normal_offset,"target":34962},{"buffer":0,"byteOffset":normal_offset,"byteLength":index_offset-normal_offset,"target":34962},{"buffer":0,"byteOffset":index_offset,"byteLength":indices.len()*2,"target":34963}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":24,"type":"VEC3","min":[-0.5,-0.5,-0.5],"max":[0.5,0.5,0.5]},{"bufferView":1,"componentType":5126,"count":24,"type":"VEC3"},{"bufferView":2,"componentType":5123,"count":36,"type":"SCALAR"}],
        "meshes":[{"primitives":[primitive(0)]}],
        "materials":[{"name":"Warm clay","pbrMetallicRoughness":{"baseColorFactor":[0.65,0.32,0.12,1],"metallicFactor":0,"roughnessFactor":0.7}}]
    });
    if !cube_only {
        json["nodes"] = serde_json::json!([
            {"name":"Objects","children":[1]}, {"name":"Cube","mesh":0,"translation":[0,0.5,0]},
            {"name":"Ground","mesh":1,"translation":[0,-0.05,0],"scale":[12,0.1,12]}
        ]);
        json["scenes"][0]["nodes"] = serde_json::json!([0, 2]);
        json["meshes"] =
            serde_json::json!([{"primitives":[primitive(0)]},{"primitives":[primitive(1)]}]);
        json["materials"].as_array_mut().expect("literal materials array").push(serde_json::json!({"name":"Ground","pbrMetallicRoughness":{"baseColorFactor":[0.25,0.28,0.33,1],"metallicFactor":0,"roughnessFactor":0.9}}));
    }
    encode_glb(json, bytes)
}

fn encode_glb(value: serde_json::Value, mut binary: Vec<u8>) -> Vec<u8> {
    let mut json = value.to_string().into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while binary.len() % 4 != 0 {
        binary.push(0);
    }
    let size = 12
        + 8
        + json.len()
        + if binary.is_empty() {
            0
        } else {
            8 + binary.len()
        };
    let mut glb = Vec::with_capacity(size);
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend((size as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(json);
    if !binary.is_empty() {
        glb.extend((binary.len() as u32).to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend(binary);
    }
    glb
}
