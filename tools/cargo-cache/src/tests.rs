use super::*;
use cache::{INDEX, Root, State, prune};
use std::fs;
#[path = "../tests/support/mod.rs"]
mod support;
use support::Sandbox;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| s.to_string()).collect()
}

#[test]
fn only_known_superseded_files_are_removed_above_the_limit() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    for name in ["old.rlib", "current.rlib", "tests.exe", "unknown.zip"] {
        fs::write(root.path.join(name), [1u8; 128]).unwrap();
    }
    let mut state = State {
        known: names(&["old.rlib", "current.rlib", "tests.exe"]),
        ..Default::default()
    };
    state
        .snapshots
        .insert("build".into(), names(&["current.rlib"]));
    state.snapshots.insert("test".into(), names(&["tests.exe"]));
    let report = prune(&root, &mut state, 1).unwrap();
    assert_eq!(report.removed, 128);
    assert!(!root.path.join("old.rlib").exists());
    for name in ["current.rlib", "tests.exe", "unknown.zip"] {
        assert!(root.path.join(name).exists());
    }
    assert!(
        report.remaining > 1,
        "The soft limit cannot justify deleting protected data"
    );
}

#[test]
fn below_threshold_leaves_superseded_files_untouched() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    fs::write(root.path.join("old.rlib"), b"keep").unwrap();
    let mut state = State {
        known: names(&["old.rlib"]),
        ..Default::default()
    };
    assert_eq!(prune(&root, &mut state, 1024).unwrap().removed, 0);
    assert!(root.path.join("old.rlib").exists());
}

#[test]
fn outside_paths_and_aliases_cannot_bypass_active_protection() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    let outside = temp.file("outside.txt", b"keep");
    fs::write(root.path.join("live.rlib"), b"keep").unwrap();
    fs::create_dir(root.path.join("sub")).unwrap();
    let mut state = State {
        known: names(&["../outside.txt", "sub/../live.rlib"]),
        ..Default::default()
    };
    state.known.insert(outside.to_string_lossy().into_owned());
    state.snapshots.insert("test".into(), names(&["live.rlib"]));
    assert!(root.local_file(Path::new("../outside.txt")).is_none());
    assert!(root.local_file(&outside).is_none());
    assert_eq!(prune(&root, &mut state, 0).unwrap().removed, 0);
    assert!(outside.exists());
    assert!(root.path.join("live.rlib").exists());
}

#[test]
fn cargo_internal_data_and_helper_are_never_cleaned() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    let paths = [
        "debug/.fingerprint/keep",
        "debug/build/pkg/output",
        "debug/incremental/keep",
        ".bevy-editor-tools/cargo-cache/debug/manager.exe",
        ".cargo-lock",
    ];
    for name in paths {
        let path = root.path.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"keep").unwrap();
    }
    let mut state = State {
        known: names(&paths),
        ..Default::default()
    };
    assert_eq!(prune(&root, &mut state, 0).unwrap().removed, 0);
    for name in paths {
        assert!(root.path.join(name).is_file());
    }
}

#[test]
fn python_index_is_migrated_without_losing_other_categories() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    fs::write(root.path.join(INDEX),br#"{"version":1,"snapshots":{"[\"build\", \"dev\", \"host\"]":["old.rlib"],"[\"test\", \"release\", \"wasm32\"]":["tests.wasm"]},"known":["old.rlib","tests.wasm"]}"#).unwrap();
    let mut state = State::load(&root).unwrap();
    let key = snapshot_key(&args(&["build"]));
    assert!(state.snapshots.contains_key(&key));
    state.snapshots.insert(key, names(&["new.rlib"]));
    state.save(&root).unwrap();
    let reloaded = State::load(&root).unwrap();
    assert_eq!(reloaded.snapshots.len(), 2);
    assert_eq!(
        reloaded.snapshots[&snapshot_key(&args(&["test", "--release", "--target=wasm32"]))],
        names(&["tests.wasm"])
    );
}

#[test]
fn corrupt_or_future_indexes_are_rejected() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    for bytes in [
        r#"{"version":2,"snapshots":{},"known":[]}"#,
        r#"{"version":1,"snapshots":{},"known":[12]}"#,
        r#"{"version":1,"snapshots":{"build":true},"known":[]}"#,
        "{",
    ] {
        fs::write(root.path.join(INDEX), bytes).unwrap();
        assert!(State::load(&root).is_err());
    }
}

#[test]
fn failed_atomic_save_does_not_remove_an_existing_temporary_file() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    let file = root
        .path
        .join(format!("{INDEX}.{}.tmp", std::process::id()));
    fs::write(&file, b"unknown").unwrap();
    assert!(State::default().save(&root).is_err());
    assert_eq!(fs::read(file).unwrap(), b"unknown");
}

#[test]
fn categories_profiles_and_targets_respect_the_program_argument_separator() {
    let key = |a: &[&str]| snapshot_key(&args(a));
    assert_eq!(
        key(&["build"]),
        key(&["run", "--", "--release", "--target=game"])
    );
    assert_eq!(key(&["check"]), key(&["clippy"]));
    assert_ne!(key(&["build"]), key(&["test"]));
    assert_ne!(key(&["build"]), key(&["build", "--release"]));
    assert_eq!(
        key(&["build", "--profile", "custom", "--target", "wasm32"]),
        key(&["build", "--profile=custom", "--target=wasm32"])
    );
    assert_ne!(
        key(&["build"]),
        key(&["build", "--target=x86_64-pc-windows-msvc"])
    );
}

#[cfg(any(windows, unix))]
#[test]
fn directory_links_are_not_traversed_or_accepted_as_a_target() {
    let temp = Sandbox::new();
    let root = Root::new(&temp.path.join("target")).unwrap();
    let outside = temp.path.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    let link = root.path.join("alias");
    #[cfg(windows)]
    {
        let result = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(Root::new(&link).is_err());
    assert!(root.local_file(&link.join("keep")).is_none());
    let mut state = State {
        known: names(&["alias/keep"]),
        ..Default::default()
    };
    assert_eq!(prune(&root, &mut state, 0).unwrap().removed, 0);
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    fs::remove_dir(&link).unwrap();
}
