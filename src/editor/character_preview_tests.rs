use super::*;
use crate::editor::dialogue::{Character, Portrait};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bevy-editor-character-preview-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, color) in [
            ("red.png", [210, 40, 60, 255]),
            ("blue.png", [30, 80, 220, 255]),
        ] {
            image::RgbaImage::from_pixel(12, 18, image::Rgba(color))
                .save(root.join(name))
                .unwrap();
        }
        Self(root)
    }
    fn character(&self, portraits: &[&str]) {
        Content::Character(Character {
            version: 1,
            name: "Hero".into(),
            description: String::new(),
            portraits: portraits
                .iter()
                .map(|path| Portrait {
                    label: (*path).into(),
                    path: (*path).into(),
                })
                .collect(),
        })
        .save(&self.0, Path::new("Hero.dcharacter"))
        .unwrap();
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(root.parent(), Some(temp.as_path()));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("bevy-editor-character-preview-")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn character_preview_uses_the_main_portrait_and_falls_back_for_empty_or_missing_images() {
    let files = Files::new();
    files.character(&["red.png", "blue.png"]);
    let source = Source::Character("Hero.dcharacter".into());
    let image = decode_source(&files.0, &source).unwrap().unwrap();
    assert_eq!(&image.rgba[..4], &[210, 40, 60, 255]);
    assert_eq!(image.original, UVec2::new(12, 18));
    std::fs::copy(files.0.join("red.png"), files.0.join("face#main.png")).unwrap();
    files.character(&["face#main.png"]);
    assert_eq!(
        &decode_source(&files.0, &source).unwrap().unwrap().rgba[..4],
        &[210, 40, 60, 255]
    );
    files.character(&[]);
    assert!(decode_source(&files.0, &source).unwrap().is_none());
    files.character(&["missing.png"]);
    assert!(decode_source(&files.0, &source).is_err());
    assert!(decode_source(&files.0, &Source::Image("../outside.png".into())).is_err());
}

fn fixture(files: &Files) -> (App, usize) {
    let mut project = crate::EditorProject::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    project.asset_root = files.0.clone();
    project.catalog.clear();
    let browser = Browser::new(&mut project);
    let file = browser
        .files
        .iter()
        .position(|file| file.path == Path::new("Hero.dcharacter"))
        .unwrap();
    let mut editor = Editor::new(project, crate::LevelEdits::default());
    editor.drawer = true;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .insert_resource(browser)
        .init_resource::<Assets<Image>>()
        .init_resource::<ImagePreviews>()
        .add_systems(Update, update);
    (app, file)
}
fn wait(app: &mut App, file: usize) {
    for _ in 0..400 {
        app.update();
        if app.world().resource::<ImagePreviews>().get(file).is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("Character thumbnail did not finish");
}
fn pixel(app: &App, file: usize) -> [u8; 4] {
    let handle = app
        .world()
        .resource::<ImagePreviews>()
        .get(file)
        .unwrap()
        .image
        .as_ref()
        .unwrap();
    app.world()
        .resource::<Assets<Image>>()
        .get(handle)
        .unwrap()
        .data
        .as_ref()
        .unwrap()[..4]
        .try_into()
        .unwrap()
}

#[test]
fn character_preview_tracks_unsaved_portraits_removal_undo_and_discard_without_rescanning() {
    let files = Files::new();
    files.character(&["red.png"]);
    let (mut app, file) = fixture(&files);
    wait(&mut app, file);
    assert_eq!(pixel(&app, file), [210, 40, 60, 255]);
    let revision = app.world().resource::<Browser>().scan_revision;
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor
            .narrative
            .open(&files.0, "Hero.dcharacter".into())
            .unwrap();
        let doc = editor.narrative.current_mut().unwrap();
        doc.checkpoint();
        let Content::Character(character) = &mut doc.content else {
            unreachable!()
        };
        character.portraits[0].path = "blue.png".into();
    }
    wait(&mut app, file);
    assert_eq!(pixel(&app, file), [30, 80, 220, 255]);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let doc = editor.narrative.current_mut().unwrap();
        doc.checkpoint();
        let Content::Character(character) = &mut doc.content else {
            unreachable!()
        };
        character.portraits.clear();
    }
    wait(&mut app, file);
    assert!(
        app.world()
            .resource::<ImagePreviews>()
            .get(file)
            .unwrap()
            .image
            .is_none()
    );
    app.world_mut()
        .resource_mut::<Editor>()
        .narrative
        .current_mut()
        .unwrap()
        .undo();
    wait(&mut app, file);
    assert_eq!(pixel(&app, file), [30, 80, 220, 255]);
    app.world_mut()
        .resource_mut::<Editor>()
        .narrative
        .tabs
        .clear();
    wait(&mut app, file);
    assert_eq!(pixel(&app, file), [210, 40, 60, 255]);
    assert_eq!(app.world().resource::<Browser>().scan_revision, revision);
}

#[test]
fn character_preview_does_not_restore_a_removed_portrait_when_an_old_worker_finishes() {
    let files = Files::new();
    files.character(&["red.png"]);
    let (mut app, file) = fixture(&files);
    wait(&mut app, file);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor
            .narrative
            .open(&files.0, "Hero.dcharacter".into())
            .unwrap();
        let Content::Character(character) = &mut editor.narrative.current_mut().unwrap().content
        else {
            unreachable!()
        };
        character.portraits[0].path = "blue.png".into();
    }
    app.update(); // Starts a request for the replacement image.
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let Content::Character(character) = &mut editor.narrative.current_mut().unwrap().content
        else {
            unreachable!()
        };
        character.portraits.clear();
    }
    wait(&mut app, file);
    let previews = app.world().resource::<ImagePreviews>();
    let preview = previews.get(file).unwrap();
    assert!(preview.image.is_none());
    assert!(preview.error.is_none());
}
