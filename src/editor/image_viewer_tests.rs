use super::*;
use crate::editor::{RoomEdits, browser::Browser, tests::lighting_ui_render};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[path = "image_brush_size_tests.rs"]
mod brush_sizing;
#[path = "image_color_drop_tests.rs"]
mod color_dropping;
#[path = "image_cursor_restore_tests.rs"]
mod cursor_restoration;
#[path = "image_edge_paint_tests.rs"]
mod edge_painting;
#[path = "image_eyedropper_tests.rs"]
mod eyedropper;
#[path = "image_layers_tests.rs"]
mod image_layers;
#[path = "image_layer_drag_tests.rs"]
mod layer_drag;
#[path = "image_layer_rename_tests.rs"]
mod layer_rename;
#[path = "image_layer_ux_tests.rs"]
mod layer_ux;
#[path = "image_paint_tests.rs"]
mod painting;
#[path = "image_panel_resize_tests.rs"]
mod panel_sizing;
#[path = "image_pen_tests.rs"]
mod pen_pressure;
#[path = "image_pencil_tests.rs"]
mod pencil;
#[path = "image_canvas_tests.rs"]
mod pixel_canvas;
#[path = "image_eraser_tests.rs"]
mod pixel_eraser;
#[path = "image_selection_tests.rs"]
mod selections;
#[path = "image_color_tests.rs"]
mod tool_colors;

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
            .add_message::<bevy::window::WindowEvent>()
            .add_plugins(paint_input::PointerPlugin)
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
fn zoom_keeps_the_point_under_the_cursor_even_when_the_image_fits() {
    let mut viewer = ImageViewer {
        dimensions: Some(UVec2::new(2000, 1000)),
        zoom: Zoom::Scale(1.0),
        ..default()
    };
    let anchor = Vec2::new(100.0, 50.0);
    viewer.zoom_at(2.0, anchor, Vec2::new(1000.0, 500.0));
    assert_eq!(viewer.pan, -anchor);
    assert_eq!(viewer.zoom, Zoom::Scale(2.0));
    viewer.dimensions = Some(UVec2::new(24, 12));
    viewer.pan = Vec2::new(135.0, -80.0);
    viewer.zoom = Zoom::Fit;
    viewer.zoom_at(0.5, anchor, Vec2::new(700.0, 900.0));
    assert_eq!(viewer.pan, Vec2::new(117.5, -15.0));
    assert_eq!(viewer.zoom, Zoom::Scale(0.5));
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
fn small_image_pans_in_view_and_edit_modes_without_modifying_the_document() {
    use crate::editor::photo_document::PhotoTool;
    use bevy::window::SystemCursorIcon;

    let fixture = Fixture::new();
    let source = fixture.0.join("pan.png");
    image::RgbaImage::from_pixel(32, 16, image::Rgba([225, 110, 65, 255]))
        .save(&source)
        .unwrap();
    let original = std::fs::read(&source).unwrap();
    let mut app = fixture.app();
    app.add_systems(Update, tool_cursor::cursor.after(update));
    let image = file(&app, "pan.png");
    app.world_mut().write_message(Action::ViewImage(image));
    wait_for_image(&mut app);
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let start = Vec2::new(640.0, 360.0);
    let delta = Vec2::new(145.0, -85.0);
    for (editing, tool, space, button, zoom) in [
        (
            false,
            PhotoTool::Hand,
            false,
            MouseButton::Left,
            ImageAction::Fit,
        ),
        (
            false,
            PhotoTool::Hand,
            false,
            MouseButton::Left,
            ImageAction::ActualSize,
        ),
        (
            true,
            PhotoTool::Hand,
            false,
            MouseButton::Left,
            ImageAction::Fit,
        ),
        (
            true,
            PhotoTool::Brush,
            true,
            MouseButton::Left,
            ImageAction::ActualSize,
        ),
        (
            true,
            PhotoTool::Eraser,
            false,
            MouseButton::Middle,
            ImageAction::Fit,
        ),
    ] {
        if editing
            && !app
                .world()
                .resource::<ImageViewer>()
                .edit
                .editing(app.world().resource::<Editor>().viewing_image)
        {
            app.world_mut()
                .write_message(Action::ImageEdit(edit::EditAction::Edit));
            wait_for_edit(&mut app);
        }
        if editing {
            app.world_mut()
                .write_message(Action::ImageEdit(edit::EditAction::Tool(tool)));
        }
        app.world_mut().write_message(Action::ImageView(zoom));
        app.update();
        if space {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Space);
        }
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(start));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(button);
        app.update();
        assert_eq!(
            app.world().get::<CursorIcon>(window),
            Some(&CursorIcon::from(SystemCursorIcon::Grabbing))
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(start + delta));
        app.update();
        assert_eq!(
            app.world().resource::<ImageViewer>().pan,
            delta,
            "Small image must move with {tool:?}, space={space}, {button:?}"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(button);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert_eq!(
            app.world().resource::<ImageViewer>().pan,
            delta,
            "Releasing the pointer must not recenter a fitted image"
        );
        assert!(app.world().resource::<ImageViewer>().gesture.is_none());
        if !editing || tool == PhotoTool::Hand || space {
            assert_eq!(
                app.world().get::<CursorIcon>(window),
                Some(&CursorIcon::from(SystemCursorIcon::Grab))
            );
        } else {
            assert!(matches!(
                app.world().get::<CursorIcon>(window),
                Some(CursorIcon::Custom(_))
            ));
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        // Resizing the viewport must not erase the user's pan offset either.
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(1000.0, 620.0);
        app.update();
        assert_eq!(app.world().resource::<ImageViewer>().pan, delta);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(1280.0, 760.0);
        app.update();
        if editing {
            let viewer = app.world().resource::<ImageViewer>();
            let doc = viewer.edit.document(viewer.tab).unwrap();
            assert!(
                !doc.dirty() && !doc.can_undo(),
                "Panning must not paint or create history"
            );
        }
    }
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::Fit));
    app.update();
    assert_eq!(app.world().resource::<ImageViewer>().pan, Vec2::ZERO);
    assert_eq!(std::fs::read(source).unwrap(), original);
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
        (pane_top - super::super::IMAGE_TOP).abs() < 1.0,
        "Image canvas must start directly below document tabs, without the scene hint row: {pane_top}"
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
    let layout = |app: &App| {
        let ui = app.world().resource::<ViewerUi>();
        [ui.overlay, ui.canvas, ui.picture].map(|entity| {
            let node = app.world().get::<ComputedNode>(entity).unwrap();
            let transform = app.world().get::<UiGlobalTransform>(entity).unwrap();
            Rect::from_center_size(transform.affine().translation, node.size())
        })
    };
    let original_layout = layout(&app);
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
        assert_eq!(
            layout(&app),
            original_layout,
            "The file manager must not resize or recenter the image workspace"
        );
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
    let interface = app.world().resource::<Interface>();
    assert_eq!(
        app.world().get::<Node>(interface.body).unwrap().top,
        px(super::super::TOP)
    );
    assert_eq!(
        app.world().get::<Node>(interface.toolbar).unwrap().height,
        px(super::super::TOP)
    );
    assert_eq!(
        app.world().get::<Node>(interface.tool_bar).unwrap().display,
        Display::Flex
    );
    let editor = app.world().resource::<Editor>();
    assert!(editor.drawer && editor.viewing_image.is_none());
    assert_eq!(editor.view, scene_view);
    assert_eq!(editor.edits, edits);
    let picture = app.world().resource::<ViewerUi>().picture;
    assert_eq!(
        app.world()
            .resource::<Assets<CanvasMaterial>>()
            .get(
                &app.world()
                    .get::<MaterialNode<CanvasMaterial>>(picture)
                    .unwrap()
                    .0
            )
            .unwrap()
            .image,
        None
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
    click_control_offset(app, window, wanted, button, Vec2::ZERO);
}

