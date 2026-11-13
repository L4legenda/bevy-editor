//! Optional GPU integration coverage, using the externally-driven headless
//! renderer and Screenshot APIs from Bevy's installed examples.
//!
//! Run with:
//! `cargo test --locked gpu_selection_brackets_follow_bounds_and_show_through_occluders -- --ignored --test-threads=1 --nocapture`

#![cfg(not(target_arch = "wasm32"))]

use std::{path::PathBuf, time::Duration};

use bevy::{
    camera::{
        ClearColorConfig, RenderTarget, ScalingMode, Viewport,
        visibility::{RenderLayers, VisibilitySystems},
    },
    gizmos::AppGizmoBuilder,
    prelude::*,
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{PollType, TextureFormat, TextureUsages},
        renderer::RenderDevice,
        view::{
            DebandDither, Tonemapping,
            screenshot::{Screenshot, ScreenshotCaptured},
        },
    },
    transform::TransformSystems,
    window::{ExitCondition, PrimaryWindow},
    winit::WinitPlugin,
};

use super::{LAYER, SelectionGizmos, draw, setup};
use crate::{
    data::PlacedModel,
    editor::{Editor, RoomEdits, Scene, Selection, ui, viewport::EditorCamera},
    project::EditorProject,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const MAX_CAPTURE_FRAMES: usize = 240;

#[derive(Resource)]
struct Targets {
    scene: Handle<Image>,
    composite: Handle<Image>,
    ui_camera: Entity,
}

#[derive(Resource, Default)]
pub(crate) struct CapturedImage(Option<Image>);

fn composite_ui(mut commands: Commands, targets: Res<Targets>) {
    let root = commands
        .spawn((
            UiTargetCamera(targets.ui_camera),
            Node {
                width: px(WIDTH),
                height: px(HEIGHT),
                ..default()
            },
        ))
        .id();
    commands.spawn((
        ImageNode::new(targets.scene.clone()),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ChildOf(root),
    ));
}

pub(crate) fn frame(app: &mut App) {
    app.update();
    app.world()
        .resource::<RenderDevice>()
        .wgpu_device()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .expect("GPU work did not finish within the headless frame budget");
}

fn amber(pixel: &image::Rgba<u8>) -> bool {
    let [red, green, blue, _] = pixel.0.map(u16::from);
    red > 150 && green > 90 && blue < 170 && red > blue + 50 && green > blue + 30
}

fn blue(pixel: &image::Rgba<u8>) -> bool {
    let [red, green, blue, _] = pixel.0.map(u16::from);
    blue > red + 60 && blue > green + 30
}

fn amber_count(image: &image::RgbaImage) -> usize {
    image.pixels().filter(|pixel| amber(pixel)).count()
}

fn bounds(image: &image::RgbaImage, accept: impl Fn(&image::Rgba<u8>) -> bool) -> [u32; 4] {
    let mut result = [WIDTH, HEIGHT, 0, 0];
    let mut count = 0;
    for (x, y, pixel) in image.enumerate_pixels() {
        if accept(pixel) {
            result[0] = result[0].min(x);
            result[1] = result[1].min(y);
            result[2] = result[2].max(x);
            result[3] = result[3].max(y);
            count += 1;
        }
    }
    assert!(count > 100, "The expected rendered geometry is missing");
    result
}

pub(crate) fn capture_when(
    app: &mut App,
    target: &Handle<Image>,
    output: &std::path::Path,
    accept: impl Fn(&image::RgbaImage) -> bool,
) -> image::RgbaImage {
    let size = app
        .world()
        .resource::<Assets<Image>>()
        .get(target)
        .expect("Screenshot target must exist")
        .texture_descriptor
        .size;
    let mut request = true;
    let mut last = None;
    for _ in 0..MAX_CAPTURE_FRAMES {
        if request {
            app.world_mut()
                .spawn(Screenshot::image(target.clone()))
                .observe(
                    |capture: On<ScreenshotCaptured>, mut output: ResMut<CapturedImage>| {
                        output.0 = Some(capture.image.clone());
                    },
                );
            request = false;
        }
        frame(app);
        let Some(image) = app.world_mut().resource_mut::<CapturedImage>().0.take() else {
            continue;
        };
        let image = image
            .try_into_dynamic()
            .expect("Screenshot target must be an ordinary RGBA image")
            .into_rgba8();
        assert_eq!(image.dimensions(), (size.width, size.height));
        if accept(&image) {
            image.save(output).unwrap();
            return image;
        }
        last = Some(image);
        request = true;
    }
    if let Some(image) = last {
        image.save(output).unwrap();
    }
    panic!(
        "Expected GPU output never appeared; inspect {}",
        output.display()
    );
}

pub(in crate::editor) fn target(app: &mut App) -> Handle<Image> {
    let mut image = Image::new_target_texture(WIDTH, HEIGHT, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    app.world_mut().resource_mut::<Assets<Image>>().add(image)
}

fn fixture() -> (App, Entity) {
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
    .init_gizmo_group::<SelectionGizmos>()
    .init_resource::<CapturedImage>()
    .add_systems(Startup, (setup, composite_ui).chain())
    .add_systems(
        PostUpdate,
        draw.after(TransformSystems::Propagate)
            .after(VisibilitySystems::VisibilityPropagate)
            .after(VisibilitySystems::CalculateBounds),
    );

    // Runtime synchronization can query a primary window without creating an
    // OS window. The actual cameras all render to GPU images.
    app.world_mut().spawn((
        PrimaryWindow,
        Window {
            resolution: (
                WIDTH + ui::SIDE as u32,
                HEIGHT + ui::TOP as u32 + ui::FOOT as u32,
            )
                .into(),
            ..default()
        },
    ));
    let scene_image = target(&mut app);
    let composite = target(&mut app);
    app.world_mut().spawn((
        EditorCamera,
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.025, 0.035, 0.05)),
            // Exercise projection and bracket padding with a nonzero viewport offset.
            viewport: Some(Viewport {
                physical_position: UVec2::new(16, 12),
                physical_size: UVec2::new(WIDTH - 32, HEIGHT - 24),
                ..default()
            }),
            ..default()
        },
        RenderTarget::Image(scene_image.clone().into()),
        RenderLayers::from_layers(&[0, LAYER]),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 4.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
    ));
    let ui_camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                order: 10,
                ..default()
            },
            RenderTarget::Image(composite.clone().into()),
            Msaa::Off,
        ))
        .id();
    app.insert_resource(Targets {
        scene: scene_image,
        composite,
        ui_camera,
    });

    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::Inherited))
        .id();
    let model_position = Vec3::new(-0.45, 0.35, 0.0);
    let model = app
        .world_mut()
        .spawn((
            Transform::from_translation(model_position),
            Visibility::Inherited,
        ))
        .id();
    let mut cube: Mesh = Cuboid::new(1.5, 1.5, 1.5).into();
    cube.try_insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[0.0, 0.2, 0.85, 1.0]; cube.count_vertices()],
    )
    .unwrap();
    let cube = app.world_mut().resource_mut::<Assets<Mesh>>().add(cube);
    let white = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        });
    app.world_mut().spawn((
        Mesh3d(cube),
        MeshMaterial3d(white),
        Transform::IDENTITY,
        ChildOf(model),
    ));
    let cover_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(2.5, 2.5, 0.2));
    let cover_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.5, 0.15),
            unlit: true,
            ..default()
        });
    let cover = app
        .world_mut()
        .spawn((
            Mesh3d(cover_mesh),
            MeshMaterial3d(cover_material),
            Transform::from_translation(model_position + Vec3::Z * 1.5),
            Visibility::Hidden,
            ChildOf(root),
        ))
        .id();

    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let asset = project.catalog[0].asset.clone();
    let mut editor = Editor::new(project, RoomEdits::default());
    editor.ready = true;
    editor.reload = false;
    editor.edits.models.push(PlacedModel {
        id: 1,
        asset: asset.clone(),
        pose: Transform::from_translation(model_position).into(),
        bounds: None,
    });
    let mut scene = Scene {
        root: Some(root),
        ..default()
    };
    scene.models.insert(1, (model, asset));
    app.insert_resource(editor).insert_resource(scene);
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        frame(&mut app);
    }
    (app, cover)
}

