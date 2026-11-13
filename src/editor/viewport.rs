use bevy::{
    anti_alias::taa::TemporalAntiAliasing,
    camera::{ImageRenderTarget, RenderTarget, ScalingMode, Viewport, visibility::RenderLayers},
    gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigGroup, GizmoConfigStore},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    input_focus::InputFocus,
    pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceTransmission},
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
    render::render_resource::{Extent3d, TextureFormat},
    ui_widgets::TextInput,
    window::PrimaryWindow,
};

use crate::{data::LevelEdits as RoomEdits, project::LensKind};

use super::{Editor, Field, Scene, Selection, Shot, Tool, ui};

#[path = "transform_handles.rs"]
mod transform_handles;
pub(super) use transform_handles::{HandleMaterial, HandlePlugin, spawn_overlay};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "rotation_gizmo_tests.rs"]
mod rotation_tests;

#[cfg(test)]
#[path = "transform_space_gizmo_tests.rs"]
mod space_tests;

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const AXIS_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.28, 0.29),
    Color::srgb(0.33, 0.89, 0.49),
    Color::srgb(0.32, 0.62, 1.0),
];
const HIGHLIGHT: Color = Color::srgb(1.0, 0.81, 0.38);
const CAMERA_COLOR: Color = Color::srgba(0.49, 0.72, 1.0, 0.72);
const EDITOR_FOV: f32 = 58.0_f32.to_radians();
const FOCUS_DURATION: f32 = 0.5;
pub(super) const CAMERA_ASPECT: f32 = 16.0 / 9.0;
pub(super) const EDITOR_GIZMO_LAYER: usize = 3;

#[derive(Component)]
pub(super) struct EditorCamera;

#[derive(Resource)]
pub(super) struct Surface(pub(super) Handle<Image>);

