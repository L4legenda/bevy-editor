use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
use bevy::{
    ecs::system::RunSystemOnce,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
};
use edit::{ColorTarget, EditAction};

fn warm(app: &mut App, frames: usize) {
    for _ in 0..frames {
        frame(app);
    }
}
fn send(app: &mut App, action: EditAction) {
    app.world_mut().write_message(Action::ImageEdit(action));
    warm(app, 3);
}
fn doc(app: &App) -> &crate::editor::photo_document::PhotoDocument {
    let viewer = app.world().resource::<ImageViewer>();
    viewer.edit.document(viewer.tab).unwrap()
}
fn source(app: &mut App, target: ColorTarget) -> Vec2 {
    let world = app.world_mut();
    world
        .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
        .find_map(|(c, n, t)| {
            (matches!(c.action,Action::ImageEdit(EditAction::ColorPicker(color)) if color==target)
                && !n.is_empty())
            .then_some(t.affine().translation * n.inverse_scale_factor())
        })
        .unwrap()
}
fn visible(app: &mut App, label: &str) -> bool {
    let world = app.world_mut();
    world
        .query::<(&AccessibleLabel, &ComputedNode, &Node)>()
        .iter(world)
        .any(|(l, n, node)| l.0 == label && !n.is_empty() && node.display != Display::None)
}
fn image_rect(app: &mut App) -> Rect {
    let viewport = app
        .world_mut()
        .run_system_once(
            |canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
             window: Single<&Window, With<PrimaryWindow>>| {
                canvas_rect(&canvas, &window)
            },
        )
        .unwrap();
    let viewer = app.world().resource::<ImageViewer>();
    viewer.image_rect(viewer.dimensions.unwrap(), viewport)
}
fn pixel_point(image: Rect, point: Vec2) -> Vec2 {
    image.min + point / UVec2::new(96, 64).as_vec2() * image.size()
}
fn lift(app: &mut App, window: Entity, target: ColorTarget) {
    let point = source(app, target);
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    warm(app, 20);
    assert!(
        visible(app, "Переносимый цвет"),
        "Holding a swatch must lift the colour"
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_none()
    );
}
fn escape(app: &mut App, window: Entity) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state,
            text: None,
            repeat: false,
        });
        warm(app, 2);
    }
}

