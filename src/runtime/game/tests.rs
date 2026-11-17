use super::*;
use bevy::{
    camera::RenderTarget,
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{PollType, TextureFormat, TextureUsages},
        renderer::RenderDevice,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use std::time::Duration;

fn native_level() -> (PathBuf, GameLevel) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native/assets");
    let level = GameLevel::read(&root, "scenes/small_room.scene").unwrap();
    (root, level)
}

#[test]
fn game_reads_a_native_level_without_the_editor_or_project_manifest() {
    let (root, level) = native_level();
    let document = LevelEdits::read(&root, "scenes/small_room.scene").unwrap();
    assert_eq!(level.edits, document);
    assert_eq!(level.title, document.scene.as_ref().unwrap().label);
    assert_eq!(level.camera, level.cameras[0].id);
    assert!(level.asset.is_empty());
}

#[test]
fn game_projection_uses_authored_lens_and_camera_kind() {
    let (_, level) = native_level();
    let mut camera = level.cameras[0].clone();
    match projection(&camera, 73.0) {
        Projection::Perspective(lens) => assert!((lens.fov - 73_f32.to_radians()).abs() < 1e-5),
        _ => panic!("Expected perspective"),
    }
    camera.projection = LensKind::Orthographic;
    match projection(&camera, 6.5) {
        Projection::Orthographic(lens) => assert!(matches!(
            lens.scaling_mode,
            ScalingMode::FixedVertical {
                viewport_height: 6.5
            }
        )),
        _ => panic!("Expected orthographic"),
    }
}

#[derive(Resource)]
struct Target(Handle<Image>);
#[derive(Resource, Default)]
struct Capture(Option<Image>);

fn retarget(mut cameras: Query<&mut RenderTarget, With<GameCamera>>, target: Res<Target>) {
    for mut camera in &mut cameras {
        *camera = RenderTarget::Image(target.0.clone().into());
    }
}

fn headless(root: &Path, level: GameLevel) -> (App, Handle<Image>) {
    let mut app = App::new();
    app.add_plugins(crate::AssetRedirectPlugin::new(root).unwrap())
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: root.to_string_lossy().into_owned(),
                    ..default()
                })
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
        .add_plugins(GameLevelPlugin(level))
        .init_resource::<Capture>()
        .add_systems(Startup, retarget.after(setup_level));
    let mut image = Image::new_target_texture(640, 400, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.insert_resource(Target(target.clone()));
    app.finish();
    app.cleanup();
    (app, target)
}

fn frame(app: &mut App) {
    app.update();
    app.world()
        .resource::<RenderDevice>()
        .wgpu_device()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .unwrap();
}

fn wait_ready(app: &mut App) {
    for _ in 0..600 {
        frame(app);
        let status = app.world().resource::<GameLevelStatus>();
        assert!(status.error.is_none(), "{:?}", status.error);
        if status.ready {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("Game level did not finish loading");
}

fn capture(app: &mut App, target: &Handle<Image>, name: &str) {
    for _ in 0..12 {
        frame(app);
    }
    app.world_mut()
        .spawn(Screenshot::image(target.clone()))
        .observe(
            |capture: On<ScreenshotCaptured>, mut result: ResMut<Capture>| {
                result.0 = Some(capture.image.clone());
            },
        );
    for _ in 0..120 {
        frame(app);
        if let Some(image) = app.world_mut().resource_mut::<Capture>().0.take() {
            let image = image.try_into_dynamic().unwrap().into_rgba8();
            assert!(
                image
                    .pixels()
                    .filter(|pixel| pixel.0[..3].iter().any(|channel| *channel > 70))
                    .count()
                    > 500,
                "The game image is blank"
            );
            let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/play-qa");
            fs::create_dir_all(&output).unwrap();
            image.save(output.join(name)).unwrap();
            return;
        }
    }
    panic!("Game screenshot did not arrive");
}

#[test]
#[ignore = "requires GPU; writes target/play-qa/native-level.png"]
fn gpu_game_level_loads_unsaved_models_camera_and_lights() {
    let (root, mut level) = native_level();
    level.edits.models[0].pose.position = [1.25, 0.0, 0.0];
    let placement = crate::CameraPlacement {
        pose: Transform::from_xyz(5.0, 3.5, 6.0)
            .looking_at(Vec3::new(1.25, 0.5, 0.0), Vec3::Y)
            .into(),
        lens: 52.0,
    };
    level.edits.cameras.insert(level.camera.clone(), placement);
    level
        .edits
        .lights
        .push(crate::LightPreset::Bulb.create(900));
    let model_id = level.edits.models[0].id;
    let (mut app, target) = headless(&root, level);
    wait_ready(&mut app);
    let world = app.world_mut();
    assert_eq!(
        world
            .query_filtered::<&Transform, With<GameCamera>>()
            .single(world)
            .unwrap()
            .translation,
        Vec3::from(placement.pose.position)
    );
    assert_eq!(
        world
            .query::<(&GameModel, &Transform)>()
            .iter(world)
            .find(|(model, _)| model.0 == model_id)
            .unwrap()
            .1
            .translation,
        Vec3::new(1.25, 0.0, 0.0)
    );
    assert!(world.query::<&PointLight>().iter(world).count() >= 1);
    assert_eq!(
        world
            .query::<&Camera>()
            .iter(world)
            .filter(|camera| camera.is_active)
            .count(),
        1
    );
    capture(&mut app, &target, "native-level.png");
}

#[test]
#[ignore = "requires GPU; validates legacy GLB node transforms and removals"]
fn gpu_game_level_applies_imported_world_edits_and_removals() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/assets");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.parent().unwrap().join("project.json")).unwrap())
            .unwrap();
    let source = &manifest["scenes"][0];
    let cameras: Vec<CameraDefinition> = serde_json::from_value(source["cameras"].clone()).unwrap();
    let mut edits = LevelEdits::default();
    edits.nodes.insert(
        "Block 1".into(),
        crate::NodeEdit {
            original: Transform::IDENTITY.into(),
            pose: Transform::from_xyz(2.0, 1.0, -2.0).into(),
        },
    );
    edits.deleted_nodes.insert("Block 2".into());
    let level = GameLevel {
        title: "Imported test".into(),
        asset: source["asset"].as_str().unwrap().into(),
        groups: serde_json::from_value(source["groups"].clone()).unwrap(),
        excluded_nodes: vec![],
        camera: cameras[0].id.clone(),
        cameras,
        edits,
        environment: SceneLighting::default(),
    };
    let (mut app, _) = headless(&root, level);
    wait_ready(&mut app);
    frame(&mut app);
    let world = app.world_mut();
    let mut nodes = world.query::<(&Name, &GlobalTransform)>();
    let pose = nodes
        .iter(world)
        .find(|(name, _)| name.as_str() == "Block 1")
        .expect("Edited imported node must exist")
        .1;
    assert!(pose.translation().distance(Vec3::new(2.0, 1.0, -2.0)) < 1e-4);
    assert!(
        !nodes
            .iter(world)
            .any(|(name, _)| name.as_str() == "Block 2")
    );
}
