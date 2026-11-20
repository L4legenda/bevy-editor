mod regions;
mod tree;
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_char,
    io::{BufReader, Write},
    path::PathBuf,
};
#[derive(Serialize, Deserialize)]
struct Request {
    positions: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
    threshold: f64,
    resolution: u32,
    padding: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    existing_uv: Option<Vec<[[f64; 2]; 3]>>,
    #[serde(default)]
    options: Options,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Options {
    method: Method,
    materials: Vec<usize>,
    protected: Vec<usize>,
    guide_seams: bool,
    seed: Option<usize>,
}
#[derive(Default, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Method {
    #[default]
    Unwrap,
    Preview,
    Suggest,
}
#[repr(C)]
struct Buffer {
    data: *mut c_char,
    length: usize,
    error: i32,
}
unsafe extern "C" {
    fn bevy_uv_process(input: *const c_char, len: usize, progress: extern "C" fn(i32)) -> Buffer;
    fn bevy_uv_free(output: Buffer);
}
#[unsafe(no_mangle)]
extern "C" fn bevy_uv_dawand_weights(size: &mut usize) -> *const u8 {
    static MODEL: &[u8] = include_bytes!("../vendor/dawand/model.safetensors");
    *size = MODEL.len();
    MODEL.as_ptr()
}
fn event(phase: &str, fraction: f32) {
    let _ = writeln!(
        std::io::stdout().lock(),
        "BEVY_UV:{}",
        serde_json::json!({"phase":phase,"fraction":fraction})
    );
}
extern "C" fn progress(phase: i32) {
    match phase {
        60 => event("DA Wand · расчёт острова", 0.6),
        80 => event("Упаковка UV-островов", 0.8),
        85 => event("Упаковка сложных UV-островов", 0.85),
        87 => event("Точная упаковка UV-островов", 0.87),
        _ => {}
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: bevy-uv-native INPUT.json OUTPUT.json".into());
    }
    let input = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    let request: Request = serde_json::from_reader(BufReader::new(std::fs::File::open(input)?))?;
    if request.positions.len() > 1_500_000
        || request.triangles.is_empty()
        || request.triangles.len() > 500_000
        || !request.positions.iter().flatten().all(|x| x.is_finite())
        || request
            .triangles
            .iter()
            .flatten()
            .any(|&i| i >= request.positions.len())
    {
        return Err("Invalid UV mesh".into());
    }
    let options = &request.options;
    let count = request.triangles.len();
    if (!options.materials.is_empty() && options.materials.len() != count)
        || options.protected.iter().any(|&i| i >= count)
        || options.seed.is_some_and(|i| i >= count)
        || request.resolution == 0
        || request.padding.saturating_mul(2) >= request.resolution
        || !request.threshold.is_finite()
        || request.threshold < 1.0
    {
        return Err("Invalid UV settings or face guidance".into());
    }
    #[derive(Serialize)]
    struct Payload<'a> {
        method: Method,
        resolution: u32,
        padding: u32,
        threshold: f64,
        count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        positions: Option<&'a [[f64; 3]]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        triangles: Option<&'a [[usize; 3]]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        existing_uv: Option<&'a [[[f64; 2]; 3]]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        seed: Option<usize>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        groups: Vec<usize>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        parts: Vec<regions::Part>,
    }
    let mut payload = Payload {
        method: options.method,
        resolution: request.resolution,
        padding: request.padding,
        threshold: request.threshold,
        count,
        positions: None,
        triangles: None,
        existing_uv: request.existing_uv.as_deref(),
        seed: options.seed,
        groups: Vec::new(),
        parts: Vec::new(),
    };
    if let Some(uv) = payload.existing_uv {
        if options.method != Method::Unwrap || uv.len() != count {
            return Err("Invalid UV repacking request".into());
        }
        payload.triangles = Some(&request.triangles);
    } else if options.method == Method::Suggest {
        if options.seed.is_none() {
            return Err("Выберите грань для DA Wand".into());
        }
        event("DA Wand · признаки поверхности", 0.12);
        payload.positions = Some(&request.positions);
        payload.triangles = Some(&request.triangles);
    } else {
        event("Подготовка поверхности", 0.08);
        let groups = if options.guide_seams {
            regions::groups(
                &request.positions,
                &request.triangles,
                &options.materials,
                &options.protected,
            )
        } else {
            vec![0; count]
        };
        if options.method == Method::Preview {
            event("Быстрый предпросмотр xatlas", 0.35);
            payload.positions = Some(&request.positions);
            payload.triangles = Some(&request.triangles);
            payload.groups = groups;
        } else {
            event("Поиск частей модели", 0.20);
            payload.parts = regions::parts(&request.positions, &request.triangles, &groups);
            event("Развёртка поверхности", 0.40);
        }
    }
    let bytes = serde_json::to_vec(&payload)?;
    drop(payload);
    let result = unsafe { bevy_uv_process(bytes.as_ptr().cast(), bytes.len(), progress) };
    if result.data.is_null() {
        return Err("Native UV allocation failed".into());
    }
    let data =
        unsafe { std::slice::from_raw_parts(result.data.cast::<u8>(), result.length) }.to_vec();
    let error = result.error;
    unsafe {
        bevy_uv_free(result);
    }
    if error != 0 {
        return Err(String::from_utf8_lossy(&data).into_owned().into());
    }
    let value: serde_json::Value = serde_json::from_slice(&data)?;
    if options.method != Method::Suggest
        && value["uv"]
            .as_array()
            .is_none_or(|v| v.len() != request.triangles.len())
    {
        return Err("Incomplete native UV output".into());
    }
    event("Проверка результата", 0.95);
    let temporary = output.with_extension("tmp");
    std::fs::write(&temporary, data)?;
    std::fs::rename(temporary, output)?;
    event("Готово", 1.);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        event(&e.to_string(), 0.);
        eprintln!("{e}");
        std::process::exit(1);
    }
}
