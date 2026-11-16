# Native UV component notices

The native worker is distributed under GPL-3.0-or-later; see `LICENSE`. Individual third-party source files retain their own copyright and license notices. This notice concerns the native worker and does not replace notices elsewhere in the editor.

| Component | Source / revision | License / notice location |
| --- | --- | --- |
| PartUV Geometric | [PartUV](https://github.com/EricWang12/PartUV), Windows source fork [IgorAherne/PartUV](https://github.com/IgorAherne/PartUV/tree/13176a925a2768ab2ed83901f33a89c6976a72f5), Zhaoning Wang and contributors | Apache-2.0 project; original composite license in `vendor/partuv/LICENSE` |
| xatlas | Jonathan Young and contributors; [f700c7790aaa030e794b52ba7791a05c085faf0c](https://github.com/jpcy/xatlas/tree/f700c7790aaa030e794b52ba7791a05c085faf0c) | MIT; `vendor/LICENSE-xatlas` |
| Eigen 3.4.0 | [Eigen](https://gitlab.com/libeigen/eigen) contributors | MPL-2.0 and individual file notices; `headers.zip:licenses/eigen` |
| libigl 2.5.0 | [libigl](https://github.com/libigl/libigl) contributors | MPL-2.0 and individual file notices; `headers.zip:licenses/libigl` |
| CGAL 6.0.1 | [CGAL](https://github.com/CGAL/cgal) authors | GPL/LGPL per module; `headers.zip:licenses/cgal`, including full license texts |
| Boost 1.87.0 | [Boost](https://www.boost.org/) contributors | BSL-1.0; `headers.zip:licenses/boost` |
| OpenABF | EduceLab, University of Kentucky; [OpenABF](https://github.com/educelab/OpenABF); headers as vendored by the pinned PartUV fork | Apache-2.0; `licenses/OpenABF-LICENSE`, `licenses/OpenABF-NOTICE`. License/notice retrieved from upstream revision `8f3ad0f36d95d6034c6a79f22bad4f203e23a536` |
| easy_profiler headers | Sergey Yagovtsev, Victor Zarubkin; [easy_profiler](https://github.com/yse/easy_profiler) | MIT OR Apache-2.0; `licenses/easy_profiler-MIT`. Profiling is compiled out |
| nlohmann/json 3.11.3 | Niels Lohmann and contributors | MIT; license preserved in `vendor/include/nlohmann/json.hpp` |
| stb_image_write | Sean Barrett and contributors | Public domain OR MIT; license preserved in `vendor/include/stb_image_write.h` |
| Visual C++ OpenMP runtime | Microsoft, `vcomp140.dll` from the installed MSVC redistributable tools | Microsoft Visual Studio redistributable runtime, bundled on Windows; not part of the GPL worker source |

Rust dependencies and their authors/licenses are recorded in the Cargo manifests and lockfiles.

Local PartUV changes select the fixed Geometric CPU configuration, disable profiler dumps, remove CUDA includes/calls from compiled CPU files, and resolve Windows header name conflicts. CUDA/PAMO, PartField, neural checkpoints and Python bindings are not built or run. The upstream composite license also describes NVIDIA code retained in the upstream source tree; those CUDA components are excluded from this worker's compilation.

The Rust adapter replaces Python mesh preparation and scikit-learn Ward clustering. The C++ adapter replaces Python bindings and xatlas-python. Source corner mapping, square atlas normalization, rigid packing fallback and cancellation are implemented locally.

Local xatlas extensions preserve caller-defined islands (including collapsed connectors), disable independent X/Y texel stretching, rasterize thin edges before padding, and allow alternate 180-degree orientations. The adapter restores a rotation/reflection and one common uniform scale for all islands. These changes allow contour packing without creating seams or changing island proportions. The opt-in flags default to upstream behavior for other callers.
