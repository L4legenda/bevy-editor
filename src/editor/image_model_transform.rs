//! Image-canvas handles for a reference model's world transform.
use super::*;
use crate::editor::image_reference::{
    ModelReference, ModelView, ReferencePose, ReferenceRender,
    live::{CameraImage, RasterFrame, Shot},
    rig::{Effector, Reach, Rig, Rotations},
};
use photocraft_doc::{DocId, LayerId};
use std::sync::Arc;

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
use crate::editor::viewport::{
    self, AXIS_COLORS, ROTATION_COLORS,
    transform_handles::{HandleMaterial, ImageOverlay, scale_cube_offsets},
};
#[path = "image_rotation_rings.rs"]
mod rotation_rings;

const RADIUS: f32 = 64.0;
const HANDLE_LENGTH: f32 = 87.5;
const MOVE_AXES: [Vec2; 2] = [Vec2::X, Vec2::NEG_Y];
const FREE: usize = 3;
const FREE_RADIUS: f32 = 82.0;
const HIGHLIGHT: Color = Color::srgb(1.0, 0.81, 0.38);

fn coordinates(vector: Vec2, u: Vec2, v: Vec2) -> Option<Vec2> {
    let determinant = u.perp_dot(v);
    (determinant.abs() > 0.12)
        .then(|| Vec2::new(vector.perp_dot(v), u.perp_dot(vector)) / determinant)
}

fn project_to_canvas(view: &Transform, direction: Vec3) -> Vec2 {
    Vec2::new(direction.dot(*view.right()), -direction.dot(*view.up()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Binding {
    tab: usize,
    photo: DocId,
    layer: LayerId,
    revision: u64,
}

impl Binding {
    fn same_layer(self, other: Self) -> bool {
        (self.tab, self.photo, self.layer) == (other.tab, other.photo, other.layer)
    }
}

pub(super) struct BoneSelection {
    bound: Binding,
    joint: String,
}

struct BoneHandle<'a> {
    key: &'a str,
    name: &'a str,
    center: Vec2,
    depth: f32,
    basis: Quat,
    effector: Option<Effector>,
}

struct BoneView<'a> {
    rig: &'a Rig,
    model: Transform,
    camera: Transform,
    origin: Vec2,
    pixels_per_unit: Vec2,
}

impl<'a> BoneView<'a> {
    fn new(
        references: &'a ReferenceRender,
        shot: Option<&'a Shot>,
        model: &ModelReference,
        dimensions: UVec2,
        image: Rect,
    ) -> Option<Self> {
        let (rig, (model, center, camera, density)) = match shot {
            Some(shot) => (shot.rig(), shot.joint_view()),
            None => (
                references.live.rig()?,
                references.live.joint_view(model, dimensions)?,
            ),
        };
        let scale = image.size() / dimensions.as_vec2();
        Some(Self {
            rig,
            model,
            camera,
            origin: image.min + center * scale,
            pixels_per_unit: density * scale,
        })
    }

    fn markers(&self, rotations: &Rotations) -> Vec<BoneHandle<'a>> {
        let affine = self.model.compute_affine();
        self.rig
            .joints(rotations)
            .into_iter()
            .map(|joint| {
                let displacement =
                    affine.transform_point3(joint.position) - self.camera.translation;
                BoneHandle {
                    key: joint.key,
                    name: joint.name,
                    center: self.origin
                        + self.pixels_per_unit * project_to_canvas(&self.camera, displacement),
                    depth: displacement.dot(*self.camera.back()),
                    basis: self.model.rotation * joint.basis,
                    effector: joint.effector,
                }
            })
            .filter(|joint| joint.center.is_finite())
            .collect()
    }

    fn reach(&self, key: &str, rotations: &Rotations) -> Option<BoneDrag> {
        let inverse = self.model.compute_affine().inverse();
        let right = inverse.transform_vector3(*self.camera.right()) / self.pixels_per_unit.x;
        let down = inverse.transform_vector3(-*self.camera.up()) / self.pixels_per_unit.y;
        if !right.is_finite() || !down.is_finite() {
            return None;
        }
        Some(BoneDrag::Reach {
            limb: Box::new(self.rig.reach(key, rotations)?),
            right,
            down,
            delta: Vec2::ZERO,
        })
    }
}

