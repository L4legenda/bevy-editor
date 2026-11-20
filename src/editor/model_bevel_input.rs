//! Interactive mesh bevel. All previews start from the same undo snapshot.
use super::*;
use crate::editor::model_mesh::{BevelOptions, Intersection, WidthType};

pub(in crate::editor) struct LastBevel {
    base: Snapshot,
    after: Vec<Object>,
    vertices: bool,
    options: BevelOptions,
    pub revision: u64,
}

impl LastBevel {
    pub fn matches(&self, doc: &Document) -> bool {
        self.after == doc.objects
    }
}

pub(super) struct Preview {
    pub previous: Option<Snapshot>,
    vertices: bool,
    options: BevelOptions,
    parameter: Parameter,
    mouse_value: f32,
    wheel: f32,
    previous_cursor: Option<Vec2>,
    motion: Vec2,
    attempted: (bool, BevelOptions),
    error: Option<String>,
    confirm_requested: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parameter {
    Amount,
    Segments,
    Profile,
    Spread,
    CustomCoordinate { point: usize, axis: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum BevelControl {
    Amount,
    Segments,
    Profile,
    CustomProfile,
    CustomCoordinate { point: usize, axis: usize },
    WidthType,
    Affect,
    Clamp,
    LoopSlide,
    OuterMiter,
    InnerMiter,
    Intersection,
    Spread,
    Harden,
    PreserveSeams,
    PreserveSharp,
    Material,
    Confirm,
}

impl Parameter {
    fn value(self, options: BevelOptions) -> f32 {
        match self {
            Self::Amount => options.amount,
            Self::Segments => options.segments as f32,
            Self::Profile => options.profile,
            Self::Spread => options.spread,
            Self::CustomCoordinate { point, axis } => options.custom_profile.unwrap()[point][axis],
        }
    }

    fn bounds(self, width_type: WidthType, options: BevelOptions) -> (f32, f32) {
        match self {
            Self::Amount if width_type == WidthType::Percent => (0.0, 100.0),
            Self::Amount => (0.0, 1.0e6),
            Self::Segments => (1.0, 100.0),
            Self::Profile => (0.0, 1.0),
            Self::Spread => (0.0, 1.0e6),
            Self::CustomCoordinate { point, axis } => {
                let points = options.custom_profile.unwrap();
                let previous = point.checked_sub(1).map_or(Vec2::X, |index| points[index]);
                let next = points.get(point + 1).copied().unwrap_or(Vec2::Y);
                if axis == 0 {
                    (
                        (next.x + 0.001).min(points[point].x),
                        (previous.x - 0.001).max(points[point].x),
                    )
                } else {
                    (previous.y, next.y)
                }
            }
        }
    }
}

impl Preview {
    fn new(vertices: bool, options: BevelOptions, previous: Option<Snapshot>) -> Self {
        Self {
            previous,
            vertices,
            options,
            parameter: Parameter::Amount,
            mouse_value: options.amount,
            wheel: 0.0,
            previous_cursor: None,
            motion: Vec2::ZERO,
            attempted: (vertices, options),
            error: None,
            confirm_requested: false,
        }
    }
}

impl Gesture {
    pub(in crate::editor) fn bevel(doc: &Document, model: usize, vertices: bool) -> Option<Self> {
        let mut gesture = Self::new(Tool::Move, doc.snapshot(), model, doc.selection_center()?);
        gesture.bevel = Some(Preview::new(vertices, BevelOptions::default(), None));
        Some(gesture)
    }

    pub(in crate::editor) fn adjust_bevel(
        doc: &Document,
        model: usize,
        last: &LastBevel,
    ) -> Option<Self> {
        if last.revision != doc.revision || !last.matches(doc) || doc.undo.is_empty() {
            return None;
        }
        let pivot = doc.selection_center()?;
        let mut gesture = Self::new(Tool::Move, last.base.clone(), model, pivot);
        gesture.bevel = Some(Preview::new(
            last.vertices,
            last.options,
            Some(doc.snapshot()),
        ));
        Some(gesture)
    }

    pub(in crate::editor) fn bevel_control_available(&self, control: BevelControl) -> bool {
        self.bevel.as_ref().is_some_and(|preview| {
            if matches!(control, BevelControl::CustomCoordinate { .. }) {
                return preview.options.custom_profile.is_some();
            }
            !preview.vertices
                || !matches!(
                    control,
                    BevelControl::LoopSlide
                        | BevelControl::OuterMiter
                        | BevelControl::InnerMiter
                        | BevelControl::Intersection
                        | BevelControl::Spread
                )
        })
    }

    pub(in crate::editor) fn bevel_control_active(&self, control: BevelControl) -> bool {
        self.bevel.as_ref().is_some_and(|preview| match control {
            BevelControl::Amount => preview.parameter == Parameter::Amount,
            BevelControl::Segments => preview.parameter == Parameter::Segments,
            BevelControl::Profile => preview.parameter == Parameter::Profile,
            BevelControl::Spread => preview.parameter == Parameter::Spread,
            BevelControl::CustomProfile => preview.options.custom_profile.is_some(),
            BevelControl::CustomCoordinate { point, axis } => {
                preview.parameter == Parameter::CustomCoordinate { point, axis }
            }
            BevelControl::Clamp => preview.options.clamp_overlap,
            BevelControl::LoopSlide => preview.options.loop_slide,
            BevelControl::Harden => preview.options.harden_normals,
            BevelControl::PreserveSeams => preview.options.preserve_uv_seams,
            BevelControl::PreserveSharp => preview.options.preserve_sharp,
            _ => false,
        })
    }

    pub(in crate::editor) fn bevel_control_label(&self, control: BevelControl) -> Option<String> {
        let preview = self.bevel.as_ref()?;
        let options = preview.options;
        Some(match control {
            BevelControl::Amount => format!("Размер · {:.4}", options.amount),
            BevelControl::Segments => format!("Сегменты · {}", options.segments),
            BevelControl::Profile => format!("Профиль · {:.2}", options.profile),
            BevelControl::CustomProfile => "Пользовательский профиль".into(),
            BevelControl::CustomCoordinate { point, axis } => {
                let label = if axis == 0 { "X" } else { "Y" };
                let value = options.custom_profile?[point][axis];
                format!("{} · {label} {value:.3}", point + 1)
            }
            BevelControl::WidthType => options.width_type.label().into(),
            BevelControl::Affect => if preview.vertices {
                "Вершины"
            } else {
                "Рёбра"
            }
            .into(),
            BevelControl::Material => options.material.map_or_else(
                || "Материал соседей".into(),
                |slot| format!("Материал · {}", slot + 1),
            ),
            BevelControl::Clamp => "Ограничить пересечения".into(),
            BevelControl::LoopSlide => "Скольжение вдоль рёбер".into(),
            BevelControl::OuterMiter => format!("Внешний стык · {}", options.outer_miter.label()),
            BevelControl::InnerMiter => {
                format!("Внутренний стык · {}", options.inner_miter.label())
            }
            BevelControl::Intersection => match options.intersection {
                Intersection::Grid => "Пересечения · сетка",
                Intersection::Cutoff => "Пересечения · отсечение",
            }
            .into(),
            BevelControl::Spread => format!("Разнос стыка · {:.4}", options.spread),
            BevelControl::Harden => "Сохранить плоские грани".into(),
            BevelControl::PreserveSeams => "Перенести UV-швы".into(),
            BevelControl::PreserveSharp => "Перенести резкие рёбра".into(),
            BevelControl::Confirm => "Применить".into(),
        })
    }

    pub(in crate::editor) fn set_bevel_control(&mut self, control: BevelControl, slots: usize) {
        if !self.bevel_control_available(control) {
            return;
        }
        let preview = self.bevel.as_mut().unwrap();
        match control {
            BevelControl::Amount
            | BevelControl::Segments
            | BevelControl::Profile
            | BevelControl::Spread
            | BevelControl::CustomCoordinate { .. } => {
                preview.parameter = match control {
                    BevelControl::Segments => Parameter::Segments,
                    BevelControl::Profile => Parameter::Profile,
                    BevelControl::Spread => Parameter::Spread,
                    BevelControl::CustomCoordinate { point, axis } => {
                        Parameter::CustomCoordinate { point, axis }
                    }
                    _ => Parameter::Amount,
                };
                preview.mouse_value = preview.parameter.value(preview.options);
                preview.previous_cursor = None;
                self.typed.clear();
            }
            BevelControl::CustomProfile => {
                if preview.options.custom_profile.take().is_none() {
                    preview.options.custom_profile = Some([
                        Vec2::new(0.75, 0.032),
                        Vec2::new(0.5, 0.134),
                        Vec2::new(0.25, 0.339),
                    ]);
                }
                preview.parameter = Parameter::Amount;
                preview.mouse_value = preview.options.amount;
                preview.previous_cursor = None;
                self.typed.clear();
            }
            BevelControl::WidthType => {
                preview.options.width_type = preview.options.width_type.next()
            }
            BevelControl::Affect => {
                preview.vertices = !preview.vertices;
                if preview.vertices {
                    preview.parameter = Parameter::Amount;
                    preview.mouse_value = preview.options.amount;
                    self.typed.clear();
                }
            }
            BevelControl::Clamp => preview.options.clamp_overlap = !preview.options.clamp_overlap,
            BevelControl::LoopSlide => preview.options.loop_slide = !preview.options.loop_slide,
            BevelControl::OuterMiter => {
                preview.options.outer_miter = preview.options.outer_miter.next_outer();
            }
            BevelControl::InnerMiter => {
                preview.options.inner_miter = preview.options.inner_miter.next_inner();
            }
            BevelControl::Intersection => {
                preview.options.intersection = preview.options.intersection.next();
            }
            BevelControl::Harden => {
                preview.options.harden_normals = !preview.options.harden_normals
            }
            BevelControl::PreserveSeams => {
                preview.options.preserve_uv_seams = !preview.options.preserve_uv_seams
            }
            BevelControl::PreserveSharp => {
                preview.options.preserve_sharp = !preview.options.preserve_sharp;
            }
            BevelControl::Material => {
                let next = preview.options.material.map_or(0, |slot| slot + 1);
                preview.options.material = (next < slots).then_some(next);
            }
            BevelControl::Confirm => preview.confirm_requested = true,
        }
    }
}

pub(super) enum Outcome {
    Preview(String),
    Confirm(LastBevel),
    Noop { removed_last: bool },
    Cancel,
}

pub(super) fn update<'a>(
    gesture: &mut Gesture,
    doc: &mut Document,
    viewer: &Viewer,
    rect: Rect,
    cursor: Vec2,
    keys: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
    wheel: f32,
    events: impl Iterator<Item = &'a KeyboardInput>,
    canvas_input: bool,
) -> Outcome {
    if keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Right) {
        return Outcome::Cancel;
    }
    let preview = gesture
        .bevel
        .as_mut()
        .expect("bevel update requires an active bevel");
    let first = preview.previous_cursor.is_none();
    let parameter = if keys.just_pressed(KeyCode::KeyA) {
        Some(Parameter::Amount)
    } else if keys.just_pressed(KeyCode::KeyS) {
        Some(Parameter::Segments)
    } else if keys.just_pressed(KeyCode::KeyP) {
        Some(Parameter::Profile)
    } else if !preview.vertices && keys.just_pressed(KeyCode::KeyD) {
        Some(Parameter::Spread)
    } else {
        None
    };
    if let Some(parameter) = parameter {
        gesture.typed.clear();
        preview.parameter = parameter;
        preview.mouse_value = parameter.value(preview.options);
    }
    if keys.just_pressed(KeyCode::KeyM) {
        preview.options.width_type = preview.options.width_type.next();
    }
    if keys.just_pressed(KeyCode::KeyC) {
        preview.options.clamp_overlap = !preview.options.clamp_overlap;
    }
    if !preview.vertices && keys.just_pressed(KeyCode::KeyL) {
        preview.options.loop_slide = !preview.options.loop_slide;
    }
    if !preview.vertices && keys.just_pressed(KeyCode::KeyO) {
        preview.options.outer_miter = preview.options.outer_miter.next_outer();
    }
    if !preview.vertices && keys.just_pressed(KeyCode::KeyI) {
        preview.options.inner_miter = preview.options.inner_miter.next_inner();
    }
    if !preview.vertices && keys.just_pressed(KeyCode::KeyN) {
        preview.options.intersection = preview.options.intersection.next();
    }
    if keys.just_pressed(KeyCode::KeyH) {
        preview.options.harden_normals = !preview.options.harden_normals;
    }
    if keys.just_pressed(KeyCode::KeyU) {
        preview.options.preserve_uv_seams = !preview.options.preserve_uv_seams;
    }
    if keys.just_pressed(KeyCode::KeyK) {
        preview.options.preserve_sharp = !preview.options.preserve_sharp;
    }
    if keys.just_pressed(KeyCode::KeyV) {
        preview.vertices = !preview.vertices;
        if preview.vertices {
            preview.parameter = Parameter::Amount;
            preview.mouse_value = preview.options.amount;
            gesture.typed.clear();
        }
    }
    if canvas_input {
        preview.wheel += wheel;
        let steps = preview.wheel.trunc() as isize;
        if steps != 0 {
            preview.wheel -= steps as f32;
            preview.options.segments = preview
                .options
                .segments
                .saturating_add_signed(steps)
                .clamp(1, 100);
            if preview.parameter == Parameter::Segments {
                preview.mouse_value = preview.options.segments as f32;
                gesture.typed.clear();
            }
        }
    }
    let width_type = preview.options.width_type;
    let (min, max) = preview.parameter.bounds(width_type, preview.options);
    if !canvas_input {
        preview.previous_cursor = None;
    } else if let Some(previous) = preview.previous_cursor.replace(cursor) {
        let pose = view_pose(viewer);
        let depth = (gesture.pivot - pose.translation).dot(*pose.forward());
        let scale = doc
            .active()
            .map_or(1.0, |o| o.transform.scale.abs().max_element());
        let width_per_pixel = view_height(viewer, depth) / rect.height() / scale.max(0.000001);
        let unit = match preview.parameter {
            Parameter::Amount if width_type == WidthType::Percent => 100.0 / rect.width().max(1.0),
            Parameter::Amount | Parameter::Spread => width_per_pixel,
            Parameter::Segments => 1.0 / 16.0,
            Parameter::Profile | Parameter::CustomCoordinate { .. } => 0.005,
        };
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let delta = (cursor - previous).dot(preview.motion) * unit;
        let value = preview.mouse_value + delta * if shift { 0.1 } else { 1.0 };
        preview.mouse_value = value.clamp(min, max);
    } else {
        preview.motion = input_direction(viewer, rect, cursor, gesture.pivot);
    }

    for event in events.filter(|event| event.state == ButtonState::Pressed) {
        match &event.logical_key {
            Key::Character(value) if !matches!(event.key_code, KeyCode::KeyB) => {
                for c in value
                    .chars()
                    .filter(|c| c.is_ascii_digit() || ['.', ','].contains(c))
                {
                    if gesture.typed.len() < 24 {
                        gesture.typed.push(if c == ',' { '.' } else { c });
                    }
                }
            }
            Key::Backspace => {
                gesture.typed.pop();
            }
            _ => {}
        }
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let value = if gesture.typed.is_empty() {
        let step = match preview.parameter {
            Parameter::Amount if width_type == WidthType::Percent => 5.0,
            Parameter::Amount | Parameter::Spread => doc.amount.max(0.000001),
            Parameter::Segments => 1.0,
            Parameter::Profile | Parameter::CustomCoordinate { .. } => 0.05,
        };
        Some(if ctrl {
            ((preview.mouse_value / step).round() * step).clamp(min, max)
        } else {
            preview.mouse_value.clamp(min, max)
        })
    } else {
        gesture
            .typed
            .parse::<f32>()
            .ok()
            .filter(|value| (min..=max).contains(value))
    };
    if let Some(value) = value {
        match preview.parameter {
            Parameter::Amount => preview.options.amount = value,
            Parameter::Segments => preview.options.segments = value.round() as usize,
            Parameter::Profile => preview.options.profile = value,
            Parameter::Spread => preview.options.spread = value,
            Parameter::CustomCoordinate { point, axis } => {
                preview.options.custom_profile.as_mut().unwrap()[point][axis] = value;
            }
        }
        let current = (preview.vertices, preview.options);
        if preview.attempted != current {
            preview.attempted = current;
            preview.error = doc
                .preview_bevel(&gesture.base, preview.vertices, preview.options)
                .err();
        }
        let confirm = std::mem::take(&mut preview.confirm_requested)
            || keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter])
            || (!first && canvas_input && mouse.just_pressed(MouseButton::Left));
        if confirm && preview.error.is_none() {
            if preview.options.amount == 0.0 {
                let removed_last = preview.previous.is_some();
                if removed_last {
                    doc.undo.pop();
                }
                return Outcome::Noop { removed_last };
            }
            if preview.previous.is_none() {
                doc.checkpoint(gesture.base.clone());
            }
            return Outcome::Confirm(LastBevel {
                base: gesture.base.clone(),
                after: doc.objects.clone(),
                vertices: preview.vertices,
                options: preview.options,
                revision: doc.revision,
            });
        }
    }
    let label = if preview.vertices {
        "Фаска вершин"
    } else {
        "Фаска рёбер"
    };
    let detail = match value {
        None => "Введите корректное значение".into(),
        Some(_) => preview.error.clone().unwrap_or_else(|| {
            let amount = preview.options.amount;
            let segments = preview.options.segments;
            let profile = if preview.options.custom_profile.is_some() {
                "свой".into()
            } else {
                format!("{:.2}", preview.options.profile)
            };
            let suffix = if width_type == WidthType::Percent {
                "%"
            } else {
                ""
            };
            format!(
                "{} {amount:.4}{suffix} · Сегменты {segments} · Профиль {profile}",
                width_type.label()
            )
        }),
    };
    Outcome::Preview(format!("{label} · {detail}"))
}