impl Surface {
    pub(super) fn new(images: &mut Assets<Image>) -> Self {
        Self(images.add(Image::new_target_texture(
            1,
            1,
            TextureFormat::Rgba8Unorm,
            Some(TextureFormat::Rgba8UnormSrgb),
        )))
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct TransformGizmos;

#[derive(Clone, Copy)]
enum Navigation {
    Fly,
    Pan,
    Orbit,
}

struct FocusFlight {
    room: super::Room,
    selection: Option<Selection>,
    start: Vec3,
    end: Vec3,
    start_pivot: Vec3,
    end_pivot: Vec3,
    elapsed: f32,
}

impl FocusFlight {
    fn new(editor: &Editor) -> Self {
        let pivot = editor
            .selection
            .and_then(|target| editor.pose(target))
            .map_or(Vec3::ZERO, |target| target.transform().translation);
        Self {
            room: editor.room,
            selection: editor.selection,
            start: editor.view.translation,
            end: pivot + *editor.view.back() * if editor.selection.is_some() { 3.0 } else { 8.0 },
            start_pivot: editor.pivot,
            end_pivot: pivot,
            elapsed: 0.0,
        }
    }

    fn advance(&mut self, editor: &mut Editor, delta: f32) -> bool {
        self.elapsed = (self.elapsed + delta).min(FOCUS_DURATION);
        let t = self.elapsed / FOCUS_DURATION;
        let eased = t * t * (3.0 - 2.0 * t);
        editor.view.translation = self.start.lerp(self.end, eased);
        editor.pivot = self.start_pivot.lerp(self.end_pivot, eased);
        if self.elapsed >= FOCUS_DURATION {
            editor.view.translation = self.end;
            editor.pivot = self.end_pivot;
            true
        } else {
            false
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct PointerState {
    nav: Option<Navigation>,
    focus_flight: Option<FocusFlight>,
    pilot: Option<Shot>,
    pivot: Vec3,
    drag: Option<Drag>,
    hover: Option<TransformHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransformHandle {
    Axis { tool: Tool, axis: usize },
    Plane { normal: usize },
    ViewRotation,
}

impl PointerState {
    pub(super) fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub(super) fn stop_navigation(&mut self) {
        self.nav = None;
        self.focus_flight = None;
        self.pilot = None;
    }
}

struct Drag {
    selection: Selection,
    handle: TransformHandle,
    tool: Tool,
    axis: usize,
    direction: Vec3,
    frame: Quat,
    before: RoomEdits,
    start: Transform,
    cursor: Vec2,
    pixels_per_metre: Vec2,
    scale_length: f32,
    ring_start: Option<Vec3>,
    plane_start: Option<Vec3>,
}

pub(super) fn setup(mut config: ResMut<GizmoConfigStore>) {
    config
        .config_mut::<DefaultGizmoConfigGroup>()
        .0
        .render_layers = RenderLayers::layer(EDITOR_GIZMO_LAYER);
    let (config, _) = config.config_mut::<TransformGizmos>();
    config.render_layers = RenderLayers::layer(EDITOR_GIZMO_LAYER);
    config.depth_bias = -1.0;
    config.line.width = 2.5;
}

pub(super) fn spawn_camera(
    commands: &mut Commands,
    editor: &Editor,
    pose: Transform,
    surface: &Surface,
) {
    let mut camera = commands.spawn((
        EditorCamera,
        bevy::camera::ShadowLodOrigin,
        Camera3d::default(),
        RenderLayers::from_layers(&[0, EDITOR_GIZMO_LAYER]),
        pose,
        ScreenSpaceTransmission::default(),
        RenderTarget::Image(surface.0.clone().into()),
    ));
    super::lighting::apply_camera_defaults(&mut camera);
    if let Some(setup) = editor.project.camera_setup {
        let path = if editor.definition().is_native() {
            &editor.definition().file
        } else {
            &editor.definition().asset
        };
        let logical = crate::asset_paths::original_asset_path(&editor.root, path)
            .unwrap_or_else(|_| path.clone());
        setup(&mut camera, &logical);
    }
    camera.entry::<RenderLayers>().and_modify(|mut layers| {
        *layers = layers
            .clone()
            .with(EDITOR_GIZMO_LAYER)
            .with(super::selection_outline::LAYER);
    });
    // A stable image matters when tracking a small handle with the pointer.
    camera.remove::<(TemporalAntiAliasing, ScreenSpaceAmbientOcclusion)>();
}

pub(super) fn navigate(
    mut editor: ResMut<Editor>,
    mut state: ResMut<PointerState>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<
        (
            &mut Camera,
            &mut RenderTarget,
            &mut Transform,
            &mut Projection,
        ),
        With<EditorCamera>,
    >,
    mut images: ResMut<Assets<Image>>,
    surface: Res<Surface>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    drawer: Res<ui::DrawerMotion>,
    focus: Res<InputFocus>,
    text_inputs: Query<(), With<TextInput>>,
    asset_drag: Option<Res<ui::AssetDrag>>,
    lighting: Option<Res<ui::LightingUi>>,
) {
    let Ok((mut camera, mut target, mut pose, mut lens)) = camera.single_mut() else {
        return;
    };
    let scale = window.scale_factor();
    if camera.order != -1 {
        camera.order = -1;
    }
    let rect = ui::viewport_rect(&window, &editor.sidebar);
    let physical = (rect.size() * scale).round().max(Vec2::ONE).as_uvec2();
    let size = Extent3d {
        width: physical.x,
        height: physical.y,
        depth_or_array_layers: 1,
    };
    if images
        .get(&surface.0)
        .is_some_and(|image| image.texture_descriptor.size != size)
        && let Some(mut image) = images.get_mut(&surface.0)
    {
        image.resize(size);
    }
    if matches!(&*target, RenderTarget::Image(image) if image.scale_factor != scale) {
        *target = RenderTarget::Image(ImageRenderTarget {
            handle: surface.0.clone(),
            scale_factor: scale,
        });
    }
    let shot = editor.selection.and_then(|selection| match selection {
        Selection::Camera(shot) if editor.pilot => Some(shot),
        _ => None,
    });
    camera.viewport = shot.map(|_| frame_viewport(physical));
    if state.pilot != shot {
        editor.finish_camera_navigation();
        state.nav = None;
        state.focus_flight = None;
        state.pilot = shot;
        if let Some(shot) = shot {
            let view = editor.camera(shot).pose.transform();
            state.pivot = view.translation + *view.forward() * 3.0;
        }
    }
    let typing = focus
        .get()
        .is_some_and(|entity| text_inputs.contains(entity));
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let enabled = window.focused
        && !editor.asset_viewer_open()
        && !editor.browser_popup
        && editor.ready
        && editor.pending.is_none()
        && !typing
        && !command
        && !drawer.resizing
        && !editor.sidebar.resizing()
        && !asset_drag.as_ref().is_some_and(|drag| drag.active());
    let enabled = enabled
        && !lighting
            .as_ref()
            .is_some_and(|lighting| lighting.blocks_scene_input());
    let preview_visible =
        editor.ready && !editor.pilot && matches!(editor.selection, Some(Selection::Camera(_)));
    let inside = enabled
        && window.cursor_position().is_some_and(|cursor| {
            ui::scene_input_rect(&window, &drawer, &editor.sidebar).contains(cursor)
                && !(preview_visible
                    && ui::camera_preview_rect(&window, &drawer, &editor.sidebar).contains(cursor))
        });
    let orbit_key = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let held = match state.nav {
        Some(Navigation::Fly) => mouse.pressed(MouseButton::Right),
        Some(Navigation::Pan) => mouse.pressed(MouseButton::Middle),
        Some(Navigation::Orbit) => mouse.pressed(MouseButton::Left),
        None => false,
    };
    if !enabled || !held {
        state.nav = None;
        editor.finish_camera_navigation();
    }
    if inside && !state.dragging() {
        if mouse.just_pressed(MouseButton::Right) {
            state.nav = Some(Navigation::Fly);
        } else if mouse.just_pressed(MouseButton::Middle) {
            state.nav = Some(Navigation::Pan);
        } else if orbit_key && mouse.just_pressed(MouseButton::Left) {
            state.nav = Some(Navigation::Orbit);
        }
    }
    let interrupted = !enabled
        || shot.is_some()
        || state.dragging()
        || state.nav.is_some()
        || keys.just_pressed(KeyCode::Escape)
        || (inside && (mouse.just_pressed(MouseButton::Left) || scroll.delta.y != 0.0));
    if std::mem::take(&mut editor.focus) && !interrupted {
        state.focus_flight = Some(FocusFlight::new(&editor));
    }
    if interrupted
        || state.focus_flight.as_ref().is_some_and(|flight| {
            flight.room != editor.room || flight.selection != editor.selection
        })
    {
        state.focus_flight = None;
    }
    if let Some(flight) = state.focus_flight.as_mut()
        && flight.advance(&mut editor, time.delta_secs())
    {
        state.focus_flight = None;
    }
    let mut placement = shot.map(|shot| editor.camera(shot));
    let mut view = placement.map_or(editor.view, |placement| placement.pose.transform());
    let start = view;
    let mut pivot = if shot.is_some() {
        state.pivot
    } else {
        editor.pivot
    };
    let mut distance = view.translation.distance(pivot).max(0.3);
    let delta = motion.delta;
    match state.nav {
        Some(Navigation::Fly) if mouse.pressed(MouseButton::Right) => {
            if delta != Vec2::ZERO {
                let (yaw, pitch, _) = view.rotation.to_euler(EulerRot::YXZ);
                view.rotation = Quat::from_euler(
                    EulerRot::YXZ,
                    yaw - delta.x * 0.004,
                    (pitch - delta.y * 0.004).clamp(-1.53, 1.53),
                    0.0,
                );
            }
            let input = |positive, negative| {
                f32::from(keys.pressed(positive)) - f32::from(keys.pressed(negative))
            };
            let movement = *view.right() * input(KeyCode::KeyD, KeyCode::KeyA)
                + *view.forward() * input(KeyCode::KeyW, KeyCode::KeyS)
                + Vec3::Y * input(KeyCode::KeyE, KeyCode::KeyQ);
            let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                11.0
            } else {
                4.0
            };
            view.translation += movement.normalize_or_zero() * speed * time.delta_secs().min(0.05);
            pivot = view.translation + *view.forward() * distance;
        }
        Some(Navigation::Pan) if mouse.pressed(MouseButton::Middle) => {
            let shift = (*view.right() * -delta.x + *view.up() * delta.y) * distance * 0.0016;
            view.translation += shift;
            pivot += shift;
        }
        Some(Navigation::Orbit) if mouse.pressed(MouseButton::Left) => {
            if delta != Vec2::ZERO {
                let direction = (view.translation - pivot).normalize_or_zero();
                let yaw = direction.x.atan2(direction.z) - delta.x * 0.006;
                let pitch = (direction.y.asin() + delta.y * 0.006).clamp(-1.5, 1.5);
                let offset = Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0) * Vec3::Z * distance;
                view = Transform::from_translation(pivot + offset).looking_at(pivot, Vec3::Y);
            }
        }
        _ => state.nav = None,
    }
    if inside && !state.dragging() && scroll.delta.y != 0.0 {
        let factor = if scroll.unit == MouseScrollUnit::Line {
            0.13
        } else {
            0.002
        };
        if let Some(placement) = placement.as_mut()
            && shot.is_some_and(|shot| {
                editor.definition().cameras[shot].projection == LensKind::Orthographic
            })
        {
            placement.lens = (placement.lens * (-scroll.delta.y * factor).exp()).clamp(0.05, 20.0);
        } else {
            let next = (distance * (-scroll.delta.y * factor).exp()).clamp(0.35, 80.0);
            view.translation += *view.forward() * (distance - next);
            distance = next;
            pivot = view.translation + *view.forward() * distance;
        }
    }
    if let Some(shot) = shot {
        if let Some(mut placement) = placement {
            let moved = view != start;
            let zoomed = placement.lens != editor.camera(shot).lens;
            if moved || zoomed {
                if moved {
                    placement.pose = view.into();
                }
                if placement.pose.valid() {
                    editor.begin_camera_navigation();
                    editor.set_camera(shot, placement);
                }
            }
            let placement = editor.camera(shot);
            *pose = placement.pose.transform();
            *lens = projection(editor.definition().cameras[shot].projection, placement.lens);
        }
        state.pivot = pivot;
        if state.nav.is_none() {
            editor.finish_camera_navigation();
        }
    } else {
        editor.view = view;
        editor.pivot = pivot;
        *pose = view;
        *lens = projection(LensKind::Perspective, EDITOR_FOV.to_degrees());
    }
}

pub(super) fn frame_viewport(size: UVec2) -> Viewport {
    let size = size.max(UVec2::ONE);
    let frame = if size.x as f32 / size.y.max(1) as f32 > CAMERA_ASPECT {
        UVec2::new((size.y as f32 * CAMERA_ASPECT) as u32, size.y)
    } else {
        UVec2::new(size.x, (size.x as f32 / CAMERA_ASPECT) as u32)
    }
    .max(UVec2::ONE);
    Viewport {
        physical_position: size.saturating_sub(frame) / 2,
        physical_size: frame,
        ..default()
    }
}

pub(super) fn projection(kind: LensKind, lens: f32) -> Projection {
    if kind == LensKind::Orthographic {
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: lens,
            },
            near: 0.01,
            far: 100.0,
            ..OrthographicProjection::default_3d()
        })
    } else {
        Projection::Perspective(PerspectiveProjection {
            fov: lens.to_radians(),
            near: 0.05,
            far: 200.0,
            ..default()
        })
    }
}

pub(super) fn length(camera: &Camera, view: &Transform, pivot: Vec3) -> f32 {
    let height = camera
        .logical_viewport_size()
        .map_or(500.0, |size| size.y)
        .max(1.0);
    let depth = (pivot - view.translation).dot(*view.forward()).max(0.06);
    (175.0 * depth * (EDITOR_FOV * 0.5).tan() / height).clamp(0.06, 8.0)
}

pub(super) fn allowed(editor: &Editor, selection: Selection, tool: Tool, axis: usize) -> bool {
    let field = match tool {
        Tool::Universal => {
            return tool
                .operations()
                .iter()
                .any(|&tool| allowed(editor, selection, tool, axis));
        }
        Tool::Move => Field::Position(axis),
        Tool::Rotate => Field::Rotation(axis),
        Tool::Scale => Field::Scale(axis),
    };
    editor.field_allowed(selection, field)
}

fn axis_length(mode: Tool, tool: Tool, length: f32) -> f32 {
    if mode == Tool::Universal && tool == Tool::Move {
        length * 1.45
    } else {
        length
    }
}

fn direction(editor: &Editor, pose: Transform, axis: usize) -> Vec3 {
    editor.gizmo_rotation(pose) * AXES[axis]
}

fn scaled_pose(start: Transform, frame: Quat, axis: usize, factor: f32) -> Transform {
    if frame.abs_diff_eq(start.rotation, 1e-5) {
        let mut pose = start;
        pose.scale[axis] *= factor;
        return pose;
    }
    let basis = Mat4::from_quat(frame);
    let mut scale = Vec3::ONE;
    scale[axis] = factor;
    let mut pose = Transform::from_matrix(
        basis * Mat4::from_scale(scale) * basis.inverse() * start.to_matrix(),
    );
    pose.translation = start.translation;
    pose.rotation = pose.rotation.normalize();
    pose
}

fn ring_points(pivot: Vec3, axis: usize, radius: f32) -> impl Iterator<Item = Vec3> {
    let u = AXES[(axis + 1) % 3];
    let v = AXES[(axis + 2) % 3];
    (0..=96).map(move |index| {
        let (sin, cos) = (std::f32::consts::TAU * index as f32 / 96.0).sin_cos();
        pivot + (u * cos + v * sin) * radius
    })
}

const ROTATION_RADIUS_PIXELS: f32 = 88.0;
const ROTATION_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.12, 0.30),
    Color::srgb(0.55, 0.85, 0.02),
    Color::srgb(0.03, 0.54, 1.0),
];

