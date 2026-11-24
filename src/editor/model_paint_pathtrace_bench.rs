use super::*;
use crate::editor::{
    model_mesh::{Corner, Face, Geometry as MeshGeometry},
    tests::lighting_ui_render::fixture,
};
use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[test]
fn camera_motion_settles_after_gestures_wheel_and_projection_changes() {
    let mut motion = CameraMotion::default();
    let stamp = ViewStamp {
        model: 0,
        pose: Transform::from_xyz(0.0, 0.0, 6.0),
        viewport: Vec2::new(900.0, 480.0),
        orthographic: false,
    };
    let ms = Duration::from_millis;
    assert!(!motion.active(stamp, false, ms(0)));
    assert!(motion.active(stamp, true, ms(40)));
    assert!(motion.active(stamp, true, ms(200)));
    assert!(motion.active(stamp, false, ms(299)));
    assert!(!motion.active(stamp, false, ms(300)));
    let zoom = ViewStamp {
        pose: Transform::from_xyz(0.0, 0.0, 5.0),
        ..stamp
    };
    assert!(motion.active(zoom, false, ms(310)));
    let orthographic = ViewStamp {
        orthographic: true,
        ..zoom
    };
    assert!(motion.active(orthographic, false, ms(320)));
    assert!(motion.active(orthographic, false, ms(419)));
    assert!(!motion.active(
        ViewStamp {
            model: 1,
            ..orthographic
        },
        false,
        ms(419)
    ));
}

const CHECKPOINTS: [u32; 5] = [1, 8, 32, 128, 256];
const COLORS: [[u8; 3]; 2] = [[185, 105, 57], [37, 118, 164]];
const ROUGHNESS: [u8; 2] = [115, 177];
const METALLIC: [u8; 2] = [108, 0];
const BACKGROUND: [f32; 3] = [0.025, 0.03, 0.04];

fn torus() -> MeshGeometry {
    let (rings, sections) = (192_usize, 96_usize);
    let mut mesh = MeshGeometry::default();
    let vertex = |u, v| u * (sections + 1) + v;
    let angles = |u, v| {
        let angle = |value, count| value as f32 / count as f32 * std::f32::consts::TAU;
        (angle(u, rings), angle(v, sections))
    };
    let normal = |u, v| {
        let (u, v) = angles(u, v);
        Vec3::new(u.cos() * v.cos(), v.sin(), u.sin() * v.cos())
    };
    for u in 0..=rings {
        let a = angles(u, 0).0;
        let ring = Vec3::new(a.cos(), 0.0, a.sin()) * 0.96;
        mesh.positions
            .extend((0..=sections).map(|v| ring + normal(u, v) * 0.38));
    }
    for u in 0..rings {
        for v in 0..sections {
            mesh.faces.push(Face {
                corners: [(u, v), (u, v + 1), (u + 1, v + 1), (u + 1, v)]
                    .into_iter()
                    .map(|(u, v)| Corner {
                        uv: Vec2::new(u as f32 / rings as f32, v as f32 / sections as f32),
                        source_normal: Some(normal(u, v)),
                        ..Corner::new(vertex(u, v))
                    })
                    .collect(),
                material: usize::from(v >= sections / 2),
                smooth: true,
            });
        }
    }
    mesh
}

fn shared_geometry(directory: &Path) -> Object {
    let mesh = torus();
    let items: Vec<_> = mesh
        .buffers()
        .into_iter()
        .map(|(slot, part)| {
            json!({
                "slot": slot, "positions": part.positions, "normals": part.normals,
                "uv": part.uv, "indices": part.indices,
            })
        })
        .collect();
    fs::write(
        directory.join("mesh.json"),
        serde_json::to_vec(&items).unwrap(),
    )
    .unwrap();
    Object {
        id: 0,
        source_node: None,
        name: "Comparison torus".into(),
        geometry: mesh.into(),
        transform: Transform::IDENTITY,
        materials: Vec::new(),
        hidden: false,
    }
}

