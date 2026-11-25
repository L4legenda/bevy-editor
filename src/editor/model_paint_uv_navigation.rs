//! Cursor-anchored navigation for the paint workspace's clipped UV image.
use super::*;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};

#[derive(Clone, Copy)]
pub(in crate::editor::ui::model_viewer::paint) struct UvView {
    zoom: f32,
    // Offset in units of the fitted image side, so docking does not change framing.
    pan: Vec2,
}

impl Default for UvView {
    fn default() -> Self {
        Self {
            zoom: 1.,
            pan: Vec2::ZERO,
        }
    }
}

impl UvView {
    fn zoom_at(&mut self, anchor: Vec2, factor: f32) {
        let previous = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.2, 32.);
        self.pan += (anchor - self.pan) * (1. - self.zoom / previous);
    }
}

pub(in crate::editor::ui::model_viewer) struct Drag {
    model: usize,
    button: Option<MouseButton>,
    start: Vec2,
    previous: Vec2,
    moved: bool,
}

pub(in crate::editor::ui::model_viewer) fn navigate(
    mut paint: ResMut<PaintState>,
    mut runtime: ResMut<Runtime>,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    window: Single<&Window, With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    drawer: Res<DrawerMotion>,
    focus: Res<InputFocus>,
    fields: Query<(), With<TextInput>>,
    texture: UvCanvas,
    mut image: Query<&mut Node, With<TextureCanvas>>,
    mut actions: MessageReader<Action>,
    mut drag: Local<Option<Drag>>,
    pointer: Res<PointerFrame>,
) {
    runtime.uv_panning = false;
    let mut fit = false;
    for action in actions.read() {
        fit |= matches!(action, Action::ModelView(ModelViewAction::Fit));
    }
    let Some(index) = editor
        .viewing_model
        .filter(|&index| editing.mode(Some(index)) == Mode::Paint)
    else {
        *drag = None;
        return;
    };
    let Some(session) = paint.sessions.get_mut(&index) else {
        return;
    };
    if fit {
        session.uv_view = UvView::default();
        *drag = None;
    }
    if session.view == View::Three || session.panel == Panel::Shader || session.graph_canvas_open()
    {
        *drag = None;
        return;
    }
    let (Some(rect), Ok(mut node)) = (texture.viewport_rect(), image.single_mut()) else {
        *drag = None;
        return;
    };
    let size = (rect.size().min_element() - 28.).max(1.);
    let blocked = editor.viewing_image.is_some()
        || editor.pending.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
        || !window.focused
        || drawer.resizing
        || focus.get().is_some_and(|entity| fields.contains(entity))
        || runtime.stroke_active()
        || keys.just_pressed(KeyCode::Escape)
        || keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let pen = pointer.pen_pan;
    if blocked
        || drag.as_ref().is_some_and(|drag| {
            drag.model != index
                || match drag.button {
                    None => pen.is_none_or(|pan| pan.start.is_some()),
                    Some(button) => pen.is_some() || !mouse.pressed(button)
                        || (button == MouseButton::Left && !alt),
                }
        })
    {
        *drag = None;
    }
    if !blocked && let Some(point) = pen.map(|pan| pan.point).or_else(|| window.cursor_position()) {
        let over = rect.contains(point) && !drawer.covers(point, &window);
        if let Some(start) = pen.and_then(|pan| pan.start)
            .filter(|point| rect.contains(*point) && !drawer.covers(*point, &window))
        {
            *drag = Some(Drag {
                model: index,
                button: None,
                start,
                previous: start,
                moved: true,
            });
        } else if pen.is_none() && drag.is_none() && over {
            let button = [MouseButton::Middle, MouseButton::Right, MouseButton::Left]
                .into_iter()
                .find(|&button| mouse.just_pressed(button) && (button != MouseButton::Left || alt));
            if let Some(button) = button {
                *drag = Some(Drag {
                    model: index,
                    button: Some(button),
                    start: point,
                    previous: point,
                    moved: false,
                });
            }
        }
        if let Some(drag) = drag.as_mut() {
            drag.moved |= drag.button != Some(MouseButton::Right)
                || crate::editor::area_selection::dragged(drag.start, point);
            if drag.moved {
                session.uv_view.pan += (point - drag.previous) / size;
                drag.previous = point;
                runtime.uv_panning = true;
            }
        } else if pen.is_none() && over
            && !mouse.any_pressed([MouseButton::Left, MouseButton::Middle, MouseButton::Right])
        {
            let rate = if scroll.unit == MouseScrollUnit::Line {
                0.15
            } else {
                0.002
            };
            if scroll.delta.y != 0. {
                session.uv_view.zoom_at(
                    (point - rect.center()) / size,
                    (scroll.delta.y * rate).exp(),
                );
            }
        }
    }
    let side = size * session.uv_view.zoom;
    let pos = rect.size() * 0.5 + session.uv_view.pan * size - Vec2::splat(side * 0.5);
    let layout = (px(pos.x), px(pos.y), px(side));
    if (node.left, node.top, node.width, node.height) != (layout.0, layout.1, layout.2, layout.2) {
        (node.left, node.top, node.width, node.height) = (layout.0, layout.1, layout.2, layout.2);
    }
}