fn rotation_radius(camera: &Camera, view: &Transform, pivot: Vec3) -> f32 {
    let global = GlobalTransform::from(*view);
    camera
        .world_to_viewport(&global, pivot)
        .ok()
        .zip(
            camera
                .world_to_viewport(&global, pivot + *view.right())
                .ok(),
        )
        .map(|(a, b)| ROTATION_RADIUS_PIXELS / a.distance(b).max(0.001))
        .unwrap_or_else(|| length(camera, view, pivot))
}

/// The gizmo uses parallel projection at its pivot depth. Every ring shares
/// exactly the same projected center, even off-center or close to the camera.
#[cfg(test)]
fn display_ring_points(
    pivot: Vec3,
    axis: usize,
    radius: f32,
    view: &Transform,
) -> impl Iterator<Item = Vec3> {
    let forward = *view.forward();
    ring_points(pivot, axis, radius).map(move |point| {
        let offset = point - pivot;
        point - forward * offset.dot(forward)
    })
}

/// The rotation reference shows the front hemisphere of each axis circle.
/// The grey view-aligned outer ring provides the complete circular silhouette.
fn universal_ring_segments(
    pivot: Vec3,
    axis: usize,
    radius: f32,
    view: &Transform,
    frame: Quat,
) -> impl Iterator<Item = (Vec3, Vec3)> {
    let forward = *view.forward();
    let flatten = move |point: Vec3| point - forward * (point - pivot).dot(forward);
    let mut previous = None;
    ring_points(pivot, axis, radius).filter_map(move |point| {
        let point = pivot + frame * (point - pivot);
        let start = previous.replace(point)?;
        let a = (start - pivot).dot(*view.back());
        let b = (point - pivot).dot(*view.back());
        let (a, b) = match (a >= 0.0, b >= 0.0) {
            (true, true) => (start, point),
            (false, false) => return None,
            _ => {
                let equator = start.lerp(point, a / (a - b));
                if a >= 0.0 {
                    (start, equator)
                } else {
                    (equator, point)
                }
            }
        };
        Some((flatten(a), flatten(b)))
    })
}

