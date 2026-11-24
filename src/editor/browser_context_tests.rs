use super::*;
use crate::editor::{
    image_previews::ImagePreviews,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render,
};
use bevy::{
    camera::RenderTarget,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::MouseButtonInput,
    },
    picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
    ui::IsDefaultUiCamera,
    window::CursorMoved,
};

#[test]
#[ignore = "requires GPU; renames via the real file menu and F2, rejects collisions, cancels, and retains dirty image layers"]
fn gpu_browser_renames_files_without_losing_open_image_edits() {
    let fixture = Fixture::new();
    std::fs::copy(
        fixture.source("picture.png"),
        fixture.assets().join("picture.png"),
    )
    .unwrap();
    image::RgbaImage::from_pixel(16, 16, image::Rgba([10, 20, 30, 255]))
        .save(fixture.assets().join("occupied.png"))
        .unwrap();
    let original = std::fs::read(fixture.assets().join("picture.png")).unwrap();
    let occupied = std::fs::read(fixture.assets().join("occupied.png")).unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.assets();
            editor.project.asset_root = fixture.assets();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    warm(&mut app, 30);
    let file = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|file| file.path == PathBuf::from("picture.png"))
        .unwrap();
    app.world_mut().write_message(Action::ViewImage(file));
    warm(&mut app, 30);
    let tab = app.world().resource::<Editor>().viewing_image.unwrap();
    app.world_mut().write_message(Action::ImageEdit(
        super::super::image_viewer::edit::EditAction::Edit,
    ));
    for _ in 0..120 {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(tab))
            .is_some()
        {
            break;
        }
    }
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(tab))
            .is_some(),
        "Image edit must load: status={}, error={:?}",
        app.world().resource::<Editor>().status,
        app.world().resource::<ImageViewer>().edit.error
    );
    app.world_mut().write_message(Action::ImageEdit(
        super::super::image_viewer::edit::EditAction::Layers(
            super::super::image_viewer::edit::layers::LayerAction::Group,
        ),
    ));
    warm(&mut app, 6);
    app.world_mut().resource_mut::<Editor>().drawer = true;
    warm(&mut app, 24);
    let file = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|file| file.path == PathBuf::from("picture.png"))
        .unwrap();
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(tab))
            .unwrap()
            .dirty()
    );
    let card = {
        let world = app.world_mut();
        world
            .query::<(Entity, &drawer::FileCard)>()
            .iter(world)
            .find(|(_, card)| card.0 == browser::Entry::Image(file))
            .unwrap()
            .0
    };
    let point = rect(&mut app, card).center();
    click(&mut app, window, point, MouseButton::Right);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/browser-rename-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("01-file-menu.png"), |_| {
        true
    });
    let rename = app.world().resource::<ContextUi>().rename;
    let point = rect(&mut app, rename).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Rename);
    let name = app.world().resource::<ContextUi>().name;
    assert_eq!(
        app.world()
            .get::<EditableText>(name)
            .unwrap()
            .value()
            .to_string(),
        "picture"
    );
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(name));
    capture_when(
        &mut app,
        &target,
        &output.join("02-rename-dialog.png"),
        |_| true,
    );
    for invalid in ["../bad", "", "occupied"] {
        app.world_mut()
            .get_mut::<EditableText>(name)
            .unwrap()
            .editor_mut()
            .set_text(invalid);
        key(&mut app, window, KeyCode::Enter, Key::Enter);
        assert!(app.world().resource::<BrowserContext>().phase == Phase::Rename);
        assert!(!app.world().resource::<BrowserContext>().error.is_empty());
        assert_eq!(
            std::fs::read(fixture.assets().join("picture.png")).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read(fixture.assets().join("occupied.png")).unwrap(),
            occupied
        );
    }
    capture_when(
        &mut app,
        &target,
        &output.join("03-name-collision.png"),
        |_| true,
    );
    app.world_mut()
        .get_mut::<EditableText>(name)
        .unwrap()
        .editor_mut()
        .set_text("Обложка");
    warm(&mut app, 2);
    let submit = app.world().resource::<ContextUi>().submit;
    let point = rect(&mut app, submit).center();
    click(&mut app, window, point, MouseButton::Left);
    warm(&mut app, 8);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Closed);
    assert!(!fixture.assets().join("picture.png").exists());
    assert_eq!(
        std::fs::read(fixture.assets().join("Обложка.png")).unwrap(),
        original
    );
    assert_eq!(app.world().resource::<Editor>().viewing_image, Some(tab));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .image_tab(tab)
            .unwrap()
            .path,
        PathBuf::from("Обложка.png")
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(tab))
            .unwrap()
            .dirty()
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-renamed-file.png"),
        |_| true,
    );
    key(&mut app, window, KeyCode::F2, Key::F2);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Rename);
    app.world_mut()
        .get_mut::<EditableText>(name)
        .unwrap()
        .editor_mut()
        .set_text("Отмена");
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Closed);
    assert!(!fixture.assets().join("Отмена.png").exists());
    key(&mut app, window, KeyCode::F2, Key::F2);
    app.world_mut()
        .get_mut::<EditableText>(name)
        .unwrap()
        .editor_mut()
        .set_text("Портрет.PNG");
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Closed);
    assert!(fixture.assets().join("Портрет.png").exists());
    assert!(!fixture.assets().join("Портрет.PNG.png").exists());
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(tab))
            .unwrap()
            .dirty()
    );
    eprintln!("Rename screenshots: {}", output.display());
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "requires GPU; duplicates and recovers a GLB through the real file menu"]
fn gpu_browser_duplicates_saved_models_from_the_file_menu() {
    let fixture = Fixture::new();
    let mut project = crate::EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_tree(&project.asset_root, &fixture.assets());
    project.asset_root = fixture.assets();
    let original_path = PathBuf::from(&project.catalog[0].asset);
    let original_bytes = std::fs::read(fixture.assets().join(&original_path)).unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture_project(project);
    app.world_mut().write_message(Action::ViewModel(0));
    warm(&mut app, 20);
    let before = app.world().resource::<Editor>().edits.clone();
    let initial_count = app.world().resource::<Editor>().project.catalog.len();
    app.world_mut().resource_mut::<Editor>().drawer = true;
    let folder = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == original_path.parent().unwrap())
        .unwrap();
    app.world_mut().resource_mut::<Browser>().navigate(folder);
    warm(&mut app, 30);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/browser-duplicate-qa");
    std::fs::create_dir_all(&output).unwrap();
    for number in 1..=2 {
        let card = {
            let world = app.world_mut();
            world
                .query::<(Entity, &drawer::FileCard)>()
                .iter(world)
                .find(|(_, card)| card.0 == browser::Entry::Model(0))
                .unwrap()
                .0
        };
        let point = rect(&mut app, card).center();
        click(&mut app, window, point, MouseButton::Right);
        assert!(app.world().resource::<BrowserContext>().file_menu());
        let duplicate = app.world().resource::<ContextUi>().duplicate;
        if number == 1 {
            capture_when(
                &mut app,
                &target,
                &output.join("01-duplicate-menu.png"),
                |_| true,
            );
        }
        let point = rect(&mut app, duplicate).center();
        click(&mut app, window, point, MouseButton::Left);
        for _ in 0..240 {
            frame(&mut app);
            if app.world().resource::<Editor>().project.catalog.len() == initial_count + number {
                break;
            }
        }
        warm(&mut app, 8);
        let editor = app.world().resource::<Editor>();
        assert_eq!(
            editor.project.catalog.len(),
            initial_count + number,
            "{}",
            editor.status
        );
        assert_eq!(editor.viewing_model, Some(0));
        assert_eq!(editor.edits, before);
        assert_eq!(
            std::fs::read(fixture.assets().join(&original_path)).unwrap(),
            original_bytes
        );
        let suffix = if number == 1 {
            " — копия".to_owned()
        } else {
            format!(" — копия ({number})")
        };
        let copy = original_path.with_file_name(format!(
            "{}{suffix}.glb",
            original_path.file_stem().unwrap().to_string_lossy()
        ));
        let copied = editor
            .project
            .catalog
            .iter()
            .position(|model| std::path::Path::new(&model.asset) == copy)
            .unwrap();
        assert_eq!(
            std::fs::read(fixture.assets().join(&copy)).unwrap(),
            original_bytes
        );
        assert_eq!(
            app.world().resource::<drawer::BrowserUi>().selected_entry,
            Some(browser::Entry::Model(copied))
        );
    }
    capture_when(
        &mut app,
        &target,
        &output.join("02-created-copies.png"),
        |_| true,
    );
    let recovered_bytes = std::fs::read(fixture.assets().join("models/blue_block.glb")).unwrap();
    recovery::seed_abandoned_copy(&fixture.assets(), &original_path, &recovered_bytes);
    let card = {
        let world = app.world_mut();
        world.query::<(Entity, &drawer::FileCard)>().iter(world)
            .find(|(_, card)| card.0 == browser::Entry::Model(0)).unwrap().0
    };
    let point = rect(&mut app, card).center();
    click(&mut app, window, point, MouseButton::Right);
    let recover = app.world().resource::<ContextUi>().recover_model;
    let point = rect(&mut app, recover).center();
    click(&mut app, window, point, MouseButton::Left);
    let recovered = original_path.with_file_name("block — восстановлено.glb");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    loop {
        frame(&mut app);
        let editor = app.world().resource::<Editor>();
        if editor.viewing_model.is_some_and(|index| PathBuf::from(&editor.project.catalog[index].asset) == recovered) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", editor.status);
    }
    assert_eq!(std::fs::read(fixture.assets().join(recovered)).unwrap(), recovered_bytes);
    assert_eq!(std::fs::read(fixture.assets().join(original_path)).unwrap(), original_bytes);
    eprintln!("Duplicate screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; recovers the offered GLB with the real startup confirmation"]
