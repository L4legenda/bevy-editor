# Native UV engine

The editor embeds a CPU-only Rust + C++ worker. End users need no Python, pip, uv, CUDA, compiler, package downloads or runtime installation. On Windows the worker uses a static C/C++ runtime; its OpenMP DLL is embedded alongside it.

## Architecture

- Rust prepares nonmanifold edge fans and builds deterministic connectivity-constrained Ward clusters from normals and normalized centroids.
- A C ABI calls the vendored PartUV Geometric pipeline directly: ABF, five iterations, the existing distortion threshold and CPU overlap checks.
- xatlas packs the real island contours without recomputing the unwrap. Local opt-in extensions keep caller-defined islands intact, include collapsed connectors in their masks, and disable independent X/Y texel snapping. The adapter applies a rotation/reflection and one common uniform scale to preserve shape and relative texel density. Tiny islands with a degenerate hull axis retain their original axes. Several orientation/placement candidates are compared; grid search is capped at 16,384 candidates per island before retaining a valid placement or using random search. Rectangular fallback remains available on validation failure.
- Per-corner output is mapped to the input triangle order. Changed coverage, non-finite UVs and coordinates outside the square are rejected.

The isolated native process lets cancellation terminate all native threads and keeps native failures out of the editor process. The editor retains document validation, stale-result protection, undo/redo, comparison, selection, checker, seams, transforms, repacking and GLB save.

## Build

Build the editor normally with Cargo. `build.rs` invokes `native/uv-build`, builds this independent Cargo package and embeds the executable and required runtime DLLs. Windows builds require the usual Rust MSVC and Visual C++ build tools on the developer machine. These tools are not required on the user's machine.

```powershell
cargo build --locked --bin bevy-editor
cargo build --locked --manifest-path native/uv/Cargo.toml --release
cargo test --manifest-path native/uv/Cargo.toml
```

C++ sources and headers are vendored. `vendor/dependencies.json` pins the source revisions and SHA-256 of `vendor/headers.zip`. The build verifies the archive before extracting it into its output directory. Cargo dependencies follow their lockfiles. `tools/vendor-archive.ps1` regenerates the archive from the original SDK archives under `target/partuv-native`; that development folder is never needed to build or run a checkout containing the archive.

`BEVY_EDITOR_UV_NATIVE_DIR` is a build-time override pointing to a prepared executable/DLL directory. At runtime `BEVY_EDITOR_UV_WORKER` overrides only the native executable and `BEVY_EDITOR_UV_HOME` changes cache/log storage. Normal users need none of these.

## Protocol and runtime

```text
bevy-uv-native INPUT.json OUTPUT.json
```

Input contains `positions`, `triangles`, `threshold`, `resolution`, `padding` and optional per-triangle/corner `existing_uv`. Supplying `existing_uv` packs existing islands; omitting it performs automatic unwrapping. Output contains per-corner `uv`, `charts` and `utilization`. Progress lines start with `BEVY_UV:` followed by JSON with `phase` and `fraction`; other output is logged. The result file is written via a temporary file.

The editor extracts the embedded bytes into `%LOCALAPPDATA%/BevyEditor/uv-runtime/native-<content hash>` and verifies cached bytes before use. Per-job JSON and logs are stored below the same runtime directory. No paths to the benchmark folder or an installed Python environment are used.

## Verification

```powershell
cargo test --lib uv_ -- --test-threads=1
$env:BEVY_EDITOR_UV_QA_MODEL = 'G:/path/to/character.glb'
cargo test --lib gpu_uv_workspace -- --ignored --nocapture --test-threads=1
```

The GPU fixture operates on a temporary model copy. It exercises the real UV workspace, auto unwrap, comparison, island dragging, undo/redo, cancellation, packing and GLB save, while verifying that project scene state and history are unchanged. Screenshots go to `target/uv-qa`. Native regressions cover extraction with only the Windows system PATH, cancellation during a live native calculation, and shape-preserving packing of sliver triangles.

For a migration comparison, feed the identical request JSON to a saved legacy worker and to the native executable. The legacy interpreter is used only to obtain a reference during development. Production contains neither the legacy worker nor an interpreter. Compare results using the Rust metric tool:

```powershell
cargo run --release --manifest-path native/uv/Cargo.toml --example quality -- INPUT.json REFERENCE.json NATIVE.json REPORT.json
cargo run --release --manifest-path native/uv/Cargo.toml --example ward_probe -- INPUT.json PREPARED.json
cargo run --release --manifest-path native/uv/Cargo.toml --example packing_check -- REPACK_INPUT.json PACKED.json CHECK.json
```

The quality tool reports surface-area-weighted angular singular-value ratios and normalized stretch, summed UV triangle area, seams, collapsed surface fraction and out-of-range faces. UV area is not an overlap certificate.

The independent `packing_check` tool requires `existing_uv` in the input. It checks a single similarity transform per original island, common scale, unchanged island count, bounds, and cross-island triangle intersections. Integration tests exercise concave shapes, sliver connectors and tiny islands.

See `NOTICE.md`, `LICENSE`, `licenses/` and the original vendor notices for component attribution. The native worker is GPL-3.0-or-later because it includes CGAL GPL components.
