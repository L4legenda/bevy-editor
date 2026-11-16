The brush falloff, flow coverage and non-premultiplied paint/erase equations in
`src/editor/model_paint_data.rs` and `model_paint_engine.rs` are adapted from
Material Painter's `src/shaders/stamp.wgsl` and `src/shaders/apply.wgsl`, commit
`1456397558798e5ef51588270db85bf3c310332f`.

Upstream: https://github.com/alla2001/material-painter
License: GPL-3.0-or-later (included). The port uses the editor's native Bevy UI,
asset system, CPU worker pool and GLB documents rather than upstream's separate
egui/wgpu application runtime.
