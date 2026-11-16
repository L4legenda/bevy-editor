use super::*;
use crate::editor::viewport::TransformHandle;
use bevy::{
    gizmos::config::{GizmoConfigGroup, GizmoConfigStore},
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    input_focus::InputFocus,
};

#[path = "model_pick.rs"]
mod picking;
pub(in crate::editor) use picking::Bvh;

pub(in crate::editor) struct Gesture {
    pub model: usize,
    pub base: Snapshot,
    pub tool: Tool,
    pub axis: Option<usize>,
    pub plane: Option<usize>,
    pub handle: Option<TransformHandle>,
    pub typed: String,
    pub start: Option<Vec2>,
    pub drag: bool,
}
impl Gesture {
    pub fn new(tool: Tool, base: Snapshot, model: usize) -> Self {
        Self {
            model,
            base,
            tool,
            axis: None,
            plane: None,
            handle: None,
            typed: String::new(),
            start: None,
            drag: false,
        }
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(in crate::editor) struct ModelGizmos;
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(in crate::editor) struct XrayGizmos;
pub(in crate::editor) fn setup_gizmos(mut configs: ResMut<GizmoConfigStore>) {
    let (c, _) = configs.config_mut::<ModelGizmos>();
    c.render_layers = RenderLayers::layer(LAYER);
    c.depth_bias = -0.002;
    // Keep a full pixel of coverage after FXAA, including nearly horizontal edges.
    c.line.width = 2.0;
    let (c, _) = configs.config_mut::<XrayGizmos>();
    c.render_layers = RenderLayers::layer(LAYER);
    c.depth_bias = -1.0;
    c.line.width = 2.0;
}

pub(in crate::editor) fn rect(
    canvas: &Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
) -> Option<Rect> {
    let (node, transform) = canvas.single().ok()?;
    let size = node.size() * node.inverse_scale_factor();
    (size.min_element() > 1.0).then(|| {
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            size,
        )
    })
}
pub(in crate::editor) fn project(viewer: &Viewer, rect: Rect, point: Vec3) -> Option<Vec2> {
    let p = view_pose(viewer)
        .to_matrix()
        .inverse()
        .transform_point3(point);
    if !viewer.orthographic && p.z >= -0.000001 {
        return None;
    }
    let half_height = view_height(viewer, -p.z) * 0.5;
    let ndc = Vec2::new(
        p.x / (half_height * rect.width() / rect.height()),
        -p.y / half_height,
    );
    Some(rect.center() + ndc * rect.size() * 0.5)
}
pub(in crate::editor) fn ray(viewer: &Viewer, rect: Rect, point: Vec2) -> (Vec3, Vec3) {
    let p = (point - rect.center()) / rect.size() * 2.0;
    let camera = view_pose(viewer);
    if viewer.orthographic {
        let half_height = view_height(viewer, viewer.distance) * 0.5;
        let offset = Vec3::new(
            p.x * half_height * rect.width() / rect.height(),
            -p.y * half_height,
            0.0,
        );
        return (
            camera.translation + camera.rotation * offset
                - *camera.forward() * orthographic_depth(viewer),
            *camera.forward(),
        );
    }
    let direction = (camera.rotation
        * Vec3::new(
            p.x * (FOV * 0.5).tan() * rect.width() / rect.height(),
            -p.y * (FOV * 0.5).tan(),
            -1.0,
        ))
    .normalize();
    (camera.translation, direction)
}
fn on_plane(viewer: &Viewer, rect: Rect, point: Vec2) -> Vec3 {
    let (o, d) = ray(viewer, rect, point);
    let n = *view_pose(viewer).forward();
    let t = (viewer.center - o).dot(n) / d.dot(n).max(0.001);
    o + d * t
}
pub(super) fn segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    point.distance(a + ab * ((point - a).dot(ab) / ab.length_squared().max(0.001)).clamp(0.0, 1.0))
}
fn plane_point(
    viewer: &Viewer,
    rect: Rect,
    point: Vec2,
    pivot: Vec3,
    normal: Vec3,
) -> Option<Vec3> {
    let (origin, direction) = ray(viewer, rect, point);
    let denominator = direction.dot(normal);
    (denominator.abs() > 0.0001)
        .then(|| origin + direction * ((pivot - origin).dot(normal) / denominator))
}
fn axis(doc: &Document, index: usize, local: bool) -> Vec3 {
    let unit = [Vec3::X, Vec3::Y, Vec3::Z][index];
    if local {
        doc.active().map_or(unit, |o| o.transform.rotation * unit)
    } else {
        unit
    }
}
pub(super) fn visible(bvh: &Bvh, viewer: &Viewer, rect: Rect, point: Vec3) -> bool {
    let Some(pixel) = project(viewer, rect, point) else {
        return false;
    };
    let (o, d) = ray(viewer, rect, pixel);
    let depth = (point - o).dot(d);
    bvh.cast(o, d, None)
        .is_none_or(|(_, _, near)| depth <= near + viewer.radius.max(0.001) * 0.005)
}

