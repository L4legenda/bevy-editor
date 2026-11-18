//! Same fixture, camera, stamp stream and populated layers as Material Painter.
use super::*;
use std::time::Instant;

#[test]
#[ignore = "requires GPU and tools/paint-perf/prepare.py; measures actual completed paint work"]
fn gpu_compare_complex_paint() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/paint-perf");
    let bytes =
        std::fs::read(root.join("complex.bin")).expect("run tools/paint-perf/prepare.py first");
    let mut vertices = Vec::with_capacity(bytes.len());
    for (i, p) in bytes.chunks_exact(32).enumerate() {
        vertices.extend_from_slice(&p[..12]);
        vertices.extend_from_slice(&p[24..32]);
        for id in [i as u32 / 3 + 1, 1, 0] {
            vertices.extend_from_slice(&id.to_le_bytes());
        }
    }
    let scene = Scene {
        count: (vertices.len() / 32) as u32,
        vertices,
    };
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture_project(project);
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let pipelines = Pipelines::new(device);
    let wait = || {
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
    };
    let mut report = Vec::new();
    let selected = std::env::var("PAINT_PERF_SELECTED")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(7);
    for size in [1024, 2048] {
        let surface = Surface {
            size,
            base: std::array::from_fn(|_| data::solid(size, [128, 128, 128, 255])),
            material: serde_json::json!({}),
            stack: Stack {
                selected,
                layers: (0..8)
                    .map(|i| {
                        let mut l = Layer::new(format!("Layer {i}"), LayerKind::Paint);
                        l.channels[0] = Some(data::solid(size, [60 + i * 5, 70, 80, 30]));
                        l
                    })
                    .collect(),
            },
        };
        let mut images = RenderAssets::<GpuImage>::default();
        let ids = std::array::from_fn(|i| {
            let format = if i == 0 {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            };
            let descriptor = TextureDescriptor {
                label: Some("benchmark image"),
                size: Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::COPY_SRC
                    | TextureUsages::COPY_DST
                    | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };
            let texture = device.create_texture(&descriptor);
            queue.write_texture(
                texture.as_image_copy(),
                &surface.base[0],
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
                descriptor.size,
            );
            let texture_view = texture.create_view(&TextureViewDescriptor::default());
            let id = AssetId::<Image>::Uuid {
                uuid: bevy::asset::uuid::Uuid::new_v4(),
            };
            images.insert(
                id,
                GpuImage {
                    texture,
                    texture_view,
                    sampler: device.create_sampler(&SamplerDescriptor::default()),
                    texture_descriptor: descriptor,
                    texture_view_descriptor: None,
                    had_data: false,
                },
            );
            id
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("paint comparison setup"),
        });
        let mut context = Context::new(
            device,
            &pipelines,
            &mut encoder,
            &surface,
            0,
            ids,
            &scene,
            1,
            0,
        );
        let pose = Transform::from_xyz(0., 0., 4.);
        let view = View {
            uv: false,
            camera: CameraMap {
                pose,
                inverse: pose.to_matrix().inverse(),
                rect: Rect::from_corners(Vec2::ZERO, Vec2::new(1024., 768.)),
                ortho: false,
                height: 8. * (FOV * 0.5).tan(),
                depth: 100.,
                epsilon: 0.0005,
            },
        };
        let brush = Brush {
            channel: 0,
            color: [230, 26, 13],
            radius: 35.,
            hardness: 0.6,
            flow: 0.35,
            opacity: 1.,
            erase: false,
            fill: false,
        };
        context.begin(device, &pipelines, &mut encoder, brush, view);
        queue.submit([encoder.finish()]);
        wait();
        let mut times = Vec::new();
        let mut cpu = Vec::new();
        for frame in 0..200 {
            let start = Instant::now();
            let points = (0..6)
                .map(|i| {
                    let t = (frame * 6 + i) as f32 * 0.011;
                    Vec2::new(512. + t.sin() * 90., 384. + (t * 1.7).cos() * 65.)
                })
                .collect::<Vec<_>>();
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("paint comparison frame"),
            });
            context.points(device, &pipelines, &mut encoder, &points, &images);
            queue.submit([encoder.finish()]);
            let submit = start.elapsed().as_secs_f64() * 1000.;
            wait();
            if frame >= 20 {
                cpu.push(submit);
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        let readback = empty(
            device,
            "benchmark final image",
            size as u64 * size as u64 * 4,
            BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            images.get(ids[0]).unwrap().texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
            },
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(MapMode::Read, |r| r.unwrap());
        wait();
        let data = readback.slice(..).get_mapped_range().unwrap().to_vec();
        readback.unmap();
        let layer_readback = empty(
            device,
            "benchmark layer validation",
            size as u64 * size as u64 * 4,
            BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(
            &context.composition.layers[selected].buffer,
            0,
            &layer_readback,
            0,
            size as u64 * size as u64 * 4,
        );
        queue.submit([encoder.finish()]);
        layer_readback
            .slice(..)
            .map_async(MapMode::Read, |r| r.unwrap());
        wait();
        let layer_data = layer_readback
            .slice(..)
            .get_mapped_range()
            .unwrap()
            .to_vec();
        layer_readback.unmap();
        let mut expected = surface.clone();
        expected.stack.layers[selected].channels[0] = Some(Arc::new(layer_data));
        let full = expected.composite(0);
        let mismatch = data
            .iter()
            .zip(&full)
            .enumerate()
            .find(|(_, (a, b))| a.abs_diff(**b) > 1);
        assert!(
            mismatch.is_none(),
            "sparse composite must match a full CPU recomposite, including untouched texels and layers above the active one: {mismatch:?}"
        );
        assert!(
            data.chunks_exact(4).any(|p| p[0] > p[1] + 30),
            "benchmark must paint visible pixels"
        );
        let name = std::env::var("PAINT_PERF_LABEL").unwrap_or_else(|_| "bevy-editor".into());
        image::save_buffer(
            root.join(format!("{name}-{size}.png")),
            &data,
            size,
            size,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        times.sort_by(f64::total_cmp);
        cpu.sort_by(f64::total_cmp);
        let row = serde_json::json!({"app":name,"triangles":scene.count/3,"texture":size,"layers":8,"selected":selected,"frames":times.len(),"stamps_per_frame":6,"mean_ms":times.iter().sum::<f64>()/times.len() as f64,"p50_ms":times[times.len()/2],"p95_ms":times[times.len()*95/100],"cpu_mean_ms":cpu.iter().sum::<f64>()/cpu.len() as f64});
        eprintln!("PAINT_COMPARE {row}");
        report.push(row);
    }
    let name = std::env::var("PAINT_PERF_LABEL").unwrap_or_else(|_| "bevy-editor".into());
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
