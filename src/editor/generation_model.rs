//! Editor-only model cache. Never stores weights in a project's asset root.
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    time::Duration,
};

pub(super) const REVISION: &str = "f90a0f7df7d5e6f71109cf333f6a95a0ae3194a6";
const VARIANT: &str = "hunyuan3d-dit-v2-mini";
const REPOSITORY: &str = "https://huggingface.co/tencent/Hunyuan3D-2mini/resolve";
const CHUNK: u64 = 8 * 1024 * 1024;
pub(super) const IGNORE: &str =
    "# Editor system files: do not commit or package with games.\n*\n!.gitignore\n";

#[derive(Clone, Copy)]
struct Artifact {
    path: &'static str,
    size: u64,
    sha: &'static str,
}
const FILES: [Artifact; 4] = [
    Artifact {
        path: "LICENSE",
        size: 17828,
        sha: "e5e6e2041e18326d82cd105a1b9404e36db55263dd9d42d4989f917c2501b67b",
    },
    Artifact {
        path: "NOTICE",
        size: 32064,
        sha: "7b7ace3714c188b7a27e20edbcceaaac836540bbb63759a37039e5624c8f6bbc",
    },
    Artifact {
        path: "hunyuan3d-dit-v2-mini/config.yaml",
        size: 1628,
        sha: "cabcba7f6115752c8fe5b370e12bf714936f70377a8a80f151872f76c2d64609",
    },
    Artifact {
        path: "hunyuan3d-dit-v2-mini/model.fp16.safetensors",
        size: 3819958234,
        sha: "3cc66f3bea33e4062b7dbc875ffe1d70c4888914aec3e91b60f94e9bd01b522b",
    },
];
pub(super) const TOTAL: u64 = 3819958234 + 1628 + 17828 + 32064;

#[derive(Default)]
pub(super) struct Progress {
    pub received: AtomicU64,
    /// 0: transfer, 1: checking files, 2: cancelling, 3: finished.
    pub phase: AtomicU8,
    pub cancelled: AtomicBool,
}
impl Progress {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.phase.store(2, Ordering::Relaxed);
    }
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("Загрузка отменена. Можно продолжить позже.".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub(super) struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn system() -> Result<Self, String> {
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Local"))
            });
        #[cfg(target_os = "macos")]
        let base = std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"));
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")));
        Ok(Self {
            root: base
                .ok_or("Не найдена системная папка пользователя")?
                .join("BevyEditor/system/models"),
        })
    }
    pub fn prepare(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root)
            .map_err(|e| format!("Не удалось создать папку моделей: {e}"))?;
        fs::write(self.root.join(".gitignore"), IGNORE).map_err(|e| e.to_string())
    }
    fn bundle(&self) -> PathBuf {
        self.root.join("Hunyuan3D-2mini").join(REVISION)
    }
    pub fn model(&self) -> PathBuf {
        self.bundle().join(VARIANT)
    }
    pub fn discover(&self, roots: &[PathBuf]) -> Option<PathBuf> {
        let mut candidates = vec![self.model()];
        if let Some(path) = std::env::var_os("BEVY_EDITOR_HUNYUAN_MODEL_DIR") {
            candidates.push(PathBuf::from(path));
        }
        // Reuse the user's existing lab without moving or changing its files.
        for root in roots {
            for parent in root.ancestors() {
                let location = parent.join("3D-AI-Lab/hunyuan-mini-model-location.txt");
                if let Ok(bytes) = fs::read(&location)
                    && bytes.len() < 16 * 1024
                    && let Ok(value) = std::str::from_utf8(&bytes)
                {
                    candidates.push(PathBuf::from(value.trim().trim_start_matches('\u{feff}')));
                }
            }
        }
        let mut hubs = Vec::new();
        if let Some(path) = std::env::var_os("HF_HUB_CACHE") {
            hubs.push(PathBuf::from(path));
        }
        if let Some(path) = std::env::var_os("HF_HOME") {
            hubs.push(PathBuf::from(path).join("hub"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            hubs.push(PathBuf::from(home).join(".cache/huggingface/hub"));
        }
        for hub in hubs {
            if let Ok(entries) =
                fs::read_dir(hub.join("models--tencent--Hunyuan3D-2mini/snapshots"))
            {
                candidates.extend(entries.flatten().map(|e| e.path().join(VARIANT)));
            }
        }
        candidates.into_iter().find(|path| installed(path))
    }
    pub fn download(&self, progress: &Progress) -> Result<PathBuf, String> {
        self.prepare()?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("download.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|_| "Модель уже скачивается в другом окне редактора".to_owned())?;
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .user_agent("BevyEditor/0.1")
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .map_err(|e| e.to_string())?;
        let base = format!("{REPOSITORY}/{REVISION}");
        let mut complete = 0;
        for artifact in FILES {
            download_file(&client, &base, &self.bundle(), artifact, complete, progress)?;
            complete += artifact.size;
        }
        progress.check()?;
        if !installed(&self.model()) {
            return Err("Проверка модели не пройдена. Повторите загрузку.".into());
        }
        progress.received.store(TOTAL, Ordering::Relaxed);
        progress.phase.store(3, Ordering::Relaxed);
        Ok(self.model())
    }
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Сервер не ответил вовремя. Можно продолжить скачивание.".into()
    } else if let Some(status) = error.status() {
        format!(
            "Сервер вернул ошибку HTTP {}. Повторите скачивание позже.",
            status.as_u16()
        )
    } else {
        "Не удалось подключиться к серверу. Проверьте соединение и повторите скачивание.".into()
    }
}

fn digest_file(path: &Path, progress: &Progress) -> Result<Sha256, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        progress.check()?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash)
}

