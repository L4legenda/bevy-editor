PhotoCraft core, vendored from the local PhotoCraft checkout at commit
`e052eabf4be74612725400955d184f85d208c0f0` (version 0.23.1).
The `craft-pcraft-format` envelope is from `storytold/craft-libs`, commit
`b8f7d93b00dfb8f172938f3d2d796ae62dcfe123`.

The CPU layer engine, compositor, operations, vectors, text, algorithms and
their native import dependencies are also vendored from the local checkout at
`85d713723dcab81ff03db6db4b3258054a36c46e` (0.23.1). Geometry, colour and raster
remain unchanged; the document and native manifest include the matching live
triangle model. No dependency uses an absolute path to the original checkout.
The Bevy interface uses PhotoCraft's layer commands and compositing semantics.
The upstream egui interface is not included. Both projects license these sources
under MIT OR Apache-2.0; the license files are included here.

The CPU text library embeds Inter and JetBrains Mono (SIL Open Font License)
and the SCOWL dictionary. Their license files accompany them in `assets/`.
Other craft-libs dependencies are pinned to upstream revisions in Cargo.lock.

Integration changes: workspace dependencies are local; imported library
manifests omit upstream benchmark and development-only dependencies. The format crate's
embedded PNG previews use the already bundled `image` PNG encoder instead of
the complete PhotoCraft codec suite. The .pcraft manifest, tile storage,
compression, validation and atomic file operations are unchanged.

The engine exposes its existing smart-filter-mask refresh function for the
native host's live-stroke renderer. The conversion of layer-style properties
in `src/editor/photo_effect_params.rs` is adapted from the upstream dialog;
opaque effect snapshots are retained to preserve unedited imported data.
