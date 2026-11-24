//! Persistent native Hunyuan worker. One JSON command per stdin line; stdout is
//! reserved for framed events. Closing stdin releases the model and CUDA context.
mod dino;
mod dit;
mod mesh;
mod tensor;
mod vae;

use anyhow::{Context, Result, ensure};
use candle_core::{DType, Device, Tensor};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Command {
    Load { model: PathBuf },
    Generate { directory: PathBuf },
    Cancel,
    Unload,
}
#[derive(Deserialize)]
struct Request {
    name: String,
    steps: u32,
    guidance: f32,
    resolution: u32,
    chunks: u32,
    seed: u64,
    simplify: bool,
    target_faces: u32,
    obj: bool,
}
struct Model {
    weights: tensor::Weights,
    device: Device,
    device_name: String,
}

fn emit(event: &str, mut value: Value) {
    value["event"] = event.into();
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "BEVY_HUNYUAN {value}");
    let _ = stdout.flush();
}
fn progress(stage: &str, f: f32, message: &str) {
    emit(
        "progress",
        json!({"stage":stage,"current":(f.clamp(0.,1.)*1000.) as u32,"total":1000,"message":message}),
    );
}
#[cfg(feature = "cuda")]
fn open_device(cancelled: &AtomicBool) -> Result<(Device, String)> {
    use candle_core::cuda::cudarc::driver::result;

    result::init().context("Не удалось запустить CUDA")?;
    let count = result::device::get_count().context("Не удалось определить видеокарты CUDA")?;
    ensure!(count > 0, "Видеокарты с поддержкой CUDA не найдены");
    let mut devices = Vec::new();
    for ordinal in 0..count {
        let handle = result::device::get(ordinal)?;
        // The CUDA driver returned this device handle above.
        let memory = unsafe { result::device::total_mem(handle) }?;
        let name = result::device::get_name(handle)?;
        devices.push((memory, ordinal as usize, name));
    }
    devices.sort_by_key(|(memory, _, _)| std::cmp::Reverse(*memory));

    let mut errors = Vec::new();
    for (memory, ordinal, name) in devices {
        tensor::check_cancel(cancelled)?;
        progress("loading", 0., &format!("Проверка CUDA на {name}"));
        match Device::new_cuda(ordinal)
            .map_err(anyhow::Error::from)
            .and_then(|device| {
                probe_device(&device)?;
                Ok(device)
            }) {
            Ok(device) => {
                eprintln!(
                    "CUDA device {ordinal}: {name}, {} MiB",
                    memory / (1024 * 1024)
                );
                return Ok((device, name));
            }
            Err(error) => errors.push(format!("{name}: {error:#}")),
        }
    }
    anyhow::bail!("Не удалось выполнить тест CUDA. {}", errors.join(" · "))
}

#[cfg(not(feature = "cuda"))]
fn open_device(_: &AtomicBool) -> Result<(Device, String)> {
    anyhow::bail!("Генератор собран без поддержки CUDA")
}

#[cfg(feature = "cuda")]
fn probe_device(device: &Device) -> Result<()> {
    // Exercise cuBLAS and the reduction kernels now: report ready only after a real CUDA calculation.
    let t = Tensor::ones((1, 64, 64), DType::F16, device)?;
    let sum = t
        .matmul(&t)?
        .to_dtype(DType::F32)?
        .sum_all()?
        .to_scalar::<f32>()?;
    ensure!(
        sum == 262144.,
        "Проверка вычислений CUDA дала неверный результат"
    );
    Ok(())
}

