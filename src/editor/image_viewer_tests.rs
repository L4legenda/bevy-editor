use super::*;
use crate::editor::{RoomEdits, browser::Browser, tests::lighting_ui_render};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bevy-editor-image-viewer-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a-landscape.SVG"), br##"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="900" viewBox="0 0 1600 900">
            <defs><linearGradient id="sky" x2="0" y2="1"><stop stop-color="#152939"/><stop offset="1" stop-color="#527b80"/></linearGradient></defs>
            <rect width="1600" height="900" fill="url(#sky)"/>
            <circle cx="1160" cy="280" r="120" fill="#e4b46a"/>
            <path d="M0 760L420 250L920 850L1260 470L1600 800V900H0Z" fill="#173b40"/>
            <path d="M0 880L720 510L1200 900Z" fill="#36716b"/>
        </svg>"##).unwrap();
        image::RgbaImage::from_fn(300, 900, |x, y| {
            if x > 38 && x < 262 && y > 55 && y < 845 {
                image::Rgba([70, 190, 160, if y < 450 { 255 } else { 128 }])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        })
        .save(root.join("b-transparent.png"))
        .unwrap();
        std::fs::write(root.join("c-broken.png"), b"broken").unwrap();
        std::fs::write(root.join("readme.txt"), b"ordinary file").unwrap();
        image::RgbaImage::from_pixel(24, 12, image::Rgba([210, 160, 80, 128]))
            .save(root.join("nested/d-small.png"))
            .unwrap();
        Self(root)
    }

    fn project(&self) -> crate::EditorProject {
        let mut project = crate::EditorProject::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        project.asset_root = self.0.clone();
        project.catalog.clear();
        project
    }

    fn app(&self) -> App {
        let mut project = self.project();
        let browser = Browser::new(&mut project);
        let mut editor = Editor::new(project, RoomEdits::default());
        editor.drawer = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(editor)
            .insert_resource(browser)
            .init_resource::<ImageViewer>()
            .init_resource::<ImagePreviews>()
            .init_resource::<Assets<Image>>()
            .init_resource::<InputFocus>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<crate::editor::Scene>()
            .init_resource::<viewport::PointerState>()
            .add_message::<Action>()
            .add_systems(
                Update,
                (
                    super::super::shortcuts,
                    update,
                    crate::editor::handle_actions,
                )
                    .chain(),
            );
        app.world_mut().spawn((
            PrimaryWindow,
            Window {
                resolution: (1280, 760).into(),
                focused: true,
                ..default()
            },
        ));
        app
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn file(app: &App, filename: &str) -> usize {
    app.world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|file| file.filename == filename)
        .unwrap()
}

fn wait_for_image(app: &mut App) {
    for _ in 0..400 {
        app.update();
        let viewer = app.world().resource::<ImageViewer>();
        if viewer.image.is_some() || viewer.error.is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("Image worker did not finish");
}

#[test]
fn svg_decoder_preserves_color_alpha_and_original_dimensions() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("alpha.svg"), br##"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="500"><rect width="1000" height="500" fill="#c86432" opacity="0.5"/></svg>"##).unwrap();
    let image =
        image_previews::decode_image(&fixture.0.join("alpha.svg"), UVec2::new(200, 200)).unwrap();
    assert_eq!(image.original, UVec2::new(1000, 500));
    assert_eq!(image.size, UVec2::new(200, 100));
    assert!(
        image.rgba[..4]
            .iter()
            .zip([200u8, 100, 50, 128])
            .all(|(&actual, wanted)| actual.abs_diff(wanted) <= 2)
    );
}

