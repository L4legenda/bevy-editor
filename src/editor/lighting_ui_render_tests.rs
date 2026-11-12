//! Full editor UI captures without an OS window or writes to project assets.
#![cfg(not(target_arch = "wasm32"))]

use std::{path::PathBuf, time::Duration};

use bevy::{
    camera::RenderTarget,
    input::{ButtonState, mouse::MouseButtonInput},
    picking::{
        input::PointerInputSettings,
        pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
    },
    prelude::*,
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{Extent3d, TextureFormat, TextureUsages},
    },
    time::TimeUpdateStrategy,
    ui::{CalculatedClip, IsDefaultUiCamera},
    window::{CursorMoved, ExitCondition, PrimaryWindow},
    winit::WinitPlugin,
};

use crate::{
    asset_paths::AssetRedirectPlugin,
    editor::{
        Action, Editor, EditorPlugin, Field, Selection,
        lighting::{EnvironmentField, LightField, LightingAction},
        selection_outline::render_tests::{CapturedImage, capture_when, frame},
        ui,
    },
    lighting::LightPreset,
    project::EditorProject,
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 900;

#[derive(Resource)]
struct Canvas(Handle<Image>);

fn retarget_ui(
    canvas: Res<Canvas>,
    mut cameras: Query<(&mut RenderTarget, &mut Camera), With<IsDefaultUiCamera>>,
) {
    let mut count = 0;
    for (mut target, mut camera) in &mut cameras {
        *target = RenderTarget::Image(canvas.0.clone().into());
        camera.order = 100;
        count += 1;
    }
    assert_eq!(
        count, 1,
        "The real editor must create its default UI camera"
    );
}

pub(in crate::editor) fn fixture() -> (App, Entity, Handle<Image>) {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugins(AssetRedirectPlugin::new(&project.asset_root).unwrap())
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: project.asset_root.to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    close_when_requested: false,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<PipelinedRenderingPlugin>(),
        )
        .add_plugins(EditorPlugin::new(project).unwrap())
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .init_resource::<CapturedImage>()
        .insert_resource(PointerInputSettings {
            is_mouse_enabled: false,
            is_touch_enabled: false,
        })
        .add_systems(
            Startup,
            retarget_ui.after(crate::editor::setup).after(ui::setup),
        );

    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                resolution: (WIDTH, HEIGHT).into(),
                focused: true,
                ..default()
            },
        ))
        .id();
    let mut image = Image::new_target_texture(WIDTH, HEIGHT, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.insert_resource(Canvas(target.clone()));
    app.finish();
    app.cleanup();
    for _ in 0..480 {
        frame(&mut app);
        if app.world().resource::<Editor>().ready {
            break;
        }
    }
    assert!(
        app.world().resource::<Editor>().ready,
        "The actual example scene failed to load: {}",
        app.world().resource::<Editor>().status
    );
    warm(&mut app, 16);
    (app, window, target)
}

fn warm(app: &mut App, frames: usize) {
    for _ in 0..frames {
        frame(app);
    }
}

fn labeled_rect(app: &mut App, wanted: &str) -> Rect {
    let world = app.world_mut();
    let mut query = world.query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>();
    let (_, node, transform) = query
        .iter(world)
        .find(|(label, _, _)| label.0 == wanted)
        .unwrap_or_else(|| panic!("Missing actual UI control: {wanted}"));
    assert!(
        node.size().min_element() > 0.0,
        "Control is hidden: {wanted}"
    );
    let center = transform.affine().translation;
    Rect::from_corners(center - node.size() * 0.5, center + node.size() * 0.5)
}

fn visible_field(app: &mut App, wanted: Field) -> bool {
    let world = app.world_mut();
    let mut query = world.query::<(
        &Field,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
    )>();
    query.iter(world).any(|(field, node, transform, clip)| {
        *field == wanted
            && node.size().min_element() > 0.0
            && clip.is_none_or(|clip| clip.contains_point(transform.affine().translation))
    })
}

