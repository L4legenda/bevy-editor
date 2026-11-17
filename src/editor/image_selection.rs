//! Selection gestures and non-accumulating free transforms over PhotoCraft commands.
use super::*;
use crate::editor::photo_document::{LayerMove, SelectionTool};
use serde_json::json;
#[path = "image_transform_preview.rs"]
pub(in crate::editor) mod live;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Mode {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Intersect => "intersect",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum TransformMode {
    #[default]
    Free,
    Scale,
    Rotate,
    Skew,
    Distort,
    Perspective,
}
#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum SelectionAction {
    Tool(SelectionTool),
    UseTool,
    Menu,
    Mode(Mode),
    Command(&'static str),
    Tolerance(i32),
    QuickSize(i32),
    SampleAll,
    Contiguous,
    Transform,
    TransformMenu,
    TransformMode(TransformMode),
    Apply,
    Cancel,
    Flip(bool),
    Rotate(f32),
    Linked,
    Nearest,
    FinishPolygon,
    RemovePoint,
    Nudge(i32, i32),
}
#[derive(Default)]
pub(in crate::editor) struct SelectionState {
    pub tool: SelectionTool,
    pub mode: Mode,
    pub menu: u8,
    pub dismissed: bool,
    pub tolerance: u8,
    pub quick_size: u16,
    pub sample_all: bool,
    pub noncontiguous: bool,
    pub mode_transform: TransformMode,
    pub unlinked: bool,
    pub nearest: bool,
    pub draft: Option<Draft>,
    pub transform: Option<Transform>,
}
pub(in crate::editor) struct Draft {
    pub id: usize,
    pub tool: SelectionTool,
    pub points: Vec<Vec2>,
    pub hover: Vec2,
    pub mode: Mode,
    pub started_shift: bool,
    pub started_alt: bool,
    pub bounds: [f32; 4],
    last_click: std::time::Instant,
}
pub(in crate::editor) struct Transform {
    pub id: usize,
    pub frame: [f32; 4],
    pub quad: [Vec2; 4],
    original: LayerMove,
    drag: Option<Drag>,
    pub(super) live: Option<live::Live>,
    pub(super) deferred: bool,
    applied_quad: [Vec2; 4],
}
struct Drag {
    hit: usize,
    start: Vec2,
    quad: [Vec2; 4],
}

impl SelectionState {
    pub fn active(&self, action: SelectionAction, tool: PhotoTool) -> bool {
        match action {
            SelectionAction::UseTool => matches!(tool, PhotoTool::Select(_)),
            SelectionAction::Tool(kind) => tool == PhotoTool::Select(kind),
            SelectionAction::Menu => self.menu == 1,
            SelectionAction::TransformMenu => self.menu == 2,
            SelectionAction::Mode(mode) => self.mode == mode,
            SelectionAction::Transform => tool == PhotoTool::Transform,
            SelectionAction::TransformMode(mode) => self.mode_transform == mode,
            SelectionAction::SampleAll => self.sample_all,
            SelectionAction::Contiguous => !self.noncontiguous,
            SelectionAction::Linked => !self.unlinked,
            SelectionAction::Nearest => self.nearest,
            _ => false,
        }
    }
}

pub fn dispatch(state: &mut State, id: usize, action: SelectionAction) {
    if state.selection.quick_size == 0 {
        state.selection.quick_size = 30;
        state.selection.tolerance = 32;
    }
    let command = match action {
        SelectionAction::Command(command) => Some(command),
        _ => None,
    };
    if !matches!(
        action,
        SelectionAction::Menu | SelectionAction::TransformMenu
    ) {
        state.selection.menu = 0;
    }
    match action {
        SelectionAction::Menu => {
            state.selection.menu = if state.selection.menu == 1 { 0 } else { 1 }
        }
        SelectionAction::TransformMenu => {
            state.selection.menu = if state.selection.menu == 2 { 0 } else { 2 }
        }
        SelectionAction::UseTool => {
            dispatch(state, id, SelectionAction::Tool(state.selection.tool))
        }
        SelectionAction::Tool(tool) => {
            state.selection.tool = tool;
            state.select_tool(PhotoTool::Select(tool));
            if tool == SelectionTool::Quick && state.selection.mode == Mode::Intersect {
                state.selection.mode = Mode::Add;
            }
        }
        SelectionAction::Mode(mode) => state.selection.mode = mode,
        SelectionAction::Tolerance(step) => {
            state.selection.tolerance =
                (i32::from(state.selection.tolerance) + step).clamp(0, 255) as u8
        }
        SelectionAction::QuickSize(step) => {
            state.selection.quick_size =
                (i32::from(state.selection.quick_size) + step).clamp(1, 256) as u16
        }
        SelectionAction::SampleAll => state.selection.sample_all ^= true,
        SelectionAction::Contiguous => state.selection.noncontiguous ^= true,
        SelectionAction::Linked => state.selection.unlinked ^= true,
        SelectionAction::Nearest => {
            state.selection.nearest ^= true;
            if let Some(transform) = state.selection.transform.as_mut()
                && transform.live.as_ref().is_some_and(live::Live::usable)
                && (transform.drag.is_some()
                    || transform.deferred
                    || live::Live::needs_job(transform.frame, transform.quad))
            {
                transform.deferred = true;
                return;
            }
            if let Some(transform) = state.selection.transform.as_mut()
                && let Some(session) = state.sessions.get_mut(&transform.id)
                && let Err(error) = session.doc.preview_transform(
                    &mut transform.original,
                    transform.frame,
                    transform.quad.map(|p| p.to_array()),
                    state.selection.nearest,
                )
            {
                state.error = Some(error);
            }
        }
        SelectionAction::TransformMode(mode) => state.selection.mode_transform = mode,
        SelectionAction::Transform => {
            state.selection.draft = None;
            state.select_tool(PhotoTool::Transform);
            if state.selection.transform.is_none() {
                if let Some(doc) = state.sessions.get(&id).map(|s| &s.doc) {
                    if let Some(frame) = doc.transform_frame() {
                        state.selection.transform = Some(Transform {
                            id,
                            frame,
                            quad: frame_quad(frame),
                            original: doc.begin_move(),
                            drag: None,
                            live: live::Live::new(doc, frame),
                            deferred: false,
                            applied_quad: frame_quad(frame),
                        });
                    } else {
                        state.error = Some(
                            "Выберите незаблокированный слой с содержимым для трансформации".into(),
                        );
                    }
                }
            }
        }
        SelectionAction::Apply => {
            if !finish_pending(state) {
                return;
            }
            state.selection.transform = None;
            state.selection.draft = None;
            state.select_tool(PhotoTool::Move);
            state.error = None;
        }
        SelectionAction::Cancel => {
            state.selection.draft = None;
            if let Some(mut transform) = state.selection.transform.take()
                && let Some(session) = state.sessions.get_mut(&transform.id)
            {
                session.doc.cancel_move(&mut transform.original);
            }
            if state.tool == PhotoTool::Transform {
                state.select_tool(PhotoTool::Move);
            }
            state.error = None;
        }
        SelectionAction::FinishPolygon => finish_selection(state, id),
        SelectionAction::RemovePoint => {
            if let Some(draft) = state.selection.draft.as_mut() {
                draft.points.pop();
                if draft.points.is_empty() {
                    state.selection.draft = None;
                }
            }
        }
        SelectionAction::Flip(horizontal) => {
            if state.selection.transform.is_none() {
                dispatch(state, id, SelectionAction::Transform);
            }
            if let Some(transform) = &state.selection.transform {
                let [a, b, c, d] = transform.quad;
                let quad = if horizontal {
                    [b, a, d, c]
                } else {
                    [d, c, b, a]
                };
                preview(state, quad, false);
            }
        }
        SelectionAction::Rotate(degrees) => {
            if state.selection.transform.is_none() {
                dispatch(state, id, SelectionAction::Transform);
            }
            if let Some(transform) = &state.selection.transform {
                let center = transform.quad.iter().copied().sum::<Vec2>() * 0.25;
                let rotation = Rot2::radians(degrees.to_radians());
                let quad = transform
                    .quad
                    .map(|point| center + rotation * (point - center));
                preview(state, quad, false);
            }
        }
        SelectionAction::Nudge(x, y) => {
            if let Some(transform) = &state.selection.transform {
                preview(
                    state,
                    transform.quad.map(|p| p + Vec2::new(x as f32, y as f32)),
                    false,
                );
            } else if let Some(session) = state.sessions.get_mut(&id) {
                if let Err(error) = session.doc.layer_command(
                    "edit.transform",
                    json!({"dx":x,"dy":y,"interpolation":"nearest",
                        "target":if session.doc.mask_target && !session.doc.filter_target {"mask"} else {"pixels"}}),
                ) {
                    state.error = Some(error);
                }
            }
        }
        _ => {}
    }
    if let Some(command) = command {
        if state.tool == PhotoTool::Transform {
            state.select_tool(PhotoTool::Move);
        }
        state.selection.transform = None;
        state.selection.draft = None;
        if let Some(session) = state.sessions.get_mut(&id) {
            if command == "select.deselect" && session.doc.document.selection.is_none() {
                return;
            }
            if let Err(error) = session.doc.layer_command(command, json!({})) {
                state.error = Some(error);
            }
        }
    }
}

pub fn frame_quad(frame: [f32; 4]) -> [Vec2; 4] {
    [
        Vec2::new(frame[0], frame[1]),
        Vec2::new(frame[2], frame[1]),
        Vec2::new(frame[2], frame[3]),
        Vec2::new(frame[0], frame[3]),
    ]
}
pub fn handles(quad: [Vec2; 4], scale: f32) -> [Vec2; 9] {
    let top = (quad[0] + quad[1]) * 0.5;
    let outward = (top - (quad[2] + quad[3]) * 0.5).normalize_or_zero();
    [
        quad[0],
        top,
        quad[1],
        (quad[1] + quad[2]) * 0.5,
        quad[2],
        (quad[2] + quad[3]) * 0.5,
        quad[3],
        (quad[3] + quad[0]) * 0.5,
        top + outward * 24.0 / scale.max(0.0001),
    ]
}
pub fn hit(quad: [Vec2; 4], point: Vec2, scale: f32) -> Option<usize> {
    if let Some((i, _)) = handles(quad, scale)
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.distance(point)))
        .filter(|(_, d)| *d * scale <= 9.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
    {
        return Some(i);
    }
    let mut positive = false;
    let mut negative = false;
    for i in 0..4 {
        let cross = (quad[(i + 1) % 4] - quad[i]).perp_dot(point - quad[i]);
        positive |= cross > 0.0;
        negative |= cross < 0.0;
    }
    if !(positive && negative) {
        Some(9)
    } else {
        None
    }
}
fn preview(state: &mut State, quad: [Vec2; 4], interactive: bool) {
    let nearest = state.selection.nearest;
    let Some(transform) = state.selection.transform.as_mut() else {
        return;
    };
    let same = transform
        .quad
        .iter()
        .zip(quad)
        .all(|(a, b)| a.distance_squared(b) < 0.000001);
    if same && (!transform.deferred || interactive) {
        return;
    }
    if let Err(error) = PhotoDocument::check_transform(quad.map(|p| p.to_array())) {
        state.error = Some(error);
        return;
    }
    if interactive && transform.live.as_ref().is_some_and(live::Live::usable) {
        transform.quad = quad;
        transform.deferred = true;
        state.error = None;
        return;
    }
    if let Some(session) = state.sessions.get_mut(&transform.id) {
        match session.doc.preview_transform(
            &mut transform.original,
            transform.frame,
            quad.map(|p| p.to_array()),
            nearest,
        ) {
            Ok(()) => {
                transform.quad = quad;
                transform.deferred = false;
                transform.applied_quad = quad;
                state.error = None;
            }
            Err(error) => {
                let _ = session.doc.preview_transform(
                    &mut transform.original,
                    transform.frame,
                    transform.quad.map(|p| p.to_array()),
                    nearest,
                );
                state.error = Some(error);
            }
        }
    }
}