fn click_control_offset(
    app: &mut App,
    window: Entity,
    wanted: impl Fn(Action) -> bool,
    button: MouseButton,
    relative_offset: Vec2,
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
                (wanted(control.action) && !node.is_empty()).then_some(
                    (transform.affine().translation + node.size() * relative_offset)
                        * node.inverse_scale_factor(),
                )
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
            uid: None,
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
    // Put real document pixels underneath the floating selector. Opening,
    // dismissing and using it must neither paint nor start canvas panning.
    let mode_center = {
        let world = app.world_mut();
        let (_, node, transform) = world
            .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(control, node, _)| {
                matches!(control.action, Action::ImageEdit(edit::EditAction::Menu))
                    && !node.is_empty()
            })
            .unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    };
    let old_view = {
        let world = app.world_mut();
        let (_, node, transform) = world
            .query_filtered::<(Entity, &ComputedNode, &UiGlobalTransform), With<ImageCanvas>>()
            .single(world)
            .unwrap();
        let canvas_center = transform.affine().translation * node.inverse_scale_factor();
        let mut viewer = world.resource_mut::<ImageViewer>();
        let old = (viewer.zoom, viewer.pan);
        viewer.zoom = Zoom::Scale(1.0);
        viewer.pan = mode_center - canvas_center;
        old
    };
    let id = app.world().resource::<Editor>().viewing_image;
    let revision = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(id)
        .unwrap()
        .revision;
    let menu_pan = app.world().resource::<ImageViewer>().pan;
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Menu))
    });
    assert!(app.world().resource::<ImageViewer>().edit.menu);
    capture_when(
        &mut app,
        &target,
        &output.join("01a-edit-mode-menu.png"),
        |_| true,
    );
    lighting_ui_render::move_pointer(&mut app, window, mode_center + Vec2::Y * 160.0);
    lighting_ui_render::mouse(&mut app, window, ButtonState::Pressed);
    lighting_ui_render::mouse(&mut app, window, ButtonState::Released);
    assert!(!app.world().resource::<ImageViewer>().edit.menu);
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(id)
            .unwrap()
            .revision,
        revision
    );
    assert_eq!(app.world().resource::<ImageViewer>().pan, menu_pan);
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Menu))
    });
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::View))
    });
    assert!(!app.world().resource::<ImageViewer>().edit.editing(id));
    assert_eq!(app.world().resource::<ImageViewer>().pan, menu_pan);
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Menu))
    });
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Edit))
    });
    {
        let mut viewer = app.world_mut().resource_mut::<ImageViewer>();
        assert!(viewer.edit.editing(id));
        assert_eq!(viewer.edit.document(id).unwrap().revision, revision);
        viewer.zoom = old_view.0;
        viewer.pan = old_view.1;
    }
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(edit::EditAction::DrawingTool)),
        MouseButton::Right,
    );
    assert!(app.world().resource::<ImageViewer>().edit.paint_menu);
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(edit::EditAction::DrawingTool)),
        MouseButton::Right,
    );
    assert!(!app.world().resource::<ImageViewer>().edit.paint_menu);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(700.0, 400.0)));
    for _ in 0..80 {
        frame(&mut app);
    }
    capture_when(
        &mut app,
        &target,
        &output.join("05-selected-brush.png"),
        |_| true,
    );
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
    let (old_zoom, old_color) = {
        let mut viewer = app.world_mut().resource_mut::<ImageViewer>();
        let previous = (viewer.zoom, viewer.edit.color());
        viewer.zoom = Zoom::Scale(32.0);
        previous
    };
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Color([
            0.0, 0.0, 0.0, 1.0,
        ])));
    for _ in 0..4 {
        frame(&mut app);
    }
    let workspace = |app: &App| {
        let ui = app.world().resource::<ViewerUi>();
        [ui.canvas, ui.picture].map(|entity| {
            let node = app.world().get::<ComputedNode>(entity).unwrap();
            let transform = app.world().get::<UiGlobalTransform>(entity).unwrap();
            Rect::from_center_size(transform.affine().translation, node.size())
        })
    };
    let fixed = workspace(&app);
    let handle = app.world().resource::<ImageViewer>().image.clone().unwrap();
    let original = app
        .world()
        .resource::<Assets<Image>>()
        .get(&handle)
        .unwrap()
        .data
        .clone()
        .unwrap();
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
        assert_eq!(
            workspace(&app),
            fixed,
            "The file manager must overlay the editor throughout its animation"
        );
    }
    for height in [260.0, 500.0] {
        app.world_mut()
            .resource_mut::<DrawerSize>()
            .preferred_height = Some(height);
        for _ in 0..4 {
            frame(&mut app);
            assert_eq!(workspace(&app), fixed);
        }
    }
    let point = Vec2::new(
        fixed[0].center().x,
        900.0 - super::super::FOOT - app.world().resource::<DrawerMotion>().visible_height + 64.0,
    );
    for button in [MouseButton::Left, MouseButton::Middle] {
        for (position, state) in [
            (point, ButtonState::Pressed),
            (point + Vec2::new(30.0, 5.0), ButtonState::Released),
        ] {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(position));
            app.world_mut()
                .write_message(bevy::window::WindowEvent::CursorMoved(
                    bevy::window::CursorMoved {
                        window,
                        position,
                        delta: None,
                    },
                ));
            let event = MouseButtonInput {
                window,
                button,
                state,
            };
            app.world_mut()
                .write_message(bevy::window::WindowEvent::MouseButtonInput(event));
            app.world_mut().write_message(event);
            frame(&mut app);
        }
    }
    app.world_mut()
        .write_message(bevy::input::mouse::MouseWheel {
            window,
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 2.0,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
    frame(&mut app);
    assert_eq!(
        app.world().resource::<ImageViewer>().zoom,
        Zoom::Scale(32.0)
    );
    assert_eq!(app.world().resource::<ImageViewer>().pan, Vec2::ZERO);
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .dirty(),
        "Clicks on the file manager must not paint through it"
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &original
    );
    capture_when(
        &mut app,
        &target,
        &output.join("06-file-manager-overlay.png"),
        |_| true,
    );
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
        assert_eq!(workspace(&app), fixed);
    }
    app.world_mut().resource_mut::<ImageViewer>().zoom = old_zoom;
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Color(old_color)));
    for _ in 0..4 {
        frame(&mut app);
    }
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

