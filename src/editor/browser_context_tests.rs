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
#[ignore = "requires GPU adapter; creates a white PNG through the real context menu and size dialog"]
fn gpu_browser_creates_white_images_with_presets_custom_sizes_and_cancel() {
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
    let preset = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find(|(_, control)| matches!(control.action, Action::BrowserImageSize(1920, 1080)))
            .unwrap()
            .0
    };
    let point = rect(&mut app, preset).center();
    click(&mut app, window, point, MouseButton::Left);
    assert_eq!(
        app.world()
            .get::<EditableText>(width)
            .unwrap()
            .value()
            .to_string(),
        "1920"
    );
    assert_eq!(
        app.world()
            .get::<EditableText>(height)
            .unwrap()
            .value()
            .to_string(),
        "1080"
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
    for name in ["picture.png", "logo.svg"] {
        app.world_mut().write_message(FileDragAndDrop::DroppedFile {
            window,
            path_buf: fixture.source(name),
        });
    }
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    finish_import(&mut app);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
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
