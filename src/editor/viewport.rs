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
    shape::Ray3d,
    ui_widgets::TextInput,
    window::PrimaryWindow,
};

use crate::{data::LevelEdits as RoomEdits, project::LensKind};

use super::{Editor, Field, Scene, Selection, Shot, Tool, ui};

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const AXIS_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.28, 0.29),
    Color::srgb(0.33, 0.89, 0.49),
    Color::srgb(0.32, 0.62, 1.0),
];
const HIGHLIGHT: Color = Color::srgb(1.0, 0.81, 0.38);
const CAMERA_COLOR: Color = Color::srgba(0.49, 0.72, 1.0, 0.72);
const EDITOR_FOV: f32 = 58.0_f32.to_radians();
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

#[derive(Resource, Default)]
pub(super) struct PointerState {
    nav: Option<Navigation>,
    pilot: Option<Shot>,
    pivot: Vec3,
    drag: Option<Drag>,
    hover: Option<usize>,
}

impl PointerState {
    pub(super) fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub(super) fn stop_navigation(&mut self) {
        self.nav = None;
        self.pilot = None;
    }
}

struct Drag {
    selection: Selection,
    tool: Tool,
    axis: usize,
    direction: Vec3,
    before: RoomEdits,
    start: Transform,
    cursor: Vec2,
    pixels_per_metre: Vec2,
    scale_length: f32,
    ring_start: Option<Vec3>,
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
    camera.insert(AmbientLight {
        brightness: 150.0,
        ..default()
    });
    if let Some(setup) = editor.project.camera_setup {
        let logical =
            crate::asset_paths::original_asset_path(&editor.root, &editor.definition().asset)
                .unwrap_or_else(|_| editor.definition().asset.clone());
        setup(&mut camera, &logical);
    }
    camera.entry::<RenderLayers>().and_modify(|mut layers| {
        *layers = layers
            .clone()
            .with(EDITOR_GIZMO_LAYER)
            .without(super::selection_outline::LAYER);
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
    let rect = ui::viewport_rect(&window);
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
        state.pilot = shot;
        if let Some(shot) = shot {
            let view = editor.camera(shot).pose.transform();
            state.pivot = view.translation + *view.forward() * 3.0;
        }
    }
    if shot.is_none() && std::mem::take(&mut editor.focus) {
        editor.pivot = editor
            .selection
            .and_then(|target| editor.pose(target))
            .map_or(Vec3::ZERO, |target| target.transform().translation);
        let back = *editor.view.back();
        editor.view.translation =
            editor.pivot + back * if editor.selection.is_some() { 3.0 } else { 8.0 };
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
        && editor.ready
        && editor.pending.is_none()
        && !typing
        && !command
        && !drawer.resizing
        && !asset_drag.as_ref().is_some_and(|drag| drag.active());
    let enabled = enabled
        && !lighting
            .as_ref()
            .is_some_and(|lighting| lighting.blocks_scene_input());
    let preview_visible =
        editor.ready && !editor.pilot && matches!(editor.selection, Some(Selection::Camera(_)));
    let inside = enabled
        && window.cursor_position().is_some_and(|cursor| {
            ui::scene_input_rect(&window, &drawer).contains(cursor)
                && !(preview_visible && ui::camera_preview_rect(&window, &drawer).contains(cursor))
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

fn allowed(editor: &Editor, selection: Selection, tool: Tool, axis: usize) -> bool {
    editor.field_allowed(
        selection,
        match tool {
            Tool::Move => Field::Position(axis),
            Tool::Rotate => Field::Rotation(axis),
            Tool::Scale => Field::Scale(axis),
        },
    )
}

fn direction(pose: Transform, tool: Tool, axis: usize) -> Vec3 {
    if tool == Tool::Scale {
        pose.rotation * AXES[axis]
    } else {
        AXES[axis]
    }
}

fn ring_points(pivot: Vec3, axis: usize, radius: f32) -> impl Iterator<Item = Vec3> {
    let u = AXES[(axis + 1) % 3];
    let v = AXES[(axis + 2) % 3];
    (0..=48).map(move |index| {
        let (sin, cos) = (std::f32::consts::TAU * index as f32 / 48.0).sin_cos();
        pivot + (u * cos + v * sin) * radius
    })
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
) -> Option<usize> {
    let global = GlobalTransform::from(*view);
    let project = |point| camera.world_to_viewport(&global, point).ok();
    let pivot = pose.translation;
    let length = length(camera, view, pivot);
    let start = project(pivot)?;
    (0..3)
        .filter(|&axis| allowed(editor, selection, editor.tool, axis))
        .filter_map(|axis| {
            let distance = if editor.tool == Tool::Rotate {
                let points: Vec<_> = ring_points(pivot, axis, length * 0.83)
                    .map(project)
                    .collect();
                points
                    .windows(2)
                    .filter_map(|line| Some(distance_to_segment(cursor, line[0]?, line[1]?)))
                    .fold(f32::INFINITY, f32::min)
            } else {
                let end = project(pivot + direction(pose, editor.tool, axis) * length)?;
                if end.distance(start) < 13.0 {
                    return None;
                }
                distance_to_segment(cursor, start.lerp(end, 0.23), end)
            };
            (distance < 9.0).then_some((axis, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(axis, _)| axis)
}

fn ring_direction(ray: Ray3d, center: Vec3, normal: Vec3) -> Option<Vec3> {
    let denominator = ray.direction.dot(normal);
    if denominator.abs() < 1e-4 {
        return None;
    }
    let distance = (center - ray.origin).dot(normal) / denominator;
    (ray.origin + *ray.direction * distance - center).try_normalize()
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
                    Tool::Move => pose.translation += drag.direction * snap(delta, 0.25),
                    Tool::Scale => {
                        let original = pose.scale[drag.axis];
                        let scale = 1.0 + delta / drag.scale_length;
                        pose.scale[drag.axis] = original.signum()
                            * snap(original.abs() * scale, 0.1).clamp(0.01, 100.0);
                    }
                    Tool::Rotate => {
                        let angle =
                            drag.ring_start
                                .zip(camera.viewport_to_world(&global, cursor).ok().and_then(
                                    |ray| ring_direction(ray, pose.translation, drag.direction),
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
                if pose.translation.is_finite() && pose.translation.abs().max_element() <= 1000.0 {
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
        ui::scene_input_rect(&window, &drawer).contains(cursor)
            && !(matches!(editor.selection, Some(Selection::Camera(_)))
                && ui::camera_preview_rect(&window, &drawer).contains(cursor))
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
        && let Some(pose) = editor.pose(selection)
    {
        let pose = pose.transform();
        pointer.hover = handle_at(&editor, selection, pose, camera, view, cursor);
        if let Some(axis) = pointer.hover
            && mouse.just_pressed(MouseButton::Left)
        {
            let direction = direction(pose, editor.tool, axis);
            let length = length(camera, view, pose.translation);
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
                tool: editor.tool,
                axis,
                direction,
                before: editor.edits.clone(),
                start: pose,
                cursor,
                pixels_per_metre,
                scale_length: length,
                ring_start: ring_direction(ray, pose.translation, direction),
            });
            return;
        }
    }
    if !mouse.just_pressed(MouseButton::Left) {
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
            let filter = |entity| find(entity).is_some();
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
    let Some(pose) = editor.pose(selection) else {
        return;
    };
    let pose = pose.transform();
    let pivot = pose.translation;
    let length = length(camera, view, pivot);
    for (axis, color) in AXIS_COLORS.into_iter().enumerate() {
        if !allowed(&editor, selection, editor.tool, axis) {
            continue;
        }
        let color = if pointer.hover == Some(axis)
            || pointer.drag.as_ref().is_some_and(|drag| drag.axis == axis)
        {
            HIGHLIGHT
        } else {
            color
        };
        if editor.tool == Tool::Rotate {
            handles.linestrip(ring_points(pivot, axis, length * 0.83), color);
        } else {
            let end = pivot + direction(pose, editor.tool, axis) * length;
            if editor.tool == Tool::Move {
                handles
                    .arrow(pivot, end, color)
                    .with_tip_length(length * 0.15);
            } else {
                handles.line(pivot, end, color);
                handles.cube(
                    Transform::from_translation(end).with_scale(Vec3::splat(length * 0.12)),
                    color,
                );
            }
        }
    }
}