#[test]
#[ignore = "requires GPU adapter"]
fn gpu_selection_brackets_follow_bounds_and_show_through_occluders() {
    let output =
        std::env::temp_dir().join(format!("bevy-editor-outline-render-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Outline GPU artifacts: {}", output.display());
    let (mut app, cover) = fixture();
    let target = app.world().resource::<Targets>().composite.clone();
    let baseline = capture_when(&mut app, &target, &output.join("baseline.png"), |image| {
        image.pixels().filter(|pixel| blue(pixel)).count() > 1000
    });
    assert_eq!(amber_count(&baseline), 0);
    let cube_bounds = bounds(&baseline, blue);
    let center = (
        (cube_bounds[0] + cube_bounds[2]) / 2,
        (cube_bounds[1] + cube_bounds[3]) / 2,
    );

    app.world_mut().resource_mut::<Editor>().selection = Some(Selection::Model(1));
    let selected = capture_when(&mut app, &target, &output.join("selected.png"), |image| {
        amber_count(image) > 32
    });
    let exterior = selected
        .enumerate_pixels()
        .filter(|(x, y, pixel)| {
            amber(pixel)
                && (*x < cube_bounds[0]
                    || *y < cube_bounds[1]
                    || *x > cube_bounds[2]
                    || *y > cube_bounds[3])
        })
        .count();
    assert!(
        exterior > 20,
        "The brackets must extend outside the original silhouette"
    );
    for y in center.1 - 8..=center.1 + 8 {
        for x in center.0 - 8..=center.0 + 8 {
            for (actual, original) in selected
                .get_pixel(x, y)
                .0
                .into_iter()
                .zip(baseline.get_pixel(x, y).0)
            {
                assert!(
                    actual.abs_diff(original) <= 2,
                    "Selected object interior was tinted at {x},{y}"
                );
            }
        }
    }
    // The middle of each edge stays open: this is a corner frame, not a
    // continuous silhouette or a full wireframe box.
    let bracket_bounds = bounds(&selected, amber);
    let mid_x = (bracket_bounds[0] + bracket_bounds[2]) / 2;
    for y in bracket_bounds[1]..=bracket_bounds[3] {
        if y.abs_diff(center.1) > 12 {
            assert!(
                !amber(selected.get_pixel(mid_x, y)),
                "The edge midpoint must remain open"
            );
        }
    }

    app.world_mut().resource_mut::<Editor>().selection = None;
    let deselected = capture_when(&mut app, &target, &output.join("deselected.png"), |image| {
        amber_count(image) == 0 && blue(image.get_pixel(center.0, center.1))
    });
    assert_eq!(
        deselected.get_pixel(center.0, center.1),
        baseline.get_pixel(center.0, center.1)
    );

    app.world_mut().resource_mut::<Editor>().selection = Some(Selection::Model(1));
    let brackets_bounds = bounds(&selected, amber);
    let brackets_pixels = amber_count(&selected);
    let cover_position = app.world().get::<Transform>(cover).unwrap().translation;
    let cover_is_visible = |image: &image::RgbaImage| {
        let [red, green, blue, _] = image.get_pixel(center.0, center.1).0.map(u16::from);
        green > red + 50 && green > blue + 50
    };
    for (name, offset, partially_visible) in [
        ("partially-occluded", 1.1, true),
        ("fully-occluded", 0.0, false),
    ] {
        app.world_mut()
            .get_mut::<Transform>(cover)
            .unwrap()
            .translation = cover_position + Vec3::X * offset;
        *app.world_mut().get_mut::<Visibility>(cover).unwrap() = Visibility::Visible;
        let occluded = capture_when(
            &mut app,
            &target,
            &output.join(format!("{name}.png")),
            |image| {
                let blue_pixels = image.pixels().filter(|pixel| blue(pixel)).count();
                cover_is_visible(image)
                    && amber_count(image) > 32
                    && if partially_visible {
                        blue_pixels > 100
                    } else {
                        blue_pixels == 0
                    }
            },
        );
        for (actual, expected) in bounds(&occluded, amber).into_iter().zip(brackets_bounds) {
            assert!(
                actual.abs_diff(expected) <= 1,
                "The full object dimensions must remain visible when {name}"
            );
        }
        assert!(
            amber_count(&occluded) * 10 >= brackets_pixels * 9,
            "Occluding geometry must not hide the selection brackets when {name}"
        );
    }
    app.world_mut().resource_mut::<Editor>().selection = None;
    capture_when(
        &mut app,
        &target,
        &output.join("occluded-deselected.png"),
        |image| cover_is_visible(image) && amber_count(image) == 0,
    );
}