fn plane_corners(pivot: Vec3, normal: usize, size: f32, frame: Quat) -> [Vec3; 4] {
    let u = frame * AXES[(normal + 1) % 3];
    let v = frame * AXES[(normal + 2) % 3];
    let center = pivot + (u + v) * size * 0.28;
    [u + v, u - v, -u - v, -u + v].map(|offset| center + offset * size * 0.065)
}

fn plane_allowed(editor: &Editor, selection: Selection, normal: usize) -> bool {
    [1, 2]
        .into_iter()
        .all(|offset| allowed(editor, selection, Tool::Move, (normal + offset) % 3))
}

fn view_rotation_allowed(editor: &Editor, selection: Selection) -> bool {
    (0..3).all(|axis| allowed(editor, selection, Tool::Rotate, axis))
}

fn outer_ring_radius(camera: &Camera, view: &Transform, pivot: Vec3, radius: f32) -> Option<f32> {
    let global = GlobalTransform::from(*view);
    let start = camera.world_to_viewport(&global, pivot).ok()?;
    let end = camera
        .world_to_viewport(&global, pivot + *view.right() * radius * 1.15)
        .ok()?;
    Some(start.distance(end))
}

fn plane_point(ray: Ray3d, center: Vec3, normal: Vec3) -> Option<Vec3> {
    let denominator = ray.direction.dot(normal);
    if denominator.abs() < 1e-4 {
        return None;
    }
    let point = ray.get_point((center - ray.origin).dot(normal) / denominator);
    point.is_finite().then_some(point)
}

