use super::*;
use crate::editor::{
    RoomEdits,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture, mouse, move_pointer},
};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    ui::ComputedStackIndex,
};

fn editor() -> Editor {
    let project = crate::EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    Editor::new(project, RoomEdits::default())
}

#[test]
fn generation_tabs_preserve_documents_and_draft() {
    let mut editor = editor();
    let original = editor.edits.clone();
    editor.open_image_tab(PathBuf::from("unsaved.png"));
    let image = editor.viewing_image.unwrap();
    editor.image_tabs[0].dirty = true;
    editor.open_generation();
    editor.generation.name = "My mesh".into();
    editor.generation.set_quality(Quality::Detailed);
    editor.open_generation();
    assert!(editor.asset_viewer_open());
    assert!(editor.generation.active && editor.generation.opened);
    assert_eq!(editor.viewing_image, None);
    editor.close_generation();
    assert_eq!(editor.viewing_image, Some(image));
    assert!(editor.image_tabs[0].dirty);
    assert_eq!(editor.edits, original);
    assert!(editor.undo.is_empty());
    editor.open_generation();
    assert_eq!(editor.generation.name, "My mesh");
    assert_eq!(editor.generation.steps, 50);
    editor.open_document(0);
    assert!(!editor.generation.active && editor.generation.opened);
    editor.close_generation();
    assert!(editor.has_scene());
    assert_eq!(editor.viewing_image, None);
}

#[test]
fn generation_service_stop_is_available_outside_its_tab() {
    let mut editor = editor();
    editor.open_generation();
    assert!(available(&editor, GenerationAction::StartModel));
    editor.generation.engine_active = true;
    editor.generation.engine_loading = true;
    assert!(!available(&editor, GenerationAction::StartModel));
    editor.close_generation();
    assert!(available(&editor, GenerationAction::StopModel));
    assert!(
        editor.generation.engine_active,
        "Closing a document tab must not turn off its service"
    );
    editor.browser_popup = true;
    assert!(available(&editor, GenerationAction::StopModel));
    editor.generation.engine_active = false;
    assert!(!available(&editor, GenerationAction::StopModel));
}

#[test]
fn generation_closing_source_tab_and_empty_workspace_is_safe() {
    let mut editor = editor();
    editor.open_image_tab(PathBuf::from("image.png"));
    let image = editor.viewing_image.unwrap();
    editor.open_generation();
    editor.close_image_tab(image);
    editor.close_document(0);
    assert!(editor.generation.active);
    editor.close_generation();
    assert!(!editor.generation.active);
    assert!(!editor.has_scene());
    assert!(editor.viewing_image.is_none());
    editor.open_generation();
    editor.open_image_tab(PathBuf::from("new.png"));
    editor.close_image_tab(editor.viewing_image.unwrap());
    assert!(
        editor.generation.active,
        "Closing an image opened from generation returns to generation"
    );
}

#[test]
fn generation_footer_toggle_restores_previous_document_and_keeps_draft() {
    let mut editor = editor();
    editor.open_image_tab(PathBuf::from("unsaved.png"));
    let image = editor.viewing_image.unwrap();
    editor.image_tabs[0].dirty = true;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<crate::editor::Scene>()
        .init_resource::<crate::editor::viewport::PointerState>()
        .add_message::<Action>()
        .add_systems(Update, crate::editor::handle_actions);

    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Toggle));
    app.update();
    assert!(app.world().resource::<Editor>().generation.active);
    app.world_mut().resource_mut::<Editor>().generation.name = "Saved draft".into();
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Open));
    app.update();
    assert!(app.world().resource::<Editor>().generation.active);
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Toggle));
    app.update();
    let editor = app.world().resource::<Editor>();
    assert!(!editor.generation.opened && !editor.generation.active);
    assert_eq!(editor.viewing_image, Some(image));
    assert!(editor.image_tabs[0].dirty);

    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Toggle));
    app.update();
    assert_eq!(
        app.world().resource::<Editor>().generation.name,
        "Saved draft"
    );
    app.world_mut().resource_mut::<Editor>().open_document(0);
    // An open but inactive generation tab is activated by its footer button.
    assert!(app.world().resource::<Editor>().generation.opened);
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Toggle));
    app.update();
    assert!(app.world().resource::<Editor>().generation.active);
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Toggle));
    app.update();
    let editor = app.world().resource::<Editor>();
    assert!(!editor.generation.active && !editor.generation.opened);
    assert!(editor.has_scene());
    assert!(editor.viewing_image.is_none());
    assert!(editor.image_tabs[0].dirty);
}