fn scroll_to_field(app: &mut App, wanted: Field) {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &Field)>();
    let (entity, _) = query
        .iter(world)
        .find(|(_, field)| **field == wanted)
        .expect("Expected lighting control is missing");
    scroll_to_entity(app, entity);
}

fn scroll_to_label(app: &mut App, wanted: &str) {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &AccessibleLabel)>();
    let (entity, _) = query
        .iter(world)
        .find(|(_, label)| label.0 == wanted)
        .expect("Expected color control is missing");
    scroll_to_entity(app, entity);
}

fn scroll_to_entity(app: &mut App, entity: Entity) {
    let world = app.world_mut();
    let center = world
        .get::<UiGlobalTransform>(entity)
        .unwrap()
        .affine()
        .translation;
    let mut current = entity;
    let scroller = loop {
        let Some(parent) = world.get::<ChildOf>(current).map(ChildOf::parent) else {
            // Splitters live outside the scrollable inspector.
            return;
        };
        if world.get::<ui::Scroller>(parent).is_some() {
            break parent;
        }
        current = parent;
    };
    let node = world.get::<ComputedNode>(scroller).unwrap();
    let transform = world.get::<UiGlobalTransform>(scroller).unwrap();
    let top = transform.affine().translation.y - node.size().y * 0.5;
    let bottom = transform.affine().translation.y + node.size().y * 0.5;
    let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
    let delta = if center.y < top + 40.0 {
        center.y - top - 40.0
    } else {
        (center.y - bottom + 45.0).max(0.0)
    };
    let mut position = world.get_mut::<ScrollPosition>(scroller).unwrap();
    position.y = (position.y + delta).clamp(0.0, max);
    warm(app, 4);
}

fn move_pointer(app: &mut App, window: Entity, position: Vec2) {
    let old = app
        .world()
        .get::<Window>(window)
        .unwrap()
        .cursor_position()
        .unwrap_or(position);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(position));
    app.world_mut().write_message(CursorMoved {
        window,
        position,
        delta: None,
    });
    pointer(
        app,
        window,
        position,
        PointerAction::Move {
            delta: position - old,
        },
    );
    warm(app, 2);
}

fn mouse(app: &mut App, window: Entity, state: ButtonState) {
    app.world_mut().write_message(MouseButtonInput {
        window,
        button: MouseButton::Left,
        state,
    });
    let position = app
        .world()
        .get::<Window>(window)
        .unwrap()
        .cursor_position()
        .unwrap();
    pointer(
        app,
        window,
        position,
        if state == ButtonState::Pressed {
            PointerAction::Press(PointerButton::Primary)
        } else {
            PointerAction::Release(PointerButton::Primary)
        },
    );
    warm(app, 2);
}

fn pointer(app: &mut App, window: Entity, position: Vec2, action: PointerAction) {
    let target = {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>();
        cameras
            .single(world)
            .unwrap()
            .normalize(Some(window))
            .unwrap()
    };
    app.world_mut().write_message(PointerInput {
        pointer_id: PointerId::Mouse,
        location: Location { target, position },
        action,
    });
}

fn drag_control(app: &mut App, window: Entity, label: &str, start: Vec2, end: Vec2) {
    scroll_to_label(app, label);
    let rect = labeled_rect(app, label);
    move_pointer(app, window, rect.min + rect.size() * start);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, rect.min + rect.size() * end);
    mouse(app, window, ButtonState::Released);
}

fn ui_pixels(image: &image::RgbaImage) -> bool {
    ui_pixels_from(image, WIDTH - ui::SIDE as u32)
}

fn ui_pixels_from(image: &image::RgbaImage, left: u32) -> bool {
    let mut text_pixels = 0;
    for y in ui::TOP as u32..image.height() - ui::FOOT as u32 {
        for x in left..image.width() {
            let [r, g, b, _] = image.get_pixel(x, y).0;
            if r > 170 && g > 160 && b > 150 {
                text_pixels += 1;
            }
        }
    }
    text_pixels > 180
}

