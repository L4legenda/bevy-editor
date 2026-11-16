//! Shift + horizontal pointer motion adjusts the tip without painting or panning.
use super::*;

struct Drag {
    tab: usize,
    tool: PhotoTool,
    pixel: bool,
    origin: Vec2,
    center: Vec2,
    original: f32,
    scale: f32,
}

#[derive(Default)]
pub(in crate::editor) struct Adjustment {
    drag: Option<Drag>,
    // Ignore the remainder of a press made during adjustment, including its release sample.
    suppress_click: bool,
}

impl Adjustment {
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }
    pub fn center(&self) -> Option<Vec2> {
        self.drag.as_ref().map(|drag| drag.center)
    }
    fn blocks_canvas(&self) -> bool {
        self.active() || self.suppress_click
    }
}

fn resized(original: f32, dx: f32, scale: f32, pixel: bool) -> f32 {
    let size = (original + 2.0 * dx / scale.max(0.001)).clamp(1.0, 256.0);
    if pixel { size.round() } else { size }
}

pub(in crate::editor) fn pointer(
    viewer: &mut ImageViewer,
    editor: &Editor,
    rect: Rect,
    input_rect: Rect,
    window: &mut Window,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    text_focused: bool,
) -> bool {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let pressed = keys.any_just_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let modifiers = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
        KeyCode::Space,
        KeyCode::KeyB,
    ]);
    let allowed = viewer.edit.editing(editor.viewing_image)
        && viewer.edit.sizeable()
        && !viewer.edit.busy()
        && !viewer.edit.menu
        && !viewer.edit.paint_menu
        && !viewer.edit.size_menu
        && viewer.edit.color_picker.is_none()
        && !viewer.edit.layers.menu
        && viewer.edit.layers.form.is_none()
        && !viewer.panel.blocks_canvas()
        && viewer.error.is_none()
        && editor.pending.is_none()
        && !editor.browser_popup
        && !editor.hdri_library_open
        && window.focused
        && !text_focused
        && !modifiers
        && !mouse.any_pressed([MouseButton::Middle, MouseButton::Right]);
    if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
        viewer.edit.size_adjust.suppress_click = false;
    }
    if !allowed {
        viewer.edit.size_adjust.drag = None;
    }
    if viewer.edit.size_adjust.drag.as_ref().is_some_and(|drag| {
        Some(drag.tab) != editor.viewing_image
            || drag.tool != viewer.edit.tool
            || drag.pixel != viewer.edit.pixel_tool()
    }) {
        viewer.edit.size_adjust.drag = None;
    }
    let cancel = keys.just_pressed(KeyCode::Escape);
    if (cancel || !shift)
        && let Some(drag) = viewer.edit.size_adjust.drag.take()
    {
        if cancel {
            viewer.edit.set_size(drag.original);
        }
        // The native pointer was hidden while the preview grew around its
        // fixed centre. Return to the original logical position, including at
        // high DPI, so finishing the adjustment does not move the brush away.
        window.set_cursor_position(Some(drag.origin));
        viewer.edit.size_adjust.suppress_click |= mouse.pressed(MouseButton::Left);
        viewer.gesture = None;
        viewer.last_click = None;
        return true;
    }
    if !cancel
        && allowed
        && pressed
        && let Some(cursor) = window.cursor_position().filter(|p| input_rect.contains(*p))
        && let Some(dimensions) = viewer.dimensions
        && let Some(tab) = editor.viewing_image
    {
        let image = viewer.image_rect(dimensions, rect);
        let per_pixel = image.size() / dimensions.as_vec2();
        let scale = per_pixel.x;
        let center = if viewer.edit.pixel_tool() {
            image.min + viewer.edit.snap_point((cursor - image.min) / per_pixel) * per_pixel
        } else {
            cursor
        };
        viewer.edit.size_adjust.drag = Some(Drag {
            tab,
            tool: viewer.edit.tool,
            pixel: viewer.edit.pixel_tool(),
            origin: cursor,
            center,
            original: viewer.edit.size,
            scale,
        });
        // Finish an existing stroke before changing its settings, preserving its final tail.
        super::flush(&mut viewer.edit, tab, &[], None, true);
        viewer.edit.gesture = None;
    }
    if let Some(drag) = &viewer.edit.size_adjust.drag
        && let Some(cursor) = window.cursor_position()
    {
        let size = resized(
            drag.original,
            cursor.x - drag.origin.x,
            drag.scale,
            drag.pixel,
        );
        viewer.edit.set_size(size);
        viewer.edit.size_adjust.suppress_click |= mouse.pressed(MouseButton::Left);
    }
    if viewer.edit.size_adjust.blocks_canvas() {
        viewer.gesture = None;
        viewer.last_click = None;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brush_size_motion_tracks_screen_radius_at_every_zoom_and_snaps_pixels() {
        assert_eq!(resized(24.0, 10.0, 1.0, false), 44.0);
        assert_eq!(resized(24.0, 10.0, 2.0, false), 34.0);
        assert_eq!(resized(24.0, 10.0, 0.5, false), 64.0);
        assert_eq!(resized(1.0, 3.2, 8.0, true), 2.0);
        assert_eq!(resized(24.0, -1000.0, 1.0, false), 1.0);
        assert_eq!(resized(24.0, 1000.0, 1.0, false), 256.0);
        assert_eq!(resized(24.0, 0.0, 1.0, false), 24.0);
    }
}