pub(in crate::editor) fn input(
    mut state: ResMut<State>,
    mut editor: ResMut<Editor>,
    mut viewer: ResMut<Viewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<DrawerMotion>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut text: MessageReader<KeyboardInput>,
    scroll: Res<AccumulatedMouseScroll>,
    mut focus: ResMut<InputFocus>,
    inputs: Query<(), With<TextInput>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    buttons: Query<(&ComputedNode, &UiGlobalTransform), With<Control>>,
    orientation: Option<Res<super::super::navigation_gizmo::State>>,
) {
    let Some(index) = editor.viewing_model.filter(|_| {
        editor.pending.is_none()
            && editor.viewing_image.is_none()
            && !editor.browser_popup
            && !editor.hdri_library_open
    }) else {
        text.clear();
        return;
    };
    if !window.focused {
        state.clear_gesture(Some(index), true);
        state.box_start = None;
        text.clear();
        return;
    }
    if matches!(state.mode(Some(index)), Mode::Uv | Mode::Paint) {
        text.clear();
        state.hover = None;
        return;
    }
    let Some(rect) = rect(&canvas) else {
        text.clear();
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        text.clear();
        return;
    };
    if state.box_start.is_none()
        && orientation
            .as_ref()
            .is_some_and(|gizmo| gizmo.blocks_input())
    {
        state.hover = None;
        text.clear();
        return;
    }
    if drawer.resizing || (state.box_start.is_none() && drawer.covers(cursor, &window)) {
        state.hover = None;
        state.box_start = None;
        if mouse.just_released(MouseButton::Left)
            && state.gesture.as_ref().is_some_and(|gesture| gesture.drag)
        {
            state.clear_gesture(Some(index), false);
        }
        text.clear();
        return;
    }
    if focus.get().is_some_and(|e| inputs.contains(e)) {
        if mouse.just_pressed(MouseButton::Left)
            && rect.contains(cursor)
            && state.input_error.is_none()
        {
            focus.clear();
        } else {
            text.clear();
            return;
        }
    }
    if state.menu || state.add_menu {
        text.clear();
        return;
    }
    if state.gesture.is_none()
        && state.box_start.is_none()
        && buttons.iter().any(|(node, transform)| {
            let p = cursor * window.scale_factor();
            let center = transform.affine().translation;
            node.size().min_element() > 0.0 && (p - center).abs().cmple(node.size() * 0.5).all()
        })
    {
        text.clear();
        return;
    }
    if state.gesture.as_ref().is_some_and(|g| g.model != index) {
        state.clear_gesture(None, true);
    }
    let Some(mut doc) = state.documents.remove(&index) else {
        text.clear();
        return;
    };
    if state.gesture.is_none() && state.bvh_stamp != Some((index, doc.revision)) {
        state.bvh = Bvh::build(&doc);
        state.bvh_stamp = Some((index, doc.revision));
    }
    state.hover = if state.gesture.is_none()
        && state.box_start.is_none()
        && rect.contains(cursor)
        && !keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
        && !mouse.pressed(MouseButton::Right)
        && !mouse.pressed(MouseButton::Middle)
    {
        handles::Frame::new(&doc, &viewer, rect, state.local)
            .and_then(|frame| frame.hit(&viewer, rect, state.tool, cursor))
    } else {
        None
    };
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if let Some(mut gesture) = state.gesture.take() {
        let initial = gesture.start.is_none();
        let start = *gesture.start.get_or_insert(cursor);
        for (i, key) in [KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ]
            .into_iter()
            .enumerate()
        {
            if keys.just_pressed(key) {
                gesture.axis = if gesture.axis == Some(i) {
                    None
                } else {
                    Some(i)
                };
                gesture.typed.clear();
                gesture.plane = None;
            }
        }
        for event in text.read() {
            if event.state != ButtonState::Pressed {
                continue;
            }
            match &event.logical_key {
                Key::Character(value) => {
                    for c in value.chars() {
                        if c.is_ascii_digit() || ['.', '-', ','].contains(&c) {
                            if gesture.typed.len() < 24 {
                                gesture.typed.push(if c == ',' { '.' } else { c });
                            }
                        }
                    }
                }
                Key::Backspace => {
                    gesture.typed.pop();
                }
                _ => {}
            }
        }
        let pivot = snapshot_pivot(&gesture.base, doc.mode).unwrap_or(viewer.center);
        let delta = (cursor - start) * if shift { 0.1 } else { 1.0 };
        let pose = view_pose(&viewer);
        let depth = (pivot - pose.translation).dot(*pose.forward());
        let world_per_pixel = view_height(&viewer, depth) / rect.height();
        let snapped = state.snap || ctrl;
        let typed = gesture.typed.parse::<f32>().ok().filter(|v| v.is_finite());
        let numeric_error = (!gesture.typed.is_empty() && typed.is_none())
            || (gesture.tool == Tool::Scale && typed.is_some_and(|v| v.abs() < 0.0001));
        let direction = gesture.axis.map(|a| axis(&doc, a, state.local));
        let matrix = match gesture.tool {
            Tool::Move | Tool::Universal => {
                let mut movement =
                    (*pose.right() * delta.x - *pose.up() * delta.y) * world_per_pixel;
                if let Some(normal) = gesture.plane.map(|a| axis(&doc, a, state.local)) {
                    movement = plane_point(&viewer, rect, start, pivot, normal)
                        .zip(plane_point(&viewer, rect, cursor, pivot, normal))
                        .map_or(Vec3::ZERO, |(a, b)| b - a);
                    if shift {
                        movement *= 0.1;
                    }
                }
                if let Some(direction) = direction {
                    let screen = project(&viewer, rect, pivot + direction)
                        .zip(project(&viewer, rect, pivot))
                        .map(|(a, b)| a - b)
                        .unwrap_or(Vec2::Y);
                    let mut amount = if screen.length() > 0.01 {
                        delta.dot(screen) / screen.length_squared()
                    } else {
                        -delta.y * world_per_pixel
                    };
                    if let Some(value) = typed {
                        amount = value;
                    } else if snapped {
                        amount = (amount / 0.25).round() * 0.25;
                    }
                    movement = direction * amount;
                } else if let Some(amount) = typed {
                    movement = *pose.right() * amount;
                } else if snapped {
                    let frame = if state.local {
                        doc.active()
                            .map_or(Quat::IDENTITY, |o| o.transform.rotation)
                    } else {
                        Quat::IDENTITY
                    };
                    movement = frame * ((frame.inverse() * movement / 0.25).round() * 0.25);
                }
                Mat4::from_translation(movement)
            }
            Tool::Rotate => {
                let pivot_pixel = project(&viewer, rect, pivot).unwrap_or(rect.center());
                let a = start - pivot_pixel;
                let b = cursor - pivot_pixel;
                let mut angle = if a.length() > 12.0 && b.length() > 12.0 {
                    a.perp_dot(b).atan2(a.dot(b))
                } else {
                    delta.x * 0.01
                };
                if shift {
                    angle *= 0.1;
                }
                if let Some(value) = typed {
                    angle = value.to_radians();
                } else if snapped {
                    angle = (angle.to_degrees() / 15.0).round() * 15.0_f32.to_radians();
                }
                let direction = direction.unwrap_or(-*pose.forward());
                if typed.is_none() {
                    if let Some((a, b)) = plane_point(&viewer, rect, start, pivot, direction)
                        .zip(plane_point(&viewer, rect, cursor, pivot, direction))
                    {
                        let a = (a - pivot).normalize_or_zero();
                        let b = (b - pivot).normalize_or_zero();
                        angle = a.cross(b).dot(direction).atan2(a.dot(b));
                        if shift {
                            angle *= 0.1;
                        }
                        if snapped {
                            angle = (angle.to_degrees() / 15.0).round() * 15.0_f32.to_radians();
                        }
                    } else {
                        angle *= -direction.dot(-*pose.forward()).signum();
                    }
                }
                Mat4::from_translation(pivot)
                    * Mat4::from_quat(Quat::from_axis_angle(direction, angle))
                    * Mat4::from_translation(-pivot)
            }
            Tool::Scale => {
                let value = typed.unwrap_or_else(|| {
                    if gesture.drag
                        && let Some(direction) = direction
                    {
                        let length = (world_per_pixel * 87.5).clamp(0.06, 8.0);
                        if let Some((a, b)) = project(&viewer, rect, pivot + direction * length)
                            .zip(project(&viewer, rect, pivot))
                        {
                            let span = a - b;
                            return (1.0 + delta.dot(span) / span.length_squared().max(0.001))
                                .max(0.01);
                        }
                    }
                    ((delta.x - delta.y) * 0.006).exp()
                });
                let mut factor = value.signum() * value.abs().clamp(0.001, 1000.0);
                if snapped && typed.is_none() {
                    factor = (factor * 10.0).round().max(0.1) / 10.0;
                }
                let mut scale = Vec3::splat(factor);
                if let Some(a) = gesture.axis {
                    scale = Vec3::ONE;
                    scale[a] = factor;
                }
                let rotation = if state.local {
                    doc.active()
                        .map_or(Quat::IDENTITY, |o| o.transform.rotation)
                } else {
                    Quat::IDENTITY
                };
                Mat4::from_translation(pivot)
                    * Mat4::from_quat(rotation)
                    * Mat4::from_scale(scale)
                    * Mat4::from_quat(rotation.inverse())
                    * Mat4::from_translation(-pivot)
            }
        };
        if state.proportional && scroll.delta.y != 0.0 {
            state.influence =
                (state.influence * (scroll.delta.y * 0.08).exp()).clamp(0.001, 10000.0);
        }
        let proportional = state
            .proportional
            .then_some((state.influence, state.connected));
        if numeric_error {
            doc.objects = gesture.base.objects.clone();
            doc.revision += 1;
        } else {
            doc.transform(&gesture.base, matrix, proportional);
        }
        if keys.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Right) {
            doc.restore(gesture.base);
            state.status.clear();
        } else if keys.just_pressed(KeyCode::Enter)
            || (!initial && mouse.just_pressed(MouseButton::Left) && !gesture.drag)
            || (gesture.drag && mouse.just_released(MouseButton::Left))
        {
            if numeric_error {
                state.status =
                    "Введите корректное число; масштаб не может быть нулевым · Esc — отмена".into();
                state.gesture = Some(gesture);
            } else if let Err(error) = doc.validate() {
                doc.restore(gesture.base);
                state.status = error;
            } else {
                doc.checkpoint(gesture.base);
                state.status.clear();
            }
        } else {
            let name = match gesture.tool {
                Tool::Move | Tool::Universal => "Перемещение",
                Tool::Rotate => "Поворот",
                Tool::Scale => "Масштаб",
            };
            let axis = gesture.axis.map_or("Свободно", |i| ["X", "Y", "Z"][i]);
            state.status = format!(
                "{name} · {axis} · {} · Enter — применить · Esc — отменить",
                gesture.typed
            );
            state.gesture = Some(gesture);
        }
        viewer.gesture = None;
    } else {
        text.clear();
        if let Some(start) = state.box_start {
            if mouse.just_pressed(MouseButton::Right)
                || mouse.just_pressed(MouseButton::Middle)
                || (state.box_auto && state.tool != Tool::Universal)
            {
                state.box_start = None;
                state.box_auto = false;
                state.box_armed = false;
            } else if mouse.just_released(MouseButton::Left) || !mouse.pressed(MouseButton::Left) {
                let allowed = drawer.input_rect(rect, window.height());
                let end = cursor.clamp(allowed.min, allowed.max);
                let mode = crate::editor::area_selection::Mode::keys(&keys);
                if state.box_auto && !crate::editor::area_selection::dragged(start, end) {
                    area::click(&mut doc, &state, &viewer, rect, start, mode);
                } else {
                    area::select(
                        &mut doc,
                        &state,
                        &viewer,
                        rect,
                        Rect::from_corners(start, end),
                        mode,
                    );
                }
                state.box_start = None;
                state.box_auto = false;
                state.box_armed = false;
            }
        } else if mouse.just_pressed(MouseButton::Left) && rect.contains(cursor) && !alt {
            if state.box_armed {
                state.box_start = Some(cursor);
                state.box_auto = false;
            } else if state.knife {
                if let Some(start) = state.knife_start.take() {
                    if start.distance(cursor) > 3.0 {
                        let (o, a) = ray(&viewer, rect, start);
                        let (other, b) = ray(&viewer, rect, cursor);
                        let world_normal = if viewer.orthographic {
                            a.cross(other - o)
                        } else {
                            a.cross(b)
                        }
                        .normalize_or_zero();
                        let before = doc.snapshot();
                        if let Some(object) = doc.active_mut() {
                            let inverse = object.transform.to_matrix().inverse();
                            let local_origin = inverse.transform_point3(o);
                            let local_normal = object
                                .transform
                                .to_matrix()
                                .transpose()
                                .transform_vector3(world_normal)
                                .normalize_or_zero();
                            match object
                                .geometry
                                .bisect(local_normal, local_normal.dot(local_origin))
                            {
                                Ok(()) => {
                                    if doc.validate().is_ok() {
                                        doc.checkpoint(before);
                                    } else {
                                        doc.restore(before);
                                    }
                                }
                                Err(error) => state.status = error,
                            }
                        }
                    }
                    state.knife = false;
                } else {
                    state.knife_start = Some(cursor);
                }
            } else {
                let frame = handles::Frame::new(&doc, &viewer, rect, state.local);
                let hit = frame
                    .as_ref()
                    .and_then(|frame| frame.hit(&viewer, rect, state.tool, cursor));
                if let Some(handle) = hit {
                    let tool = match handle {
                        TransformHandle::Axis { tool, .. } => tool,
                        TransformHandle::ViewRotation => Tool::Rotate,
                        _ => Tool::Move,
                    };
                    let mut gesture = Gesture::new(tool, doc.snapshot(), index);
                    gesture.handle = Some(handle);
                    if let TransformHandle::Axis { axis, .. } = handle {
                        gesture.axis = Some(axis);
                    }
                    if let TransformHandle::Plane { normal } = handle {
                        gesture.plane = Some(normal);
                    }
                    gesture.start = Some(cursor);
                    gesture.drag = true;
                    state.gesture = Some(gesture);
                    viewer.gesture = None;
                } else {
                    if state.tool == Tool::Universal {
                        state.box_start = Some(cursor);
                        state.box_auto = true;
                    } else {
                        area::click(
                            &mut doc,
                            &state,
                            &viewer,
                            rect,
                            cursor,
                            crate::editor::area_selection::Mode::keys(&keys),
                        );
                    }
                }
            }
        }
    }
    if doc.dirty() {
        editor.dirty_models.insert(index);
    } else {
        editor.dirty_models.remove(&index);
    }
    state.documents.insert(index, doc);
}

