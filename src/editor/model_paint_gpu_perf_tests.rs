//! Same fixture, camera, stamp stream and populated layers as Material Painter.
use super::*;
use std::time::Instant;

#[test]
#[ignore = "requires GPU; compares directional blur and pixelation mask passes with the CPU"]
fn gpu_directional_blur_and_pixelate_match_cpu_mask() {
    use data::mask::{Filter, FilterKind, Mask};

    let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let pipelines = Pipelines::new(device);
    let size = 256u32;
    let bytes = u64::from(size).pow(2) * 4;
    let tiles = empty(device, "unused sparse tiles", 16, BufferUsages::STORAGE);
    let output = empty(device, "mask filter output", bytes,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let readback = empty(device, "mask filter readback", bytes,
        BufferUsages::COPY_DST | BufferUsages::MAP_READ);
    let pixels: Arc<Vec<_>> = Arc::new((0..size).flat_map(|y| {
        (0..size).flat_map(move |x| {
            let gray = if x < y / 2 || (70..112).contains(&x) && y < 40 { 255 } else { 0 };
            [gray, gray, gray, 255]
        })
    }).collect());
    let bounds: Vec<_> = [size; 4].into_iter().flat_map(u32::to_le_bytes).collect();
    for (kind, params) in [
        (FilterKind::DirectionalBlur, [8.5, 0.]),
        (FilterKind::DirectionalBlur, [8.5, 37.]),
        (FilterKind::DirectionalBlur, [8.5, 90.]),
        (FilterKind::Pixelate, [32., 0.]),
        (FilterKind::Pixelate, [37., 0.]),
    ] {
        let mut filter = Filter::new(kind);
        for (slot, value) in params.into_iter().enumerate() {
            filter.set_value(slot, value);
        }
        let mask = Mask::new(pixels.clone(), vec![filter]);
        let filters = MaskFilters::new(device, &pipelines, &mask, size, &output, &tiles).unwrap();
        queue.write_buffer(filters.bounds.as_ref().unwrap(), 0, &bounds);
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        for group in &filters.passes {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_pipeline(&pipelines.mask_filter);
            pass.set_bind_group(0, &**group, &[]);
            pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        let submission = queue.submit([encoder.finish()]);
        readback.slice(..).map_async(MapMode::Read, |result| result.unwrap());
        device.poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        }).unwrap();
        let gpu = readback.slice(..).get_mapped_range().unwrap();
        let max_error = gpu.iter().zip(mask.effective()).map(|(a, b)| a.abs_diff(*b)).max();
        let tolerance = u8::from(kind == FilterKind::DirectionalBlur);
        assert!(max_error.is_some_and(|error| error <= tolerance), "{kind:?} {params:?}, max error {max_error:?}");
        assert_ne!(&gpu[..], &pixels[..]);
        drop(gpu);
        readback.unmap();
    }
}

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
        vertices.extend_from_slice(&p[12..24]);
    }
    let scene = Scene {
        count: (vertices.len() / 44) as u32,
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
            baked: None,
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
            1,
            ids,
            &scene,
            1,
            0,
            4.,
            None,
            None,
        );
        let pose = Transform::from_xyz(0., 0., 4.);
        let view = View {
            uv: false,
            camera: CameraMap {
                pose,
                inverse: pose.to_matrix().inverse(),
                rect: Rect::from_corners(Vec2::ZERO, Vec2::new(1024., 768.)),
                clip: Rect::from_corners(Vec2::ZERO, Vec2::new(1024., 768.)),
                ortho: false,
                fov: FOV,
                height: 8. * (FOV * 0.5).tan(),
                depth: 100.,
                epsilon: 0.0005,
            },
        };
        let brush = Brush {
            channel: 0,
            color: [230, 26, 13],
            material: None,
            radius: 35.,
            hardness: 0.6,
            flow: 0.35,
            opacity: 1.,
            erase: false,
            fill: false,
            fill_mode: FillMode::default(),
            fill_angle: 30.,
            fill_padding: 0.,
            variation: stroke::Variation::default(),
            tip: None,
            grain: None,
            mirror: [0.; 4],
            projection: BrushProjection::Screen,
            paint_through: false,
            facing_limit: 85_f32.to_radians().cos(),
        };
        context.begin(device, &pipelines, &mut encoder, 0, brush, view, &images, None);
        queue.submit([encoder.finish()]);
        wait();
        let mut times = Vec::new();
        let mut cpu = Vec::new();
        for frame in 0..200 {
            let start = Instant::now();
            let points = (0..6)
                .map(|i| {
                    let t = (frame * 6 + i) as f32 * 0.011;
                    Stamp::from(Vec2::new(
                        512. + t.sin() * 90.,
                        384. + (t * 1.7).cos() * 65.,
                    ))
                })
                .collect::<Vec<_>>();
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("paint comparison frame"),
            });
            context.points(device, &pipelines, &mut encoder, &points, None, &images);
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
