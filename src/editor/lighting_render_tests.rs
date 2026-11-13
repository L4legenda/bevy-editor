//! Optional GPU acceptance coverage for all four light renderers and exposure.
//! Run with `cargo +1.97.1 test --locked gpu_light_types_color_exposure_and_undo -- --ignored --test-threads=1 --nocapture`.
#![cfg(not(target_arch = "wasm32"))]

use bevy::{
    camera::{ClearColorConfig, Exposure, Hdr, RenderTarget},
    prelude::*,
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        view::{DebandDither, Tonemapping},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};

use crate::{
    editor::{
        Action, Editor, Field, RoomEdits, Scene, Selection, handle_actions,
        lighting::{self, EnvironmentField, LightEntity, LightField, LightingAction},
        selection_outline::render_tests::{CapturedImage, capture_when, frame, target},
        viewport::{EditorCamera, PointerState},
    },
    lighting::{LightKind, LightPreset},
};

fn channels(image: &image::RgbaImage) -> [f64; 3] {
    let mut totals = [0_u64; 3];
    let mut count = 0_u64;
    for (x, y, pixel) in image.enumerate_pixels() {
        if (48..272).contains(&x) && (40..224).contains(&y) {
            for (total, channel) in totals.iter_mut().zip(pixel.0) {
                *total += u64::from(channel);
            }
            count += 1;
        }
    }
    totals.map(|total| total as f64 / count as f64)
}

fn brightness(image: &image::RgbaImage) -> f64 {
    let [red, green, blue] = channels(image);
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

fn lit_pixels(image: &image::RgbaImage) -> usize {
    image
        .pixels()
        .filter(|pixel| pixel.0[..3].iter().any(|channel| *channel > 12))
        .count()
}

fn issue(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    frame(app);
}

fn change(app: &mut App, action: LightingAction) {
    issue(app, Action::Lighting(action));
}

fn custom_lights(app: &mut App) -> usize {
    app.world_mut()
        .query::<&LightEntity>()
        .iter(app.world())
        .count()
}

fn image_difference(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    let total: u64 = a
        .pixels()
        .zip(b.pixels())
        .map(|(a, b)| {
            a.0[..3]
                .iter()
                .zip(&b.0[..3])
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum::<u64>()
        })
        .sum();
    total as f64 / (a.width() * a.height() * 3) as f64
}

#[test]
#[ignore = "requires GPU adapter; renders HDRI diffuse light and metallic reflections"]
fn gpu_hdri_lighting_reflections_rotation_brightness_and_undo() {
    use crate::EnvironmentPreset;
    let directory = std::env::temp_dir().join(format!("bevy-editor-hdri-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    println!("HDRI GPU artifacts: {}", directory.display());
    let (mut app, output, camera) = fixture();
    let sphere = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Sphere::new(0.7).mesh().uv(48, 32));
    let metal = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.9, 0.9),
            metallic: 1.0,
            perceptual_roughness: 0.08,
            ..default()
        });
    app.world_mut().spawn((
        Mesh3d(sphere),
        MeshMaterial3d(metal),
        Transform::from_xyz(1.1, 0.7, 0.0),
    ));
    let baseline = capture_when(&mut app, &output, &directory.join("01-off.png"), |image| {
        brightness(image) < 2.0
    });
    issue(&mut app, Action::Select(Selection::Environment));
    change(&mut app, LightingAction::ToggleHdriBackground);
    change(
        &mut app,
        LightingAction::SetEnvironment(Some(EnvironmentPreset::SmallStudio)),
    );
    let studio = capture_when(
        &mut app,
        &output,
        &directory.join("02-studio.png"),
        |image| brightness(image) > 12.0 && image_difference(image, &baseline) > 8.0,
    );
    assert!(
        app.world()
            .get::<bevy::light::EnvironmentMapLight>(camera)
            .is_some()
    );
    assert_eq!(
        custom_lights(&mut app),
        0,
        "The environment alone must light the scene"
    );
    change(
        &mut app,
        LightingAction::SetNumber(Field::Environment(EnvironmentField::HdriRotation), 120.0),
    );
    let rotated = capture_when(
        &mut app,
        &output,
        &directory.join("03-rotated.png"),
        |image| image_difference(image, &studio) > 2.0,
    );
    change(
        &mut app,
        LightingAction::SetEnvironment(Some(EnvironmentPreset::VeniceSunset)),
    );
    let sunset = capture_when(
        &mut app,
        &output,
        &directory.join("04-sunset.png"),
        |image| image_difference(image, &rotated) > 5.0 && brightness(image) > 5.0,
    );
    change(
        &mut app,
        LightingAction::SetNumber(Field::Environment(EnvironmentField::HdriBrightness), 0.0),
    );
    capture_when(
        &mut app,
        &output,
        &directory.join("05-zero-brightness.png"),
        |image| brightness(image) < 2.0,
    );
    issue(&mut app, Action::Undo);
    capture_when(&mut app, &output, &directory.join("06-undo.png"), |image| {
        image_difference(image, &sunset) < 0.1
    });
    change(&mut app, LightingAction::SetEnvironment(None));
    capture_when(
        &mut app,
        &output,
        &directory.join("07-disabled.png"),
        |image| brightness(image) < 2.0,
    );
    assert!(
        app.world()
            .get::<bevy::light::EnvironmentMapLight>(camera)
            .is_none()
    );
}

