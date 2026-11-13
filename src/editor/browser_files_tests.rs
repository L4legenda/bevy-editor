use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bevy-browser-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("assets/textures")).unwrap();
        fs::create_dir_all(root.join("external")).unwrap();
        Self(root)
    }
    fn assets(&self) -> PathBuf {
        self.0.join("assets")
    }
    fn source(&self, name: &str, data: &[u8]) -> PathBuf {
        let path = self.0.join("external").join(name);
        fs::write(&path, data).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn created_png_has_the_requested_size_and_opaque_white_pixels() {
    let fixture = Fixture::new();
    let path = create_image(
        &fixture.assets(),
        Path::new("textures"),
        "Белый холст.PNG",
        37,
        19,
    )
    .unwrap();
    assert_eq!(path, PathBuf::from("textures/Белый холст.png"));
    let image = image::open(fixture.assets().join(path))
        .unwrap()
        .into_rgba8();
    assert_eq!(image.dimensions(), (37, 19));
    assert!(image.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
    let tiny = create_image(&fixture.assets(), Path::new(""), "Точка", 1, 1).unwrap();
    assert_eq!(
        image::image_dimensions(fixture.assets().join(tiny)).unwrap(),
        (1, 1)
    );
}

#[test]
fn image_creation_does_not_overwrite_existing_files_even_with_concurrent_requests() {
    let fixture = Fixture::new();
    let root = fixture.assets();
    fs::write(
        root.join("textures/Новое изображение.png"),
        b"original artwork",
    )
    .unwrap();
    assert_eq!(
        suggested_image_name(&root, Path::new("textures")),
        "Новое изображение (2)"
    );
    let paths = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            create_image(&root, Path::new("textures"), "Новое изображение", 12, 10).unwrap()
        });
        let b = scope.spawn(|| {
            create_image(&root, Path::new("textures"), "Новое изображение", 12, 10).unwrap()
        });
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_ne!(paths[0], paths[1]);
    assert_eq!(
        fs::read(root.join("textures/Новое изображение.png")).unwrap(),
        b"original artwork"
    );
    for path in paths {
        assert_eq!(image::image_dimensions(root.join(path)).unwrap(), (12, 10));
    }
}

#[test]
fn invalid_image_dimensions_names_and_destinations_create_no_files() {
    let fixture = Fixture::new();
    let root = fixture.assets();
    for (width, height) in [
        (0, 100),
        (10, 0),
        (8193, 1),
        (8192, 8192),
        (u32::MAX, u32::MAX),
    ] {
        assert!(create_image(&root, Path::new("textures"), "Bad", width, height).is_err());
    }
    for name in ["", " ", "../outside", "a/b", "NUL", ".png"] {
        assert!(create_image(&root, Path::new("textures"), name, 10, 10).is_err());
    }
    for parent in [Path::new("../external"), fixture.0.as_path()] {
        assert!(create_image(&root, parent, "Outside", 10, 10).is_err());
    }
    assert!(validate_image_size(4096, 4096).is_ok());
    assert!(validate_image_size(8192, 2048).is_ok());
    assert_eq!(fs::read_dir(root.join("textures")).unwrap().count(), 0);
    assert!(!fixture.0.join("external/Outside.png").exists());
}

#[test]
fn browser_import_copies_multiple_images_and_preserves_sources_and_existing_files() {
    let fixture = Fixture::new();
    let source = fixture.source("Текстура.PNG", b"new image bytes");
    fs::write(
        fixture.assets().join("textures/Текстура.PNG"),
        b"existing image",
    )
    .unwrap();
    let svg = fixture.source("icon.svg", b"svg content");
    let report = import_images(
        &fixture.assets(),
        Path::new("textures"),
        vec![source.clone(), svg.clone()],
    );
    assert!(report.errors.is_empty());
    assert_eq!(report.imported.len(), 2);
    assert_eq!(
        fs::read(fixture.assets().join("textures/Текстура.PNG")).unwrap(),
        b"existing image"
    );
    assert_eq!(
        fs::read(fixture.assets().join("textures/Текстура (2).PNG")).unwrap(),
        b"new image bytes"
    );
    assert_eq!(fs::read(&source).unwrap(), b"new image bytes");
    assert_eq!(fs::read(&svg).unwrap(), b"svg content");
    assert!(
        report
            .imported
            .iter()
            .all(|path| path.starts_with("textures"))
    );
}

