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
        let parent = world
            .get::<ChildOf>(current)
            .expect("Control must be inside the inspector")
            .parent();
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
    let canvas = app.world().resource::<Canvas>().0.clone();
    let target = RenderTarget::Image(canvas.into())
        .normalize(Some(window))
        .unwrap();
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
    let mut text_pixels = 0;
    for y in ui::TOP as u32..image.height() - ui::FOOT as u32 {
        for x in WIDTH - ui::SIDE as u32..WIDTH {
            let [r, g, b, _] = image.get_pixel(x, y).0;
            if r > 170 && g > 160 && b > 150 {
                text_pixels += 1;
            }
        }
    }
    text_pixels > 180
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
