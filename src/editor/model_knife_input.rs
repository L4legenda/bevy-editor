use super::*;
use crate::editor::model_mesh::KnifeSegment;
use std::time::{Duration, Instant};

#[path = "model_knife_projection.rs"]
mod projection;

#[derive(Clone, Copy, Default)]
enum AngleMode {
    #[default]
    Free,
    Screen,
    Relative,
}

pub(super) struct Preview {
    paths: Vec<Vec<Vec2>>,
    selected: Option<BTreeSet<usize>>,
    edges: BTreeMap<Edge, Vec<usize>>,
    projection: Option<projection::Projection>,
    confirmed: Vec<KnifeSegment>,
    live: Vec<KnifeSegment>,
    changed: bool,
    through: bool,
    xray: bool,
    angle: AngleMode,
    angle_step: f32,
    axis: Option<(usize, bool)>,
    reference: usize,
    measure: usize,
    last_click: Option<(Instant, Vec2)>,
    drag: bool,
    message: Option<String>,
}

impl Gesture {
    pub(in crate::editor) fn knife(
        doc: &Document,
        model: usize,
        only_selected: bool,
    ) -> Option<Self> {
        let object = doc.active().filter(|object| !object.hidden)?;
        let mut gesture = Self::new(
            Tool::Move,
            doc.snapshot(),
            model,
            object.transform.translation,
        );
        gesture.knife = Some(Preview {
            paths: vec![Vec::new()],
            selected: only_selected.then(|| doc.selection.selected_faces(&object.geometry)),
            edges: object.geometry.edges(),
            projection: None,
            confirmed: Vec::new(),
            live: Vec::new(),
            changed: false,
            through: false,
            xray: false,
            angle: AngleMode::Free,
            angle_step: 45.0,
            axis: None,
            reference: 0,
            measure: 0,
            last_click: None,
            drag: false,
            message: None,
        });
        Some(gesture)
    }
}