#[test]
fn zoom_keeps_the_point_under_the_cursor_and_clamps_pan_on_resize() {
    let mut viewer = ImageViewer {
        dimensions: Some(UVec2::new(2000, 1000)),
        zoom: Zoom::Scale(1.0),
        ..default()
    };
    let anchor = Vec2::new(100.0, 50.0);
    viewer.zoom_at(2.0, anchor, Vec2::new(1000.0, 500.0));
    assert_eq!(viewer.pan, -anchor);
    assert_eq!(viewer.zoom, Zoom::Scale(2.0));
    viewer.pan = Vec2::splat(10000.0);
    viewer.clamp_pan(Vec2::new(1000.0, 500.0));
    assert_eq!(viewer.pan, Vec2::new(1500.0, 750.0));
    viewer.zoom = Zoom::Fit;
    viewer.clamp_pan(Vec2::new(700.0, 900.0));
    assert_eq!(viewer.pan, Vec2::ZERO);
    assert_eq!(
        Zoom::Fit.scale(Vec2::new(24.0, 12.0), Vec2::splat(900.0)),
        1.0
    );
    viewer.scale_factor = Some(2.0);
    assert_eq!(
        viewer.source_size(UVec2::new(2000, 1000)),
        Vec2::new(1000.0, 500.0)
    );
}

#[test]
fn image_tabs_load_navigate_recover_errors_and_close_independently() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let first = file(&app, "a-landscape.SVG");
    app.world_mut().write_message(Action::ViewImage(first));
    wait_for_image(&mut app);
    assert_eq!(
        app.world().resource::<ImageViewer>().dimensions,
        Some(UVec2::new(1600, 900))
    );
    assert!(
        neighbor(
            app.world().resource::<Editor>(),
            app.world().resource::<Browser>(),
            false
        )
        .is_none()
    );
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Next));
    wait_for_image(&mut app);
    assert_eq!(active_path(&app), PathBuf::from("b-transparent.png"));
    assert_eq!(
        app.world().resource::<ImageViewer>().dimensions,
        Some(UVec2::new(300, 900))
    );
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::ActualSize));
    app.update();
    assert!(app.world().resource::<ImageViewer>().is_actual_size());
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Next));
    wait_for_image(&mut app);
    assert_eq!(active_path(&app), PathBuf::from("c-broken.png"));
    assert!(app.world().resource::<ImageViewer>().error.is_some());
    assert!(
        neighbor(
            app.world().resource::<Editor>(),
            app.world().resource::<Browser>(),
            true
        )
        .is_none()
    );
    image::RgbaImage::from_pixel(16, 8, image::Rgba([255, 255, 255, 255]))
        .save(fixture.0.join("c-broken.png"))
        .unwrap();
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Retry));
    wait_for_image(&mut app);
    assert_eq!(
        app.world().resource::<ImageViewer>().dimensions,
        Some(UVec2::new(16, 8))
    );
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Previous));
    wait_for_image(&mut app);
    assert!(app.world().resource::<ImageViewer>().is_actual_size());
    let closing = app.world().resource::<Editor>().viewing_image.unwrap();
    app.world_mut()
        .write_message(Action::CloseImageTab(closing));
    app.update();
    assert_eq!(app.world().resource::<Editor>().image_tabs.len(), 2);
    assert_ne!(
        app.world().resource::<Editor>().viewing_image,
        Some(closing)
    );
    close_images(&mut app);
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
    assert!(app.world().resource::<Editor>().drawer);
    assert!(app.world().resource::<ImageViewer>().image.is_none());
}

#[test]
fn rapid_navigation_rejects_obsolete_results_and_viewer_shortcuts_leave_scene_untouched() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let first = file(&app, "a-landscape.SVG");
    let initial_tool = app.world().resource::<Editor>().tool;
    app.world_mut().write_message(Action::ViewImage(first));
    app.update();
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Next));
    wait_for_image(&mut app);
    assert_eq!(active_path(&app), PathBuf::from("b-transparent.png"));
    assert_eq!(
        app.world().resource::<ImageViewer>().dimensions,
        Some(UVec2::new(300, 900))
    );
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::KeyW);
        keys.press(KeyCode::Delete);
        keys.press(KeyCode::Digit1);
    }
    app.update();
    assert_eq!(app.world().resource::<Editor>().tool, initial_tool);
    assert!(app.world().resource::<ImageViewer>().is_actual_size());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert!(app.world().resource::<Editor>().viewing_image.is_some());
    assert!(!app.world().resource::<Editor>().drawer);
    let closing = app.world().resource::<Editor>().viewing_image.unwrap();
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyW);
    }
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    assert!(
        app.world()
            .resource::<Editor>()
            .image_tab(closing)
            .is_none()
    );
    close_images(&mut app);
    assert!(!app.world().resource::<Editor>().dirty());
}

