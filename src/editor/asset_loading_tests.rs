//! Exercise the real browser and GPU shader while holding preview readiness.
use super::*;
use crate::editor::{
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::fixture,
    ui::asset_loading::{LoadingMaterial, LoadingPreview},
};
use bevy::ui::UiGlobalTransform;
use image::GenericImageView;

#[test]
fn failed_thumbnails_finish_waiting_and_can_become_ready_after_retry() {
    let mut previews = Thumbnails::new(&mut Assets::<Image>::default(), 1, 1);
    let entry = Entry::Model(0);
    assert_eq!(previews.state(entry), PreviewState::Loading);
    previews.fail(Target::Model(0));
    assert_eq!(previews.state(entry), PreviewState::Failed);
    previews.complete(Target::Model(0));
    assert_eq!(previews.state(entry), PreviewState::Ready);
}

#[derive(Resource)]
struct HeldPreview(PreviewState);

fn hold_preview(state: Res<HeldPreview>, mut previews: ResMut<Thumbnails>) {
    let target = Target::Model(0);
    match state.0 {
        PreviewState::Loading => {
            previews.completed.remove(&target);
            previews.failed.remove(&target);
        }
        PreviewState::Ready => previews.complete(target),
        PreviewState::Failed => previews.fail(target),
    }
}

fn overlays(app: &mut App, well: Entity) -> usize {
    let world = app.world_mut();
    world
        .query_filtered::<&ChildOf, With<MaterialNode<LoadingMaterial>>>()
        .iter(world)
        .filter(|parent| parent.parent() == well)
        .count()
}

#[test]
#[ignore = "requires GPU adapter; captures animated loading, successful reveal, cache reuse and failure"]
fn gpu_asset_loading_shader_animates_reveals_and_stops_on_failure() {
    let (mut app, _, canvas) = fixture();
    // Prepare a genuine preview first; the held readiness never replaces its pixels.
    for _ in 0..480 {
        if app
            .world()
            .resource::<Thumbnails>()
            .completed
            .contains(&Target::Model(0))
        {
            break;
        }
        frame(&mut app);
    }
    assert_eq!(
        app.world().resource::<Thumbnails>().state(Entry::Model(0)),
        PreviewState::Ready
    );
    app.insert_resource(HeldPreview(PreviewState::Loading));
    app.add_systems(
        Update,
        hold_preview
            .after(update)
            .before(crate::editor::ui::asset_loading::sync),
    );
    let folder = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == std::path::Path::new("models"))
        .unwrap();
    app.world_mut().resource_mut::<Browser>().navigate(folder);
    app.world_mut().resource_mut::<Editor>().drawer = true;
    for _ in 0..28 {
        frame(&mut app);
    }

    let (well, bounds) = {
        let world = app.world_mut();
        let card = world
            .query::<(Entity, &AccessibleLabel)>()
            .iter(world)
            .find(|(_, label)| label.0.starts_with("Открыть модель block.glb"))
            .expect("The actual browser must have the model card")
            .0;
        let (well, _, transform, computed) = world.query_filtered::<
            (Entity, &ChildOf, &UiGlobalTransform, &ComputedNode), With<LoadingPreview>
        >().iter(world).find(|(_, parent, ..)| parent.parent() == card).unwrap();
        (
            well,
            Rect::from_center_size(transform.affine().translation, computed.size()),
        )
    };
    assert_eq!(overlays(&mut app, well), 1);
    let origin = bounds.min.ceil().as_uvec2();
    let size = bounds.size().floor().as_uvec2();
    let output =
        std::env::temp_dir().join(format!("bevy-editor-asset-loading-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Asset loading GPU captures: {}", output.display());
    let first = capture_when(&mut app, &canvas, &output.join("01-loading.png"), |image| {
        image::imageops::crop_imm(image, origin.x, origin.y, size.x, size.y)
            .pixels()
            .filter(|(_, _, pixel)| {
                let [r, g, b, _] = pixel.0.map(u16::from);
                r > 150 && g > 100 && r > b + 25
            })
            .count()
            > 20
    });
    for _ in 0..18 {
        frame(&mut app);
    }
    let second = capture_when(
        &mut app,
        &canvas,
        &output.join("02-loading-motion.png"),
        |_| true,
    );
    let a = image::imageops::crop_imm(&first, origin.x, origin.y, size.x, size.y);
    let b = image::imageops::crop_imm(&second, origin.x, origin.y, size.x, size.y);
    let changed = a
        .pixels()
        .zip(b.pixels())
        .filter(|((_, _, a), (_, _, b))| {
            (0..3)
                .map(|channel| a[channel].abs_diff(b[channel]) as u32)
                .sum::<u32>()
                > 18
        })
        .count();
    assert!(
        changed > 80,
        "The loading shader must animate; only {changed} pixels changed"
    );

    app.world_mut().resource_mut::<HeldPreview>().0 = PreviewState::Ready;
    frame(&mut app);
    assert_eq!(
        overlays(&mut app, well),
        1,
        "The ready thumbnail must fade in"
    );
    for _ in 0..26 {
        frame(&mut app);
    }
    assert_eq!(overlays(&mut app, well), 0);
    capture_when(&mut app, &canvas, &output.join("03-ready.png"), |_| true);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    frame(&mut app);
    app.world_mut().resource_mut::<Editor>().drawer = true;
    frame(&mut app);
    assert_eq!(
        overlays(&mut app, well),
        0,
        "Cached previews must not flash the loader"
    );

    app.world_mut().resource_mut::<HeldPreview>().0 = PreviewState::Loading;
    frame(&mut app);
    assert_eq!(overlays(&mut app, well), 1);
    app.world_mut().resource_mut::<HeldPreview>().0 = PreviewState::Failed;
    for _ in 0..26 {
        frame(&mut app);
    }
    assert_eq!(
        overlays(&mut app, well),
        0,
        "A failed preview must stop animating"
    );
    assert!(
        app.world_mut()
            .query::<&Text>()
            .iter(app.world())
            .any(|text| text.0 == "Нет превью")
    );
    capture_when(
        &mut app,
        &canvas,
        &output.join("04-unavailable.png"),
        |image| {
            let pixel = image.get_pixel(origin.x + 6, origin.y + size.y / 2).0;
            pixel[..3].iter().all(|&channel| channel < 24)
        },
    );
}
