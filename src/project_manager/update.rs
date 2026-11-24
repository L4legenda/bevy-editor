use super::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, mpsc};

const REPOSITORY: &str = "L4legenda/bevy-editor";
const MAX_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Default, Resource)]
pub(super) struct Updates {
    pub available: Option<Release>,
    pub downloading: bool,
    task: Option<Mutex<mpsc::Receiver<Result<Outcome, String>>>>,
}

enum Outcome {
    Checked(Option<Release>),
    Installing,
}

#[derive(Deserialize)]
pub(super) struct Release {
    pub tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize, Clone)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}

fn version(tag: &str) -> Option<[u64; 5]> {
    let (version, build) = tag.strip_prefix('v')?.split_once("+build.")?;
    let mut parts = version.split('.');
    let mut build = build.split('.');
    let result = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        build.next()?.parse().ok()?,
        build.next()?.parse().ok()?,
    ];
    let commit = build.next()?;
    (parts.next().is_none()
        && build.next().is_none()
        && commit.len() == 8
        && commit.bytes().all(|b| b.is_ascii_hexdigit()))
    .then_some(result)
}

fn client() -> reqwest::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .user_agent(concat!("BevyEditor/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(600))
        .build()
}

impl Release {
    fn asset(&self) -> Option<Asset> {
        let url = format!(
            "https://github.com/{REPOSITORY}/releases/download/{}/bevy-editor.exe",
            self.tag_name
        );
        self.assets
            .iter()
            .find(|asset| {
                asset.name == "bevy-editor.exe"
                    && (1..=MAX_BYTES).contains(&asset.size)
                    && asset.browser_download_url.replace("%2B", "+").eq(&url)
                    && asset
                        .digest
                        .as_deref()
                        .and_then(|d| d.strip_prefix("sha256:"))
                        .is_some_and(|hex| {
                            hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
                        })
            })
            .cloned()
    }
}

impl Updates {
    fn run(&mut self, job: impl FnOnce() -> Result<Outcome, String> + Send + 'static) {
        let (send, receive) = mpsc::channel();
        self.task = Some(Mutex::new(receive));
        std::thread::spawn(move || {
            let _ = send.send(job());
        });
    }

    pub fn check(&mut self) {
        let Some(current) = option_env!("BEVY_EDITOR_RELEASE_TAG").and_then(version) else {
            return;
        };
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return;
        }
        self.run(move || {
            let release: Release = client()
                .and_then(|client| {
                    client
                        .get(format!(
                            "https://api.github.com/repos/{REPOSITORY}/releases/latest"
                        ))
                        .send()?
                        .error_for_status()?
                        .json()
                })
                .map_err(|e| e.to_string())?;
            Ok(Outcome::Checked(
                (!release.draft
                    && !release.prerelease
                    && version(&release.tag_name).is_some_and(|latest| latest > current)
                    && release.asset().is_some())
                .then_some(release),
            ))
        });
    }

    pub fn install(&mut self, exe: PathBuf) {
        if self.task.is_none()
            && let Some(asset) = self.available.as_ref().and_then(Release::asset)
        {
            self.downloading = true;
            self.run(move || {
                install(&exe, asset)
                    .map(|()| Outcome::Installing)
                    .map_err(|e| e.to_string())
            });
        }
    }
}

pub(super) fn poll(
    mut updates: ResMut<Updates>,
    mut manager: ResMut<Manager>,
    mut exits: MessageWriter<AppExit>,
) {
    let Some(task) = updates.task.as_ref() else {
        return;
    };
    let result = match task.lock().unwrap().try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => Err("Проверка обновления прервалась".into()),
    };
    updates.task = None;
    let downloading = std::mem::take(&mut updates.downloading);
    match result {
        Ok(Outcome::Checked(release)) => updates.available = release,
        Ok(Outcome::Installing) => {
            exits.write(AppExit::Success);
        }
        Err(error) if downloading => {
            manager.error(format!("Не удалось обновить редактор: {error}"))
        }
        Err(error) => debug!("Release check failed: {error}"),
    }
}

fn install(exe: &Path, asset: Asset) -> io::Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let staged = exe.with_extension(format!("update-{}-{nonce}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    let result = (|| {
        let response = client()
            .and_then(|client| {
                client
                    .get(&asset.browser_download_url)
                    .send()?
                    .error_for_status()
            })
            .map_err(io::Error::other)?;
        use io::Read;
        let mut hash = Sha256::new();
        let mut response = response.take(asset.size + 1);
        let mut total = 0;
        let mut buffer = [0; 65536];
        loop {
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > asset.size {
                return Err(io::Error::other("Размер файла не совпадает с релизом"));
            }
            hash.update(&buffer[..count]);
            io::Write::write_all(&mut file, &buffer[..count])?;
        }
        let digest = format!("sha256:{:x}", hash.finalize());
        if total != asset.size
            || !asset
                .digest
                .is_some_and(|expected| expected.eq_ignore_ascii_case(&digest))
        {
            return Err(io::Error::other(
                "Контрольная сумма файла не совпадает с релизом",
            ));
        }
        file.sync_all()
    })();
    drop(file);
    let result = result.and_then(|()| replace_after_exit(exe, &staged));
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}

#[cfg(windows)]
fn replace_after_exit(exe: &Path, staged: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let system =
        std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("SystemRoot is missing"))?;
    Command::new(PathBuf::from(system).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("replace_editor.ps1"),
        ])
        .env("BEVY_UPDATE_EXE", exe)
        .env("BEVY_UPDATE_STAGED", staged)
        .env("BEVY_UPDATE_PID", std::process::id().to_string())
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn replace_after_exit(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows release required",
    ))
}

#[cfg(test)]
#[test]
fn compares_release_builds_numerically() {
    assert!(version("v0.1.0+build.10.1.abcdef01") > version("v0.1.0+build.9.2.abcdef01"));
    assert!(version("v0.2.0+build.1.1.abcdef01") > version("v0.1.9+build.99.1.abcdef01"));
    assert_eq!(version("develop-4-abcdef01"), None);
}