fn gpu_browser_offers_model_recovery_at_startup() {
    let fixture = Fixture::new();
    let mut project = crate::EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_tree(&project.asset_root, &fixture.assets());
    project.asset_root = fixture.assets();
    let source = std::path::Path::new("models/block.glb");
    let original = std::fs::read(project.asset_root.join(source)).unwrap();
    let recovered = std::fs::read(project.asset_root.join("models/blue_block.glb")).unwrap();
    recovery::seed_abandoned_copy(&project.asset_root, source, &recovered);
    let (mut app, window, _) = lighting_ui_render::fixture_project(project);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    let button = loop {
        frame(&mut app);
        let world = app.world_mut();
        let visible = world.query::<(Entity, &Control, &ComputedNode)>().iter(world)
            .find(|(_, control, node)| matches!(control.action, Action::SaveAndContinue) && !node.is_empty())
            .map(|(entity, _, _)| entity);
        if let Some(entity) = visible {
            break entity;
        }
        assert!(std::time::Instant::now() < deadline, "recovery confirmation did not appear");
    };
    assert_eq!(app.world().resource::<Editor>().pending, Some(crate::editor::PendingAction::RecoverModel));
    assert_eq!(app.world().resource::<BrowserContext>().recovery_path(), Some(source));
    let point = rect(&mut app, button).center();
    click(&mut app, window, point, MouseButton::Left);
    let target = source.with_file_name("block — восстановлено.glb");
    loop {
        frame(&mut app);
        let editor = app.world().resource::<Editor>();
        if editor.viewing_model.is_some_and(|index| std::path::Path::new(&editor.project.catalog[index].asset) == target) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", editor.status);
    }
    assert_eq!(std::fs::read(fixture.assets().join(target)).unwrap(), recovered);
    assert_eq!(std::fs::read(fixture.assets().join(source)).unwrap(), original);
    assert!(recovery::startup_offer(&fixture.assets(), &[]).is_none());
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("bevy-browser-context-{}", std::process::id()));
        std::fs::create_dir_all(root.join("assets/textures")).unwrap();
        std::fs::create_dir_all(root.join("assets/reference")).unwrap();
        std::fs::create_dir_all(root.join("external")).unwrap();
        image::RgbaImage::from_fn(480, 240, |x, y| {
            image::Rgba([180 + (x / 12) as u8, 90 + (y / 3) as u8, 55, 255])
        })
        .save(root.join("external/picture.png"))
        .unwrap();
        std::fs::write(root.join("external/logo.svg"), br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><circle cx="32" cy="32" r="26" fill="#67cfa4" opacity="0.8"/></svg>"##).unwrap();
        std::fs::write(root.join("external/notes.txt"), b"keep").unwrap();
        Self(root)
    }
    fn assets(&self) -> PathBuf {
        self.0.join("assets")
    }
    fn source(&self, name: &str) -> PathBuf {
        self.0.join("external").join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn warm(app: &mut App, count: usize) {
    for _ in 0..count {
        frame(app);
    }
}
fn rect(app: &mut App, entity: Entity) -> Rect {
    let world = app.world();
    let node = world.get::<ComputedNode>(entity).unwrap();
    let transform = world.get::<UiGlobalTransform>(entity).unwrap();
    assert!(!node.is_empty());
    Rect::from_center_size(transform.affine().translation, node.size())
}
fn pointer(app: &mut App, window: Entity, point: Vec2, action: PointerAction) {
    let target = {
        let world = app.world_mut();
        world
            .query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>()
            .single(world)
            .unwrap()
            .normalize(Some(window))
            .unwrap()
    };
    app.world_mut().write_message(PointerInput {
        pointer_id: PointerId::Mouse,
        location: Location {
            target,
            position: point,
        },
        action,
    });
}
fn move_to(app: &mut App, window: Entity, point: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.world_mut().write_message(CursorMoved {
        window,
        position: point,
        delta: None,
    });
    pointer(
        app,
        window,
        point,
        PointerAction::Move { delta: Vec2::ZERO },
    );
    warm(app, 2);
}
fn mouse(app: &mut App, window: Entity, button: MouseButton, pressed: bool) {
    let position = app
        .world()
        .get::<Window>(window)
        .unwrap()
        .cursor_position()
        .unwrap();
    let pointer_button = if button == MouseButton::Right {
        PointerButton::Secondary
    } else {
        PointerButton::Primary
    };
    app.world_mut().write_message(MouseButtonInput {
        window,
        button,
        state: if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        },
    });
    pointer(
        app,
        window,
        position,
        if pressed {
            PointerAction::Press(pointer_button)
        } else {
            PointerAction::Release(pointer_button)
        },
    );
    warm(app, 2);
}
fn click(app: &mut App, window: Entity, point: Vec2, button: MouseButton) {
    move_to(app, window, point);
    mouse(app, window, button, true);
    mouse(app, window, button, false);
}
fn key(app: &mut App, window: Entity, key_code: KeyCode, logical_key: Key) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key: logical_key.clone(),
            state,
            text: None,
            repeat: false,
        });
        warm(app, 2);
    }
}
fn folder_card(app: &mut App, path: &str) -> Vec2 {
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let index = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == PathBuf::from(path))
        .unwrap();
    let world = app.world_mut();
    let mut controls = world.query::<(Entity, &Control, &UiGlobalTransform, &ChildOf)>();
    controls
        .iter(world)
        .find_map(|(_, control, transform, parent)| {
            (matches!(control.action, Action::Browse(folder) if folder == index)
                && parent.parent() == grid)
                .then_some(transform.affine().translation)
        })
        .unwrap()
}