fn quad_contains(point: Vec2, corners: [Vec2; 4]) -> bool {
    let mut lower = f32::INFINITY;
    let mut upper = f32::NEG_INFINITY;
    let mut area = 0.0;
    for index in 0..4 {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        let side = (b - a).perp_dot(point - a);
        lower = lower.min(side);
        upper = upper.max(side);
        area += a.perp_dot(b);
    }
    area.abs() >= 6.0 && (lower >= 0.0 || upper <= 0.0)
}

fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let segment = b - a;
    let t = ((point - a).dot(segment) / segment.length_squared().max(1e-6)).clamp(0.0, 1.0);
    point.distance(a + segment * t)
}

fn handle_at(
    editor: &Editor,
    selection: Selection,
    pose: Transform,
    camera: &Camera,
    view: &Transform,
    cursor: Vec2,
    rotation_radius: f32,
) -> Option<TransformHandle> {
    let global = GlobalTransform::from(*view);
    let project = |point| camera.world_to_viewport(&global, point).ok();
    let pivot = pose.translation;
    let frame = editor.gizmo_rotation(pose);
    let length = length(camera, view, pivot);
    let start = project(pivot)?;
    if editor.tool == Tool::Universal {
        for normal in (0..3).rev() {
            if !plane_allowed(editor, selection, normal) {
                continue;
            }
            let [Some(a), Some(b), Some(c), Some(d)] =
                plane_corners(pivot, normal, length, frame).map(project)
            else {
                continue;
            };
            if quad_contains(cursor, [a, b, c, d]) {
                return Some(TransformHandle::Plane { normal });
            }
        }
    }
    let mut nearest: Option<(TransformHandle, f32)> = None;
    for &tool in editor.tool.operations() {
        for axis in 0..3 {
            if !allowed(editor, selection, tool, axis) {
                continue;
            }
            let distance = if tool == Tool::Rotate {
                universal_ring_segments(pivot, axis, rotation_radius, view, frame)
                    .filter_map(|(a, b)| {
                        Some(distance_to_segment(cursor, project(a)?, project(b)?))
                    })
                    .fold(f32::INFINITY, f32::min)
            } else {
                let extent = axis_length(editor.tool, tool, length);
                let Some(end) = project(pivot + direction(editor, pose, axis) * extent) else {
                    continue;
                };
                if end.distance(start) < 13.0 {
                    continue;
                }
                distance_to_segment(cursor, start.lerp(end, 0.23), end)
            };
            if distance < 9.0 && nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some((TransformHandle::Axis { tool, axis }, distance));
            }
        }
    }
    if editor.tool.operations().contains(&Tool::Rotate)
        && view_rotation_allowed(editor, selection)
        && let Some(radius) = outer_ring_radius(camera, view, pivot, rotation_radius)
    {
        let distance = (cursor.distance(start) - radius).abs();
        if distance < 5.0 && nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((TransformHandle::ViewRotation, distance));
        }
    }
    nearest.map(|(handle, _)| handle)
}