pub(super) enum Outcome {
    Preview(String),
    Commit,
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
    events: impl Iterator<Item = &'a KeyboardInput>,
) -> Outcome {
    if keys.just_pressed(KeyCode::Escape) {
        return Outcome::Cancel;
    }
    let preview = gesture.knife.as_mut().unwrap();
    let Some(object) = doc.active() else {
        return Outcome::Cancel;
    };
    if keys.just_pressed(KeyCode::KeyC) {
        preview.through = !preview.through;
        preview.changed = true;
    }
    if keys.just_pressed(KeyCode::KeyV) {
        preview.xray = !preview.xray;
    }
    if keys.just_pressed(KeyCode::KeyA) {
        preview.angle = match preview.angle {
            AngleMode::Free => AngleMode::Screen,
            AngleMode::Screen => AngleMode::Relative,
            AngleMode::Relative => AngleMode::Free,
        };
        preview.axis = None;
        gesture.typed.clear();
    }
    if keys.just_pressed(KeyCode::KeyR) {
        preview.reference = preview.reference.wrapping_add(1);
    }
    if keys.just_pressed(KeyCode::KeyS) {
        preview.measure = (preview.measure + 1) % 4;
    }
    for (axis, key) in [KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ]
        .into_iter()
        .enumerate()
    {
        if keys.just_pressed(key) {
            preview.axis = match preview.axis {
                Some((i, false)) if i == axis => Some((axis, true)),
                Some((i, true)) if i == axis => None,
                _ => Some((axis, false)),
            };
        }
    }
    if !matches!(preview.angle, AngleMode::Free) {
        for event in events.filter(|event| event.state == ButtonState::Pressed) {
            match &event.logical_key {
                Key::Character(value) => {
                    for c in value.chars().filter(|c| c.is_ascii_digit() || *c == '.') {
                        if gesture.typed.len() < 12 {
                            gesture.typed.push(c);
                        }
                    }
                }
                Key::Backspace => {
                    gesture.typed.pop();
                }
                _ => {}
            }
        }
        if let Ok(step) = gesture.typed.parse::<f32>() {
            preview.angle_step = step.clamp(0.1, 180.0);
        }
    }
    let control = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if control && keys.just_pressed(KeyCode::KeyZ) {
        if preview.paths.last().is_some_and(Vec::is_empty) && preview.paths.len() > 1 {
            preview.paths.pop();
        }
        preview.paths.last_mut().unwrap().pop();
        preview.last_click = None;
        preview.changed = true;
        preview.message = None;
    }
    if mouse.just_pressed(MouseButton::Right) && preview.paths.last().is_some_and(|p| !p.is_empty())
    {
        preview.paths.push(Vec::new());
        preview.last_click = None;
    }
    if preview
        .projection
        .as_ref()
        .is_none_or(|projection| !projection.matches(viewer, rect))
    {
        preview.projection = Some(projection::Projection::new(bvh, viewer, rect));
        preview.changed = true;
    }
    let cursor = canvas_input.then(|| {
        adjusted_cursor(
            preview,
            object,
            bvh,
            viewer,
            rect,
            cursor,
            keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            control,
        )
    });
    if mouse.just_released(MouseButton::Left) {
        preview.drag = false;
    }
    if let Some(cursor) = cursor {
        let click = mouse.just_pressed(MouseButton::Left);
        let path = preview.paths.last_mut().unwrap();
        if click {
            let now = Instant::now();
            let closed = path.len() >= 3
                && preview.last_click.is_some_and(|(time, point)| {
                    now.duration_since(time) < Duration::from_millis(350)
                        && point.distance(cursor) < 5.0
                });
            if path.last().is_none_or(|p| p.distance(cursor) > 1.0) {
                path.push(cursor);
            }
            if closed {
                path.push(path[0]);
                preview.paths.push(Vec::new());
                preview.last_click = None;
            } else {
                preview.last_click = Some((now, cursor));
                preview.drag = true;
            }
            preview.changed = true;
            preview.message = None;
        } else if preview.drag
            && mouse.pressed(MouseButton::Left)
            && path
                .last()
                .is_some_and(|point| point.distance(cursor) >= 6.0)
        {
            path.push(cursor);
            preview.changed = true;
        }
    }
    let projection = preview.projection.as_ref().unwrap();
    let trace = |a, b| projection.trace(object, a, b, preview.through, preview.selected.as_ref());
    if preview.changed {
        preview.confirmed = preview
            .paths
            .iter()
            .flat_map(|path| path.windows(2))
            .flat_map(|pair| trace(pair[0], pair[1]))
            .collect();
        preview.changed = false;
    }
    preview.live = preview
        .paths
        .last()
        .and_then(|p| p.last())
        .copied()
        .zip(cursor)
        .map_or_else(Vec::new, |(a, b)| trace(a, b));

    if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space]) {
        let mut snapshot = gesture.base.clone();
        let object = snapshot
            .objects
            .iter_mut()
            .find(|o| Some(o.id) == snapshot.active)
            .unwrap();
        match object.geometry.apply_knife(&preview.confirmed) {
            Ok(edges)
                if snapshot
                    .objects
                    .iter()
                    .map(|o| o.geometry.positions.len())
                    .sum::<usize>()
                    <= 250_000 =>
            {
                snapshot.selection.clear();
                snapshot.selection.kind = Element::Edge;
                snapshot.selection.edges = edges;
                doc.restore(snapshot);
                doc.checkpoint(gesture.base.clone());
                return Outcome::Commit;
            }
            Ok(_) => preview.message = Some("Разрез превысит лимит модели в 250 000 вершин".into()),
            Err(error) => preview.message = Some(error),
        }
    }
    let status = preview.message.clone().unwrap_or_else(|| {
        let mut status = format!(
            "Нож · {} · {}",
            if preview.selected.is_some() {
                "Только выделенное"
            } else {
                "Все грани"
            },
            if preview.through {
                "Насквозь"
            } else {
                "Видимые участки"
            }
        );
        if let Some((axis, local)) = preview.axis {
            status += &format!(
                " · {} {}",
                ["X", "Y", "Z"][axis],
                if local {
                    "локальная"
                } else {
                    "глобальная"
                }
            );
        } else if !matches!(preview.angle, AngleMode::Free) {
            status += &format!(" · Шаг угла {:.1}°", preview.angle_step);
        }
        if matches!(preview.measure, 1 | 3) {
            let length: f32 = preview
                .live
                .iter()
                .map(|cut| {
                    object
                        .transform
                        .transform_point(cut.ends[0])
                        .distance(object.transform.transform_point(cut.ends[1]))
                })
                .sum();
            status += &format!(" · Длина {:.4}", length);
        }
        if matches!(preview.measure, 2 | 3)
            && let Some((start, end)) = preview
                .paths
                .last()
                .and_then(|path| path.last())
                .copied()
                .zip(cursor)
        {
            let delta = end - start;
            status += &format!(" · Угол {:.1}°", (-delta.y).atan2(delta.x).to_degrees());
        }
        status
    });
    Outcome::Preview(status)
}

