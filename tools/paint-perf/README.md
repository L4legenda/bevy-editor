# Painting comparison

`prepare.py` creates a UV-mapped relief sphere with 262,144 triangles and copies
Material Painter into `target/paint-perf/material-painter`. It adds a benchmark
only to that disposable copy; the original source tree is read only. No model or
benchmark output is committed.

Both benchmarks use the exact same float32 vertex file, 1024 × 768 camera, 58°
FOV, distance 4, brush radius 35, hardness 0.6, flow 0.35, eight populated color
layers, and six identical pointer samples per frame. The top layer is active.
Each texture size (1024 and 2048) runs 20 warmup and 180 measured frames.

The reported time includes submitting and **completing** brush/apply/composite
work; Material Painter also includes its stamp resolution and region lookup.
It excludes initial uploads, pipeline compilation, UI rendering, viewport
shading, VSync, undo and disk save. Consequently these are paint-processing
milliseconds, **not application FPS**. A blocking GPU poll is used only inside
the benchmark. Production painting never waits for the GPU.

```powershell
python tools/paint-perf/prepare.py G:/material-painter
$env:PAINT_PERF_DIR = "$PWD/target/paint-perf"
cargo test --locked --manifest-path target/paint-perf/material-painter/Cargo.toml --target-dir target/paint-perf/mp-build --bin material_painter compare_complex_paint -- --ignored --nocapture --test-threads=1
$env:PAINT_PERF_LABEL = 'bevy-after'
cargo test --locked --lib --config profile.test.opt-level=1 --config profile.test.debug=0 gpu_compare_complex_paint -- --ignored --nocapture --test-threads=1
```

The tests emit JSON timing records and UV PNGs under `target/paint-perf`.
The editor benchmark also compares every resulting pixel against a full CPU
composite. Run the same binary with `PAINT_PERF_SELECTED=0` or `3` to verify
painting below other populated layers. Use `gpu_model_paint_benchmark_40k` for
an additional complete native-editor measurement with idle frame cost reported
separately, and the other `gpu_model_paint_` tests for input, undo, save/reopen,
all channels, UV/3D views and no readback during a held stroke.

Run comparisons sequentially on the same adapter with other GPU applications
idle. Preserve the baseline executable/JSON before changing the implementation.

`compare.ps1` runs three sequential rounds with rotating application order,
using the saved `bevy-before.exe` and `bevy-after.exe`, and writes `comparison.json`.
The measured result and limitations are recorded in
[`docs/paint-performance.md`](../../docs/paint-performance.md).