pub(super) fn poll(state: &mut State) {
    if let Some(t) = state.selection.transform.as_mut()
        && let Some(session) = state.sessions.get_mut(&t.id)
        && let Some(live) = &mut t.live
    {
        live.poll(session.doc.revision);
        if let Some((quad, nearest, revision, result)) = live.poll_commit()
            && t.drag.is_none()
            && t.deferred
            && t.quad == quad
            && state.selection.nearest == nearest
        {
            if revision != session.doc.revision {
                live.disable();
                t.deferred = false;
                t.quad = t.applied_quad;
            } else {
                match result {
                    Ok((document, image, changed)) => {
                        session
                            .doc
                            .accept_transform(&mut t.original, document, changed);
                        session.prepared =
                            Some((session.doc.revision, session.doc.view_revision, image));
                        t.applied_quad = quad;
                        t.deferred = false;
                        state.error = None;
                    }
                    Err(error) => {
                        live.disable();
                        t.deferred = false;
                        t.quad = t.applied_quad;
                        state.error = Some(error);
                    }
                }
            }
        }
        if t.drag.is_none() && t.deferred && live.usable() && live::Live::needs_job(t.frame, t.quad)
        {
            live.commit(
                &session.doc,
                &t.original,
                t.frame,
                t.quad,
                state.selection.nearest,
            );
        }
    }
}

