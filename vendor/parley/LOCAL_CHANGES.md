# Local changes from Parley 0.11.1

The crate is vendored unmodified except `src/editing/editor.rs`:

- `PlainEditor::set_ranged_styles` accepts owned UTF-8 ranges and style properties.
- The existing ranged layout builder receives those properties before IME styling.
- Buffer edits remap ranges and inherit insertion style before layout, keeping native
  cursor positions, hit testing, selection and line wrapping consistent with rich text.
- Editors without ranged styles keep the normal plain-text behaviour.

Upstream: https://github.com/linebender/parley, version 0.11.1.
