use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "bevy-black-hole-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("images")).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn browser_delete_trash_roundtrip_preserves_bytes_and_original_path() {
    let files = Files::new();
    let original = PathBuf::from("images/Звёздное небо.png");
    std::fs::write(files.0.join(&original), b"image bytes").unwrap();
    let removed = trash::remove(&files.0, &original).unwrap();
    assert!(!files.0.join(&original).exists());
    assert_eq!(removed.original, original);
    trash::restore(&files.0, &removed).unwrap();
    assert_eq!(
        std::fs::read(files.0.join(original)).unwrap(),
        b"image bytes"
    );
}

#[test]
fn browser_delete_restore_never_replaces_a_new_file() {
    let files = Files::new();
    std::fs::write(files.0.join("image.png"), b"old").unwrap();
    let removed = trash::remove(&files.0, std::path::Path::new("image.png")).unwrap();
    std::fs::write(files.0.join("image.png"), b"new").unwrap();
    assert!(trash::restore(&files.0, &removed).is_err());
    assert_eq!(std::fs::read(files.0.join("image.png")).unwrap(), b"new");
    std::fs::remove_file(files.0.join("image.png")).unwrap();
    trash::restore(&files.0, &removed).unwrap();
    assert_eq!(std::fs::read(files.0.join("image.png")).unwrap(), b"old");
}

#[test]
fn browser_delete_rejects_parent_paths_directories_and_its_own_trash() {
    let files = Files::new();
    for path in [
        "../outside.png",
        "images/../image.png",
        "images",
        "",
        ".bevy-editor-trash/file",
    ] {
        assert!(trash::remove(&files.0, std::path::Path::new(path)).is_err());
    }
    assert!(files.0.join("images").is_dir());
}

#[test]
fn browser_delete_same_named_files_have_independent_trash_records() {
    let files = Files::new();
    std::fs::write(files.0.join("same.png"), b"first").unwrap();
    std::fs::write(files.0.join("images/same.png"), b"second").unwrap();
    let first = trash::remove(&files.0, std::path::Path::new("same.png")).unwrap();
    let second = trash::remove(&files.0, std::path::Path::new("images/same.png")).unwrap();
    trash::restore(&files.0, &second).unwrap();
    trash::restore(&files.0, &first).unwrap();
    assert_eq!(std::fs::read(files.0.join("same.png")).unwrap(), b"first");
    assert_eq!(
        std::fs::read(files.0.join("images/same.png")).unwrap(),
        b"second"
    );
}

#[cfg(windows)]
#[test]
fn browser_delete_locked_file_stays_in_place() {
    use std::os::windows::fs::OpenOptionsExt;
    let files = Files::new();
    let path = files.0.join("locked.png");
    std::fs::write(&path, b"keep these bytes").unwrap();
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(trash::remove(&files.0, std::path::Path::new("locked.png")).is_err());
    assert!(path.exists());
    drop(locked);
    assert_eq!(std::fs::read(path).unwrap(), b"keep these bytes");
}

#[test]
fn browser_delete_empty_trash_preserves_active_and_restored_files() {
    let files = Files::new();
    std::fs::write(files.0.join("keep.png"), b"active").unwrap();
    std::fs::write(files.0.join("deleted.png"), b"deleted").unwrap();
    std::fs::write(files.0.join("restored.png"), b"restored").unwrap();
    trash::remove(&files.0, std::path::Path::new("deleted.png")).unwrap();
    let restored = trash::remove(&files.0, std::path::Path::new("restored.png")).unwrap();
    trash::restore(&files.0, &restored).unwrap();
    trash::empty(&files.0).unwrap();
    assert!(!files.0.join(".bevy-editor-trash").exists());
    assert!(!files.0.join("deleted.png").exists());
    assert_eq!(std::fs::read(files.0.join("keep.png")).unwrap(), b"active");
    assert_eq!(
        std::fs::read(files.0.join("restored.png")).unwrap(),
        b"restored"
    );
}

