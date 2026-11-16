use prepare_hdri::SOURCES;
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "bevy-hdri-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let temp = std::env::temp_dir().canonicalize().unwrap();
        if self.0.parent() == Some(temp.as_path())
            && self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("bevy-hdri-test-")
            && self.0.canonicalize().ok().as_ref() == Some(&self.0)
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
#[test]
fn all_twenty_maps_are_generated_without_python_on_path() {
    let temp = Sandbox::new();
    let input = temp.0.join("panoramas with spaces");
    let out = temp.0.join("maps");
    fs::create_dir(&input).unwrap();
    for name in SOURCES {
        let file = fs::File::create(input.join(format!("{name}_1k.hdr"))).unwrap();
        image::codecs::hdr::HdrEncoder::new(file)
            .encode(&vec![image::Rgb([4., 2., 0.5]); 128 * 64], 128, 64)
            .unwrap();
    }
    let result = Command::new(env!("CARGO_BIN_EXE_prepare-hdri"))
        .arg("--source")
        .arg(&input)
        .arg("--output")
        .arg(&out)
        .args(["--size=16", "--samples=32"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("Finished all ten environments"));
    assert_eq!(fs::read_dir(&out).unwrap().count(), 20);
    for name in SOURCES {
        for (kind, size, levels) in [("diffuse", 32, 1), ("specular", 16, 5)] {
            let bytes = fs::read(out.join(format!("{name}.{kind}.ktx2"))).unwrap();
            let reader = ktx2::Reader::new(bytes).unwrap();
            assert_eq!(reader.header().pixel_width, size);
            assert_eq!(reader.header().level_count, levels);
            for frame in reader.levels() {
                let mut raw = Vec::new();
                ruzstd::decoding::StreamingDecoder::new(frame.data)
                    .unwrap()
                    .read_to_end(&mut raw)
                    .unwrap();
                assert_eq!(raw.len() as u64, frame.uncompressed_byte_length);
                for (i, h) in raw.chunks_exact(2).enumerate() {
                    let value = half::f16::from_bits(u16::from_le_bytes([h[0], h[1]])).to_f32();
                    let expected = [4., 2., 0.5, 1.][i % 4];
                    assert!((value - expected).abs() < 0.01, "{name} {kind} {value}");
                }
            }
        }
    }
}
#[test]
fn help_invalid_options_and_missing_inputs_do_not_write_output() {
    assert!(
        Command::new(env!("CARGO_BIN_EXE_prepare-hdri"))
            .arg("--help")
            .status()
            .unwrap()
            .success()
    );
    let temp = Sandbox::new();
    let out = temp.0.join("maps");
    for extra in [vec!["--size", "3"], vec!["--samples", "0"], vec![]] {
        let result = Command::new(env!("CARGO_BIN_EXE_prepare-hdri"))
            .arg("--source")
            .arg(&temp.0)
            .arg("--output")
            .arg(&out)
            .args(extra)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!out.exists());
    }
}
