# Cargo cache manager

Rust-only artifact inventory for `tools/cargo.ps1` and the Windows editor launcher.
The crate has its own workspace and lockfile, so bootstrapping does not compile
Bevy. PowerShell only builds and launches this executable; Python is not used.

Builds do not scan the target size or delete cached artifacts. There is no size
threshold. Build/run share a category; check/clippy share a category; profiles and
explicit target triples have separate inventory snapshots.

Cargo's `compiler-artifact` messages include fresh dependencies. Only a successful
Cargo command with a successful `build-finished` event and observed artifacts
updates the index. Compilation diagnostics and program/test
output pass through; a program's JSON output is not parsed as Cargo messages.
Exit codes are preserved. Inventory failures do not fail a successful build.

`target/.bevy-editor-cache.json` version 1 remains compatible, including keys
written by the old Python wrapper. The index is replaced atomically. The process
holds `.bevy-editor-build.lock` from before Cargo starts until the inventory is saved;
PowerShell also locks the independent helper's bootstrap stage.

Cleanup is a separate, explicit action: double-click `clean-cache.cmd` in the
repository root, or run `./tools/cargo.ps1 clean`. PowerShell invokes `cargo clean`
for this repository's fixed `target` directory without bootstrapping the helper.
It holds the same build lock, rejects a linked target and refuses cleanup while
programs from that target are running. All target contents, including the helper,
inventory, current builds and generated QA files, are removed. Sources and projects
outside target are kept. The next editor launch rebuilds the application.
Plain Cargo commands bypass the wrapper's shared build lock and inventory.

Tests use isolated temporary projects and never prune the editor's real cache:

```powershell
cargo test --locked --manifest-path tools/cargo-cache/Cargo.toml --target-dir target/.bevy-editor-tools/cargo-cache
```

They cover actual Cargo builds, fresh dependencies, retention of old artifacts,
failed builds/runs, malformed indexes, path escape and junction protection,
cross-process locking, and the Windows PowerShell launcher with no Python on PATH.
The manual-script test checks full cleanup in an isolated temporary project,
rejection during a build or running program, and preservation of files outside target.