fn bone_at(markers: &[BoneHandle<'_>], pointer: Vec2, input: Rect) -> Option<usize> {
    markers
        .iter()
        .enumerate()
        .filter(|(_, joint)| {
            input.contains(joint.center) && pointer.distance_squared(joint.center) <= 81.0
        })
        .min_by(|(_, a), (_, b)| {
            // The larger wrist handle remains grabbable among the finger dots.
            let grip =
                |j: &BoneHandle| j.effector.is_some() && pointer.distance_squared(j.center) <= 36.0;
            grip(b)
                .cmp(&grip(a))
                .then_with(|| {
                    pointer
                        .distance_squared(a.center)
                        .total_cmp(&pointer.distance_squared(b.center))
                })
                .then_with(|| b.depth.total_cmp(&a.depth))
        })
        .map(|(index, _)| index)
}

fn effector_hint(effector: Effector) -> &'static str {
    match effector {
        Effector::Hand => "Кисть · тяните точку",
        Effector::Foot => "Стопа · тяните точку",
    }
}

#[cfg(test)]
pub(super) fn test_bone_center(
    viewer: &ImageViewer,
    references: &ReferenceRender,
    tab: usize,
    canvas: Rect,
    name: &str,
) -> Vec2 {
    let dimensions = viewer.dimensions.unwrap();
    let model = viewer.edit.model_reference(Some(tab)).unwrap();
    let shot = viewer
        .layer_preview
        .as_ref()
        .and_then(LayerPreview::displayed_shot);
    BoneView::new(
        references,
        shot,
        &model,
        dimensions,
        viewer.image_rect(dimensions, canvas),
    )
    .unwrap()
    .markers(shot.map_or(&model.joints, |s| &s.view.pose.joints))
    .into_iter()
    .find(|joint| joint.name == name)
    .expect("The reference rig must have the test joint")
    .center
}

use super::edit::selection::{
    frame_quad,
    live::{Live, Placement},
};

/// Cache the surrounding layers. A fully visible model can reuse its authored
/// pixels during translation. A clipped model needs a fresh render of the whole
/// mesh so the newly visible area has the same shading as the rest of the model.
pub(in crate::editor) struct LayerPreview {
    bound: Binding,
    generation: u64,
    view_revision: u64,
    tool: Tool,
    live: Option<Live>,
    frame: [f32; 4],
    offset: Vec2,
    model: ModelReference,
    size: UVec2,
    render_3d: bool,
    requested: Option<ReferencePose>,
    camera_image: Option<CameraImage>,
    composing: Option<Task<(Arc<Shot>, photocraft_raster::Rgba8Image)>>,
    composing_id: Option<u64>,
    composite_shot: Option<Arc<Shot>>,
}

impl LayerPreview {
    fn new(
        bound: Binding,
        photo: &crate::editor::photo_document::PhotoDocument,
        model: &ModelReference,
        tool: Tool,
        generation: u64,
    ) -> Self {
        let size = UVec2::new(photo.size().width, photo.size().height);
        let raster_frame = photo
            .active_layer()
            .filter(|_| tool == Tool::Move)
            .and_then(|layer| layer.surface())
            .map(|surface| surface.content_bounds())
            .filter(|rect| {
                !rect.is_empty()
                    && rect.x0 > 0
                    && rect.y0 > 0
                    && rect.x1 < size.x as i32
                    && rect.y1 < size.y as i32
            })
            .map(|rect| {
                [
                    rect.x0 as f32,
                    rect.y0 as f32,
                    rect.x1 as f32,
                    rect.y1 as f32,
                ]
            });
        let render_3d = raster_frame.is_none();
        // For a live camera the surrounding layers are static. Only the camera's
        // frame is used to place the model in the final composition.
        let frame = raster_frame.unwrap_or([0., 0., 1., 1.]);
        let live = Live::for_model(photo, frame, bound.layer);
        Self {
            bound,
            generation,
            view_revision: photo.view_revision,
            tool,
            live,
            frame,
            offset: Vec2::ZERO,
            render_3d,
            model: model.clone(),
            size,
            requested: None,
            camera_image: None,
            composite_shot: None,
            composing: None,
            composing_id: None,
        }
    }

