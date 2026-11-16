use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bevy-duplicate-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, bytes: &[u8]) {
        fs::write(self.0.join(path), bytes).unwrap();
    }
    fn clean_staging(&self) {
        assert!(fs::read_dir(&self.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".bevy-editor-copy-")
        }));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn duplicate_file_preserves_original_bytes_and_import_settings_with_unique_names() {
    let files = Fixture::new();
    files.write("Куб.glb", b"model bytes and relative texture URIs");
    files.write("Куб.glb.meta", b"loader settings");
    let first = duplicate(&files.0, Path::new("Куб.glb")).unwrap();
    let second = duplicate(&files.0, Path::new("Куб.glb")).unwrap();
    assert_eq!(first, Path::new("Куб — копия.glb"));
    assert_eq!(second, Path::new("Куб — копия (2).glb"));
    for path in [&first, &second] {
        assert_eq!(
            fs::read(files.0.join(path)).unwrap(),
            fs::read(files.0.join("Куб.glb")).unwrap()
        );
        assert_eq!(
            fs::read(files.0.join(file_moves::meta_path(path))).unwrap(),
            b"loader settings"
        );
    }
    assert!(!files.0.join(crate::asset_paths::FILE).exists());
    files.clean_staging();
}

#[test]
fn duplicate_file_reserves_orphan_sidecars_and_retained_asset_aliases() {
    let files = Fixture::new();
    files.write("notes.txt", b"original");
    files.write("notes — копия.txt.meta", b"keep orphan");
    files.write("other.txt", b"other file");
    let mut redirects = crate::asset_paths::Redirects::default();
    redirects
        .record_move(Path::new("notes — копия (2).txt"), Path::new("other.txt"))
        .unwrap();
    fs::write(
        files.0.join(crate::asset_paths::FILE),
        redirects.encode().unwrap(),
    )
    .unwrap();
    let path = duplicate(&files.0, Path::new("notes.txt")).unwrap();
    assert_eq!(path, Path::new("notes — копия (3).txt"));
    assert_eq!(
        fs::read(files.0.join("notes — копия.txt.meta")).unwrap(),
        b"keep orphan"
    );
    assert_eq!(fs::read(files.0.join("other.txt")).unwrap(), b"other file");
    files.clean_staging();
}

#[test]
fn duplicate_file_copies_native_image_layers_and_retargets_only_the_manifest_name() {
    let files = Fixture::new();
    let source = files.0.join("photo.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([31, 78, 152, 255]))
        .save(&source)
        .unwrap();
    let document = crate::editor::photo_document::PhotoDocument::load(&source).unwrap();
    let source_image = fs::read(&source).unwrap();
    let source_native = fs::read(files.0.join("photo.pcraft")).unwrap();
    // A native file with no matching image must not be replaced by a new copy.
    files.write("photo — копия.pcraft", b"keep orphan layers");
    let to = duplicate(&files.0, Path::new("photo.png")).unwrap();
    assert_eq!(to, Path::new("photo — копия (2).png"));
    assert_eq!(fs::read(files.0.join(&to)).unwrap(), source_image);
    assert_eq!(
        fs::read(files.0.join("photo.pcraft")).unwrap(),
        source_native
    );
    let copied = crate::editor::photo_document::PhotoDocument::load(&files.0.join(&to)).unwrap();
    assert_eq!(copied.document.name, "photo — копия (2).png");
    assert_eq!(copied.document.id, document.document.id);
    let original = photocraft_format::zip::ZipReader::new(&source_native).unwrap();
    let copied_native = fs::read(files.0.join(to.with_extension("pcraft"))).unwrap();
    let copied_archive = photocraft_format::zip::ZipReader::new(&copied_native).unwrap();
    for entry in original
        .entries
        .iter()
        .filter(|entry| entry.name != "manifest.json")
    {
        assert_eq!(
            original.read(entry, 1 << 20).unwrap(),
            copied_archive.read_by_name(&entry.name, 1 << 20).unwrap()
        );
    }
    files.clean_staging();
}

#[test]
fn duplicate_file_rejects_invalid_sources_and_corrupt_layers_without_publishing_a_copy() {
    let files = Fixture::new();
    for source in ["../outside.txt", "missing.txt", ""] {
        assert!(duplicate(&files.0, Path::new(source)).is_err());
    }
    files.write("image.png", b"saved image");
    files.write("image.pcraft", b"invalid native bundle");
    assert!(duplicate(&files.0, Path::new("image.png")).is_err());
    assert!(!files.0.join("image — копия.png").exists());
    assert_eq!(fs::read(files.0.join("image.png")).unwrap(), b"saved image");
    files.clean_staging();
}

#[test]
fn duplicate_file_handles_simultaneous_requests_without_overwriting_copies() {
    let files = Fixture::new();
    files.write("README", b"no extension");
    files.write("README.meta", b"metadata");
    let workers: Vec<_> = (0..6)
        .map(|_| {
            let root = files.0.clone();
            std::thread::spawn(move || duplicate(&root, Path::new("README")).unwrap())
        })
        .collect();
    let paths: std::collections::BTreeSet<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(paths.len(), 6);
    for path in paths {
        assert_eq!(fs::read(files.0.join(&path)).unwrap(), b"no extension");
        assert_eq!(
            fs::read(files.0.join(file_moves::meta_path(&path))).unwrap(),
            b"metadata"
        );
    }
    files.clean_staging();
}
