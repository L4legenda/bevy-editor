pub mod cache;
use cache::{BuildLock, Root, State, prune};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub fn snapshot_key(args: &[OsString]) -> String {
    let command = args[0].to_string_lossy();
    let group = match command.as_ref() {
        "run" => "build",
        "clippy" => "check",
        other => other,
    };
    let (mut profile, mut target) = ("dev".to_owned(), "host".to_owned());
    for (i, arg) in args.iter().enumerate() {
        let arg = arg.to_string_lossy();
        if arg == "--" {
            break;
        }
        if arg == "--release" {
            profile = "release".into();
        }
        for option in ["--profile", "--target"] {
            let value = if arg == option {
                args.get(i + 1).map(|s| s.to_string_lossy().into_owned())
            } else {
                arg.strip_prefix(&format!("{option}=")).map(str::to_owned)
            };
            if let Some(value) = value {
                if option == "--profile" {
                    profile = value;
                } else {
                    target = value;
                }
            }
        }
    }
    serde_json::to_string(&[group, &profile, &target]).unwrap()
}

pub fn limit_bytes(text: &str) -> Result<u64, String> {
    let gib: f64 = text
        .parse()
        .map_err(|_| "BEVY_EDITOR_TARGET_LIMIT_GB must be a finite positive number.")?;
    let bytes = gib * 1024f64.powi(3);
    if !gib.is_finite() || gib <= 0. || !bytes.is_finite() || bytes >= u64::MAX as f64 {
        return Err("BEVY_EDITOR_TARGET_LIMIT_GB must be a finite positive number.".into());
    }
    Ok(bytes as u64)
}

struct OwnedChild {
    process: Child,
    waited: bool,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.waited {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}

pub fn run(cargo: &Path, target: &Path, limit: u64, args: &[OsString]) -> io::Result<i32> {
    let Some(command) = args.first().and_then(|s| s.to_str()) else {
        return Err(io::Error::other("Missing Cargo command"));
    };
    if !matches!(command, "build" | "check" | "test" | "run" | "clippy") {
        return Err(io::Error::other(
            "Supported commands: build, check, test, run, clippy.",
        ));
    }
    for arg in args.iter().take_while(|s| *s != "--") {
        let arg = arg.to_string_lossy();
        if arg.starts_with("--message-format")
            || arg == "--target-dir"
            || arg.starts_with("--target-dir=")
        {
            return Err(io::Error::other(
                "This wrapper manages --message-format and --target-dir itself",
            ));
        }
    }
    let root = Root::new(target)?;
    let _lock = BuildLock::acquire(&root)?;
    let child = Command::new(cargo)
        .arg(&args[0])
        .arg("--message-format=json-render-diagnostics")
        .args(&args[1..])
        .env("CARGO_TARGET_DIR", &root.path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit())
        .spawn()?;
    let mut child = OwnedChild {
        process: child,
        waited: false,
    };
    let output = child.process.stdout.take().unwrap();
    let mut artifacts = BTreeSet::new();
    let mut finished = None;
    let mut stdout = io::stdout().lock();
    let mut reader = io::BufReader::new(output);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let message = if finished.is_none() && line.first() == Some(&b'{') {
            serde_json::from_slice::<Value>(&line)
                .ok()
                .filter(|v| v["reason"].is_string())
        } else {
            None
        };
        let Some(message) = message else {
            stdout.write_all(&line)?;
            stdout.flush()?;
            continue;
        };
        match message["reason"].as_str() {
            Some("compiler-artifact") => {
                if let Some(files) = message["filenames"].as_array() {
                    for name in files.iter().filter_map(Value::as_str) {
                        if let Some(path) = root.local_file(Path::new(name)) {
                            artifacts.insert(root.relative(&path));
                        }
                    }
                }
            }
            Some("build-finished") => finished = Some(message["success"] == true),
            Some("compiler-message") => {
                if let Some(rendered) = message["message"]["rendered"].as_str() {
                    let mut stderr = io::stderr().lock();
                    stderr.write_all(rendered.as_bytes())?;
                    stderr.flush()?;
                }
            }
            _ => {}
        }
    }
    let status = child.process.wait()?;
    child.waited = true;
    if status.success() && finished == Some(true) && !artifacts.is_empty() {
        let maintenance = (|| -> io::Result<()> {
            let mut state = State::load(&root)?;
            state.known.extend(artifacts.iter().cloned());
            state.snapshots.insert(snapshot_key(args), artifacts);
            let report = prune(&root, &mut state, limit)?;
            if report.removed > 0 {
                writeln!(
                    stdout,
                    "Cache: removed {:.1} MiB of superseded artifacts.",
                    report.removed as f64 / 1024f64.powi(2)
                )?;
            }
            if report.remaining > limit {
                writeln!(
                    stdout,
                    "Cache: size remains above the threshold; current and untracked files were kept."
                )?;
            }
            state.save(&root)
        })();
        // Cache maintenance must never turn a successful Cargo command into a failure.
        if let Err(error) = maintenance {
            eprintln!("Cache maintenance skipped: {error}");
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        Ok(status.code().unwrap_or(128 + status.signal().unwrap_or(1)))
    }
    #[cfg(not(unix))]
    {
        Ok(status.code().unwrap_or(1))
    }
}

pub fn main_args(mut args: impl Iterator<Item = OsString>) -> Result<i32, String> {
    let usage = "Usage: bevy-cargo-cache <cargo> <target-dir> <limit-GiB> <build|check|test|run|clippy> [Cargo arguments]";
    let cargo = args.next().ok_or(usage)?;
    if cargo == "--help" {
        println!("{usage}");
        return Ok(0);
    }
    let target = PathBuf::from(args.next().ok_or(usage)?);
    let limit = args.next().ok_or(usage)?;
    let limit = limit_bytes(limit.to_str().ok_or(usage)?)?;
    run(Path::new(&cargo), &target, limit, &args.collect::<Vec<_>>()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
