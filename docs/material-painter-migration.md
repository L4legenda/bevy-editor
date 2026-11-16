# Material Painter integration

Source inspected: `G:\material-painter`, Material Painter 0.1.7. Its layout is
defined in `src/app/dock.rs`; feature boundaries are documented in its AGENTS.md.
The source repository is GPL-3.0-or-later. Brush coverage and straight-alpha
paint/erase math are adapted from its stamp/apply shaders; attribution and the
license are in `vendor/material-painter`. The workspace uses our Bevy UI and icons.

## First stage: Material Paint workspace

- Model mode selector: Object, Edit, UV, Paint (Рисование).
- The existing isolated 3D view, camera navigation, lighting and environment.
- A neighbouring 2D UV preview of the selected object's material slot; raster
  preparation runs on a worker, without blocking UI input.
- Left tool rail: brush, eraser, polygon fill, picker, path; brush configuration.
- Bottom resource shelf and category navigation; six procedural material swatches.
- Right tabs: layer stack, mesh maps, texture sets, preview; properties underneath.
- In-memory layer configuration, add/copy/delete/reorder/visibility and undo/redo.
- Per-model workspace state and a return to the original model-editing layout.

## Surface painting and editable GLB layers

- Brush strokes paint the visible model surface using its UV0 coordinates. Screen
  interpolation avoids connecting distant UV islands; BVH occlusion prevents
  painting through other surfaces. Alt-orbit and camera navigation retain priority.
- The texture pane also supports painting, erasing, polygon fill and a color picker.
- A stroke accumulates flow coverage with smooth hardness falloff. Stroke opacity
  caps the result, and a complete stroke creates one undo entry.
- Every object/material slot has an independent texture set and layer history.
  Paint/fill layers support opacity, visibility, ordering, duplication and deletion.
  Material presets set the fill color, roughness and metallic values.
- Texture channels are color, roughness, metallic, height (converted to normal
  detail), opacity, emission, normals and specular. The current Bevy preview uses
  an averaged specular level; the GLB stores its per-pixel specular texture.
- Live brush rasterization and compositing run on Bevy's GPU device. Geometry
  preparation and CPU document snapshots use the compute pool. Scene asset
  materials remain isolated from the model's working material.
- Save / Ctrl+S uses the model's existing atomic GLB save path. Standard glTF
  textures contain the flattened result, while versioned primitive extras point
  to losslessly compressed base textures and layer buffers in the same GLB.
  Reopening restores editable layers. Rename/duplicate naturally keep all data.
  Saving during a stroke waits for its worker results.
- Texture sets currently use 1024 × 1024. A valid UV0 unwrap is required; the UI
  links to the editor's existing UV workspace for meshes without one. Overlapping
  UVs share texture pixels, as in ordinary UV painting.

The existing environment, scene tools and mesh-editing operations stay independent.
Mesh-map baking, masks, folder parenting, path strokes, graph editing, SPP import,
UDIMs and upstream's separate path tracer are subsequent migration work. Path and
folder creation remain disabled until their backends are connected.

## Interactive painting performance

Upstream `engine.rs::add_region` and the stamp/composite passes keep textures on
the GPU, scissor draws to the stroke region, and resume compositing at the changed
layer. The first Bevy port instead rasterized each stamp independently with a BVH
ray per texel, recomposited eight complete maps and replaced nine Image assets on
each result. At 1024px that uploaded 36 MiB per batch and repeatedly prepared GPU
textures/bind groups. Rebuilding layer panels and copying/comparing pixel buffers
on the main thread added stalls.

The live path now uses resident GPU buffers and the application's existing render
device/queue, with no second graphics context:

1. Prepare transformed triangle vertices on a worker. Render depth and object/face
   IDs for the stroke view; the UV pass supports ordinary and overlapping UVs.
2. Rasterize screen/UV brush coverage into a persistent floating-point mask. Flow
   accumulates there. Only stamp coordinates and brush uniforms cross from CPU to
   GPU while drawing; a stationary brush enqueues no extra work.
3. A compute shader applies that mask to a GPU copy of the layer taken at stroke
   start, preserving opacity caps and straight-alpha erasing. Visible layers in
   the active channel are then composited on GPU.
4. GPU passes pack color/opacity and metallic/roughness, derive height/normal detail,
   and copy into the existing material/2D preview textures without recreating them.
5. At release, asynchronously copy the active layer to a readback buffer. A worker
   checks changes and publishes one undo transaction into the existing CPU/GLB
   document. GPU submission order and stroke IDs preserve consecutive strokes.
   Save/close wait for outstanding snapshots; runtime code never waits on the GPU.

Shader pipelines are created at renderer startup, before the first brush press.
Initial previews and standalone layer configuration/undo still use the CPU
compositor; they are not on the interactive stamp path. The old CPU rasterizers
remain under `cfg(test)` as correctness/performance references.

Native GPU tests cover live color before any CPU readback, all eight channels,
occlusion, erasing/fill, orthographic views, stable image allocations, history and
GLB save/reopen. `gpu_model_paint_benchmark_40k` measures a held stroke over 40,000
triangles and asserts zero readbacks during drawing; the earlier CPU benchmark
remains available for comparison.

## Remaining functional migration

| Feature | Source modules | Integration boundary |
|---|---|---|
| Folder parenting, masks, additional blend modes, GPU-resident history | `engine.rs`, `engine/`, `textures.rs`, `undo.rs`, `shaders/` | Extend the current editable texture sets and GPU path |
| Pressure, stabilization, symmetry, alpha tips, extra projections and paths | `app/view_input`, `brushes.rs`, `polyfill.rs`, `app/path_tool.rs` | Extend native viewport painting and tablet events |
| UDIM and additional mesh formats | `app/texture_sets.rs`, `mesh.rs`, `import.rs`, `unwrap.rs` | Existing GLB documents and UV workspace |
| Mesh-map and high-poly/cage baking | `bake.rs`, `bake_cache.rs`, `cage.rs`, `app/cage_paint.rs` | Background preparation, incremental GPU jobs and cancellation |
| Smart materials, masks, generators, shelf and online libraries | `presets.rs`, `library.rs`, `app/library_ui.rs` | Asset catalog, thumbnails and downloads |
| Material Designer and custom graph nodes | `graph/`, `app/graph_ui.rs` | Native graph workspace, typed connections and GPU evaluation |
| Shader Maker and preview shaders | `app/shader_ui.rs`, `app/preview_shader.rs`, `graph/shading.rs` | Shader editor, diagnostics and preview pipelines |
| Substance import, export presets and channel packing | `app/project_io.rs`, `spp_import.rs`, `app/spp_*`, `export.rs` | Extend the current embedded GLB layer document |
| Ray-traced preview, post effects, pen and SpaceMouse | `raytrace.rs`, `post.rs`, `pen.rs`, `spacemouse.rs` | Optional viewport backends and native input |

The source uses eframe/egui 0.37 and wgpu 30. Its UI and renderer cannot be
installed unchanged as a Bevy widget. Reuse of its engine must preserve its
license notices and replace the egui/window/GPU integration points; replacing
the application's existing environment system is not part of the requested port.