pub(super) fn finish_pending(state: &mut State) -> bool {
    let Some(t) = state.selection.transform.as_mut().filter(|t| t.deferred) else {
        return true;
    };
    t.drag = None;
    if let Some(live) = &mut t.live {
        live.cancel_commit();
    }
    let quad = t.quad;
    preview(state, quad, false);
    !state
        .selection
        .transform
        .as_ref()
        .is_some_and(|t| t.deferred)
}

fn release(state: &mut State) {
    if let Some(t) = state.selection.transform.as_mut().filter(|t| t.deferred)
        && let Some(live) = t.live.as_mut().filter(|l| l.usable())
        && live::Live::needs_job(t.frame, t.quad)
        && let Some(session) = state.sessions.get(&t.id)
    {
        live.commit(
            &session.doc,
            &t.original,
            t.frame,
            t.quad,
            state.selection.nearest,
        );
        return;
    }
    finish_pending(state);
}

/// Stop a dragged corner just before a quad folds, as PhotoCraft does.
fn keep_convex(from: [Vec2; 4], to: [Vec2; 4]) -> [Vec2; 4] {
    let turns = |q: [Vec2; 4]| -> [f32; 4] {
        std::array::from_fn(|i| (q[(i + 1) % 4] - q[i]).perp_dot(q[(i + 2) % 4] - q[(i + 1) % 4]))
    };
    let before = turns(from);
    let sign = before[0].signum();
    let minimum = before.iter().map(|v| v.abs()).fold(f32::INFINITY, f32::min) * 0.001;
    let valid = |q| turns(q).iter().all(|v| *v * sign > minimum);
    if minimum <= 0.0 || !minimum.is_finite() || valid(to) {
        return to;
    }
    let at = |t| std::array::from_fn(|i| from[i].lerp(to[i], t));
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = (low + high) * 0.5;
        if valid(at(mid)) {
            low = mid
        } else {
            high = mid
        }
    }
    at(low)
}

