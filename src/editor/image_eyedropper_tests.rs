use super::*;
use crate::editor::photo_document::PhotoTool;
use bevy::{
    ecs::system::RunSystemOnce,
    window::{CustomCursor, SystemCursorIcon},
};
use edit::EditAction;

fn input(app: &mut App, window: Entity, position: Vec2, button: Option<bool>) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(position));
    if let Some(down) = button {
        let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        if down {
            mouse.press(MouseButton::Left);
        } else {
            mouse.release(MouseButton::Left);
        }
    }
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn image_data(app: &App) -> Vec<u8> {
    let viewer = app.world().resource::<ImageViewer>();
    app.world()
        .resource::<Assets<Image>>()
        .get(viewer.image.as_ref().unwrap())
        .unwrap()
        .data
        .clone()
        .unwrap()
}

#[test]
fn eyedropper_and_alt_sample_without_painting_restore_the_tool_and_release_the_cursor() {
    let fixture = Fixture::new();
    let source = fixture.0.join("eyedropper.png");
    let red = [240, 40, 60, 255];
    let blue = [40, 100, 230, 255];
    image::RgbaImage::from_fn(64, 64, |x, _| image::Rgba(if x < 32 { red } else { blue }))
        .save(&source)
        .unwrap();
    let source_bytes = std::fs::read(&source).unwrap();
    let mut app = fixture.app();
    app.add_systems(
        Update,
        tool_cursor::cursor.after(crate::editor::handle_actions),
    )
    .add_systems(Last, tool_cursor::cleanup);
    let index = file(&app, "eyedropper.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let rect = app
        .world_mut()
        .run_system_once(
            |canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
             window: Single<&Window, With<PrimaryWindow>>| {
                canvas_rect(&canvas, &window)
            },
        )
        .unwrap();
    let image = app
        .world()
        .resource::<ImageViewer>()
        .image_rect(UVec2::splat(64), rect);
    let left = image.min + Vec2::new(10.0, 20.0);
    let right = image.min + Vec2::new(50.0, 20.0);
    let original = image_data(&app);
    let color = |app: &App| app.world().resource::<ImageViewer>().edit.color();
    let rgba = |pixel: [u8; 4]| pixel.map(|v| v as f32 / 255.0);
    let cursor = |app: &App| app.world().get::<CursorIcon>(window).unwrap().clone();
    for tool in [
        PhotoTool::Brush,
        PhotoTool::Pencil,
        PhotoTool::Eraser,
        PhotoTool::Fill,
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(tool)));
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Size(17.0)));
        input(&mut app, window, left, None);
        let before = cursor(&app);
        for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(alt);
            input(&mut app, window, left, None);
            assert_eq!(
                app.world()
                    .resource::<ImageViewer>()
                    .pipette_preview
                    .unwrap()
                    .color,
                Color::srgb_u8(red[0], red[1], red[2])
            );
            let pipette = cursor(&app);
            let CursorIcon::Custom(CustomCursor::Image(ref bitmap)) = pipette else {
                panic!("Alt must show a custom pipette cursor");
            };
            assert_eq!(bitmap.hotspot, (4, 24));
            assert_ne!(pipette, before);
            assert!(
                app.world().resource::<ImageViewer>().brush_cursor.is_none(),
                "Hide the brush outline while sampling"
            );
            assert_eq!(app.world().resource::<ImageViewer>().edit.tool, tool);
            input(&mut app, window, left, Some(true));
            assert_eq!(color(&app), rgba(red));
            input(&mut app, window, right, None);
            assert_eq!(color(&app), rgba(blue));
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(alt);
            input(&mut app, window, left, None);
            assert_eq!(
                cursor(&app),
                pipette,
                "A sampling press stays a sampler until release"
            );
            assert_eq!(color(&app), rgba(red));
            input(&mut app, window, left, Some(false));
            assert!(
                app.world()
                    .resource::<ImageViewer>()
                    .pipette_preview
                    .is_none()
            );
            assert_eq!(
                cursor(&app),
                before,
                "Release restores the selected tool's cursor"
            );
            assert_eq!(image_data(&app), original);
            let viewer = app.world().resource::<ImageViewer>();
            let doc = viewer.edit.document(viewer.tab).unwrap();
            assert!(!doc.dirty() && !doc.can_undo());
        }
    }
    // Alt sampling must not turn a large brush touching the image from outside
    // into a stroke, or start painting when Alt is released mid-press.
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Tool(PhotoTool::Brush)));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    input(
        &mut app,
        window,
        image.min + Vec2::new(-2.0, 20.0),
        Some(true),
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::AltLeft);
    input(&mut app, window, left, None);
    input(&mut app, window, left, Some(false));
    assert_eq!(image_data(&app), original);

    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Tool(PhotoTool::Eyedropper)));
    input(&mut app, window, right, Some(true));
    assert_eq!(color(&app), rgba(blue));
    input(&mut app, window, right, Some(false));
    let pipette = cursor(&app);
    let CursorIcon::Custom(CustomCursor::Image(ref bitmap)) = pipette else {
        panic!("Expected pipette");
    };
    assert_eq!(bitmap.hotspot, (4, 24));
    input(&mut app, window, Vec2::splat(10.0), None);
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .pipette_preview
            .is_none()
    );
    assert_eq!(cursor(&app), CursorIcon::from(SystemCursorIcon::Default));
    input(&mut app, window, right, None);
    assert_eq!(cursor(&app), pipette);
    app.world_mut().write_message(Action::Open(0));
    app.update();
    assert_eq!(cursor(&app), CursorIcon::from(SystemCursorIcon::Default));
    // A late panel/chrome restore must recognise a pipette just like a brush.
    app.world_mut().entity_mut(window).insert(pipette);
    app.update();
    assert_eq!(cursor(&app), CursorIcon::from(SystemCursorIcon::Default));
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
}