fn adjusted_cursor(
    preview: &Preview,
    object: &Object,
    bvh: &Bvh,
    viewer: &Viewer,
    rect: Rect,
    cursor: Vec2,
    midpoint: bool,
    ignore_snap: bool,
) -> Vec2 {
    let world = |v| {
        object
            .transform
            .transform_point(object.geometry.positions[v])
    };
    let p = |v| project(viewer, rect, world(v));
    let nearest_on = |p: Vec2, a: Vec2, b: Vec2| {
        let delta = b - a;
        let t = ((p - a).dot(delta) / delta.length_squared().max(1.0e-6)).clamp(0.0, 1.0);
        (a.lerp(b, t), t)
    };
    let edges = || {
        preview.edges.iter().filter(|(_, faces)| {
            preview
                .selected
                .as_ref()
                .is_none_or(|selected| faces.iter().any(|face| selected.contains(face)))
        })
    };
    let mut snapped = cursor;
    if !ignore_snap {
        let mut best = if midpoint { 324.0 } else { 100.0 };
        let pose = view_pose(viewer);
        for (&edge, _) in edges() {
            let (Some(a), Some(b)) = (p(edge.0), p(edge.1)) else {
                continue;
            };
            let [wa, wb] = [world(edge.0), world(edge.1)];
            let (on_edge, t) = nearest_on(cursor, a, b);
            let candidate = if midpoint {
                let middle = (wa + wb) * 0.5;
                let Some(at) = project(viewer, rect, middle) else {
                    continue;
                };
                (at, middle)
            } else if cursor.distance_squared(a).min(cursor.distance_squared(b)) <= 36.0 {
                if cursor.distance_squared(a) < cursor.distance_squared(b) {
                    (a, wa)
                } else {
                    (b, wb)
                }
            } else {
                let factor = if viewer.orthographic {
                    t
                } else {
                    let depth_a = (wa - pose.translation).dot(*pose.forward());
                    let depth_b = (wb - pose.translation).dot(*pose.forward());
                    (t / depth_b) / ((1.0 - t) / depth_a + t / depth_b)
                };
                (on_edge, wa.lerp(wb, factor))
            };
            let distance = cursor.distance_squared(candidate.0);
            if distance < best {
                if preview.through || visible(bvh, viewer, rect, candidate.1) {
                    best = distance;
                    snapped = candidate.0;
                }
            }
        }
        if !midpoint {
            for pair in preview.paths.iter().flat_map(|path| path.windows(2)) {
                let (candidate, _) = nearest_on(cursor, pair[0], pair[1]);
                let distance = cursor.distance_squared(candidate);
                if distance < best {
                    best = distance;
                    snapped = candidate;
                }
            }
        }
    }
    let Some(&anchor) = preview.paths.last().and_then(|path| path.last()) else {
        return snapped;
    };
    let delta = snapped - anchor;
    if let Some((axis, local)) = preview.axis {
        let v = [Vec3::X, Vec3::Y, Vec3::Z][axis];
        let direction = if local {
            object.transform.rotation * v
        } else {
            v
        };
        let (o, d) = ray(viewer, rect, anchor);
        let world = bvh
            .cast(o, d, Some(object.id))
            .map_or(object.transform.translation, |(_, _, t)| o + d * t);
        if let (Some(a), Some(b)) = (
            project(viewer, rect, world),
            project(viewer, rect, world + direction * viewer.radius.max(0.001)),
        ) {
            let direction = (b - a).normalize_or_zero();
            if direction != Vec2::ZERO {
                return anchor + direction * delta.dot(direction);
            }
        }
    }
    if matches!(preview.angle, AngleMode::Free) {
        return snapped;
    }
    let step = preview.angle_step.to_radians();
    if matches!(preview.angle, AngleMode::Relative) {
        let (origin, direction) = ray(viewer, rect, anchor);
        if let Some((_, face, distance)) = bvh.cast_where(origin, direction, |triangle| {
            triangle.object == object.id
                && preview
                    .selected
                    .as_ref()
                    .is_none_or(|faces| faces.contains(&triangle.face))
        }) {
            let origin = origin + direction * distance;
            let face = &object.geometry.faces[face];
            let transform = object.transform.to_matrix();
            let normal = transform
                .inverse()
                .transpose()
                .transform_vector3(object.geometry.normal(face))
                .normalize_or_zero();
            let mut references: Vec<_> = face
                .edges()
                .filter_map(|(a, b)| {
                    let distance = segment(anchor, p(a)?, p(b)?);
                    let edge = world(b) - world(a);
                    let reference = (edge - normal * edge.dot(normal)).normalize_or_zero();
                    (reference != Vec3::ZERO).then_some((distance, reference))
                })
                .collect();
            references.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let Some(&(_, reference)) =
                references.get(preview.reference % references.len().max(1))
                && let Some(point) = plane_point(viewer, rect, snapped, origin, normal)
            {
                let side = normal.cross(reference);
                let delta = point - origin;
                let angle = (delta.dot(side).atan2(delta.dot(reference)) / step).round() * step;
                let position =
                    origin + (reference * angle.cos() + side * angle.sin()) * delta.length();
                if let Some(at) = project(viewer, rect, position) {
                    return at;
                }
            }
        }
    }
    let angle = (delta.to_angle() / step).round() * step;
    anchor + Vec2::from_angle(angle) * delta.length()
}