#[test]
#[ignore = "requires GPU adapter; captures the actual browser and image viewer"]
fn gpu_image_gallery_and_viewer_match_editor_and_block_scene_input() {
    use crate::editor::selection_outline::render_tests::{capture_when, frame};
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/image-viewer-qa");
    std::fs::create_dir_all(&output).unwrap();
    let mut project = app.world().resource::<Editor>().project.clone();
    project.asset_root = fixture.0.clone();
    app.world_mut()
        .resource_mut::<Browser>()
        .refresh(&mut project);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.root = fixture.0.clone();
        editor.project.asset_root = fixture.0.clone();
        editor.drawer = true;
    }
    for _ in 0..90 {
        frame(&mut app);
    }
    capture_when(&mut app, &target, &output.join("01-gallery.png"), |_| true);
    // Double-click through real pointer picking, including drag suppression.
    let landscape = file(&app, "a-landscape.SVG");
    for _ in 0..2 {
        click_action(
            &mut app,
            window,
            |action| matches!(action, Action::ViewImage(index) if index == landscape),
        );
    }
    for _ in 0..90 {
        frame(&mut app);
    }
    assert!(app.world().resource::<ImageViewer>().image.is_some());
    let first_tab = app.world().resource::<Editor>().viewing_image.unwrap();
    assert_eq!(app.world().resource::<Editor>().image_tabs.len(), 1);
    click_action(&mut app, window, |action| matches!(action, Action::Open(0)));
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
    click_action(
        &mut app,
        window,
        |action| matches!(action, Action::OpenImageTab(id) if id == first_tab),
    );
    for _ in 0..12 {
        frame(&mut app);
    }
    assert_eq!(
        app.world().resource::<Editor>().viewing_image,
        Some(first_tab)
    );
    // Models and images remain separate documents in the common tab strip.
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..12 {
        frame(&mut app);
    }
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(0));
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
    click_action(
        &mut app,
        window,
        |action| matches!(action, Action::OpenImageTab(id) if id == first_tab),
    );
    click_action(&mut app, window, |action| {
        matches!(action, Action::CloseModelTab(0))
    });
    for _ in 0..12 {
        frame(&mut app);
    }
    assert_eq!(
        app.world().resource::<Editor>().viewing_image,
        Some(first_tab)
    );
    let pane = app.world().resource::<ViewerUi>().overlay;
    let pane_node = app.world().get::<ComputedNode>(pane).unwrap();
    let pane_transform = app.world().get::<UiGlobalTransform>(pane).unwrap();
    let pane_top = pane_transform.affine().translation.y - pane_node.size().y * 0.5;
    assert!(
        pane_top >= super::super::TOP - 1.0,
        "Image tab must leave the top document tabs accessible"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("02-viewer-fit.png"),
        |image| {
            image
                .pixels()
                .filter(|pixel| pixel[1] as u16 > pixel[0] as u16 + 25)
                .count()
                > 10000
        },
    );
    let scene_view = app.world().resource::<Editor>().view;
    let edits = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
    }
    assert_eq!(
        app.world().resource::<Editor>().viewing_image,
        Some(first_tab)
    );
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Next));
    for _ in 0..45 {
        frame(&mut app);
    }
    capture_when(
        &mut app,
        &target,
        &output.join("03-transparency.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::ActualSize));
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Background));
    for _ in 0..5 {
        frame(&mut app);
    }
    capture_when(
        &mut app,
        &target,
        &output.join("04-native-dark.png"),
        |_| true,
    );
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(1000.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: 1000,
            height: 620,
            depth_or_array_layers: 1,
        });
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Fit));
    for _ in 0..12 {
        frame(&mut app);
    }
    capture_when(&mut app, &target, &output.join("05-compact.png"), |_| true);
    {
        let world = app.world_mut();
        let mut controls = world.query::<(&Control, &ComputedNode, &UiGlobalTransform)>();
        for (control, node, transform) in controls.iter(world) {
            if matches!(control.action, Action::ImageView(action) if !matches!(action, ImageAction::Retry))
            {
                let center = transform.affine().translation;
                assert!(node.size().min_element() > 0.0);
                assert!(
                    center.x >= 20.0 && center.x <= 980.0 && center.y >= 20.0 && center.y <= 600.0,
                    "Viewer control outside compact window: {center:?}"
                );
            }
        }
    }
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Next));
    for _ in 0..30 {
        frame(&mut app);
    }
    assert!(app.world().resource::<ImageViewer>().error.is_some());
    capture_when(&mut app, &target, &output.join("06-error.png"), |_| true);
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
    }
    let tabs: Vec<_> = app
        .world()
        .resource::<Editor>()
        .image_tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    for id in tabs {
        app.world_mut().write_message(Action::CloseImageTab(id));
    }
    for _ in 0..5 {
        frame(&mut app);
    }
    let editor = app.world().resource::<Editor>();
    assert!(editor.drawer && editor.viewing_image.is_none());
    assert_eq!(editor.view, scene_view);
    assert_eq!(editor.edits, edits);
    let picture = app.world().resource::<ViewerUi>().picture;
    assert_eq!(
        app.world().get::<ImageNode>(picture).unwrap().image,
        Handle::default()
    );
    println!("Image viewer GPU artifacts: {}", output.display());
}

