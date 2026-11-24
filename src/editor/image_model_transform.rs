//! Image-canvas handles for a reference model's world transform.
use super::*;
use crate::editor::image_reference::{ModelReference, ReferenceRender};
use bevy::{math::Rot2, ui::UiTransform};
use photocraft_doc::{DocId, LayerId};

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
use crate::editor::viewport::{AXIS_COLORS, ROTATION_COLORS};
#[path = "image_rotation_rings.rs"]
mod rotation_rings;

const RADIUS: f32 = 64.0;
const RING_STEPS: usize = 32;
const FREE: usize = 3;
const FREE_RADIUS: f32 = 82.0;
const PLANE_START: usize = 4;
const HANDLE_COUNT: usize = 7;
const HIGHLIGHT: Color = Color::srgb(1.0, 0.81, 0.38);

fn coordinates(vector: Vec2, u: Vec2, v: Vec2) -> Option<Vec2> {
    let determinant = u.perp_dot(v);
    (determinant.abs() > 0.12)
        .then(|| Vec2::new(vector.perp_dot(v), u.perp_dot(vector)) / determinant)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Binding {
    tab: usize,
    photo: DocId,
    layer: LayerId,
    revision: u64,
}

use super::edit::selection::{
    frame_quad,
    live::{Live, Placement},
};

/// Cache the surrounding layers. Translation reuses authored pixels, while rotation
/// and scale composite the prepared mesh directly from its offscreen camera.
pub(in crate::editor) struct LayerPreview {
    bound: Binding,
    view_revision: u64,
    live: Option<Live>,
    frame: [f32; 4],
    offset: Vec2,
    waiting_for_mesh: bool,
    model: ModelReference,
    size: UVec2,
    render_3d: bool,
    requested: Option<crate::Pose>,
    camera_image: Option<(Handle<Image>, [f32; 4])>,
}

impl LayerPreview {
    fn new(
        bound: Binding,
        photo: &crate::editor::photo_document::PhotoDocument,
        render: &ReferenceRender,
        model: &ModelReference,
        tool: Tool,
    ) -> Self {
        let size = UVec2::new(photo.size().width, photo.size().height);
        let mut frame = [0.0; 4];
        let mut waiting_for_mesh = false;
        let render_3d = matches!(tool, Tool::Rotate | Tool::Scale);
        let live = photo.active_layer().and_then(|layer| {
            if render_3d {
                // Under/above are static. The live camera supplies the moving plane.
                frame = [0., 0., 1., 1.];
                return Live::for_model(photo, frame, layer.id, None);
            }
            let mut rect = layer.surface()?.content_bounds();
            let clipped = rect.is_empty()
                || rect.x0 <= 0
                || rect.y0 <= 0
                || rect.x1 >= size.x as i32
                || rect.y1 >= size.y as i32;
            let extension = if clipped {
                if !render.live.mesh_ready(&model.asset) {
                    waiting_for_mesh = true;
                    return None;
                }
                let (full, extend) = render.live.extend_layer(model, size, rect)?;
                rect = full;
                Some(extend)
            } else {
                None
            };
            frame = [
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ];
            let live = Live::for_model(photo, frame, layer.id, extension)?;
            Some(live)
        });
        Self {
            bound,
            view_revision: photo.view_revision,
            live,
            frame,
            offset: Vec2::ZERO,
            waiting_for_mesh,
            render_3d,
            model: model.clone(),
            size,
            requested: None,
            camera_image: None,
        }
    }

    pub(in crate::editor) fn camera_request(
        &self,
    ) -> Option<(&ModelReference, UVec2, Option<crate::Pose>)> {
        (self.render_3d && self.live.is_some()).then_some((&self.model, self.size, self.requested))
    }
    pub(in crate::editor) fn set_camera_image(&mut self, image: Option<(Handle<Image>, [f32; 4])>) {
        self.camera_image = image;
    }
    fn request_pose(&mut self, pose: Option<crate::Pose>) {
        self.requested = pose.filter(|p| *p != self.model.pose);
        self.offset = self
            .requested
            .map_or(Vec2::ZERO, |p| projected_offset(&self.model, p));
        if self.requested.is_none() {
            self.camera_image = None;
        }
    }
    fn on_gpu(&self) -> bool {
        if self.render_3d && self.camera_image.is_none() {
            return false;
        }
        self.live
            .as_ref()
            .is_some_and(|live| live.ready_for_display())
    }

    pub(super) fn placement(&mut self) -> Option<Placement<'_>> {
        let camera = self
            .camera_image
            .as_ref()
            .filter(|_| self.requested.is_some());
        let frame = camera.map_or(self.frame, |(_, frame)| *frame);
        let deferred = if self.render_3d {
            camera.is_some()
        } else {
            self.offset != Vec2::ZERO
        };
        Some(Placement {
            live: self.live.as_mut()?,
            frame,
            quad: frame_quad(frame).map(|p| if self.render_3d { p } else { p + self.offset }),
            camera: camera.map(|(image, _)| image.clone()),
            deferred,
        })
    }
}

