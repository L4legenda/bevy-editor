use super::*;

#[test]
fn numeric_scrub_respects_ranges_integer_steps_and_decimal_commas() {
    let number = Number::integer(3, 256);
    assert_eq!(number.text(number.constrain(-100.0, 32.0), false), "3");
    assert_eq!(number.text(number.constrain(999.0, 32.0), false), "256");
    assert_eq!(number.text(17.6, false), "18");
    let decimal = Number::decimal(0.0001, 10000.0, 0.01, 4);
    assert_eq!(decimal.text(1.2345, true), "1,2345");
    let scale = Number::scene(Field::Scale(0));
    assert!(scale.constrain(0.0, 0.01) < 0.0);
    assert!(scale.constrain(0.0, -0.01) > 0.0);
    assert_eq!(scale.text(-0.00001, false), "0.00");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires GPU; drags real scene, model, primitive, image, brush and color numeric inputs"]
fn gpu_numeric_scrub_all_numeric_inputs_live_cancel_and_one_undo() {
    use crate::editor::{
        Action,
        browser::Browser,
        model_primitives::Primitive,
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture, mouse, move_pointer},
        ui::image_viewer::edit::ColorTarget,
        ui::model_viewer::editing::EditAction,
        ui::{ImageEditAction, ImageViewer, ModelEditingState, ModelViewAction},
    };
    use bevy::input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    };
    use std::path::PathBuf;

    fn warm(app: &mut App, frames: usize) {
        for _ in 0..frames {
            frame(app);
        }
    }
    fn center(app: &App, entity: Entity) -> Vec2 {
        let node = app.world().get::<ComputedNode>(entity).unwrap();
        assert!(!node.is_empty(), "Numeric input must be visible");
        app.world()
            .get::<UiGlobalTransform>(entity)
            .unwrap()
            .affine()
            .translation
            * node.inverse_scale_factor()
    }
    fn find(app: &mut App, label: &str) -> Entity {
        let world = app.world_mut();
        world
            .query::<(Entity, &AccessibleLabel)>()
            .iter(world)
            .find(|(_, name)| name.0 == label)
            .unwrap_or_else(|| panic!("Missing input: {label}"))
            .0
    }
    fn value(app: &App, entity: Entity) -> f32 {
        app.world()
            .get::<EditableText>(entity)
            .unwrap()
            .value()
            .to_string()
            .replace(',', ".")
            .parse()
            .unwrap()
    }
    fn set(app: &mut App, entity: Entity, value: &str) {
        app.world_mut()
            .get_mut::<EditableText>(entity)
            .unwrap()
            .editor_mut()
            .set_text(value);
        warm(app, 3);
    }
    fn begin(app: &mut App, window: Entity, entity: Entity) -> Vec2 {
        assert!(app.world().get::<Number>(entity).is_some());
        let point = center(app, entity);
        move_pointer(app, window, point);
        mouse(app, window, ButtonState::Pressed);
        point
    }
    fn key(app: &mut App, window: Entity, code: KeyCode, logical: Key, state: ButtonState) {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code: code,
            logical_key: logical,
            state,
            text: None,
            repeat: false,
        });
        warm(app, 2);
    }
    fn escape(app: &mut App, window: Entity) {
        key(
            app,
            window,
            KeyCode::Escape,
            Key::Escape,
            ButtonState::Pressed,
        );
        key(
            app,
            window,
            KeyCode::Escape,
            Key::Escape,
            ButtonState::Released,
        );
    }
    fn send(app: &mut App, action: Action) {
        app.world_mut().write_message(action);
        warm(app, 5);
    }

    let (mut app, window, target) = fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    send(&mut app, Action::Select(Selection::Camera(0)));
    let field = {
        let world = app.world_mut();
        world
            .query_filtered::<(Entity, &Field), With<TextInput>>()
            .iter(world)
            .find(|(_, field)| **field == Field::Position(0))
            .unwrap()
            .0
    };
    let before = app.world().resource::<Editor>().camera(0).pose.position[0];
    let undo = app.world().resource::<Editor>().undo.len();
    let original_cursor = CursorIcon::from(SystemCursorIcon::Crosshair);
    app.world_mut()
        .entity_mut(window)
        .insert(original_cursor.clone());
    let point = begin(&mut app, window, field);
    move_pointer(&mut app, window, point + Vec2::X);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<Editor>().camera(0).pose.position[0],
        before
    );
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(field));
    let point = begin(&mut app, window, field);
    move_pointer(&mut app, window, point - Vec2::new(80.0, 0.0));
    assert!(app.world().resource::<Scrub>().active(field));
    assert_eq!(
        app.world().get::<TextReadWriteMode>(field),
        Some(&TextReadWriteMode::Static)
    );
    assert!(
        (app.world().resource::<Editor>().camera(0).pose.position[0] - (before - 4.0)).abs() < 0.01
    );
    assert_eq!(app.world().resource::<Editor>().undo.len(), undo);
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::EwResize))
    );
    key(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Pressed,
    );
    move_pointer(&mut app, window, point - Vec2::new(100.0, 0.0));
    assert!((value(&app, field) - (before - 4.1)).abs() < 0.01);
    key(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Released,
    );
    key(
        &mut app,
        window,
        KeyCode::ControlLeft,
        Key::Control,
        ButtonState::Pressed,
    );
    move_pointer(&mut app, window, point - Vec2::new(110.0, 0.0));
    assert!((value(&app, field) - (before - 9.1)).abs() < 0.01);
    key(
        &mut app,
        window,
        KeyCode::ControlLeft,
        Key::Control,
        ButtonState::Released,
    );
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<Editor>().undo.len(), undo + 1);
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&original_cursor)
    );
    assert_eq!(
        app.world().get::<TextReadWriteMode>(field),
        Some(&TextReadWriteMode::Editable)
    );
    send(&mut app, Action::Undo);
    assert_eq!(
        app.world().resource::<Editor>().camera(0).pose.position[0],
        before
    );
    let point = begin(&mut app, window, field);
    move_pointer(&mut app, window, point - Vec2::new(60.0, 0.0));
    escape(&mut app, window);
    assert_eq!(
        app.world().resource::<Editor>().camera(0).pose.position[0],
        before
    );
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<Editor>().undo.len(), undo);

    send(&mut app, Action::ViewModel(0));
    for _ in 0..180 {
        frame(&mut app);
        if app
            .world()
            .resource::<ModelEditingState>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    warm(&mut app, 8);
    let model_field = find(&mut app, "Модель: X");
    let position = app.world().resource::<ModelEditingState>().documents[&0]
        .active()
        .unwrap()
        .transform
        .translation
        .x;
    let model_undo = app.world().resource::<ModelEditingState>().documents[&0]
        .undo
        .len();
    let point = begin(&mut app, window, model_field);
    move_pointer(&mut app, window, point - Vec2::new(70.0, 0.0));
    assert!(
        (app.world().resource::<ModelEditingState>().documents[&0]
            .active()
            .unwrap()
            .transform
            .translation
            .x
            - (position - 0.7))
            .abs()
            < 0.001
    );
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<ModelEditingState>().documents[&0]
            .undo
            .len(),
        model_undo + 1
    );
    send(
        &mut app,
        Action::ModelView(ModelViewAction::Edit(EditAction::Undo)),
    );
    let point = begin(&mut app, window, model_field);
    move_pointer(&mut app, window, point - Vec2::new(40.0, 0.0));
    escape(&mut app, window);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<ModelEditingState>().documents[&0]
            .active()
            .unwrap()
            .transform
            .translation
            .x,
        position
    );
    assert_eq!(
        app.world().resource::<ModelEditingState>().documents[&0]
            .undo
            .len(),
        model_undo
    );

    app.world_mut().resource_mut::<Editor>().drawer = true;
    warm(&mut app, 24);
    send(&mut app, Action::BrowserCreatePrimitive(Primitive::Cube));
    let size = find(&mut app, "Параметр фигуры: Размер X, м");
    let name = find(&mut app, "Название новой 3D-модели");
    assert!(app.world().get::<Number>(name).is_none());
    let point = begin(&mut app, window, size);
    move_pointer(&mut app, window, point + Vec2::new(120.0, 0.0));
    assert!((value(&app, size) - 3.2).abs() < 0.0001);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/numeric-scrub-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("01-primitive-drag.png"),
        |_| true,
    );
    escape(&mut app, window);
    assert_eq!(value(&app, size), 2.0);
    assert!(app.world().resource::<Editor>().browser_popup);
    mouse(&mut app, window, ButtonState::Released);
    escape(&mut app, window);
    send(&mut app, Action::BrowserCreatePrimitive(Primitive::Torus));
    let minor = find(&mut app, "Параметр фигуры: Малый радиус, м");
    let point = begin(&mut app, window, minor);
    move_pointer(&mut app, window, point + Vec2::new(200.0, 0.0));
    mouse(&mut app, window, ButtonState::Released);
    assert!(value(&app, minor) < 1.0);
    escape(&mut app, window);
    send(&mut app, Action::BrowserNewImage);
    let width = find(&mut app, "Ширина изображения");
    let height = find(&mut app, "Высота изображения");
    set(&mut app, height, "8192");
    let point = begin(&mut app, window, width);
    key(
        &mut app,
        window,
        KeyCode::ControlLeft,
        Key::Control,
        ButtonState::Pressed,
    );
    move_pointer(&mut app, window, point + Vec2::new(200.0, 0.0));
    key(
        &mut app,
        window,
        KeyCode::ControlLeft,
        Key::Control,
        ButtonState::Released,
    );
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(value(&app, width), 2048.0);
    escape(&mut app, window);

    struct Files(PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let path = self.0.canonicalize().unwrap();
            assert!(path.starts_with(std::env::temp_dir().canonicalize().unwrap()));
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("bevy-numeric-scrub-")
            );
            std::fs::remove_dir_all(path).unwrap();
        }
    }
    let files = Files(std::env::temp_dir().join(format!(
        "bevy-numeric-scrub-{}",
        bevy::asset::uuid::Uuid::new_v4()
    )));
    std::fs::create_dir_all(&files.0).unwrap();
    image::RgbaImage::from_pixel(32, 32, image::Rgba([120, 130, 140, 255]))
        .save(files.0.join("image.png"))
        .unwrap();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = files.0.clone();
            editor.project.asset_root = files.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let image = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|file| file.path == std::path::Path::new("image.png"))
        .unwrap();
    send(&mut app, Action::ViewImage(image));
    warm(&mut app, 25);
    send(&mut app, Action::ImageEdit(ImageEditAction::Edit));
    for _ in 0..150 {
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
    send(
        &mut app,
        Action::ImageEdit(ImageEditAction::Tool(
            crate::editor::photo_document::PhotoTool::Eraser,
        )),
    );
    send(&mut app, Action::ImageEdit(ImageEditAction::SizeMenu));
    let brush = find(&mut app, "Размер кисти · px · Shift + движение мыши");
    let original = app.world().resource::<ImageViewer>().edit.size;
    let point = begin(&mut app, window, brush);
    move_pointer(&mut app, window, point + Vec2::new(20.0, 0.0));
    assert!((app.world().resource::<ImageViewer>().edit.size - (original + 5.0)).abs() < 0.01);
    key(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Pressed,
    );
    move_pointer(&mut app, window, point + Vec2::new(40.0, 0.0));
    assert!((app.world().resource::<ImageViewer>().edit.size - (original + 5.5)).abs() < 0.01);
    key(
        &mut app,
        window,
        KeyCode::ShiftLeft,
        Key::Shift,
        ButtonState::Released,
    );
    assert!(app.world().resource::<Scrub>().active(brush));
    escape(&mut app, window);
    assert_eq!(app.world().resource::<ImageViewer>().edit.size, original);
    assert_eq!(value(&app, brush), original);
    assert!(app.world().resource::<ImageViewer>().edit.size_menu);
    mouse(&mut app, window, ButtonState::Released);
    escape(&mut app, window);
    send(
        &mut app,
        Action::ImageEdit(ImageEditAction::ColorPicker(ColorTarget::Foreground)),
    );
    let red = find(&mut app, "Красный R · 0–255");
    let original = value(&app, red);
    let point = begin(&mut app, window, red);
    move_pointer(&mut app, window, point + Vec2::new(80.0, 0.0));
    assert_eq!(value(&app, red), (original + 80.0).min(255.0));
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .as_ref()
            .unwrap()
            .draft[0]
            > 0.1
    );
    escape(&mut app, window);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(value(&app, red), original);
    assert!(
        (app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .as_ref()
            .unwrap()
            .draft[0]
            - original / 255.0)
            .abs()
            < 0.0001
    );
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .color_picker
            .is_some()
    );
    capture_when(
        &mut app,
        &target,
        &output.join("02-color-fields.png"),
        |_| true,
    );
}