fn folder_tree_item(app: &mut App, path: &str) -> (Entity, Vec2) {
    let index = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == PathBuf::from(path))
        .unwrap();
    let world = app.world_mut();
    let mut controls = world
        .query_filtered::<(Entity, &Control, &ComputedNode, &UiGlobalTransform), With<AssetOpen>>();
    controls
        .iter(world)
        .find_map(|(entity, control, node, transform)| {
            (matches!(control.look, Look::Regular)
                && matches!(control.action, Action::Browse(folder) if folder == index)
                && !node.is_empty())
            .then_some((entity, transform.affine().translation))
        })
        .expect("The real folder tree item must be visible")
}

#[test]
#[ignore = "requires GPU; double-clicks real tree entries before/after a file-only rename and filesystem refresh"]
fn gpu_browser_folder_tree_keeps_navigation_after_file_rename_and_refresh() {
    let fixture = Fixture::new();
    std::fs::write(fixture.assets().join("notes.dialog"), b"{}").unwrap();
    let (mut app, window, _) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.assets();
            editor.project.asset_root = fixture.assets();
            editor.drawer = true;
            browser.refresh(&mut editor.project);
        });
    warm(&mut app, 20);

    let assert_open = |app: &App, path: &str| {
        let browser = app.world().resource::<Browser>();
        assert_eq!(browser.folders[browser.current].path, PathBuf::from(path));
    };
    let (entity, point) = folder_tree_item(&mut app, "textures");
    click(&mut app, window, point, MouseButton::Left);
    assert_open(&app, "");
    assert_eq!(folder_tree_item(&mut app, "textures").0, entity);
    click(&mut app, window, point, MouseButton::Left);
    assert_open(&app, "textures");

    // Renaming a narrative file advances the scan generation without changing
    // the hierarchy. Tree entries must not keep the old FileCard/DropFolder IDs.
    std::fs::rename(
        fixture.assets().join("notes.dialog"),
        fixture.assets().join("renamed.dialog"),
    )
    .unwrap();
    {
        let mut browser = app.world_mut().resource_mut::<Browser>();
        let tree_revision = browser.tree_revision;
        assert!(browser.rename_narrative(
            &fixture.assets(),
            std::path::Path::new("notes.dialog"),
            std::path::Path::new("renamed.dialog"),
        ));
        assert_eq!(browser.tree_revision, tree_revision);
    }
    warm(&mut app, 5);
    let (entity, point) = folder_tree_item(&mut app, "reference");
    let scan = app.world().resource::<Browser>().scan_revision;
    assert_eq!(app.world().get::<drawer::FileCard>(entity).unwrap().1, scan);
    assert_eq!(
        app.world().get::<drawer::DropFolder>(entity).unwrap().1,
        scan
    );
    click(&mut app, window, point, MouseButton::Left);
    assert_open(&app, "textures");
    click(&mut app, window, point, MouseButton::Left);
    assert_open(&app, "reference");

    app.world_mut().write_message(Action::BrowseRefresh);
    warm(&mut app, 5);
    let (_, point) = folder_tree_item(&mut app, "textures");
    click(&mut app, window, point, MouseButton::Left);
    assert_open(&app, "reference");
    click(&mut app, window, point, MouseButton::Left);
    assert_open(&app, "textures");
}

