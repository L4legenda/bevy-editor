//! Scene-style handles share the scene shader, ring geometry and hit regions.
use super::*;
use crate::editor::viewport::{self, TransformHandle as Handle};
use viewport::transform_handles::{HandleMaterial, ModelOverlay};

pub(in crate::editor) struct Frame {
    pub pivot: Vec3,
    pub rotation: Quat,
    pub length: f32,
    pub radius: f32,
    pub pose: Transform,
}
impl Frame {
    pub fn new(doc: &Document, viewer: &Viewer, rect: Rect, local: bool) -> Option<Self> {
        let pivot = doc.pivot()?;
        let pose = view_pose(viewer);
        let depth = (pivot - pose.translation).dot(*pose.forward());
        if !viewer.orthographic && depth <= 0.000001 {
            return None;
        }
        let pixel = view_height(viewer, depth) / rect.height();
        Some(Self {
            pivot,
            pose,
            rotation: if local {
                doc.active()
                    .map_or(Quat::IDENTITY, |o| o.transform.rotation)
            } else {
                Quat::IDENTITY
            },
            length: if viewer.orthographic {
                87.5 * pixel
            } else {
                (87.5 * pixel).clamp(0.06, 8.0)
            },
            radius: 88.0 * pixel,
        })
    }
    pub fn rings(&self, axis: usize) -> impl Iterator<Item = (Vec3, Vec3)> {
        viewport::universal_ring_segments(self.pivot, axis, self.radius, &self.pose, self.rotation)
    }
    pub fn plane(&self, normal: usize) -> [Vec3; 4] {
        viewport::plane_corners(self.pivot, normal, self.length, self.rotation)
    }
    pub fn tip(&self, mode: Tool, tool: Tool, axis: usize) -> Vec3 {
        self.pivot
            + self.rotation * viewport::AXES[axis] * viewport::axis_length(mode, tool, self.length)
    }
    pub fn hit(&self, viewer: &Viewer, rect: Rect, tool: Tool, cursor: Vec2) -> Option<Handle> {
        if tool.operations().is_empty() {
            return None;
        }
        let project = |p| input::project(viewer, rect, p);
        let center = project(self.pivot)?;
        if tool.operations().contains(&Tool::Move) && center.distance(cursor) < 9.0 {
            return Some(Handle::ViewTranslation);
        }
        if tool == Tool::Move {
            for normal in (0..3).rev() {
                if let [Some(a), Some(b), Some(c), Some(d)] = self.plane(normal).map(project) {
                    if viewport::quad_contains(cursor, [a, b, c, d]) {
                        return Some(Handle::Plane { normal });
                    }
                }
            }
        }
        let mut best = 9.0;
        let mut result = None;
        for &operation in tool.operations() {
            for axis in 0..3 {
                let distance = if operation == Tool::Rotate {
                    self.rings(axis)
                        .filter_map(|(a, b)| Some(input::segment(cursor, project(a)?, project(b)?)))
                        .fold(f32::INFINITY, f32::min)
                } else {
                    let Some(end) = project(self.tip(tool, operation, axis)) else {
                        continue;
                    };
                    if end.distance(center) < 13.0 {
                        continue;
                    }
                    input::segment(cursor, center.lerp(end, 0.23), end)
                };
                if distance < best {
                    best = distance;
                    result = Some(Handle::Axis {
                        tool: operation,
                        axis,
                    });
                }
            }
        }
        if tool.operations().contains(&Tool::Rotate) {
            if let Some(edge) = project(self.pivot + *self.pose.right() * self.radius * 1.15) {
                let distance = (cursor.distance(center) - edge.distance(center)).abs();
                if distance < 5.0 && distance < best {
                    result = Some(Handle::ViewRotation);
                }
            }
        }
        result
    }
}