#[test]
fn generation_defaults_validation_and_keyboard_isolation() {
    let mut editor = editor();
    assert_eq!(
        (
            editor.generation.steps,
            editor.generation.resolution,
            editor.generation.chunks,
            editor.generation.seed
        ),
        (30, 380, 20_000, 12345)
    );
    for (kind, value) in [
        (Field::Steps, "0"),
        (Field::Steps, "101"),
        (Field::Guidance, "NaN"),
        (Field::Resolution, "513"),
        (Field::Chunks, "100"),
        (Field::Seed, "-1"),
        (Field::Faces, "99"),
        (Field::Name, " "),
    ] {
        assert!(!apply_field(&mut editor.generation, kind, value));
    }
    assert!(apply_field(&mut editor.generation, Field::Guidance, "5,5"));
    assert_eq!(editor.generation.guidance, 5.5);
    assert!(Workspace::accepts(std::path::Path::new("source.JPEG")));
    assert!(!Workspace::accepts(std::path::Path::new("source.svg")));
    editor.open_generation();
    assert!(!available(&editor, GenerationAction::Generate));
    let browser = Browser::new(&mut editor.project);
    assert!(!super::super::available(&editor, &browser, Action::Delete));
    assert!(!super::super::available(&editor, &browser, Action::Undo));
    assert!(super::super::available(&editor, &browser, Action::Open(0)));
}

#[test]
fn generation_source_accepts_supported_image_cards_and_rejects_other_entries() {
    let mut editor = editor();
    let mut browser = Browser::new(&mut editor.project);
    for name in ["source.png", "photo.JPG", "photo.jpeg", "source.WebP"] {
        browser.files[0].path = name.into();
        assert_eq!(source_file(&browser, browser::Entry::Image(0)), Some(0));
        assert_eq!(source_file(&browser, browser::Entry::File(0)), None);
    }
    for name in ["source.svg", "source.pcraft", "model.glb", "notes.txt"] {
        browser.files[0].path = name.into();
        assert_eq!(source_file(&browser, browser::Entry::Image(0)), None);
    }
    for entry in [
        browser::Entry::Folder(0),
        browser::Entry::Model(0),
        browser::Entry::Scene { file: 0, room: 0 },
        browser::Entry::Image(usize::MAX),
    ] {
        assert_eq!(source_file(&browser, entry), None);
    }
}

fn warm(app: &mut App, count: usize) {
    for _ in 0..count {
        frame(app);
    }
}
fn click(app: &mut App, window: Entity, label: &str) {
    let point = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(l, n, _)| l.0 == label && !n.is_empty())
            .map(|(_, _, t)| t.translation)
            .unwrap_or_else(|| panic!("Missing visible control: {label}"))
    };
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 5);
}

fn toggle_browser_shortcut(app: &mut App, window: Entity) {
    for (key_code, logical_key, state) in [
        (KeyCode::ControlLeft, Key::Control, ButtonState::Pressed),
        (KeyCode::Space, Key::Space, ButtonState::Pressed),
        (KeyCode::Space, Key::Space, ButtonState::Released),
        (KeyCode::ControlLeft, Key::Control, ButtonState::Released),
    ] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key,
            state,
            text: None,
            repeat: false,
        });
        warm(app, 2);
    }
    warm(app, 18);
}

