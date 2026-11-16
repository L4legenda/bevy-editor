use super::*;
use crate::editor::{
    photo_document::PhotoTool,
    selection_outline::render_tests::{capture_when, frame},
};

fn picture_rect(app: &App) -> Rect {
    let picture = app.world().resource::<ViewerUi>().picture;
    let node = app.world().get::<ComputedNode>(picture).unwrap();
    let transform = app.world().get::<UiGlobalTransform>(picture).unwrap();
    Rect::from_center_size(transform.affine().translation, node.size())
}

fn sample(image: &image::RgbaImage, rect: Rect, x: f32, y: f32) -> [u8; 4] {
    let point = rect.min + Vec2::new(x, y) * (rect.size() / 128.0);
    image
        .get_pixel(point.x.floor() as u32, point.y.floor() as u32)
        .0
}

fn assert_grid(image: &image::RgbaImage, rect: Rect, shown: bool) {
    let cell = rect.width() / 128.0;
    let white_center = sample(image, rect, 8.5, 64.5)[0];
    let white_line = sample(image, rect, 8.0 + 0.5 / cell, 64.5)[0];
    let black_center = sample(image, rect, 40.5, 64.5)[0];
    let black_line = sample(image, rect, 40.0 + 0.5 / cell, 64.5)[0];
    assert!(white_center > 248 && black_center < 5);
    if shown {
        assert!(
            (185..240).contains(&white_line),
            "Grid invisible on white: {white_line}"
        );
        assert!(
            (35..95).contains(&black_line),
            "Grid invisible on black: {black_line}"
        );
        assert!(
            sample(image, rect, 40.0 + 1.5 / cell, 64.5)[0] < 5,
            "The grid must remain one physical pixel wide"
        );
    } else {
        assert!(
            white_line > 248 && black_line < 5,
            "Grid must disappear at normal zoom"
        );
    }
    // A transparent cell only shows the original 27/35 checker, including its grid boundaries.
    for x in [105.0 + 0.5 / cell, 105.5] {
        let checker = sample(image, rect, x, 64.5)[0];
        assert!(
            (25..39).contains(&checker),
            "Grid must not draw over transparency: {checker}"
        );
    }
    if cell >= 2.0 {
        assert!(
            sample(image, rect, 72.25, 64.5)[0] > 248,
            "Magnified white texels must have flat interiors"
        );
        assert!(
            sample(image, rect, 73.25, 64.5)[0] < 5,
            "Magnified black texels must not blur"
        );
    }
}

fn capture(
    app: &mut App,
    target: &Handle<Image>,
    output: &std::path::Path,
) -> (image::RgbaImage, Rect) {
    for _ in 0..6 {
        frame(app);
    }
    let rect = picture_rect(app);
    let image = capture_when(app, target, output, |image| {
        sample(image, rect, 40.5, 64.5)[0] < 5 && sample(image, rect, 8.5, 64.5)[0] > 248
    });
    (image, rect)
}

#[test]
#[ignore = "requires GPU; verifies zoom-driven pixel grid, sharp eraser display, pan and Retina"]
fn gpu_photo_canvas_grid_and_sharpness_follow_zoom_instead_of_the_tool() {
    let fixture = Fixture::new();
    image::RgbaImage::from_fn(128, 128, |x, _| match x {
        0..32 => image::Rgba([255; 4]),
        32..64 => image::Rgba([0, 0, 0, 255]),
        64..96 if x % 2 == 0 => image::Rgba([255; 4]),
        64..96 => image::Rgba([0, 0, 0, 255]),
        _ => image::Rgba([0; 4]),
    })
    .save(fixture.0.join("pixel-grid.png"))
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
    let index = file(&app, "pixel-grid.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..30 {
        frame(&mut app);
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-pixel-grid-qa");
    std::fs::create_dir_all(&output).unwrap();
    for (zoom, name, shown) in [
        (4.0, "01-400-percent.png", false),
        (5.0, "02-500-percent.png", false),
        (8.0, "03-800-percent.png", true),
    ] {
        app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(zoom);
        let (image, rect) = capture(&mut app, &target, &output.join(name));
        assert_grid(&image, rect, shown);
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
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
    for _ in 0..6 {
        frame(&mut app);
    }
    let image_id = app
        .world()
        .resource::<ImageViewer>()
        .image
        .as_ref()
        .unwrap()
        .id();
    let mut image_events =
        bevy::ecs::message::MessageCursor::<bevy::asset::AssetEvent<Image>>::default();
    image_events.clear(
        app.world()
            .resource::<Messages<bevy::asset::AssetEvent<Image>>>(),
    );
    let mut baseline = None;
    for (tool, name) in [
        (PhotoTool::Brush, "04-brush.png"),
        (PhotoTool::Pencil, "05-pencil.png"),
        (PhotoTool::Eraser, "06-eraser.png"),
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Tool(tool)));
        let (image, rect) = capture(&mut app, &target, &output.join(name));
        assert_grid(&image, rect, true);
        assert!(!image_events.read(app.world().resource::<Messages<bevy::asset::AssetEvent<Image>>>())
            .any(|event| matches!(event, bevy::asset::AssetEvent::Modified { id } if *id == image_id)),
            "Tool selection must not rebuild or upload the document texture");
        // Tool selection must not change any displayed document pixel or checker pixel.
        let crop = image::imageops::crop_imm(
            &image,
            (rect.min.x + 64.0) as u32,
            (rect.min.y + 500.0) as u32,
            800,
            100,
        )
        .to_image();
        if let Some(baseline) = &baseline {
            assert_eq!(
                &crop, baseline,
                "Switching to {tool:?} changed canvas filtering"
            );
        } else {
            baseline = Some(crop);
        }
        assert!(
            !app.world()
                .resource::<ImageViewer>()
                .edit
                .document(app.world().resource::<Editor>().viewing_image)
                .unwrap()
                .dirty()
        );
    }
    app.world_mut().resource_mut::<ImageViewer>().pan = Vec2::new(13.375, -11.125);
    let (image, rect) = capture(&mut app, &target, &output.join("07-panned.png"));
    assert_grid(&image, rect, true);
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
    let (image, rect) = capture(&mut app, &target, &output.join("08-retina.png"));
    assert_grid(&image, rect, true);
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(4.0);
    let (image, rect) = capture(&mut app, &target, &output.join("09-retina-zoom-out.png"));
    assert_grid(&image, rect, false);
    println!("Pixel canvas captures: {}", output.display());
}
