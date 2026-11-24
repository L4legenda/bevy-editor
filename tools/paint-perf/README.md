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

## Mesh-map baking

```powershell
python -B tools/paint-perf/prepare.py G:/material-painter --bake
./tools/paint-perf/compare-bake.ps1
```

Both applications bake the same 16,384-triangle relief torus at 512 and 1024
pixels, then project its 262,144-triangle high-poly OBJ at 512 pixels. Both
use Standard quality and all built-in maps. The script checks the adapter and
graphics API and records the median of three runs after one warm-up.

The timer starts with CPU scene preparation and stops when the maps are in
CPU memory, before PNG encoding or disk writes. Material Painter uses its
existing mesh buffers and bake pipelines, as it does with an open mesh.
The editors retain their own padding and map packing. Raw samples and logs
are written to `target/paint-perf`.

To isolate queueing in Bevy, run the same test twice with
`PAINT_BAKE_SERIAL` set to `1` and then `0`.

```powershell
$env:PAINT_PERF_DIR = "$PWD/target/paint-perf"
$env:WGPU_BACKEND = 'vulkan'
foreach ($mode in '1', '0') {
    $env:PAINT_BAKE_SERIAL = $mode
    cargo test --locked --offline --lib --config profile.test.opt-level=1 --config profile.test.debug=0 --config profile.test.package.bevy-editor.opt-level=1 gpu_compare_mesh_bake -- --ignored --nocapture --test-threads=1
}
Remove-Item Env:PAINT_BAKE_SERIAL
```

The mode changes only how many visibility and custom-map packets can be in
flight. Output includes timing by stage and a digest of the baked texels,
projection and coverage. Compare digests for the same resolution and source.

For the same-device visibility comparison, set
`PAINT_BAKE_COMPARE_VISIBILITY=1` and run `gpu_compare_mesh_bake` once with
`PAINT_BAKE_SERIAL=0`. The test alternates CPU readback and direct GPU transfer
to the denoiser, checks identical digests, and reports
`visibility_readback_bytes` before denoising. Final map readback is included in
the timing in both modes. Clear the comparison variable before running the
cross-application script, which expects one set of results per editor.

`PAINT_BAKE_COMPARE_PROJECTION=1` selects only the 512-pixel high-poly case and
alternates one and three outstanding projection batches. Keep the visibility
comparison variable unset and set `PAINT_BAKE_SERIAL=0` to keep visibility
queueing fixed. The test reports `projection_peak` and checks the output digest
across both schedules.

`PAINT_BAKE_COMPARE_HIGH_SOURCE=1` selects the same high-poly case and compares
fresh high-poly preparation with reuse of the parsed mesh, BVH and relief.
Change only this comparison variable and leave the result cache disabled.
The test checks `high_source_hit`, `high_relief_hit` and identical digests.

## Path tracing

```powershell
python -B tools/paint-perf/prepare.py G:/material-painter
./tools/paint-perf/compare-pathtrace.ps1
```

The comparison uses the same 36,864-triangle torus, two material sets,
perspective camera and four bounces at 512 × 320. Bevy samples the editor
HDRI cubemap. Material Painter receives an equirectangular HDR conversion
of the same image through its native environment loader. The cube and
panorama have different sampling footprints.

Each timed frame submits both tracing and display processing and waits for
the GPU. The timing excludes warm-up, packing, readback and the UI. The
comparison uses scene-linear captures, applies the same display curve for
the image grid and measures luminance deviation relative to each renderer's
own 256-sample frame. The reports and grid are saved in
`target/paint-perf/path-trace/compare`.

For the complete editor viewport, run `gpu_measure_paint_trace_during_camera_orbit`
as an ignored library test. It sends Alt-left orbit input on the same torus
with the preview paused during camera motion and with tracing forced to
continue. Each timed frame includes the UI and raster camera and waits for
GPU completion. The headless fixture runs with a fixed simulation step
and bypasses presentation and VSync. The report is written to
`target/paint-perf/path-trace/interactive-orbit.json`.