#[test]
#[ignore = "requires GPU adapter; writes editor layout screenshots under /tmp"]
fn gpu_sidebar_resize_changes_panes_and_keeps_the_rendered_viewport_aligned() {
    let output =
        PathBuf::from("/tmp").join(format!("bevy-editor-sidebar-resize-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Sidebar resize GPU artifacts: {}", output.display());
    let (mut app, window, target) = fixture();
    let initial = app.world().resource::<Editor>().edits.clone();
    let original_side = labeled_rect(&mut app, "Правая панель");
    let original_inspector = labeled_rect(&mut app, "Инспектор свойств");
    capture_when(
        &mut app,
        &target,
        &output.join("01-default-layout.png"),
        ui_pixels,
    );

    drag_control(
        &mut app,
        window,
        "Изменить ширину правой панели",
        Vec2::splat(0.5),
        Vec2::new(0.5 - 180.0 / 6.0, 0.5),
    );
    let side = labeled_rect(&mut app, "Правая панель");
    assert!((side.width() - original_side.width() - 180.0).abs() < 1.0);
    let viewport = {
        let editor = app.world().resource::<Editor>();
        ui::viewport_rect(app.world().get::<Window>(window).unwrap(), &editor.sidebar)
    };
    assert!(
        (viewport.max.x - side.min.x).abs() < 1.0,
        "The scene and sidebar must meet at the same border"
    );
    let surface = &app.world().resource::<crate::editor::viewport::Surface>().0;
    let size = app
        .world()
        .resource::<Assets<Image>>()
        .get(surface)
        .unwrap()
        .texture_descriptor
        .size;
    assert_eq!(size.width, viewport.width().round() as u32);
    assert_eq!(size.height, viewport.height().round() as u32);
    capture_when(
        &mut app,
        &target,
        &output.join("02-wide-sidebar.png"),
        |image| ui_pixels_from(image, side.min.x as u32),
    );

    drag_control(
        &mut app,
        window,
        "Изменить высоту инспектора",
        Vec2::splat(0.5),
        Vec2::new(0.5, 0.5 - 200.0 / 6.0),
    );
    let inspector = labeled_rect(&mut app, "Инспектор свойств");
    assert!((inspector.height() - original_inspector.height() - 200.0).abs() < 1.0);
    let editor = app.world().resource::<Editor>();
    assert_eq!(
        editor.edits, initial,
        "Layout changes must not edit scene data"
    );
    assert!(editor.undo.is_empty());
    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    warm(&mut app, 4);
    assert!(
        (labeled_rect(&mut app, "Инспектор свойств").height() - inspector.height()).abs() < 1.0,
        "Selecting lighting must preserve the user's panel split"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("03-taller-inspector.png"),
        |image| ui_pixels_from(image, side.min.x as u32),
    );

    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(1000.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: 1000,
            height: 620,
            depth_or_array_layers: 1,
        });
    warm(&mut app, 5);
    let small = labeled_rect(&mut app, "Инспектор свойств");
    assert!(small.height() < inspector.height());
    assert!(small.max.y <= 620.0 - ui::FOOT + 1.0);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(WIDTH as f32, HEIGHT as f32);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        });
    warm(&mut app, 5);
    assert!(
        (labeled_rect(&mut app, "Инспектор свойств").height() - inspector.height()).abs() < 1.0
    );
    assert!((labeled_rect(&mut app, "Правая панель").width() - side.width()).abs() < 1.0);
    capture_when(
        &mut app,
        &target,
        &output.join("04-restored-layout.png"),
        |image| ui_pixels_from(image, side.min.x as u32),
    );

    // Passing between independent splitters must not leave a resize cursor
    // installed over the scene after both controls release it.
    use bevy::window::{CursorIcon, SystemCursorIcon};
    move_pointer(&mut app, window, Vec2::new(100.0, 200.0));
    let original_cursor = app.world().get::<CursorIcon>(window).cloned();
    app.world_mut().write_message(Action::Drawer);
    warm(&mut app, 20);
    let width_grip = labeled_rect(&mut app, "Изменить ширину правой панели");
    move_pointer(
        &mut app,
        window,
        Vec2::new(width_grip.center().x, ui::TOP + 50.0),
    );
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::EwResize))
    );
    let drawer_top =
        HEIGHT as f32 - ui::FOOT - app.world().resource::<ui::DrawerMotion>().visible_height;
    move_pointer(&mut app, window, Vec2::new(400.0, drawer_top + 3.0));
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::NsResize))
    );
    move_pointer(&mut app, window, Vec2::new(100.0, 200.0));
    assert_eq!(
        app.world().get::<CursorIcon>(window).cloned(),
        original_cursor
    );
}