fn active_path(app: &App) -> PathBuf {
    let editor = app.world().resource::<Editor>();
    editor
        .image_tab(editor.viewing_image.unwrap())
        .unwrap()
        .path
        .clone()
}

fn close_images(app: &mut App) {
    let tabs: Vec<_> = app
        .world()
        .resource::<Editor>()
        .image_tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    for id in tabs {
        app.world_mut().write_message(Action::CloseImageTab(id));
    }
    app.update();
}

fn click_action(app: &mut App, window: Entity, wanted: impl Fn(Action) -> bool) {
    click_control(app, window, wanted, MouseButton::Left);
}

fn click_control(
    app: &mut App,
    window: Entity,
    wanted: impl Fn(Action) -> bool,
    button: MouseButton,
) {
    use crate::editor::selection_outline::render_tests::frame;
    use bevy::{
        camera::RenderTarget,
        input::{ButtonState, mouse::MouseButtonInput},
        picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
        ui::IsDefaultUiCamera,
    };
    let point = {
        let world = app.world_mut();
        let mut controls = world.query::<(&Control, &ComputedNode, &UiGlobalTransform)>();
        controls
            .iter(world)
            .find_map(|(control, node, transform)| {
                (wanted(control.action) && !node.is_empty())
                    .then_some(transform.affine().translation)
            })
            .expect("Missing document tab control")
    };
    let target = {
        let world = app.world_mut();
        world
            .query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>()
            .single(world)
            .unwrap()
            .normalize(Some(window))
            .unwrap()
    };
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.world_mut().write_message(PointerInput {
        pointer_id: PointerId::Mouse,
        location: Location {
            target: target.clone(),
            position: point,
        },
        action: PointerAction::Move { delta: Vec2::ZERO },
    });
    frame(app);
    let pointer_button = if button == MouseButton::Right {
        PointerButton::Secondary
    } else {
        PointerButton::Primary
    };
    for (state, action) in [
        (ButtonState::Pressed, PointerAction::Press(pointer_button)),
        (
            ButtonState::Released,
            PointerAction::Release(pointer_button),
        ),
    ] {
        app.world_mut().write_message(MouseButtonInput {
            window,
            button,
            state,
        });
        app.world_mut().write_message(PointerInput {
            pointer_id: PointerId::Mouse,
            location: Location {
                target: target.clone(),
                position: point,
            },
            action,
        });
        frame(app);
        frame(app);
    }
}