#[test]
#[ignore = "requires GPU: verifies hover swatch, no-click colour sampling, Alt, viewport edges and DPI"]
fn gpu_eyedropper_hover_swatch_tracks_colour_without_picking_it() {
    use crate::editor::browser::Browser;
    use crate::editor::selection_outline::render_tests::{capture_when, frame};
    use crate::editor::tests::lighting_ui_render::move_pointer;
    let fixture = Fixture::new();
    let red = [240, 40, 60, 255];
    let blue = [40, 100, 230, 255];
    image::RgbaImage::from_fn(96, 64, |x, _| image::Rgba(if x < 48 { red } else { blue }))
        .save(fixture.0.join("hover.png"))
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
    let id = file(&app, "hover.png");
    app.world_mut().write_message(Action::ViewImage(id));
    for _ in 0..35 {
        frame(&mut app);
    }
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        if app.world().resource::<ImageViewer>().edit.editing(Some(id)) {
            break;
        }
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-eyedropper-qa");
    std::fs::create_dir_all(&output).unwrap();
    let assert_swatch = |app: &mut App, pixel: [u8; 4]| {
        let viewer = app.world().resource::<ImageViewer>();
        assert_eq!(
            viewer.pipette_preview.unwrap().color,
            Color::srgb_u8(pixel[0], pixel[1], pixel[2])
        );
        let world = app.world_mut();
        assert!(
            world
                .query::<(&AccessibleLabel, &ComputedNode)>()
                .iter(world)
                .any(|(l, n)| l.0 == "Цвет под пипеткой" && !n.is_empty())
        );
        assert!(
            world
                .query::<(&AccessibleLabel, &BackgroundColor)>()
                .iter(world)
                .any(|(l, c)| l.0 == "Образец цвета пипетки"
                    && c.0 == Color::srgb_u8(pixel[0], pixel[1], pixel[2]))
        );
    };
    for scale in [1.0, 1.5] {
        super::panel_sizing::resize_target(&mut app, window, &target, 1280.0, scale);
        {
            let mut viewer = app.world_mut().resource_mut::<ImageViewer>();
            viewer.zoom = Zoom::Scale(3.0);
            viewer.pan = Vec2::new(45.0, 30.0);
        }
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(PhotoTool::Eyedropper)));
        frame(&mut app);
        let viewport = app
            .world_mut()
            .run_system_once(
                |canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
                 window: Single<&Window, With<PrimaryWindow>>| {
                    canvas_rect(&canvas, &window)
                },
            )
            .unwrap();
        let image = app
            .world()
            .resource::<ImageViewer>()
            .image_rect(UVec2::new(96, 64), viewport);
        let revision = app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .revision;
        let color = app.world().resource::<ImageViewer>().edit.color();
        move_pointer(
            &mut app,
            window,
            image.min + image.size() * Vec2::new(0.2, 0.5),
        );
        assert_swatch(&mut app, red);
        if scale == 1.0 {
            capture_when(&mut app, &target, &output.join("hover-red.png"), |_| true);
        }
        move_pointer(
            &mut app,
            window,
            image.min + image.size() * Vec2::new(0.8, 0.5),
        );
        assert_swatch(&mut app, blue);
        if scale == 1.0 {
            capture_when(&mut app, &target, &output.join("hover-blue.png"), |_| true);
        }
        assert_eq!(
            app.world().resource::<ImageViewer>().edit.color(),
            color,
            "Hover must not pick a colour"
        );
        assert_eq!(
            app.world()
                .resource::<ImageViewer>()
                .edit
                .document(Some(id))
                .unwrap()
                .revision,
            revision
        );
        move_pointer(&mut app, window, image.min - Vec2::splat(4.0));
        assert!(
            app.world()
                .resource::<ImageViewer>()
                .pipette_preview
                .is_none()
        );
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(PhotoTool::Brush)));
        frame(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::AltLeft);
        move_pointer(&mut app, window, image.center() + Vec2::new(12.0, 0.0));
        assert_swatch(&mut app, blue);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::AltLeft);
        frame(&mut app);
        assert!(
            app.world()
                .resource::<ImageViewer>()
                .pipette_preview
                .is_none()
        );
        // Keep the swatch inside the workspace when the hotspot approaches its edge.
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(PhotoTool::Eyedropper)));
        let edge = viewport.max - Vec2::splat(4.0);
        app.world_mut().resource_mut::<ImageViewer>().pan +=
            edge - (image.min + image.size() * Vec2::new(0.8, 0.5));
        move_pointer(&mut app, window, edge);
        assert_swatch(&mut app, blue);
        let sample = app
            .world()
            .resource::<ImageViewer>()
            .pipette_preview
            .unwrap();
        assert!(sample.position.cmpge(viewport.min).all());
        assert!(
            (sample.position + Vec2::splat(28.0))
                .cmple(viewport.max)
                .all()
        );
        move_pointer(&mut app, window, Vec2::new(1200.0, 250.0));
        assert!(
            app.world()
                .resource::<ImageViewer>()
                .pipette_preview
                .is_none()
        );
        let world = app.world_mut();
        assert!(
            world
                .query::<(&AccessibleLabel, &ComputedNode)>()
                .iter(world)
                .any(|(l, n)| l.0 == "Цвет под пипеткой" && n.is_empty())
        );
    }
}
