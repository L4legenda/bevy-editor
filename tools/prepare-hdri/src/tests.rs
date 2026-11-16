use super::*;
use std::io::Read;
fn uniform(color: [f32; 3]) -> Panorama {
    Panorama::new(128, 64, vec![color; 128 * 64]).unwrap()
}
fn unpack(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    ruzstd::decoding::StreamingDecoder::new(bytes)
        .unwrap()
        .read_to_end(&mut result)
        .unwrap();
    result
}
fn value(raw: &[u8], index: usize) -> f64 {
    half::f16::from_bits(u16::from_le_bytes([raw[index * 2], raw[index * 2 + 1]])).to_f64()
}

#[test]
fn ktx_faces_keep_vulkan_orientation() {
    for (face, expected) in [
        [1., 0., 0.],
        [-1., 0., 0.],
        [0., 1., 0.],
        [0., -1., 0.],
        [0., 0., 1.],
        [0., 0., -1.],
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(cube_direction(face as u32, 0, 0, 1), expected);
    }
    let d = cube_direction(0, 0, 0, 16);
    assert!(d[0] > 0. && d[1] > 0. && d[2] > 0.);
}
#[test]
fn bilinear_sampling_wraps_seam_and_clamps_poles() {
    let panorama = Panorama::new(4, 2, (0..8).map(|i| [i as f32, 0., 0.]).collect()).unwrap();
    assert_eq!(panorama.sample_uv(0., 0.), [1.5, 0., 0.]);
    assert_eq!(panorama.sample_uv(1., 0.), panorama.sample_uv(0., 0.));
    assert_eq!(panorama.sample_uv(0.125, 1.), [4., 0., 0.]);
    assert_eq!(panorama.sample_uv(0.125, 0.5), [2., 0., 0.]);
}
#[test]
fn uniform_radiance_survives_sh_and_every_ggx_roughness_level() {
    let expected = [4., 2., 0.5];
    let diffuse = diffuse_cube(&uniform(expected));
    assert_eq!(diffuse.size, 32);
    let specular = specular_mips(uniform(expected), 16, 64).unwrap();
    assert_eq!(specular.len(), 5);
    for mip in std::iter::once(&diffuse).chain(&specular) {
        assert_eq!(mip.pixels.len(), 6 * mip.size as usize * mip.size as usize);
        for pixel in &mip.pixels {
            for c in 0..3 {
                assert!((pixel[c] - expected[c] as f64).abs() < 0.002, "{pixel:?}");
            }
        }
    }
}
#[test]
fn source_mips_crop_odd_edges_like_the_previous_baker() {
    let pano = Panorama::new(5, 3, (0..15).map(|i| [i as f32, 0., 0.]).collect()).unwrap();
    let mip = pano.half_size();
    assert_eq!((mip.width, mip.height), (2, 1));
    assert_eq!(mip.pixels, vec![[3., 0., 0.], [5., 0., 0.]]);
}
#[test]
fn ktx_layout_format_compression_and_all_channels_round_trip() {
    let mips: Vec<_> = (0..3)
        .map(|level| {
            let size = 4 >> level;
            CubeMip {
                size,
                pixels: (0..6 * size * size)
                    .map(|i| [(100 * level + i / (size * size)) as f64, 0.5, 2.])
                    .collect(),
            }
        })
        .collect();
    let bytes = ktx::encode(&mips).unwrap();
    let reader = ktx2::Reader::new(&bytes).unwrap();
    let h = reader.header();
    assert_eq!(h.format, Some(ktx2::Format::R16G16B16A16_SFLOAT));
    assert_eq!(h.face_count, 6);
    assert_eq!(h.pixel_width, 4);
    assert_eq!(h.pixel_height, 4);
    assert_eq!(h.level_count, 3);
    assert_eq!(
        h.supercompression_scheme,
        Some(ktx2::SupercompressionScheme::Zstandard)
    );
    for (level, frame) in reader.levels().enumerate() {
        let raw = unpack(frame.data);
        let mip = &mips[level];
        assert_eq!(raw.len() as u64, frame.uncompressed_byte_length);
        assert_eq!(raw.len(), mip.pixels.len() * 8);
        for (i, p) in mip.pixels.iter().enumerate() {
            for c in 0..3 {
                assert_eq!(value(&raw, i * 4 + c), p[c]);
            }
            assert_eq!(value(&raw, i * 4 + 3), 1.);
        }
        let offset =
            u64::from_le_bytes(bytes[80 + level * 24..88 + level * 24].try_into().unwrap());
        assert_eq!(offset % 8, 0);
    }
}
#[test]
fn half_float_output_clamps_like_the_previous_baker() {
    let bytes = ktx::encode(&[CubeMip {
        size: 1,
        pixels: vec![[-1., 1e20, 0.25]; 6],
    }])
    .unwrap();
    let reader = ktx2::Reader::new(&bytes).unwrap();
    let raw = unpack(reader.levels().next().unwrap().data);
    assert_eq!(
        (
            value(&raw, 0),
            value(&raw, 1),
            value(&raw, 2),
            value(&raw, 3)
        ),
        (0., 60000., 0.25, 1.)
    );
}
#[test]
fn invalid_images_options_and_mip_chains_are_rejected() {
    assert!(Panorama::new(0, 1, vec![]).is_err());
    assert!(Panorama::new(1, 1, vec![[f32::NAN, 0., 0.]]).is_err());
    for (size, samples) in [(0, 256), (3, 256), (8192, 256), (256, 0)] {
        assert!(validate_options(size, samples).is_err());
    }
    assert!(ktx::encode(&[]).is_err());
    assert!(
        ktx::encode(&[CubeMip {
            size: 2,
            pixels: vec![[0.; 3]; 6]
        }])
        .is_err()
    );
}