#[test]
fn image_tabs_restore_zoom_pan_and_identity_after_scene_switch_and_browser_rescan() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let first = file(&app, "a-landscape.SVG");
    app.world_mut().write_message(Action::ViewImage(first));
    wait_for_image(&mut app);
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::ActualSize));
    app.update();
    app.world_mut().resource_mut::<ImageViewer>().pan = Vec2::new(100.0, 50.0);
    app.world_mut().write_message(Action::Open(0));
    app.update();
    app.update();
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
    assert_eq!(app.world().resource::<Editor>().image_tabs.len(), 1);
    app.world_mut().write_message(Action::OpenImageTab(id));
    wait_for_image(&mut app);
    assert!(app.world().resource::<ImageViewer>().is_actual_size());
    assert_eq!(
        app.world().resource::<ImageViewer>().pan,
        Vec2::new(100.0, 50.0)
    );
    image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]))
        .save(fixture.0.join("0-new.png"))
        .unwrap();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            browser.refresh(&mut editor.project);
        });
    app.update();
    assert_eq!(app.world().resource::<Editor>().viewing_image, Some(id));
    assert_eq!(active_path(&app), PathBuf::from("a-landscape.SVG"));
    let remapped = file(&app, "a-landscape.SVG");
    app.world_mut().write_message(Action::ViewImage(remapped));
    app.update();
    assert_eq!(app.world().resource::<Editor>().image_tabs.len(), 1);
    app.world_mut().write_message(Action::CloseImageTab(id));
    app.update();
    assert!(app.world().resource::<Editor>().has_scene());
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
}

#[test]
fn closing_the_last_image_tab_returns_to_the_model_that_opened_it() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.project.catalog.push(crate::project::CatalogItem {
            asset: "model.glb".into(),
            label: "Model".into(),
            category: String::new(),
            solid: false,
            preview_direction: [1.0; 3],
        });
        editor.open_model_tab(0);
    }
    let first = file(&app, "a-landscape.SVG");
    app.world_mut().write_message(Action::ViewImage(first));
    wait_for_image(&mut app);
    assert!(app.world().resource::<Editor>().viewing_model.is_none());
    close_images(&mut app);
    assert_eq!(app.world().resource::<Editor>().viewing_model, Some(0));
}

fn wait_for_edit(app: &mut App) {
    for _ in 0..600 {
        app.update();
        let viewer = app.world().resource::<ImageViewer>();
        if viewer.edit.document(viewer.tab).is_some() {
            return;
        }
        assert!(viewer.edit.error.is_none(), "{:?}", viewer.edit.error);
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("PhotoCraft worker did not finish");
}

#[test]
fn editing_creates_native_sidecar_and_unsaved_tab_close_can_cancel_or_save() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let image = file(&app, "b-transparent.png");
    let original = std::fs::read(fixture.0.join("b-transparent.png")).unwrap();
    app.world_mut().write_message(Action::ViewImage(image));
    wait_for_image(&mut app);
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    assert!(!app.world().resource::<ImageViewer>().edit.editing(Some(id)));
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    assert!(fixture.0.join("b-transparent.pcraft").is_file());
    assert_eq!(
        std::fs::read(fixture.0.join("b-transparent.png")).unwrap(),
        original
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::AddLayer));
    app.update();
    assert!(
        app.world()
            .resource::<Editor>()
            .image_tab(id)
            .unwrap()
            .dirty
    );
    app.world_mut().write_message(Action::CloseImageTab(id));
    app.update();
    assert_eq!(
        app.world().resource::<Editor>().pending,
        Some(crate::editor::PendingAction::CloseImageTab(id))
    );
    app.world_mut().write_message(Action::CancelPending);
    app.update();
    assert_eq!(app.world().resource::<Editor>().viewing_image, Some(id));
    assert!(app.world().resource::<Editor>().pending.is_none());
    app.world_mut().write_message(Action::CloseImageTab(id));
    app.update();
    app.world_mut().write_message(Action::SaveAndContinue);
    for _ in 0..600 {
        app.update();
        if app.world().resource::<Editor>().image_tab(id).is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.world().resource::<Editor>().image_tab(id).is_none());
    let native = photocraft_format::load_path(&fixture.0.join("b-transparent.pcraft")).unwrap();
    assert_eq!(native.layers.len(), 2);
    assert!(app.world().resource::<Editor>().pending.is_none());
}

