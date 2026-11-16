//! Catalog cards must match scene lighting and show reloaded GLB content.
use super::*;
use crate::editor::{
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::fixture_project,
};
use std::path::PathBuf;

fn warm(app: &mut App, frames: usize) {
    for _ in 0..frames {
        frame(app);
    }
}
fn ready(app: &mut App, require_loading: bool) {
    let mut invalidated = !require_loading;
    let mut events = bevy::ecs::message::MessageCursor::<AssetEvent<WorldAsset>>::default();
    let mut reloads = Vec::new();
    for _ in 0..600 {
        frame(app);
        for event in events.read(app.world().resource::<Messages<AssetEvent<WorldAsset>>>()) {
            reloads.push(format!("{event:?}"));
        }
        let previews = app.world().resource::<Thumbnails>();
        invalidated |= previews.state(Entry::Model(0)) == PreviewState::Loading;
        if invalidated
            && previews.state(Entry::Model(0)) == PreviewState::Ready
            && previews.job.is_none()
        {
            warm(app, 30);
            return;
        }
    }
    let previews = app.world().resource::<Thumbnails>();
    panic!(
        "A changed model card did not regenerate: invalidated={invalidated}, state={:?}, job={:?}, worlds={reloads:?}",
        previews.state(Entry::Model(0)),
        previews
            .job
            .as_ref()
            .map(|j| (j.target, j.instances.len(), j.camera, j.stable_frames))
    );
}
fn card_bounds(app: &mut App) -> Rect {
    let handle = app.world().resource::<Thumbnails>().images[0].clone();
    let world = app.world_mut();
    let (_, node, transform) = world
        .query::<(&ImageNode, &ComputedNode, &bevy::ui::UiGlobalTransform)>()
        .iter(world)
        .find(|(image, node, _)| image.image == handle && node.size().min_element() > 0.0)
        .unwrap();
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}
fn crop(image: &image::RgbaImage, rect: Rect) -> image::RgbaImage {
    image::imageops::crop_imm(
        image,
        rect.min.x.ceil() as u32,
        rect.min.y.ceil() as u32,
        rect.width().floor() as u32,
        rect.height().floor() as u32,
    )
    .to_image()
}
fn clay(image: &image::RgbaImage) -> [f32; 3] {
    let mut total = [0u64; 3];
    let mut count = 0;
    for pixel in image.pixels() {
        let [r, g, b, _] = pixel.0.map(u16::from);
        if r > g + 10 && r > b + 25 {
            for c in 0..3 {
                total[c] += pixel[c] as u64;
            }
            count += 1;
        }
    }
    assert!(
        count > 200,
        "The clay material must be visible in both renders"
    );
    total.map(|v| v as f32 / count as f32)
}
fn rewrite_model(path: &std::path::Path) {
    let bytes = std::fs::read(path).unwrap();
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut root: serde_json::Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    root["nodes"][0]["scale"] = serde_json::json!([1.8, 1.0, 1.0]);
    root["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] =
        serde_json::json!([0.8, 0.12, 0.025, 1.0]);
    let mut json = serde_json::to_vec(&root).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut output = bytes[..12].to_vec();
    output.extend_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(&bytes[16..20]);
    output.extend_from_slice(&json);
    output.extend_from_slice(&bytes[20 + length..]);
    let total = output.len() as u32;
    output[8..12].copy_from_slice(&total.to_le_bytes());
    std::fs::write(path, output).unwrap();
}

#[test]
#[ignore = "requires GPU; compares catalog/scene materials and verifies lighting and saved-GLB cache invalidation"]
fn gpu_model_thumbnail_matches_scene_and_updates_after_asset_reload() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-thumbnail-match-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(directory.join("models")).unwrap();
    std::fs::create_dir_all(directory.join("scenes")).unwrap();
    let model = directory.join("models/block.glb");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/assets/models/block.glb"),
        &model,
    )
    .unwrap();
    let pose =
        Transform::from_translation(Vec3::new(3.1, 3.1, 4.6)).looking_at(Vec3::Y * 0.5, Vec3::Y);
    let scene = serde_json::json!({
        "scene":{"label":"Test scene","cameras":[{"id":"main","label":"Camera","placement":crate::CameraPlacement{pose:pose.into(),lens:58.0},"projection":"perspective"}]},
        "models":[{"id":1,"asset":"models/block.glb","pose":crate::Pose::from(Transform::IDENTITY)}],
        "lighting":crate::SceneLighting::editor_default()
    });
    std::fs::write(
        directory.join("scenes/main.scene"),
        serde_json::to_vec(&scene).unwrap(),
    )
    .unwrap();
    let project = serde_json::json!({"title":"Preview regression","asset_root":".","scenes":[{"file":"scenes/main.scene"}],
        "catalog":[{"label":"Clay cube","asset":"models/block.glb","category":"Primitives","solid":true,"preview_direction":[1.55,1.3,2.3]}]});
    let manifest = directory.join("project.bevyeditor");
    std::fs::write(&manifest, serde_json::to_vec(&project).unwrap()).unwrap();
    let (mut app, _, target) = fixture_project(crate::EditorProject::load(manifest).unwrap());
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.selection = None;
        editor.view = pose;
        editor.pivot = Vec3::Y * 0.5;
        editor.drawer = true;
    }
    let folder = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == std::path::Path::new("models"))
        .unwrap();
    app.world_mut().resource_mut::<Browser>().navigate(folder);
    ready(&mut app, false);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/thumbnail-match-qa");
    std::fs::create_dir_all(&output).unwrap();
    let card = card_bounds(&mut app);
    let before = capture_when(
        &mut app,
        &target,
        &output.join("01-scene-and-card.png"),
        |_| true,
    );
    let thumbnail = crop(&before, card);
    let view = app.world().resource::<Thumbnails>().view.unwrap().entity;
    let environment = app.world().resource::<Editor>().environment().clone();
    assert_eq!(
        app.world().get::<Exposure>(view).unwrap().ev100,
        environment.exposure_ev100
    );
    assert_eq!(
        app.world().get::<AmbientLight>(view).unwrap().brightness,
        environment.ambient_brightness
    );
    assert_eq!(
        *app.world().get::<Tonemapping>(view).unwrap(),
        Tonemapping::from(environment.tonemapping)
    );
    assert!(
        !app.world_mut()
            .query::<&Name>()
            .iter(app.world())
            .any(|n| n.as_str() == "Thumbnail studio floor")
    );
    let scene_target = app.world().resource::<viewport::Surface>().0.clone();
    let scene_image = capture_when(
        &mut app,
        &scene_target,
        &output.join("02-scene-render.png"),
        |_| true,
    );
    let region = {
        let world = app.world_mut();
        let (camera, global) = world
            .query_filtered::<(&Camera, &GlobalTransform), With<viewport::EditorCamera>>()
            .single(world)
            .unwrap();
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for x in [-0.5, 0.5] {
            for y in [0.0, 1.0] {
                for z in [-0.5, 0.5] {
                    let p = camera
                        .world_to_viewport(global, Vec3::new(x, y, z))
                        .unwrap();
                    min = min.min(p);
                    max = max.max(p);
                }
            }
        }
        Rect::from_corners(min, max)
    };
    let scene_color = clay(&crop(&scene_image, region));
    let card_color = clay(&thumbnail);
    for c in 0..3 {
        assert!(
            (scene_color[c] - card_color[c]).abs() < 25.0,
            "Preview/scene color differs: {card_color:?} vs {scene_color:?}"
        );
    }
    app.world_mut()
        .resource_mut::<Editor>()
        .edits
        .lighting
        .as_mut()
        .unwrap()
        .exposure_ev100 += 2.0;
    ready(&mut app, true);
    let dark = capture_when(
        &mut app,
        &target,
        &output.join("03-lighting-change.png"),
        |_| true,
    );
    let dark_thumbnail = crop(&dark, card);
    assert!(clay(&dark_thumbnail)[0] + 20.0 < card_color[0]);
    let handle = app.world().resource::<Thumbnails>().images[0].clone();
    rewrite_model(&model);
    // The catalog releases the GLTF root after rendering. The scene keeps its
    // Scene0 subasset alive; AssetServer::reload requires a live asset path.
    app.world()
        .resource::<AssetServer>()
        .reload("models/block.glb#Scene0");
    ready(&mut app, true);
    assert_eq!(
        app.world().resource::<Thumbnails>().images[0],
        handle,
        "Card handles remain stable across regeneration"
    );
    let changed = capture_when(
        &mut app,
        &target,
        &output.join("04-reloaded-model.png"),
        |_| true,
    );
    let changed = crop(&changed, card);
    assert!(
        dark_thumbnail
            .pixels()
            .zip(changed.pixels())
            .filter(|(a, b)| a.0[..3]
                .iter()
                .zip(&b.0[..3])
                .map(|(a, b)| a.abs_diff(*b) as u32)
                .sum::<u32>()
                > 30)
            .count()
            > 500
    );
    drop(app);
    assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(directory).unwrap();
    eprintln!("Thumbnail match screenshots: {}", output.display());
}