#[test]
#[ignore = "requires GPU: long press, real pointer drag, flood fill, shader rendering, cancellation and DPI"]
fn gpu_color_drop_fills_connected_region_with_undo_and_animated_feedback() {
    let fixture = Fixture::new();
    image::RgbaImage::from_fn(96, 64, |x, _| {
        if (47..49).contains(&x) {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255; 4])
        }
    })
    .save(fixture.0.join("colour-drop.png"))
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
    let index = file(&app, "colour-drop.png");
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
    warm(&mut app, 8);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/color-drop-qa");
    std::fs::create_dir_all(&output).unwrap();
    let original = doc(&app).composite().pixels;
    let red = [0.92, 0.14, 0.09, 1.0];
    send(&mut app, EditAction::Color(red));
    // A short click and keyboard activation retain the existing colour picker.
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(EditAction::ColorPicker(ColorTarget::Foreground))
        )
    });
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_some()
    );
    send(&mut app, EditAction::ColorCancel);
    for scale in [1.0, 1.5] {
        super::panel_sizing::resize_target(&mut app, window, &target, 1280.0, scale);
        {
            let mut viewer = app.world_mut().resource_mut::<ImageViewer>();
            viewer.zoom = Zoom::Scale(3.5);
            viewer.pan = Vec2::new(32.0, -20.0);
        }
        warm(&mut app, 4);
        let image = image_rect(&mut app);
        let point = pixel_point(image, Vec2::new(20.5, 30.5));
        let revision = doc(&app).revision;
        lift(&mut app, window, ColorTarget::Foreground);
        move_pointer(&mut app, window, point);
        warm(&mut app, 10);
        assert!(visible(&mut app, "Точка заливки"));
        assert!(app.world().resource::<ImageViewer>().brush_cursor.is_none());
        assert_eq!(
            doc(&app).revision,
            revision,
            "Picking up and hovering never paints"
        );
        let bead = point + Vec2::new(19.0, -25.0);
        capture_when(
            &mut app,
            &target,
            &output.join(format!("01-drag-{scale}.png")),
            |image| {
                let centre = (bead * scale).as_uvec2();
                let mut red_pixels = 0;
                for y in centre.y.saturating_sub(14)..(centre.y + 14).min(image.height()) {
                    for x in centre.x.saturating_sub(14)..(centre.x + 14).min(image.width()) {
                        let p = image.get_pixel(x, y).0;
                        if p[0] > 150 && p[1] < 125 && p[2] < 110 {
                            red_pixels += 1;
                        }
                    }
                }
                red_pixels > 80
            },
        );
        mouse(&mut app, window, ButtonState::Released);
        assert!(visible(&mut app, "Волна заливки"));
        assert!(!visible(&mut app, "Переносимый цвет"));
        assert!(
            app.world()
                .resource::<ImageViewer>()
                .edit
                .color_picker
                .is_none()
        );
        let pixels = doc(&app).composite();
        let at =
            |x: u32| &pixels.pixels[((32 * 96 + x) * 4) as usize..((32 * 96 + x) * 4 + 4) as usize];
        assert_eq!(at(20), [235, 36, 23, 255]);
        assert_eq!(at(48), [0, 0, 0, 255]);
        assert_eq!(
            at(75),
            [255; 4],
            "The black divider must isolate the other white region"
        );
        capture_when(
            &mut app,
            &target,
            &output.join(format!("02-drop-{scale}.png")),
            |_| true,
        );
        send(&mut app, EditAction::Undo);
        assert_eq!(
            doc(&app).composite().pixels,
            original,
            "The entire fill has exactly one undo step"
        );
        assert_eq!(app.world().resource::<ImageViewer>().edit.color(), red);
        warm(&mut app, 35);
        assert!(!visible(&mut app, "Волна заливки"));
    }
    // Esc, a drop over UI/pasteboard, and releasing back on the swatch do no work.
    for destination in [0, 1, 2] {
        let revision = doc(&app).revision;
        lift(&mut app, window, ColorTarget::Foreground);
        let point = if destination == 2 {
            source(&mut app, ColorTarget::Foreground)
        } else {
            Vec2::new(440.0, 150.0)
        };
        move_pointer(&mut app, window, point);
        if destination == 0 {
            escape(&mut app, window);
        }
        mouse(&mut app, window, ButtonState::Released);
        warm(&mut app, 20);
        assert_eq!(doc(&app).revision, revision);
        assert_eq!(doc(&app).composite().pixels, original);
        assert!(
            app.world()
                .resource::<ImageViewer>()
                .edit
                .color_picker
                .is_none()
        );
        assert!(!visible(&mut app, "Переносимый цвет"));
    }
    // The rear swatch carries its own white colour without swapping the active brush colour.
    send(&mut app, EditAction::Color([0.1, 0.25, 0.9, 1.0]));
    send(&mut app, EditAction::SwapColors);
    let foreground = app.world().resource::<ImageViewer>().edit.color();
    lift(&mut app, window, ColorTarget::Background);
    let image = image_rect(&mut app);
    move_pointer(&mut app, window, pixel_point(image, Vec2::new(75.0, 30.0)));
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.color(),
        foreground
    );
    let pixels = doc(&app).composite();
    assert_eq!(
        &pixels.pixels[(30 * 96 + 75) * 4..(30 * 96 + 75) * 4 + 4],
        &[26, 64, 230, 255]
    );
    send(&mut app, EditAction::Undo);
    // A locked layer rejects the destination, and switching tabs clears the bead.
    send(
        &mut app,
        EditAction::Layers(edit::layers::LayerAction::Lock("all")),
    );
    let revision = doc(&app).revision;
    lift(&mut app, window, ColorTarget::Foreground);
    move_pointer(&mut app, window, pixel_point(image, Vec2::new(20.0, 30.0)));
    assert!(!visible(&mut app, "Точка заливки"));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 20);
    assert_eq!(doc(&app).revision, revision);
    send(
        &mut app,
        EditAction::Layers(edit::layers::LayerAction::Lock("all")),
    );
    lift(&mut app, window, ColorTarget::Foreground);
    app.world_mut().write_message(Action::Open(0));
    warm(&mut app, 3);
    assert!(!visible(&mut app, "Переносимый цвет"));
    assert!(!app.world().resource::<ImageViewer>().edit.color_dragging);
    mouse(&mut app, window, ButtonState::Released);
}