#[test]
#[ignore = "requires GPU; drags real browser image cards into generation, replaces the source and rejects invalid or cancelled drops"]
fn gpu_generation_accepts_browser_image_drops_without_moving_files_or_opening_tabs() {
    struct Files(PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let files = Files(std::env::temp_dir().join(format!(
        "bevy-generation-drag-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&files.0).unwrap();
    for (name, size, color) in [
        ("first.png", (120, 80), [100, 190, 160, 255]),
        ("second.png", (96, 64), [220, 120, 65, 255]),
    ] {
        image::RgbaImage::from_pixel(size.0, size.1, image::Rgba(color))
            .save(files.0.join(name))
            .unwrap();
    }
    std::fs::write(
        files.0.join("unsupported.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"/>"#,
    )
    .unwrap();
    let (mut app, window, target) = fixture();
    let before = app.world().resource::<Editor>().edits.clone();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = files.0.clone();
            editor.project.asset_root = files.0.clone();
            browser.refresh(&mut editor.project);
        });
    warm(&mut app, 5);
    click(&mut app, window, "Переключить 3D генерацию");
    app.world_mut()
        .insert_resource(installation::State::installed_fixture());
    app.world_mut().insert_resource(run::State::ready_fixture());
    warm(&mut app, 3);
    click(&mut app, window, "Файловый менеджер · Ctrl+Space");
    warm(&mut app, 20);
    let source = {
        let world = app.world_mut();
        world
            .query_filtered::<Entity, With<SourceDrop>>()
            .single(world)
            .unwrap()
    };
    let destination = app
        .world()
        .get::<UiGlobalTransform>(source)
        .unwrap()
        .translation;
    let begin = |app: &mut App, name: &str, destination| {
        if !app.world().resource::<Editor>().drawer {
            click(app, window, "Файловый менеджер · Ctrl+Space");
            warm(app, 20);
        }
        let file = app
            .world()
            .resource::<Browser>()
            .files
            .iter()
            .position(|file| file.filename == name)
            .unwrap();
        let start = {
            let world = app.world_mut();
            world
                .query::<(&drawer::FileCard, &UiGlobalTransform)>()
                .iter(world)
                .find_map(|(card, transform)| {
                    (card.0 == browser::Entry::Image(file)).then_some(transform.translation)
                })
                .unwrap()
        };
        move_pointer(app, window, start);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, destination);
        assert!(app.world().resource::<AssetDrag>().active());
    };
    for (name, size) in [
        ("first.png", UVec2::new(120, 80)),
        ("second.png", UVec2::new(96, 64)),
    ] {
        let original = std::fs::read(files.0.join(name)).unwrap();
        begin(&mut app, name, destination);
        assert_eq!(
            app.world().resource::<AssetDrag>().target,
            Some(drawer::DropTarget::GenerationSource)
        );
        assert_eq!(
            *app.world().get::<BorderColor>(source).unwrap(),
            BorderColor::all(GREEN)
        );
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-qa");
        std::fs::create_dir_all(&output).unwrap();
        capture_when(
            &mut app,
            &target,
            &output.join(if name == "first.png" {
                "04-image-drop-hover.png"
            } else {
                "06-image-replace-hover.png"
            }),
            |_| true,
        );
        mouse(&mut app, window, ButtonState::Released);
        assert!(
            !app.world().resource::<Editor>().drawer,
            "An accepted source image must collapse the file manager"
        );
        for _ in 0..120 {
            frame(&mut app);
            if app.world().resource::<GenerationUi>().loaded.as_ref() == Some(&files.0.join(name)) {
                break;
            }
        }
        let ui = app.world().resource::<GenerationUi>();
        assert_eq!(ui.loaded.as_ref(), Some(&files.0.join(name)));
        assert_eq!(ui.dimensions, Some(size));
        assert!(ui.texture.is_some());
        assert_eq!(std::fs::read(files.0.join(name)).unwrap(), original);
        warm(&mut app, 20);
        assert_eq!(app.world().resource::<DrawerMotion>().visible_height, 0.0);
        if name == "first.png" {
            capture_when(
                &mut app,
                &target,
                &output.join("05-image-drop-collapsed.png"),
                |_| true,
            );
        }
    }
    let selected = Some(files.0.join("second.png"));
    for (name, point) in [
        ("unsupported.svg", destination),
        ("first.png", Vec2::new(950.0, destination.y)),
    ] {
        begin(&mut app, name, point);
        assert_eq!(app.world().resource::<AssetDrag>().target, None);
        mouse(&mut app, window, ButtonState::Released);
        assert_eq!(app.world().resource::<Editor>().generation.source, selected);
        assert!(app.world().resource::<Editor>().drawer);
    }
    begin(&mut app, "first.png", destination);
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state,
            text: None,
            repeat: false,
        });
        warm(&mut app, 2);
    }
    mouse(&mut app, window, ButtonState::Released);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.generation.source, selected);
    assert!(editor.drawer && editor.generation.active);
    assert!(editor.viewing_image.is_none() && editor.image_tabs.is_empty());
    assert_eq!(editor.edits, before);
    assert!(!app.world().resource::<AssetDrag>().active());
}

