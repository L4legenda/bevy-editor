use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use bevy::{
    input::{ButtonState, mouse::MouseButtonInput},
    window::{CursorMoved, SystemCursorIcon, WindowEvent},
};

fn grip_rect(app: &mut App) -> Rect {
    let world = app.world_mut();
    let mut query = world
        .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<panel_resize::ResizeGrip>>();
    let (node, transform) = query.single(world).unwrap();
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}

fn canvas_width(app: &mut App) -> f32 {
    let world = app.world_mut();
    let mut query = world.query_filtered::<&ComputedNode, With<ImageCanvas>>();
    let node = query.single(world).unwrap();
    node.size().x * node.inverse_scale_factor()
}

fn move_to(app: &mut App, window: Entity, position: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(position));
    app.world_mut()
        .write_message(WindowEvent::CursorMoved(CursorMoved {
            window,
            position,
            delta: None,
        }));
    frame(app);
}

fn button(app: &mut App, window: Entity, state: ButtonState) {
    let input = MouseButtonInput {
        window,
        button: MouseButton::Left,
        state,
    };
    app.world_mut()
        .write_message(WindowEvent::MouseButtonInput(input));
    app.world_mut().write_message(input);
    frame(app);
}

pub(super) fn resize_target(
    app: &mut App,
    window: Entity,
    target: &Handle<Image>,
    width: f32,
    scale: f32,
) {
    {
        let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
        window.resolution.set_scale_factor_override(Some(scale));
        window.resolution.set(width, 900.0);
    }
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(target)
        .unwrap()
        .resize(Extent3d {
            width: (width * scale) as u32,
            height: (900.0 * scale) as u32,
            depth_or_array_layers: 1,
        });
    {
        let world = app.world_mut();
        let mut cameras = world
            .query_filtered::<&mut bevy::camera::RenderTarget, With<bevy::ui::IsDefaultUiCamera>>();
        *cameras.single_mut(world).unwrap() =
            bevy::camera::RenderTarget::Image(bevy::camera::ImageRenderTarget {
                handle: target.clone(),
                scale_factor: scale,
            });
    }
    for _ in 0..8 {
        frame(app);
    }
}

fn assert_clean(app: &App, original: &[u8]) {
    let viewer = app.world().resource::<ImageViewer>();
    let doc = viewer.edit.document(viewer.tab).unwrap();
    assert!(
        !doc.dirty() && !doc.can_undo(),
        "Inspector resizing must not paint or create history"
    );
    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(viewer.image.as_ref().unwrap())
        .unwrap();
    assert_eq!(image.data.as_ref().unwrap(), original);
    assert_eq!(
        viewer.pan,
        Vec2::ZERO,
        "Inspector resizing must not pan the canvas"
    );
}

#[test]
#[ignore = "requires GPU; drags the image inspector at both DPI scales and checks canvas size, pointer capture and preserved pixels"]
fn gpu_photo_inspector_resizes_without_painting_and_remembers_its_width() {
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(32, 32, image::Rgba([225, 110, 65, 255]))
        .save(fixture.0.join("panel.png"))
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
    let index = file(&app, "panel.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..30 {
        frame(&mut app);
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
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(32.0);
    for _ in 0..8 {
        frame(&mut app);
    }
    let original = {
        let viewer = app.world().resource::<ImageViewer>();
        app.world()
            .resource::<Assets<Image>>()
            .get(viewer.image.as_ref().unwrap())
            .unwrap()
            .data
            .clone()
            .unwrap()
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-panel-resize-qa");
    std::fs::create_dir_all(&output).unwrap();
    for scale in [1.0, 2.0] {
        resize_target(&mut app, window, &target, 1280.0, scale);
        let initial = app.world().resource::<ImageViewer>().panel.width(1280.0);
        let canvas = canvas_width(&mut app);
        let start = grip_rect(&mut app).center();
        move_to(&mut app, window, start - Vec2::X * 60.0);
        move_to(&mut app, window, start);
        assert_eq!(
            app.world().get::<CursorIcon>(window),
            Some(&CursorIcon::from(SystemCursorIcon::EwResize))
        );
        button(&mut app, window, ButtonState::Pressed);
        move_to(&mut app, window, start - Vec2::X * 120.0);
        assert_eq!(
            app.world().resource::<ImageViewer>().panel.width(1280.0),
            initial + 120.0
        );
        assert!((canvas_width(&mut app) - (canvas - 120.0)).abs() < 1.0);
        assert_clean(&app, &original);
        button(&mut app, window, ButtonState::Released);
        frame(&mut app);
        assert!(!app.world().resource::<ImageViewer>().panel.capturing());
        let restored = initial + 120.0;
        resize_target(&mut app, window, &target, 720.0, scale);
        assert!(app.world().resource::<ImageViewer>().panel.width(720.0) <= 346.0);
        assert!(canvas_width(&mut app) >= 319.0);
        resize_target(&mut app, window, &target, 1280.0, scale);
        assert_eq!(
            app.world().resource::<ImageViewer>().panel.width(1280.0),
            restored
        );
        capture_when(
            &mut app,
            &target,
            &output.join(format!("wide-{}x.png", scale as u32)),
            |_| true,
        );
        assert_clean(&app, &original);
    }
    let wide = app.world().resource::<ImageViewer>().panel.width(1280.0);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::View));
    for _ in 0..4 {
        frame(&mut app);
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..8 {
        frame(&mut app);
    }
    assert_eq!(
        app.world().resource::<ImageViewer>().panel.width(1280.0),
        wide
    );
    let start = grip_rect(&mut app).center();
    move_to(&mut app, window, start);
    button(&mut app, window, ButtonState::Pressed);
    move_to(&mut app, window, Vec2::new(1272.0, start.y));
    button(&mut app, window, ButtonState::Released);
    frame(&mut app);
    assert_eq!(
        app.world().resource::<ImageViewer>().panel.width(1280.0),
        220.0
    );
    assert_clean(&app, &original);
    move_to(&mut app, window, Vec2::new(20.0, 20.0));
    assert!(!matches!(
        app.world().get::<CursorIcon>(window),
        Some(CursorIcon::System(SystemCursorIcon::EwResize))
    ));
    capture_when(&mut app, &target, &output.join("narrow-2x.png"), |_| true);
    println!("Image inspector captures: {}", output.display());
}