#[test]
#[ignore = "requires GPU adapter; writes full editor UI screenshots under /tmp"]
fn gpu_lighting_menu_and_inspectors_fit_the_real_editor() {
    let output =
        PathBuf::from("/tmp").join(format!("bevy-editor-lighting-ui-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Lighting UI GPU artifacts: {}", output.display());
    let (mut app, window, target) = fixture();

    app.world_mut().write_message(Action::LightMenu);
    warm(&mut app, 8);
    assert!(app.world().resource::<ui::LightingUi>().is_menu_open());
    capture_when(
        &mut app,
        &target,
        &output.join("01-add-light-menu.png"),
        ui_pixels,
    );
    let menu = labeled_rect(&mut app, "Добавление света и схемы освещения");
    let add = labeled_rect(&mut app, "+ Свет");
    assert!(add.min.x >= WIDTH as f32 - ui::SIDE && add.min.y >= ui::TOP);
    assert!(
        menu.min.y >= add.max.y + 4.0,
        "The light menu must open below the scene-tree button"
    );
    assert!(menu.min.x >= 0.0 && menu.max.x <= WIDTH as f32);
    assert!(menu.min.y >= ui::TOP - 3.0 && menu.max.y <= HEIGHT as f32 - ui::FOOT);
    let mut cards = Vec::new();
    for label in ["Точечный", "Прожектор", "Солнце", "Панель"] {
        let card = labeled_rect(&mut app, &format!("Добавить: {label}"));
        assert!(
            card.min.x > menu.min.x && card.max.x < menu.max.x,
            "Type card spills outside the popover: {label}"
        );
        assert!(card.min.y >= menu.min.y && card.max.y < menu.max.y);
        for previous in &cards {
            let previous: &Rect = previous;
            assert!(
                previous.max.x <= card.min.x || card.max.x <= previous.min.x,
                "Light type cards overlap"
            );
        }
        cards.push(card);
    }

    app.world_mut()
        .write_message(Action::Lighting(LightingAction::Add(LightPreset::Spot)));
    warm(&mut app, 8);
    assert!(!app.world().resource::<ui::LightingUi>().is_menu_open());
    assert!(matches!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Light(_))
    ));
    let name = labeled_rect(&mut app, "Имя источника света");
    assert!(name.min.x >= WIDTH as f32 - ui::SIDE && name.max.x <= WIDTH as f32);
    capture_when(
        &mut app,
        &target,
        &output.join("02-spot-inspector.png"),
        ui_pixels,
    );
    let color = labeled_rect(&mut app, "Цвет источника: насыщенность и яркость");
    assert!(color.min.x >= WIDTH as f32 - ui::SIDE && color.max.x <= WIDTH as f32);
    assert!(visible_field(&mut app, Field::Light(LightField::Intensity)));
    let intensity_before = app.world().resource::<Editor>().edits.lights[0].intensity;
    let history_before = app.world().resource::<Editor>().undo.len();
    drag_control(
        &mut app,
        window,
        "Интенсивность",
        Vec2::new(0.45, 0.5),
        Vec2::new(0.6, 0.5),
    );
    let editor = app.world().resource::<Editor>();
    assert!(
        editor.edits.lights[0].intensity > intensity_before * 2.0,
        "The real slider must edit the light"
    );
    assert_eq!(
        editor.undo.len(),
        history_before + 1,
        "A pointer drag must be one undo action"
    );
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 2);
    assert_eq!(
        app.world().resource::<Editor>().edits.lights[0].intensity,
        intensity_before
    );

    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetColor([255; 3])));
    warm(&mut app, 2);
    drag_control(
        &mut app,
        window,
        "Цвет источника: оттенок",
        Vec2::new(0.66, 0.5),
        Vec2::new(0.66, 0.5),
    );
    drag_control(
        &mut app,
        window,
        "Цвет источника: насыщенность и яркость",
        Vec2::new(0.6, 0.4),
        Vec2::new(0.8, 0.2),
    );
    let editor = app.world().resource::<Editor>();
    let [red, green, blue] = editor.edits.lights[0].color;
    assert!(
        blue > red + 0.4 && blue > green + 0.4,
        "The hue bar and color plane must select blue"
    );
    assert!(editor.edits.lights[0].temperature.is_none());
    capture_when(
        &mut app,
        &target,
        &output.join("02-color-picker-edited.png"),
        ui_pixels,
    );
    scroll_to_field(&mut app, Field::Light(LightField::OuterAngle));
    assert!(visible_field(
        &mut app,
        Field::Light(LightField::InnerAngle)
    ));
    assert!(visible_field(
        &mut app,
        Field::Light(LightField::OuterAngle)
    ));
    capture_when(
        &mut app,
        &target,
        &output.join("03-spot-cone-settings.png"),
        ui_pixels,
    );

    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    warm(&mut app, 8);
    scroll_to_field(&mut app, Field::Environment(EnvironmentField::Exposure));
    assert!(visible_field(
        &mut app,
        Field::Environment(EnvironmentField::Exposure)
    ));
    assert!(!visible_field(
        &mut app,
        Field::Light(LightField::OuterAngle)
    ));
    let environment_image = capture_when(
        &mut app,
        &target,
        &output.join("04-environment-inspector.png"),
        ui_pixels,
    );
    let plane = labeled_rect(&mut app, "Цвет фонового света: насыщенность и яркость");
    let sample_y = (plane.min.y + plane.height() * 0.1) as u32;
    let left = environment_image
        .get_pixel((plane.min.x + 4.0) as u32, sample_y)
        .0;
    let right = environment_image
        .get_pixel((plane.max.x - 4.0) as u32, sample_y)
        .0;
    assert!(
        left[0] > 200 && left[1] > 180 && left[2] > 180,
        "The color plane must fill its pointer area, including the white edge"
    );
    assert!(
        right[0] > 200 && right[1] < 40 && right[2] < 40,
        "The color plane must fill its pointer area, including the saturated edge"
    );
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::ToggleFog));
    warm(&mut app, 3);
    let ambient = app.world().resource::<Editor>().environment().ambient_color;
    drag_control(
        &mut app,
        window,
        "Цвет тумана: оттенок",
        Vec2::new(0.66, 0.5),
        Vec2::new(0.66, 0.5),
    );
    drag_control(
        &mut app,
        window,
        "Цвет тумана: насыщенность и яркость",
        Vec2::new(0.7, 0.2),
        Vec2::new(0.8, 0.1),
    );
    let environment = app.world().resource::<Editor>().environment();
    let [red, green, blue] = environment.fog.as_ref().unwrap().color;
    assert!(blue > red + 0.4 && blue > green + 0.4);
    assert_eq!(
        environment.ambient_color, ambient,
        "Fog color must not edit ambient light"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-fog-color-picker.png"),
        ui_pixels,
    );

    app.world_mut()
        .write_message(Action::Lighting(LightingAction::Add(LightPreset::Softbox)));
    app.world_mut().write_message(Action::LightAdvanced);
    warm(&mut app, 8);
    scroll_to_field(&mut app, Field::Light(LightField::Height));
    assert!(visible_field(&mut app, Field::Light(LightField::Width)));
    assert!(visible_field(&mut app, Field::Light(LightField::Height)));
    assert!(!visible_field(
        &mut app,
        Field::Light(LightField::ShadowNear)
    ));
    capture_when(
        &mut app,
        &target,
        &output.join("05-rect-light-inspector.png"),
        ui_pixels,
    );

    // The real popover must remain inside the window and scroll on short layouts.
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(WIDTH as f32, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: WIDTH,
            height: 620,
            depth_or_array_layers: 1,
        });
    app.world_mut().write_message(Action::LightMenu);
    warm(&mut app, 8);
    let short_menu = labeled_rect(&mut app, "Добавление света и схемы освещения");
    assert!(short_menu.max.y <= 620.0 - ui::FOOT);
    let last_choice = labeled_rect(
        &mut app,
        "Показывать значки и контуры источников в редакторе. Это настройка интерфейса, она не меняет освещение сцены.",
    );
    assert!(
        last_choice.min.y >= short_menu.min.y && last_choice.max.y <= short_menu.max.y,
        "All light choices should remain reachable in a short window"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("06-short-window-menu.png"),
        ui_pixels,
    );
}