fn enter_name(app: &mut App, window: Entity, name: &str) {
    let input = app.world().resource::<ContextUi>().name;
    app.world_mut()
        .get_mut::<EditableText>(input)
        .unwrap()
        .editor_mut()
        .set_text(name);
    key(app, window, KeyCode::Enter, Key::Enter);
    warm(app, 4);
}
fn finish_import(app: &mut App) {
    for _ in 0..180 {
        frame(app);
        let context = app.world().resource::<BrowserContext>();
        if context.pending.is_none() && context.completed.is_none() && context.queue.is_empty() {
            warm(app, 8);
            return;
        }
    }
    panic!("Image import did not finish");
}

#[test]
#[ignore = "requires GPU; drops SPP projects, opens the collected import report and closes it"]
fn gpu_browser_displays_the_spp_import_report() {
    use crate::editor::model_paint_spp::fixture as spp;

    let fixture = Fixture::new();
    let (mut app, window, _) = lighting_ui_render::fixture();
    app.world_mut().resource_scope(|world, mut browser: Mut<Browser>| {
        let mut editor = world.resource_mut::<Editor>();
        editor.root = fixture.assets();
        editor.project.asset_root = fixture.assets();
        editor.drawer = true;
        browser.refresh(&mut editor.project);
    });
    warm(&mut app, 45);
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let point = rect(&mut app, grid).max - Vec2::splat(24.0);
    move_to(&mut app, window, point);
    for name in ["Report.spp", "Second.spp"] {
        let path = fixture.source(name);
        std::fs::write(
            &path,
            spp::hdf5_file(&[
                ("paint/scene.assbin", spp::assbin_quad()),
                ("paint/document.bin", spp::document_with_uv_stencil("missing.image")),
            ]),
        )
        .unwrap();
        app.world_mut().write_message(FileDragAndDrop::DroppedFile { window, path_buf: path });
    }
    finish_import(&mut app);
    {
        let world = app.world();
        let ui = world.resource::<ContextUi>();
        assert!(world.resource::<BrowserContext>().phase == Phase::Report);
        assert_eq!(world.get::<Node>(ui.report).unwrap().display, Display::Flex);
        assert_eq!(world.get::<Node>(ui.dialog).unwrap().display, Display::None);
        assert_eq!(world.resource::<InputFocus>().get(), Some(ui.report_close));
        let body = &world.get::<Text>(ui.report_text).unwrap().0;
        for line in [
            "Report_SPP.glb",
            "Second_SPP.glb",
            "карта «цвет» отсутствует в кэше",
            "карта «шероховатость» отсутствует в кэше",
            "«Scratches» · восстановлен частично",
        ] {
            assert!(body.contains(line), "{line}\n{body}");
        }
        assert!(world.resource::<Editor>().browser_popup);
    }
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    let world = app.world();
    let ui = world.resource::<ContextUi>();
    assert_eq!(world.get::<Node>(ui.report).unwrap().display, Display::None);
    assert!(!world.resource::<Editor>().browser_popup);
}