pub fn transformed_quad(
    quad: [Vec2; 4],
    hit: usize,
    start: Vec2,
    end: Vec2,
    mode: TransformMode,
    linked: bool,
    shift: bool,
    alt: bool,
    control: bool,
) -> [Vec2; 4] {
    let center = quad.iter().copied().sum::<Vec2>() * 0.25;
    let mut delta = end - start;
    if hit == 9 {
        if shift {
            if delta.x.abs() > delta.y.abs() {
                delta.y = 0.0
            } else {
                delta.x = 0.0
            }
        }
        return quad.map(|p| p + delta.round());
    }
    if hit == 8 || mode == TransformMode::Rotate {
        let a = start - center;
        let b = end - center;
        let mut angle = a.perp_dot(b).atan2(a.dot(b));
        if shift {
            angle = (angle / (std::f32::consts::PI / 12.0)).round() * (std::f32::consts::PI / 12.0);
        }
        let rotation = Rot2::radians(angle);
        return quad.map(|p| center + rotation * (p - center));
    }
    if hit >= 8 {
        return quad;
    }
    let corner = hit % 2 == 0;
    let i = hit / 2;
    let paired_axis = || {
        let horizontal = [1, 0, 3, 2][i];
        let vertical = [3, 2, 1, 0][i];
        let along = |j: usize| {
            let axis = (quad[i] - quad[j]).normalize_or_zero();
            axis * delta.dot(axis)
        };
        let (h, v) = (along(horizontal), along(vertical));
        if h.length_squared() >= v.length_squared() {
            (h, horizontal)
        } else {
            (v, vertical)
        }
    };
    if corner && (mode == TransformMode::Perspective || (control && shift && alt)) {
        let (movement, partner) = paired_axis();
        let mut out = quad;
        out[i] += movement;
        out[partner] -= movement;
        return keep_convex(quad, out);
    }
    if corner && mode == TransformMode::Skew {
        let (movement, _) = paired_axis();
        let mut out = quad;
        out[i] += movement;
        if alt {
            out[(i + 2) % 4] -= movement
        }
        return keep_convex(quad, out);
    }
    if mode == TransformMode::Distort || (control && corner) {
        let mut out = quad;
        if corner {
            out[i] += delta;
        } else {
            out[i] += delta;
            out[(i + 1) % 4] += delta;
        }
        return keep_convex(quad, out);
    }
    if !corner && (mode == TransformMode::Skew || mode == TransformMode::Perspective || control) {
        let tangent = (quad[(i + 1) % 4] - quad[i]).normalize_or_zero();
        let movement = if shift || mode != TransformMode::Free && mode != TransformMode::Scale {
            tangent * delta.dot(tangent)
        } else {
            delta
        };
        let mut out = quad;
        out[i] += movement;
        out[(i + 1) % 4] += movement;
        if alt {
            out[(i + 2) % 4] -= movement;
            out[(i + 3) % 4] -= movement;
        }
        return keep_convex(quad, out);
    }
    let locations = handles(quad, 1.0);
    let anchor = if alt {
        center
    } else {
        locations[(hit + 4) % 8]
    };
    let x = quad[1] - quad[0];
    let y = quad[3] - quad[0];
    let determinant = x.perp_dot(y);
    if determinant.abs() < 0.0001 {
        return quad;
    }
    let local = |p: Vec2| Vec2::new(p.perp_dot(y) / determinant, x.perp_dot(p) / determinant);
    let before = local(locations[hit] - anchor);
    let after = local(locations[hit] + delta - anchor);
    let mut sx = if before.x.abs() > 0.00001 {
        after.x / before.x
    } else {
        1.0
    };
    let mut sy = if before.y.abs() > 0.00001 {
        after.y / before.y
    } else {
        1.0
    };
    if linked != shift {
        if corner {
            let diagonal = locations[hit] - anchor;
            let scale = (locations[hit] + delta - anchor).dot(diagonal)
                / diagonal.length_squared().max(0.00001);
            sx = scale;
            sy = scale;
        } else {
            if before.x.abs() > 0.00001 {
                sy = sx.abs()
            } else {
                sx = sy.abs()
            }
        }
    }
    let bounded = |v: f32| {
        if v < 0.0 {
            -v.abs().clamp(0.01, 256.0)
        } else {
            v.clamp(0.01, 256.0)
        }
    };
    sx = bounded(sx);
    sy = bounded(sy);
    quad.map(|p| {
        let q = local(p - anchor);
        anchor + x * q.x * sx + y * q.y * sy
    })
}

