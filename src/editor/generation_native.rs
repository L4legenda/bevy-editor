//! Persistent Rust/Candle process transport. CUDA and weights never enter a game build.
use super::{Progress, ResultModel};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Clone, Debug)]
pub(in crate::editor) struct Runtime {
    pub executable: PathBuf,
}
impl Runtime {
    pub fn discover(roots: &[PathBuf]) -> Result<Self, String> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("BEVY_EDITOR_HUNYUAN_NATIVE") {
            candidates.push(path.into());
        }
        let filename = if cfg!(windows) {
            "bevy-hunyuan.exe"
        } else {
            "bevy-hunyuan"
        };
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.join(filename));
            }
        }
        for root in roots
            .iter()
            .chain(std::iter::once(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))))
        {
            for parent in root.ancestors() {
                candidates.push(
                    parent
                        .join("crates/hunyuan-native/target/release")
                        .join(filename),
                );
                candidates.push(
                    parent
                        .join("crates/hunyuan-native/target/debug")
                        .join(filename),
                );
            }
        }
        candidates.into_iter().find(|p|p.is_file()).map(|executable|Self{executable})
            .ok_or_else(||"Компонент 3D генерации не подготовлен. Закройте редактор и запустите run-editor.cmd: запускатель соберёт генератор и подготовит CUDA для NVIDIA. Для готового релиза распакуйте полный архив с 3D генерацией.".into())
    }
    pub fn launch(&self, model: &Path, cache: &Path) -> Result<Service, String> {
        fs::create_dir_all(cache).map_err(|e| e.to_string())?;
        let log =
            fs::File::create(cache.join("generator-stderr.log")).map_err(|e| e.to_string())?;
        let mut command = Command::new(&self.executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        let mut paths = vec![
            self.executable
                .parent()
                .unwrap_or(Path::new("."))
                .to_owned(),
        ];
        if let Some(cuda) = std::env::var_os("CUDA_PATH") {
            paths.push(PathBuf::from(cuda).join("bin"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(
                PathBuf::from(local).join("BevyEditor/system/runtime/cuda-12.8.1/toolkit/bin"),
            );
        }
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Не удалось запустить Rust-генератор: {e}"))?;
        let input = child.stdin.take().ok_or("Нет канала команд генератора")?;
        let output = child.stdout.take().ok_or("Нет канала ответов генератора")?;
        let (tx, rx) = mpsc::channel();
        let directory = cache.to_owned();
        std::thread::spawn(move || {
            let mut log = fs::File::create(directory.join("generator-stdout.log")).ok();
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Some(log) = &mut log {
                    let _ = writeln!(log, "{line}");
                }
                if let Some(json) = line.strip_prefix("BEVY_HUNYUAN ") {
                    if let Ok(value) = serde_json::from_str::<Value>(json) {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let service = Service(Arc::new(Session {
            child: Mutex::new(Some(child)),
            input: Mutex::new(input),
            events: Mutex::new(rx),
            ready: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }));
        service.send(json!({"command":"load","model":model}))?;
        Ok(service)
    }
}

pub(super) struct Session {
    child: Mutex<Option<Child>>,
    input: Mutex<ChildStdin>,
    events: Mutex<mpsc::Receiver<Value>>,
    ready: AtomicBool,
    stopped: AtomicBool,
}
impl Session {
    pub fn shutdown(&self) {
        self.ready.store(false, Ordering::Relaxed);
        self.stopped.store(true, Ordering::Relaxed);
        // Terminate only the handle owned by this service. A process exit destroys its CUDA
        // context as well as host weights, even if generation is executing a kernel.
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[derive(Clone)]
pub(in crate::editor) struct Service(Arc<Session>);
impl Service {
    pub fn shutdown(&self) {
        self.0.shutdown();
    }
    pub fn ready(&self) -> bool {
        self.0.ready.load(Ordering::Relaxed) && !self.0.stopped.load(Ordering::Relaxed)
    }
    pub fn alive(&self) -> bool {
        if self.0.stopped.load(Ordering::Relaxed) {
            return false;
        }
        let mut owned = self.0.child.lock().unwrap_or_else(|e| e.into_inner());
        matches!(owned.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
    }
    fn send(&self, value: Value) -> Result<(), String> {
        let mut input = self.0.input.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(input, "{value}")
            .and_then(|_| input.flush())
            .map_err(|e| format!("Нет связи с Rust-генератором: {e}"))
    }
    fn wait(&self, terminal: &str, progress: &Progress) -> Result<Value, String> {
        let events = self.0.events.lock().unwrap_or_else(|e| e.into_inner());
        let mut cancel_sent = false;
        loop {
            if self.0.stopped.load(Ordering::Relaxed) {
                return Err("3D генератор остановлен".into());
            }
            if progress.cancelled.load(Ordering::Relaxed) && !cancel_sent {
                if terminal == "ready" {
                    self.shutdown();
                    return Err("Запуск модели отменён".into());
                }
                self.send(json!({"command":"cancel"}))?;
                cancel_sent = true;
            }
            match events.recv_timeout(Duration::from_millis(50)) {
                Ok(value)=>{
                    progress.event(&format!("BEVY_HUNYUAN {value}"));
                    match value["event"].as_str() {
                        Some(event) if event==terminal=>return Ok(value),
                        Some("cancelled")=>return Err("Генерация отменена".into()),
                        Some("error")=>return Err(value["message"].as_str().unwrap_or("Ошибка генератора").into()),
                        _=>{}
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout)=>{if !self.alive(){return Err("Rust-генератор завершился. Подробности — в системном generator-stderr.log. Проверьте CUDA 12 и драйвер NVIDIA.".into());}}
                Err(mpsc::RecvTimeoutError::Disconnected)=>return Err("Rust-генератор завершился. Проверьте generator-stderr.log и библиотеки CUDA 12.".into()),
            }
        }
    }
    pub fn wait_ready(&self, progress: &Progress) -> Result<(), String> {
        *progress.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(&self.0));
        self.wait("ready", progress)?;
        self.0.ready.store(true, Ordering::Relaxed);
        Ok(())
    }
    pub fn generate(&self, directory: &Path, progress: &Progress) -> Result<ResultModel, String> {
        if !self.ready() {
            return Err("Сначала включите модель".into());
        }
        *progress.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(&self.0));
        self.send(json!({"command":"generate","directory":directory}))?;
        self.wait("complete", progress)?;
        let mut result: ResultModel = serde_json::from_slice(
            &fs::read(directory.join("result.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        result.directory = directory.to_owned();
        if result.faces == 0 || result.vertices == 0 || !result.glb().is_file() {
            return Err("Генератор не вернул 3D-модель".into());
        }
        crate::editor::model_document::glb::Glb::load(&result.glb())?;
        Ok(result)
    }
}