#[test]
#[ignore = "requires GPU; clicks the generation footer twice and browses files above the active generation workspace"]
fn gpu_generation_footer_toggles_and_file_manager_stays_above_workspace() {
    let (mut app, window, target) = fixture();
    warm(&mut app, 25);
    let before = app.world().resource::<Editor>().edits.clone();
    let room = app.world().resource::<Editor>().room;
    click(&mut app, window, "Переключить 3D генерацию");
    app.world_mut()
        .insert_resource(installation::State::installed_fixture());
    app.world_mut().insert_resource(run::State::ready_fixture());
    warm(&mut app, 3);
    click(&mut app, window, "Детально");
    // The upper document tab activates the workspace; only the footer toggles it.
    click(&mut app, window, "3D генерация · активная вкладка");
    assert!(app.world().resource::<Editor>().generation.active);
    let generation = app.world().resource::<GenerationUi>().root;
    let geometry = |app: &App| {
        let node = app.world().get::<ComputedNode>(generation).unwrap();
        let transform = app.world().get::<UiGlobalTransform>(generation).unwrap();
        Rect::from_center_size(transform.translation, node.size())
    };
    let fixed = geometry(&app);
    click(&mut app, window, "Файловый менеджер · Ctrl+Space");
    warm(&mut app, 18);
    assert!(app.world().resource::<Editor>().drawer);
    assert!(app.world().resource::<Editor>().generation.active);
    assert_eq!(geometry(&app), fixed);
    let panel = app.world().resource::<drawer::BrowserUi>().panel;
    assert!(
        app.world().get::<ComputedStackIndex>(panel).unwrap().0
            > app.world().get::<ComputedStackIndex>(generation).unwrap().0,
        "The file manager must render above generation and receive its pointer input"
    );
    let folder = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == PathBuf::from("models"))
        .unwrap();
    let point = {
        let world = app.world_mut();
        world
            .query_filtered::<(&Control, &ComputedNode, &UiGlobalTransform), With<AssetOpen>>()
            .iter(world)
            .find_map(|(control, node, transform)| {
                (matches!(control.look, Look::Regular)
                    && matches!(control.action, Action::Browse(id) if id == folder)
                    && !node.is_empty())
                .then_some(transform.translation)
            })
            .unwrap()
    };
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<drawer::BrowserUi>().selected_entry,
        Some(browser::Entry::Folder(folder))
    );
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    let browser = app.world().resource::<Browser>();
    assert_eq!(
        browser.folders[browser.current].path,
        PathBuf::from("models")
    );
    assert!(app.world().resource::<Editor>().generation.active);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("03-file-manager-overlay.png"),
        |_| true,
    );

    click(&mut app, window, "Файловый менеджер · Ctrl+Space");
    warm(&mut app, 18);
    assert!(!app.world().resource::<Editor>().drawer);
    toggle_browser_shortcut(&mut app, window);
    assert!(app.world().resource::<Editor>().drawer);
    assert_eq!(geometry(&app), fixed);
    toggle_browser_shortcut(&mut app, window);
    assert!(!app.world().resource::<Editor>().drawer);

    click(&mut app, window, "Переключить 3D генерацию");
    let editor = app.world().resource::<Editor>();
    assert!(!editor.generation.active && !editor.generation.opened);
    assert!(editor.has_scene());
    assert_eq!(editor.room, room);
    assert_eq!(editor.edits, before);
    click(&mut app, window, "Переключить 3D генерацию");
    let editor = app.world().resource::<Editor>();
    assert!(editor.generation.active && editor.generation.opened);
    assert_eq!(editor.generation.quality, Quality::Detailed);
    assert_eq!(editor.generation.resolution, 512);
    click(&mut app, window, "Переключить 3D генерацию");
    assert!(!app.world().resource::<Editor>().generation.opened);
}