#[test]
fn editor_exit_checks_unsaved_images_and_discard_keeps_the_saved_files() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let index = file(&app, "b-transparent.png");
    let source = fixture.0.join("b-transparent.png");
    let before = std::fs::read(&source).unwrap();
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::AddLayer));
    app.update();
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    app.world_mut().resource_mut::<Editor>().request_exit();
    assert_eq!(
        app.world().resource::<Editor>().pending,
        Some(crate::editor::PendingAction::CloseImageEditor(id))
    );
    app.world_mut().write_message(Action::CancelPending);
    app.update();
    assert!(!app.world().resource::<Editor>().exit_requested);
    app.world_mut().resource_mut::<Editor>().request_exit();
    app.world_mut().write_message(Action::DiscardAndContinue);
    app.update();
    assert!(app.world().resource::<Editor>().exit_requested);
    assert_eq!(std::fs::read(&source).unwrap(), before);
    assert_eq!(
        photocraft_format::load_path(&source.with_extension("pcraft"))
            .unwrap()
            .layers
            .len(),
        1
    );
}

#[test]
fn focused_photo_slider_uses_arrow_keys_without_navigating_to_another_image() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let index = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    let id = app.world().resource::<Editor>().viewing_image;
    let slider = app
        .world_mut()
        .spawn(bevy::ui_widgets::Slider::default())
        .id();
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(slider, bevy::input_focus::FocusCause::Auto);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ArrowRight);
    app.update();
    assert_eq!(app.world().resource::<Editor>().viewing_image, id);
    assert_eq!(app.world().resource::<Editor>().image_tabs.len(), 1);
}