#[test]
#[ignore = "requires GPU adapter; captures real drag placement and Retina coordinates under /tmp"]
fn gpu_dragged_model_preview_tracks_cursor_and_release_matches_the_preview() {
    use crate::editor::{
        asset_placement::{PlacementGhost, PlacementPreview},
        browser::Browser,
        viewport::EditorCamera,
    };
    let output =
        PathBuf::from("/tmp").join(format!("bevy-editor-placement-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Placement GPU artifacts: {}", output.display());
    let (mut app, window, target) = fixture();
    let initial = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::Drawer);
    let models = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == std::path::Path::new("models"))
        .unwrap();
    app.world_mut().write_message(Action::Browse(models));
    warm(&mut app, 24);
    let card_label = "Выбрать модель block.glb · Блок";

    fn cursor_at(app: &mut App, window: Entity, point: Vec3) -> Vec2 {
        let world = app.world_mut();
        let mut query = world.query_filtered::<(&Camera, &GlobalTransform), With<EditorCamera>>();
        let (camera, transform) = query.single(world).unwrap();
        let cursor = camera.world_to_viewport(transform, point).unwrap() + Vec2::new(0.0, ui::TOP);
        let editor = world.resource::<Editor>();
        assert!(
            ui::scene_input_rect(
                world.get::<Window>(window).unwrap(),
                world.resource::<ui::DrawerMotion>(),
                &editor.sidebar
            )
            .contains(cursor),
            "Fixture drop point is behind UI: {cursor:?}"
        );
        cursor
    }
    fn begin(app: &mut App, window: Entity, label: &str, cursor: Vec2) {
        let scale = app.world().get::<Window>(window).unwrap().scale_factor();
        let card = labeled_rect(app, label).center() / scale;
        move_pointer(app, window, card);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, cursor);
        for _ in 0..240 {
            frame(app);
            if app
                .world()
                .resource::<PlacementPreview>()
                .position()
                .is_some()
            {
                break;
            }
        }
        assert!(
            app.world().resource::<ui::AssetDrag>().active(),
            "The real file-card gesture did not start"
        );
        assert!(
            app.world()
                .resource::<PlacementPreview>()
                .position()
                .is_some(),
            "Preview geometry did not load: {}",
            app.world().resource::<Editor>().status
        );
    }
    fn no_ghosts(app: &mut App) {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<PlacementGhost>>();
        assert_eq!(query.iter(world).count(), 0);
    }
    let first = cursor_at(&mut app, window, Vec3::new(-3.0, 0.0, -5.0));
    begin(&mut app, window, card_label, first);
    let a = app
        .world()
        .resource::<PlacementPreview>()
        .position()
        .unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("00-initial-preview.png"),
        ui_pixels,
    );
    {
        let world = app.world_mut();
        let mut roots = world.query_filtered::<Entity, With<PlacementGhost>>();
        let root = roots.single(world).unwrap();
        let mut meshes = world.query::<(Entity, &MeshMaterial3d<StandardMaterial>)>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        let mut count = 0;
        for (entity, handle) in meshes.iter(world) {
            let mut ancestor = Some(entity);
            let mut belongs_to_preview = false;
            while let Some(entity) = ancestor {
                if entity == root {
                    belongs_to_preview = true;
                    break;
                }
                ancestor = world.get::<ChildOf>(entity).map(ChildOf::parent);
            }
            if belongs_to_preview {
                let material = materials.get(&handle.0).unwrap();
                let color = material.base_color.to_srgba();
                assert!(
                    color.red > color.green && color.green > color.blue,
                    "The GLB's warm material must stay warm"
                );
                assert!((color.alpha - 0.8).abs() < 1e-6);
                assert!(!material.unlit);
                count += 1;
            }
        }
        assert!(count > 0);
    }
    assert!(
        a.abs_diff_eq(Vec3::new(-3.0, 0.0, -5.0), 0.003),
        "Unexpected placement {a:?}; contact {:?}",
        app.world().resource::<PlacementPreview>().contact()
    );
    assert_eq!(app.world().resource::<Editor>().edits, initial);
    capture_when(
        &mut app,
        &target,
        &output.join("01-preview-left.png"),
        ui_pixels,
    );
    let second = cursor_at(&mut app, window, Vec3::new(3.0, 0.0, -5.0));
    move_pointer(&mut app, window, second);
    warm(&mut app, 4);
    let b = app
        .world()
        .resource::<PlacementPreview>()
        .position()
        .unwrap();
    assert!(
        a.distance(b) > 3.5,
        "The preview must move with the pointer"
    );
    assert!(b.abs_diff_eq(Vec3::new(3.0, 0.0, -5.0), 0.003));
    assert_eq!(app.world().resource::<Editor>().edits, initial);
    assert!(app.world().resource::<Editor>().undo.is_empty());
    capture_when(
        &mut app,
        &target,
        &output.join("02-preview-right.png"),
        ui_pixels,
    );
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    {
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits.models.len(), 1);
        assert!(Vec3::from(editor.edits.models[0].pose.position).abs_diff_eq(b, 0.003));
        assert!(editor.edits.models[0].bounds.is_some());
        assert_eq!(editor.undo.len(), 1);
    }
    no_ghosts(&mut app);
    capture_when(&mut app, &target, &output.join("03-placed.png"), ui_pixels);

    let top = cursor_at(&mut app, window, Vec3::new(-2.0, 1.0, 0.0));
    begin(&mut app, window, card_label, top);
    assert!(
        (app.world()
            .resource::<PlacementPreview>()
            .contact()
            .unwrap()
            .y
            - 1.0)
            .abs()
            < 0.003
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-on-object.png"),
        ui_pixels,
    );
    app.world_mut()
        .write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: bevy::input::keyboard::Key::Escape,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
    frame(&mut app);
    app.world_mut()
        .write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: bevy::input::keyboard::Key::Escape,
            state: ButtonState::Released,
            text: None,
            repeat: false,
            window,
        });
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    no_ghosts(&mut app);

    // Image targets and pointer coordinates use independent scale factors.
    // Recreate Retina while preserving the same logical editor dimensions.
    {
        let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
        window.resolution.set_scale_factor_override(Some(2.0));
        window.resolution.set(WIDTH as f32, HEIGHT as f32);
    }
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: WIDTH * 2,
            height: HEIGHT * 2,
            depth_or_array_layers: 1,
        });
    {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&mut RenderTarget, With<IsDefaultUiCamera>>();
        *cameras.single_mut(world).unwrap() =
            RenderTarget::Image(bevy::camera::ImageRenderTarget {
                handle: target.clone(),
                scale_factor: 2.0,
            });
    }
    app.world_mut().resource_mut::<Editor>().snap = true;
    warm(&mut app, 12);
    let retina = cursor_at(&mut app, window, Vec3::new(-2.13, 0.0, -4.78));
    begin(&mut app, window, card_label, retina);
    let expected = app
        .world()
        .resource::<PlacementPreview>()
        .position()
        .unwrap();
    assert!(
        expected.abs_diff_eq(Vec3::new(-2.25, 0.0, -4.75), 0.003),
        "Retina drop must retain logical coordinates and grid snapping: {expected:?}"
    );
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 4);
    let placed = app.world().resource::<Editor>().edits.models.clone();
    assert_eq!(placed.len(), 2);
    assert!(Vec3::from(placed[1].pose.position).abs_diff_eq(expected, 0.003));
    no_ghosts(&mut app);
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    app.world_mut().write_message(Action::Redo);
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<Editor>().edits.models, placed);
}
