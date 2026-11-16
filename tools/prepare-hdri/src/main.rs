use prepare_hdri::{Panorama, SOURCES, diffuse_cube, ktx, specular_mips, validate_options};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    time::Instant,
};

struct Options {
    source: PathBuf,
    output: PathBuf,
    size: u32,
    samples: u32,
}
fn options() -> Result<Option<Options>, String> {
    let mut args = std::env::args_os().skip(1);
    let mut source = None;
    let mut output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/hdri");
    let (mut size, mut samples) = (256, 256);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!(
                "Prepare the ten bundled CC0 HDR environments (Rust, no Python).\nUsage: prepare-hdri --source DIR [--output DIR] [--size 256] [--samples 256]\nInput: <name>_1k.hdr; output: <name>.diffuse.ktx2 and <name>.specular.ktx2.\nDiffuse is 32x32; --size controls specular faces and must be a power of two."
            );
            return Ok(None);
        }
        let text = arg.to_string_lossy();
        let (key, inline) = text
            .split_once('=')
            .map_or((text.as_ref(), None), |(k, v)| (k, Some(OsString::from(v))));
        if !matches!(key, "--source" | "--output" | "--size" | "--samples") {
            return Err(format!("Unknown option: {text}"));
        }
        let value = inline
            .or_else(|| args.next())
            .ok_or_else(|| format!("Missing value for {key}"))?;
        match key {
            "--source" => source = Some(PathBuf::from(value)),
            "--output" => output = value.into(),
            "--size" => {
                size = value
                    .to_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("Invalid --size")?
            }
            "--samples" => {
                samples = value
                    .to_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or("Invalid --samples")?
            }
            _ => unreachable!(),
        }
    }
    validate_options(size, samples).map_err(|e| e.to_string())?;
    Ok(Some(Options {
        source: source.ok_or("--source is required (use --help)")?,
        output,
        size,
        samples,
    }))
}
fn run(o: Options) -> io::Result<()> {
    // Check the entire fixed set before writing anything to the output folder.
    for name in SOURCES {
        let file = o.source.join(format!("{name}_1k.hdr"));
        if !file.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Missing panorama: {}", file.display()),
            ));
        }
    }
    std::fs::create_dir_all(&o.output)?;
    let start = Instant::now();
    for (i, name) in SOURCES.into_iter().enumerate() {
        println!("Baking {name} ({}/{})", i + 1, SOURCES.len());
        let image = Panorama::read(&o.source.join(format!("{name}_1k.hdr")))?;
        let diffuse = diffuse_cube(&image);
        let specular = specular_mips(image, o.size, o.samples)?;
        ktx::write(&o.output.join(format!("{name}.diffuse.ktx2")), &[diffuse])?;
        ktx::write(&o.output.join(format!("{name}.specular.ktx2")), &specular)?;
    }
    println!(
        "Finished all ten environments in {:.1}s: {}",
        start.elapsed().as_secs_f64(),
        Path::new(&o.output).display()
    );
    Ok(())
}
fn main() {
    let result = options().and_then(|o| o.map_or(Ok(()), |o| run(o).map_err(|e| e.to_string())));
    if let Err(error) = result {
        eprintln!("HDRI preparation: {error}");
        std::process::exit(1);
    }
}