fn shared_hdr(directory: &Path, device: &RenderDevice, queue: &RenderQueue) -> Environment {
    use super::super::{PANORAMA_HEIGHT, PANORAMA_WIDTH, project_cubemap};
    let image = Image::from_buffer(
        include_bytes!("../../assets/hdri/kloofendal_48d_partly_cloudy_puresky.specular.ktx2"),
        ImageType::Extension("ktx2"),
        CompressedImageFormats::NONE,
        false,
        ImageSampler::linear(),
        RenderAssetUsages::all(),
    )
    .unwrap();
    let panorama: Vec<_> = project_cubemap(&image)
        .unwrap()
        .into_iter()
        .map(|rgb| image::Rgb(rgb.to_array()))
        .collect();
    image::codecs::hdr::HdrEncoder::new(
        fs::File::create(directory.join("editor-sky.hdr")).unwrap(),
    )
    .encode(&panorama, PANORAMA_WIDTH, PANORAMA_HEIGHT)
    .unwrap();
    let view = device
        .create_texture_with_data(
            queue,
            &image.texture_descriptor,
            image.data_order,
            image.data.as_ref().unwrap(),
        )
        .create_view(image.texture_view_descriptor.as_ref().unwrap());
    Environment::from_cubemap(device, &image, view).unwrap()
}

fn maps() -> Vec<Layer> {
    (0..2)
        .map(|index| {
            let color = Vec3::from_array(COLORS[index].map(|value| value as f32 / 255.0));
            let mut layer = Layer::original(
                (0, index),
                &StandardMaterial {
                    base_color: Color::srgb(color.x, color.y, color.z),
                    perceptual_roughness: ROUGHNESS[index] as f32 / 255.0,
                    metallic: METALLIC[index] as f32 / 255.0,
                    ..default()
                },
            );
            layer.maps[4].factor = Vec4::splat(128.0 / 255.0);
            layer
        })
        .collect()
}

