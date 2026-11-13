PhotoCraft core, vendored from the local PhotoCraft checkout at commit
`e052eabf4be74612725400955d184f85d208c0f0` (version 0.23.1).
The `craft-pcraft-format` envelope is from `storytold/craft-libs`, commit
`b8f7d93b00dfb8f172938f3d2d796ae62dcfe123`.

Only the geometry, ICC color management, color, raster, document, brush and
native format libraries are included. Their native document and brush models
are used by Bevy Editor's image editor. Both projects license these sources
under MIT OR Apache-2.0; the license files are included here.

Integration changes: workspace dependencies are local; the format crate's
embedded PNG previews use the already bundled `image` PNG encoder instead of
the complete PhotoCraft codec suite. The .pcraft manifest, tile storage,
compression, validation and atomic file operations are unchanged.