#[test]
fn browser_delete_empty_trash_is_idempotent_without_creating_directories() {
    let files = Files::new();
    trash::empty(&files.0).unwrap();
    trash::empty(&files.0).unwrap();
    assert!(!files.0.join(".bevy-editor-trash").exists());
    assert!(files.0.join("images").is_dir());
}

#[cfg(any(windows, unix))]
fn directory_link(link: &std::path::Path, target: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:BEVY_TRASH_TEST_LINK -Target $env:BEVY_TRASH_TEST_TARGET -ErrorAction Stop | Out-Null"])
            .env("BEVY_TRASH_TEST_LINK", link)
            .env("BEVY_TRASH_TEST_TARGET", target)
            .creation_flags(0x08000000)
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(any(windows, unix))]
#[test]
fn browser_delete_empty_trash_never_follows_child_directory_links() {
    let files = Files::new();
    let outside = Files::new();
    std::fs::write(outside.0.join("keep.txt"), b"outside trash").unwrap();
    let trash_path = files.0.join(".bevy-editor-trash");
    std::fs::create_dir(&trash_path).unwrap();
    directory_link(&trash_path.join("external"), &outside.0);
    trash::empty(&files.0).unwrap();
    assert!(!trash_path.exists());
    assert_eq!(
        std::fs::read(outside.0.join("keep.txt")).unwrap(),
        b"outside trash"
    );
}

#[cfg(any(windows, unix))]
#[test]
fn browser_delete_empty_trash_rejects_a_redirected_trash_root() {
    let files = Files::new();
    let outside = Files::new();
    std::fs::write(outside.0.join("keep.txt"), b"outside project").unwrap();
    directory_link(&files.0.join(".bevy-editor-trash"), &outside.0);
    assert!(trash::empty(&files.0).is_err());
    assert_eq!(
        std::fs::read(outside.0.join("keep.txt")).unwrap(),
        b"outside project"
    );
}

fn exit_app(files: &Files) -> (App, Entity) {
    let mut project = crate::project::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    project.asset_root = files.0.clone();
    let mut editor = Editor::new(project, crate::editor::RoomEdits::default());
    editor.ready = true;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(editor)
        .init_resource::<crate::editor::Scene>()
        .init_resource::<crate::editor::viewport::PointerState>()
        .init_resource::<Deletion>()
        .add_message::<Action>()
        .add_message::<bevy::window::WindowCloseRequested>()
        .add_systems(
            Update,
            (crate::editor::handle_actions, crate::editor::close_editor).chain(),
        )
        .add_systems(Last, empty_trash_on_exit);
    let window = app.world_mut().spawn_empty().id();
    (app, window)
}

#[test]
fn browser_delete_clean_editor_exit_empties_existing_trash() {
    let files = Files::new();
    std::fs::write(files.0.join("deleted.png"), b"deleted").unwrap();
    trash::remove(&files.0, std::path::Path::new("deleted.png")).unwrap();
    let (mut app, window) = exit_app(&files);
    app.update();
    assert!(files.0.join(".bevy-editor-trash").exists());
    app.world_mut()
        .write_message(bevy::window::WindowCloseRequested { window });
    app.update();
    assert_eq!(app.should_exit(), Some(AppExit::Success));
    assert!(!files.0.join(".bevy-editor-trash").exists());
}