    pub(in crate::editor) fn camera_request(&self) -> Option<(ModelView, bool)> {
        let native = self.live.as_ref().is_none_or(|live| !live.usable());
        (self.render_3d || native).then(|| {
            (
                ModelView::new(
                    self.bound.photo,
                    Some(self.bound.layer),
                    &self.model,
                    self.size,
                    self.requested
                        .clone()
                        .unwrap_or_else(|| self.model.reference_pose()),
                ),
                native && self.requested.is_some(),
            )
        })
    }
    pub(in crate::editor) fn set_camera_image(&mut self, image: Option<CameraImage>) {
        self.camera_image = image;
    }

    fn composite(
        &mut self,
        photo: &crate::editor::photo_document::PhotoDocument,
        raster: Option<&RasterFrame>,
    ) -> Option<(Arc<Shot>, photocraft_raster::Rgba8Image)> {
        let completed = self
            .composing
            .as_mut()
            .and_then(|task| block_on(poll_once(task)));
        if completed.is_some() {
            self.composing = None;
        }
        if self.composing.is_none()
            && let Some(frame) = raster
            && self.composing_id != Some(frame.id())
            && let Some((view, _)) = self.camera_request()
            && frame.view().same_layer_view(&view)
        {
            self.composing_id = Some(frame.id());
            let mut doc = photo.document.clone();
            let layer = self.bound.layer;
            let surface = frame.surface.clone();
            let shot = frame.shot.clone();
            self.composing = Some(AsyncComputeTaskPool::get().spawn(async move {
                doc.layer_mut(layer).unwrap().content =
                    photocraft_doc::LayerContent::Raster(surface);
                let image = crate::editor::photo_document::composite_region(
                    &doc,
                    photocraft_geom::Rect::from_size(doc.size),
                );
                (shot, image)
            }));
        }
        completed
    }
    fn request_pose(&mut self, pose: Option<ReferencePose>) {
        self.requested =
            pose.filter(|p| p.root != self.model.pose || p.joints != self.model.joints);
        self.offset = self
            .requested
            .as_ref()
            .map_or(Vec2::ZERO, |p| projected_offset(&self.model, p.root));
        if self.requested.is_none() {
            self.camera_image = None;
            self.composite_shot = None;
            self.composing = None;
            self.composing_id = None;
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

    fn displayed_shot(&self) -> Option<&Shot> {
        self.requested.as_ref()?;
        if self.on_gpu() {
            self.camera_image.as_ref().map(|frame| frame.shot.as_ref())
        } else {
            self.composite_shot.as_deref()
        }
    }

    pub(super) fn placement(&mut self) -> Option<Placement<'_>> {
        let camera = self
            .camera_image
            .as_ref()
            .filter(|_| self.requested.is_some());
        let frame = camera.map_or(self.frame, |frame| frame.rect);
        let deferred = if self.render_3d {
            camera.is_some()
        } else {
            self.offset != Vec2::ZERO
        };
        Some(Placement {
            live: self.live.as_mut()?,
            frame,
            quad: frame_quad(frame).map(|p| if self.render_3d { p } else { p + self.offset }),
            camera: camera.map(|frame| frame.image.clone()),
            deferred,
        })
    }
}

enum BoneDrag {
    Rotate {
        key: String,
        basis: Quat,
    },
    Reach {
        limb: Box<Reach>,
        right: Vec3,
        down: Vec3,
        delta: Vec2,
    },
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
    bone: Option<BoneDrag>,
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
        project_to_canvas(&self.view, direction)
    }

    fn segments(&self, tool: Tool, axis: usize) -> Vec<(Vec2, Vec2)> {
        match tool {
            Tool::Move => self
                .move_tip(axis)
                .map(|tip| vec![(self.center, tip)])
                .unwrap_or_default(),
            Tool::Rotate => rotation_rings::segments(self, axis),
            Tool::Scale => {
                let direction = self.scale_direction(axis);
                vec![(
                    self.center + direction * 12.0,
                    self.center + direction * RADIUS,
                )]
            }
            Tool::Universal => Vec::new(),
        }
    }

    fn scale_direction(&self, axis: usize) -> Vec2 {
        let projected = self.project(self.model.pose.transform().rotation * AXES[axis]);
        if projected.length_squared() > 0.04 {
            projected.normalize()
        } else {
            Vec2::from_angle((axis as f32 * 2.0 + 1.0) * std::f32::consts::FRAC_PI_3)
        }
    }

