//! Hold a tool colour, carry it to a document pixel, and flood-fill with one undo step.
use super::*;
use bevy::{picking::events::PointerPress, time::Real};
use photocraft_doc::{DocId, LayerId};

#[path = "image_color_drop_fx.rs"]
mod fx;
pub(in crate::editor) use fx::{ColorDropPlugin, setup};

const HOLD_SECONDS: f64 = 0.28;
const HOLD_SLOP: f32 = 10.0;

#[derive(Component)]
struct Source(ColorTarget);

struct Carry {
    source: Entity,
    image: usize,
    document: DocId,
    layer: LayerId,
    mask: (bool, bool),
    color: [f32; 4],
    origin: Vec2,
    started: f64,
    lifted: Option<f64>,
}

#[derive(Clone, Copy)]
enum Feedback {
    Land {
        at: Vec2,
        color: [f32; 4],
        started: f64,
    },
    Return {
        from: Vec2,
        to: Vec2,
        color: [f32; 4],
        started: f64,
    },
}

#[derive(Resource, Default)]
pub(in crate::editor) struct ColorDrop {
    carry: Option<Carry>,
    blocked_source: Option<Entity>,
    cursor: Vec2,
    target: Option<IVec2>,
    feedback: Option<Feedback>,
}

impl ColorDrop {
    pub fn suppresses(&self, entity: Entity) -> bool {
        self.blocked_source == Some(entity)
            || self
                .carry
                .as_ref()
                .is_some_and(|carry| carry.source == entity && carry.lifted.is_some())
    }

    fn cancel(&mut self, now: f64, visible: bool) {
        if let Some(carry) = self.carry.take() {
            if carry.lifted.is_some() {
                self.blocked_source = Some(carry.source);
                self.feedback = visible.then_some(Feedback::Return {
                    from: self.cursor + fx::OFFSET,
                    to: carry.origin,
                    color: carry.color,
                    started: now,
                });
            }
        }
        self.target = None;
    }
}

pub(super) fn attach(commands: &mut Commands, entity: Entity, target: ColorTarget) {
    commands
        .entity(entity)
        .insert(Source(target))
        .observe(press);
    layers::ux::help(
        commands,
        entity,
        "Клик — выбрать цвет. Удерживайте и перетащите цвет на изображение, чтобы залить область. Esc — отменить перенос.",
    );
}

fn ready(editor: &Editor, viewer: &ImageViewer) -> bool {
    viewer.edit.editing(editor.viewing_image)
        && editor.pending.is_none()
        && !editor.browser_popup
        && !editor.hdri_library_open
        && !viewer.edit.busy()
        && !viewer.panel.blocks_canvas()
        && viewer.edit.color_picker.is_none()
        && !viewer.edit.layers.menu
        && viewer.edit.layers.form.is_none()
        && !viewer.edit.menu
        && !viewer.edit.paint_menu
        && !viewer.edit.size_menu
        && viewer.edit.selection.transform.is_none()
        && viewer.edit.selection.draft.is_none()
}

fn press(
    event: On<PointerPress>,
    sources: Query<&Source>,
    editor: Res<Editor>,
    mut viewer: ResMut<ImageViewer>,
    mut drop: ResMut<ColorDrop>,
    time: Res<Time<Real>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    if event.button != PointerButton::Primary || !window.focused || !ready(&editor, &viewer) {
        return;
    }
    let Ok(source) = sources.get(event.entity) else {
        return;
    };
    let Some(image) = editor.viewing_image else {
        return;
    };
    let Some(doc) = viewer.edit.document(Some(image)) else {
        return;
    };
    let Some(layer) = doc.active_id() else { return };
    drop.blocked_source = None;
    drop.feedback = None;
    drop.cursor = event.pointer.position;
    drop.carry = Some(Carry {
        source: event.entity,
        image,
        document: doc.document.id,
        layer,
        mask: (doc.mask_target, doc.filter_target),
        color: match source.0 {
            ColorTarget::Background => viewer.edit.background(),
            _ => viewer.edit.color(),
        },
        origin: event.pointer.position,
        started: time.elapsed_secs_f64(),
        lifted: None,
    });
    viewer.edit.color_dragging = true;
}

fn pixel_at(point: Vec2, image: Rect, viewport: Rect, size: UVec2) -> Option<IVec2> {
    if !point.is_finite()
        || !image.size().cmpgt(Vec2::ZERO).all()
        || !viewport.contains(point)
        || !image.contains(point)
    {
        return None;
    }
    let pixel = ((point - image.min) / image.size() * size.as_vec2())
        .floor()
        .as_ivec2();
    (pixel.cmpge(IVec2::ZERO).all() && pixel.cmplt(size.as_ivec2()).all()).then_some(pixel)
}