fn finish_selection(state: &mut State, id: usize) {
    let Some(draft) = state.selection.draft.take().filter(|d| d.id == id) else {
        return;
    };
    let mut params = json!({"mode":draft.mode.name(),"antiAlias":true,"sampleAllLayers":state.selection.sample_all});
    let [x0, y0, x1, y1] = draft.bounds;
    let x = x0.floor() as i32;
    let y = y0.floor() as i32;
    let width = (x1.ceil() as i32 - x).max(0);
    let height = (y1.ceil() as i32 - y).max(0);
    let command = match draft.tool {
        SelectionTool::Rectangle
        | SelectionTool::Ellipse
        | SelectionTool::Row
        | SelectionTool::Column => {
            if width == 0 || height == 0 {
                if draft.mode == Mode::Replace {
                    dispatch(state, id, SelectionAction::Command("select.deselect"));
                }
                return;
            }
            params["x"] = json!(x);
            params["y"] = json!(y);
            params["width"] = json!(width);
            params["height"] = json!(height);
            params["ellipse"] = json!(draft.tool == SelectionTool::Ellipse);
            if draft.tool == SelectionTool::Row {
                params["line"] = json!("row");
            }
            if draft.tool == SelectionTool::Column {
                params["line"] = json!("column");
            }
            "select.rect"
        }
        SelectionTool::Lasso | SelectionTool::Polygon => {
            if draft.points.len() < 3 {
                return;
            }
            params["points"] = json!(
                draft
                    .points
                    .iter()
                    .map(|p| p.to_array())
                    .collect::<Vec<_>>()
            );
            params["polygonal"] = json!(draft.tool == SelectionTool::Polygon);
            "select.lasso"
        }
        SelectionTool::Quick => {
            params["points"] = json!(
                draft
                    .points
                    .iter()
                    .map(|p| p.to_array())
                    .collect::<Vec<_>>()
            );
            params["size"] = json!(state.selection.quick_size.max(1));
            "select.quick"
        }
        SelectionTool::Object => {
            if width == 0 || height == 0 {
                params["point"] = json!(draft.hover.to_array());
            } else {
                params["rect"] = json!([x, y, width, height]);
            }
            "select.object"
        }
        SelectionTool::Wand => {
            params["x"] = json!(draft.hover.x);
            params["y"] = json!(draft.hover.y);
            params["tolerance"] = json!(state.selection.tolerance);
            params["contiguous"] = json!(!state.selection.noncontiguous);
            "select.magicWand"
        }
    };
    if let Some(session) = state.sessions.get_mut(&id)
        && let Err(error) = session.doc.layer_command(command, params)
    {
        state.error = Some(error);
    }
}