pub(in crate::editor) fn sync(
    state: Res<State>,
    editor: Res<Editor>,
    viewer: Res<Viewer>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    overlays: Query<&MaterialNode<HandleMaterial>, With<ModelOverlay>>,
    mut materials: ResMut<Assets<HandleMaterial>>,
) {
    for overlay in &overlays {
        let Some(mut material) = materials.get_mut(&overlay.0) else {
            continue;
        };
        *material = HandleMaterial::default();
        if editor.pending.is_some()
            || editor.viewing_image.is_some()
            || state.tool.operations().is_empty()
            || state.extrude_tool
            || state.gesture.as_ref().is_some_and(|g| g.is_extruding())
        {
            continue;
        }
        let Some(doc) = editor.viewing_model.and_then(|i| state.documents.get(&i)) else {
            continue;
        };
        if matches!(doc.mode, Mode::Uv | Mode::Paint) {
            continue;
        }
        let Some(rect) = input::rect(&canvas) else {
            continue;
        };
        let Some(mut frame) = Frame::new(doc, &viewer, rect, state.local) else {
            continue;
        };
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
        let project = |p| input::project(&viewer, rect, p).map(|p| p - rect.min);
        let Some(center) = project(frame.pivot) else {
            continue;
        };
        let selected = state
            .gesture
            .as_ref()
            .and_then(|g| g.handle)
            .or(state.hover);
        let color = |handle, color: Color| {
            if selected == Some(handle) {
                Color::srgb(1.0, 0.81, 0.38)
            } else {
                color
            }
        };
        let mode = match state.tool {
            Tool::Universal => 3.0,
            Tool::Move => 0.0,
            Tool::Scale => 1.0,
            Tool::Rotate => 2.0,
        };
        material.viewport = rect.size().extend(1.0).extend(mode);
        material.pivot = center
            .extend(f32::from(state.tool.operations().contains(&Tool::Move)))
            .extend(f32::from(selected == Some(Handle::ViewTranslation)));
        for &operation in state.tool.operations() {
            if operation == Tool::Rotate {
                continue;
            }
            for axis in 0..3 {
                let tip = frame.tip(state.tool, operation, axis);
                let Some(end) = project(tip) else {
                    continue;
                };
                if end.distance(center) < 13.0 {
                    continue;
                }
                material.tips[axis] = end.extend(1.0).extend(0.0);
                material.colors[axis] = color(
                    Handle::Axis {
                        tool: operation,
                        axis,
                    },
                    viewport::AXIS_COLORS[axis],
                )
                .to_linear();
                if operation == Tool::Scale {
                    let eye = frame.rotation.inverse() * (frame.pose.translation - tip);
                    for face in 0..3 {
                        let normal =
                            viewport::AXES[face] * if eye[face] >= 0.0 { 1.0 } else { -1.0 };
                        let u = viewport::AXES[(face + 1) % 3];
                        let v = viewport::AXES[(face + 2) % 3];
                        for (corner, offset) in
                            [u + v, u - v, -u - v, -u + v].into_iter().enumerate()
                        {
                            if let Some(point) = project(
                                tip + frame.rotation * (normal + offset) * frame.length * 0.06,
                            ) {
                                material.faces[axis * 12 + face * 4 + corner] =
                                    point.extend(0.0).extend(0.0);
                            }
                        }
                    }
                }
            }
        }
        if state.tool == Tool::Move {
            for normal in 0..3 {
                if let [Some(a), Some(b), Some(c), Some(d)] = frame.plane(normal).map(project) {
                    if !viewport::quad_contains((a + b + c + d) * 0.25, [a, b, c, d]) {
                        continue;
                    }
                    for (i, p) in [a, b, c, d].into_iter().enumerate() {
                        material.planes[normal * 4 + i] = p.extend(1.0).extend(0.0);
                    }
                    material.plane_colors[normal] =
                        color(Handle::Plane { normal }, viewport::AXIS_COLORS[normal]).to_linear();
                }
            }
        }
        if state.tool.operations().contains(&Tool::Rotate)
            && let Some(end) = project(frame.pivot + *frame.pose.right() * frame.radius * 1.15)
        {
            material.outer_ring = Vec4::new(
                end.distance(center),
                f32::from(selected == Some(Handle::ViewRotation)),
                1.0,
                0.0,
            );
        }
    }
}