fn fixture() -> (App, Handle<Image>, Entity) {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>(),
    )
    .add_plugins(crate::HdriPlugin)
    .init_resource::<CapturedImage>()
    .init_resource::<PointerState>()
    .add_message::<Action>()
    .add_systems(Update, (handle_actions, lighting::sync).chain());
    let output = target(&mut app);
    let view = Transform::from_xyz(3.0, 2.8, 5.0).looking_at(Vec3::new(0.0, 0.4, 0.0), Vec3::Y);
    let camera = app
        .world_mut()
        .spawn((
            EditorCamera,
            bevy::camera::ShadowLodOrigin,
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Image(output.clone().into()),
            Projection::Perspective(PerspectiveProjection {
                fov: 50.0_f32.to_radians(),
                near: 0.1,
                far: 100.0,
                ..default()
            }),
            view,
            Hdr,
            AmbientLight {
                brightness: 0.0,
                ..default()
            },
            Exposure { ev100: 8.0 },
            Msaa::Off,
            Tonemapping::None,
            DebandDither::Disabled,
        ))
        .id();
    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::Inherited))
        .id();
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.7, 0.7, 0.7),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            ..default()
        });
    let cube = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 1.0, 1.0));
    let floor = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Plane3d::default().mesh().size(10.0, 10.0));
    app.world_mut().spawn((
        Mesh3d(cube),
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(0.0, 0.5, 0.0),
        ChildOf(root),
    ));
    app.world_mut().spawn((
        Mesh3d(floor),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        ChildOf(root),
    ));
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let mut editor = Editor::new(project, RoomEdits::default());
    editor.ready = true;
    editor.reload = false;
    editor.view = view;
    editor.pivot = Vec3::ZERO;
    app.insert_resource(editor).insert_resource(Scene {
        root: Some(root),
        ..default()
    });
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        frame(&mut app);
    }
    (app, output, camera)
}