fn input_direction(viewer: &Viewer, rect: Rect, cursor: Vec2, pivot: Vec3) -> Vec2 {
    let delta = cursor - project(viewer, rect, pivot).unwrap_or(rect.center());
    if delta.length() < 8.0 {
        Vec2::new(1.0, -1.0).normalize()
    } else {
        delta.normalize()
    }
}

pub(super) fn draw_guide(
    gesture: &Gesture,
    viewer: &Viewer,
    rect: Rect,
    overlay: &mut Gizmos<XrayGizmos>,
) {
    let Some(cursor) = gesture.bevel.as_ref().and_then(|preview| preview.previous_cursor) else {
        return;
    };
    let Some(anchor) = project(viewer, rect, gesture.pivot) else {
        return;
    };
    let delta = anchor - cursor;
    let length = delta.length();
    if !rect.contains(cursor) || !length.is_finite() || length <= 4.0 {
        return;
    }

    // Space the dashes in screen pixels, on the same overlay as the model handles.
    let direction = delta / length;
    let cursor_world = on_plane(viewer, rect, cursor);
    let step = on_plane(viewer, rect, cursor + direction) - cursor_world;
    let normal = Vec2::new(-direction.y, direction.x);
    let highlight = on_plane(viewer, rect, cursor + normal) - cursor_world;
    let visible_length = length.min(rect.size().length());
    let mut distance = 4.0;
    while distance < visible_length {
        if !rect.contains(cursor + direction * distance) {
            break;
        }
        let start = cursor_world + step * distance;
        let end = cursor_world + step * (distance + 3.0).min(visible_length);
        overlay.line(
            start + highlight,
            end + highlight,
            Color::srgba(0.94, 0.94, 0.97, 0.75),
        );
        overlay.line(start, end, Color::srgb(0.09, 0.09, 0.12));
        distance += 7.0;
    }
}