#[test]
#[ignore = "requires GPU adapter; verifies brush cursor contrast, zoom, DPI and cursor restoration"]
fn gpu_photo_cursor_is_visible_on_light_dark_and_grey_and_tracks_zoom() {
    use crate::editor::{
        photo_document::PhotoTool,
        selection_outline::render_tests::{capture_when, frame},
    };
    use bevy::window::{CursorIcon, CustomCursor, SystemCursorIcon};

    let fixture = Fixture::new();
    image::RgbaImage::from_fn(512, 512, |x, _| {
        let value = if x < 171 {
            255
        } else if x < 341 {
            0
        } else {
            128
        };
        image::Rgba([value, value, value, 255])
    })
    .save(fixture.0.join("cursor.png"))
    .unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    for _ in 0..30 {
        frame(&mut app);
    }
    let index = file(&app, "cursor.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..30 {
        frame(&mut app);
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        let world = app.world();
        if world
            .resource::<ImageViewer>()
            .edit
            .editing(world.resource::<Editor>().viewing_image)
        {
            break;
        }
    }
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::ActualSize));
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Tool(PhotoTool::Brush)));
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Size(32.0)));
    for _ in 0..8 {
        frame(&mut app);
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-cursor-qa");
    std::fs::create_dir_all(&output).unwrap();
    let previous = CursorIcon::from(SystemCursorIcon::Default);
    app.world_mut().entity_mut(window).insert(previous.clone());
    let center = {
        let world = app.world_mut();
        let mut query =
            world.query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>();
        let (node, transform) = query.single(world).unwrap();
        transform.affine().translation * node.inverse_scale_factor()
    };
    let move_to = |app: &mut App, position| {
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(position));
        for _ in 0..4 {
            frame(app);
        }
    };
    let native = |app: &App| {
        let CursorIcon::Custom(CustomCursor::Image(image)) =
            app.world().get::<CursorIcon>(window).unwrap()
        else {
            panic!("The visible tool needs an OS-owned bitmap cursor");
        };
        image.clone()
    };
    for (name, offset, background) in [
        ("white", -128.0, 255i16),
        ("black", 0.0, 0),
        ("grey", 128.0, 128),
    ] {
        let point = center + Vec2::new(offset, 0.0);
        move_to(&mut app, point);
        let cursor = native(&app);
        let image = app
            .world()
            .resource::<Assets<Image>>()
            .get(&cursor.handle)
            .unwrap();
        assert_eq!(image.width(), 39);
        assert_eq!(cursor.hotspot, (19, 19));
        let screenshot = capture_when(
            &mut app,
            &target,
            &output.join(format!("{name}.png")),
            |_| true,
        );
        // Sample the perimeter away from the centre marker. A white-only ring fails on white.
        let mut visible = 0;
        for y in -19..=19 {
            for x in -19..=19 {
                let radius = (x as f32).hypot(y as f32);
                if (14.0..=19.0).contains(&radius) {
                    let pixel = screenshot
                        .get_pixel((point.x as i32 + x) as u32, (point.y as i32 + y) as u32);
                    visible += usize::from((i16::from(pixel[0]) - background).abs() > 45);
                }
            }
        }
        assert!(
            visible > 45,
            "Brush outline disappears on {name}: {visible}"
        );
    }
    let old = native(&app);
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(2.0);
    for _ in 0..4 {
        frame(&mut app);
    }
    let enlarged = native(&app);
    assert_ne!(old.handle, enlarged.handle);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&enlarged.handle)
            .unwrap()
            .width(),
        71
    );
    let frames = app.world().resource::<Assets<Image>>().len();
    move_to(&mut app, center + Vec2::new(20.0, 20.0));
    assert_eq!(
        native(&app),
        enlarged,
        "Pointer motion must reuse the OS cursor bitmap"
    );
    assert_eq!(app.world().resource::<Assets<Image>>().len(), frames);

    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
    {
        let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
        window.resolution.set_scale_factor_override(Some(2.0));
        window.resolution.set(1280.0, 900.0);
    }
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(bevy::render::render_resource::Extent3d {
            width: 2560,
            height: 1800,
            depth_or_array_layers: 1,
        });
    {
        let world = app.world_mut();
        let mut cameras = world
            .query_filtered::<&mut bevy::camera::RenderTarget, With<bevy::ui::IsDefaultUiCamera>>();
        *cameras.single_mut(world).unwrap() =
            bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {
                handle: target.clone(),
                scale_factor: 2.0,
            });
    }
    for _ in 0..6 {
        frame(&mut app);
    }
    let rect = {
        let world = app.world_mut();
        let mut query =
            world.query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>();
        let (node, transform) = query.single(world).unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    move_to(&mut app, rect.center());
    let dense = native(&app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&dense.handle)
            .unwrap()
            .width(),
        41,
        "At 1:1 the tip remains 32 physical pixels across; DPI changes the contrast halo"
    );

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Space);
    frame(&mut app);
    assert!(matches!(
        app.world().get::<CursorIcon>(window),
        Some(CursorIcon::System(SystemCursorIcon::Grab))
    ));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::Space);
    frame(&mut app);
    assert!(matches!(
        app.world().get::<CursorIcon>(window),
        Some(CursorIcon::Custom(_))
    ));
    app.world_mut()
        .resource_mut::<ImageViewer>()
        .edit
        .paint_menu = true;
    frame(&mut app);
    assert_eq!(app.world().get::<CursorIcon>(window), Some(&previous));
    app.world_mut()
        .resource_mut::<ImageViewer>()
        .edit
        .paint_menu = false;
    move_to(&mut app, Vec2::new(8.0, 8.0));
    assert_eq!(app.world().get::<CursorIcon>(window), Some(&previous));
    let image_tab = app.world().resource::<Editor>().viewing_image.unwrap();
    move_to(&mut app, rect.center());
    native(&app);
    app.world_mut().write_message(Action::Open(0));
    frame(&mut app);
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
    assert_eq!(app.world().get::<CursorIcon>(window), Some(&previous));
    assert!(app.world().resource::<ImageViewer>().brush_cursor.is_none());
    app.world_mut()
        .write_message(Action::OpenImageTab(image_tab));
    for _ in 0..6 {
        frame(&mut app);
    }
    native(&app);
    app.world_mut()
        .write_message(Action::CloseImageTab(image_tab));
    frame(&mut app);
    assert!(app.world().resource::<Editor>().viewing_image.is_none());
    assert_eq!(app.world().get::<CursorIcon>(window), Some(&previous));
    assert!(app.world().resource::<ImageViewer>().brush_cursor.is_none());
    println!("Painting cursor captures: {}", output.display());
}