#[test]
#[ignore = "requires GPU; times the editor's real Alt-left orbit on a two-material mesh"]
fn gpu_measure_paint_trace_during_camera_orbit() {
    use crate::editor::{
        Action,
        model_document::MaterialSlot,
        selection_outline::render_tests::frame,
        tests::lighting_ui_render::{mouse, move_pointer},
    };
    use bevy::{
        input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
        },
        ui::UiGlobalTransform,
    };

    let (mut app, window, _) = fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<editing::State>()
            .documents
            .contains_key(&0)
        {
            break;
        }
    }
    let geometry = torus();
    let triangles: usize = geometry
        .buffers()
        .values()
        .map(|mesh| mesh.indices.len() / 3)
        .sum();
    let slots = {
        let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
        COLORS
            .iter()
            .enumerate()
            .map(|(index, [r, g, b])| MaterialSlot {
                bake_fixes: Default::default(),
                material: materials.add(StandardMaterial {
                    base_color: Color::srgb_u8(*r, *g, *b),
                    perceptual_roughness: ROUGHNESS[index] as f32 / 255.0,
                    metallic: METALLIC[index] as f32 / 255.0,
                    ..default()
                }),
                source: json!({ "name": format!("Set {}", index + 1) }),
                paint: None,
                authored: None,
            })
            .collect()
    };
    {
        let mut state = app.world_mut().resource_mut::<editing::State>();
        let doc = state.documents.get_mut(&0).unwrap();
        doc.objects[0].geometry = geometry.into();
        doc.objects[0].transform = Transform::IDENTITY;
        doc.objects[0].materials = slots;
        doc.revision += 1;
        doc.mode = Mode::Paint;
    }
    frame(&mut app);
    {
        let mut paint = app.world_mut().resource_mut::<PaintState>();
        let session = paint.sessions.get_mut(&0).unwrap();
        session.view = View::Three;
        session.path_trace = true;
        session.trace_settings.samples = 16;
        session.revision += 1;
    }
    let initial_view = |app: &mut App| {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::ZERO;
        viewer.distance = 4.6;
        viewer.yaw = 0.56;
        viewer.pitch = 0.61;
        viewer.orthographic = false;
    };
    let settle = |app: &mut App| {
        for _ in 0..120 {
            frame(app);
            if samples(app.world()) == Some(16) {
                return;
            }
        }
        panic!("The traced camera did not settle");
    };
    initial_view(&mut app);
    settle(&mut app);
    let (center, pixels) = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        (
            transform.affine().translation * node.inverse_scale_factor(),
            node.size().round().as_uvec2(),
        )
    };
    let image = |app: &mut App| {
        let world = app.world_mut();
        world
            .query_filtered::<&ImageNode, With<CanvasImage>>()
            .single(world)
            .unwrap()
            .image
            .id()
    };
    let raster = app.world().resource::<ViewerUi>().image.id();
    let traced = image(&mut app);
    assert_ne!(raster, traced);
    let alt = |app: &mut App, state| {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::AltLeft,
            logical_key: Key::Alt,
            state,
            text: None,
            repeat: false,
            window,
        });
        frame(app);
    };

    let mut runs = Vec::new();
    // Alternate the run order to reduce bias from device clocks and caching.
    for pause in [false, true, true, false] {
        app.world_mut().resource_mut::<Main>().bypass_camera_pause = !pause;
        initial_view(&mut app);
        settle(&mut app);
        move_pointer(&mut app, window, center);
        alt(&mut app, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Pressed);
        let start_yaw = app.world().resource::<Viewer>().yaw;
        let start_samples = dispatches(app.world());
        let mut times = Vec::new();
        for tick in 0..96 {
            let offset = if tick < 48 { tick } else { 96 - tick };
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(center + Vec2::new(offset as f32 * 2.5, 0.0)));
            let start = Instant::now();
            frame(&mut app);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        assert!((app.world().resource::<Viewer>().yaw - start_yaw).abs() > 0.001);
        let during_orbit = dispatches(app.world()) - start_samples;
        if pause {
            assert_eq!(during_orbit, 0);
            assert_eq!(image(&mut app), raster);
        } else {
            assert!(during_orbit >= 90);
            assert_eq!(image(&mut app), traced);
        }
        mouse(&mut app, window, ButtonState::Released);
        alt(&mut app, ButtonState::Released);
        if pause {
            assert_eq!(image(&mut app), raster);
        }
        let mut recovery_frames = 0;
        while image(&mut app) != traced && recovery_frames < 30 {
            frame(&mut app);
            recovery_frames += 1;
        }
        assert_eq!(
            image(&mut app),
            traced,
            "restore the same trace target after orbit"
        );
        if pause {
            assert!(recovery_frames > 0);
            assert!(samples(app.world()).is_some_and(|count| count < 16));
        }
        let mut ordered = times.clone();
        ordered.sort_by(f64::total_cmp);
        runs.push(json!({
            "pause_during_camera_motion": pause,
            "traced_frames_during_orbit": during_orbit,
            "median_ms": ordered[ordered.len() / 2],
            "p95_ms": ordered[(ordered.len() - 1) * 95 / 100],
            "frames_after_release_until_trace_returns": recovery_frames,
            "frame_ms": times,
        }));
    }
    let adapter = app
        .world()
        .resource::<bevy::render::renderer::RenderAdapterInfo>();
    let report = json!({
        "adapter": format!("{} {:?}", adapter.name, adapter.backend),
        "canvas_size": pixels.to_array(),
        "triangles": triangles,
        "frames_per_run": 96,
        "runs": runs,
    });
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/paint-perf/path-trace");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("interactive-orbit.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    for run in report["runs"].as_array().unwrap() {
        println!(
            "Pause: {} | samples: {} | median: {:.2} ms | p95: {:.2} ms | resume: {} frames",
            run["pause_during_camera_motion"],
            run["traced_frames_during_orbit"],
            run["median_ms"].as_f64().unwrap(),
            run["p95_ms"].as_f64().unwrap(),
            run["frames_after_release_until_trace_returns"],
        );
    }
}