pub(super) struct Drag {
    bound: Binding,
    tool: Tool,
    axis: usize,
    origin: Vec2,
    direction: Vec2,
    sensitivity: f32,
    center: Vec2,
    view: Transform,
    initial: crate::Pose,
    pub(super) preview: crate::Pose,
}

struct Frame {
    model: ModelReference,
    center: Vec2,
    view: Transform,
    pixels_per_unit: f32,
}

impl Frame {
    fn new(model: ModelReference, image: Rect, dimensions: UVec2) -> Self {
        let pixels_per_unit = model.pixels_per_unit.max(1e-4) * image.width() / dimensions.x as f32;
        let center = image.min + model.canvas_center() / dimensions.as_vec2() * image.size();
        let view = model.view();
        Self {
            model,
            center,
            view,
            pixels_per_unit,
        }
    }

    fn project(&self, direction: Vec3) -> Vec2 {
        Vec2::new(
            direction.dot(*self.view.right()),
            -direction.dot(*self.view.up()),
        )
    }

    fn segments(&self, tool: Tool, axis: usize) -> Vec<(Vec2, Vec2)> {
        if axis >= PLANE_START {
            if tool != Tool::Move {
                return Vec::new();
            }
            let normal = axis - PLANE_START;
            let u = self.project(AXES[(normal + 1) % 3]);
            let v = self.project(AXES[(normal + 2) % 3]);
            if coordinates(Vec2::ZERO, u, v).is_none() {
                return Vec::new();
            }
            let corners = [
                Vec2::new(18.0, 18.0),
                Vec2::new(33.0, 18.0),
                Vec2::splat(33.0),
                Vec2::new(18.0, 33.0),
            ]
            .map(|p| self.center + u * p.x + v * p.y);
            return (0..4).map(|i| (corners[i], corners[(i + 1) % 4])).collect();
        }
        if tool == Tool::Rotate {
            return rotation_rings::segments(self, axis);
        }
        if axis == FREE {
            let radius = 8.0;
            if tool == Tool::Scale {
                let corners = [
                    Vec2::NEG_ONE,
                    Vec2::new(1.0, -1.0),
                    Vec2::ONE,
                    Vec2::new(-1.0, 1.0),
                ];
                return (0..4)
                    .map(|i| {
                        (
                            self.center + corners[i] * radius,
                            self.center + corners[(i + 1) % 4] * radius,
                        )
                    })
                    .collect();
            }
            let point = |i: usize| {
                self.center
                    + radius
                        * Vec2::from_angle(std::f32::consts::TAU * i as f32 / RING_STEPS as f32)
            };
            return (0..RING_STEPS).map(|i| (point(i), point(i + 1))).collect();
        }
        let direction = if tool == Tool::Scale {
            self.model.pose.transform().rotation * AXES[axis]
        } else {
            AXES[axis]
        };
        let projected = self.project(direction);
        let direction = if projected.length_squared() > 0.04 {
            projected.normalize()
        } else {
            Vec2::from_angle((axis as f32 * 2.0 + 1.0) * std::f32::consts::FRAC_PI_3)
        };
        vec![(
            self.center + direction * 12.0,
            self.center + direction * RADIUS,
        )]
    }

