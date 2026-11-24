use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
use bevy::input::ButtonState;

fn bounds(app: &mut App, name: &str) -> Option<Rect> {
    let world = app.world_mut();
    world
        .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .find(|(label, node, _)| label.0 == name && !node.is_empty())
        .map(|(_, node, transform)| {
            let dpi = node.inverse_scale_factor();
            Rect::from_center_size(transform.affine().translation * dpi, node.size() * dpi)
        })
}

#[test]
#[ignore = "requires GPU: full canvas miniature, viewport reticle and shared live pixels"]
fn gpu_overview_keeps_the_whole_canvas_visible_when_zoomed_and_updates_after_fill() {
    let fixture = Fixture::new();
    image::RgbaImage::from_fn(400, 220, |x, _| {
        image::Rgba(if x < 200 {
            [220, 65, 48, 255]
        } else {
            [38, 155, 226, 255]
        })
    })
    .save(fixture.0.join("overview.png"))
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
    let index = file(&app, "overview.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..40 {
        frame(&mut app);
    }
    assert!(bounds(&mut app, "Обзор всего холста").is_none());
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        if bounds(&mut app, "Обзор всего холста").is_some() {
            break;
        }
    }
    for _ in 0..6 {
        frame(&mut app);
    }
    let overview = bounds(&mut app, "Обзор всего холста").unwrap();
    assert!(overview.min.x > 960.0 && overview.min.y < 220.0);
    assert!(bounds(&mut app, "Видимая область холста").is_none());
    {
        let mut viewer = app.world_mut().resource_mut::<ImageViewer>();
        viewer.zoom = Zoom::Scale(6.0);
        viewer.pan = Vec2::new(610.0, 0.0);
    }
    for _ in 0..7 {
        frame(&mut app);
    }
    let reticle = bounds(&mut app, "Видимая область холста").unwrap();
    assert!(overview.contains(reticle.min) && overview.contains(reticle.max));
    assert!(reticle.width() < overview.width() * 0.6 && reticle.center().x < overview.center().x);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/canvas-overview-qa");
    std::fs::create_dir_all(&output).unwrap();
    let left = (overview.center() - Vec2::new(60.0, 20.0)).as_uvec2();
    let right = (overview.center() + Vec2::new(60.0, -20.0)).as_uvec2();
    let right_visible = |pixels: &image::RgbaImage| pixels.get_pixel(right.x, right.y)[2] > 200;
    capture_when(
        &mut app,
        &target,
        &output.join("overview-zoomed.png"),
        |pixels| pixels.get_pixel(left.x, left.y)[0] > 200 && right_visible(pixels),
    );
    for action in [
        edit::EditAction::Tool(crate::editor::photo_document::PhotoTool::Fill),
        edit::EditAction::Color([0.05, 0.78, 0.34, 1.0]),
    ] {
        app.world_mut().write_message(Action::ImageEdit(action));
    }
    frame(&mut app);
    move_pointer(&mut app, window, Vec2::new(480.0, 450.0));
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    capture_when(
        &mut app,
        &target,
        &output.join("overview-after-fill.png"),
        |pixels| {
            let pixel = pixels.get_pixel(left.x, left.y);
            pixel[0] < 40 && pixel[1] > 180 && right_visible(pixels)
        },
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::View));
    for _ in 0..6 {
        frame(&mut app);
    }
    assert!(bounds(&mut app, "Обзор всего холста").is_none());
}
