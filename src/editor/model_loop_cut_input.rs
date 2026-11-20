use super::*;
use crate::editor::model_mesh::{LoopCut, LoopCutOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum LoopCutControl {
    Count,
    Position,
    Even,
    Flip,
    Clamp,
    CorrectUv,
    Smoothness,
    Falloff,
    Confirm,
}

pub(super) struct Preview {
    adjacency: BTreeMap<Edge, Vec<usize>>,
    hovered: Option<Edge>,
    cut: Option<LoopCut>,
    options: LoopCutOptions,
    placing: bool,
    adjusting: bool,
    mouse_origin: Option<(f32, f32)>,
    wheel: f32,
    input_field: LoopCutControl,
    confirm: bool,
    message: Option<String>,
}

pub(in crate::editor) struct LastLoopCut {
    base: Snapshot,
    seed: Edge,
    options: LoopCutOptions,
    pub revision: u64,
}

impl Gesture {
    pub(in crate::editor) fn loop_cut(doc: &Document, model: usize) -> Option<Self> {
        let object = doc.active().filter(|object| !object.hidden)?;
        let mut gesture = Self::new(
            Tool::Move,
            doc.snapshot(),
            model,
            object.transform.translation,
        );
        gesture.loop_cut = Some(Preview {
            adjacency: object.geometry.edges(),
            hovered: None,
            cut: None,
            options: default(),
            placing: false,
            adjusting: false,
            mouse_origin: None,
            wheel: 0.0,
            input_field: LoopCutControl::Count,
            confirm: false,
            message: None,
        });
        Some(gesture)
    }

    pub(in crate::editor) fn adjust_loop_cut(
        doc: &Document,
        model: usize,
        last: &LastLoopCut,
    ) -> Option<Self> {
        if doc.revision != last.revision || doc.undo.is_empty() {
            return None;
        }
        let mut gesture = Self::loop_cut(doc, model)?;
        gesture.base = last.base.clone();
        let mesh = &base_object(&gesture.base)?.geometry;
        let preview = gesture.loop_cut.as_mut()?;
        preview.adjacency = mesh.edges();
        preview.cut = LoopCut::find_in(mesh, &preview.adjacency, last.seed).ok();
        preview.options = last.options;
        preview.placing = true;
        preview.adjusting = true;
        preview.input_field = LoopCutControl::Position;
        Some(gesture)
    }

    pub(in crate::editor) fn loop_control_available(&self, control: LoopCutControl) -> bool {
        self.loop_cut.as_ref().is_some_and(|preview| match control {
            LoopCutControl::Position
            | LoopCutControl::Even
            | LoopCutControl::Flip
            | LoopCutControl::Clamp => preview.placing && preview.options.count == 1,
            LoopCutControl::Confirm => preview.placing,
            _ => true,
        })
    }

    pub(in crate::editor) fn loop_control_active(&self, control: LoopCutControl) -> bool {
        self.loop_cut.as_ref().is_some_and(|preview| match control {
            LoopCutControl::Even => preview.options.even,
            LoopCutControl::Flip => preview.options.flipped,
            LoopCutControl::Clamp => preview.options.clamp,
            LoopCutControl::CorrectUv => preview.options.correct_uv,
            LoopCutControl::Falloff => preview.options.smooth_falloff,
            _ => preview.input_field == control,
        })
    }

    pub(in crate::editor) fn loop_control_label(&self, control: LoopCutControl) -> Option<String> {
        let options = self.loop_cut.as_ref()?.options;
        Some(match control {
            LoopCutControl::Count => format!("Разрезы · {}", options.count),
            LoopCutControl::Position => format!("Положение · {:.2}%", options.position * 100.0),
            LoopCutControl::Even => "Равное расстояние".into(),
            LoopCutControl::Flip => "Другая сторона".into(),
            LoopCutControl::Clamp => "Ограничить полосой".into(),
            LoopCutControl::CorrectUv => "Корректировать UV".into(),
            LoopCutControl::Smoothness => format!("Сглаживание · {:.2}", options.smoothness),
            LoopCutControl::Falloff => if options.smooth_falloff {
                "Плавный профиль"
            } else {
                "Равномерный профиль"
            }
            .into(),
            LoopCutControl::Confirm => "Применить".into(),
        })
    }

    pub(in crate::editor) fn set_loop_control(&mut self, control: LoopCutControl) {
        if !self.loop_control_available(control) {
            return;
        }
        let preview = self.loop_cut.as_mut().unwrap();
        preview.message = None;
        match control {
            LoopCutControl::Count | LoopCutControl::Position | LoopCutControl::Smoothness => {
                preview.input_field = control;
                preview.mouse_origin = None;
                self.typed.clear();
            }
            LoopCutControl::Even => preview.options.even = !preview.options.even,
            LoopCutControl::Flip => preview.options.flipped = !preview.options.flipped,
            LoopCutControl::Clamp => preview.options.clamp = !preview.options.clamp,
            LoopCutControl::CorrectUv => preview.options.correct_uv = !preview.options.correct_uv,
            LoopCutControl::Falloff => {
                preview.options.smooth_falloff = !preview.options.smooth_falloff
            }
            LoopCutControl::Confirm => preview.confirm = true,
        }
    }
}

fn base_object(base: &Snapshot) -> Option<&Object> {
    base.objects
        .iter()
        .find(|object| Some(object.id) == base.active)
}

pub(super) enum Outcome {
    Preview(String),
    Confirm(LastLoopCut),
    Cancel,
}

pub(super) fn update<'a>(
    gesture: &mut Gesture,
    doc: &mut Document,
    bvh: &Bvh,
    viewer: &Viewer,
    rect: Rect,
    cursor: Vec2,
    canvas_input: bool,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    wheel: f32,
    events: impl Iterator<Item = &'a KeyboardInput>,
) -> Outcome {
    let preview = gesture.loop_cut.as_mut().unwrap();
    if keys.just_pressed(KeyCode::Escape)
        || (!preview.placing && mouse.just_pressed(MouseButton::Right))
    {
        return Outcome::Cancel;
    }
    let Some(object) = base_object(&gesture.base) else {
        return Outcome::Cancel;
    };
    let mesh = &object.geometry;
    if !preview.placing {
        let hovered = canvas_input
            .then(|| {
                preview
                    .adjacency
                    .iter()
                    .filter_map(|(&edge, faces)| {
                        if !faces.iter().any(|&i| mesh.faces[i].corners.len() == 4) {
                            return None;
                        }
                        let a = object.transform.transform_point(mesh.positions[edge.0]);
                        let b = object.transform.transform_point(mesh.positions[edge.1]);
                        let distance =
                            segment(cursor, project(viewer, rect, a)?, project(viewer, rect, b)?);
                        if distance >= 12.0 {
                            return None;
                        }
                        let factor = cursor_factor(object, edge, viewer, rect, cursor)
                            .unwrap_or(0.5)
                            .clamp(0.0, 1.0);
                        visible(bvh, viewer, rect, a.lerp(b, factor)).then_some((distance, edge))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, edge)| edge)
            })
            .flatten();
        if hovered != preview.hovered {
            preview.hovered = hovered;
            preview.message = None;
            preview.cut =
                hovered.and_then(
                    |edge| match LoopCut::find_in(mesh, &preview.adjacency, edge) {
                        Ok(cut) => Some(cut),
                        Err(error) => {
                            preview.message = Some(error);
                            None
                        }
                    },
                );
        }
    }

    if !preview.placing || preview.adjusting || preview.input_field == LoopCutControl::Count {
        preview.wheel += wheel;
        let steps = preview.wheel.trunc() as isize
            + isize::from(keys.just_pressed(KeyCode::PageUp))
            - isize::from(keys.just_pressed(KeyCode::PageDown));
        preview.wheel -= preview.wheel.trunc();
        if steps != 0 {
            preview.options.count = preview
                .options
                .count
                .saturating_add_signed(steps)
                .clamp(1, 100);
            gesture.typed.clear();
            preview.message = None;
        }
    }
    if preview.placing {
        if keys.just_pressed(KeyCode::KeyE) {
            preview.options.even = !preview.options.even;
        }
        if keys.just_pressed(KeyCode::KeyF) {
            preview.options.flipped = !preview.options.flipped;
        }
        if keys.just_pressed(KeyCode::KeyC) {
            preview.options.clamp = !preview.options.clamp;
        }
        if keys.just_pressed(KeyCode::KeyU) {
            preview.options.correct_uv = !preview.options.correct_uv;
        }
    }
    if keys.just_pressed(KeyCode::KeyS) {
        preview.input_field = if preview.input_field == LoopCutControl::Smoothness {
            if preview.placing {
                LoopCutControl::Position
            } else {
                LoopCutControl::Count
            }
        } else {
            LoopCutControl::Smoothness
        };
        preview.mouse_origin = None;
        gesture.typed.clear();
    }
    if keys.just_pressed(KeyCode::KeyP) {
        preview.options.smooth_falloff = !preview.options.smooth_falloff;
    }
    for event in events.filter(|event| event.state == ButtonState::Pressed) {
        match &event.logical_key {
            Key::Character(value) => {
                for character in value
                    .chars()
                    .filter(|c| c.is_ascii_digit() || ['.', ',', '-'].contains(c))
                {
                    if gesture.typed.len() < 12 {
                        gesture
                            .typed
                            .push(if character == ',' { '.' } else { character });
                    }
                }
            }
            Key::Backspace => {
                gesture.typed.pop();
            }
            _ => {}
        }
    }
    if let Ok(number) = gesture.typed.parse::<f32>() {
        preview.message = None;
        if preview.input_field == LoopCutControl::Smoothness {
            preview.options.smoothness = number.clamp(-1.0, 1.0);
        } else if preview.input_field == LoopCutControl::Count {
            preview.options.count = (number.round() as usize).clamp(1, 100);
        } else {
            preview.options.position = (number / 100.0).clamp(-10.0, 10.0);
        }
    } else if preview.placing
        && preview.input_field == LoopCutControl::Position
        && canvas_input
        && gesture.typed.is_empty()
        && let Some(cut) = &preview.cut
        && let Some(factor) = cursor_factor(object, cut.seed(), viewer, rect, cursor)
    {
        let (origin, position) = *preview
            .mouse_origin
            .get_or_insert((factor, preview.options.position));
        let position = (position + factor - origin).clamp(-10.0, 10.0);
        if position != preview.options.position {
            preview.message = None;
            preview.options.position = position;
        }
    }

    if preview.options.clamp {
        preview.options.position = preview.options.position.clamp(0.001, 0.999);
    }
    let click = canvas_input && mouse.just_pressed(MouseButton::Left);
    let enter = keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
    let centered = preview.placing && canvas_input && mouse.just_pressed(MouseButton::Right);
    if !preview.placing && (click || enter) && preview.cut.is_some() {
        preview.placing = true;
        preview.input_field = LoopCutControl::Position;
        gesture.typed.clear();
    } else if preview.placing
        && (click || enter || centered || std::mem::take(&mut preview.confirm))
        && let Some(cut) = &preview.cut
    {
        let mut options = preview.options;
        if centered {
            options.position = 0.5;
            options.even = false;
        }
        let mut snapshot = gesture.base.clone();
        let object = snapshot
            .objects
            .iter_mut()
            .find(|object| Some(object.id) == snapshot.active)
            .unwrap();
        match cut.apply(&mut object.geometry, options) {
            Ok(edges) => {
                if snapshot
                    .objects
                    .iter()
                    .map(|object| object.geometry.positions.len())
                    .sum::<usize>()
                    > 250_000
                {
                    preview.message = Some("Разрез превысит лимит модели в 250 000 вершин".into());
                } else {
                    snapshot.selection.clear();
                    snapshot.selection.kind = Element::Edge;
                    snapshot.selection.edges = edges;
                    doc.restore(snapshot);
                    if !preview.adjusting {
                        doc.checkpoint(gesture.base.clone());
                    }
                    return Outcome::Confirm(LastLoopCut {
                        base: gesture.base.clone(),
                        seed: cut.seed(),
                        options,
                        revision: doc.revision,
                    });
                }
            }
            Err(error) => preview.message = Some(error),
        }
    }
    let status = preview.message.clone().unwrap_or_else(|| {
        if preview
            .cut
            .as_ref()
            .is_some_and(|cut| !cut.fits_vertex_limit(mesh, preview.options))
        {
            return "Разрез превысит лимит модели в 250 000 вершин".into();
        }
        if preview.placing {
            format!(
                "Разрез петли · {} · {:.2}% · UV {} · Равноудалённость {} · Сглаживание {:.2}",
                preview.options.count,
                preview.options.position * 100.0,
                if preview.options.correct_uv {
                    "вкл"
                } else {
                    "выкл"
                },
                if preview.options.even {
                    if preview.options.flipped {
                        "сторона 2"
                    } else {
                        "сторона 1"
                    }
                } else {
                    "выкл"
                },
                preview.options.smoothness
            )
        } else {
            format!(
                "Разрез петли · {} · Наведите курсор на ребро",
                preview.options.count
            )
        }
    });
    Outcome::Preview(status)
}