#[test]
#[ignore = "requires GPU; right-clicks real file cards and empty space, checks selection and executes the file menu"]
fn gpu_browser_context_menu_selects_files_and_separates_creation_actions() {
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut().resource_mut::<Editor>().drawer = true;
    {
        let mut browser = app.world_mut().resource_mut::<Browser>();
        let folder = browser
            .folders
            .iter()
            .position(|folder| folder.path == PathBuf::from("models"))
            .unwrap();
        browser.navigate(folder);
    }
    warm(&mut app, 35);
    let card = |app: &mut App, model| {
        let world = app.world_mut();
        world
            .query::<(Entity, &drawer::FileCard)>()
            .iter(world)
            .find_map(|(entity, card)| (card.0 == browser::Entry::Model(model)).then_some(entity))
            .unwrap()
    };
    let first = card(&mut app, 0);
    let second = card(&mut app, 1);
    let first_point = rect(&mut app, first).center();
    let second_point = rect(&mut app, second).min + Vec2::splat(20.0);
    let remove = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find_map(|(entity, control)| {
                matches!(control.action, Action::BrowserDeleteFile).then_some(entity)
            })
            .unwrap()
    };
    let assert_menu = |app: &App, selected: Option<browser::Entry>| {
        let context = app.world().resource::<BrowserContext>();
        let ui = app.world().resource::<ContextUi>();
        assert!(context.phase == Phase::Menu);
        assert_eq!(
            app.world().resource::<drawer::BrowserUi>().selected_entry,
            selected
        );
        for entity in ui.creation {
            assert_eq!(
                app.world().get::<Node>(entity).unwrap().display,
                if selected.is_some() {
                    Display::None
                } else {
                    Display::Flex
                }
            );
        }
        for entity in [ui.open, remove] {
            assert_eq!(
                app.world().get::<Node>(entity).unwrap().display,
                if selected.is_some() {
                    Display::Flex
                } else {
                    Display::None
                }
            );
        }
        if let Some(entry) = context.menu_entry {
            assert!(
                app.world()
                    .get::<InteractionDisabled>(entry.entity)
                    .is_some()
            );
            assert_eq!(
                *app.world().get::<BorderColor>(entry.entity).unwrap(),
                BorderColor::all(FOCUS)
            );
            assert_eq!(
                app.world().get::<BackgroundColor>(entry.entity).unwrap().0,
                ACTIVE
            );
        }
    };

    // No preliminary left click: the file under RMB becomes selected immediately.
    click(&mut app, window, second_point, MouseButton::Right);
    assert_menu(&app, Some(browser::Entry::Model(1)));
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/browser-context-actions-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("01-file-menu.png"), |_| {
        true
    });
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(second));
    assert_eq!(
        app.world().resource::<drawer::BrowserUi>().selected_entry,
        Some(browser::Entry::Model(1))
    );

    // Opening a selected card's menu and retargeting another card keep selection visible.
    click(&mut app, window, first_point, MouseButton::Left);
    click(&mut app, window, first_point, MouseButton::Right);
    assert_menu(&app, Some(browser::Entry::Model(0)));
    click(&mut app, window, second_point, MouseButton::Right);
    assert_menu(&app, Some(browser::Entry::Model(1)));

    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    click(&mut app, window, empty, MouseButton::Right);
    assert_menu(&app, None);
    assert!(app.world().resource::<Editor>().viewing_model.is_none());
    let [(unwrap, _), (udim, _), (size, _), (bake, _), (quality, _), (names, _)] =
        app.world().resource::<ContextUi>().import_labels;
    for expected in [AutoUnwrap::All, AutoUnwrap::Off, AutoUnwrap::Missing] {
        let point = rect(&mut app, unwrap).center();
        click(&mut app, window, point, MouseButton::Left);
        assert_eq!(app.world().resource::<UvState>().auto_unwrap(), expected);
        assert!(app.world().get::<AccessibleLabel>(unwrap).unwrap().0.ends_with(expected.label()));
    }
    let point = rect(&mut app, udim).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(!app.world().resource::<UvState>().detect_udim_on_import());
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<UvState>().detect_udim_on_import());
    let point = rect(&mut app, size).center();
    click(&mut app, window, point, MouseButton::Left);
    assert_eq!(app.world().resource::<UvState>().texture_size_on_import(), 2048);
    let point = rect(&mut app, bake).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<UvState>().bake_on_load());
    let point = rect(&mut app, quality).center();
    click(&mut app, window, point, MouseButton::Left);
    assert_eq!(
        app.world().resource::<UvState>().bake_defaults().quality,
        crate::editor::model_bake_settings::Quality::Standard,
    );
    let point = rect(&mut app, names).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(!app.world().resource::<UvState>().bake_defaults().match_names);
    capture_when(&mut app, &target, &output.join("02-empty-menu.png"), |_| {
        true
    });

    click(&mut app, window, second_point, MouseButton::Right);
    assert_menu(&app, Some(browser::Entry::Model(1)));
    let open = app.world().resource::<ContextUi>().open;
    let open_point = rect(&mut app, open).center();
    // A right click inside the popup must not select a card underneath it.
    click(&mut app, window, open_point, MouseButton::Right);
    assert_menu(&app, Some(browser::Entry::Model(1)));
    click(&mut app, window, open_point, MouseButton::Left);
    warm(&mut app, 12);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Closed);
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(1));
}