#[test]
fn browser_import_rejects_non_images_and_missing_files_without_stopping_the_batch() {
    let fixture = Fixture::new();
    let image = fixture.source("photo.jpg", b"jpeg");
    let text = fixture.source("notes.txt", b"important document");
    let missing = fixture.0.join("missing.png");
    let report = import_images(
        &fixture.assets(),
        Path::new(""),
        vec![text.clone(), missing, image],
    );
    assert_eq!(report.imported, vec![PathBuf::from("photo.jpg")]);
    assert_eq!(report.errors.len(), 2);
    assert!(!fixture.assets().join("notes.txt").exists());
    assert!(text.exists());
    assert!(report.status().contains("1"));
}

#[test]
fn browser_import_skips_repeated_sources_and_files_already_in_the_destination() {
    let fixture = Fixture::new();
    let source = fixture.source("photo.webp", b"webp");
    let report = import_images(
        &fixture.assets(),
        Path::new("textures"),
        vec![source.clone(), source],
    );
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.skipped, 1);
    let report = import_images(
        &fixture.assets(),
        Path::new("textures"),
        vec![fixture.assets().join("textures/photo.webp")],
    );
    assert!(report.imported.is_empty());
    assert_eq!(report.skipped, 1);
    assert!(!fixture.assets().join("textures/photo (2).webp").exists());
}

#[test]
fn browser_new_folder_uses_the_selected_parent_and_never_replaces_an_existing_entry() {
    let fixture = Fixture::new();
    let path = create_folder(&fixture.assets(), Path::new("textures"), "Картинки").unwrap();
    assert_eq!(path, PathBuf::from("textures/Картинки"));
    assert!(fixture.assets().join(&path).is_dir());
    fs::write(fixture.assets().join(&path).join("keep.txt"), b"keep").unwrap();
    assert!(create_folder(&fixture.assets(), Path::new("textures"), "Картинки").is_err());
    assert_eq!(
        fs::read(fixture.assets().join(path).join("keep.txt")).unwrap(),
        b"keep"
    );
    fs::write(fixture.assets().join("textures/file"), b"keep").unwrap();
    assert!(create_folder(&fixture.assets(), Path::new("textures"), "file").is_err());
    assert!(create_folder(&fixture.assets(), Path::new(""), "Новая папка").is_ok());
    assert_eq!(
        suggested_name(&fixture.assets(), Path::new("")),
        "Новая папка (2)"
    );
}

#[test]
fn browser_folder_names_reject_hidden_paths_and_nonportable_names() {
    for name in [
        "",
        " ",
        ".",
        "..",
        ".hidden",
        "../escape",
        "folder/name",
        "folder\\name",
        "C:drive",
        "trailing.",
        " trailing",
        "trailing ",
        "CON",
        "con.txt",
        "NUL",
        "COM1",
        "lpt9",
        "bad?name",
        "line\nfeed",
    ] {
        assert!(
            validate_name(name).is_err(),
            "Accepted invalid folder name: {name:?}"
        );
    }
    for name in [
        "Картинки",
        "Textures 2",
        "texture.pack",
        "COM10",
        "AUXiliary",
    ] {
        assert!(validate_name(name).is_ok());
    }
}

#[test]
fn browser_writes_cannot_escape_the_asset_root() {
    let fixture = Fixture::new();
    let source = fixture.source("photo.png", b"png");
    for parent in [Path::new("../external"), fixture.0.as_path()] {
        assert!(create_folder(&fixture.assets(), parent, "escape").is_err());
        let report = import_images(&fixture.assets(), parent, vec![source.clone()]);
        assert!(report.imported.is_empty());
        assert_eq!(report.errors.len(), 1);
    }
    assert!(!fixture.0.join("external/escape").exists());
}
