//! Numerical comparison against the already bundled, pre-migration textures.
use prepare_hdri::SOURCES;
use std::{io::Read, path::PathBuf};
fn unpack(bytes: &[u8]) -> Vec<f64> {
    let mut raw = Vec::new();
    ruzstd::decoding::StreamingDecoder::new(bytes)
        .unwrap()
        .read_to_end(&mut raw)
        .unwrap();
    raw.chunks_exact(2)
        .map(|v| half::f16::from_bits(u16::from_le_bytes([v[0], v[1]])).to_f64())
        .collect()
}
#[test]
#[ignore = "run the Rust baker on the original ten panoramas and set PREPARE_HDRI_GENERATED"]
fn regenerated_maps_preserve_the_bundled_lighting_and_reflections() {
    let generated =
        PathBuf::from(std::env::var_os("PREPARE_HDRI_GENERATED").expect("PREPARE_HDRI_GENERATED"));
    let original = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/hdri");
    let mut worst = 0f64;
    for name in SOURCES {
        for kind in ["diffuse", "specular"] {
            let file = format!("{name}.{kind}.ktx2");
            let old = std::fs::read(original.join(&file)).unwrap();
            let new = std::fs::read(generated.join(&file)).unwrap();
            assert_eq!(&old[..80], &new[..80], "Container/format changed: {file}");
            let a = ktx2::Reader::new(&old).unwrap();
            let b = ktx2::Reader::new(&new).unwrap();
            let dfd = 80 + 24 * a.header().level_count as usize;
            assert_eq!(&old[dfd..dfd + 92], &new[dfd..dfd + 92]);
            let mut error = 0.;
            let mut energy = 0.;
            let mut maximum = 0f64;
            for (i, (a, b)) in a.levels().zip(b.levels()).enumerate() {
                assert_eq!(a.uncompressed_byte_length, b.uncompressed_byte_length);
                let a = unpack(a.data);
                let b = unpack(b.data);
                assert_eq!(a.len(), b.len());
                for (channel, (&a, &b)) in a.iter().zip(&b).enumerate() {
                    assert!(a.is_finite() && b.is_finite());
                    if channel % 4 == 3 {
                        assert_eq!(b, 1.);
                        continue;
                    }
                    let delta = (a - b).abs();
                    maximum = maximum.max(delta / (a.abs().max(b.abs()) + 0.01));
                    error += delta * delta;
                    energy += a * a;
                    assert!(
                        delta <= 0.02 * a.abs().max(b.abs()) + 0.001,
                        "{file} level {i}, channel {channel}: {a} != {b}"
                    );
                }
            }
            let nrmse = (error / energy.max(1e-20)).sqrt();
            worst = worst.max(nrmse);
            println!("{file}: relative RMS {nrmse:.8}, peak normalized error {maximum:.8}");
            assert!(nrmse < 0.002, "{file}: normalized RMS {nrmse}");
        }
    }
    println!("Twenty maps compared; worst relative RMS: {worst:.8}");
}