#[test]
#[ignore = "requires GPU; creates a scene via the actual context menu, validates/cancels names and opens its card"]
fn gpu_browser_creates_scene_from_context_menu_and_opens_it_on_double_click() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.assets().join("scenes")).unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.assets();
            editor.project.asset_root = fixture.assets();
            editor.drawer = true;
            browser.refresh(&mut editor.project);
            let folder = browser
                .folders
                .iter()
                .position(|folder| folder.path == PathBuf::from("scenes"))
                .unwrap();
            browser.navigate(folder);
        });
    warm(&mut app, 25);
    app.world_mut()
        .write_message(Action::Select(Selection::Camera(0)));
    app.world_mut()
        .write_message(Action::Number(Field::Lens, 65.0));
    warm(&mut app, 3);
    let (previous_room, previous_edits) = {
        let editor = app.world().resource::<Editor>();
        (editor.room, editor.edits.clone())
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/browser-scene-qa");
    std::fs::create_dir_all(&output).unwrap();
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    click(&mut app, window, empty, MouseButton::Right);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
    capture_when(&mut app, &target, &output.join("01-menu.png"), |_| true);
    let new_scene = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find(|(_, control)| matches!(control.action, Action::BrowserNewScene))
            .unwrap()
            .0
    };
    let point = rect(&mut app, new_scene).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Scene);
    let name = app.world().resource::<ContextUi>().name;
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(name));
    app.world_mut()
        .get_mut::<EditableText>(name)
        .unwrap()
        .editor_mut()
        .set_text("Отменено");
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(!fixture.assets().join("scenes/Отменено.scene").exists());

    click(&mut app, window, empty, MouseButton::Right);
    let point = rect(&mut app, new_scene).center();
    click(&mut app, window, point, MouseButton::Left);
    enter_name(&mut app, window, "../outside");
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Scene);
    assert!(!app.world().resource::<BrowserContext>().error.is_empty());
    app.world_mut()
        .get_mut::<EditableText>(name)
        .unwrap()
        .editor_mut()
        .set_text("Новая локация.SCENE");
    warm(&mut app, 3);
    capture_when(&mut app, &target, &output.join("02-dialog.png"), |_| true);
    let submit = app.world().resource::<ContextUi>().submit;
    let point = rect(&mut app, submit).center();
    click(&mut app, window, point, MouseButton::Left);
    finish_import(&mut app);
    let path = PathBuf::from("scenes/Новая локация.scene");
    let document =
        crate::LevelEdits::<String>::read(&fixture.assets(), path.to_str().unwrap()).unwrap();
    assert_eq!(document.scene.as_ref().unwrap().label, "Новая локация");
    assert!(document.models.is_empty());
    let room = app
        .world()
        .resource::<Editor>()
        .project
        .scenes
        .iter()
        .position(|scene| PathBuf::from(&scene.file) == path)
        .unwrap();
    let file = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|file| file.path == path)
        .unwrap();
    assert_eq!(
        app.world().resource::<drawer::BrowserUi>().selected_entry,
        Some(browser::Entry::Scene { file, room })
    );
    assert_eq!(app.world().resource::<Editor>().room, previous_room);
    assert_eq!(app.world().resource::<Editor>().edits, previous_edits);
    let card = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control, &ChildOf)>()
            .iter(world)
            .find(|(_, control, parent)| {
                matches!(control.action, Action::Open(id) if id == room) && parent.parent() == grid
            })
            .unwrap()
            .0
    };
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(card));
    let point = rect(&mut app, card).center();
    click(&mut app, window, point, MouseButton::Left);
    assert_eq!(app.world().resource::<Editor>().room, previous_room);
    click(&mut app, window, point, MouseButton::Left);
    warm(&mut app, 12);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.room, room);
    assert!(editor.ready && editor.definition().is_native());
    assert!(editor.edits.models.is_empty());
    assert!(editor.document_dirty(previous_room));
    capture_when(
        &mut app,
        &target,
        &output.join("03-empty-scene.png"),
        |_| true,
    );
}

#[test]
#[ignore = "requires GPU adapter; creates a white PNG through the real context menu and size dialog"]
fn gpu_browser_creates_white_images_with_custom_sizes_and_cancel() {
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.assets();
            editor.project.asset_root = fixture.assets();
            editor.drawer = true;
            browser.refresh(&mut editor.project);
            let folder = browser
                .folders
                .iter()
                .position(|folder| folder.path == PathBuf::from("textures"))
                .unwrap();
            browser.navigate(folder);
        });
    warm(&mut app, 24);
    let output =
        std::env::temp_dir().join(format!("bevy-browser-new-image-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("New image dialog GPU captures: {}", output.display());
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    click(&mut app, window, empty, MouseButton::Right);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
    capture_when(&mut app, &target, &output.join("01-menu.png"), |_| true);
    let new_image = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find(|(_, control)| matches!(control.action, Action::BrowserNewImage))
            .unwrap()
            .0
    };
    let point = rect(&mut app, new_image).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Image);
    let (name, width, height, dialog, submit) = {
        let ui = app.world().resource::<ContextUi>();
        (ui.name, ui.size.width, ui.size.height, ui.dialog, ui.submit)
    };
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(name));
    capture_when(
        &mut app,
        &target,
        &output.join("02-default-dialog.png"),
        |_| true,
    );
    assert_eq!(
        app.world()
            .get::<EditableText>(width)
            .unwrap()
            .value()
            .to_string(),
        "1024"
    );
    assert_eq!(
        app.world()
            .get::<EditableText>(height)
            .unwrap()
            .value()
            .to_string(),
        "1024"
    );
    app.world_mut()
        .get_mut::<EditableText>(name)
        .unwrap()
        .editor_mut()
        .set_text("Белый холст");
    app.world_mut()
        .get_mut::<EditableText>(width)
        .unwrap()
        .editor_mut()
        .set_text("0");
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Image);
    assert!(!app.world().resource::<BrowserContext>().error.is_empty());
    assert!(!fixture.assets().join("textures/Белый холст.png").exists());
    app.world_mut()
        .get_mut::<EditableText>(width)
        .unwrap()
        .editor_mut()
        .set_text("320");
    app.world_mut()
        .get_mut::<EditableText>(height)
        .unwrap()
        .editor_mut()
        .set_text("180");
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(1000.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(bevy::render::render_resource::Extent3d {
            width: 1000,
            height: 620,
            depth_or_array_layers: 1,
        });
    warm(&mut app, 8);
    let bounds = rect(&mut app, dialog);
    assert!(
        bounds.min.cmpge(Vec2::ZERO).all() && bounds.max.cmple(Vec2::new(1000.0, 620.0)).all(),
        "The size dialog must fit the minimum window: {bounds:?}"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("02-size-dialog.png"),
        |_| true,
    );
    let point = rect(&mut app, submit).center();
    click(&mut app, window, point, MouseButton::Left);
    finish_import(&mut app);
    let path = fixture.assets().join("textures/Белый холст.png");
    let image = image::open(path).unwrap().into_rgba8();
    assert_eq!(image.dimensions(), (320, 180));
    assert!(image.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
    assert!(!app.world().resource::<Editor>().browser_popup);
    let selected = app
        .world()
        .resource::<drawer::BrowserUi>()
        .selected
        .expect("New PNG must be selected");
    assert_eq!(
        app.world().resource::<Browser>().files[selected].path,
        PathBuf::from("textures/Белый холст.png")
    );
    warm(&mut app, 24);
    capture_when(
        &mut app,
        &target,
        &output.join("03-created-white-image.png"),
        |_| true,
    );

    let before = std::fs::read_dir(fixture.assets().join("textures"))
        .unwrap()
        .count();
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    click(&mut app, window, empty, MouseButton::Right);
    let point = rect(&mut app, new_image).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Image);
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(!app.world().resource::<Editor>().browser_popup);
    assert_eq!(
        std::fs::read_dir(fixture.assets().join("textures"))
            .unwrap()
            .count(),
        before
    );
    assert!(app.world().resource::<BrowserContext>().pending.is_none());
}