fn model_target_pixels(app: &mut App, image: &Handle<Image>) -> Vec<u8> {
    use bevy::render::gpu_readback::{Readback, ReadbackComplete};
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    app.world_mut()
        .spawn(Readback::texture(image.clone()))
        .observe(
            move |mut event: On<ReadbackComplete>, mut commands: Commands| {
                let _ = send.try_send(std::mem::take(&mut event.event_mut().data));
                commands.entity(event.entity).try_despawn();
            },
        );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        crate::editor::selection_outline::render_tests::frame(app);
        if let Ok(pixels) = receive.try_recv() {
            return pixels;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Missing live model texture"
        );
    }
}

#[test]
#[ignore = "requires GPU adapter; drags GLB and FBX onto the PhotoCraft canvas"]
fn gpu_model_cards_add_transparent_reference_layers_and_save() {
    use crate::editor::{
        image_reference::ReferenceCamera,
        model_document::{MaterialSlot, Object, glb::Glb},
        model_primitives::Primitive,
        selection_outline::render_tests::{capture_when, frame},
        ui::drawer::{AssetDrag, DropTarget, FileCard},
    };
    use bevy::input::ButtonState;
    use lighting_ui_render::{mouse, move_pointer};

    #[derive(Resource)]
    struct SimulateEmptyCapture(bool);

    let fixture = Fixture::new();
    let models = fixture.0.join("models");
    std::fs::create_dir(&models).unwrap();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for directory in ["scenes", "levels"] {
        let destination = fixture.0.join(directory);
        std::fs::create_dir(&destination).unwrap();
        for file in std::fs::read_dir(fixtures.join("legacy/assets").join(directory)).unwrap() {
            let file = file.unwrap();
            std::fs::copy(file.path(), destination.join(file.file_name())).unwrap();
        }
    }
    let primitive = Primitive::Cube;
    let cube = Object {
        id: 0,
        source_node: None,
        name: "Куб".into(),
        geometry: primitive.geometry(&primitive.defaults()).unwrap().into(),
        transform: Transform::IDENTITY,
        materials: vec![MaterialSlot {
            bake_fixes: Default::default(),
            authored: None,
            material: default(),
            source: serde_json::json!({"material": 0}),
            paint: None,
        }],
        hidden: false,
    };
    std::fs::write(models.join("cube.glb"), Glb::new_asset(&[cube]).unwrap()).unwrap();
    std::fs::copy(
        fixtures.join("fbx/embedded/props_embedded.fbx"),
        models.join("props.fbx"),
    )
    .unwrap();
    let source = fixture.0.join("canvas.png");
    image::RgbaImage::new(600, 480).save(&source).unwrap();
    let (mut app, window, output) =
        lighting_ui_render::fixture_project_with_pipeline_compilation(fixture.project(), false);
    app.insert_resource(SimulateEmptyCapture(true)).add_systems(
        PostUpdate,
        |simulate: Res<SimulateEmptyCapture>,
         mut cameras: Query<&mut Camera, With<ReferenceCamera>>| {
            if simulate.0 {
                for mut camera in &mut cameras {
                    camera.is_active = false;
                }
            }
        },
    );
    let image = file(&app, "canvas.png");
    app.world_mut().write_message(Action::ViewImage(image));
    frame(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    let tab = app.world().resource::<Editor>().viewing_image.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .editing(Some(tab))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "PhotoCraft did not open"
        );
    }
    let folder = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == std::path::Path::new("models"))
        .unwrap();
    app.world_mut().write_message(Action::OpenBrowser);
    app.world_mut().write_message(Action::Browse(folder));
    for _ in 0..28 {
        frame(&mut app);
    }

    for (index, name) in ["cube.glb", "props.fbx"].into_iter().enumerate() {
        app.world_mut().resource_mut::<SimulateEmptyCapture>().0 = index == 0;
        let model = app
            .world()
            .resource::<Editor>()
            .project
            .catalog
            .iter()
            .position(|item| item.asset == format!("models/{name}"))
            .unwrap();
        let card = {
            let world = app.world_mut();
            world
                .query::<(&FileCard, &ComputedNode, &UiGlobalTransform)>()
                .iter(world)
                .find_map(|(card, node, transform)| {
                    (card.0 == browser::Entry::Model(model) && !node.is_empty())
                        .then_some(transform.translation * node.inverse_scale_factor())
                })
                .unwrap()
        };
        let drop_point = {
            let world = app.world_mut();
            let (node, transform) = world
                .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>()
                .single(world)
                .unwrap();
            let canvas_center = transform.translation * node.inverse_scale_factor();
            canvas_center + Vec2::new(index as f32 * 280.0 - 140.0, -75.0)
        };
        move_pointer(&mut app, window, card);
        mouse(&mut app, window, ButtonState::Pressed);
        move_pointer(&mut app, window, drop_point);
        assert!(
            matches!(app.world().resource::<AssetDrag>().target, Some(DropTarget::ModelReference { document, .. }) if document == tab)
        );
        mouse(&mut app, window, ButtonState::Released);

        let started = std::time::Instant::now();
        loop {
            frame(&mut app);
            let state = &app.world().resource::<ImageViewer>().edit;
            assert!(state.error.is_none(), "{:?}", state.error);
            let doc = state.document(Some(tab)).unwrap();
            if doc.document.layers.len() == index + 2 {
                let layer = doc.active_layer().unwrap();
                assert_eq!(layer.name, format!("Референс · {name}"));
                assert!(layer.locks.pixels && doc.movable());
                let photocraft_doc::LayerContent::Raster(surface) = &layer.content else {
                    panic!("Expected a raster reference");
                };
                let bounds = surface.content_bounds();
                let mut pixels = vec![[0; 4]; bounds.width() as usize * bounds.height() as usize];
                surface.read_rgba8_into(bounds, &mut pixels);
                assert!(
                    pixels.iter().any(|p| p[3] == 0),
                    "The silhouette must have no backdrop"
                );
                let pixels = doc.preview().pixels;
                assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
                assert!(
                    pixels
                        .chunks_exact(4)
                        .any(|p| p[3] == 255 && p[..3] != [0, 0, 0])
                );
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(40),
                "{}",
                app.world().resource::<Editor>().status
            );
        }
    }
    assert!(app.world().resource::<Editor>().edits.models.is_empty());
    for (action, count) in [(edit::EditAction::Undo, 2), (edit::EditAction::Redo, 3)] {
        app.world_mut().write_message(Action::ImageEdit(action));
        frame(&mut app);
        let doc = app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(tab))
            .unwrap();
        assert_eq!(doc.document.layers.len(), count);
    }
    // Real filesystem events must preserve the open PhotoCraft tab, authored layers and model identity.
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Save));
    frame(&mut app);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while app.world().resource::<ImageViewer>().edit.file_busy(tab) {
        frame(&mut app);
        assert!(std::time::Instant::now() < deadline);
    }
    let model_uid = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .model_reference(Some(tab))
        .unwrap()
        .uid
        .unwrap();
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.model_tool,
        Some(Tool::Move)
    );
    let old_bundle = source.with_extension("pcraft");
    assert!(old_bundle.exists());
    let renamed = fixture.0.join("nested/renamed.png");
    std::fs::rename(&source, &renamed).unwrap();
    std::fs::rename(models.join("props.fbx"), models.join("moved.fbx")).unwrap();
    let source = renamed;
    loop {
        frame(&mut app);
        let editor = app.world().resource::<Editor>();
        if editor
            .image_tab(tab)
            .is_some_and(|t| t.path == std::path::Path::new("nested/renamed.png"))
            && editor
                .project
                .catalog
                .iter()
                .any(|m| m.asset == "models/moved.fbx" && m.uid.as_deref() == Some(&model_uid))
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", editor.status);
    }
    assert!(!old_bundle.exists());
    let restored = crate::editor::photo_document::PhotoDocument::load(&source).unwrap();
    assert_eq!(restored.document.layers.len(), 3);
    app.world_mut().write_message(Action::Drawer);
    for _ in 0..25 {
        frame(&mut app);
    }
    // The cube fits inside the document. Its pixels should follow pointer samples
    // on the GPU without replacing the canvas texture or moving the second model.
    let select_layer = |app: &mut App, index| {
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::SelectLayer(index)));
        frame(app);
    };
    select_layer(&mut app, 1);
    let cube_canvas = {
        let world = app.world_mut();
        let (n, t) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>()
            .single(world)
            .unwrap();
        Rect::from_center_size(
            t.translation * n.inverse_scale_factor(),
            n.size() * n.inverse_scale_factor(),
        )
    };
    let center = edit::interface::model_handle_center(app.world());
    let origin = cube_canvas.min + center;
    move_pointer(&mut app, window, origin);
    mouse(&mut app, window, ButtonState::Pressed);
    let mut textures = None;
    let mut grabbed_texel: Option<Vec2> = None;
    for x in [3.0, 7.0, 13.0, 19.0] {
        move_pointer(&mut app, window, origin + Vec2::new(x, 0.0));
        let end = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let m = app
                .world()
                .resource::<Assets<CanvasMaterial>>()
                .get(&super::canvas_material::HANDLE)
                .unwrap();
            if m.transform_style.x == 1.0 {
                // The shader must sample the same point of the moving layer under
                // the cursor each frame, even at fractional image zoom.
                let viewer = app.world().resource::<ImageViewer>();
                let rect = viewer.image_rect(viewer.dimensions.unwrap(), cube_canvas);
                let uv = ((origin + Vec2::X * x - rect.min) / rect.size()).extend(1.0);
                let rows = m.transform_rows;
                let sample = Vec2::new(rows[0].truncate().dot(uv), rows[1].truncate().dot(uv))
                    / rows[2].truncate().dot(uv);
                if let Some(original) = grabbed_texel {
                    assert!(original.abs_diff_eq(sample, 1e-4), "Grabbed texel slipped");
                }
                grabbed_texel = Some(sample);
                let handles = (
                    m.transform_under.clone(),
                    m.transform_moving.clone(),
                    m.transform_above.clone(),
                );
                if let Some(previous) = &textures {
                    assert_eq!(&handles, previous);
                } else {
                    textures = Some(handles);
                }
                assert!(
                    edit::interface::model_handle_center(app.world())
                        .abs_diff_eq(center + Vec2::X * x, 0.1)
                );
                break;
            }
            assert!(
                textures.is_none() && std::time::Instant::now() < end,
                "Move preview dropped a pointer sample"
            );
            frame(&mut app);
        }
    }
    let end_center = edit::interface::model_handle_center(app.world());
    mouse(&mut app, window, ButtonState::Released);
    let end = std::time::Instant::now() + Duration::from_secs(15);
    assert!(
        app.world()
            .resource::<crate::editor::image_reference::ReferenceRender>()
            .pending_for(Some(tab)),
        "Test must sample the release-to-render handoff"
    );
    while app
        .world()
        .resource::<crate::editor::image_reference::ReferenceRender>()
        .pending_for(Some(tab))
    {
        assert!(
            edit::interface::model_handle_center(app.world()).abs_diff_eq(end_center, 0.1),
            "Handle jumped back after release"
        );
        assert_eq!(
            app.world()
                .resource::<Assets<CanvasMaterial>>()
                .get(&super::canvas_material::HANDLE)
                .unwrap()
                .transform_style
                .x,
            1.0
        );
        assert!(std::time::Instant::now() < end);
        frame(&mut app);
    }
    frame(&mut app);
    assert!(edit::interface::model_handle_center(app.world()).abs_diff_eq(end_center, 0.1));
    assert_eq!(
        app.world()
            .resource::<Assets<CanvasMaterial>>()
            .get(&super::canvas_material::HANDLE)
            .unwrap()
            .transform_style
            .x,
        0.0
    );
    // Commit a clipped layer, then pull it back into view. The cached moving
    // texture must include the geometry that was outside the saved document.
    let wait_model = |app: &mut App| {
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while app
            .world()
            .resource::<crate::editor::image_reference::ReferenceRender>()
            .pending_for(Some(tab))
        {
            frame(app);
            assert!(std::time::Instant::now() < deadline);
        }
        frame(app);
    };
    let viewer = app.world().resource::<ImageViewer>();
    let size = viewer.dimensions.unwrap();
    let image = viewer.image_rect(size, cube_canvas);
    let start = cube_canvas.min + end_center;
    let edge = Vec2::new(image.max.x - 22.0, start.y);
    move_pointer(&mut app, window, start);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, edge);
    mouse(&mut app, window, ButtonState::Released);
    wait_model(&mut app);
    let layer = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(tab))
        .unwrap()
        .active_layer()
        .unwrap();
    assert_eq!(layer.surface().unwrap().content_bounds().x1, size.x as i32);
    let center = cube_canvas.min + edit::interface::model_handle_center(app.world());
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center - Vec2::X * 80.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        frame(&mut app);
        let m = app
            .world()
            .resource::<Assets<CanvasMaterial>>()
            .get(&super::canvas_material::HANDLE)
            .unwrap();
        if m.transform_style.x == 1.0 {
            let moving = app
                .world()
                .resource::<Assets<Image>>()
                .get(m.transform_moving.as_ref().unwrap())
                .unwrap();
            let boundary = ((1.0 - m.transform_frame.x) / m.transform_frame.z
                * moving.width() as f32)
                .ceil()
                .max(0.0) as usize;
            assert!(boundary < moving.width() as usize);
            assert!(
                moving
                    .data
                    .as_ref()
                    .unwrap()
                    .chunks_exact(moving.width() as usize * 4)
                    .any(|row| row[boundary * 4..].chunks_exact(4).any(|p| p[3] > 128)),
                "Revealed model geometry was cropped"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Clipped model did not acquire a GPU preview"
        );
    }
    mouse(&mut app, window, ButtonState::Released);
    wait_model(&mut app);
    select_layer(&mut app, 2);
    let model_pose = |app: &App| {
        app.world()
            .resource::<ImageViewer>()
            .edit
            .model_reference(Some(tab))
            .unwrap()
            .pose
    };
    for tool in [Tool::Move, Tool::Rotate, Tool::Scale] {
        let before = model_pose(&app);
        click_action(&mut app, window, |action| {
            matches!(action,
            Action::ImageEdit(edit::EditAction::ModelTool(selected)) if selected == tool)
        });
        let canvas = {
            let world = app.world_mut();
            let (node, transform) = world
                .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>()
                .single(world)
                .unwrap();
            let center = transform.translation * node.inverse_scale_factor();
            Rect::from_center_size(center, node.size() * node.inverse_scale_factor())
        };
        if tool == Tool::Rotate {
            let center = canvas.min + edit::interface::model_handle_center(app.world());
            move_pointer(&mut app, window, canvas.min + Vec2::splat(4.0));
            let scale = app.world().get::<Window>(window).unwrap().scale_factor();
            let crop = ((center - Vec2::splat(86.0)) * scale)
                .max(Vec2::ZERO)
                .as_uvec2();
            let side = (172.0 * scale).ceil() as u32;
            let screenshot = capture_when(
                &mut app,
                &output,
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("target/model-rotation-handles.png"),
                |image| {
                    let mut count = [0; 3];
                    for y in crop.y..(crop.y + side).min(image.height()) {
                        for x in crop.x..(crop.x + side).min(image.width()) {
                            let [r, g, b, _] = image.get_pixel(x, y).0;
                            count[0] += usize::from(r > 190 && g < 90 && b < 135);
                            count[1] += usize::from(g > 165 && r < 180 && b < 80);
                            count[2] += usize::from(b > 190 && r < 100 && g > 75);
                        }
                    }
                    count.iter().all(|&n| n > 45)
                },
            );
            let crop_width = side.min(screenshot.width() - crop.x);
            let crop_height = side.min(screenshot.height() - crop.y);
            image::imageops::crop_imm(&screenshot, crop.x, crop.y, crop_width, crop_height)
                .to_image()
                .save(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("target/model-rotation-detail.png"),
                )
                .unwrap();
        }
        let (start, end) = super::model_transform::test_handle(
            app.world().resource::<ImageViewer>(),
            tab,
            canvas,
            tool,
        );
        move_pointer(&mut app, window, start);
        mouse(&mut app, window, ButtonState::Pressed);
        let placement = |app: &App| {
            let material = app
                .world()
                .resource::<Assets<CanvasMaterial>>()
                .get(&super::canvas_material::HANDLE)
                .unwrap();
            (material.transform_style.x == 1.0).then(|| {
                (
                    material.transform_moving.clone().unwrap(),
                    material.transform_rows,
                )
            })
        };
        move_pointer(&mut app, window, start.lerp(end, 0.45));
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let (target, first_rows) = loop {
            frame(&mut app);
            if let Some(placement) = placement(&app) {
                break placement;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "No live canvas during {tool:?}"
            );
        };
        let first_pixels = (tool != Tool::Move).then(|| model_target_pixels(&mut app, &target));
        move_pointer(&mut app, window, end);
        let (end_target, end_rows) = placement(&app).expect("Live canvas dropped a pointer sample");
        assert_eq!(end_target, target);
        let mut end_pixels = first_pixels.map(|first| {
            let last = model_target_pixels(&mut app, &target);
            assert!(last.chunks_exact(4).any(|px| px[3] > 128));
            assert!(
                first != last || first_rows != end_rows,
                "{tool:?} preview stayed still"
            );
            last
        });
        let expected = app
            .world()
            .resource::<ImageViewer>()
            .model_drag
            .as_ref()
            .unwrap()
            .preview;
        let handles = edit::interface::model_handle_geometry(app.world());
        assert_eq!(handles.1, expected.rotation);
        assert_ne!(expected, before);
        assert_eq!(model_pose(&app), before);
        mouse(&mut app, window, ButtonState::Released);
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        let mut pending_frames = 0;
        while model_pose(&app) == before {
            let viewer = app.world().resource::<ImageViewer>();
            assert_eq!(viewer.released_model.map(|(_, pose)| pose), Some(expected));
            assert_eq!(
                placement(&app),
                Some((target.clone(), end_rows)),
                "{tool:?} jumped on release"
            );
            assert_eq!(edit::interface::model_handle_geometry(app.world()), handles);
            pending_frames += 1;
            if let Some(last) = end_pixels.take() {
                assert!(
                    model_target_pixels(&mut app, &target) == last,
                    "{tool:?} image changed after release"
                );
            } else {
                frame(&mut app);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Transform did not apply: {:?}",
                app.world().resource::<ImageViewer>().edit.error
            );
        }
        assert_eq!(edit::interface::model_handle_geometry(app.world()), handles);
        assert!(pending_frames > 0);
        assert_eq!(model_pose(&app), expected);
        let after = model_pose(&app);
        match tool {
            Tool::Move => assert_ne!(before.position, after.position),
            Tool::Rotate => assert_ne!(before.rotation, after.rotation),
            Tool::Scale => assert_ne!(before.scale, after.scale),
            Tool::Universal => unreachable!(),
        }
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Undo));
        frame(&mut app);
        assert_eq!(model_pose(&app), before);
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Redo));
        frame(&mut app);
        assert_eq!(model_pose(&app), after);
    }
    capture_when(
        &mut app,
        &output,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-reference.png"),
        |_| true,
    );
    exercise_model_inspector(&mut app, window, tab);

    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Save));
    frame(&mut app);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while app.world().resource::<ImageViewer>().edit.file_busy(tab) {
        frame(&mut app);
        assert!(
            std::time::Instant::now() < deadline,
            "Reference layers did not save"
        );
    }
    let reloaded = crate::editor::photo_document::PhotoDocument::load(&source).unwrap();
    assert_eq!(reloaded.document.layers.len(), 3);
    assert!(
        reloaded.document.layers[1..]
            .iter()
            .all(|layer| layer.locks.pixels)
    );
    let reloaded_pose = reloaded.document.layers[2]
        .xmp
        .as_deref()
        .and_then(crate::editor::image_reference::ModelReference::from_xmp)
        .unwrap()
        .pose;
    assert_eq!(reloaded_pose, model_pose(&app));
}