    fn hit(&self, tool: Tool, pointer: Vec2) -> Option<(usize, Vec2, f32)> {
        let distance = pointer.distance(self.center);
        if tool != Tool::Rotate && distance <= 11.0 {
            return Some((
                FREE,
                Vec2::new(1.0, -1.0).normalize(),
                if tool == Tool::Move {
                    self.pixels_per_unit.recip()
                } else {
                    RADIUS.recip()
                },
            ));
        }
        if tool == Tool::Rotate
            && let Some(hit) = rotation_rings::label_hit(self, pointer)
        {
            return Some(hit);
        }
        if tool == Tool::Rotate && (distance - FREE_RADIUS).abs() < 8.0 {
            return Some((FREE, Vec2::ZERO, 1.0));
        }
        if tool == Tool::Move {
            for normal in 0..3 {
                let u = self.project(AXES[(normal + 1) % 3]);
                let v = self.project(AXES[(normal + 2) % 3]);
                if let Some(point) = coordinates(pointer - self.center, u, v)
                    && point.cmpge(Vec2::splat(15.0)).all()
                    && point.cmple(Vec2::splat(35.0)).all()
                {
                    return Some((
                        PLANE_START + normal,
                        Vec2::ZERO,
                        self.pixels_per_unit.recip(),
                    ));
                }
            }
        }
        let mut best = None;
        let mut nearest = 8.0;
        for axis in 0..3 {
            for (a, b) in self.segments(tool, axis) {
                let segment = b - a;
                let length = segment.length();
                if length < 0.5 {
                    continue;
                }
                let along = segment / length;
                let distance =
                    pointer.distance(a + along * (pointer - a).dot(along).clamp(0.0, length));
                if distance < nearest {
                    nearest = distance;
                    let factor = match tool {
                        Tool::Move => {
                            1.0 / (self.pixels_per_unit
                                * self.project(AXES[axis]).length().max(0.25))
                        }
                        Tool::Rotate => (rotation_rings::ANGLE_STEP / length).min(0.04),
                        _ => 1.0 / RADIUS,
                    };
                    best = Some((axis, along, factor));
                }
            }
        }
        best
    }
}

#[cfg(test)]
pub(super) fn test_move_hit(
    viewer: &ImageViewer,
    tab: usize,
    canvas: Rect,
) -> impl Fn(Vec2) -> bool {
    let dimensions = viewer.dimensions.unwrap();
    let frame = Frame::new(
        viewer.edit.model_reference(Some(tab)).unwrap(),
        viewer.image_rect(dimensions, canvas),
        dimensions,
    );
    move |point| frame.hit(Tool::Move, point).is_some()
}

#[cfg(test)]
pub(super) fn test_handle(
    viewer: &ImageViewer,
    tab: usize,
    canvas: Rect,
    tool: Tool,
) -> (Vec2, Vec2) {
    let dimensions = viewer.dimensions.unwrap();
    let frame = Frame::new(
        viewer.edit.model_reference(Some(tab)).unwrap(),
        viewer.image_rect(dimensions, canvas),
        dimensions,
    );
    for (a, b) in frame.segments(tool, 0) {
        let point = (a + b) * 0.5;
        if canvas.contains(point)
            && let Some((0, direction, _)) = frame.hit(tool, point)
        {
            return (point, point + direction * 34.0);
        }
    }
    panic!("No visible X handle");
}