#[test]
#[ignore = "requires GPU adapter; verifies native drop messages and actual browser menus"]
fn gpu_browser_import_and_context_folders() {
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    {
        let world = app.world_mut();
        world.resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.assets();
            editor.project.asset_root = fixture.assets();
            editor.drawer = true;
            browser.refresh(&mut editor.project);
        });
    }
    warm(&mut app, 45);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/browser-files-qa");
    std::fs::create_dir_all(&output).unwrap();
    let original_view = app.world().resource::<Editor>().view;
    let original_edits = app.world().resource::<Editor>().edits.clone();
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    click(&mut app, window, empty, MouseButton::Right);
    assert!(app.world().resource::<Editor>().browser_popup);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
    let [(unwrap, _), ..] = app.world().resource::<ContextUi>().import_labels;
    let point = rect(&mut app, unwrap).center();
    click(&mut app, window, point, MouseButton::Left);
    assert_eq!(app.world().resource::<UvState>().auto_unwrap(), AutoUnwrap::All);
    capture_when(
        &mut app,
        &target,
        &output.join("01-context-menu.png"),
        |_| true,
    );
    let item = app.world().resource::<ContextUi>().new_folder;
    let point = rect(&mut app, item).center();
    click(&mut app, window, point, MouseButton::Left);
    assert!(app.world().resource::<BrowserContext>().folder_dialog());
    let input = app.world().resource::<ContextUi>().name;
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    enter_name(&mut app, window, "../outside");
    assert!(!app.world().resource::<BrowserContext>().error.is_empty());
    assert!(!fixture.0.join("outside").exists());
    enter_name(&mut app, window, "textures");
    assert!(!app.world().resource::<BrowserContext>().error.is_empty());
    capture_when(
        &mut app,
        &target,
        &output.join("02-folder-validation.png"),
        |_| true,
    );
    enter_name(&mut app, window, "Иллюстрации");
    assert!(fixture.assets().join("Иллюстрации").is_dir());
    assert!(!app.world().resource::<Editor>().browser_popup);
    assert!(
        app.world()
            .resource::<Browser>()
            .folders
            .iter()
            .any(|folder| folder.path == PathBuf::from("Иллюстрации"))
    );

    let textures = folder_card(&mut app, "textures");
    click(&mut app, window, textures, MouseButton::Right);
    assert_eq!(
        app.world().resource::<BrowserContext>().parent,
        PathBuf::from("textures")
    );
    let item = app.world().resource::<ContextUi>().new_folder;
    let point = rect(&mut app, item).center();
    click(&mut app, window, point, MouseButton::Left);
    enter_name(&mut app, window, "Вложенная");
    assert!(fixture.assets().join("textures/Вложенная").is_dir());
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    move_to(&mut app, window, empty);
    app.world_mut().write_message(FileDragAndDrop::HoveredFile {
        window,
        path_buf: fixture.source("picture.png"),
    });
    warm(&mut app, 4);
    assert!(app.world().resource::<BrowserContext>().target.is_some());
    capture_when(
        &mut app,
        &target,
        &output.join("03-drop-indicator.png"),
        |_| true,
    );
    std::fs::write(
        fixture.source("existing-uv.obj"),
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0.125 0.125\nvt 0.375 0.125\nvt 0.125 0.375\nf 1/1 2/2 3/3\n",
    ).unwrap();
    for name in ["picture.png", "logo.svg", "existing-uv.obj"] {
        app.world_mut().write_message(FileDragAndDrop::DroppedFile {
            window,
            path_buf: fixture.source(name),
        });
    }
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    finish_import(&mut app);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    assert!(fixture.assets().join("textures/existing-uv.glb").exists());
    assert!(app.world().resource::<Editor>().status.contains("Построена UV-развёртка · наборов: 1"));
    for name in ["picture.png", "logo.svg"] {
        assert!(fixture.source(name).exists());
        assert_eq!(
            std::fs::read(fixture.source(name)).unwrap(),
            std::fs::read(fixture.assets().join("textures").join(name)).unwrap()
        );
    }
    let image = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|file| file.path == PathBuf::from("textures/picture.png"))
        .unwrap();
    for _ in 0..90 {
        if app
            .world()
            .resource::<ImagePreviews>()
            .get(image)
            .is_some_and(|preview| preview.image.is_some())
        {
            break;
        }
        frame(&mut app);
    }
    assert!(
        app.world()
            .resource::<ImagePreviews>()
            .get(image)
            .is_some_and(|preview| preview.image.is_some())
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-imported-images.png"),
        |_| true,
    );
    // Import and folder creation stay available with an image document active.
    app.world_mut().write_message(Action::ViewImage(image));
    warm(&mut app, 12);
    let image_tab = app.world().resource::<Editor>().viewing_image.unwrap();
    for name in ["picture.png", "notes.txt"] {
        app.world_mut().write_message(FileDragAndDrop::DroppedFile {
            window,
            path_buf: fixture.source(name),
        });
    }
    finish_import(&mut app);
    assert!(fixture.assets().join("textures/picture (2).png").exists());
    assert!(!fixture.assets().join("textures/notes.txt").exists());
    assert!(
        app.world()
            .resource::<Editor>()
            .status
            .contains("notes.txt")
    );

    // Dropping onto a different tree folder must not change the open directory.
    let reference = {
        let world = app.world_mut();
        let browser = world.resource::<Browser>();
        let index = browser
            .folders
            .iter()
            .position(|folder| folder.path == PathBuf::from("reference"))
            .unwrap();
        let mut controls = world.query::<(&Control, &UiGlobalTransform)>();
        controls
            .iter(world)
            .find_map(|(control, transform)| {
                matches!(control.action, Action::Browse(folder) if folder == index)
                    .then_some(transform.affine().translation)
            })
            .unwrap()
    };
    move_to(&mut app, window, reference);
    app.world_mut().write_message(FileDragAndDrop::DroppedFile {
        window,
        path_buf: fixture.source("picture.png"),
    });
    finish_import(&mut app);
    assert!(fixture.assets().join("reference/picture.png").exists());
    let browser = app.world().resource::<Browser>();
    assert_eq!(
        browser.folders[browser.current].path,
        PathBuf::from("textures")
    );

    for (key_code, logical_key) in [
        (KeyCode::ControlLeft, Key::Control),
        (KeyCode::ShiftLeft, Key::Shift),
        (KeyCode::KeyN, Key::Character("N".into())),
    ] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
        });
    }
    warm(&mut app, 2);
    for (key_code, logical_key) in [
        (KeyCode::ControlLeft, Key::Control),
        (KeyCode::ShiftLeft, Key::Shift),
        (KeyCode::KeyN, Key::Character("N".into())),
    ] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key,
            state: ButtonState::Released,
            text: None,
            repeat: false,
        });
    }
    warm(&mut app, 2);
    assert!(app.world().resource::<BrowserContext>().folder_dialog());
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(1000.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(bevy::render::render_resource::Extent3d {
            width: 1000,
            height: 620,
            depth_or_array_layers: 1,
        });
    warm(&mut app, 8);
    capture_when(&mut app, &target, &output.join("05-new-folder.png"), |_| {
        true
    });
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(!app.world().resource::<Editor>().browser_popup);
    assert!(app.world().resource::<Editor>().drawer);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.viewing_image, Some(image_tab));
    assert_eq!(
        editor.image_tab(image_tab).unwrap().path,
        PathBuf::from("textures/picture.png")
    );
    app.world_mut()
        .write_message(Action::CloseImageTab(image_tab));
    warm(&mut app, 4);
    let empty = rect(&mut app, grid).max - Vec2::splat(24.0);
    click(&mut app, window, empty, MouseButton::Right);
    // A click that dismisses the menu must never open the image underneath it.
    let image_point = {
        let world = app.world_mut();
        let mut controls = world.query::<(&Control, &UiGlobalTransform)>();
        controls
            .iter(world)
            .find_map(|(control, transform)| {
                matches!(control.action, Action::ViewImage(_))
                    .then_some(transform.affine().translation)
            })
            .unwrap()
    };
    click(&mut app, window, image_point, MouseButton::Left);
    warm(&mut app, 4);
    assert!(!app.world().resource::<Editor>().asset_viewer_open());
    assert!(!app.world().resource::<Editor>().browser_popup);
    // Right-clicking an image must not activate its primary "open" action,
    // including a fast secondary press/release delivered in one frame.
    move_to(&mut app, window, image_point);
    for (state, action) in [
        (
            ButtonState::Pressed,
            PointerAction::Press(PointerButton::Secondary),
        ),
        (
            ButtonState::Released,
            PointerAction::Release(PointerButton::Secondary),
        ),
    ] {
        app.world_mut().write_message(MouseButtonInput {
            window,
            button: MouseButton::Right,
            state,
        });
        pointer(&mut app, window, image_point, action);
    }
    warm(&mut app, 4);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
    assert!(!app.world().resource::<Editor>().asset_viewer_open());
    let menu = app.world().resource::<ContextUi>().menu;
    let menu = rect(&mut app, menu);
    assert!(menu.min.x >= 0.0 && menu.min.y >= 0.0 && menu.max.x <= 1000.0 && menu.max.y <= 620.0);
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert_eq!(app.world().resource::<Editor>().view, original_view);
    assert_eq!(app.world().resource::<Editor>().edits, original_edits);
    println!(
        "Browser import and folder GPU artifacts: {}",
        output.display()
    );
}
