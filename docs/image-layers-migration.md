# Image layers

Reference: the local PhotoCraft checkout at `G:/photocraft` (0.23.1).
The integration uses its document model, compositor and command implementations;
the editor owns the native Bevy interface, source-image export and history.

## Parity work

- [x] Portable vendored CPU engine and compositor, with attribution.
- [x] Stable layer IDs, nested groups, multi-selection and undo/redo with painting.
- [x] Add, duplicate, delete, rename, arrange, group/ungroup and drag placement.
- [x] Visibility/isolation, labels, links, locks, opacity/fill and all blend modes.
- [x] Pixel/vector masks, clipping, mask properties, mask editing and applying masks.
- [x] Fill/adjustment layers, editable properties and rasterization.
- [x] Layer effects, blend options, presets, copy/paste and global light.
- [x] Shared styles, gradients, shapes, patterns and effect defaults persist between runs.
- [x] Smart objects, frames/artboards, merge/flatten and layer export.
- [x] Compact native tree, thumbnails, contextual controls and keyboard shortcuts.
- [x] 16 document/compositor/command/history tests and 4 property-form tests passed.
- [x] 17 existing image-editor regression tests passed; GPU checks are opt-in.
- [x] GPU checks: native layer tree, effects, masks/filter masks, properties, panel options,
  painting, saving and reopening. Captures: `target/image-layers-qa`, `target/photo-editor-qa`.
- [x] Final native UI verification and `cargo build --locked --bin bevy-editor` passed.

Commands unrelated to image-layer editing must not become extra UI in this editor.
Keep existing painting capture, canvas controls, panel resizing and file rollback intact.

## Layer UX · 2026-11-16

References: PhotoCraft's basic Layers layout, the [Krita Layers docker](https://docs.krita.org/en/reference_manual/dockers/layers.html),
and [Photoshop's Layers panel](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/get-started-layers/work-with-the-layers-panel.html).

- The layer stack and selected layer's properties have separate tabs. The stack grows with the window.
- New layers, duplication, grouping, masks, ordering and deletion have direct controls.
- The default menu offers common tasks. A searchable list of less common commands is secondary.
- Blend modes and parameter choices use Russian labels without changing their native engine values.
- Property dialogs use bounded sliders, step buttons, colour swatches and option buttons. Advanced parameters are collapsed.
- Effects have individual settings, visibility and removal controls. Layer and property controls explain their purpose on hover.
- Appearance dialogs preview native commands from a retained snapshot. Cancel restores pixels, settings and history;
  Apply retains a single undo step. No intermediate previews are written to disk.
- UI verification captures belong in `target/layer-ux-qa`, not in the documentation assets.

User preset libraries are shared between image tabs and stored in
`%APPDATA%/BevyEditor/image-layer-libraries.pcraft` on Windows. The native envelope
preserves pattern pixels together with styles, gradients, shapes and effect defaults.
An unreadable existing library is preserved; edits remain available in the session
and the interface reports that they could not be persisted.