    fn move_tip(&self, axis: usize) -> Option<Vec2> {
        MOVE_AXES
            .get(axis)
            .map(|direction| self.center + *direction * HANDLE_LENGTH)
    }

    fn handle_material(
        &self,
        tool: Tool,
        viewport: Rect,
        highlight: Option<usize>,
    ) -> HandleMaterial {
        let scale = tool == Tool::Scale;
        let center = self.center - viewport.min;
        let mut material = HandleMaterial {
            viewport: viewport.size().extend(1.0).extend(if scale { 1.0 } else { 0.0 }),
            pivot: center.extend(1.0).extend(if highlight == Some(FREE) { 1.0 } else { 0.0 }),
            ..default()
        };
        for (axis, color) in AXIS_COLORS.into_iter().enumerate() {
            let tip = if scale {
                Some(self.center + self.scale_direction(axis) * RADIUS)
            } else {
                self.move_tip(axis)
            };
            let Some(tip) = tip else { continue };
            let tip = tip - viewport.min;
            material.tips[axis] = tip.extend(1.0).extend(0.0);
            let tint = if highlight == Some(axis) { HIGHLIGHT } else { color };
            material.colors[axis] = tint.to_linear();
            if scale {
                let rotation = self.model.pose.transform().rotation;
                for (corner, offset) in
                    scale_cube_offsets(rotation, *self.view.back(), HANDLE_LENGTH)
                        .into_iter()
                        .enumerate()
                {
                    material.faces[axis * 12 + corner] =
                        (tip + self.project(offset)).extend(0.0).extend(0.0);
                }
            }
        }
        material
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
        let mut best = None;
        let mut nearest = if tool == Tool::Scale { 10.0 } else { 8.0 };
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
                        Tool::Move => self.pixels_per_unit.recip(),
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
        let point = if tool == Tool::Scale { b } else { (a + b) * 0.5 };
        if canvas.contains(point)
            && let Some((0, direction, _)) = frame.hit(tool, point)
        {
            return (point, point + direction * 34.0);
        }
    }
    panic!("No visible X handle");
}

impl Drag {
    fn reference_pose(&self, model: &ModelReference) -> ReferencePose {
        let mut pose = model.reference_pose();
        match &self.bone {
            Some(BoneDrag::Reach {
                limb,
                right,
                down,
                delta,
            }) => {
                limb.apply(*right * delta.x + *down * delta.y, &mut pose.joints);
            }
            Some(BoneDrag::Rotate { key, basis }) if self.preview != self.initial => {
                let rotation = (basis.inverse() * self.preview.transform().rotation).normalize();
                pose.joints.insert(key.clone(), rotation.to_array());
            }
            Some(BoneDrag::Rotate { .. }) => {}
            None => pose.root = self.preview,
        }
        pose
    }