// Only selection and geometry are needed to compute the immutable gesture pivot.
fn snapshot_pivot(base: &Snapshot, mode: Mode) -> Option<Vec3> {
    if mode == Mode::Edit {
        let o = base.objects.iter().find(|o| Some(o.id) == base.active)?;
        let selected = base.selection.selected_vertices(&o.geometry);
        if selected.is_empty() {
            return None;
        }
        Some(
            selected
                .iter()
                .map(|&v| o.transform.transform_point(o.geometry.positions[v]))
                .sum::<Vec3>()
                / selected.len() as f32,
        )
    } else {
        let objects: Vec<_> = base
            .objects
            .iter()
            .filter(|o| base.selected.contains(&o.id) && !o.hidden)
            .collect();
        if objects.is_empty() {
            None
        } else {
            Some(
                objects
                    .iter()
                    .map(|o| o.transform.translation)
                    .sum::<Vec3>()
                    / objects.len() as f32,
            )
        }
    }
}
fn world_per_pixel(viewer: &Viewer, rect: Rect) -> f32 {
    viewer.distance * 2.0 * (FOV * 0.5).tan() / rect.height()
}

pub(in crate::editor) fn draw(
    editor: Res<Editor>,
    state: Res<State>,
    viewer: Res<Viewer>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut depth: Gizmos<ModelGizmos>,
    mut overlay: Gizmos<XrayGizmos>,
    mut selected_depth: Gizmos<selection::SelectedDepth>,
    mut selected_xray: Gizmos<selection::SelectedXray>,
) {
    let Some(doc) = editor
        .viewing_model
        .and_then(|i| state.documents.get(&i))
        .filter(|_| editor.pending.is_none())
    else {
        return;
    };
    if matches!(doc.mode, Mode::Uv | Mode::Paint) {
        return;
    }
    let Some(rect) = rect(&canvas) else {
        return;
    };
    let pose = view_pose(&viewer);
    macro_rules! line {
        ($a:expr,$b:expr,$c:expr) => {{
            if state.xray {
                overlay.line($a, $b, $c);
            } else {
                depth.line($a, $b, $c);
            }
        }};
    }
    macro_rules! selected_line {
        ($a:expr, $b:expr) => {{
            let (a, b) = ($a, $b);
            if state.xray {
                selected_xray.line(a, b, selection::SELECTED);
            } else {
                selected_depth.line(a, b, selection::SELECTED);
            }
            if let Some([a1, b1, a2, b2]) = selection::edge_border(&viewer, rect, &pose, a, b) {
                line!(a1, b1, selection::BORDER);
                line!(a2, b2, selection::BORDER);
            }
        }};
    }
    macro_rules! dot {
        ($p:expr) => {{
            let p: Vec3 = $p;
            let scale = view_height(
                &viewer,
                (p - pose.translation).dot(*pose.forward()).max(0.000001),
            ) / rect.height();
            let right = *pose.right() * scale * 1.5;
            for y in [-1.0, 0.0, 1.0] {
                let center = p + *pose.up() * scale * y;
                line!(center - right, center + right, selection::BORDER);
            }
        }};
    }
    let orange = selection::SELECTED;
    let gray = Color::srgb(0.13, 0.16, 0.20);
    let step = world_per_pixel(&viewer, rect);
    for object in &doc.objects {
        if object.hidden {
            continue;
        }
        let Some(rendered) = state.render_objects.get(&object.id) else {
            continue;
        };
        if state.xray && !(doc.mode == Mode::Edit && doc.active == Some(object.id)) {
            for e in rendered.edges.keys() {
                overlay.line(
                    object
                        .transform
                        .transform_point(object.geometry.positions[e.0]),
                    object
                        .transform
                        .transform_point(object.geometry.positions[e.1]),
                    gray,
                );
            }
        }
        if doc.mode == Mode::Edit && doc.active == Some(object.id) {
            let chosen = doc.selection.selected_vertices(&object.geometry);
            for (e, faces) in rendered.edges.iter() {
                let selected = match doc.selection.kind {
                    Element::Vertex => false,
                    Element::Edge => doc.selection.edges.contains(e),
                    Element::Face => faces.iter().any(|f| doc.selection.faces.contains(f)),
                };
                let a = object
                    .transform
                    .transform_point(object.geometry.positions[e.0]);
                let b = object
                    .transform
                    .transform_point(object.geometry.positions[e.1]);
                if !selected {
                    line!(a, b, gray);
                }
            }
            if doc.selection.kind == Element::Vertex {
                for (i, p) in object.geometry.positions.iter().enumerate() {
                    let p = object.transform.transform_point(*p);
                    if !chosen.contains(&i) {
                        dot!(p);
                    }
                }
            }
            if doc.selection.kind == Element::Face {
                for (i, f) in object.geometry.faces.iter().enumerate() {
                    let center = object.transform.transform_point(object.geometry.center(f));
                    if !doc.selection.faces.contains(&i) {
                        dot!(center);
                    }
                }
            }
        } else if doc.mode == Mode::Object && doc.selected.contains(&object.id) {
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for p in &object.geometry.positions {
                min = min.min(*p);
                max = max.max(*p);
            }
            if min.is_finite() {
                let padding = Vec3::splat(
                    step * 3.0 / object.transform.scale.abs().min_element().max(0.000001),
                );
                for (a, b) in crate::editor::selection_outline::corner_segments(
                    min - padding,
                    max + padding,
                    bevy::math::Affine3A::from_mat4(object.transform.to_matrix()),
                ) {
                    selected_line!(a, b);
                }
            }
        }
    }
    if let Some(mut frame) = handles::Frame::new(doc, &viewer, rect, state.local) {
        if state.local
            && let Some(g) = state.gesture.as_ref().filter(|g| g.tool == Tool::Rotate)
        {
            frame.rotation = g
                .base
                .objects
                .iter()
                .find(|o| Some(o.id) == g.base.active)
                .map_or(frame.rotation, |o| o.transform.rotation);
        }
        if state.tool.operations().contains(&Tool::Rotate) {
            for axis in 0..3 {
                let handle = TransformHandle::Axis {
                    tool: Tool::Rotate,
                    axis,
                };
                let chosen = state
                    .gesture
                    .as_ref()
                    .and_then(|g| g.handle)
                    .or(state.hover)
                    == Some(handle);
                let color = if chosen {
                    Color::srgb(1.0, 0.81, 0.38)
                } else {
                    crate::editor::viewport::ROTATION_COLORS[axis]
                };
                for (a, b) in frame.rings(axis) {
                    overlay.line(a, b, color);
                }
            }
        }
        let pivot = frame.pivot;
        if state.proportional && doc.mode == Mode::Edit {
            let radius = state.influence;
            let mut previous = pivot + *pose.right() * radius;
            for i in 1..=64 {
                let angle = i as f32 * std::f32::consts::TAU / 64.0;
                let next =
                    pivot + (*pose.right() * angle.cos() + *pose.up() * angle.sin()) * radius;
                overlay.line(previous, next, Color::srgba(0.8, 0.8, 0.9, 0.5));
                previous = next;
            }
        }
    }
    if let Some(cursor) = window.cursor_position() {
        if let Some(start) = state.knife_start {
            overlay.line(
                on_plane(&viewer, rect, start),
                on_plane(&viewer, rect, cursor),
                orange,
            );
        }
    }
}