fn load(model: PathBuf, cancelled: &AtomicBool) -> Result<Model> {
    progress("loading", 0., "Поиск видеокарты");
    let (device, device_name) = open_device(cancelled)?;
    let file = if model.is_dir() {
        model.join("model.fp16.safetensors")
    } else {
        model
    };
    let weights = tensor::Weights::load(&file, cancelled, &|f, m| {
        progress("loading", 0.05 + f * 0.95, m)
    })?;
    device.synchronize()?;
    Ok(Model {
        weights,
        device,
        device_name,
    })
}
fn generate(model: &Model, directory: PathBuf, cancelled: &AtomicBool) -> Result<()> {
    let start = Instant::now();
    let request: Request = serde_json::from_slice(&std::fs::read(directory.join("request.json"))?)?;
    ensure!(
        (1..=100).contains(&request.steps)
            && (16..=512).contains(&request.resolution)
            && request.guidance.is_finite()
            && (1.0..=20.0).contains(&request.guidance),
        "Неверные параметры генерации"
    );
    progress("conditioning", 0., "Подготовка изображения");
    let cond = {
        let image = dino::prepare(&directory.join("input"), &model.device)?;
        dino::encode(&model.weights, &image, cancelled, &|f, m| {
            progress("conditioning", f, m)
        })?
    };
    model.device.synchronize()?;
    let latent = {
        emit(
            "progress",
            json!({"stage":"diffusion", "current":0, "total":request.steps}),
        );
        let w = model.weights.stage("model.", &model.device, cancelled)?;
        dit::sample(
            &w,
            &cond,
            request.steps,
            request.guidance,
            request.seed,
            cancelled,
            &|f, m| {
                emit(
                    "progress",
                    json!({"stage":"diffusion", "current":(f*request.steps as f32).round() as u32, "total":request.steps, "message":m}),
                )
            },
        )?
    };
    drop(cond);
    model.device.synchronize()?;
    progress("surface", 0., "Декодирование формы");
    let field = {
        let w = model.weights.stage("vae.", &model.device, cancelled)?;
        let latent = vae::decode(&w, &latent, cancelled)?;
        vae::grid(
            &w,
            &latent,
            request.resolution,
            request.chunks,
            cancelled,
            &|f, m| progress("surface", f, m),
        )?
    };
    drop(latent);
    model.device.synchronize()?;
    tensor::check_cancel(cancelled)?;
    let mut mesh = mesh::Mesh::extract(&field, request.resolution)?;
    drop(field);
    let raw_faces = mesh.faces.len();
    mesh.glb(&directory.join("mesh-raw.glb"))?;
    if request.simplify {
        mesh::simplify(
            &mut mesh,
            request.target_faces as usize,
            cancelled,
            &|f, m| progress("optimizing", f, m),
        )?;
    }
    tensor::check_cancel(cancelled)?;
    progress("exporting", 0., "Сохранение GLB");
    mesh.glb(&directory.join("mesh.glb"))?;
    if request.obj {
        mesh.obj(&directory.join("mesh.obj"))?;
    }
    let details = json!({"name":request.name,"seed":request.seed,"steps":request.steps,"guidance":request.guidance,
        "resolution":request.resolution,"vertices":mesh.vertices.len(),"faces":mesh.faces.len(),"raw_faces":raw_faces,
        "seconds":start.elapsed().as_secs_f64(),"obj":request.obj,"backend":"candle-cuda"});
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&details)?,
    )?;
    emit("complete", details);
    Ok(())
}
fn main() -> Result<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let reader_cancel = cancelled.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.len() > 16 * 1024 {
                emit("error", json!({"message":"Command too large"}));
                continue;
            }
            match serde_json::from_str::<Command>(&line) {
                Ok(Command::Cancel) => {
                    reader_cancel.store(true, Ordering::Relaxed);
                }
                Ok(Command::Unload) => {
                    reader_cancel.store(true, Ordering::Relaxed);
                    let _ = tx.send(Command::Unload);
                    break;
                }
                Ok(command) => {
                    reader_cancel.store(false, Ordering::Relaxed);
                    if tx.send(command).is_err() {
                        break;
                    }
                }
                Err(error) => emit(
                    "error",
                    json!({"message":format!("Invalid command: {error}")}),
                ),
            }
        }
        reader_cancel.store(true, Ordering::Relaxed);
        let _ = tx.send(Command::Unload);
    });
    let mut model: Option<Model> = None;
    emit(
        "hello",
        json!({"protocol":1,"backend":"candle-cuda","version":env!("CARGO_PKG_VERSION")}),
    );
    for command in rx {
        let result = match command {
            Command::Load { model: path } => {
                if let Some(loaded) = &model {
                    emit(
                        "ready",
                        json!({"resident":true,"device":loaded.device_name}),
                    );
                    continue;
                }
                match load(path, &cancelled) {
                    Ok(loaded) => {
                        emit(
                            "ready",
                            json!({"resident":true,"device":loaded.device_name}),
                        );
                        model = Some(loaded);
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            Command::Generate { directory } => match &model {
                Some(model) => generate(model, directory, &cancelled),
                None => Err(anyhow::anyhow!("Сначала включите модель")),
            },
            Command::Unload => {
                drop(model.take());
                emit("unloaded", json!({}));
                break;
            }
            Command::Cancel => continue,
        };
        if let Err(error) = result {
            if cancelled.load(Ordering::Relaxed) {
                emit("cancelled", json!({"message":"Генерация отменена"}));
            } else {
                emit("error", json!({"message":format!("{error:#}")}));
            }
        }
    }
    Ok(())
}