pub(in crate::editor) fn input(
    mut drop: ResMut<ColorDrop>,
    mut viewer: ResMut<ImageViewer>,
    mut editor: ResMut<Editor>,
    time: Res<Time<Real>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    controls: Query<(&ComputedNode, &UiGlobalTransform), With<CanvasControl>>,
    drawer: Option<Res<DrawerMotion>>,
    mut focus: ResMut<InputFocus>,
) {
    viewer.edit.color_dragging = false;
    if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
        drop.blocked_source = None;
    }
    let now = time.elapsed_secs_f64();
    let Some(carry) = drop.carry.as_ref() else {
        return;
    };
    // A drag owns its original document, layer and mask; changing any cancels it.
    let valid = ready(&editor, &viewer)
        && editor.viewing_image == Some(carry.image)
        && viewer.edit.document(Some(carry.image)).is_some_and(|doc| {
            doc.document.id == carry.document
                && doc.active_id() == Some(carry.layer)
                && (doc.mask_target, doc.filter_target) == carry.mask
        });
    if !valid
        || !window.focused
        || keys.just_pressed(KeyCode::Escape)
        || mouse.just_pressed(MouseButton::Right)
        || (!mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left))
    {
        drop.cancel(now, valid && window.focused);
        // Swallow the release after Escape, so it cannot become a brush stroke.
        viewer.edit.color_dragging = valid && mouse.pressed(MouseButton::Left);
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        drop.target = None;
        if mouse.just_released(MouseButton::Left) {
            drop.cancel(now, false);
        }
        viewer.edit.color_dragging = true;
        return;
    };
    drop.cursor = cursor;
    let carry = drop.carry.as_mut().unwrap();
    if carry.lifted.is_none() {
        if cursor.distance(carry.origin) > HOLD_SLOP {
            drop.cancel(now, false);
            viewer.edit.color_dragging = true;
            return;
        }
        if now - carry.started >= HOLD_SECONDS && mouse.pressed(MouseButton::Left) {
            carry.lifted = Some(now);
            drop.blocked_source = Some(carry.source);
            focus.clear();
        }
    }
    let carry = drop.carry.as_ref().unwrap();
    let doc = viewer.edit.document(Some(carry.image)).unwrap();
    let paintable = doc.paintable();
    let viewport = canvas
        .single()
        .ok()
        .filter(|(n, _)| !n.is_empty())
        .map(|_| canvas_rect(&canvas, &window));
    let target = if carry.lifted.is_some()
        && paintable
        && !over_controls(&controls, cursor)
        && !drawer
            .as_ref()
            .is_some_and(|drawer| drawer.covers(cursor, &window))
    {
        viewport.and_then(|viewport| {
            let size = viewer.dimensions?;
            pixel_at(cursor, viewer.image_rect(size, viewport), viewport, size)
        })
    } else {
        None
    };
    drop.target = target;
    viewer.edit.color_dragging = true;
    if !mouse.just_released(MouseButton::Left) {
        return;
    }
    let carry = drop.carry.take().unwrap();
    if carry.lifted.is_none() {
        return;
    } // A short click is still the colour picker.
    drop.blocked_source = Some(carry.source);
    if let Some(pixel) = target {
        let doc = &mut viewer.edit.sessions.get_mut(&carry.image).unwrap().doc;
        let target = if doc.mask_target && doc.filter_target {
            "filterMask"
        } else if doc.mask_target {
            "mask"
        } else {
            "pixels"
        };
        match doc.layer_command(
            "paint.bucket",
            serde_json::json!({
                "x":pixel.x, "y":pixel.y, "color":carry.color, "tolerance":0,
                "antiAlias":false, "target":target,
            }),
        ) {
            Ok(_) => {
                drop.feedback = Some(Feedback::Land {
                    at: cursor,
                    color: carry.color,
                    started: now,
                });
                viewer.edit.error = None;
                editor.status = "Область залита · Ctrl+Z — отменить".into();
            }
            Err(error) => {
                viewer.edit.error = Some(error);
                drop.feedback = Some(Feedback::Return {
                    from: cursor + fx::OFFSET,
                    to: carry.origin,
                    color: carry.color,
                    started: now,
                });
            }
        }
    } else {
        drop.feedback = Some(Feedback::Return {
            from: cursor + fx::OFFSET,
            to: carry.origin,
            color: carry.color,
            started: now,
        });
        if !paintable {
            editor.status = "Для заливки выберите видимый незаблокированный слой".into();
        }
    }
    drop.target = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_drop_pixels_follow_zoom_pan_and_clip_at_document_and_viewport_edges() {
        let image = Rect::from_corners(Vec2::new(-37.5, 90.0), Vec2::new(262.5, 390.0));
        let viewport = Rect::from_corners(Vec2::new(20.0, 100.0), Vec2::new(240.0, 350.0));
        let size = UVec2::splat(100);
        assert_eq!(
            pixel_at(Vec2::new(21.0, 105.0), image, viewport, size),
            Some(IVec2::new(19, 5))
        );
        for point in [
            Vec2::new(19.0, 105.0),
            Vec2::new(241.0, 150.0),
            Vec2::splat(f32::NAN),
        ] {
            assert_eq!(pixel_at(point, image, viewport, size), None);
        }
        assert_eq!(pixel_at(image.max, image, image, size), None);
    }
}