#[test]
#[ignore = "requires GPU adapter"]
fn gpu_light_types_color_exposure_and_undo() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-editor-lighting-render-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    println!("Lighting GPU artifacts: {}", directory.display());
    let (mut app, output, camera) = fixture();
    let baseline = capture_when(
        &mut app,
        &output,
        &directory.join("baseline.png"),
        |image| brightness(image) < 2.0,
    );
    let dark = brightness(&baseline);

    for (preset, kind, name, intensity) in [
        (LightPreset::Bulb, LightKind::Point, "point", 5000.0),
        (LightPreset::Spot, LightKind::Spot, "spot", 5000.0),
        (LightPreset::Sun, LightKind::Directional, "sun", 10000.0),
        (LightPreset::Softbox, LightKind::Rect, "rect", 5000.0),
    ] {
        change(&mut app, LightingAction::Add(preset));
        change(
            &mut app,
            LightingAction::SetNumber(Field::Light(LightField::Intensity), intensity),
        );
        assert_eq!(app.world().resource::<Editor>().edits.lights[0].kind, kind);
        assert_eq!(custom_lights(&mut app), 1);
        let lit = capture_when(
            &mut app,
            &output,
            &directory.join(format!("{name}.png")),
            |image| brightness(image) > dark + 1.0 && lit_pixels(image) > 128,
        );
        assert!(
            brightness(&lit) > dark + 1.0,
            "{kind:?} did not light the mesh"
        );
        while !app.world().resource::<Editor>().undo.is_empty() {
            issue(&mut app, Action::Undo);
        }
        assert!(app.world().resource::<Editor>().edits.lights.is_empty());
        assert!(app.world().resource::<Editor>().edits.lighting.is_none());
        assert_eq!(custom_lights(&mut app), 0);
        capture_when(
            &mut app,
            &output,
            &directory.join(format!("{name}-undone.png")),
            |image| brightness(image) <= dark + 0.5 && lit_pixels(image) < 16,
        );
    }

    change(&mut app, LightingAction::Add(LightPreset::Bulb));
    change(
        &mut app,
        LightingAction::SetNumber(Field::Light(LightField::Intensity), 5000.0),
    );
    change(&mut app, LightingAction::SetColor([255, 0, 0]));
    let red = capture_when(&mut app, &output, &directory.join("red.png"), |image| {
        let [red, _, blue] = channels(image);
        red > blue + 4.0 && red > 4.0
    });
    change(&mut app, LightingAction::SetColor([0, 0, 255]));
    let blue = capture_when(&mut app, &output, &directory.join("blue.png"), |image| {
        let [red, _, blue] = channels(image);
        blue > red + 4.0 && blue > 4.0
    });
    assert!(channels(&red)[0] > channels(&blue)[0] + 4.0);
    assert!(channels(&blue)[2] > channels(&red)[2] + 4.0);

    issue(&mut app, Action::Select(Selection::Environment));
    assert_eq!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Environment),
        "environment selection was lost before editing exposure"
    );
    change(
        &mut app,
        LightingAction::SetNumber(Field::Environment(EnvironmentField::Exposure), 14.0),
    );
    {
        let editor = app.world().resource::<Editor>();
        assert!(
            editor.edits.lighting.is_some(),
            "exposure edit was rejected: status={:?}, defaults={:?}",
            editor.status,
            editor.lighting_defaults
        );
        assert_eq!(
            editor.environment().exposure_ev100,
            14.0,
            "exposure edit did not update scene settings: status={:?}",
            editor.status
        );
        assert_eq!(
            app.world().get::<Exposure>(camera).unwrap().ev100,
            14.0,
            "scene exposure was not applied to the camera: status={:?}",
            editor.status
        );
    }
    let dimmed = capture_when(
        &mut app,
        &output,
        &directory.join("exposure.png"),
        |image| channels(image)[2] < channels(&blue)[2] * 0.8,
    );
    issue(&mut app, Action::Undo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .environment()
            .exposure_ev100,
        5.0
    );
    assert_eq!(app.world().get::<Exposure>(camera).unwrap().ev100, 5.0);
    capture_when(
        &mut app,
        &output,
        &directory.join("exposure-undone.png"),
        |image| {
            channels(image)[2] > channels(&dimmed)[2] + 2.0
                && (channels(image)[2] - channels(&blue)[2]).abs() < 2.0
        },
    );
    while !app.world().resource::<Editor>().undo.is_empty() {
        issue(&mut app, Action::Undo);
    }
    assert_eq!(custom_lights(&mut app), 0);
    capture_when(
        &mut app,
        &output,
        &directory.join("all-undone.png"),
        |image| brightness(image) <= dark + 0.5 && lit_pixels(image) < 16,
    );
}