impl Drag {
    fn update(&mut self, pointer: Vec2, snap: bool) {
        if pointer.distance_squared(self.origin) < 1.0 {
            self.preview = self.initial;
            return;
        }
        let amount = (pointer - self.origin).dot(self.direction) * self.sensitivity;
        let index = self.axis;
        let mut transform = self.initial.transform();
        if index >= PLANE_START {
            let u = AXES[(index - PLANE_START + 1) % 3];
            let v = AXES[(index - PLANE_START + 2) % 3];
            let project =
                |axis: Vec3| Vec2::new(axis.dot(*self.view.right()), -axis.dot(*self.view.up()));
            if let Some(delta) = coordinates(pointer - self.origin, project(u), project(v)) {
                let mut delta = delta * self.sensitivity;
                if snap {
                    delta = (delta * 4.0).round() * 0.25;
                }
                let displacement = u * delta.x + v * delta.y;
                transform.translation = (transform.translation + displacement)
                    .clamp(Vec3::splat(-100_000.0), Vec3::splat(100_000.0));
            }
            self.preview = transform.into();
            return;
        }
        if index == FREE {
            let delta = pointer - self.origin;
            match self.tool {
                Tool::Move => {
                    let mut offset = delta * self.sensitivity;
                    if snap {
                        offset = (offset * 4.0).round() * 0.25;
                    }
                    let displacement = *self.view.right() * offset.x - *self.view.up() * offset.y;
                    transform.translation = (transform.translation + displacement)
                        .clamp(Vec3::splat(-100_000.0), Vec3::splat(100_000.0));
                }
                Tool::Rotate => {
                    let a = self.origin - self.center;
                    let b = pointer - self.center;
                    let mut angle = -a.perp_dot(b).atan2(a.dot(b));
                    let step = 15.0_f32.to_radians();
                    if snap {
                        angle = (angle / step).round() * step;
                    }
                    transform.rotation =
                        Quat::from_axis_angle(*self.view.back(), angle) * transform.rotation;
                }
                Tool::Scale => {
                    let mut factor = amount.clamp(-4.0, 4.0).exp();
                    if snap {
                        factor = (factor * 10.0).round().max(1.0) * 0.1;
                    }
                    let minimum = 0.0001 / transform.scale.abs().min_element();
                    let maximum = 1000.0 / transform.scale.abs().max_element();
                    transform.scale *= factor.clamp(minimum.min(maximum), maximum);
                }
                Tool::Universal => {}
            }
            self.preview = transform.into();
            return;
        }
        match self.tool {
            Tool::Move => {
                let amount = if snap {
                    (amount * 4.0).round() * 0.25
                } else {
                    amount
                };
                transform.translation[index] =
                    (transform.translation[index] + amount).clamp(-100_000.0, 100_000.0);
            }
            Tool::Rotate => {
                let step = 15.0_f32.to_radians();
                let project = |axis: Vec3| {
                    Vec2::new(axis.dot(*self.view.right()), -axis.dot(*self.view.up()))
                };
                let u = project(AXES[(index + 1) % 3]);
                let v = project(AXES[(index + 2) % 3]);
                let amount = coordinates(self.origin - self.center, u, v)
                    .zip(coordinates(pointer - self.center, u, v))
                    .map_or(amount, |(start, end)| {
                        start.perp_dot(end).atan2(start.dot(end))
                    });
                let amount = if snap {
                    (amount / step).round() * step
                } else {
                    amount
                };
                transform.rotation =
                    Quat::from_axis_angle(AXES[index], amount) * transform.rotation;
            }
            Tool::Scale => {
                let scale = transform.scale[index];
                let magnitude = scale.abs() * (1.0 + amount);
                let magnitude = if snap {
                    (magnitude * 10.0).round() * 0.1
                } else {
                    magnitude
                };
                transform.scale[index] = scale.signum() * magnitude.clamp(0.0001, 1000.0);
            }
            Tool::Universal => {}
        }
        self.preview = transform.into();
    }
}

fn silhouette(viewer: &ImageViewer, tab: usize, canvas: Rect, cursor: Vec2) -> Option<()> {
    let size = viewer.dimensions?;
    let image = viewer.image_rect(size, canvas);
    if !image.contains(cursor) {
        return None;
    }
    let point = ((cursor - image.min) / image.size() * size.as_vec2())
        .floor()
        .as_ivec2();
    let layer = viewer.edit.document(Some(tab))?.active_layer()?;
    let photocraft_doc::LayerContent::Raster(surface) = &layer.content else {
        return None;
    };
    (layer.visible
        && layer.opacity * layer.fill_opacity > 0.0
        && surface.sample_channel(point.x, point.y, 3) > 0.0
        && layer
            .mask
            .as_ref()
            .is_none_or(|m| m.value(point.x, point.y) > 0.0))
    .then_some(())
}

fn target(
    frame: &Frame,
    tool: Tool,
    viewer: &ImageViewer,
    tab: usize,
    canvas: Rect,
    cursor: Vec2,
) -> Option<(usize, Vec2, f32)> {
    frame.hit(tool, cursor).or_else(|| {
        if tool != Tool::Move {
            return None;
        }
        silhouette(viewer, tab, canvas, cursor)?;
        Some((FREE, Vec2::ZERO, frame.pixels_per_unit.recip()))
    })
}

fn projected_offset(model: &ModelReference, pose: crate::Pose) -> Vec2 {
    let delta = Vec3::from(pose.position) - Vec3::from(model.pose.position);
    let view = model.view();
    Vec2::new(delta.dot(*view.right()), -delta.dot(*view.up())) * model.pixels_per_unit
}