    fn update(&mut self, pointer: Vec2, snap: bool) {
        if let Some(BoneDrag::Reach { delta, .. }) = &mut self.bone {
            let motion = pointer - self.origin;
            *delta = if motion.length_squared() >= 9.0 {
                motion
            } else {
                Vec2::ZERO
            };
            return;
        }
        if pointer.distance_squared(self.origin) < 1.0 {
            self.preview = self.initial;
            return;
        }
        let amount = (pointer - self.origin).dot(self.direction) * self.sensitivity;
        let index = self.axis;
        let mut transform = self.initial.transform();
        if self.tool == Tool::Move {
            let mut offset = if index == FREE {
                (pointer - self.origin) * self.sensitivity
            } else {
                self.direction * amount
            };
            if snap {
                offset = (offset * 4.0).round() * 0.25;
            }
            let displacement = *self.view.right() * offset.x - *self.view.up() * offset.y;
            // Stop in the drag direction at the pose limit, preserving camera depth.
            let mut fraction = 1.0_f32;
            for (component, step) in transform
                .translation
                .to_array()
                .into_iter()
                .zip(displacement.to_array())
            {
                if step != 0.0 && (component + step).abs() > 100_000.0 {
                    fraction = fraction.min((100_000.0_f32.copysign(step) - component) / step);
                }
            }
            transform.translation += displacement * fraction.max(0.0);
            self.preview = transform.into();
            return;
        }
        if index == FREE {
            match self.tool {
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
                Tool::Move | Tool::Universal => {}
            }
            self.preview = transform.into();
            return;
        }
        match self.tool {
            Tool::Rotate => {
                let step = 15.0_f32.to_radians();
                let u = project_to_canvas(&self.view, AXES[(index + 1) % 3]);
                let v = project_to_canvas(&self.view, AXES[(index + 2) % 3]);
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
            Tool::Move | Tool::Universal => {}
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
    project_to_canvas(&model.view(), delta) * model.pixels_per_unit
}

fn display_composite(viewer: &mut ImageViewer, references: &ReferenceRender, bound: Binding) {
    let Some(cache) = &mut viewer.layer_preview else {
        return;
    };
    if !cache.on_gpu()
        && let Some(photo) = viewer.edit.document(Some(bound.tab))
        && let Some((shot, image)) = cache.composite(photo, references.live.raster())
    {
        cache.composite_shot = Some(shot);
        viewer.edit.display_model_preview(bound.tab, image);
        viewer.model_preview = Some((bound.tab, bound.revision));
    }
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
    let reset = std::mem::take(&mut viewer.edit.model_pose_reset);
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
        viewer.bone_selection = None;
        viewer.layer_preview = None;
        viewer.clear_model_preview();
        return viewer.model_drag.take().is_some();
    };
    let posing = viewer.edit.model_posing;
    if !posing
        || viewer
            .bone_selection
            .as_ref()
            .is_some_and(|selection| !selection.bound.same_layer(bound))
    {
        viewer.bone_selection = None;
    }
    if reset && posing {
        viewer.model_drag = None;
        if let Err(error) = references.pose_selected(
            bound.tab,
            viewer.edit.document_mut(bound.tab).unwrap(),
            Default::default(),
        ) {
            viewer.edit.error = Some(error);
        }
        return true;
    }
    {
        let photo = viewer.edit.document(Some(bound.tab)).unwrap();
        if viewer.layer_preview.as_ref().is_none_or(|cache| {
            cache.bound != bound
                || cache.tool != tool
                || cache.model.asset != frame.model.asset
                || cache.view_revision != photo.view_revision
        }) {
            let mut raster_model = frame.model.clone();
            if let Some(previous) = photo
                .active_layer()
                .and_then(|l| l.xmp.as_ref())
                .and_then(|packet| references.previous_pose(packet))
            {
                raster_model.pose = previous.root;
                raster_model.joints.clone_from(&previous.joints);
            }
            viewer.layer_preview = Some(LayerPreview::new(
                bound,
                photo,
                &raster_model,
                tool,
                references.live.generation,
            ));
        } else if let Some(cache) = &mut viewer.layer_preview
            && cache.generation != references.live.generation
        {
            // The last displayed frame still owns its original rig. Only work
            // awaiting composition needs to be invalidated on re-instantiation.
            cache.generation = references.live.generation;
            cache.composing = None;
            cache.composing_id = None;
        }
        if let Some(live) = viewer.layer_preview.as_mut().and_then(|m| m.live.as_mut()) {
            live.poll(bound.revision);
        }
    }
    let released = frame.model.pending_frame.and_then(|_| {
        let pose = frame.model.reference_pose();
        (references.pending_reference_pose(bound.tab, bound.layer) == Some(&pose)).then_some(pose)
    });
    if viewer.model_drag.is_none() && released.is_none() {
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
        return viewer.model_drag.take().is_some();
    }
    if let Some(pose) = released.as_ref()
        && let Some(cache) = &mut viewer.layer_preview
    {
        cache.request_pose(Some(pose.clone()));
    }
    if viewer.model_drag.is_none() && released.is_some() {
        display_composite(viewer, references, bound);
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
            || drag.bone.is_some() != posing
        {
            viewer.clear_model_preview();
            return true;
        }
        if let Some(cursor) = window.cursor_position() {
            drag.update(
                cursor,
                keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            );
        }
        let pose = drag.reference_pose(&frame.model);
        let changed = pose.root != frame.model.pose || pose.joints != frame.model.joints;
        let on_gpu = if let Some(cache) = &mut viewer.layer_preview {
            cache.request_pose(Some(pose.clone()));
            cache.on_gpu()
        } else {
            false
        };
        if mouse.just_released(MouseButton::Left) {
            let doc = viewer.edit.document_mut(bound.tab).unwrap();
            let result = if posing {
                references.pose_selected(bound.tab, doc, pose.joints.clone())
            } else {
                references.transform_selected(bound.tab, doc, pose.root)
            };
            if let Err(error) = result {
                viewer.clear_model_preview();
                viewer.edit.error = Some(error);
            } else if changed {
                let committed = Binding {
                    revision: doc.revision,
                    ..bound
                };
                if let Some(cache) = &mut viewer.layer_preview {
                    cache.bound = committed;
                }
                if viewer.model_preview == Some((bound.tab, bound.revision)) {
                    viewer
                        .edit
                        .retain_model_preview(bound.tab, bound.revision, committed.revision);
                    viewer.model_preview = Some((bound.tab, committed.revision));
                }
            }
        } else if mouse.pressed(MouseButton::Left) {
            if changed && !on_gpu {
                display_composite(viewer, references, bound);
            } else if !changed {
                viewer.clear_model_preview();
            }
            viewer.model_drag = Some(drag);
        } else {
            viewer.clear_model_preview();
        }
        return true;
    }
    if mouse.just_pressed(MouseButton::Left)
        && !over_control
        && let Some(cursor) = window.cursor_position().filter(|p| input.contains(*p))
        && references
            .pending_reference_pose(bound.tab, bound.layer)
            .is_none()
    {
        let mut frame = frame;
        let mut bone = None;
        let mut initial = frame.model.pose;
        let hit = if posing {
            let dimensions = viewer.dimensions.unwrap();
            let shot = viewer
                .layer_preview
                .as_ref()
                .and_then(LayerPreview::displayed_shot);
            let Some(view) = BoneView::new(
                references,
                shot,
                &frame.model,
                dimensions,
                viewer.image_rect(dimensions, canvas),
            ) else {
                return true;
            };
            let rotations = shot.map_or(&frame.model.joints, |s| &s.view.pose.joints);
            let markers = view.markers(rotations);
            if let Some(index) = bone_at(&markers, cursor, input) {
                let marker = &markers[index];
                viewer.bone_selection = Some(BoneSelection {
                    bound,
                    joint: marker.key.to_owned(),
                });
                if marker.effector.is_none() {
                    return true;
                }
                let Some(reach) = view.reach(marker.key, rotations) else {
                    return true;
                };
                bone = Some(reach);
                frame.center = marker.center;
                Some((FREE, Vec2::ZERO, 1.0))
            } else {
                let selected = viewer.bone_selection.as_ref().and_then(|selection| {
                    markers.iter().find(|marker| marker.key == selection.joint)
                });
                let hit = selected.and_then(|joint| {
                    frame.center = joint.center;
                    frame.hit(Tool::Rotate, cursor).map(|hit| (joint, hit))
                });
                if let Some((joint, hit)) = hit {
                    let offset = frame
                        .model
                        .joints
                        .get(joint.key)
                        .map_or(Quat::IDENTITY, |offset| Quat::from_array(*offset));
                    initial = Transform::from_rotation((joint.basis * offset).normalize()).into();
                    bone = Some(BoneDrag::Rotate {
                        key: joint.key.to_owned(),
                        basis: joint.basis,
                    });
                    Some(hit)
                } else {
                    viewer.bone_selection = None;
                    return true;
                }
            }
        } else {
            target(&frame, tool, viewer, bound.tab, canvas, cursor)
        };
        let Some((axis, direction, sensitivity)) = hit else {
            return true;
        };
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
            initial,
            preview: initial,
            bone,
        });
    }
    // While a model tool is selected, a click on the canvas must not paint.
    mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left)
}

