use super::*;
use crate::editor::{
    photo_document::PhotoTool,
    selection_outline::render_tests::{capture_when, frame},
};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::MouseButtonInput,
    },
    window::{CursorMoved, CustomCursor, WindowEvent},
};

fn pixels(app: &App) -> Vec<u8> {
    let viewer = app.world().resource::<ImageViewer>();
    app.world()
        .resource::<Assets<Image>>()
        .get(viewer.image.as_ref().unwrap())
        .unwrap()
        .data
        .clone()
        .unwrap()
}

fn assert_erased_row(data: &[u8]) {
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(
                data[(y * 32 + x) * 4 + 3],
                if y == 12 && (10..=18).contains(&x) {
                    0
                } else {
                    255
                },
                "Pixel eraser touched the wrong cell or smoothed {x},{y}"
            );
        }
    }
}

#[test]
#[ignore = "requires GPU; verifies pixel eraser controls, snapped cursor, live alpha, undo and saved pixels"]
fn gpu_pixel_eraser_snaps_cursor_and_erases_whole_pixels() {
    use edit::{EditAction, EraserMode};

    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(32, 32, image::Rgba([225, 110, 65, 255]))
        .save(fixture.0.join("eraser.png"))
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
    let index = file(&app, "eraser.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..30 {
        frame(&mut app);
    }
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(EditAction::Menu))
    });
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(EditAction::Edit))
    });
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
    {
        let world = app.world_mut();
        assert!(
            !world
                .query::<&Control>()
                .iter(world)
                .any(|control| matches!(
                    control.action,
                    Action::ImageEdit(EditAction::PaintMenu | EditAction::EraserMenu)
                )),
            "The corner hints must not be independent buttons"
        );
    }
    // Left clicks on the decorative corners reach the normal tool buttons.
    for eraser in [true, false] {
        click_control_offset(
            &mut app,
            window,
            |a| {
                if eraser {
                    matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser)))
                } else {
                    matches!(a, Action::ImageEdit(EditAction::DrawingTool))
                }
            },
            MouseButton::Left,
            Vec2::splat(0.4),
        );
        let state = &app.world().resource::<ImageViewer>().edit;
        assert_eq!(
            state.tool,
            if eraser {
                PhotoTool::Eraser
            } else {
                PhotoTool::Brush
            }
        );
        assert!(
            !state.paint_menu,
            "Left clicking the hint must not open a flyout"
        );
    }
    // Right clicks anywhere on a tool, including its corner, open its flyout.
    // Switching flyouts and dismissing them do not select a tool or edit pixels.
    click_control_offset(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser))),
        MouseButton::Right,
        Vec2::splat(0.4),
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .active(EditAction::EraserMenu, None)
    );
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.tool,
        PhotoTool::Brush
    );
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(EditAction::DrawingTool)),
        MouseButton::Right,
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .active(EditAction::PaintMenu, None)
    );
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser))),
        MouseButton::Right,
    );
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state,
            text: None,
            repeat: false,
        });
        frame(&mut app);
    }
    assert!(!app.world().resource::<ImageViewer>().edit.paint_menu);
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.tool,
        PhotoTool::Brush
    );
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser))),
        MouseButton::Right,
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .active(EditAction::EraserMenu, None)
    );
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-eraser-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("00-eraser-flyout.png"),
        |_| true,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(EditAction::EraserMode(EraserMode::Pixel))
        )
    });
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    {
        let viewer = app.world().resource::<ImageViewer>();
        assert_eq!(viewer.edit.tool, PhotoTool::Eraser);
        assert!(!viewer.edit.paint_menu);
        assert_eq!(viewer.edit.eraser_mode, EraserMode::Pixel);
        assert_eq!(viewer.edit.size, 1.0);
        assert!(!viewer.edit.document(Some(id)).unwrap().dirty());
        assert!(!viewer.edit.document(Some(id)).unwrap().can_undo());
    }
    {
        let world = app.world_mut();
        let mut sliders = world.query::<(&AccessibleLabel, Has<InteractionDisabled>)>();
        assert!(
            sliders
                .iter(world)
                .any(|(label, disabled)| label.0 == "Жёсткость" && disabled)
        );
    }
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(16.0);
    for _ in 0..6 {
        frame(&mut app);
    }
    let rect = {
        let picture = app.world().resource::<ViewerUi>().picture;
        let node = app.world().get::<ComputedNode>(picture).unwrap();
        let transform = app.world().get::<UiGlobalTransform>(picture).unwrap();
        Rect::from_center_size(transform.affine().translation, node.size())
    };
    assert_eq!(rect.size(), Vec2::splat(512.0));
    let move_to = |app: &mut App, source: Vec2| {
        let position = rect.min + source * 16.0;
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
    };
    let button = |app: &mut App, state| {
        let input = MouseButtonInput {
            window,
            button: MouseButton::Left,
            state,
        };
        app.world_mut()
            .write_message(WindowEvent::MouseButtonInput(input));
        app.world_mut().write_message(input);
    };
    move_to(&mut app, Vec2::new(10.2, 12.8));
    frame(&mut app);
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(rect.min + Vec2::new(10.5, 12.5) * 16.0),
        "Cursor must enclose the exact cell erased on click"
    );
    let CursorIcon::Custom(CustomCursor::Image(cursor)) =
        app.world().get::<CursorIcon>(window).unwrap()
    else {
        panic!("Expected transparent native cursor with snapped overlay");
    };
    let native = app
        .world()
        .resource::<Assets<Image>>()
        .get(&cursor.handle)
        .unwrap();
    assert_eq!(native.texture_descriptor.size.width, 1);
    assert_eq!(native.data.as_ref().unwrap(), &[0; 4]);
    let center = rect.min + Vec2::new(10.5, 12.5) * 16.0;
    capture_when(
        &mut app,
        &target,
        &output.join("01-pixel-mode.png"),
        |image| image.get_pixel(center.x as u32, center.y as u32)[0] > 200,
    );
    button(&mut app, ButtonState::Pressed);
    frame(&mut app);
    assert_eq!(
        pixels(&app)[(12 * 32 + 10) * 4 + 3],
        0,
        "Erase in the input frame"
    );
    for point in [Vec2::new(14.1, 12.1), Vec2::new(18.7, 12.8)] {
        move_to(&mut app, point);
    }
    frame(&mut app);
    let live = pixels(&app);
    assert_erased_row(&live);
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    assert_eq!(
        pixels(&app),
        live,
        "Release must not alter the live pixel stroke"
    );
    capture_when(&mut app, &target, &output.join("02-erased.png"), |image| {
        image.get_pixel(center.x as u32, center.y as u32)[0] < 50
    });
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Undo));
    frame(&mut app);
    assert!(pixels(&app).chunks_exact(4).all(|p| p[3] == 255));
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .can_undo(),
        "One gesture must create one undo step"
    );
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Redo));
    frame(&mut app);
    assert_eq!(pixels(&app), live);
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(EditAction::Save))
    });
    for _ in 0..120 {
        frame(&mut app);
        if !app.world().resource::<ImageViewer>().edit.busy() {
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
    let saved = image::open(fixture.0.join("eraser.png"))
        .unwrap()
        .into_rgba8();
    assert_erased_row(saved.as_raw());
    let native = photocraft_format::load_path(&fixture.0.join("eraser.pcraft")).unwrap();
    let surface = native.layers[0].surface().unwrap();
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(
                surface.rgba(x, y)[3],
                if y == 12 && (10..=18).contains(&x) {
                    0.0
                } else {
                    1.0
                }
            );
        }
    }
    // Settings changes do not add document history or change any rendered pixel.
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Size(3.7)));
    frame(&mut app);
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 4.0);
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser))),
        MouseButton::Right,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(EditAction::EraserMode(EraserMode::Brush))
        )
    });
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 24.0);
    assert!(!app.world().resource::<ImageViewer>().edit.paint_menu);
    click_control(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser))),
        MouseButton::Right,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(EditAction::EraserMode(EraserMode::Pixel))
        )
    });
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 4.0);
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(EditAction::DrawingTool))
    });
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(EditAction::Tool(PhotoTool::Eraser)))
    });
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.eraser_mode,
        EraserMode::Pixel
    );
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 4.0);
    assert_eq!(pixels(&app), live);
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .dirty()
    );
    println!("Pixel eraser captures: {}", output.display());
}
