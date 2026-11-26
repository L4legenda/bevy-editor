use super::{scan::Scanner, *};
use std::time::Instant;

struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bevy-asset-index-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn file(&self, path: &str, bytes: &[u8]) {
        let abs = self.0.join(path);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, bytes).unwrap();
    }
    fn id(&self, path: &str) -> String {
        index(&self.0).unwrap().uid(path).unwrap().to_owned()
    }
    fn scan(&self) -> Scanner {
        Scanner::new(&self.0).unwrap()
    }
    fn rename(&self, from: &str, to: &str) {
        fs::rename(self.0.join(from), self.0.join(to)).unwrap();
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        INDICES.write().unwrap().remove(&self.0);
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn immediate_move_after_atomic_save_updates_all_windows() {
    let p = Project::new();
    p.file("original.png", b"before");
    let mut first = p.scan();
    first.run(None).unwrap();
    let uid = p.id("original.png");
    let mut other = p.scan();
    other.run(None).unwrap();
    p.file("save.tmp", b"new pixels");
    p.rename("save.tmp", "original.png");
    scan::record_save(&p.0, &p.0.join("original.png")).unwrap();
    p.rename("original.png", "moved.png");
    let moves = vec![("original.png".to_owned(), "moved.png".to_owned())];
    assert_eq!(first.run(None).unwrap().moves, moves);
    assert_eq!(other.run(None).unwrap().moves, moves);
    assert_eq!(p.id("moved.png"), uid);
}
#[test]
fn paired_native_rename_survives_external_atomic_replace() {
    let p = Project::new();
    p.file("old.fbx", b"before");
    let mut scan = p.scan();
    scan.run(None).unwrap();
    let id = p.id("old.fbx");
    p.file("save.tmp", b"replacement");
    p.rename("save.tmp", "old.fbx");
    p.rename("old.fbx", "new.fbx");
    scan.events(None, &[("old.fbx".into(), "new.fbx".into())])
        .unwrap();
    assert_eq!(id, p.id("new.fbx"));
}
#[test]
fn copies_replacements_and_reuse_of_old_name() {
    let p = Project::new();
    p.file("a.glb", b"a model");
    p.file("texture.png", b"image");
    let mut scan = p.scan();
    assert_eq!(scan.run(None).unwrap().hashed, 12);
    let a = p.id("a.glb");
    let portable = fs::read(p.0.join(FILE)).unwrap();
    assert_eq!(p.scan().run(None).unwrap().hashed, 0);
    assert_eq!(fs::read(p.0.join(FILE)).unwrap(), portable);
    fs::copy(p.0.join("a.glb"), p.0.join("copy.glb")).unwrap();
    scan.run(None).unwrap();
    assert_ne!(p.id("copy.glb"), a);
    p.rename("a.glb", "chair.glb");
    p.file("a.glb", b"replacement");
    let changes = scan.run(None).unwrap();
    assert_eq!(changes.hashed, 11);
    assert_eq!(p.id("chair.glb"), a);
    assert_ne!(p.id("a.glb"), a);
    assert_eq!(index(&p.0).unwrap().resolve("a.glb", Some(&a)), "chair.glb");
    p.file("atomic.tmp", b"another revision");
    fs::rename(p.0.join("atomic.tmp"), p.0.join("chair.glb")).unwrap();
    scan.run(None).unwrap();
    assert_eq!(p.id("chair.glb"), a);
    p.rename("chair.glb", "Chair.GLB");
    scan.run(None).unwrap();
    assert_eq!(p.id("Chair.GLB"), a);
    assert!(crate::data::ModelFormat::from_path("Chair.GLB").is_some());
}
#[test]
fn cacheless_checkout_ambiguous_duplicates_and_explicit_relink() {
    let p = Project::new();
    p.file("t.png", b"same");
    p.file("chair.glb", b"unique");
    p.scan().run(None).unwrap();
    let t = p.id("t.png");
    let model = p.id("chair.glb");
    p.rename("chair.glb", "c.glb");
    p.rename("t.png", "one.png");
    fs::copy(p.0.join("one.png"), p.0.join("two.png")).unwrap();
    fs::remove_file(p.0.join(scan::CACHE)).unwrap();
    let mut scan = p.scan();
    scan.run(None).unwrap();
    assert_eq!(p.id("c.glb"), model);
    let candidate = p.id("two.png");
    assert_ne!(t, p.id("one.png"));
    assert_ne!(t, candidate);
    assert_eq!(index(&p.0).unwrap().missing().count(), 1);
    assert!(scan.relink(&t, "c.glb").is_err());
    scan.relink(&t, "two.png").unwrap();
    scan.run(None).unwrap();
    let fresh = Index::build(read_manifest(&p.0).unwrap().0).unwrap();
    assert_eq!(fresh.resolve("t.png", Some(&t)), "two.png");
    assert_eq!(fresh.resolve("two.png", Some(&candidate)), "two.png");
    assert_eq!(fresh.resolve("t.png", None), "two.png");
}

#[test]
fn cold_project_reopens_and_runtime_uses_saved_scene_uids() {
    let p = Project::new();
    let project = crate::project_manager::create_test_project(&p.0).unwrap();
    let root = &project.asset_root;
    let mut scan = Scanner::new(root).unwrap();
    scan.run(None).unwrap();
    let mut project = crate::EditorProject::load(&p.0).unwrap();
    let uid = project.catalog[0].uid.clone().unwrap();
    let source = project.catalog[0].asset.clone();
    let file = project.scenes[0].file.clone();
    project.read_scene(0).unwrap().write(root, &file).unwrap();
    let scene = fs::read(root.join(&file)).unwrap();
    assert!(String::from_utf8_lossy(&scene).contains(&uid));
    project.asset_root = "assets".into();
    fs::write(
        p.0.join(crate::project::PROJECT_FILE_NAME),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    fs::rename(root.join(&source), root.join("moved.glb")).unwrap();
    // Opening the editor must succeed before background recovery finishes.
    crate::EditorProject::load(&p.0).unwrap();
    scan.run(None).unwrap();
    fs::write(root.join(&source), b"unrelated model").unwrap();
    scan.run(None).unwrap();
    INDICES
        .write()
        .unwrap()
        .remove(&root.canonicalize().unwrap());
    let reopened = crate::EditorProject::load(&p.0).unwrap();
    assert_eq!(reopened.catalog[0].asset, "moved.glb");
    let edits = crate::LevelEdits::<String>::read(root, &file).unwrap();
    assert!(
        edits
            .models
            .iter()
            .any(|m| m.uid.as_ref() == Some(&uid) && m.asset == "moved.glb")
    );
    let game = crate::runtime::GameLevel::read(root, &file).unwrap();
    assert!(game.edits.models.iter().any(|m| m.asset == "moved.glb"));
    assert_eq!(fs::read(root.join(&file)).unwrap(), scene);
    edits.write(root, &file).unwrap();
    let saved = bevy_scene_format::Scene::from_slice(&fs::read(root.join(file)).unwrap()).unwrap();
    assert_eq!(
        saved
            .resources
            .values()
            .filter(|r| r.uid.as_ref() == Some(&uid))
            .count(),
        1
    );
}

#[test]
fn incremental_folder_rename_and_unobserved_areas() {
    let p = Project::new();
    p.file("a/t.png", b"pixels");
    p.file("other/b.glb", b"mesh");
    let mut scan = p.scan();
    scan.run(None).unwrap();
    let id = p.id("a/t.png");
    p.rename("a", "B");
    p.file("other/unseen.png", b"other");
    let changes = scan
        .run(Some(&["a".into(), "B".into()].into_iter().collect()))
        .unwrap();
    assert_eq!(changes.hashed, 0);
    assert_eq!(p.id("B/t.png"), id);
    assert!(index(&p.0).unwrap().uid("other/unseen.png").is_none());
    scan.run(None).unwrap();
    assert!(index(&p.0).unwrap().uid("other/unseen.png").is_some());
}
#[test]
#[ignore = "creates 10,000 files and two 64 MiB models"]
fn large_project_index_benchmark() {
    let p = Project::new();
    for i in 0u32..9998 {
        p.file(&format!("textures/{i:04}.png"), &i.to_le_bytes());
    }
    for name in ["a.glb", "b.glb"] {
        p.file(name, name.as_bytes());
        fs::OpenOptions::new()
            .write(true)
            .open(p.0.join(name))
            .unwrap()
            .set_len(64 << 20)
            .unwrap();
    }
    let time = Instant::now();
    let mut scan = p.scan();
    let cold = scan.run(None).unwrap();
    let cold_time = time.elapsed();
    let time = Instant::now();
    let mut scan = p.scan();
    let warm = scan.run(None).unwrap();
    let warm_time = time.elapsed();
    p.rename("a.glb", "large.glb");
    let time = Instant::now();
    let moved = scan
        .run(Some(
            &["a.glb".into(), "large.glb".into()].into_iter().collect(),
        ))
        .unwrap();
    eprintln!(
        "10000 assets: cold {cold_time:?} / {} bytes, warm {warm_time:?} / {} bytes, move {:?} / {} bytes",
        cold.hashed,
        warm.hashed,
        time.elapsed(),
        moved.hashed
    );
    assert_eq!(warm.hashed, 0);
    assert_eq!(moved.hashed, 0);
}

#[test]
fn source_notifications_only_include_changed_content() {
    let project = Project::new();
    project.file("character.glb", b"character");
    let mut scanner = project.scan();
    scanner.run(None).unwrap();
    project.file("unrelated.png", b"unrelated");
    assert_eq!(scanner.run(None).unwrap().modified, ["unrelated.png"]);
    project.file("character.glb", b"character");
    assert!(scanner.run(None).unwrap().modified.is_empty());
    project.file("character.glb", b"new shape");
    assert_eq!(scanner.run(None).unwrap().modified, ["character.glb"]);
}
