# Native image-to-3D generation

The footer opens **3D генерация**. With installed weights the initial screen has
**Включить модель** in the center. Loading reports progress; source, quality and
output controls appear after initialization succeeds. Model loading includes a
real CUDA/cuBLAS calculation and checkpoint validation.

The generator is a persistent hidden Rust process. Switching or closing tabs
preserves the loaded weights and a running generation. The bottom-right status
is visible throughout the editor; **Остановить** terminates only the owned
worker, releasing host weights and its CUDA context. Closing the editor does the
same. **Отменить генерацию** cancels the current job and retains the loaded model.

## Implementation

`crates/hunyuan-native` is an independent Rust crate using Candle 0.9.2 and CUDA.
It implements DINOv2 ViT-g/14 with SwiGLU, mini dual/single-stream DiT, FlowMatch
Euler, ShapeVAE, alpha composition/resizing, adaptive occupancy evaluation,
Surface Nets, topology-checked QEM reduction and self-contained GLB/OBJ export.
There is no Python, PyTorch, Python package installation, external inference
server or fallback to those in this generation path. Native NVIDIA CUDA driver
and libraries are required.

Weights remain in RAM across jobs. For an 8 GB GPU alongside the editor,
conditioning, diffusion and surface decoding transfer weights to CUDA in turn.
Attention and surface queries are chunked to bound working memory. These stage
transfers do not reread/reinitialize the checkpoint on each generation.

The standard mini fp16 Safetensors checkpoint is used without weight conversion
or retraining. Native seeded noise is reproducible, but does not reproduce the
PyTorch RNG bit for bit. Rust image filters, adaptive evaluation and Surface Nets
also differ from the old OpenCV/marching cubes path; the same seed does not
promise an identical mesh across backends. Generation is shape-only, with a
neutral material, without texturing or automatic background removal.

## Build and distribution

For a Windows source checkout, start `run-editor.vbs` or `run-editor.cmd`.
They use `tools/build-editor.ps1` to build the editor and, when an NVIDIA driver
is detected, prepare `bevy-hunyuan.exe` and all required CUDA DLLs in `target/debug`.
Cargo checks the generator's freshness after a pull. First-time preparation can
download the pinned CUDA toolkit and compile kernels; model weights are loaded
only after pressing **Включить модель** in the editor. `BEVY_EDITOR_PREPARE_3D=0`
skips the optional CUDA preparation; `1` forces it. The shared build lock covers
generator preparation too. Plain `cargo run` does not invoke this launcher step.

On Windows with Rust and Visual Studio C++ build tools:

```powershell
./tools/build-hunyuan.ps1
# Optimized worker plus native libraries, next to a packaged editor:
./tools/build-hunyuan.ps1 -Release -Destination ./dist
```

The build helper uses an installed CUDA toolkit or downloads pinned, SHA-256
verified CUDA 12.8.1 NVIDIA redistributables into the editor system cache.
PowerShell is a development/build helper, never an inference runtime. The
packaged worker requires neither Rust nor Python on an end user's machine.
Kernels are built to PTX for Ampere or newer by default. Developers may set
`CUDA_COMPUTE_CAP` for another supported target before building.

The editor discovers `bevy-hunyuan.exe` alongside its own executable, in the
native crate's release/debug output, or at `BEVY_EDITOR_HUNYUAN_NATIVE`. CUDA DLLs
are found alongside the worker, in `CUDA_PATH/bin`, or in the managed runtime
cache. No shell commands are constructed from user input. Commands and progress
use stdin/stdout JSON; events and stderr are logged in the system model cache.

The generator is separate from the editor/game crate: a game build does not
link Candle, CUDA or weights. This package is editor tooling and must not be
copied into game assets.

## Storage and model download

The managed cache is outside project assets, on Windows at
`%LOCALAPPDATA%/BevyEditor/system/models`. Its `.gitignore` excludes its contents;
model paths are never serialized into documents. A missing-model screen offers
**Скачать** with byte progress, cancellation, resume and SHA-256 verification.

The standard Tencent fp16 checkpoint, config, LICENSE and NOTICE are pinned to
Hugging Face revision `f90a0f7df7d5e6f71109cf333f6a95a0ae3194a6` (about 3.82 GB).
Existing lab/Hugging Face weights are reused read-only. Fast/Turbo checkpoints
and Hunyuan3D-Paint are not supported.

Each generation gets a unique `.jobs` directory with an input snapshot, resolved
parameters and seed, `mesh-raw.glb`, final GLB, optional OBJ and result metadata.
The form keeps source, quality and name visible. Manual paths, advanced settings
and export options are collapsed. Progress covers loading, image analysis,
diffusion, surface evaluation, reduction, preview loading and project import.

The preview supports orbit, zoom, framing and **Сетка**. **Добавить в проект**
copies only final GLB and optional OBJ to `models/generated`, using suffixes to
avoid overwrites and rolling back partial imports. Runtime, weights, logs,
snapshots and raw meshes remain outside the project. **Открыть модель** opens the
imported asset in the normal model editor. Scene drafts and undo are untouched.

## Verification

Native CPU tests check surface orientation, closed topology after reduction,
GLB chunks/embedded normals and grid interpolation. Editor `generation_` tests
cover requests, progress, downloads, drafts and imports. Ignored GPU fixtures
exercise resident model controls and real preview/import.
`gpu_generation_live_hunyuan_image_to_project` loads the native model, generates
from a local cutout, previews, orbits and imports GLB/OBJ to a temporary project.
The cached fixture uses `target/generation-real-job.txt`.

Tencent architecture attribution, LICENSE and NOTICE are retained in the native
crate. Model license files stay with the downloaded weights.
