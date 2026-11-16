The brush falloff, flow coverage and non-premultiplied paint/erase equations in
`src/editor/model_paint_data.rs`, `model_paint_engine.rs` and `model_paint_gpu*.wgsl` are adapted from
Material Painter's `src/shaders/stamp.wgsl` and `src/shaders/apply.wgsl`, commit
`1456397558798e5ef51588270db85bf3c310332f`.

Upstream: https://github.com/alla2001/material-painter
License: GPL-3.0-or-later (included). The port uses the editor's native Bevy UI,
asset system, Bevy render device, worker pool and GLB documents rather than
upstream's separate egui/wgpu application runtime. Live stamps, stroke opacity,
erasing and compositing run on GPU; asynchronous layer snapshots retain the
editor's undo and GLB persistence.
