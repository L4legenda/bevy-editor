The brush falloff, flow coverage and non-premultiplied paint/erase equations in
`src/editor/model_paint_data.rs`, `model_paint_engine.rs` and `model_paint_gpu*.wgsl` are adapted from
Material Painter's `src/shaders/stamp.wgsl` and `src/shaders/apply.wgsl`. The
layer blend equations are adapted from `src/shaders/wear.wgsl`. Mask filters
and mask fill blending in `src/editor/model_paint_mask*.rs` and
`model_paint_gpu_mask_filter.wgsl` are adapted from
`src/shaders/mask_filter.wgsl` and the noise helpers in `src/shaders/common.wgsl`.
The Clouds and Height mask generators and overpaint/erase behavior adapt
`src/shaders/wear.wgsl`, `src/shaders/composite.wgsl` and the mask pass in
`src/shaders/apply.wgsl`. Source commit
`1456397558798e5ef51588270db85bf3c310332f`.

`assets/paint/brush-tips.png` contains all 24 original procedural stamp tips
rendered at 256 × 256 using `BRUSH_ALPHAS[i].render(256)` from
`src/brushes.rs` and its `src/textures.rs` helpers at that revision. Tiles
are in source order, six per row. The shader samples them using the rotated
offset from Material Painter's `src/shaders/stamp.wgsl`.

`assets/paint/brush-grains.png` contains all 25 tileable procedural textures
rendered with `TEXTURES[i].render(256)` from `src/textures.rs` at the same
revision. The tiles are in source order, five per row. The grain coverage
adapts `src/shaders/apply.wgsl`, and the reflection adapts the screen-space
symmetry in `src/shaders/stamp.wgsl` to Bevy's scene depth and object IDs.

The stamp spacing, pressure interpolation and stabilizer in
`src/editor/model_paint_stroke.rs` adapt `App::continue_stroke` from `src/app.rs`.
The surface and UV hit resolution in `src/editor/model_paint_surface.rs`
adapts `resolve_stamps` in `src/app.rs`, and the projected GPU coverage
adapts `stamp_surface` and `stamp_uv` from `src/shaders/stamp.wgsl`.
The region modes in `src/editor/model_paint_fill.rs` adapt `src/polyfill.rs`
to the editor's polygon topology and GPU stroke history.
Rectangle and lasso selection and hover outlines use the interaction in
`src/app.rs::view_input` and the fill geometry in `src/polyfill.rs` as references.
UV padding follows the border strips in `src/polyfill.rs::uv_geometry` and
the occupancy test in `src/engine.rs::encode_polys`.
Brush scatter, size jitter, and angle jitter follow `src/app.rs::make_stamp`.
The straight stroke connects the last painted point as in `src/app.rs::view_input`.
Curve sampling, stamp spacing, and the path tool interaction in
`src/editor/model_paint_path.rs` follow `src/app/path_tool.rs`.
The built-in material catalog in `src/editor/model_paint_materials.rs`
uses the 54 material presets from `src/presets.rs` at the same revision.
The progressive shader in `src/editor/model_paint_pathtrace.wgsl` and
environment sampling distribution in `model_paint_pathtrace.rs` adapt
`src/shaders/pathtrace.wgsl`, `src/shaders/common.wgsl` and
`src/environment.rs` from that revision. The Bevy adapter reuses the model
editor BVH and samples its HDRI cubemap. The shared viewport effects in
`model_paint_post.wgsl` and `model_paint_post.rs` adapt
`src/shaders/post.wgsl`, `src/shaders/bloom.wgsl`, the grain hash from
`src/shaders/common.wgsl` and bloom scheduling from `src/post.rs`.
`src/editor/model_paint_smart.rs` adapts the 63 smart material recipes and
micro-detail layers from that file. The original CC0 Poly Haven maps are
in `assets/paint/pbr`, with their attribution in `assets/paint/pbr/LICENSE.txt`.

Upstream: https://github.com/alla2001/material-painter
License: GPL-3.0-or-later (included). The port uses the editor's native Bevy UI,
asset system, Bevy render device, worker pool and GLB documents rather than
upstream's separate egui/wgpu application runtime. Live stamps, stroke opacity,
erasing and compositing run on GPU; asynchronous layer snapshots retain the
editor's undo and GLB persistence.

`graph/` adapts Material Painter's `src/graph`, the WGSL helpers from
`src/shaders/common.wgsl` and `src/shaders/wear.wgsl`, and texture helpers
from `src/gpu.rs` at the same revision. Bevy Editor uses its existing render
device and queue to run the graph. The bridge stores the graph definition and
rendered textures in the paint layer.

The shading graph compiler and preview presets come from
`src/graph/shading.rs`. The 3D preview uses Bevy's PBR lighting and textures
from the selected model.