pub(super) fn draw(
    gesture: &Gesture,
    viewer: &Viewer,
    rect: Rect,
    depth: &mut Gizmos<ModelGizmos>,
    xray: &mut Gizmos<XrayGizmos>,
) {
    let Some(preview) = &gesture.knife else {
        return;
    };
    let Some(object) = gesture
        .base
        .objects
        .iter()
        .find(|o| Some(o.id) == gesture.base.active)
    else {
        return;
    };
    let pose = view_pose(viewer);
    for (segments, color) in [
        (&preview.confirmed, Color::srgb(1.0, 0.62, 0.22)),
        (&preview.live, Color::srgb(0.66, 0.48, 1.0)),
    ] {
        for cut in segments {
            let [a, b] = cut.ends.map(|p| object.transform.transform_point(p));
            if preview.xray {
                xray.line(a, b, color);
            } else {
                depth.line(a, b, color);
            }
            for point in [a, b] {
                let height = view_height(viewer, (point - pose.translation).dot(*pose.forward()));
                let radius = 2.5 * height / rect.height();
                for axis in [*pose.right(), *pose.up()] {
                    let offset = axis * radius;
                    if preview.xray {
                        xray.line(point - offset, point + offset, color);
                    } else {
                        depth.line(point - offset, point + offset, color);
                    }
                }
            }
        }
    }
}
