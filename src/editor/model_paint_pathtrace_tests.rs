use super::*;
use crate::editor::{
    model_document::{Document, glb::Glb},
    model_mesh::{Corner, Face, Geometry},
    tests::lighting_ui_render::fixture,
};
use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::math::proj;
use std::{path::Path, time::Duration};

#[test]
fn path_trace_cdf_favors_brighter_regions_and_covers_black_texels() {
    let cdf = environment_distribution(2, 1, &[Vec3::ZERO, Vec3::splat(10.)]);
    assert_eq!(
        cdf.len(),
        1 + CDF_HEIGHT + 1 + CDF_HEIGHT * (2 * CDF_WIDTH + 1)
    );
    assert_eq!(cdf[1], 0.0);
    assert_eq!(cdf[1 + CDF_HEIGHT], 1.0);
    let middle_row = 2 + CDF_HEIGHT + (CDF_HEIGHT / 2) * (CDF_WIDTH + 1);
    assert!((0.0..0.001).contains(&cdf[middle_row + CDF_WIDTH / 2]));
    assert!(cdf[middle_row + CDF_WIDTH / 2] > 0.0);
    assert_eq!(cdf[middle_row + CDF_WIDTH], 1.0);
}

#[test]
fn path_trace_reads_the_editor_default_hdr_cubemap() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/hdri/kloofendal_48d_partly_cloudy_puresky.specular.ktx2");
    let image = Image::from_buffer(
        &std::fs::read(path).unwrap(),
        ImageType::Extension("ktx2"),
        CompressedImageFormats::NONE,
        false,
        ImageSampler::linear(),
        RenderAssetUsages::all(),
    )
    .unwrap();
    let sky = project_cubemap(&image).unwrap();
    assert_eq!(sky.len(), PANORAMA_WIDTH * PANORAMA_HEIGHT);
    assert!(sky.iter().all(|color| color.is_finite()));
    // Coarse point sampling of this HDRI missed almost all of the sun.
    assert!(sky.iter().any(|color| color.max_element() > 10_000.0));
    let horizon = PANORAMA_HEIGHT / 2 * PANORAMA_WIDTH;
    assert!(
        sky[..PANORAMA_WIDTH]
            .iter()
            .zip(&sky[horizon..])
            .any(|(zenith, horizon)| zenith.distance(*horizon) > 0.1)
    );
    for (index, axis) in [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(face_coordinates(axis), (index, Vec2::ZERO));
    }
}

pub(super) fn texture(
    device: &RenderDevice,
    queue: &RenderQueue,
    format: TextureFormat,
    layers: u32,
    bytes: &[u8],
    dimension: TextureViewDimension,
) -> TextureView {
    let size = Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: layers,
    };
    device
        .create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("Path trace test input"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            bytes,
        )
        .create_view(&TextureViewDescriptor {
            dimension: Some(dimension),
            ..default()
        })
}

pub(super) fn read(device: &RenderDevice, queue: &RenderQueue, target: &Target) -> Vec<[f32; 4]> {
    let stride = (target.size.x * 8).div_ceil(256) * 256;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Path trace test readback"),
        size: u64::from(stride) * u64::from(target.size.y),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_texture_to_buffer(
        target.output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(target.size.y),
            },
        },
        Extent3d {
            width: target.size.x,
            height: target.size.y,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    let slice = buffer.slice(..);
    slice.map_async(MapMode::Read, move |result| send.send(result).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    receive
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    slice
        .get_mapped_range()
        .unwrap()
        .chunks_exact(stride as usize)
        .flat_map(|row| row[..target.size.x as usize * 8].chunks_exact(8))
        .map(|pixel| {
            std::array::from_fn(|i| {
                half::f16::from_le_bytes([pixel[i * 2], pixel[i * 2 + 1]]).to_f32()
            })
        })
        .collect()
}

fn document() -> Document {
    let source = Glb::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model-edit/warped-cube.glb"),
    )
    .unwrap();
    let mut geometry = Geometry::default();
    for (slot, x) in [-0.8, 0.0, 8.0, 9.0, 10.0].into_iter().enumerate() {
        let first = geometry.positions.len();
        geometry.positions.extend([
            Vec3::new(x, -0.65, 0.0),
            Vec3::new(x + 0.8, -0.65, 0.0),
            Vec3::new(x + 0.8, 0.65, 0.0),
            Vec3::new(x, 0.65, 0.0),
        ]);
        geometry.faces.push(Face {
            corners: [Vec2::Y, Vec2::ONE, Vec2::X, Vec2::ZERO]
                .into_iter()
                .enumerate()
                .map(|(i, uv)| Corner {
                    uv,
                    ..Corner::new(first + i)
                })
                .collect(),
            material: slot % 2,
            smooth: false,
        });
    }
    Document::new(
        source,
        vec![Object {
            id: 17,
            source_node: None,
            name: "Path trace test".into(),
            geometry: geometry.into(),
            transform: Transform::IDENTITY,
            materials: Vec::new(),
            hidden: false,
        }],
    )
}

