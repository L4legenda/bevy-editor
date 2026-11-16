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
    input_focus::FocusCause,
};

const SIZE_FIELD: &str = "Размер кисти · px · Shift + движение мыши";

fn move_cpu(app: &mut App, window: Entity, point: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

#[test]
fn brush_size_shift_motion_and_escape_do_not_paint_or_pan() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    app.add_systems(
        Update,
        tool_cursor::cursor.after(crate::editor::handle_actions),
    );
    let index = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::ActualSize));
    app.update();
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let origin = Vec2::new(600.0, 350.0);
    move_cpu(&mut app, window, origin);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    move_cpu(&mut app, window, origin);
    assert!(app.world().resource::<ImageViewer>().edit.resizing_brush());
    move_cpu(&mut app, window, origin + Vec2::new(20.0, 60.0));
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 64.0);
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(origin)
    );
    move_cpu(&mut app, window, origin + Vec2::new(20.0, -60.0));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.size,
        64.0,
        "Vertical motion must not change the tip"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ShiftLeft);
    move_cpu(&mut app, window, origin + Vec2::new(20.0, -60.0));
    assert_eq!(
        app.world().get::<Window>(window).unwrap().cursor_position(),
        Some(origin),
        "Finishing the adjustment must return the pointer to its press origin"
    );
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(origin)
    );
    // Releasing Shift before the mouse must not create the start or tail of a stroke.
    move_cpu(&mut app, window, origin + Vec2::splat(10.0));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    move_cpu(&mut app, window, origin);
    move_cpu(&mut app, window, origin);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftRight);
    move_cpu(&mut app, window, origin);
    move_cpu(&mut app, window, origin - Vec2::new(10.0, 0.0));
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 44.0);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    move_cpu(&mut app, window, origin - Vec2::new(10.0, 0.0));
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 64.0);
    assert!(!app.world().resource::<ImageViewer>().edit.resizing_brush());
    assert_eq!(
        app.world().get::<Window>(window).unwrap().cursor_position(),
        Some(origin),
        "Cancelling must also return the pointer to its original position"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    for (tool, pixel_eraser) in [(PhotoTool::Pencil, false), (PhotoTool::Eraser, true)] {
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Tool(tool)));
        if pixel_eraser {
            app.world_mut()
                .write_message(Action::ImageEdit(edit::EditAction::EraserMode(
                    edit::EraserMode::Pixel,
                )));
        }
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Size(1.0)));
        app.world_mut()
            .write_message(Action::ImageView(ImageAction::ActualSize));
        app.update();
        app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(8.0);
        move_cpu(&mut app, window, origin);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        move_cpu(&mut app, window, origin);
        let center = app.world().resource::<ImageViewer>().brush_cursor_position;
        move_cpu(&mut app, window, origin + Vec2::new(3.2, 0.0));
        assert_eq!(app.world().resource::<ImageViewer>().edit.size, 2.0);
        assert_eq!(
            app.world().resource::<ImageViewer>().brush_cursor_position,
            center,
            "Pixel-size parity must not move the resize preview centre"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        move_cpu(&mut app, window, origin + Vec2::new(3.2, 0.0));
        assert_eq!(
            app.world().get::<Window>(window).unwrap().cursor_position(),
            Some(origin),
            "Pixel tools restore the actual pointer origin, independently of grid snapping"
        );
    }
    // Leaving the OS window while Shift is held must retain the origin until
    // release; there is no current cursor position to use for the return.
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    move_cpu(&mut app, window, origin);
    move_cpu(&mut app, window, origin + Vec2::new(8.0, 20.0));
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    app.update();
    assert!(app.world().resource::<ImageViewer>().edit.resizing_brush());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ShiftLeft);
    app.update();
    assert_eq!(
        app.world().get::<Window>(window).unwrap().cursor_position(),
        Some(origin)
    );
    assert!(!app.world().resource::<ImageViewer>().edit.resizing_brush());

    // Losing focus cancels adjustment ownership without moving the pointer
    // back from another application.
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    move_cpu(&mut app, window, origin);
    let away = origin + Vec2::new(10.0, 20.0);
    move_cpu(&mut app, window, away);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    app.update();
    assert!(!app.world().resource::<ImageViewer>().edit.resizing_brush());
    assert_eq!(
        app.world().get::<Window>(window).unwrap().cursor_position(),
        Some(away)
    );
    let viewer = app.world().resource::<ImageViewer>();
    assert_eq!(viewer.pan, Vec2::ZERO);
    let doc = viewer.edit.document(viewer.tab).unwrap();
    assert!(!doc.dirty() && !doc.can_undo());
}

