//! Exercise the real UI pipeline: shader validation, motion, alpha and resizing.
use super::*;
use crate::editor::selection_outline::render_tests::{CapturedImage, capture_when, frame};
use bevy::{
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{TextureFormat, TextureUsages},
    },
    time::TimeUpdateStrategy,
    window::{ExitCondition, PrimaryWindow},
    winit::WinitPlugin,
};
use std::{fs, time::Duration};

fn golden(pixel: &image::Rgba<u8>) -> bool {
    let [r, g, b, _] = pixel.0;
    r > 145 && g > 85 && r.saturating_sub(b) > 45
}

#[test]
#[ignore = "requires a GPU adapter; captures the animated black-hole UI material"]
fn gpu_black_hole_lenses_disk_and_animates_without_material_churn() {
    let output =
        std::env::temp_dir().join(format!("bevy-editor-black-hole-{}", std::process::id()));
    fs::create_dir_all(&output).unwrap();
    println!("Black-hole captures: {}", output.display());
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
    .add_plugins(OrbitalPlugin)
    .init_resource::<CapturedImage>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / 60.0,
    )));
    let mut image = Image::new_target_texture(640, 360, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.world_mut().spawn((
        Camera2d,
        bevy::ui::IsDefaultUiCamera,
        bevy::camera::RenderTarget::Image(target.clone().into()),
    ));
    let root = app
        .world_mut()
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(crate::ui_theme::PANEL),
        ))
        .id();
    let stage = {
        let mut commands = app.world_mut().commands();
        illustration(&mut commands, root)
    };
    app.finish();
    app.cleanup();
    for _ in 0..32 {
        frame(&mut app);
    }
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    let first = capture_when(&mut app, &target, &output.join("frame-00.png"), |image| {
        image.pixels().filter(|pixel| golden(pixel)).count() > 1500
    });
    let [r, g, b, _] = first.get_pixel(320, 180).0;
    assert!(
        r < 25 && g < 25 && b < 30,
        "The event horizon must stay dark"
    );
    // A lensed image rises above the thin projected disk, rather than looking
    // like an ordinary elliptical planetary ring.
    let upper_arc = first
        .enumerate_pixels()
        .filter(|(x, y, pixel)| *x > 260 && *x < 380 && *y < 155 && golden(pixel))
        .count();
    assert!(upper_arc > 120, "The upper lensed disk image is missing");
    for (x, y) in [(101, 61), (538, 61), (101, 298), (538, 298)] {
        let corner = first.get_pixel(x, y);
        let background = first.get_pixel(10, 10);
        assert_eq!(
            corner, background,
            "The material must have transparent corners"
        );
    }

    let mut changed = 0;
    for sample in 1..=47 {
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(Duration::from_secs_f64(1.0 / 12.0));
        let image = capture_when(
            &mut app,
            &target,
            &output.join(format!("frame-{sample:02}.png")),
            |_| true,
        );
        if sample == 24 {
            changed = first
                .pixels()
                .zip(image.pixels())
                .filter(|(a, b)| a.0[0].abs_diff(b.0[0]) > 12)
                .count();
        }
    }
    assert!(
        changed > 600,
        "The accretion flow must move between frames: {changed}"
    );
    assert_eq!(app.world().resource::<Assets<BlackHoleMaterial>>().len(), 1);
    app.world_mut().get_mut::<Node>(root).unwrap().width = px(300);
    for _ in 0..8 {
        frame(&mut app);
    }
    capture_when(&mut app, &target, &output.join("narrow.png"), |image| {
        image.pixels().filter(|pixel| golden(pixel)).count() > 800
    });
    app.world_mut().get_mut::<Node>(root).unwrap().width = percent(100);
    for _ in 0..8 {
        frame(&mut app);
    }
    let neutral = capture_when(
        &mut app,
        &target,
        &output.join("cursor-neutral.png"),
        |_| true,
    );
    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                resolution: (640, 360).into(),
                focused: true,
                ..default()
            },
        ))
        .id();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(470.0, 110.0)));
    for _ in 0..60 {
        frame(&mut app);
    }
    let pointed = capture_when(&mut app, &target, &output.join("cursor-right.png"), |_| {
        true
    });
    assert!(
        neutral == pointed,
        "Moving the pointer must not change the black-hole viewpoint or glow"
    );
    app.world_mut().entity_mut(stage).despawn();
    frame(&mut app);
    assert_eq!(app.world().resource::<Assets<BlackHoleMaterial>>().len(), 1);
}