pub(super) fn pointer(
    viewer: &mut ImageViewer,
    editor: &mut Editor,
    rect: Rect,
    input_rect: Rect,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    focus: &mut InputFocus,
    samples: &[super::super::paint_input::Sample],
) -> bool {
    let Some(id) = editor.viewing_image else {
        return false;
    };
    let state = &viewer.edit;
    let selected = matches!(state.tool, PhotoTool::Select(_) | PhotoTool::Transform);
    if !selected && state.selection.transform.is_none() {
        return false;
    }
    if !state.editing(Some(id)) {
        return false;
    }
    if state.selection.menu != 0 || state.selection.dismissed {
        return true;
    }
    if viewer.panel.blocks_canvas()
        || state.busy()
        || state.menu
        || state.paint_menu
        || state.size_menu
        || state.color_picker.is_some()
        || state.layers.menu
        || state.layers.form.is_some()
    {
        return true;
    }
    if keys.pressed(KeyCode::Space) || mouse.pressed(MouseButton::Middle) {
        return false;
    }
    let Some(dimensions) = viewer.dimensions else {
        return true;
    };
    let image = viewer.image_rect(dimensions, rect);
    let scale = image.width() / dimensions.x as f32;
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let control = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let state = &mut viewer.edit;
    if state.tool == PhotoTool::Transform && state.selection.transform.is_none() {
        dispatch(state, id, SelectionAction::Transform);
    }
    let mut pending = None;
    for sample in samples {
        use super::super::paint_input::Sample;
        let (cursor, down, up) = match *sample {
            Sample::Pressure(_) => continue,
            Sample::Down(p) => (p, true, false),
            Sample::Move(p) => (p, false, false),
            Sample::Up(p) => (p, false, true),
        };
        if down && !input_rect.contains(cursor) {
            continue;
        }
        let point =
            ((cursor - image.min) / scale).clamp(Vec2::splat(-32768.0), Vec2::splat(32768.0));
        if !point.is_finite() {
            continue;
        }
        if let Some(transform) = state.selection.transform.as_mut().filter(|t| t.id == id) {
            if down {
                if let Some(hit) = hit(transform.quad, point, scale) {
                    transform.drag = Some(Drag {
                        hit,
                        start: point,
                        quad: transform.quad,
                    });
                    focus.clear();
                }
            }
            if let Some(drag) = &transform.drag {
                pending = Some(transformed_quad(
                    drag.quad,
                    drag.hit,
                    drag.start,
                    point,
                    state.selection.mode_transform,
                    !state.selection.unlinked,
                    shift,
                    alt,
                    control,
                ));
            }
            if up {
                transform.drag = None;
            }
            continue;
        }
        let PhotoTool::Select(tool) = state.tool else {
            continue;
        };
        if down {
            focus.clear();
            if tool == SelectionTool::Polygon
                && let Some(draft) = state.selection.draft.as_mut()
            {
                let close = draft.points.len() >= 3
                    && ((point - draft.points[0]).length() * scale < 8.0
                        || ((point - draft.hover).length() * scale < 6.0
                            && draft.last_click.elapsed().as_millis() < 400));
                if close {
                    finish_selection(state, id);
                    continue;
                }
                if draft.points.len() < 512 {
                    draft.points.push(point);
                }
                draft.last_click = std::time::Instant::now();
                draft.hover = point;
                continue;
            }
            if matches!(tool, SelectionTool::Wand | SelectionTool::Quick)
                && !Rect::from_corners(Vec2::ZERO, dimensions.as_vec2()).contains(point)
            {
                continue;
            }
            let mode = match (shift, alt) {
                (true, true) if tool != SelectionTool::Quick => Mode::Intersect,
                (true, true) => Mode::Subtract,
                (true, false) => Mode::Add,
                (false, true) => Mode::Subtract,
                _ => state.selection.mode,
            };
            state.selection.draft = Some(Draft {
                id,
                tool,
                points: vec![point],
                hover: point,
                mode,
                started_shift: shift,
                started_alt: alt,
                bounds: [point.x, point.y, point.x, point.y],
                last_click: std::time::Instant::now(),
            });
        }
        if let Some(draft) = state.selection.draft.as_mut() {
            draft.hover = point;
            let start = draft.points.first().copied().unwrap_or(point);
            let mut delta = point - start;
            if shift && !draft.started_shift {
                let side = delta.abs().max_element();
                delta = Vec2::new(delta.x.signum() * side, delta.y.signum() * side);
            }
            let other = start + delta;
            let low = if alt && !draft.started_alt {
                start - delta.abs()
            } else {
                start.min(other)
            };
            let high = if alt && !draft.started_alt {
                start + delta.abs()
            } else {
                start.max(other)
            };
            draft.bounds = [low.x, low.y, high.x, high.y];
            if matches!(tool, SelectionTool::Lasso | SelectionTool::Quick)
                && draft
                    .points
                    .last()
                    .is_none_or(|p| p.distance(point) * scale >= 1.5)
            {
                if draft.points.len() >= 512 {
                    draft.points = draft.points.iter().step_by(2).copied().collect();
                }
                draft.points.push(point);
            }
            if tool == SelectionTool::Row {
                draft.bounds = [
                    0.0,
                    point.y.floor(),
                    dimensions.x as f32,
                    point.y.floor() + 1.0,
                ];
            }
            if tool == SelectionTool::Column {
                draft.bounds = [
                    point.x.floor(),
                    0.0,
                    point.x.floor() + 1.0,
                    dimensions.y as f32,
                ];
            }
        }
        if (up && tool != SelectionTool::Polygon) || (down && tool == SelectionTool::Wand) {
            finish_selection(state, id);
        }
    }
    if let Some(quad) = pending {
        preview(state, quad, true);
    }
    if !mouse.pressed(MouseButton::Left)
        && let Some(transform) = state.selection.transform.as_mut()
    {
        transform.drag = None;
    }
    if !mouse.pressed(MouseButton::Left) {
        release(state);
    }
    if let Some(tab) = editor.image_tabs.iter_mut().find(|t| t.id == id) {
        tab.dirty = state.sessions.get(&id).is_some_and(|s| s.doc.dirty())
            || state
                .selection
                .transform
                .as_ref()
                .is_some_and(|t| t.id == id && t.deferred);
    }
    true
}

