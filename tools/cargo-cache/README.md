# Cargo cache manager

Rust-only cache maintenance for `tools/cargo.ps1` and the Windows editor launcher.
The crate has its own workspace and lockfile, so bootstrapping does not compile
Bevy. PowerShell only builds and launches this executable; Python is not used.

The default threshold remains **6 GiB**, configured through
`BEVY_EDITOR_TARGET_LIMIT_GB` in the launcher. This is a soft limit: only
previously observed Cargo artifacts absent from all retained snapshots may be
deleted. Unknown files and current build, test and check artifacts take priority
over reaching the threshold. Build/run share a category; check/clippy share a
category; profiles and explicit target triples have separate snapshots.

Cargo's `compiler-artifact` messages include fresh dependencies. Only a successful
Cargo command with a successful `build-finished` event and observed artifacts
updates the index and runs maintenance. Compilation diagnostics and program/test
output pass through; a program's JSON output is not parsed as Cargo messages.
Exit codes are preserved. Maintenance failures do not fail a successful build.

`target/.bevy-editor-cache.json` version 1 remains compatible, including keys
written by the old Python wrapper. The index is replaced atomically. The process
holds `.bevy-editor-build.lock` from before Cargo starts until maintenance ends;
PowerShell also locks the independent helper's bootstrap stage.

Each deletion is a single file with a verified absolute path inside the selected
target directory. Links/junctions, Cargo internals, the running helper and its
build directory are protected. Age is never a reason to remove a file.
Plain Cargo commands bypass the wrapper's lock and inventory; refresh the relevant
category through the wrapper after using Cargo directly.

Tests use isolated temporary projects and never prune the editor's real cache:

```powershell
cargo test --locked --manifest-path tools/cargo-cache/Cargo.toml --target-dir target/.bevy-editor-tools/cargo-cache
```

They cover actual Cargo builds, fresh dependencies, test-artifact retention,
failed builds/runs, malformed indexes, path escape and junction protection,
cross-process locking, and the Windows PowerShell launcher with no Python on PATH.