fn download_file(
    client: &reqwest::blocking::Client,
    base: &str,
    directory: &Path,
    artifact: Artifact,
    complete: u64,
    progress: &Progress,
) -> Result<(), String> {
    progress.check()?;
    let destination = directory.join(artifact.path);
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    progress.phase.store(1, Ordering::Relaxed);
    if fs::metadata(&destination).is_ok_and(|m| m.len() == artifact.size)
        && format!("{:x}", digest_file(&destination, progress)?.finalize()) == artifact.sha
    {
        progress
            .received
            .store(complete + artifact.size, Ordering::Relaxed);
        return Ok(());
    }
    let partial = destination.with_extension(format!(
        "{}.part",
        destination
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    let mut offset = fs::metadata(&partial).map_or(0, |m| m.len());
    if offset > artifact.size {
        fs::remove_file(&partial).map_err(|e| e.to_string())?;
        offset = 0;
    }
    let mut hash = if offset > 0 {
        digest_file(&partial, progress)?
    } else {
        Sha256::new()
    };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .map_err(|e| e.to_string())?;
    progress
        .received
        .store(complete + offset, Ordering::Relaxed);
    progress.phase.store(0, Ordering::Relaxed);
    while offset < artifact.size {
        progress.check()?;
        let end = (offset + CHUNK).min(artifact.size) - 1;
        let mut response = client
            .get(format!("{base}/{}?download=true", artifact.path))
            .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(network_error)?;
        let expected = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let header = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .ok_or("Сервер не подтвердил диапазон загрузки")?;
            let expected = format!("bytes {offset}-{end}/{}", artifact.size);
            if header != expected {
                return Err("Сервер вернул неверный диапазон. Повторите загрузку.".into());
            }
            end - offset + 1
        } else if response.status() == reqwest::StatusCode::OK && offset == 0 {
            artifact.size
        } else {
            return Err("Сервер не поддерживает продолжение загрузки. Повторите позже.".into());
        };
        let mut received = 0;
        let mut buffer = [0; 64 * 1024];
        loop {
            progress.check()?;
            let count = response
                .read(&mut buffer)
                .map_err(|_| "Соединение прервано. Можно продолжить скачивание.".to_owned())?;
            if count == 0 {
                break;
            }
            if received + count as u64 > expected {
                return Err("Размер ответа превышает размер файла модели".into());
            }
            file.write_all(&buffer[..count])
                .map_err(|e| format!("Не удалось записать модель: {e}"))?;
            hash.update(&buffer[..count]);
            received += count as u64;
            offset += count as u64;
            progress
                .received
                .store(complete + offset, Ordering::Relaxed);
        }
        if received != expected {
            return Err("Загрузка прервалась. Нажмите «Продолжить скачивание».".into());
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    progress.check()?;
    progress.phase.store(1, Ordering::Relaxed);
    if format!("{:x}", hash.finalize()) != artifact.sha {
        let _ = fs::remove_file(&partial);
        return Err("Контрольная сумма не совпала. Нажмите «Скачать», чтобы повторить.".into());
    }
    if destination.exists() {
        fs::remove_file(&destination).map_err(|e| e.to_string())?;
    }
    fs::rename(partial, destination).map_err(|e| e.to_string())?;
    Ok(())
}

/// Fast offline detection, including real safetensors structure, without reading
/// 3.8 GB of tensors on every editor launch. Downloads are SHA-256 checked once.
pub(super) fn installed(path: &Path) -> bool {
    let config = path.join("config.yaml");
    if fs::read(&config)
        .ok()
        .filter(|b| b.len() as u64 == FILES[2].size)
        .is_none_or(|b| format!("{:x}", Sha256::digest(&b)) != FILES[2].sha)
    {
        return false;
    }
    valid_weights(&path.join("model.fp16.safetensors"), FILES[3].size)
}

fn valid_weights(path: &Path, expected: u64) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() == expected)
    {
        return false;
    }
    let mut length = [0; 8];
    if file.read_exact(&mut length).is_err() {
        return false;
    }
    let length = u64::from_le_bytes(length);
    if !(2..=8 * 1024 * 1024).contains(&length) || length + 8 >= expected {
        return false;
    }
    let mut header = vec![0; length as usize];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    let Ok(serde_json::Value::Object(header)) = serde_json::from_slice(&header) else {
        return false;
    };
    let mut maximum = 0;
    let mut tensors = 0;
    for (key, value) in header {
        if key == "__metadata__" {
            continue;
        }
        let Some(offsets) = value
            .get("data_offsets")
            .and_then(|v| v.as_array())
            .filter(|v| v.len() == 2)
        else {
            return false;
        };
        let (Some(start), Some(end)) = (offsets[0].as_u64(), offsets[1].as_u64()) else {
            return false;
        };
        if start > end || end > expected - length - 8 {
            return false;
        }
        maximum = maximum.max(end);
        tensors += 1;
    }
    tensors > 0 && maximum == expected - length - 8
}

#[cfg(test)]
#[path = "generation_model_tests.rs"]
mod tests;
