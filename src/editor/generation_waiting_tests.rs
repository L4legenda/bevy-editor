//! Validate the actual GPU shader and its lifetime in the editor workspace.
use super::*;
use crate::editor::{
    generation_backend::ResultModel,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture, mouse, move_pointer},
};
use bevy::input::ButtonState;
use std::{fs, path::PathBuf};

fn warm(app: &mut App, count: usize) {
    for _ in 0..count {
        frame(app);
    }
}

fn toggle(app: &mut App, window: Entity) {
    let point = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(label, node, _)| label.0 == "Переключить 3D генерацию" && !node.is_empty())
            .map(|(_, _, transform)| transform.translation)
            .unwrap()
    };
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}

fn clock(app: &App, handle: &Handle<WaitingMaterial>) -> f32 {
    app.world()
        .resource::<Assets<WaitingMaterial>>()
        .get(handle)
        .unwrap()
        .motion
        .x
}

#[test]
#[ignore = "requires a GPU; validates the empty-result shader, animation, suspension and replacement with a real GLB"]
fn gpu_generation_waiting_shader_animates_only_in_empty_visible_preview() {
    let (mut app, window, target) = fixture();
    warm(&mut app, 45);
    app.world_mut()
        .insert_resource(installation::State::installed_fixture());
    app.world_mut().insert_resource(run::State::ready_fixture());
    let before = app.world().resource::<Editor>().edits.clone();
    toggle(&mut app, window);
    warm(&mut app, 16);
    let (entity, handle, bounds) = {
        let world = app.world_mut();
        let (entity, material, computed, transform) = world
            .query_filtered::<(
                Entity,
                &MaterialNode<WaitingMaterial>,
                &ComputedNode,
                &UiGlobalTransform,
            ), With<Waiting>>()
            .single(world)
            .unwrap();
        let min = transform.translation - computed.size() * 0.5;
        let max = min + computed.size();
        assert!(!computed.is_empty());
        (entity, material.0.clone(), (min.as_uvec2(), max.as_uvec2()))
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-waiting-qa");
    fs::create_dir_all(&output).unwrap();
    let inside =
        |x: u32, y: u32| x >= bounds.0.x && x < bounds.1.x && y >= bounds.0.y && y < bounds.1.y;
    let first = capture_when(&mut app, &target, &output.join("01-waiting.png"), |image| {
        image
            .enumerate_pixels()
            .filter(|(x, y, pixel)| {
                let [r, g, b, _] = pixel.0;
                inside(*x, *y) && r > 100 && g > 65 && r > b.saturating_add(25)
            })
            .count()
            > 500
    });
    warm(&mut app, 100);
    let second = capture_when(&mut app, &target, &output.join("02-moving.png"), |_| true);
    let changed = first
        .enumerate_pixels()
        .filter(|(x, y, pixel)| {
            inside(*x, *y)
                && pixel
                    .0
                    .iter()
                    .zip(second.get_pixel(*x, *y).0)
                    .any(|(a, b)| a.abs_diff(b) > 18)
        })
        .count();
    assert!(
        changed > 1200,
        "The rendered form must visibly turn: {changed} pixels changed"
    );

    for (phase, name) in [
        (std::f32::consts::PI, "04-front-orbit.png"),
        (std::f32::consts::PI * 3.0, "05-back-orbit.png"),
    ] {
        app.world_mut()
            .resource_mut::<Assets<WaitingMaterial>>()
            .get_mut(&handle)
            .unwrap()
            .motion
            .x = phase;
        capture_when(&mut app, &target, &output.join(name), |_| true);
    }

    toggle(&mut app, window);
    let paused = clock(&app, &handle);
    warm(&mut app, 20);
    assert_eq!(
        clock(&app, &handle),
        paused,
        "Hidden tabs must not animate or invalidate GPU assets"
    );
    toggle(&mut app, window);
    warm(&mut app, 12);
    assert!(clock(&app, &handle) > paused);
    assert_eq!(app.world().resource::<Assets<WaitingMaterial>>().len(), 1);

    // The real result lifecycle must remove the artwork and free its redraw loop.
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/assets/models/blue_block.glb"),
        output.join("mesh.glb"),
    )
    .unwrap();
    app.world_mut().resource_mut::<run::State>().result = Some(ResultModel {
        directory: output.clone(),
        name: "Shader replacement".into(),
        seed: 1,
        steps: 1,
        guidance: 5.0,
        resolution: 128,
        vertices: 8,
        faces: 12,
        raw_faces: 12,
        seconds: 1.0,
        obj: false,
    });
    warm(&mut app, 50);
    assert!(app.world().get::<ComputedNode>(entity).unwrap().is_empty());
    let stopped = clock(&app, &handle);
    warm(&mut app, 12);
    assert_eq!(clock(&app, &handle), stopped);
    capture_when(&mut app, &target, &output.join("03-result.png"), |_| true);
    assert_eq!(app.world().resource::<Editor>().edits, before);
}
