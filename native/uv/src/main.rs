mod tree;
use serde::{Deserialize, Serialize};
use std::{ffi::c_char, io::Write, path::PathBuf};
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
    tree: Vec<[usize; 3]>,
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
fn event(phase: &str, fraction: f32) {
    let _ = writeln!(
        std::io::stdout().lock(),
        "BEVY_UV:{}",
        serde_json::json!({"phase":phase,"fraction":fraction})
    );
}
extern "C" fn progress(phase: i32) {
    match phase {
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
    let mut request: Request = serde_json::from_reader(std::fs::File::open(input)?)?;
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
    if request.existing_uv.is_none() {
        event("Подготовка поверхности", 0.08);
        (request.positions, request.triangles) =
            tree::prepare(&request.positions, &request.triangles);
        event("Поиск частей модели", 0.20);
        request.tree = tree::build(&request.positions, &request.triangles);
    }
    event("Развёртка и упаковка поверхности", 0.40);
    let bytes = serde_json::to_vec(&request)?;
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
    if value["uv"]
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
