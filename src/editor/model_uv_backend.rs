//! Versioned, local CPU runtime and cancellable PartUV subprocesses.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const WORKER: &str = include_str!("../../assets/python/partuv_worker.py");
const WHEEL: &str = "https://github.com/danielboxer/UVgami/releases/download/partuv-v0.1.6/partuv-0.1.6-cp311-cp311-win_amd64.whl";
const WHEEL_HASH: &str = "2142f9a465f293974842514379082f5dfd4c1b790f5fe45c217a1dd35efb3e16";

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Request {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[usize; 3]>,
    pub threshold: f32,
    pub resolution: u32,
    pub padding: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_uv: Option<Vec<[[f32; 2]; 3]>>,
}
#[derive(Clone, Deserialize, Debug)]
pub(super) struct Output {
    pub uv: Vec<[[f32; 2]; 3]>,
    pub charts: usize,
    pub utilization: f32,
}
#[derive(Default, Clone)]
pub(super) struct Status {
    pub phase: String,
    pub fraction: f32,
}
#[derive(Default)]
pub(super) struct Progress {
    pub cancelled: AtomicBool,
    status: Mutex<Status>,
    child: Mutex<Option<Child>>,
}
impl Progress {
    pub fn snapshot(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn set(&self, phase: impl Into<String>, fraction: f32) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = Status {
            phase: phase.into(),
            fraction,
        };
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // Windows venv launchers own a second interpreter process. Reap the
            // entire owned tree off the UI thread so cancellation stays instant.
            std::thread::spawn(move || terminate(child));
        }
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("Развёртка отменена".into())
        } else {
            Ok(())
        }
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(child) = self
            .child
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            std::thread::spawn(move || terminate(child));
        }
    }
}
pub(super) fn store() -> PathBuf {
    std::env::var_os("BEVY_EDITOR_UV_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .or_else(|| std::env::var_os("XDG_DATA_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("BevyEditor/uv-runtime")
        })
}
fn python(root: &Path) -> PathBuf {
    root.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    })
}
fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
fn terminate(mut child: Child) {
    #[cfg(windows)]
    if child.try_wait().is_ok_and(|status| status.is_none()) {
        // Only the child still owned by this handle; never match by executable.
        let mut kill = Command::new(
            std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("C:/Windows"))
                .join("System32/taskkill.exe"),
        );
        let _ = hidden(&mut kill)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
fn execute(
    mut command: Command,
    progress: &Arc<Progress>,
    log: &Path,
    events: bool,
) -> Result<(), String> {
    progress.check()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .map_err(|e| e.to_string())?,
        ));
    let mut child = hidden(&mut command)
        .spawn()
        .map_err(|e| format!("Не удалось запустить инструмент: {e}"))?;
    let stdout = child.stdout.take().ok_or("Нет канала ответа инструмента")?;
    {
        let mut owned = progress.child.lock().unwrap_or_else(|e| e.into_inner());
        if progress.cancelled.load(Ordering::Relaxed) {
            terminate(child);
            return Err("Развёртка отменена".into());
        }
        *owned = Some(child);
    }
    let mut output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| e.to_string())?;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let _ = writeln!(output, "{line}");
        if events {
            if let Some(data) = line
                .strip_prefix("BEVY_UV:")
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            {
                if let (Some(phase), Some(fraction)) =
                    (data["phase"].as_str(), data["fraction"].as_f64())
                {
                    progress.set(phase, fraction as f32);
                }
            }
        }
    }
    progress.check()?;
    let mut child = progress
        .child
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or("Развёртка отменена")?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let phase = progress.snapshot().phase;
        Err(format!(
            "{} · подробности: {}",
            if phase.is_empty() {
                "Инструмент завершился с ошибкой"
            } else {
                &phase
            },
            log.display()
        ))
    }
}
fn locate_uv() -> Option<PathBuf> {
    let mut paths: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(if cfg!(windows) { "uv.exe" } else { "uv" }))
            .collect();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        paths.push(PathBuf::from(home).join(if cfg!(windows) {
            ".local/bin/uv.exe"
        } else {
            ".local/bin/uv"
        }));
    }
    paths.into_iter().find(|p| p.is_file())
}
fn download(url: &str, path: &Path, progress: &Progress) -> Result<(), String> {
    progress.check()?;
    let mut response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Не удалось скачать инструмент: {e}"))?;
    let temporary = path.with_extension("download");
    let mut out = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    let mut buffer = [0; 64 * 1024];
    loop {
        progress.check()?;
        let count = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        out.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
    }
    drop(out);
    fs::rename(temporary, path).map_err(|e| e.to_string())
}
fn runtime(progress: &Arc<Progress>, log: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("BEVY_EDITOR_PARTUV_PYTHON").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        return Err("Не найден указанный Python для PartUV".into());
    }
    let base = store();
    let env = base.join("partuv-0.1.6");
    let python = python(&env);
    let marker = env.join("bevy-uv-ready-v1");
    if python.is_file() && marker.is_file() {
        return Ok(python);
    }
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Err("Автоустановка PartUV доступна на Windows x64. Можно указать BEVY_EDITOR_PARTUV_PYTHON для готовой среды.".into());
    }
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    progress.set("Подготовка локального инструмента", 0.02);
    let uv = if let Some(uv) = locate_uv() {
        uv
    } else {
        let uv = base.join("uv.exe");
        if !uv.is_file() {
            let archive = base.join("uv-bootstrap.zip");
            download(
                "https://github.com/astral-sh/uv/releases/download/0.9.11/uv-x86_64-pc-windows-msvc.zip",
                &archive,
                progress,
            )?;
            let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let name = zip
                .file_names()
                .filter_map(Result::ok)
                .find(|name| name.ends_with("uv.exe"))
                .ok_or("Архив не содержит uv")?
                .into_owned();
            let mut entry = zip.by_name(&name).map_err(|e| e.to_string())?;
            let mut file = fs::File::create(&uv).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        }
        uv
    };
    let mut command = Command::new(&uv);
    command
        .args([
            "--no-config",
            "venv",
            "--allow-existing",
            "--python",
            "3.11",
        ])
        .arg(&env)
        .env("UV_NATIVE_TLS", "true");
    execute(command, progress, log, false)?;
    let wheel = base.join("partuv-0.1.6-cp311-cp311-win_amd64.whl");
    download(WHEEL, &wheel, progress)?;
    use sha2::{Digest, Sha256};
    let hash = format!(
        "{:x}",
        Sha256::digest(fs::read(&wheel).map_err(|e| e.to_string())?)
    );
    if hash != WHEEL_HASH {
        return Err("Контрольная сумма PartUV не совпала. Повторите установку.".into());
    }
    progress.set(
        "Установка PartUV Geometric · только при первом запуске",
        0.04,
    );
    let mut command = Command::new(uv);
    command
        .args(["--no-config", "pip", "install", "--python"])
        .arg(&python)
        .arg(&wheel)
        .args(["numpy==1.26.4", "xatlas==0.0.9"])
        .env("UV_NATIVE_TLS", "true");
    execute(command, progress, log, false)?;
    progress.check()?;
    fs::write(marker, "partuv=0.1.6;xatlas=0.0.9;numpy=1.26.4").map_err(|e| e.to_string())?;
    Ok(python)
}
pub(super) fn run(request: Request, progress: Arc<Progress>) -> Result<Output, String> {
    let root = store().join("jobs").join(format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let log = root.join("worker.log");
    let python = runtime(&progress, &log)?;
    progress.check()?;
    let worker = root.join("partuv_worker.py");
    fs::write(&worker, WORKER).map_err(|e| e.to_string())?;
    let input = root.join("input.json");
    let output = root.join("output.json");
    serde_json::to_writer(
        fs::File::create(&input).map_err(|e| e.to_string())?,
        &request,
    )
    .map_err(|e| e.to_string())?;
    let mut command = Command::new(python);
    command
        .arg("-u")
        .arg(worker)
        .arg(input)
        .arg(&output)
        .env("OMP_NUM_THREADS", "6")
        .env("OPENBLAS_NUM_THREADS", "6");
    execute(command, &progress, &log, true)?;
    progress.check()?;
    let size = fs::metadata(&output).map_err(|e| e.to_string())?.len();
    if size > 128 * 1024 * 1024 {
        return Err("Ответ PartUV слишком большой".into());
    }
    let result: Output =
        serde_json::from_reader(fs::File::open(output).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Повреждён ответ PartUV: {e}"))?;
    if result.charts == 0
        || result.charts > request.triangles.len()
        || !result.utilization.is_finite()
        || !(0.0..=1.00001).contains(&result.utilization)
        || result.uv.len() != request.triangles.len()
        || !result
            .uv
            .iter()
            .flatten()
            .flatten()
            .all(|v| v.is_finite() && (-0.00001..=1.00001).contains(v))
    {
        return Err("UV-карта не прошла проверку. Исходная модель сохранена.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the local Python runtime; verifies live child-tree cancellation"]
    fn uv_worker_cancel_reaps_the_windows_venv_interpreter() {
        let python = std::env::var_os("BEVY_EDITOR_PARTUV_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| python(&store().join("partuv-0.1.6")));
        assert!(python.is_file());
        let dir = std::env::temp_dir().join(format!("uv-cancel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("started.txt");
        let _ = fs::remove_file(&marker);
        let progress = Arc::new(Progress::default());
        let worker_progress = progress.clone();
        let worker_marker = marker.clone();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut command = Command::new(python);
            command.args(["-c", "import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text('started'); time.sleep(60)"]).arg(worker_marker);
            let result = execute(command, &worker_progress, &dir.join("log.txt"), false);
            send.send(result).unwrap();
        });
        let start = std::time::Instant::now();
        while !marker.is_file() && start.elapsed().as_secs() < 15 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            marker.is_file(),
            "Interpreter must actually be running before cancellation"
        );
        let start = std::time::Instant::now();
        progress.cancel();
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "Cancellation must not block the UI"
        );
        let result = receive
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("All inherited stdout handles must close when the process tree exits");
        assert!(result.is_err());
    }
}