#[test]
#[ignore = "requires GPU adapter; verifies real footer, tab, settings, image upload and disabled backend"]
fn gpu_generation_workspace_native_controls_and_image_preview() {
    let (mut app, window, target) = fixture();
    warm(&mut app, 45);
    let before = app.world().resource::<Editor>().edits.clone();
    click(&mut app, window, "Переключить 3D генерацию");
    app.world_mut()
        .insert_resource(installation::State::installed_fixture());
    app.world_mut().insert_resource(run::State::ready_fixture());
    warm(&mut app, 3);
    assert!(app.world().resource::<Editor>().generation.active);
    assert!(!app.world().resource::<Editor>().dirty());
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("01-generation.png"), |_| {
        true
    });
    click(&mut app, window, "Детально");
    assert_eq!(app.world().resource::<Editor>().generation.resolution, 512);
    click(&mut app, window, "Расширенные параметры");
    assert!(app.world().resource::<GenerationUi>().advanced_open);
    let path =
        std::env::temp_dir().join(format!("bevy-generation-input-{}.png", std::process::id()));
    image::RgbaImage::from_fn(120, 80, |x, y| {
        if (20..100).contains(&x) && (10..70).contains(&y) {
            image::Rgba([100, 190, 160, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    })
    .save(&path)
    .unwrap();
    app.world_mut().write_message(FileDragAndDrop::DroppedFile {
        window,
        path_buf: path.clone(),
    });
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<GenerationUi>().texture.is_some() {
            break;
        }
    }
    assert_eq!(
        app.world().resource::<GenerationUi>().dimensions,
        Some(UVec2::new(120, 80))
    );
    let generate = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find_map(|(e, c)| {
                matches!(c.action, Action::Generation(GenerationAction::Generate)).then_some(e)
            })
            .unwrap()
    };
    assert!(app.world().get::<InteractionDisabled>(generate).is_some());
    // Even an injected generate command cannot start a model or modify documents.
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Generate));
    warm(&mut app, 3);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    capture_when(
        &mut app,
        &target,
        &output.join("02-generation-image-and-settings.png"),
        |_| true,
    );
    click(&mut app, window, "Выбрать из проекта");
    assert!(app.world().resource::<GenerationUi>().picking);
    click(&mut app, window, "Закрыть");
    assert!(!app.world().resource::<GenerationUi>().picking);
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Close));
    warm(&mut app, 5);
    assert!(!app.world().resource::<Editor>().generation.active);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    std::fs::remove_file(path).unwrap();
}