#[test]
#[ignore = "requires GPU adapter; exercises the embedded PhotoCraft tools, mode menu, layers and saving"]
fn gpu_photo_editor_paints_saves_and_reopens_native_layers() {
    use crate::editor::{
        photo_document::PhotoTool,
        selection_outline::render_tests::{capture_when, frame},
    };
    use bevy::{
        camera::RenderTarget,
        input::{ButtonState, mouse::MouseButtonInput},
        picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
        ui::IsDefaultUiCamera,
    };
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            browser.refresh(&mut editor.project);
            editor.drawer = false;
        });
    for _ in 0..45 {
        frame(&mut app);
    }
    let image = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(image));
    for _ in 0..25 {
        frame(&mut app);
    }
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Menu))
    });
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-editor-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("01-mode-menu.png"), |_| {
        true
    });
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Edit))
    });
    for _ in 0..90 {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
        {
            break;
        }
    }
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
    );
    for _ in 0..12 {
        frame(&mut app);
    }
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(edit::EditAction::DrawingTool)),
        MouseButton::Right,
    );
    assert!(app.world().resource::<ImageViewer>().edit.paint_menu);
    capture_when(
        &mut app,
        &target,
        &output.join("04-brush-pencil-menu.png"),
        |_| true,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Tool(PhotoTool::Pencil))
        )
    });
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.tool,
        PhotoTool::Pencil
    );
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 1.0);
    assert!(!app.world().resource::<ImageViewer>().edit.paint_menu);
    assert!(
        !app.world()
            .resource::<Editor>()
            .image_tab(app.world().resource::<Editor>().viewing_image.unwrap())
            .unwrap()
            .dirty,
        "Choosing a tool must not paint through the menu"
    );
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(edit::EditAction::DrawingTool)),
        MouseButton::Right,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Tool(PhotoTool::Brush))
        )
    });
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 24.0);
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::AddLayer))
    });
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Color([
            1.0, 0.1, 0.1, 1.0,
        ])));
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Tool(PhotoTool::Brush)));
    frame(&mut app);
    let point = {
        let world = app.world_mut();
        let mut query =
            world.query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>();
        let (node, transform) = query.single(world).unwrap();
        assert!(
            node.size().x > 500.0 && node.size().y > 400.0,
            "Editing canvas must fill its workspace: {:?}",
            node.size()
        );
        transform.affine().translation
    };
    let pointer_target = {
        let world = app.world_mut();
        world
            .query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>()
            .single(world)
            .unwrap()
            .normalize(Some(window))
            .unwrap()
    };
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.world_mut().write_message(PointerInput {
        pointer_id: PointerId::Mouse,
        location: Location {
            target: pointer_target.clone(),
            position: point,
        },
        action: PointerAction::Move { delta: Vec2::ZERO },
    });
    frame(&mut app);
    for (state, action) in [
        (
            ButtonState::Pressed,
            PointerAction::Press(PointerButton::Primary),
        ),
        (
            ButtonState::Released,
            PointerAction::Release(PointerButton::Primary),
        ),
    ] {
        app.world_mut().write_message(MouseButtonInput {
            window,
            button: MouseButton::Left,
            state,
        });
        app.world_mut().write_message(PointerInput {
            pointer_id: PointerId::Mouse,
            location: Location {
                target: pointer_target.clone(),
                position: point,
            },
            action,
        });
        frame(&mut app);
    }
    for _ in 0..8 {
        frame(&mut app);
    }
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    let doc = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap();
    assert_eq!(doc.document.layers.len(), 2);
    assert!(doc.document.layers[1].surface().unwrap().rgba(150, 450)[0] > 0.9);
    capture_when(&mut app, &target, &output.join("02-editing.png"), |image| {
        image
            .pixels()
            .filter(|p| p[0] as u16 > p[1] as u16 + 35 && p[0] as u16 > p[2] as u16 + 35)
            .count()
            > 4000
    });
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Save))
    });
    for _ in 0..120 {
        frame(&mut app);
        if !app
            .world()
            .resource::<Editor>()
            .image_tab(id)
            .unwrap()
            .dirty
        {
            break;
        }
    }
    assert!(
        !app.world()
            .resource::<Editor>()
            .image_tab(id)
            .unwrap()
            .dirty
    );
    let pixels = image::open(fixture.0.join("b-transparent.png"))
        .unwrap()
        .into_rgba8();
    assert!(pixels.get_pixel(150, 450)[0] > 240);
    assert_eq!(
        photocraft_format::load_path(&fixture.0.join("b-transparent.pcraft"))
            .unwrap()
            .layers
            .len(),
        2
    );
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(1000.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: 1000,
            height: 620,
            depth_or_array_layers: 1,
        });
    for _ in 0..12 {
        frame(&mut app);
    }
    capture_when(&mut app, &target, &output.join("03-compact.png"), |_| true);
    app.world_mut().write_message(Action::CloseImageTab(id));
    frame(&mut app);
    let reopened = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(reopened));
    for _ in 0..12 {
        frame(&mut app);
    }
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..90 {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(app.world().resource::<Editor>().viewing_image)
            .is_some()
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(app.world().resource::<Editor>().viewing_image)
            .unwrap()
            .document
            .layers
            .len(),
        2
    );
    println!("PhotoCraft editor screenshots: {}", output.display());
}