#[test]
#[ignore = "requires a GPU with compute storage and RGBA16F writes"]
fn gpu_paint_path_trace_accumulates_distinct_materials_and_resets() {
    let (app, _, _) = fixture();
    let device = app.world().resource::<RenderDevice>();
    let queue = app.world().resource::<RenderQueue>();
    let pipeline = Pipeline::new(device).unwrap();
    let scene = Scene::new(device, &document().objects, |object, slot| {
        assert_eq!(object, 17);
        Some(slot as u32)
    })
    .unwrap();
    let rgba = |format, bytes: [u8; 8]| {
        texture(
            device,
            queue,
            format,
            2,
            &bytes,
            TextureViewDimension::D2Array,
        )
    };
    let color = rgba(
        TextureFormat::Rgba8UnormSrgb,
        [50, 0, 0, 255, 0, 50, 0, 255],
    );
    let emission = rgba(
        TextureFormat::Rgba8UnormSrgb,
        [220, 0, 0, 255, 0, 220, 0, 255],
    );
    let neutral = rgba(TextureFormat::Rgba8Unorm, [128; 8]);
    let zero = rgba(TextureFormat::Rgba8Unorm, [0; 8]);
    let normal = rgba(
        TextureFormat::Rgba8Unorm,
        [128, 128, 255, 255, 128, 128, 255, 255],
    );
    let sky = Vec3::new(0.15, 0.25, 0.5);
    let hdr: Vec<_> = sky
        .extend(1.0)
        .to_array()
        .into_iter()
        .flat_map(|value| half::f16::from_f32(value).to_le_bytes())
        .collect();
    let hdr = hdr.repeat(6);
    let sky_view = texture(
        device,
        queue,
        TextureFormat::Rgba16Float,
        6,
        &hdr,
        TextureViewDimension::Cube,
    );
    let sky_image = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        hdr,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::all(),
    );
    let environment = Environment::from_cubemap(device, &sky_image, sky_view).unwrap();
    let eye = Vec3::Z * 3.0;
    let view = Mat4::from_translation(-eye);
    let projection = proj::orthographic(-1.2, 1.2, -1.0, 1.0, 0.01, 100.0);
    let mut frame = Frame {
        scene: &scene,
        maps: MaterialMaps {
            color: &color,
            roughness: &neutral,
            metallic: &zero,
            height: &zero,
            baked_normal: &normal,
            emission: &emission,
            normal: &normal,
            specular: &neutral,
            resolution: 1,
        },
        environment: &environment,
        camera: Camera {
            inverse_view_projection: (projection * view).inverse(),
            eye,
            orthographic: true,
        },
        lighting: Lighting {
            environment_rotation: 0.0,
            environment_intensity: 1.0,
            background: None,
            background_blur: 0.0,
            background_intensity: 1.0,
            key_direction: Vec3::Z,
            key_color: Vec3::ONE,
            key_intensity: 0.0,
        },
        settings: Settings {
            samples: 16,
            ..default()
        },
        revision: 0,
    };
    let size = UVec2::new(48, 40);
    let mut target = Target::new(device, size).unwrap();
    for sample in 1..=16 {
        assert!(target.step(device, queue, &pipeline, &frame));
        assert_eq!(target.samples(), sample);
    }
    assert!(!target.step(device, queue, &pipeline, &frame));
    let pixels = read(device, queue, &target);
    assert!(pixels.iter().flatten().all(|value| value.is_finite()));
    let [r, g, b, a] = pixels[20 * 48 + 16];
    assert!(
        r > 0.65 && g < 0.15 && b < 0.15 && a == 1.0,
        "{r} {g} {b} {a}"
    );
    let [r, g, b, a] = pixels[20 * 48 + 32];
    assert!(
        g > 0.65 && r < 0.15 && b < 0.15 && a == 1.0,
        "{r} {g} {b} {a}"
    );
    assert!((Vec3::from_slice(&pixels[0]) - sky).length() < 0.002);

    frame.revision += 1;
    assert!(target.step(device, queue, &pipeline, &frame));
    assert_eq!(target.samples(), 1);
    frame.camera.orthographic = false;
    frame.camera.inverse_view_projection =
        (proj::perspective(0.7, 1.2, 0.01, 100.0) * view).inverse();
    frame.lighting.background = Some(Vec3::new(0.05, 0.02, 0.01));
    assert!(target.step(device, queue, &pipeline, &frame));
    assert_eq!(target.samples(), 1);
    let pixels = read(device, queue, &target);
    assert!((Vec3::from_slice(&pixels[0]) - frame.lighting.background.unwrap()).length() < 0.0002);
}