pub(super) fn pointer(
    viewer: &mut ImageViewer,
    editor: &mut Editor,
    references: &mut ReferenceRender,
    canvas: Rect,
    input: Rect,
    window: &Window,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    over_control: bool,
) -> bool {
    let context = (|| {
        let tab = editor.viewing_image?;
        let tool = viewer.edit.model_tool?;
        let model = viewer
            .edit
            .model_reference(Some(tab))?
            .resolved(&editor.root);
        let photo = viewer.edit.document(Some(tab))?;
        let dimensions = viewer.dimensions?;
        let model = references.live.calibrate(model, dimensions)?;
        (viewer.edit.editing(Some(tab))
            && !viewer.panel.blocks_canvas()
            && viewer.edit.color_picker.is_none()
            && !viewer.edit.layers.menu
            && viewer.edit.layers.form.is_none())
        .then(|| {
            (
                Binding {
                    tab,
                    photo: photo.document.id,
                    layer: photo.active_id().unwrap(),
                    revision: photo.revision,
                },
                tool,
                Frame::new(model, viewer.image_rect(dimensions, canvas), dimensions),
            )
        })
    })();
    let Some((bound, tool, frame)) = context else {
        viewer.released_model = None;
        viewer.layer_preview = None;
        viewer.clear_model_preview();
        references.live.reset();
        return viewer.model_drag.take().is_some();
    };
    {
        let photo = viewer.edit.document(Some(bound.tab)).unwrap();
        if viewer.layer_preview.as_ref().is_none_or(|cache| {
            cache.bound != bound
                || cache.render_3d != (tool != Tool::Move)
                || cache.model.asset != frame.model.asset
                || cache.view_revision != photo.view_revision
                || (cache.waiting_for_mesh && references.live.mesh_ready(&frame.model.asset))
        }) {
            viewer.layer_preview = Some(LayerPreview::new(
                bound,
                photo,
                references,
                &frame.model,
                tool,
            ));
        }
        if let Some(live) = viewer.layer_preview.as_mut().and_then(|m| m.live.as_mut()) {
            live.poll(bound.revision);
        }
    }
    let released = viewer
        .released_model
        .filter(|(binding, _)| *binding == bound)
        .and_then(|(_, pose)| {
            (references.pending_pose(bound.tab, bound.layer) == Some(pose)).then_some(pose)
        });
    if viewer.model_drag.is_none() && released.is_none() {
        if viewer.released_model.take().is_some() {
            references.live.reset();
        }
        if let Some(cache) = &mut viewer.layer_preview {
            cache.request_pose(None);
        }
        viewer.clear_model_preview();
    }
    if (keys.just_pressed(KeyCode::Escape) || keys.pressed(KeyCode::Space))
        && viewer.model_drag.is_some()
    {
        if let Some(cache) = &mut viewer.layer_preview {
            cache.request_pose(None);
        }
        viewer.clear_model_preview();
        references.live.reset();
        return viewer.model_drag.take().is_some();
    }
    if let Some(pose) = released
        && let Some(cache) = &mut viewer.layer_preview
    {
        cache.request_pose(Some(pose));
    }
    if viewer.model_drag.is_none()
        && let Some(pose) = released
        && viewer
            .layer_preview
            .as_ref()
            .is_none_or(|cache| !cache.on_gpu())
        && let Some(image) = references.live.poll(
            viewer.edit.document(Some(bound.tab)).unwrap(),
            &frame.model,
            pose,
            true,
        )
    {
        viewer.edit.display_model_preview(bound.tab, image);
        viewer.model_preview = Some((bound.tab, bound.revision));
    }
    if keys.pressed(KeyCode::Space) {
        return false;
    }
    if let Some(mut drag) = viewer.model_drag.take() {
        if drag.bound.photo != bound.photo
            || drag.bound.tab != bound.tab
            || drag.bound.layer != bound.layer
            || drag.bound.revision != bound.revision
            || drag.tool != tool
        {
            viewer.clear_model_preview();
            references.live.reset();
            return true;
        }
        if let Some(cursor) = window.cursor_position() {
            drag.update(
                cursor,
                keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            );
        }
        let on_gpu = if let Some(cache) = &mut viewer.layer_preview {
            cache.request_pose(Some(drag.preview));
            cache.on_gpu()
        } else {
            false
        };
        if mouse.just_released(MouseButton::Left) {
            let doc = viewer.edit.document(Some(bound.tab)).unwrap();
            if let Err(error) = references.transform_selected(bound.tab, doc, drag.preview) {
                viewer.clear_model_preview();
                viewer.edit.error = Some(error);
            } else if drag.preview != drag.initial {
                viewer.released_model = Some((bound, drag.preview));
            }
        } else if mouse.pressed(MouseButton::Left) {
            if drag.preview != drag.initial && !on_gpu {
                let doc = viewer.edit.document(Some(bound.tab)).unwrap();
                if let Some(image) = references.live.poll(doc, &frame.model, drag.preview, false) {
                    viewer.edit.display_model_preview(bound.tab, image);
                    viewer.model_preview = Some((bound.tab, bound.revision));
                }
            } else if drag.preview == drag.initial {
                viewer.clear_model_preview();
                references.live.reset();
            }
            viewer.model_drag = Some(drag);
        } else {
            viewer.clear_model_preview();
            references.live.reset();
        }
        return true;
    }
    if mouse.just_pressed(MouseButton::Left)
        && !over_control
        && let Some(cursor) = window.cursor_position().filter(|p| input.contains(*p))
        && !references.pending_for(Some(bound.tab))
        && let Some((axis, direction, sensitivity)) =
            target(&frame, tool, viewer, bound.tab, canvas, cursor)
    {
        references.live.reset();
        viewer.gesture = None;
        viewer.model_drag = Some(Drag {
            bound,
            tool,
            axis,
            origin: cursor,
            direction,
            sensitivity,
            center: frame.center,
            view: frame.view,
            initial: frame.model.pose,
            preview: frame.model.pose,
        });
    }
    // While a model tool is selected, a click on the canvas must not paint.
    mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left)
}

