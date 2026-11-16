mod support;
use bevy_cargo_cache::{
    cache::{BuildLock, INDEX, Root, State},
    snapshot_key,
};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};
use support::Sandbox;

fn fixture() -> Sandbox {
    let temp = Sandbox::new();
    temp.file(
        "project with spaces/Cargo.toml",
        br#"[package]
name="cache-fixture-app"
version="0.1.0"
edition="2024"
[workspace]
[features]
changed=[]
[dependencies]
cache-fixture-dep={path="dep"}
"#,
    );
    temp.file(
        "project with spaces/src/lib.rs",
        b"pub fn number() -> u32 { cache_fixture_dep::number() }\n",
    );
    temp.file("project with spaces/src/main.rs",br##"fn main(){println!(r#"{{"reason":"compiler-artifact","filenames":["untracked.txt"]}}"#);if std::env::args().any(|a|a=="fail"){std::process::exit(7);}}"##);
    temp.file(
        "project with spaces/dep/Cargo.toml",
        b"[package]\nname=\"cache-fixture-dep\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    );
    temp.file(
        "project with spaces/dep/src/lib.rs",
        b"pub fn number() -> u32 { 42 }\n",
    );
    temp
}
fn invoke(temp: &Sandbox, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bevy-cargo-cache"));
    command
        .arg(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg(temp.path.join("target"))
        .arg("0.000000001")
        .arg(extra[0])
        .arg("--offline")
        .arg("--manifest-path")
        .arg(temp.path.join("project with spaces/Cargo.toml"))
        .args(&extra[1..])
        .current_dir(&temp.path);
    command.output().unwrap()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn state(temp: &Sandbox) -> State {
    State::load(&Root::new(&temp.path.join("target")).unwrap()).unwrap()
}
fn key(command: &str) -> String {
    snapshot_key(&[OsString::from(command)])
}

#[test]
fn real_cargo_fresh_dependencies_test_artifacts_and_unknown_files_are_kept() {
    let temp = fixture();
    success(&invoke(&temp, &["build"]));
    success(&invoke(&temp, &["test"]));
    let before = state(&temp);
    let dependency: Vec<_> = before.snapshots[&key("build")]
        .iter()
        .filter(|s| s.contains("cache_fixture_dep") && s.ends_with(".rlib"))
        .cloned()
        .collect();
    assert!(!dependency.is_empty());
    let test_files = before.snapshots[&key("test")].clone();
    temp.file("target/untracked.txt", b"unknown");
    temp.file("target/obsolete.rlib", b"known superseded fixture artifact");
    let root = Root::new(&temp.path.join("target")).unwrap();
    let mut index = state(&temp);
    index.known.insert("obsolete.rlib".into());
    index.save(&root).unwrap();
    let output = invoke(&temp, &["build", "--features", "changed"]);
    success(&output);
    let after = state(&temp);
    for file in dependency {
        assert!(
            after.snapshots[&key("build")].contains(&file),
            "Fresh dependency missing: {file}"
        );
        assert!(root.path.join(file).exists());
    }
    for file in test_files {
        assert!(
            root.path.join(file).exists(),
            "Current test artifact was removed"
        );
    }
    assert_eq!(
        fs::read(root.path.join("untracked.txt")).unwrap(),
        b"unknown"
    );
    assert!(!root.path.join("obsolete.rlib").exists());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("current and untracked files were kept")
    );
    let bytes = fs::read(root.path.join(INDEX)).unwrap();
    temp.file(
        "project with spaces/src/lib.rs",
        b"this is not valid Rust!\n",
    );
    temp.file("target/keep-after-failure.rlib", b"keep");
    let failed = invoke(&temp, &["build"]);
    assert!(!failed.status.success());
    assert_eq!(fs::read(root.path.join(INDEX)).unwrap(), bytes);
    assert!(root.path.join("keep-after-failure.rlib").exists());
}

#[test]
fn cargo_run_preserves_program_json_and_nonzero_exit_code() {
    let temp = fixture();
    temp.file("target/untracked.txt", b"keep");
    let output = invoke(&temp, &["run"]);
    success(&output);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(r#"{"reason":"compiler-artifact","filenames":["untracked.txt"]}"#)
    );
    assert!(!state(&temp).known.contains("untracked.txt"));
    let before = fs::read(temp.path.join("target").join(INDEX)).unwrap();
    let output = invoke(&temp, &["run", "--", "fail"]);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        fs::read(temp.path.join("target").join(INDEX)).unwrap(),
        before
    );
}

#[test]
fn a_corrupt_index_does_not_fail_a_successful_build_or_delete_files() {
    let temp = fixture();
    temp.file(Path::new("target").join(INDEX), b"{broken");
    temp.file("target/keep.rlib", b"keep");
    let output = invoke(&temp, &["build"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Cache maintenance skipped"));
    assert_eq!(
        fs::read(temp.path.join("target").join(INDEX)).unwrap(),
        b"{broken"
    );
    assert!(temp.path.join("target/keep.rlib").exists());
}

#[test]
fn another_process_cannot_build_until_the_owner_releases_the_lock() {
    let temp = fixture();
    let root = Root::new(&temp.path.join("target")).unwrap();
    let lock = BuildLock::acquire(&root).unwrap();
    let output = invoke(&temp, &["build"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Another editor build is active"));
    assert!(!root.path.join(INDEX).exists());
    drop(lock);
    success(&invoke(&temp, &["build"]));
}

#[cfg(windows)]
#[test]
fn powershell_launcher_builds_and_locks_without_python_on_path() {
    let temp = fixture();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"));
    temp.file(
        "tools/cargo.ps1",
        &fs::read(source.parent().unwrap().join("cargo.ps1")).unwrap(),
    );
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        "src/lib.rs",
        "src/main.rs",
        "src/cache.rs",
    ] {
        temp.file(
            Path::new("tools/cargo-cache").join(file),
            &fs::read(source.join(file)).unwrap(),
        );
    }
    let system = std::env::var_os("SystemRoot").unwrap();
    let powershell = Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let cargo = std::env::var_os("CARGO").unwrap();
    let cargo_bin = Path::new(&cargo).parent().unwrap();
    let path =
        std::env::join_paths([cargo_bin.to_owned(), Path::new(&system).join("System32")]).unwrap();
    let launch = || {
        Command::new(&powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(temp.path.join("tools/cargo.ps1"))
            .args(["build", "--offline", "--manifest-path"])
            .arg(temp.path.join("project with spaces/Cargo.toml"))
            .env("PATH", &path)
            .env("CARGO_NET_OFFLINE", "true")
            .env("BEVY_EDITOR_TARGET_LIMIT_GB", "0.000000001")
            .current_dir(&temp.path)
            .output()
            .unwrap()
    };
    success(&launch());
    assert!(temp.path.join("target").join(INDEX).is_file());
    let root = Root::new(&temp.path.join("target")).unwrap();
    let lock = BuildLock::acquire(&root).unwrap();
    let output = launch();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Another editor build is active"));
    drop(lock);
    success(&launch());
}