#[test]
fn browser_delete_exit_cancel_retains_trash_until_discard_is_confirmed() {
    let files = Files::new();
    std::fs::write(files.0.join("deleted.png"), b"deleted").unwrap();
    let deleted = trash::remove(&files.0, std::path::Path::new("deleted.png")).unwrap();
    let (mut app, window) = exit_app(&files);
    app.world_mut()
        .resource_mut::<Deletion>()
        .history
        .push(deleted);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let camera = editor.project.scenes[0].cameras[0].clone();
        editor.edits.cameras.insert(camera.id, camera.placement);
        assert!(editor.dirty());
    }
    app.world_mut()
        .write_message(bevy::window::WindowCloseRequested { window });
    app.update();
    assert_eq!(
        app.world().resource::<Editor>().pending,
        Some(crate::editor::PendingAction::Close)
    );
    assert!(app.should_exit().is_none());
    assert!(files.0.join(".bevy-editor-trash").exists());
    app.world_mut().write_message(Action::CancelPending);
    app.update();
    assert!(app.world().resource::<Editor>().pending.is_none());
    assert!(app.should_exit().is_none());
    assert!(files.0.join(".bevy-editor-trash").exists());
    assert_eq!(app.world().resource::<Deletion>().history.len(), 1);
    app.world_mut()
        .write_message(bevy::window::WindowCloseRequested { window });
    app.update();
    app.world_mut().write_message(Action::DiscardAndContinue);
    app.update();
    assert_eq!(app.should_exit(), Some(AppExit::Success));
    assert!(!files.0.join(".bevy-editor-trash").exists());
    assert!(app.world().resource::<Deletion>().history.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
mod gpu {
    use super::*;
    use crate::editor::{
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render,
    };
    use bevy::{
        input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
            mouse::MouseButtonInput,
        },
        picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
        time::TimeUpdateStrategy,
    };

    fn warm(app: &mut App, count: usize) {
        for _ in 0..count {
            frame(app);
        }
    }
    fn rect(app: &App, entity: Entity) -> Rect {
        let world = app.world();
        Rect::from_center_size(
            world
                .get::<UiGlobalTransform>(entity)
                .unwrap()
                .affine()
                .translation,
            world.get::<ComputedNode>(entity).unwrap().size(),
        )
    }
    fn mouse(app: &mut App, window: Entity, point: Vec2, button: MouseButton, pressed: bool) {
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(point));
        let target = {
            let world = app.world_mut();
            world
                .query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>()
                .single(world)
                .unwrap()
                .normalize(Some(window))
                .unwrap()
        };
        let pointer_button = if button == MouseButton::Right {
            PointerButton::Secondary
        } else {
            PointerButton::Primary
        };
        for action in [
            PointerAction::Move { delta: Vec2::ONE },
            if pressed {
                PointerAction::Press(pointer_button)
            } else {
                PointerAction::Release(pointer_button)
            },
        ] {
            app.world_mut().write_message(PointerInput {
                pointer_id: PointerId::Mouse,
                location: Location {
                    target: target.clone(),
                    position: point,
                },
                action,
            });
        }
        app.world_mut().write_message(MouseButtonInput {
            window,
            button,
            state: if pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
        });
        warm(app, 2);
    }
    fn click(app: &mut App, window: Entity, point: Vec2, button: MouseButton) {
        mouse(app, window, point, button, true);
        mouse(app, window, point, button, false);
    }
    fn keyboard(app: &mut App, window: Entity, code: KeyCode, key: Key, state: ButtonState) {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: code,
            logical_key: key,
            state,
            text: None,
            repeat: false,
        });
        warm(app, 2);
    }
    fn card(app: &mut App, name: &str) -> Entity {
        let world = app.world_mut();
        let browser = world.resource::<Browser>();
        let index = browser
            .files
            .iter()
            .position(|file| file.filename == name)
            .unwrap();
        world
            .query::<(Entity, &drawer::FileCard)>()
            .iter(world)
            .find_map(|(entity, card)| {
                matches!(card.0, Entry::Image(file) if file == index).then_some(entity)
            })
            .unwrap()
    }

    #[test]
    #[ignore = "requires GPU adapter; captures the actual card shader and exercises delete/restore input"]
    fn gpu_browser_delete_black_hole_consumes_card_and_restores_file() {
        let files = Files::new();
        let source = files.0.join("nebula.png");
        image::RgbaImage::from_fn(320, 200, |x, y| {
            image::Rgba([
                220,
                (x * 180 / 320 + 45) as u8,
                (y * 160 / 200 + 35) as u8,
                255,
            ])
        })
        .save(&source)
        .unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let (mut app, window, target) = lighting_ui_render::fixture();
        app.world_mut().write_message(Action::Add(0));
        warm(&mut app, 3);
        let edits = app.world().resource::<Editor>().edits.clone();
        app.world_mut()
            .resource_scope(|world, mut browser: Mut<Browser>| {
                let mut editor = world.resource_mut::<Editor>();
                editor.root = files.0.clone();
                editor.project.asset_root = files.0.clone();
                editor.drawer = true;
                browser.refresh(&mut editor.project);
            });
        warm(&mut app, 48);
        let entity = card(&mut app, "nebula.png");
        let source_rect = rect(&app, entity);
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/black-hole-delete-qa");
        std::fs::create_dir_all(&output).unwrap();
        capture_when(&mut app, &target, &output.join("00-before.png"), |_| true);
        click(&mut app, window, source_rect.center(), MouseButton::Right);
        assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
        let remove = app.world().resource::<Menu>().remove;
        assert!(app.world().get::<Node>(remove).unwrap().display != Display::None);
        app.world_mut().trigger(Activate { entity: remove });
        for _ in 0..90 {
            frame(&mut app);
            if app
                .world()
                .resource::<Deletion>()
                .pending
                .as_ref()
                .is_some_and(|pending| pending.effect.is_some())
            {
                break;
            }
        }
        assert!(!source.exists(), "The file should be in recoverable trash");
        assert_eq!(
            *app.world().get::<Visibility>(entity).unwrap(),
            Visibility::Hidden
        );
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        for (index, progress) in [(1, 0.12), (2, 0.42), (3, 0.75)] {
            app.world_mut()
                .resource_mut::<Deletion>()
                .pending
                .as_mut()
                .unwrap()
                .age = DURATION * progress;
            warm(&mut app, 3);
            let image = capture_when(
                &mut app,
                &target,
                &output.join(format!("0{index}-consuming.png")),
                |_| true,
            );
            if index == 2 {
                let centre = source_rect.center().as_uvec2();
                let pixel = image.get_pixel(centre.x, centre.y).0;
                assert!(
                    pixel[0] < 45 && pixel[1] < 45 && pixel[2] < 45,
                    "The event horizon must be black: {pixel:?}"
                );
                let lit = image
                    .enumerate_pixels()
                    .filter(|(x, y, pixel)| {
                        source_rect.contains(Vec2::new(*x as f32, *y as f32))
                            && pixel[0] > 180
                            && pixel[1] > 65
                    })
                    .count();
                assert!(
                    lit > 30,
                    "The material must render a luminous accretion disk"
                );
            }
        }
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
        warm(&mut app, 45);
        assert!(app.world().resource::<Deletion>().pending.is_none());
        assert!(
            !app.world()
                .resource::<Browser>()
                .files
                .iter()
                .any(|file| file.filename == "nebula.png")
        );
        assert_eq!(app.world().resource::<Editor>().edits, edits);
        keyboard(
            &mut app,
            window,
            KeyCode::ControlLeft,
            Key::Control,
            ButtonState::Pressed,
        );
        keyboard(
            &mut app,
            window,
            KeyCode::KeyZ,
            Key::Character("z".into()),
            ButtonState::Pressed,
        );
        keyboard(
            &mut app,
            window,
            KeyCode::KeyZ,
            Key::Character("z".into()),
            ButtonState::Released,
        );
        keyboard(
            &mut app,
            window,
            KeyCode::ControlLeft,
            Key::Control,
            ButtonState::Released,
        );
        warm(&mut app, 8);
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert!(app.world().resource::<Deletion>().history.is_empty());
        let entity = card(&mut app, "nebula.png");
        let point = rect(&app, entity).center();
        click(&mut app, window, point, MouseButton::Left);
        keyboard(
            &mut app,
            window,
            KeyCode::Delete,
            Key::Delete,
            ButtonState::Pressed,
        );
        keyboard(
            &mut app,
            window,
            KeyCode::Delete,
            Key::Delete,
            ButtonState::Released,
        );
        warm(&mut app, 100);
        assert!(!source.exists());
        assert_eq!(
            app.world().resource::<Editor>().edits,
            edits,
            "Delete in the browser must leave 3D objects untouched"
        );
        assert_eq!(
            app.world_mut()
                .query::<&MaterialNode<BlackHoleMaterial>>()
                .iter(app.world())
                .count(),
            0
        );
        app.world_mut().write_message(Action::BrowserRestoreFile);
        warm(&mut app, 8);
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }
}
