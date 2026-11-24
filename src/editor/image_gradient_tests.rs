use super::*;
use crate::editor::{
    photo_document::{PhotoDocument, PhotoTool},
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{mouse, move_pointer},
};
use bevy::{
    ecs::system::RunSystemOnce,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
};
use edit::EditAction;

fn warm(app: &mut App, count: usize) {
    for _ in 0..count {
        frame(app);
    }
}

fn send(app: &mut App, action: EditAction) {
    app.world_mut().write_message(Action::ImageEdit(action));
    warm(app, 3);
}

fn doc(app: &App) -> &PhotoDocument {
    let viewer = app.world().resource::<ImageViewer>();
    viewer.edit.document(viewer.tab).unwrap()
}

#[test]
fn gradient_preview_respects_selection_transparency_and_matches_commit() {
    let fixture = Fixture::new();
    let file = fixture.0.join("gradient.png");
    image::RgbaImage::from_pixel(64, 48, image::Rgba([0; 4]))
        .save(&file)
        .unwrap();
    let mut doc = PhotoDocument::load(&file).unwrap();
    let mut selection = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
    let bounds = photocraft_geom::Rect::new(8, 8, 56, 40);
    selection.write_interleaved(bounds, &vec![255; 48 * 32]);
    doc.document.selection = Some(selection);
    let before = doc.composite().pixels;
    let settings = gradient::Settings {
        transparent: true,
        ..default()
    };
    let draft = gradient::Draft {
        from: Vec2::new(16.5, 24.5),
        to: Vec2::new(48.5, 24.5),
        colors: settings.colors([1.0, 0.0, 0.0, 1.0], [1.0; 4]),
        radial: false,
        min_length: 4.0,
        revision: doc.revision,
        view_revision: doc.view_revision,
        layer: doc.active_id(),
        target: doc.paint_target(),
    };
    assert!(draft.valid_for(&doc));
    let params = draft.params();
    let preview = doc.gradient_preview(params.clone())().unwrap();
    assert_eq!(doc.composite().pixels, before);
    assert!(!doc.can_undo());
    doc.layer_command("paint.gradient", params).unwrap();
    assert_eq!(doc.composite().pixels, preview.pixels);
    let alpha = |x: usize, y: usize| preview.pixels[(y * 64 + x) * 4 + 3];
    assert_eq!(alpha(4, 24), 0, "selection excludes this pixel");
    assert_eq!(alpha(12, 24), 255);
    assert!((126..=129).contains(&alpha(32, 24)));
    assert_eq!(
        alpha(52, 24),
        0,
        "transparent endpoint leaves the layer alone"
    );
    doc.undo();
    assert_eq!(doc.composite().pixels, before);
    assert!(!doc.can_undo(), "one drag makes one undo step");
    doc.redo();
    assert_eq!(doc.composite().pixels, preview.pixels);
    let snapped = gradient::snap(Vec2::ZERO, Vec2::new(72.0, 60.0), true);
    assert!((snapped.x - snapped.y).abs() < 0.001);
}

#[test]
#[ignore = "requires GPU: gesture, exact live preview, shader guides and cancellation"]
fn gpu_gradient_canvas_gesture() {
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(96, 64, image::Rgba([255; 4]))
        .save(fixture.0.join("gradient.png"))
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
    let index = file(&app, "gradient.png");
    app.world_mut().write_message(Action::ViewImage(index));
    warm(&mut app, 40);
    send(&mut app, EditAction::Edit);
    for _ in 0..120 {
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
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(5.0);
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Gradient)))
    });
    send(&mut app, EditAction::Color([0.75, 0.10, 0.30, 1.0]));
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
    let screen = |x, y| image.min + Vec2::new(x, y) * image.size() / Vec2::new(96.0, 64.0);
    let from = screen(16.0, 32.0);
    let to = screen(74.0, 32.0);
    let pixel = screen(8.0, 18.0).as_uvec2();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/gradient-qa");
    std::fs::create_dir_all(&output).unwrap();
    let before = doc(&app).composite().pixels;
    let drag = |app: &mut App| {
        move_pointer(app, window, from);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, to);
        warm(app, 10);
    };
    drag(&mut app);
    let shot = capture_when(
        &mut app,
        &target,
        &output.join("linear-draft.png"),
        |image| {
            let p = image.get_pixel(pixel.x, pixel.y).0;
            p[0] > 155 && p[1] < 70 && p[2] > 50
        },
    );
    let marker = from.as_uvec2();
    assert!(
        (-9..=9).any(|offset| {
            let p = shot
                .get_pixel((marker.x as i32 + offset) as u32, marker.y)
                .0;
            p[0] > 220 && p[1] > 160 && p[2] < 205
        }),
        "shader renders the gold endpoint"
    );
    assert_eq!(doc(&app).composite().pixels, before);
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
    warm(&mut app, 14);
    assert_eq!(doc(&app).composite().pixels, before);
    assert!(!doc(&app).can_undo());
    assert!(app.world().resource::<gradient::Preview>().image.is_none());
    click_action(&mut app, window, |action| {
        matches!(action, Action::ImageEdit(EditAction::GradientRadial(true)))
    });
    drag(&mut app);
    capture_when(
        &mut app,
        &target,
        &output.join("radial-draft.png"),
        |image| {
            let centre = image.get_pixel(marker.x, marker.y + 30).0;
            centre[1] < 100
        },
    );
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 14);
    assert_ne!(doc(&app).composite().pixels, before);
    assert!(app.world().resource::<gradient::Preview>().image.is_none());
    send(&mut app, EditAction::Undo);
    assert_eq!(doc(&app).composite().pixels, before);
    assert!(!doc(&app).can_undo());
}