/// Exercise keyboard focus, native window rendering, and silhouette picking after an external rename.
fn exercise_model_inspector(app: &mut App, window: Entity, tab: usize) {
    use crate::editor::selection_outline::render_tests::frame;
    use bevy::input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    };
    use lighting_ui_render::{mouse, move_pointer};
    #[cfg(windows)]
    #[allow(deprecated)]
    let (_events, native) = {
        use winit::{event_loop::EventLoop, platform::windows::EventLoopBuilderExtWindows};
        let events = EventLoop::builder().with_any_thread(true).build().unwrap();
        let native = bevy::window::WindowWrapper::new(
            events
                .create_window(
                    winit::window::Window::default_attributes()
                        .with_visible(false)
                        .with_inner_size(winit::dpi::PhysicalSize::new(1280, 900)),
                )
                .unwrap(),
        );
        app.world_mut()
            .entity_mut(window)
            .insert(bevy::window::RawHandleWrapper::new(&native).unwrap());
        let world = app.world_mut();
        *world
            .query_filtered::<&mut bevy::camera::RenderTarget, With<bevy::ui::IsDefaultUiCamera>>()
            .single_mut(world)
            .unwrap() = bevy::camera::RenderTarget::Window(bevy::window::WindowRef::Entity(window));
        (events, native)
    };
    let pose = |app: &App| {
        app.world()
            .resource::<ImageViewer>()
            .edit
            .model_reference(Some(tab))
            .unwrap()
            .pose
    };
    let press = |app: &mut App, code: KeyCode, key: Key| {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code: code,
                logical_key: key.clone(),
                state,
                text: None,
                repeat: false,
                window,
            });
            frame(app);
        }
    };
    let wait = |app: &mut App| {
        let end = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            frame(app);
            if !app
                .world()
                .resource::<crate::editor::image_reference::ReferenceRender>()
                .pending_for(Some(tab))
            {
                break;
            }
            assert!(std::time::Instant::now() < end, "Transform did not finish");
        }
    };
    for (code, letter, tool) in [
        (KeyCode::KeyR, "r", Some(Tool::Rotate)),
        (KeyCode::KeyS, "s", Some(Tool::Scale)),
        (KeyCode::KeyQ, "q", None),
        (KeyCode::KeyG, "g", Some(Tool::Move)),
    ] {
        press(app, code, Key::Character(letter.into()));
        assert_eq!(app.world().resource::<ImageViewer>().edit.model_tool, tool);
    }
    for (dpi, zoom, bg) in [(1.0, 1.0, Backdrop::Dark), (2.0, 1.5, Backdrop::Light)] {
        app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(zoom);
        app.world_mut().resource_mut::<ImageViewer>().background = bg;
        #[cfg(windows)]
        {
            let _ = native.request_inner_size(winit::dpi::PhysicalSize::new(
                (1280.0 * dpi) as u32,
                (900.0 * dpi) as u32,
            ));
            let mut win = app.world_mut().get_mut::<Window>(window).unwrap();
            win.resolution.set_scale_factor_override(Some(dpi));
            win.resolution.set(1280.0, 900.0);
        }
        for _ in 0..8 {
            frame(app);
        }
        let canvas = {
            let world = app.world_mut();
            let (n, t) = world
                .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>()
                .single(world)
                .unwrap();
            Rect::from_center_size(
                t.translation * n.inverse_scale_factor(),
                n.size() * n.inverse_scale_factor(),
            )
        };
        let (solid, clear) = {
            let viewer = app.world().resource::<ImageViewer>();
            let size = viewer.dimensions.unwrap();
            let image = viewer.image_rect(size, canvas);
            let doc = viewer.edit.document(Some(tab)).unwrap();
            let photocraft_doc::LayerContent::Raster(surface) =
                &doc.active_layer().unwrap().content
            else {
                unreachable!()
            };
            let on_handle = super::model_transform::test_move_hit(viewer, tab, canvas);
            let mut clear = None;
            let mut solid = None;
            for y in (0..size.y).step_by(2) {
                for x in (0..size.x).step_by(2) {
                    let p = image.min
                        + (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / size.as_vec2()
                            * image.size();
                    if !canvas.contains(p) || on_handle(p) {
                        continue;
                    }
                    if surface.sample_channel(x as i32, y as i32, 3) > 0.5 {
                        solid = Some(p);
                    } else {
                        clear = Some(p);
                    }
                    if solid.is_some() && clear.is_some() {
                        break;
                    }
                }
                if solid.is_some() && clear.is_some() {
                    break;
                }
            }
            (
                solid.expect("visible silhouette outside handles"),
                clear.unwrap(),
            )
        };
        let before = pose(app);
        move_pointer(app, window, clear);
        mouse(app, window, ButtonState::Pressed);
        assert!(app.world().resource::<ImageViewer>().model_drag.is_none());
        mouse(app, window, ButtonState::Released);
        move_pointer(app, window, solid);
        mouse(app, window, ButtonState::Pressed);
        assert!(
            app.world().resource::<ImageViewer>().model_drag.is_some(),
            "silhouette at DPI {dpi}"
        );
        if dpi == 1.0 {
            let revision = app
                .world()
                .resource::<ImageViewer>()
                .edit
                .document(Some(tab))
                .unwrap()
                .revision;
            mouse(app, window, ButtonState::Released);
            assert_eq!(
                app.world()
                    .resource::<ImageViewer>()
                    .edit
                    .document(Some(tab))
                    .unwrap()
                    .revision,
                revision
            );
            mouse(app, window, ButtonState::Pressed);
        }
        move_pointer(app, window, solid + Vec2::new(14.0, 9.0));
        if dpi == 1.0 {
            press(app, KeyCode::Escape, Key::Escape);
        }
        mouse(app, window, ButtonState::Released);
        wait(app);
        if dpi == 1.0 {
            assert_eq!(pose(app), before);
        } else {
            let delta = Vec3::from(pose(app).position) - Vec3::from(before.position);
            assert!(delta.length() > 0.001);
            let model = app
                .world()
                .resource::<ImageViewer>()
                .edit
                .model_reference(Some(tab))
                .unwrap();
            assert!(delta.dot(*model.view().back()).abs() < 1e-4);
        }
    }
    let field = |app: &mut App, name: &str| {
        let w = app.world_mut();
        w.query_filtered::<(Entity, &AccessibleLabel, &ComputedNode), With<super::model_fields::PoseField>>()
            .iter(w)
            .find(|(_, label, n)| label.0 == name && !n.is_empty())
            .unwrap()
            .0
    };
    let x = field(app, "Позиция X");
    let y = field(app, "Позиция Y");
    let write = |app: &mut App, id, value: &str| {
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(id, bevy::input_focus::FocusCause::Auto);
        frame(app);
        app.world_mut()
            .get_mut::<EditableText>(id)
            .unwrap()
            .editor_mut()
            .set_text(value);
    };
    let before = pose(app);
    write(app, x, "bad");
    press(app, KeyCode::Enter, Key::Enter);
    assert_eq!(pose(app), before);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(x));
    press(app, KeyCode::Escape, Key::Escape);
    assert_eq!(pose(app), before);
    write(app, x, "0,125");
    write(app, y, "0.25");
    press(app, KeyCode::Enter, Key::Enter);
    wait(app);
    assert_eq!(pose(app).position[..2], [0.125, 0.25]);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    frame(app);
    assert_eq!(pose(app).position[..2], [0.125, before.position[1]]);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Redo));
    frame(app);
    assert_eq!(pose(app).position[..2], [0.125, 0.25]);
}
