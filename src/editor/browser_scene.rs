//! Empty native scene documents, created without touching shared model assets.
use bevy::prelude::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(super) fn stem(name: &str) -> Result<String, String> {
    let mut name = name.to_owned();
    if name.to_ascii_lowercase().ends_with(".scene") {
        name.truncate(name.len() - 6);
    }
    if name.trim().is_empty() {
        return Err("Введите название сцены".into());
    }
    super::browser_files::validate_name(&name).map_err(str::to_owned)?;
    Ok(name)
}

pub(super) fn suggested_name(root: &Path, parent: &Path) -> String {
    for suffix in 1..10_000 {
        let name = if suffix == 1 {
            "Новая сцена".into()
        } else {
            format!("Новая сцена ({suffix})")
        };
        if !root.join(parent).join(format!("{name}.scene")).exists() {
            return name;
        }
    }
    "Новая сцена".into()
}

pub(super) fn create(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, String> {
    let stem = stem(name)?;
    let directory = super::browser_files::directory(root, parent)
        .map_err(|error| format!("Не удалось открыть папку: {error}"))?;
    for suffix in 1..10_000 {
        let label = if suffix == 1 {
            stem.clone()
        } else {
            format!("{stem} ({suffix})")
        };
        let filename = format!("{label}.scene");
        let document = crate::LevelEdits::<String> {
            scene: Some(crate::SceneSettings {
                label,
                cameras: vec![crate::CameraDefinition {
                    id: "main".into(),
                    label: "Главная камера".into(),
                    placement: crate::CameraPlacement {
                        pose: Transform::from_xyz(5.0, 4.0, 7.0)
                            .looking_at(Vec3::ZERO, Vec3::Y)
                            .into(),
                        lens: 50.0,
                    },
                    projection: crate::LensKind::Perspective,
                }],
            }),
            lighting: Some(crate::SceneLighting::editor_default()),
            ..default()
        };
        document.validate(root).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
        let destination = directory.join(&filename);
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Не удалось создать сцену: {error}")),
        };
        let result = file
            .write_all(&bytes)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all());
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&destination);
            return Err(format!("Не удалось записать сцену: {error}"));
        }
        return Ok(parent.join(filename));
    }
    Err("Не удалось подобрать свободное имя сцены".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "bevy-browser-scene-{}",
                bevy::asset::uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(root.join("scenes")).unwrap();
            Self(root)
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn browser_scene_creation_writes_a_discoverable_empty_native_document() {
        let files = Files::new();
        let path = create(&files.0, Path::new("scenes"), "Новая локация.SCENE").unwrap();
        assert_eq!(path, PathBuf::from("scenes/Новая локация.scene"));
        let document =
            crate::LevelEdits::<String>::read(&files.0, &path.to_string_lossy().replace('\\', "/"))
                .unwrap();
        assert!(
            document.models.is_empty() && document.nodes.is_empty() && document.lights.is_empty()
        );
        let settings = document.scene.unwrap();
        assert_eq!(settings.label, "Новая локация");
        assert_eq!(settings.cameras.len(), 1);
        assert_eq!(settings.cameras[0].id, "main");
        assert!(settings.cameras[0].placement.pose.valid());
        assert!(document.lighting.is_some());
        let mut project = crate::EditorProject::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        project.asset_root = files.0.clone();
        let browser = crate::editor::browser::Browser::new(&mut project);
        let scene = project
            .scenes
            .iter()
            .find(|scene| Path::new(&scene.file) == path)
            .unwrap();
        assert!(scene.is_native());
        assert!(
            browser
                .files
                .iter()
                .any(|file| file.path == path
                    && file.kind == crate::editor::browser::FileKind::Scene)
        );
    }

    #[test]
    fn browser_scene_creation_preserves_existing_files_and_handles_concurrent_names() {
        let files = Files::new();
        let parent = Path::new("scenes");
        let first = create(&files.0, parent, "Новая сцена").unwrap();
        let original = fs::read(files.0.join(&first)).unwrap();
        assert_eq!(suggested_name(&files.0, parent), "Новая сцена (2)");
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(|| create(&files.0, parent, "Новая сцена").unwrap());
            let b = scope.spawn(|| create(&files.0, parent, "Новая сцена.scene").unwrap());
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_ne!(a, b);
        assert_ne!(a, first);
        assert_ne!(b, first);
        assert_eq!(fs::read(files.0.join(first)).unwrap(), original);
        for path in [a, b] {
            let doc = crate::LevelEdits::<String>::read(
                &files.0,
                &path.to_string_lossy().replace('\\', "/"),
            )
            .unwrap();
            assert_eq!(
                doc.scene.unwrap().label,
                path.file_stem().unwrap().to_str().unwrap()
            );
        }
    }

    #[test]
    fn browser_scene_creation_rejects_invalid_names_and_folders() {
        let files = Files::new();
        for name in [
            "",
            " ",
            ".scene",
            "../outside",
            ".hidden",
            "CON",
            "wrong/name",
            "trailing.",
        ] {
            assert!(
                create(&files.0, Path::new("scenes"), name).is_err(),
                "{name}"
            );
        }
        for parent in ["../outside", "scenes/../scenes", "missing"] {
            assert!(create(&files.0, Path::new(parent), "Мир").is_err());
        }
        assert_eq!(fs::read_dir(files.0.join("scenes")).unwrap().count(), 0);
    }
}