fn handle_hint(tool: Tool, axis: usize, drag: Option<&Drag>) -> String {
    let label = ["X", "Y", "Z", "холст", "YZ", "ZX", "XY"][axis];
    let name = match (tool, axis) {
        (Tool::Move, _) => "Перемещение",
        (Tool::Rotate, _) => "Поворот",
        (Tool::Scale, FREE) => "Равномерный масштаб",
        (Tool::Scale, _) => "Масштаб",
        _ => return String::new(),
    };
    let Some(drag) = drag else {
        return if tool == Tool::Scale && axis == FREE {
            format!("{name} · Shift — шаг")
        } else {
            format!("{name} · {label} · Shift — шаг")
        };
    };
    let before = drag.initial.transform();
    let after = drag.preview.transform();
    let value = match tool {
        Tool::Move if axis < 3 => format!("{:+.2}", (after.translation - before.translation)[axis]),
        Tool::Move => {
            let v = after.translation - before.translation;
            format!("X {:+.2}  Y {:+.2}  Z {:+.2}", v.x, v.y, v.z)
        }
        Tool::Scale => format!(
            "×{:.2}",
            after.scale[axis.min(2)] / before.scale[axis.min(2)]
        ),
        Tool::Rotate => {
            let mut q = after.rotation * before.rotation.inverse();
            if q.w < 0.0 {
                q = -q;
            }
            let direction = if axis == FREE {
                *drag.view.back()
            } else {
                AXES[axis]
            };
            format!(
                "{:+.1}°",
                (2.0 * q.xyz().dot(direction).atan2(q.w)).to_degrees()
            )
        }
        _ => String::new(),
    };
    format!("{label}  {value}").replace('.', ",")
}

pub(super) struct Overlay {
    hint: Entity,
    hint_text: Entity,
    hint_value: String,
    nodes: Vec<Entity>,
    rings: rotation_rings::Overlay,
    last: Option<(Tool, Vec2, Vec3, [f32; 4], Option<usize>, bool)>,
}

impl Overlay {
    #[cfg(test)]
    pub(super) fn handle_geometry(&self) -> (Vec2, [f32; 4]) {
        let (_, center, _, rotation, _, _) = self.last.unwrap();
        (center, rotation)
    }

