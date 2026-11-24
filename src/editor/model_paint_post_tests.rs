use super::*;
use crate::editor::tests::lighting_ui_render::fixture;

#[test]
#[ignore = "requires GPU; checks individual viewport effects against a shared HDR input"]
fn gpu_post_effects_grade_and_bloom_the_same_hdr_frame() {
    let (app, _, _) = fixture();
    let device = app.world().resource::<RenderDevice>();
    let queue = app.world().resource::<RenderQueue>();
    let mut processor = Processor::new(device, queue);
    let size = 64_u32;
    let extent = Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: 1,
    };
    let descriptor = |format, usage| TextureDescriptor {
        label: Some("Paint post test"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    };
    let pixels: Vec<_> = (0..size)
        .flat_map(|y| {
            (0..size).flat_map(move |x| {
                let value = if x.abs_diff(32) <= 1 && y.abs_diff(32) <= 1 {
                    [8.0, 8.0, 8.0, 1.0]
                } else if x < 16 {
                    [0.66, 0.31, 0.13, 1.0]
                } else {
                    [0.13, 0.31, 0.66, 1.0]
                };
                value
                    .into_iter()
                    .flat_map(|value| half::f16::from_f32(value).to_le_bytes())
            })
        })
        .collect();
    let hdr = device
        .create_texture_with_data(
            queue,
            &descriptor(TextureFormat::Rgba16Float, TextureUsages::TEXTURE_BINDING),
            TextureDataOrder::LayerMajor,
            &pixels,
        )
        .create_view(&default());
    let output = device.create_texture(&descriptor(
        TextureFormat::Rgba8UnormSrgb,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    ));
    let view = output.create_view(&default());
    let mut render = |effects| {
        let mut encoder = device.create_command_encoder(&default());
        processor.record(
            device,
            &mut encoder,
            &hdr,
            &view,
            UVec2::splat(size),
            Grade {
                exposure: 0.0,
                curve: crate::ToneMap::Linear,
                effects,
            },
            true,
        );
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("Paint post test readback"),
            size: u64::from(size * size * 4),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
            },
            extent,
        );
        let completed = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (send, receive) = std::sync::mpsc::channel();
        slice.map_async(MapMode::Read, move |result| send.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(completed),
                timeout: Some(std::time::Duration::from_secs(20)),
            })
            .unwrap();
        receive.recv().unwrap().unwrap();
        slice.get_mapped_range().unwrap().to_vec()
    };
    let baseline = render(Effects::default());
    for (field, _) in Field::ALL {
        let mut effects = Effects::default();
        field.set(
            &mut effects,
            match field {
                Field::Saturation => 0.0,
                Field::Contrast => 1.5,
                _ => 1.0,
            },
        );
        if field == Field::BloomThreshold {
            effects.bloom = 1.0;
            let lit = render(effects);
            effects.bloom_threshold = 4.0;
            assert_ne!(lit, render(effects));
        } else {
            let changed = render(effects);
            assert_ne!(baseline, changed, "{field:?}");
        }
    }
    let bloom = render(Effects {
        bloom: 1.0,
        ..default()
    });
    let halo = ((39 * size + 32) * 4) as usize;
    assert!(
        bloom[halo] > baseline[halo],
        "bloom should spread beyond the bright pixels"
    );
    let gray = render(Effects {
        saturation: 0.0,
        ..default()
    });
    assert!(gray.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]));
    let filmic = render(Effects {
        curve: Some(crate::ToneMap::BlenderFilmic),
        ..default()
    });
    assert_ne!(baseline, filmic);
    assert_eq!(baseline, render(Effects::default()));
}