fn entity(app: &mut App, label: &str) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &AccessibleLabel)>()
        .iter(world)
        .find_map(|(e, l)| (l.0 == label).then_some(e))
        .unwrap()
}

fn bounds(app: &mut App, entity: Entity) -> Rect {
    let world = app.world();
    let node = world.get::<ComputedNode>(entity).unwrap();
    Rect::from_center_size(
        world
            .get::<UiGlobalTransform>(entity)
            .unwrap()
            .affine()
            .translation
            * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}

fn key_state(
    app: &mut App,
    window: Entity,
    key_code: KeyCode,
    logical_key: Key,
    state: ButtonState,
) {
    app.world_mut().write_message(KeyboardInput {
        window,
        key_code,
        logical_key,
        state,
        text: None,
        repeat: false,
    });
    frame(app);
    frame(app);
}

fn key(app: &mut App, window: Entity, code: KeyCode, key: Key) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        key_state(app, window, code, key.clone(), state);
    }
}

fn pointer(app: &mut App, window: Entity, point: Vec2, state: Option<ButtonState>) {
    use bevy::{
        camera::RenderTarget,
        picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
        ui::IsDefaultUiCamera,
        window::{CursorMoved, WindowEvent},
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
    let previous = app
        .world()
        .get::<Window>(window)
        .unwrap()
        .cursor_position()
        .unwrap_or(point);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.world_mut()
        .write_message(WindowEvent::CursorMoved(CursorMoved {
            window,
            position: point,
            delta: None,
        }));
    let action = if let Some(state) = state {
        let mouse = MouseButtonInput {
            window,
            button: MouseButton::Left,
            state,
        };
        app.world_mut().write_message(mouse);
        app.world_mut()
            .write_message(WindowEvent::MouseButtonInput(mouse));
        if state == ButtonState::Pressed {
            PointerAction::Press(PointerButton::Primary)
        } else {
            PointerAction::Release(PointerButton::Primary)
        }
    } else {
        PointerAction::Move {
            delta: point - previous,
        }
    };
    app.world_mut().write_message(PointerInput {
        pointer_id: PointerId::Mouse,
        location: Location {
            target,
            position: point,
        },
        action,
    });
    frame(app);
    frame(app);
}

fn type_size(app: &mut App, field: Entity, value: &str) {
    open_size(app);
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(field, FocusCause::Auto);
    app.world_mut()
        .get_mut::<EditableText>(field)
        .unwrap()
        .editor_mut()
        .set_text(value);
    frame(app);
    frame(app);
}

fn open_size(app: &mut App) {
    if !app.world().resource::<ImageViewer>().edit.size_menu {
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap();
        click_action(app, window, |a| {
            matches!(a, Action::ImageEdit(edit::EditAction::SizeMenu))
        });
    }
}

fn assert_centered(image: &image::RgbaImage, center: Vec2, diameter: f32) {
    let color = image.get_pixel(center.x as u32, center.y as u32).0;
    let radius = diameter * 0.5;
    let pixels: Vec<_> = ((center.x - radius - 8.0) as u32..=(center.x + radius + 8.0) as u32)
        .filter(|&x| {
            let p = image.get_pixel(x, center.y as u32).0;
            (0..3).any(|c| p[c].abs_diff(color[c]) > 50)
        })
        .collect();
    let left = *pixels.first().expect("Missing left rim") as f32;
    let right = *pixels.last().expect("Missing right rim") as f32;
    assert!(
        ((left + right) * 0.5 - center.x).abs() <= 1.5,
        "Preview centre moved: {left}..{right}, anchor {center:?}"
    );
    assert!(left < center.x - radius + 2.0 && right > center.x + radius - 2.0);
}

#[test]
#[ignore = "requires GPU; changes size through the toolbar, typed decimals and Shift, checks cursor and untouched pixels"]
fn gpu_brush_size_toolbar_and_shift_update_tip_without_painting() {
    let fixture = Fixture::new();
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
    let index = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..25 {
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
    for _ in 0..6 {
        frame(&mut app);
    }
    let field = entity(&mut app, SIZE_FIELD);
    let slider = entity(&mut app, "Размер кисти");
    let button = entity(&mut app, "Настройки кисти");
    let settings = entity(&mut app, "Параметры кисти");
    let hardness = entity(&mut app, "Жёсткость");
    let opacity = entity(&mut app, "Непрозрачность кисти");
    assert!(bounds(&mut app, hardness).is_empty());
    assert!(bounds(&mut app, opacity).is_empty());
    let button_rect = bounds(&mut app, button);
    assert!(button_rect.min.y > IMAGE_TOP && button_rect.max.y <= IMAGE_TOP + 44.0);
    assert!(
        button_rect.width() <= 136.0,
        "The toolbar control must stay compact"
    );
    open_size(&mut app);
    assert!(bounds(&mut app, field).min.y > button_rect.max.y);
    assert!(
        bounds(&mut app, slider).width() >= 220.0,
        "Use a wide slider in the popover"
    );
    let settings_rect = bounds(&mut app, settings);
    for setting in [hardness, opacity] {
        let setting_rect = bounds(&mut app, setting);
        assert!(settings_rect.contains(setting_rect.min));
        assert!(settings_rect.contains(setting_rect.max));
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-brush-size-qa");
    std::fs::create_dir_all(&output).unwrap();
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
    for setting in [hardness, opacity] {
        let track = bounds(&mut app, setting);
        let start = track.min + track.size() * Vec2::new(0.8, 0.5);
        let end = track.min + track.size() * Vec2::new(0.4, 0.5);
        pointer(&mut app, window, start, None);
        pointer(&mut app, window, start, Some(ButtonState::Pressed));
        pointer(&mut app, window, end, None);
        pointer(&mut app, window, end, Some(ButtonState::Released));
        assert!(
            app.world().resource::<ImageViewer>().edit.size_menu,
            "Brush settings must stay open while adjusting each slider"
        );
    }
    {
        let viewer = app.world().resource::<ImageViewer>();
        assert!((0.3..0.5).contains(&viewer.edit.hardness));
        assert!((0.3..0.5).contains(&viewer.edit.opacity));
    }
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(!app.world().resource::<ImageViewer>().edit.size_menu);
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert!(bounds(&mut app, hardness).is_empty());
    assert!(bounds(&mut app, opacity).is_empty());
    type_size(&mut app, field, "45,84");
    assert!((app.world().resource::<ImageViewer>().edit.size - 45.84).abs() < 0.01);
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert_eq!(
        app.world()
            .get::<EditableText>(field)
            .unwrap()
            .value()
            .to_string(),
        "45.84"
    );
    // The last character and Enter may be delivered in one frame by the OS.
    type_size(&mut app, field, "");
    for (code, logical, text) in [
        (
            KeyCode::Digit6,
            Key::Character("64".into()),
            Some("64".into()),
        ),
        (KeyCode::Enter, Key::Enter, None),
    ] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: code,
            logical_key: logical,
            state: ButtonState::Pressed,
            text,
            repeat: false,
        });
    }
    frame(&mut app);
    frame(&mut app);
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 64.0);
    key_state(
        &mut app,
        window,
        KeyCode::Enter,
        Key::Enter,
        ButtonState::Released,
    );
    key_state(
        &mut app,
        window,
        KeyCode::Digit6,
        Key::Character("6".into()),
        ButtonState::Released,
    );
    type_size(&mut app, field, "45.84");
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    type_size(&mut app, field, "0");
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert_eq!(
        app.world()
            .get::<EditableText>(field)
            .unwrap()
            .value()
            .to_string(),
        "45.84"
    );
    open_size(&mut app);
    capture_when(
        &mut app,
        &target,
        &output.join("02-size-popover.png"),
        |_| true,
    );
    let slider_rect = bounds(&mut app, slider);
    let start = slider_rect.min + slider_rect.size() * Vec2::new(0.2, 0.5);
    let end = slider_rect.min + slider_rect.size() * Vec2::new(0.65, 0.5);
    pointer(&mut app, window, start, None);
    pointer(&mut app, window, start, Some(ButtonState::Pressed));
    pointer(&mut app, window, end, None);
    pointer(&mut app, window, end, Some(ButtonState::Released));
    assert!(app.world().resource::<ImageViewer>().edit.size > 100.0);
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(!app.world().resource::<ImageViewer>().edit.size_menu);
    assert!(app.world().resource::<InputFocus>().get().is_none());
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Size(24.0)));
    app.world_mut()
        .write_message(Action::ImageView(ImageAction::ActualSize));
    frame(&mut app);
    pointer(&mut app, window, button_rect.center(), None);
    app.world_mut()
        .write_message(bevy::input::mouse::MouseWheel {
            phase: bevy::input::touch::TouchPhase::Moved,
            window,
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 1.0,
        });
    frame(&mut app);
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 25.0);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Size(24.0)));
    frame(&mut app);
    pointer(&mut app, window, Vec2::new(520.0, 420.0), None);
    let old_cursor = app.world().resource::<ImageViewer>().brush_cursor;
    // Shift on the canvas also dismisses the popover and releases its text focus.
    open_size(&mut app);
    pointer(&mut app, window, Vec2::new(520.0, 420.0), None);
    key_state(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Pressed,
    );
    pointer(&mut app, window, Vec2::new(540.0, 420.0), None);
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 64.0);
    assert!(!app.world().resource::<ImageViewer>().edit.size_menu);
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(Vec2::new(520.0, 420.0))
    );
    assert_ne!(
        app.world().resource::<ImageViewer>().brush_cursor,
        old_cursor
    );
    assert_eq!(
        app.world()
            .get::<EditableText>(field)
            .unwrap()
            .value()
            .to_string(),
        "64"
    );
    let preview = capture_when(
        &mut app,
        &target,
        &output.join("01-toolbar-and-shift.png"),
        |_| true,
    );
    assert_centered(&preview, Vec2::new(520.0, 420.0), 64.0);
    if let Some(CursorIcon::Custom(bevy::window::CustomCursor::Image(cursor))) =
        app.world().get::<CursorIcon>(window)
    {
        let image = app
            .world()
            .resource::<Assets<Image>>()
            .get(&cursor.handle)
            .unwrap();
        assert_eq!(
            image.width(),
            1,
            "A moving native ring must not appear over the anchored preview"
        );
        assert_eq!(image.data.as_ref().unwrap()[3], 0);
    } else {
        panic!("Expected transparent native cursor during resizing");
    }
    key_state(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Released,
    );
    assert_eq!(
        app.world().get::<Window>(window).unwrap().cursor_position(),
        Some(Vec2::new(520.0, 420.0))
    );
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(Vec2::new(520.0, 420.0))
    );
    super::panel_sizing::resize_target(&mut app, window, &target, 1280.0, 2.0);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Size(24.0)));
    frame(&mut app);
    pointer(&mut app, window, Vec2::new(520.0, 420.0), None);
    key_state(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Pressed,
    );
    pointer(&mut app, window, Vec2::new(540.0, 420.0), None);
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, 104.0);
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(Vec2::new(520.0, 420.0))
    );
    let dense = capture_when(
        &mut app,
        &target,
        &output.join("03-centred-resize-2x.png"),
        |_| true,
    );
    assert_centered(&dense, Vec2::new(1040.0, 840.0), 104.0);
    key_state(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Released,
    );
    assert_eq!(
        app.world().get::<Window>(window).unwrap().cursor_position(),
        Some(Vec2::new(520.0, 420.0))
    );
    assert_eq!(
        app.world()
            .get::<Window>(window)
            .unwrap()
            .physical_cursor_position(),
        Some(Vec2::new(1040.0, 840.0))
    );
    assert_eq!(
        app.world().resource::<ImageViewer>().brush_cursor_position,
        Some(Vec2::new(520.0, 420.0))
    );
    let viewer = app.world().resource::<ImageViewer>();
    assert!(!viewer.edit.resizing_brush());
    assert_eq!(viewer.pan, Vec2::ZERO);
    let doc = viewer.edit.document(viewer.tab).unwrap();
    assert!(!doc.dirty() && !doc.can_undo());
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(viewer.image.as_ref().unwrap())
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &original
    );
    println!("Brush size capture: {}", output.display());
}
