use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::MouseButtonInput,
    },
    input_focus::FocusCause,
};
use edit::{ColorTarget, EditAction};

fn send(app: &mut App, action: EditAction) {
    app.world_mut().write_message(Action::ImageEdit(action));
    app.update();
}

#[test]
fn color_picker_commits_only_on_confirm_and_keeps_document_history_clean() {
    let fixture = Fixture::new();
    let original = std::fs::read(fixture.0.join("b-transparent.png")).unwrap();
    let mut app = fixture.app();
    let index = file(&app, "b-transparent.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    send(&mut app, EditAction::Edit);
    for _ in 0..150 {
        app.update();
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.0, 1.0, 1.0];
    send(&mut app, EditAction::ColorPicker(ColorTarget::Foreground));
    send(&mut app, EditAction::ColorDraft(red));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.color(),
        [0.0, 0.0, 0.0, 1.0]
    );
    send(&mut app, EditAction::ColorCancel);
    // A queued draft input must not leak through after cancellation.
    send(&mut app, EditAction::ColorDraft(blue));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.color(),
        [0.0, 0.0, 0.0, 1.0]
    );
    send(&mut app, EditAction::ColorPicker(ColorTarget::Foreground));
    send(&mut app, EditAction::ColorDraft(red));
    send(&mut app, EditAction::ColorConfirm);
    assert_eq!(app.world().resource::<ImageViewer>().edit.color(), red);
    send(&mut app, EditAction::ColorPicker(ColorTarget::Background));
    send(&mut app, EditAction::ColorDraft(blue));
    send(&mut app, EditAction::ColorConfirm);
    {
        let state = &app.world().resource::<ImageViewer>().edit;
        assert_eq!(state.color(), red);
        assert_eq!(state.background(), blue);
    }
    send(&mut app, EditAction::SwapColors);
    {
        let state = &app.world().resource::<ImageViewer>().edit;
        assert_eq!(state.color(), blue);
        assert_eq!(state.background(), red);
    }
    send(&mut app, EditAction::ResetColors);
    let viewer = app.world().resource::<ImageViewer>();
    assert_eq!(viewer.edit.color(), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(viewer.edit.background(), [1.0; 4]);
    let document = viewer.edit.document(viewer.tab).unwrap();
    assert!(!document.dirty() && !document.can_undo());
    assert_eq!(
        std::fs::read(fixture.0.join("b-transparent.png")).unwrap(),
        original
    );
}

fn field(app: &mut App, label: &str) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &AccessibleLabel)>()
        .iter(world)
        .find_map(|(e, name)| (name.0 == label).then_some(e))
        .unwrap()
}

fn rect(app: &mut App, label: &str) -> Rect {
    let e = field(app, label);
    let world = app.world();
    let node = world.get::<ComputedNode>(e).unwrap();
    let transform = world.get::<UiGlobalTransform>(e).unwrap();
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}

fn edit_text(app: &mut App, label: &str, value: &str) {
    let e = field(app, label);
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(e, FocusCause::Auto);
    app.world_mut()
        .get_mut::<EditableText>(e)
        .unwrap()
        .editor_mut()
        .set_text(value);
    frame(app);
    frame(app);
}

fn key(app: &mut App, window: Entity, key_code: KeyCode, logical_key: Key) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key: logical_key.clone(),
            state,
            text: None,
            repeat: false,
        });
        frame(app);
        frame(app);
    }
}

fn drag(app: &mut App, window: Entity, start: Vec2, end: Vec2) {
    for (point, state) in [(start, ButtonState::Pressed), (end, ButtonState::Released)] {
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(point));
        let input = MouseButtonInput {
            window,
            button: MouseButton::Left,
            state,
        };
        app.world_mut().write_message(input);
        app.world_mut()
            .write_message(bevy::window::WindowEvent::MouseButtonInput(input));
        frame(app);
        frame(app);
    }
}

fn click(app: &mut App, window: Entity, action: impl Fn(EditAction) -> bool) {
    click_action(
        app,
        window,
        |a| matches!(a, Action::ImageEdit(a) if action(a)),
    );
}