#[test]
#[ignore = "requires GPU; renders benchmark fixture shared with Material Painter"]
fn gpu_compare_paint_pathtrace() {
    let directory = PathBuf::from(std::env::var_os("PAINT_TRACE_DIR").unwrap());
    fs::create_dir_all(&directory).unwrap();
    let (app, _, _) = fixture();
    let world = app.world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let info = world.resource::<bevy::render::renderer::RenderAdapterInfo>();
    let programs = Programs::new(device, queue).unwrap();
    let size = UVec2::new(512, 320);
    let viewer = Viewer {
        yaw: 0.56,
        pitch: 0.61,
        distance: 4.6,
        center: Vec3::ZERO,
        ..default()
    };
    let fov = 0.7;
    let camera = view_pose(&viewer);
    let projection = proj::perspective(fov, size.x as f32 / size.y as f32, 0.02, 100.0);
    let object = shared_geometry(&directory);
    let triangles: usize = object
        .geometry
        .buffers()
        .values()
        .map(|mesh| mesh.indices.len() / 3)
        .sum();
    let scene = Scene::new(device, &[object], |_, slot| Some(slot as u32)).unwrap();
    let env = shared_hdr(&directory, device, queue);
    let mut arrays = Arrays::new(device, 2, 256);
    arrays.pack(device, queue, &programs, &maps(), &default(), &default());
    let [color, rm, normal, emission, specular] = &arrays.views;
    let frame = Frame {
        scene: &scene,
        maps: MaterialMaps {
            color,
            roughness: rm,
            metallic: rm,
            baked_normal: normal,
            height: rm,
            emission,
            normal,
            specular,
            resolution: 256,
        },
        environment: &env,
        camera: super::super::Camera {
            inverse_view_projection: (projection * camera.to_matrix().inverse()).inverse(),
            eye: camera.translation,
            orthographic: false,
        },
        lighting: Lighting {
            environment_rotation: 0.0,
            environment_intensity: 1.0,
            background: Some(Vec3::from_array(BACKGROUND)),
            background_blur: 0.0,
            background_intensity: 1.0,
            key_direction: Vec3::Y,
            key_color: Vec3::ONE,
            key_intensity: 0.0,
        },
        settings: Settings {
            precomposed: true,
            baked_normal: false,
            samples: *CHECKPOINTS.last().unwrap(),
            ..default()
        },
        revision: 0,
    };
    let output = device.create_texture(&output_image(size).texture_descriptor);
    let grade = Grade {
        exposure: 0.0,
        curve: crate::ToneMap::KhronosPbrNeutral,
        effects: default(),
    };
    let mut target = Target::new(device, size).unwrap();
    let mut post = Processor::new(device, queue);
    let mut step = |target: &mut Target| {
        assert!(target.step(device, queue, &programs.trace, &frame));
        show(device, queue, &mut post, &target.view, &output, size, grade);
        device.poll(PollType::wait_indefinitely()).unwrap();
    };
    // Warm the pipelines and resources. Preserving the target keeps allocation
    // and shader compilation outside the steady-state frame timings.
    for _ in 0..8 {
        step(&mut target);
    }
    target.key = None;
    let mut times = Vec::new();
    let mut checkpoints = Vec::new();
    for sample in 1..=frame.settings.samples {
        let start = Instant::now();
        step(&mut target);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        if CHECKPOINTS.contains(&sample) {
            let data: Vec<_> = super::super::tests::read(device, queue, &target)
                .into_iter()
                .flatten()
                .flat_map(|value| half::f16::from_f32(value).to_le_bytes())
                .collect();
            fs::write(directory.join(format!("bevy-{sample}.rgba16")), data).unwrap();
            checkpoints.push(json!({"samples":sample, "total_ms":times.iter().sum::<f64>()}));
        }
    }
    let settings = json!({
        "size": size.to_array(), "triangles": triangles,
        "yaw": viewer.yaw, "pitch": viewer.pitch, "distance": viewer.distance, "fov": fov,
        "colors": COLORS, "roughness": ROUGHNESS, "metallic": METALLIC,
        "background": BACKGROUND, "checkpoints": CHECKPOINTS,
        "bounces": frame.settings.bounces, "texture_resolution": arrays.size,
    });
    fs::write(
        directory.join("input.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    let report = json!({
        "adapter": format!("{} {:?}", info.name, info.backend),
        "frame_ms": times, "checkpoints": checkpoints,
    });
    fs::write(
        directory.join("bevy.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
