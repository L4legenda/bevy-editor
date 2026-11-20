//! Bundled Rust/C++ worker, with no Python environment, packages or downloads.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
include!(concat!(env!("OUT_DIR"), "/uv_native_bundle.rs"));
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub(super) enum Method {
    #[default]
    Unwrap,
    Preview,
    Suggest,
}
#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub(super) struct Options {
    pub method: Method,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected: Vec<usize>,
    pub guide_seams: bool,
    pub seed: Option<usize>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Request {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[usize; 3]>,
    pub threshold: f32,
    pub resolution: u32,
    pub padding: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_uv: Option<Vec<[[f32; 2]; 3]>>,
    #[serde(default)]
    pub options: Options,
}
#[derive(Clone, Deserialize, Debug)]
pub(super) struct Output {
    #[serde(default)]
    pub suggestion: Vec<usize>,
    #[serde(default)]
    pub uv: Vec<[[f32; 2]; 3]>,
    #[serde(default)]
    pub charts: usize,
    #[serde(default)]
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
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            std::thread::spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
            });
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
        if let Some(mut c) = self
            .child
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
pub(super) fn store() -> PathBuf {
    let root = std::env::var_os("BEVY_EDITOR_UV_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .or_else(|| std::env::var_os("XDG_DATA_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("BevyEditor/uv-runtime")
        });
    if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(root)
    }
}
fn runtime(progress: &Progress) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("BEVY_EDITOR_UV_WORKER").map(PathBuf::from) {
        return if path.is_file() {
            Ok(path)
        } else {
            Err("Не найден нативный движок UV".into())
        };
    }
    materialize(&store(), progress)
}
fn materialize(base: &Path, progress: &Progress) -> Result<PathBuf, String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for (name, bytes) in NATIVE_BUNDLE {
        hash.update(name.as_bytes());
        hash.update(bytes);
    }
    let root = base.join(format!("native-{:x}", hash.finalize()));
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut executable = None;
    for &(name, bytes) in NATIVE_BUNDLE {
        progress.check()?;
        let path = root.join(name);
        if fs::read(&path).ok().as_deref() != Some(bytes) {
            let temporary = root.join(format!(
                "{name}.{}.{}.tmp",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
            if let Err(e) = fs::rename(&temporary, &path) {
                if fs::read(&path).ok().as_deref() != Some(bytes) {
                    return Err(format!("Не удалось подготовить нативный движок UV: {e}"));
                }
                let _ = fs::remove_file(temporary);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if name == "bevy-uv-native" {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
            }
        }
        if name == "bevy-uv-native.exe" || name == "bevy-uv-native" {
            executable = Some(path);
        }
    }
    executable.ok_or_else(|| "В сборке отсутствует нативный движок UV".into())
}
pub(super) fn run(request: Request, progress: Arc<Progress>) -> Result<Output, String> {
    progress.check()?;
    progress.set("Подготовка нативного движка", 0.02);
    let worker = runtime(&progress)?;
    let root = store().join("jobs").join(format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let input = root.join("input.json");
    let output = root.join("output.json");
    let log = root.join("worker.log");
    {
        let mut writer = BufWriter::new(fs::File::create(&input).map_err(|e| e.to_string())?);
        serde_json::to_writer(&mut writer, &request).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
    }
    progress.check()?;
    let mut command = Command::new(worker);
    command
        .arg(&input)
        .arg(&output)
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .map_err(|e| e.to_string())?,
        ));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Не удалось запустить нативный движок UV: {e}"))?;
    let stdout = child.stdout.take().ok_or("Нет канала ответа движка")?;
    {
        let mut owned = progress.child.lock().unwrap_or_else(|e| e.into_inner());
        if progress.cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Развёртка отменена".into());
        }
        *owned = Some(child);
    }
    let mut log_file = fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .map_err(|e| e.to_string())?;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let _ = writeln!(log_file, "{line}");
        if let Some(value) = line
            .strip_prefix("BEVY_UV:")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        {
            if let (Some(phase), Some(fraction)) =
                (value["phase"].as_str(), value["fraction"].as_f64())
            {
                progress.set(phase, (fraction as f32).clamp(0., 1.));
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
    if !status.success() {
        return Err(format!(
            "{} · подробности: {}",
            progress.snapshot().phase,
            log.display()
        ));
    }
    progress.check()?;
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
        return Err("Ответ движка UV слишком большой".into());
    }
    let result: Output = serde_json::from_reader(BufReader::new(
        fs::File::open(output).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| format!("Повреждён ответ нативного движка UV: {e}"))?;
    if request.options.method == Method::Suggest {
        let seed = request.options.seed.ok_or("Не выбрана грань для DA Wand")?;
        if !result.suggestion.contains(&seed)
            || result.suggestion.len() > request.triangles.len()
            || result
                .suggestion
                .iter()
                .any(|&i| i >= request.triangles.len())
        {
            return Err("Некорректное предложение DA Wand".into());
        }
        return Ok(result);
    }
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
    fn plane(n: usize) -> Request {
        let mut positions = Vec::new();
        for y in 0..=n {
            for x in 0..=n {
                positions.push([x as f32 / n as f32, y as f32 / n as f32, 0.]);
            }
        }
        let mut triangles = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = y * (n + 1) + x;
                triangles.extend([[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]);
            }
        }
        Request {
            positions,
            triangles,
            threshold: 1.25,
            resolution: 1024,
            padding: 8,
            existing_uv: None,
            options: Default::default(),
        }
    }
    #[test]
    fn uv_native_bundle_runs_with_only_system_path_and_no_python_environment() {
        let dir = std::env::temp_dir().join(format!(
            "uv-bundle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let worker = materialize(&dir, &Progress::default()).unwrap();
        let request = plane(2);
        let input = dir.join("input.json");
        let output = dir.join("output.json");
        fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let mut command = Command::new(worker);
        command
            .arg(input)
            .arg(&output)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("VIRTUAL_ENV")
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .env("BEVY_EDITOR_PARTUV_PYTHON", dir.join("missing-python.exe"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
            command.env(
                "PATH",
                PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32"),
            );
        }
        assert!(
            command.status().unwrap().success(),
            "The bundled engine must require no environment installation"
        );
        let result: Output = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        assert_eq!(result.uv.len(), request.triangles.len());
        assert!(
            result
                .uv
                .iter()
                .flatten()
                .flatten()
                .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
        );
    }
    #[test]
    fn uv_native_packing_preserves_slivers_and_each_whole_chart() {
        let uv: [[f32; 2]; 9] = [
            [0., 0.],
            [1., 0.],
            [1., 1.],
            [0., 1.],
            [1.5, 1e-10],
            [0., 0.],
            [0.4, 0.],
            [0.4, 0.2],
            [0., 0.2],
        ];
        let triangles = vec![[0, 1, 2], [0, 2, 3], [0, 4, 1], [5, 6, 7], [5, 7, 8]];
        let before: Vec<_> = triangles.iter().map(|face| face.map(|i| uv[i])).collect();
        let request = Request {
            positions: uv.iter().map(|p| [p[0], p[1], 0.]).collect(),
            triangles,
            threshold: 1.25,
            resolution: 1024,
            padding: 8,
            options: Default::default(),
            existing_uv: Some(before.clone()),
        };
        let result = run(request, Arc::new(Progress::default())).unwrap();
        assert_eq!(result.charts, 2);
        for range in [0..3, 3..5] {
            let source: Vec<_> = before[range.clone()].iter().flatten().collect();
            let packed: Vec<_> = result.uv[range].iter().flatten().collect();
            let mut scale = None;
            for i in 0..source.len() {
                for j in 0..i {
                    let distance = |a: &[f32; 2], b: &[f32; 2]| {
                        ((a[0] as f64 - b[0] as f64).powi(2) + (a[1] as f64 - b[1] as f64).powi(2))
                            .sqrt()
                    };
                    let original = distance(source[i], source[j]);
                    if original > 1e-6 {
                        let ratio = distance(packed[i], packed[j]) / original;
                        let expected = *scale.get_or_insert(ratio);
                        assert!(
                            (ratio / expected - 1.).abs() < 0.001,
                            "Packing must preserve sliver corners and chart shape"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn uv_native_cancellation_stops_a_live_calculation_without_blocking() {
        let progress = Arc::new(Progress::default());
        let owned = progress.clone();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            send.send(run(plane(96), owned)).unwrap();
        });
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 20 {
            if progress.snapshot().phase.starts_with("Развёртка") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(progress.snapshot().phase.starts_with("Развёртка"));
        let start = std::time::Instant::now();
        progress.cancel();
        assert!(start.elapsed() < std::time::Duration::from_millis(200));
        assert!(
            receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
    }
}