#[test]
#[ignore = "requires GPU; uses toolbar swatches, numeric/HEX input, colour plane, palette and modal confirmation"]
fn gpu_color_picker_swatches_fields_and_cancel_preserve_canvas() {
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
        .write_message(Action::ImageEdit(EditAction::Edit));
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
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-color-picker-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("01-toolbar.png"), |_| true);
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
    click(&mut app, window, |a| {
        matches!(a, EditAction::ColorPicker(ColorTarget::Foreground))
    });
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_some()
    );
    edit_text(&mut app, "HEX цвета", "FF8800");
    {
        let state = &app.world().resource::<ImageViewer>().edit;
        assert_eq!(state.color_picker.as_ref().unwrap().hex(), "FF8800");
        assert_eq!(state.color(), [0.0, 0.0, 0.0, 1.0]);
    }
    edit_text(&mut app, "HEX цвета", "ZZZZZZ");
    assert!(
        !app.world()
            .resource::<ImageViewer>()
            .edit
            .available(EditAction::ColorConfirm, Some(0))
    );
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_some()
    );
    edit_text(&mut app, "HEX цвета", "FF8800");
    edit_text(&mut app, "Оттенок H · 0–360", "120");
    assert!(
        (app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .as_ref()
            .unwrap()
            .hsv
            .hue
            - 120.0)
            .abs()
            < 0.1
    );
    edit_text(&mut app, "Зелёный G · 0–255", "180");
    assert!(
        (app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .as_ref()
            .unwrap()
            .draft[1]
            * 255.0
            - 180.0)
            .abs()
            < 0.1
    );
    edit_text(&mut app, "HEX цвета", "FF8800");
    let palette_len = app.world().resource::<ImageViewer>().edit.swatches.len();
    click(&mut app, window, |a| matches!(a, EditAction::AddSwatch));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.swatches.len(),
        palette_len + 1
    );
    capture_when(
        &mut app,
        &target,
        &output.join("02-foreground-dialog.png"),
        |_| true,
    );
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_none()
    );
    let foreground = app.world().resource::<ImageViewer>().edit.color();
    assert!((foreground[1] * 255.0 - 136.0).abs() < 0.1);
    click(&mut app, window, |a| {
        matches!(a, EditAction::ColorPicker(ColorTarget::Background))
    });
    let plane = rect(&mut app, "Поле цвета");
    let hue = rect(&mut app, "Полоса оттенка");
    drag(
        &mut app,
        window,
        hue.center(),
        hue.min + hue.size() * Vec2::new(0.5, 1.0 / 3.0),
    );
    drag(
        &mut app,
        window,
        plane.min + plane.size() * 0.5,
        plane.min + plane.size() * Vec2::new(0.8, 0.2),
    );
    {
        let state = &app.world().resource::<ImageViewer>().edit;
        let picker = state.color_picker.as_ref().unwrap();
        assert!((picker.hsv.hue - 240.0).abs() < 1.0);
        assert!((picker.hsv.saturation - 0.8).abs() < 0.01);
        assert!((picker.hsv.value - 0.8).abs() < 0.01);
        assert_eq!(state.background(), [1.0; 4]);
    }
    let screenshot = capture_when(
        &mut app,
        &target,
        &output.join("03-background-dialog.png"),
        |_| true,
    );
    // The SV image must fill the same rectangle that receives pointer input.
    let top_left = screenshot
        .get_pixel((plane.min.x + 8.0) as u32, (plane.min.y + 8.0) as u32)
        .0;
    let top_right = screenshot
        .get_pixel((plane.max.x - 8.0) as u32, (plane.min.y + 8.0) as u32)
        .0;
    assert!(
        top_left[0] > 200 && top_left[1] > 200 && top_left[2] > 200,
        "{top_left:?}"
    );
    assert!(
        top_right[0] < 35 && top_right[1] < 35 && top_right[2] > 200,
        "{top_right:?}"
    );
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    {
        let state = &app.world().resource::<ImageViewer>().edit;
        assert!(state.color_picker.is_none());
        assert_eq!(state.color(), foreground);
        assert_eq!(state.background(), [1.0; 4]);
    }
    click(&mut app, window, |a| {
        matches!(a, EditAction::ColorPicker(ColorTarget::Background))
    });
    edit_text(&mut app, "HEX цвета", "1B2C3D");
    click(&mut app, window, |a| matches!(a, EditAction::ColorCancel));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.background(),
        [1.0; 4]
    );
    click(&mut app, window, |a| {
        matches!(a, EditAction::ColorPicker(ColorTarget::Background))
    });
    edit_text(&mut app, "HEX цвета", "1B2C3D");
    click(&mut app, window, |a| matches!(a, EditAction::ColorConfirm));
    let background = app.world().resource::<ImageViewer>().edit.background();
    assert!((background[0] * 255.0 - 27.0).abs() < 0.1);
    click(&mut app, window, |a| matches!(a, EditAction::SwapColors));
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.color(),
        background
    );
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.background(),
        foreground
    );
    key(&mut app, window, KeyCode::KeyD, Key::Character("d".into()));
    let viewer = app.world().resource::<ImageViewer>();
    assert_eq!(viewer.edit.color(), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(viewer.edit.background(), [1.0; 4]);
    let doc = viewer.edit.document(viewer.tab).unwrap();
    assert!(
        !doc.dirty() && !doc.can_undo(),
        "Picking a colour must not paint behind the dialog"
    );
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
    println!("Colour picker captures: {}", output.display());
}