    pub(super) fn setup(commands: &mut Commands, canvas: Entity, font: &Handle<Font>) -> Self {
        let hint = surface(
            commands,
            canvas,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                padding: UiRect::axes(px(7), px(4)),
                border_radius: BorderRadius::all(px(4)),
                overflow: Overflow::clip(),
                ..default()
            },
            Color::srgba(0.06, 0.07, 0.09, 0.96),
        );
        commands.entity(hint).insert((Pickable::IGNORE, ZIndex(14)));
        let hint_text = text(commands, hint, font, "", 12.0, Color::WHITE);
        commands.entity(hint_text).insert(Pickable::IGNORE);
        let nodes = (0..3 * 5 + RING_STEPS + (HANDLE_COUNT - PLANE_START) * 4)
            .map(|_| {
                commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            display: Display::None,
                            border_radius: BorderRadius::all(px(2)),
                            ..default()
                        },
                        UiTransform::IDENTITY,
                        Outline::new(px(1), px(0), Color::srgba(0.02, 0.02, 0.02, 0.9)),
                        BackgroundColor(Color::NONE),
                        Pickable::IGNORE,
                        ZIndex(12),
                        ChildOf(canvas),
                    ))
                    .id()
            })
            .collect();
        Self {
            hint,
            hint_text,
            hint_value: String::new(),
            rings: rotation_rings::Overlay::setup(commands, canvas, font),
            nodes,
            last: None,
        }
    }

    pub(super) fn refresh(
        &mut self,
        editor: &Editor,
        viewer: &ImageViewer,
        references: &ReferenceRender,
        viewport: Rect,
        pointer: Option<Vec2>,
        density: f32,
        images: &mut Assets<Image>,
        nodes: &mut Query<&mut Node>,
        commands: &mut Commands,
    ) {
        let geometry = (|| {
            let tab = editor.viewing_image?;
            let tool = viewer.edit.model_tool?;
            let dimensions = viewer.dimensions?;
            let model = viewer.edit.model_reference(Some(tab))?;
            let mut model = if model.pixels_per_unit == 0.0 {
                references
                    .live
                    .calibrate(model.resolved(&editor.root), dimensions)?
            } else {
                model
            };
            if !viewer.edit.editing(Some(tab)) || editor.pending.is_some() {
                return None;
            }
            let layer = viewer.edit.document(Some(tab))?.active_id()?;
            if let Some(pose) = viewer
                .model_drag
                .as_ref()
                .filter(|drag| drag.bound.tab == tab && drag.bound.layer == layer)
                .map(|drag| drag.preview)
                .or_else(|| references.pending_pose(tab, layer))
            {
                model.pose = pose;
            }
            let frame = Frame::new(model, viewer.image_rect(dimensions, viewport), dimensions);
            viewport.contains(frame.center).then_some((tool, frame))
        })();
        let dragging = viewer.model_drag.is_some();
        let highlight = viewer
            .model_drag
            .as_ref()
            .map(|drag| drag.axis)
            .or_else(|| {
                let (tool, frame) = geometry.as_ref()?;
                target(
                    frame,
                    *tool,
                    viewer,
                    editor.viewing_image?,
                    viewport,
                    pointer.filter(|point| viewport.contains(*point))?,
                )
                .map(|hit| hit.0)
            });
        let rotating = geometry
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::Rotate)
            .map(|(_, frame)| frame);
        self.rings.refresh(
            rotating,
            viewport.min,
            highlight,
            dragging,
            density,
            images,
            nodes,
            commands,
        );
        if let Ok(mut node) = nodes.get_mut(self.hint) {
            if let (Some(axis), Some((tool, _)), Some(cursor)) = (highlight, &geometry, pointer) {
                let value = handle_hint(*tool, axis, viewer.model_drag.as_ref());
                let width = (value.chars().count() as f32 * 8.0 + 16.0).min(viewport.width());
                let pos = (cursor - viewport.min + Vec2::splat(14.0)).clamp(
                    Vec2::ZERO,
                    (viewport.size() - Vec2::new(width, 25.0)).max(Vec2::ZERO),
                );
                node.display = Display::Flex;
                node.max_width = px(width);
                node.left = px(pos.x);
                node.top = px(pos.y);
                if value != self.hint_value {
                    commands.entity(self.hint_text).insert(Text::new(&value));
                    self.hint_value = value;
                }
            } else {
                node.display = Display::None;
            }
        }
        let key = geometry.as_ref().map(|(tool, frame)| {
            (
                *tool,
                frame.center - viewport.min,
                *frame.view.back(),
                frame.model.pose.rotation,
                highlight,
                dragging,
            )
        });
        if self.last == key {
            return;
        }
        let had_segments = self.last.is_some_and(|(tool, ..)| tool != Tool::Rotate);
        self.last = key;
        if geometry
            .as_ref()
            .is_none_or(|(tool, _)| *tool == Tool::Rotate)
        {
            if had_segments {
                for &entity in &self.nodes {
                    if let Ok(mut node) = nodes.get_mut(entity) {
                        node.display = Display::None;
                    }
                }
            }
            return;
        }
        for axis in 0..HANDLE_COUNT {
            let mut segments = geometry
                .as_ref()
                .map(|(tool, frame)| frame.segments(*tool, axis))
                .unwrap_or_default();
            let mut color = Color::srgb(0.88, 0.89, 0.92);
            if let Some((tool, _)) = &geometry {
                if axis >= PLANE_START {
                    color = AXIS_COLORS[axis - PLANE_START].with_alpha(0.75);
                } else if axis != FREE {
                    color = AXIS_COLORS[axis];
                    if let Some(&(start, end)) = segments.first() {
                        let direction = (end - start).normalize();
                        let side = direction.perp() * 5.0;
                        let base = end - direction * 11.0;
                        if *tool == Tool::Move {
                            segments.extend([
                                (end, base + side),
                                (end, base - side),
                                (base + side, base - side),
                            ]);
                        } else {
                            let back = end - direction * 10.0;
                            segments.extend([
                                (end + side, end - side),
                                (end + side, back + side),
                                (end - side, back - side),
                                (back - side, back + side),
                            ]);
                        }
                    }
                }
            }
            if highlight == Some(axis) {
                color = HIGHLIGHT;
            } else if dragging {
                color = color.with_alpha(0.3);
            }
            let range = if axis < FREE {
                axis * 5..(axis + 1) * 5
            } else if axis == FREE {
                15..15 + RING_STEPS
            } else {
                let start = 15 + RING_STEPS + (axis - PLANE_START) * 4;
                start..start + 4
            };
            for (index, &entity) in self.nodes[range].iter().enumerate() {
                let Ok(mut node) = nodes.get_mut(entity) else {
                    continue;
                };
                let Some(&(start, end)) = segments.get(index) else {
                    node.display = Display::None;
                    continue;
                };
                let midpoint = (start + end) * 0.5 - viewport.min;
                let vector = end - start;
                let length = vector.length();
                node.display = Display::Flex;
                node.left = px(midpoint.x - length * 0.5);
                node.top = px(midpoint.y - 1.5);
                node.width = px(length + 1.0);
                node.height = px(3);
                commands.entity(entity).insert((
                    BackgroundColor(color),
                    UiTransform::from_rotation(Rot2::radians(vector.y.atan2(vector.x))),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(tool: Tool, axis: usize, origin: Vec2) -> Drag {
        let pose = Transform::IDENTITY.into();
        Drag {
            bound: Binding {
                tab: 0,
                photo: DocId::fresh(),
                layer: LayerId::fresh(),
                revision: 0,
            },
            tool,
            axis,
            origin,
            direction: Vec2::X,
            sensitivity: 0.01,
            center: Vec2::ZERO,
            view: Transform::IDENTITY,
            initial: pose,
            preview: pose,
        }
    }

    #[test]
    fn model_transform_free_and_plane_handles_constrain_the_correct_components() {
        let mut free = drag(Tool::Move, FREE, Vec2::ZERO);
        free.update(Vec2::new(50.0, -100.0), false);
        assert_eq!(free.preview.position, [0.5, 1.0, 0.0]);

        free.view = Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.8, 0.3, 0.0));
        free.update(Vec2::new(50.0, -100.0), true);
        assert!(
            Vec3::from(free.preview.position)
                .dot(*free.view.back())
                .abs()
                < 1e-5
        );
        free.view = Transform::IDENTITY;
        free.update(Vec2::new(50.0, -100.0), true);
        let mut plane = drag(Tool::Move, PLANE_START + 2, Vec2::ZERO);
        plane.update(Vec2::new(50.0, -100.0), false);
        assert_eq!(plane.preview.position, free.preview.position);

        let mut uniform = drag(Tool::Scale, FREE, Vec2::ZERO);
        uniform.update(Vec2::new(30.0, 0.0), false);
        assert!(uniform.preview.scale[0] > 1.0);
        assert_eq!(uniform.preview.scale, [uniform.preview.scale[0]; 3]);

        let mut ring = drag(Tool::Rotate, FREE, Vec2::X * FREE_RADIUS);
        ring.update(Vec2::Y * FREE_RADIUS, false);
        assert!((ring.preview.transform().rotation * Vec3::X).abs_diff_eq(Vec3::NEG_Y, 1e-5));
    }
}