fn handle_hint(tool: Tool, axis: usize, drag: Option<&Drag>) -> String {
    let label = ["X", "Y", "Z", "холст"][axis];
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
        Tool::Move => {
            let offset = project_to_canvas(&drag.view, after.translation - before.translation)
                * Vec2::new(1.0, -1.0);
            if axis == FREE {
                format!("X {:+.2}  Y {:+.2}", offset.x, offset.y)
            } else {
                format!("{:+.2}", offset[axis])
            }
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
    canvas: Entity,
    markers: Vec<Entity>,
    hint: Entity,
    hint_text: Entity,
    hint_value: String,
    handle_material: Handle<HandleMaterial>,
    rings: rotation_rings::Overlay,
    last: Option<(Tool, Vec2, Vec2, Vec3, [f32; 4], Option<usize>)>,
}

impl Overlay {
    #[cfg(test)]
    pub(super) fn handle_geometry(&self) -> (Vec2, [f32; 4]) {
        let (_, center, _, _, rotation, _) = self.last.unwrap();
        (center, rotation)
    }

    pub(super) fn setup(
        commands: &mut Commands,
        canvas: Entity,
        font: &Handle<Font>,
        handles: &mut Assets<HandleMaterial>,
    ) -> Self {
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
        let (overlay, handle_material) = viewport::spawn_overlay(commands, handles, canvas);
        commands.entity(overlay).insert((ImageOverlay, ZIndex(12)));
        Self {
            canvas,
            markers: Vec::new(),
            hint,
            hint_text,
            hint_value: String::new(),
            rings: rotation_rings::Overlay::setup(commands, canvas, font),
            handle_material,
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
        handles: &mut Assets<HandleMaterial>,
        nodes: &mut Query<&mut Node>,
        commands: &mut Commands,
    ) {
        let mut markers = Vec::new();
        let mut selected_bone = None;
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
            let pending = references.pending_reference_pose(tab, layer);
            let displayed = viewer
                .layer_preview
                .as_ref()
                .filter(|preview| preview.bound.tab == tab && preview.bound.layer == layer)
                .and_then(LayerPreview::displayed_shot);
            if viewer.edit.model_posing {
                // Keep the dots on the rendered limbs while the preview warms up
                // or a software frame is still being composed.
                if let Some(pose) = displayed.map(|shot| &shot.view.pose)
                    .or_else(|| {
                        let layer = viewer.edit.document(Some(tab))?.active_layer()?;
                        references.previous_pose(layer.xmp.as_ref()?)
                    })
                {
                    model.pose = pose.root;
                    model.joints.clone_from(&pose.joints);
                }
            } else if let Some(drag) = viewer
                .model_drag
                .as_ref()
                .filter(|drag| drag.bound.tab == tab && drag.bound.layer == layer)
            {
                let pose = drag.reference_pose(&model);
                model.pose = pose.root;
                model.joints = pose.joints;
            } else if let Some(pose) = pending {
                model.pose = pose.root;
                model.joints.clone_from(&pose.joints);
            }
            let image = viewer.image_rect(dimensions, viewport);
            let mut frame = Frame::new(model, image, dimensions);
            if viewer.edit.model_posing {
                let model = frame.model.clone().resolved(&editor.root);
                markers = BoneView::new(references, displayed, &model, dimensions, image)?
                    .markers(&model.joints);
                let selection = viewer.bone_selection.as_ref()?;
                let photo = viewer.edit.document(Some(tab))?;
                if (
                    selection.bound.tab,
                    selection.bound.photo,
                    selection.bound.layer,
                ) != (tab, photo.document.id, layer)
                {
                    return None;
                }
                let joint = markers.iter().find(|bone| bone.key == selection.joint)?;
                frame.center = joint.center;
                let label = match joint.effector {
                    Some(kind) => effector_hint(kind),
                    None => joint.name,
                };
                selected_bone = Some((joint.key, label));
            }
            viewport.contains(frame.center).then_some((tool, frame))
        })();
        let hovered_bone = pointer.and_then(|point| bone_at(&markers, point, viewport));
        let mut visible = 0;
        for (index, joint) in markers
            .iter()
            .enumerate()
            .filter(|(_, joint)| viewport.contains(joint.center))
        {
            let selected = selected_bone.is_some_and(|(key, _)| key == joint.key);
            let hovered = hovered_bone == Some(index);
            let ik = joint.effector.is_some();
            let size = if ik {
                13.0
            } else if selected || hovered {
                11.0
            } else {
                8.0
            };
            let order = ZIndex(if ik { 13 } else { 11 });
            let point = joint.center - viewport.min;
            let marker = Node {
                position_type: PositionType::Absolute,
                left: px(point.x - size / 2.0),
                top: px(point.y - size / 2.0),
                width: px(size),
                height: px(size),
                border: UiRect::all(px(1.5)),
                border_radius: if ik {
                    BorderRadius::all(px(3))
                } else {
                    BorderRadius::MAX
                },
                ..default()
            };
            let color = BackgroundColor(if selected || hovered {
                HIGHLIGHT
            } else if ik {
                Color::srgb(0.20, 0.86, 0.79)
            } else {
                Color::WHITE
            });
            if let Some(&entity) = self.markers.get(visible) {
                if let Ok(mut node) = nodes.get_mut(entity) {
                    node.set_if_neq(marker);
                }
                commands.entity(entity).insert((color, order));
            } else {
                self.markers.push(
                    commands
                        .spawn((
                            marker,
                            color,
                            BorderColor::all(Color::srgb(0.12, 0.14, 0.17)),
                            Pickable::IGNORE,
                            order,
                            ChildOf(self.canvas),
                        ))
                        .id(),
                );
            }
            visible += 1;
        }
        for &entity in &self.markers[visible..] {
            if let Ok(mut node) = nodes.get_mut(entity) {
                node.display = Display::None;
            }
        }
        let dragging = viewer.model_drag.is_some();
        let reaching = viewer
            .model_drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.bone, Some(BoneDrag::Reach { .. })));
        let highlight = viewer
            .model_drag
            .as_ref()
            .map(|drag| drag.axis)
            .or_else(|| {
                if hovered_bone.is_some() {
                    return None;
                }
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
            })
            .filter(|_| !reaching);
        let rotating = geometry
            .as_ref()
            .filter(|(tool, _)| *tool == Tool::Rotate && !reaching)
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
            } else if let Some(name) = hovered_bone
                .filter(|_| !dragging)
                .and_then(|i| markers[i].effector.map(effector_hint))
                .or_else(|| selected_bone.map(|(_, name)| name))
            {
                node.display = Display::Flex;
                node.max_width = px(viewport.width());
                node.left = px(12.0);
                node.top = px(12.0);
                if name != self.hint_value {
                    commands.entity(self.hint_text).insert(Text::new(name));
                    self.hint_value = name.to_owned();
                }
            } else {
                node.display = Display::None;
            }
        }
        let key = geometry.as_ref().map(|(tool, frame)| {
            (
                *tool,
                frame.center - viewport.min,
                viewport.size(),
                *frame.view.back(),
                frame.model.pose.rotation,
                highlight,
            )
        });
        if self.last == key {
            return;
        }
        self.last = key;
        if let Some(mut material) = handles.get_mut(&self.handle_material) {
            material.viewport.z = 0.0;
            if let Some((tool @ (Tool::Move | Tool::Scale), frame)) = &geometry {
                *material = frame.handle_material(*tool, viewport, highlight);
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
            bone: None,
        }
    }

    #[test]
    fn model_transform_free_and_axis_handles_constrain_the_correct_components() {
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
        for (index, direction) in MOVE_AXES.into_iter().enumerate() {
            let mut axis = drag(Tool::Move, index, Vec2::ZERO);
            axis.direction = direction;
            axis.view = free.view;
            axis.initial.position = [1.0, 2.0, 3.0];
            axis.update(direction * 53.0 + direction.perp() * 100.0, true);
            let delta = Vec3::from(axis.preview.position) - Vec3::from(axis.initial.position);
            assert!(project_to_canvas(&axis.view, delta).abs_diff_eq(direction * 0.5, 1e-5));
            assert!(delta.dot(*axis.view.back()).abs() < 1e-5);
        }

        let mut uniform = drag(Tool::Scale, FREE, Vec2::ZERO);
        uniform.update(Vec2::new(30.0, 0.0), false);
        assert!(uniform.preview.scale[0] > 1.0);
        assert_eq!(uniform.preview.scale, [uniform.preview.scale[0]; 3]);

        let mut ring = drag(Tool::Rotate, FREE, Vec2::X * FREE_RADIUS);
        ring.update(Vec2::Y * FREE_RADIUS, false);
        assert!((ring.preview.transform().rotation * Vec3::X).abs_diff_eq(Vec3::NEG_Y, 1e-5));
    }
}