fn cursor_factor(
    object: &Object,
    edge: Edge,
    viewer: &Viewer,
    rect: Rect,
    cursor: Vec2,
) -> Option<f32> {
    let world = |i| {
        object
            .transform
            .transform_point(object.geometry.positions[i])
    };
    let (wa, wb) = (world(edge.0), world(edge.1));
    let a = project(viewer, rect, wa)?;
    let delta = project(viewer, rect, wb)? - a;
    let length_squared = delta.length_squared();
    if length_squared <= 1.0 {
        return None;
    }
    let t = (cursor - a).dot(delta) / length_squared;
    if viewer.orthographic {
        return Some(t);
    }
    let pose = view_pose(viewer);
    let depth_a = (wa - pose.translation).dot(*pose.forward());
    let depth_b = (wb - pose.translation).dot(*pose.forward());
    let denominator = (1.0 - t) / depth_a + t / depth_b;
    (denominator > 1.0e-8).then(|| (t / depth_b) / denominator)
}

pub(super) fn draw(gesture: &Gesture, gizmos: &mut Gizmos<ModelGizmos>) {
    let Some(preview) = &gesture.loop_cut else {
        return;
    };
    let Some(cut) = &preview.cut else { return };
    let Some(object) = base_object(&gesture.base) else {
        return;
    };
    for [a, b] in cut.preview(&object.geometry, preview.options) {
        gizmos.line(
            object.transform.transform_point(a),
            object.transform.transform_point(b),
            Color::srgb(1.0, 0.77, 0.25),
        );
    }
}