/// Invert the same parallel projection used for drawing, rather than using
/// a perspective ray against a ring which no longer has perspective distortion.
fn rotation_direction(
    camera: &Camera,
    view: &Transform,
    cursor: Vec2,
    center: Vec3,
    normal: Vec3,
) -> Option<Vec3> {
    let ray = camera
        .viewport_to_world(&GlobalTransform::from(*view), cursor)
        .ok()?;
    let forward = *view.forward();
    let depth = (center - ray.origin).dot(forward) / ray.direction.dot(forward);
    let plane_point = ray.get_point(depth);
    let denominator = forward.dot(normal);
    if denominator.abs() < 1e-4 {
        return None;
    }
    let distance = (center - plane_point).dot(normal) / denominator;
    (plane_point + forward * distance - center).try_normalize()
}

pub(super) fn manipulate(
    mut editor: ResMut<Editor>,
    mut pointer: ResMut<PointerState>,
    scene: Res<Scene>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &Transform), With<EditorCamera>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    parents: Query<&ChildOf>,
    mut meshes: MeshRayCast,
    drawer: Res<ui::DrawerMotion>,
    asset_drag: Option<Res<ui::AssetDrag>>,
    lighting: Option<Res<ui::LightingUi>>,
) {
    let Ok((camera, view)) = camera.single() else {
        return;
    };
    if editor.pending.is_some()
        || editor.asset_viewer_open()
        || editor.browser_popup
        || asset_drag.as_ref().is_some_and(|drag| drag.active())
        || lighting
            .as_ref()
            .is_some_and(|lighting| lighting.blocks_scene_input())
    {
        if let Some(drag) = pointer.drag.take() {
            editor.finish_edit(drag.before);
        }
        pointer.stop_navigation();
        return;
    }
    let global = GlobalTransform::from(*view);
    let cursor = window
        .cursor_position()
        .map(|cursor| cursor - Vec2::new(0.0, ui::TOP));
    if let Some(drag) = pointer.drag.take() {
        if keys.just_pressed(KeyCode::Escape) || !window.focused {
            editor.edits = drag.before;
        } else if !mouse.pressed(MouseButton::Left) {
            editor.finish_edit(drag.before);
        } else {
            if let Some(cursor) = cursor {
                let offset = cursor - drag.cursor;
                let pixels = drag.pixels_per_metre;
                let delta = offset.dot(pixels) / pixels.length_squared().max(1e-4);
                let snap = |value: f32, step: f32| {
                    if editor.snap {
                        (value / step).round() * step
                    } else {
                        value
                    }
                };
                let mut pose = drag.start;
                match drag.tool {
                    Tool::Universal => unreachable!("dragging resolves to one transform operation"),
                    Tool::Move if matches!(drag.handle, TransformHandle::Plane { .. }) => {
                        if let Some(point) =
                            drag.plane_start
                                .zip(camera.viewport_to_world(&global, cursor).ok().and_then(
                                    |ray| plane_point(ray, pose.translation, drag.direction),
                                ))
                                .map(|(start, end)| end - start)
                        {
                            let local = drag.frame.inverse() * point;
                            let mut shift = Vec3::ZERO;
                            for axis in 0..3 {
                                if axis != drag.axis {
                                    shift[axis] = snap(local[axis], 0.25);
                                }
                            }
                            pose.translation += drag.frame * shift;
                        }
                    }
                    Tool::Move => pose.translation += drag.direction * snap(delta, 0.25),
                    Tool::Scale => {
                        let original = if editor.local_space {
                            pose.scale[drag.axis].abs()
                        } else {
                            (pose.rotation.inverse() * drag.direction * pose.scale).length()
                        };
                        let scale = snap(original * (1.0 + delta / drag.scale_length), 0.1)
                            .clamp(0.01, 100.0);
                        pose = scaled_pose(pose, drag.frame, drag.axis, scale / original);
                    }
                    Tool::Rotate => {
                        let angle = drag
                            .ring_start
                            .zip(rotation_direction(
                                camera,
                                view,
                                cursor,
                                pose.translation,
                                drag.direction,
                            ))
                            .map_or((offset.x - offset.y) * 0.011, |(start, end)| {
                                drag.direction.dot(start.cross(end)).atan2(start.dot(end))
                            });
                        pose.rotation = Quat::from_axis_angle(
                            drag.direction,
                            snap(angle, 15.0_f32.to_radians()),
                        ) * pose.rotation;
                    }
                }
                if pose.translation.is_finite()
                    && pose.translation.abs().max_element() <= 1000.0
                    && crate::Pose::from(pose).valid()
                {
                    editor.set_pose(drag.selection, pose);
                }
            }
            pointer.drag = Some(drag);
        }
        return;
    }

    pointer.hover = None;
    if !editor.ready || editor.pilot || pointer.nav.is_some() || !window.focused {
        return;
    }
    if !window.cursor_position().is_some_and(|cursor| {
        ui::scene_input_rect(&window, &drawer, &editor.sidebar).contains(cursor)
            && !(matches!(editor.selection, Some(Selection::Camera(_)))
                && ui::camera_preview_rect(&window, &drawer, &editor.sidebar).contains(cursor))
    }) {
        return;
    }
    let Some(cursor) = cursor.filter(|point| {
        camera
            .logical_viewport_rect()
            .is_some_and(|rect| rect.contains(*point))
    }) else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(&global, cursor) else {
        return;
    };
    if let Some(selection) = editor.selection
        && !editor.object_hidden(selection)
        && let Some(pose) = editor.pose(selection)
    {
        let pose = pose.transform();
        let rotation_radius = rotation_radius(camera, view, pose.translation);
        pointer.hover = handle_at(
            &editor,
            selection,
            pose,
            camera,
            view,
            cursor,
            rotation_radius,
        );
        if let Some(handle) = pointer.hover
            && mouse.just_pressed(MouseButton::Left)
        {
            let (tool, axis, direction) = match handle {
                TransformHandle::Axis { tool, axis } => {
                    (tool, axis, direction(&editor, pose, axis))
                }
                TransformHandle::Plane { normal } => {
                    (Tool::Move, normal, direction(&editor, pose, normal))
                }
                TransformHandle::ViewRotation => (Tool::Rotate, 0, *view.back()),
            };
            let length = axis_length(editor.tool, tool, length(camera, view, pose.translation));
            let pixels_per_metre = camera
                .world_to_viewport(&global, pose.translation + direction * length)
                .and_then(|end| {
                    camera
                        .world_to_viewport(&global, pose.translation)
                        .map(|start| (end - start) / length)
                })
                .unwrap_or(Vec2::X);
            pointer.drag = Some(Drag {
                selection,
                handle,
                tool,
                axis,
                direction,
                frame: editor.gizmo_rotation(pose),
                before: editor.edits.clone(),
                start: pose,
                cursor,
                pixels_per_metre,
                scale_length: if tool == Tool::Rotate {
                    rotation_radius
                } else {
                    length
                },
                ring_start: rotation_direction(camera, view, cursor, pose.translation, direction),
                plane_start: matches!(handle, TransformHandle::Plane { .. })
                    .then(|| plane_point(ray, pose.translation, direction))
                    .flatten(),
            });
            return;
        }
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    // Transform tools keep the current selection, including clicks on empty
    // space. Only the cursor mode performs scene picking or deselection.
    if editor.tool != Tool::Universal {
        return;
    }

    let camera_hit = shots(&editor).find(|&shot| {
        let position = editor.camera(shot).pose.transform().translation;
        position.distance_squared(view.translation) > 0.04
            && camera
                .world_to_viewport(&global, position)
                .is_ok_and(|point| point.distance(cursor) < 9.0)
    });
    let light_hit = super::lighting_gizmos::hit(&editor, camera, view, cursor);
    let target = camera_hit
        .map(Selection::Camera)
        .or_else(|| light_hit.map(Selection::Light))
        .or_else(|| {
            let find = |entity| {
                std::iter::once(entity)
                    .chain(parents.iter_ancestors(entity))
                    .find_map(|entity| {
                        editor
                            .imported
                            .iter()
                            .position(|node| node.entity == entity)
                            .map(Selection::Imported)
                            .or_else(|| {
                                scene.models.iter().find_map(|(&id, (root, _))| {
                                    (*root == entity).then_some(Selection::Model(id))
                                })
                            })
                    })
            };
            let filter = |entity| find(entity).is_some_and(|target| !editor.object_hidden(target));
            let settings = MeshRayCastSettings::default().with_filter(&filter);
            meshes
                .cast_ray(ray, &settings)
                .first()
                .and_then(|(entity, _)| find(*entity))
        });
    editor.select(target);
}

pub(super) fn shots(editor: &Editor) -> std::ops::Range<usize> {
    0..if editor.has_scene() {
        editor.definition().cameras.len()
    } else {
        0
    }
}

pub(super) fn draw(
    editor: Res<Editor>,
    pointer: Res<PointerState>,
    camera: Query<(&Camera, &Transform), With<EditorCamera>>,
    mut handles: Gizmos<TransformGizmos>,
) {
    let Ok((camera, view)) = camera.single() else {
        return;
    };
    if editor.pilot || !editor.ready || !camera.is_active {
        return;
    }
    super::lighting_gizmos::draw(&editor, camera, view, &mut handles);
    for shot in shots(&editor) {
        let placement = editor.camera(shot);
        let pose = placement.pose.transform();
        if pose.translation.distance_squared(view.translation) <= 0.04 {
            continue;
        }
        let selected = editor.selection == Some(Selection::Camera(shot));
        let size = length(camera, view, pose.translation) * if selected { 0.43 } else { 0.14 };
        let forward = *pose.forward() * size;
        let right = *pose.right() * size * 0.70;
        let up = *pose.up() * size * 0.45;
        let origin = pose.translation;
        let corners = [right + up, right - up, -right - up, -right + up]
            .map(|offset| origin + forward + offset);
        let color = if selected { HIGHLIGHT } else { CAMERA_COLOR };
        handles.lineloop(corners, color);
        for corner in corners {
            handles.line(origin, corner, color);
        }
        if selected {
            let orthographic =
                editor.definition().cameras[shot].projection == LensKind::Orthographic;
            let corners_at = |depth: f32| {
                let half_height = if orthographic {
                    placement.lens * 0.5
                } else {
                    (placement.lens.to_radians() * 0.5).tan() * depth
                };
                let right = *pose.right() * half_height * CAMERA_ASPECT;
                let up = *pose.up() * half_height;
                let center = origin + *pose.forward() * depth;
                [right + up, right - up, -right - up, -right + up].map(|offset| center + offset)
            };
            let near = corners_at(0.05);
            let far = corners_at(3.0);
            handles.lineloop(near, CAMERA_COLOR);
            handles.lineloop(far, HIGHLIGHT);
            for (start, end) in near.into_iter().zip(far) {
                handles.line(start, end, CAMERA_COLOR);
            }
        }
    }
    let Some(selection) = editor.selection else {
        return;
    };
    if editor.object_hidden(selection) {
        return;
    }
    let Some(pose) = editor.pose(selection) else {
        return;
    };
    let pose = pose.transform();
    let pivot = pose.translation;
    let frame = editor.gizmo_rotation(pose);
    if editor.tool.operations().contains(&Tool::Rotate) {
        let active = pointer
            .drag
            .as_ref()
            .filter(|drag| drag.tool == Tool::Rotate && drag.selection == selection);
        // Geometry can rotate during a drag; keep its handle radius stable.
        let radius = active.map_or_else(
            || rotation_radius(camera, view, pivot),
            |drag| drag.scale_length,
        );
        for (axis, color) in ROTATION_COLORS.into_iter().enumerate() {
            if !allowed(&editor, selection, Tool::Rotate, axis) {
                continue;
            }
            let mut color = axis_color(&pointer, Tool::Rotate, axis, color);
            if active.is_some_and(|drag| {
                drag.handle
                    != (TransformHandle::Axis {
                        tool: Tool::Rotate,
                        axis,
                    })
            }) {
                color = color.with_alpha(0.25);
            }
            for (a, b) in universal_ring_segments(pivot, axis, radius, view, frame) {
                handles.line(a, b, color);
            }
        }
    }
}

fn axis_color(pointer: &PointerState, tool: Tool, axis: usize, color: Color) -> Color {
    handle_color(pointer, TransformHandle::Axis { tool, axis }, color)
}

fn handle_color(pointer: &PointerState, handle: TransformHandle, color: Color) -> Color {
    if pointer.hover == Some(handle)
        || pointer
            .drag
            .as_ref()
            .is_some_and(|drag| drag.handle == handle)
    {
        HIGHLIGHT
    } else {
        color
    }
}