pub fn shortcuts(
    keys: &ButtonInput<KeyCode>,
    viewer: &ImageViewer,
    actions: &mut MessageWriter<crate::editor::Action>,
) -> bool {
    let state = &viewer.edit;
    if state.color_picker.is_some() || state.layers.form.is_some() || state.layers.menu {
        return false;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let action = if keys.just_pressed(KeyCode::Escape)
        && (state.selection.draft.is_some()
            || state.selection.transform.is_some()
            || state.selection.menu != 0)
    {
        Some(SelectionAction::Cancel)
    } else if keys.just_pressed(KeyCode::Enter) && state.selection.transform.is_some() {
        Some(SelectionAction::Apply)
    } else if keys.just_pressed(KeyCode::Enter)
        && state
            .selection
            .draft
            .as_ref()
            .is_some_and(|d| d.tool == SelectionTool::Polygon)
    {
        Some(SelectionAction::FinishPolygon)
    } else if keys.just_pressed(KeyCode::Backspace) && state.selection.draft.is_some() {
        Some(SelectionAction::RemovePoint)
    } else if command {
        if keys.just_pressed(KeyCode::KeyT) {
            Some(SelectionAction::Transform)
        } else if keys.just_pressed(KeyCode::KeyA) {
            Some(SelectionAction::Command("select.all"))
        } else if keys.just_pressed(KeyCode::KeyD) {
            Some(SelectionAction::Command("select.deselect"))
        } else if shift && keys.just_pressed(KeyCode::KeyI) {
            Some(SelectionAction::Command("select.inverse"))
        } else if keys.just_pressed(KeyCode::KeyZ) && state.selection.transform.is_some() {
            Some(SelectionAction::Cancel)
        } else {
            None
        }
    } else if keys.just_pressed(KeyCode::Delete)
        && state
            .document(viewer.tab)
            .is_some_and(|d| d.document.selection.is_some())
    {
        Some(SelectionAction::Command("edit.clear"))
    } else if keys.just_pressed(KeyCode::KeyM) {
        Some(SelectionAction::Tool(
            if shift && state.tool == PhotoTool::Select(SelectionTool::Rectangle) {
                SelectionTool::Ellipse
            } else {
                SelectionTool::Rectangle
            },
        ))
    } else if keys.just_pressed(KeyCode::KeyL) {
        Some(SelectionAction::Tool(
            if shift && state.tool == PhotoTool::Select(SelectionTool::Lasso) {
                SelectionTool::Polygon
            } else {
                SelectionTool::Lasso
            },
        ))
    } else if keys.just_pressed(KeyCode::KeyW) {
        Some(SelectionAction::Tool(
            if shift && state.tool == PhotoTool::Select(SelectionTool::Wand) {
                SelectionTool::Quick
            } else {
                SelectionTool::Wand
            },
        ))
    } else if state.selection.transform.is_some()
        || (state.tool == PhotoTool::Move
            && state
                .document(viewer.tab)
                .is_some_and(|d| d.document.selection.is_some()))
    {
        let step = if shift { 10 } else { 1 };
        if keys.just_pressed(KeyCode::ArrowLeft) {
            Some(SelectionAction::Nudge(-step, 0))
        } else if keys.just_pressed(KeyCode::ArrowRight) {
            Some(SelectionAction::Nudge(step, 0))
        } else if keys.just_pressed(KeyCode::ArrowUp) {
            Some(SelectionAction::Nudge(0, -step))
        } else if keys.just_pressed(KeyCode::ArrowDown) {
            Some(SelectionAction::Nudge(0, step))
        } else {
            None
        }
    } else {
        None
    };
    if let Some(action) = action {
        actions.write(crate::editor::Action::ImageEdit(EditAction::Selection(
            action,
        )));
        true
    } else {
        false
    }
}
