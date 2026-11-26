//! The model workspace uses the same fly/orbit/pan gestures as the scene view.
use super::*;
use super::super::image_viewer::paint_input::PenPan;
use bevy::{
    picking::{events::PointerClick, pointer::{PointerButton, PointerId}},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::RequestRedraw,
};
use bevy_scene_format::{Revision, atomic_write, read_limited};
use std::{io, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Settings {
    pub orbit_speed: f32,
    pub zoom_speed: f32,
    pub invert_zoom: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { orbit_speed: 1., zoom_speed: 1., invert_zoom: false }
    }
}

impl Settings {
    pub const MIN_SPEED: f32 = 0.1;
    pub const MAX_SPEED: f32 = 5.;

    pub fn zoom_scale(self) -> f32 {
        if self.invert_zoom { -self.zoom_speed } else { self.zoom_speed }
    }
}

pub(super) struct Preferences {
    path: Option<PathBuf>,
    pub settings: Settings,
}

impl Default for Preferences {
    fn default() -> Self {
        let path = paint::presets::directory()
            .and_then(|folder| folder.parent().map(|root| root.join("paint-navigation.json")));
        Self::open(path)
    }
}

impl Preferences {
    pub fn open(path: Option<PathBuf>) -> Self {
        let mut settings: Settings = path.as_deref()
            .and_then(|path| read_limited(path, 4096).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        settings.orbit_speed = settings.orbit_speed.clamp(Settings::MIN_SPEED, Settings::MAX_SPEED);
        settings.zoom_speed = settings.zoom_speed.clamp(Settings::MIN_SPEED, Settings::MAX_SPEED);
        Self { path, settings }
    }

    pub fn save(&mut self, settings: Settings) -> io::Result<()> {
        if self.settings != settings {
            if let Some(path) = &self.path {
                atomic_write(path, &serde_json::to_vec(&settings)?, Revision::read(path)?)?;
            }
            self.settings = settings;
        }
        Ok(())
    }
}

pub(super) struct OrbitFocus {
    model: usize,
    geometry: u64,
    visibility: u64,
    camera: ModelCamera,
    task: Task<Option<Vec3>>,
}

pub(super) fn focus_orbit(
    mut click: On<PointerClick>,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<paint::PaintState>,
    maker: Res<paint::ShaderMaker>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<DrawerMotion>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    mut viewer: ResMut<Viewer>,
) {
    if click.count != 2 || click.button != PointerButton::Primary || click.pointer.id != PointerId::Mouse
        || !keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
        || keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight])
        || !window.focused || editor.pending.is_some() || editor.viewing_image.is_some()
        || editor.browser_popup || editor.hdri_library_open || drawer.resizing
    {
        return;
    }
    let Some(model) = editor.viewing_model.filter(|&model| {
        viewer.fitted && viewer.model == Some(model)
            && editing.mode(Some(model)) == crate::editor::model_document::Mode::Paint
    }) else { return; };
    if paint.blocks_view_navigation(Some(model), maker.shows_model())
        || !paint.has_3d_view(Some(model), maker.shows_model())
    {
        return;
    }
    let Ok((node, layout)) = canvas.get(click.entity) else { return; };
    let scale = node.inverse_scale_factor();
    let point = click.pointer.position;
    let rect = Rect::from_center_size(layout.affine().translation * scale, node.size() * scale);
    if !rect.contains(point) || rect.size().min_element() < 1. || drawer.covers(point, &window) {
        return;
    }
    let (Some(doc), Some(visibility)) = (editing.documents.get(&model), paint.set_visibility(model)) else { return; };
    let objects = doc.objects.clone();
    let visibility = visibility.clone();
    let (origin, direction) = editing::input::ray(&viewer, rect, point);
    viewer.orbit_focus = Some(OrbitFocus {
        model,
        geometry: doc.revision,
        visibility: visibility.revision,
        camera: viewer.camera(),
        task: AsyncComputeTaskPool::get().spawn(async move {
            editing::input::nearest_on_objects(
                &objects, origin, direction, |object, slot| visibility.shown(object, slot),
            )
        }),
    });
    click.propagate(false);
}

pub(super) fn finish_orbit_focus(
    viewer: &mut Viewer,
    model: Option<usize>,
    editing: &editing::State,
    paint: &paint::PaintState,
    allowed: bool,
    redraw: &mut MessageWriter<RequestRedraw>,
) {
    let Some(mut focus) = viewer.orbit_focus.take() else { return; };
    if !allowed || model != Some(focus.model) || viewer.camera() != focus.camera
        || !editing.documents.get(&focus.model).is_some_and(|doc| doc.revision == focus.geometry)
        || !paint.set_visibility(focus.model).is_some_and(|state| state.revision == focus.visibility)
    {
        return;
    }
    match block_on(poll_once(&mut focus.task)) {
        Some(Some(point)) => viewer.center = point,
        Some(None) => {}
        None => {
            viewer.orbit_focus = Some(focus);
            redraw.write(RequestRedraw);
        }
    }
}

pub(super) fn background(
    shading: Res<shading::Shading>,
    cameras: Query<(&Transform, &Projection, Option<&bevy::light::Skybox>), With<ViewCamera>>,
    images: Res<Assets<Image>>,
    mut backgrounds: Query<
        (&mut Transform, &mut Visibility),
        (With<ModelBackground>, Without<ViewCamera>),
    >,
) {
    let view = cameras.single().ok();
    let panorama = view
        .filter(|(_, projection, _)| !matches!(projection, Projection::Orthographic(_)))
        .and_then(|(_, _, sky)| sky)
        .and_then(|sky| sky.image.as_ref())
        .is_some_and(|image| images.contains(image));
    for (mut pose, mut visibility) in &mut backgrounds {
        *visibility = if view.is_some() && !panorama && shading.mode == shading::Mode::Rendered {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some((camera, _, _)) = view {
            pose.translation = camera.translation;
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Navigation {
    Fly,
    Pan,
    PenPan,
    Orbit,
    Zoom,
}

pub(super) fn drive(
    viewer: &mut Viewer,
    window: &Window,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    scroll: &AccumulatedMouseScroll,
    dt: f32,
    motion: Vec2,
    rect: Rect,
    blocked: bool,
    pen: Option<PenPan>,
    paint_settings: Option<Settings>,
) {
    let settings = paint_settings.unwrap_or_default();
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if blocked || !window.focused || command || keys.just_pressed(KeyCode::Escape) {
        viewer.gesture = None;
        return;
    }
    let cursor = pen.map(|pan| pan.point)
        .or_else(|| window.cursor_position())
        .or_else(|| viewer.gesture.map(|(_, point)| point));
    let Some(cursor) = cursor else {
        return;
    };
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if let Some(pan) = pen {
        if let Some(start) = pan.start.filter(|point| rect.contains(*point)) {
            viewer.gesture = Some((Navigation::PenPan, start));
        } else if !viewer.gesture.is_some_and(|(nav, _)| matches!(nav, Navigation::PenPan)) {
            viewer.gesture = None;
        }
    } else if rect.contains(cursor) {
        let nav = if mouse.just_pressed(MouseButton::Right) && alt && paint_settings.is_some() {
            Some(Navigation::Zoom)
        } else if mouse.just_pressed(MouseButton::Right) {
            Some(Navigation::Fly)
        } else if mouse.just_pressed(MouseButton::Middle) {
            Some(Navigation::Pan)
        } else if alt && mouse.just_pressed(MouseButton::Left) {
            Some(Navigation::Orbit)
        } else {
            None
        };
        if let Some(nav) = nav {
            viewer.gesture = Some((nav, cursor));
        }
    }
    if let Some((nav, previous)) = viewer.gesture {
        let held = match nav {
            Navigation::Fly => mouse.pressed(MouseButton::Right),
            Navigation::Pan => mouse.pressed(MouseButton::Middle),
            Navigation::PenPan => pen.is_some(),
            Navigation::Orbit => mouse.pressed(MouseButton::Left),
            Navigation::Zoom => alt && mouse.pressed(MouseButton::Right),
        };
        if held {
            let delta = if matches!(nav, Navigation::PenPan) {
                cursor - previous
            } else if motion != Vec2::ZERO {
                motion
            } else {
                cursor - previous
            };
            match nav {
                Navigation::Fly => {
                    let eye = view_pose(viewer).translation;
                    if delta != Vec2::ZERO {
                        viewer.orthographic = false;
                        viewer.yaw -= delta.x * 0.004;
                        viewer.pitch = (viewer.pitch + delta.y * 0.004).clamp(-1.53, 1.53);
                    }
                    let pose = view_pose(viewer);
                    let input = |p, n| f32::from(keys.pressed(p)) - f32::from(keys.pressed(n));
                    let movement = *pose.right() * input(KeyCode::KeyD, KeyCode::KeyA)
                        + *pose.forward() * input(KeyCode::KeyW, KeyCode::KeyS)
                        + Vec3::Y * input(KeyCode::KeyE, KeyCode::KeyQ);
                    if movement != Vec3::ZERO {
                        viewer.orthographic = false;
                    }
                    let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                        11.0
                    } else {
                        4.0
                    };
                    viewer.center = eye
                        + movement.normalize_or_zero() * speed * dt.min(0.05)
                        + *pose.forward() * viewer.distance;
                }
                Navigation::Pan | Navigation::PenPan => {
                    let pose = view_pose(viewer);
                    viewer.center += (*pose.right() * -delta.x + *pose.up() * delta.y)
                        * viewer.distance
                        * 0.0016;
                }
                Navigation::Orbit => {
                    if delta != Vec2::ZERO {
                        viewer.orthographic = false;
                        let rate = 0.006 * settings.orbit_speed;
                        viewer.yaw -= delta.x * rate;
                        viewer.pitch = (viewer.pitch + delta.y * rate).clamp(-1.5, 1.5);
                    }
                }
                Navigation::Zoom => {
                    if delta.y != 0. {
                        zoom(viewer, delta.y * 0.01 * settings.zoom_speed);
                    }
                }
            }
            viewer.gesture = Some((nav, cursor));
        } else {
            viewer.gesture = None;
        }
    }
    if pen.is_none() && rect.contains(cursor) && scroll.delta.y != 0.0 {
        let factor = if scroll.unit == MouseScrollUnit::Line {
            0.13
        } else {
            0.002
        };
        zoom(viewer, -scroll.delta.y * factor * settings.zoom_scale());
    }
}

pub(super) fn zoom(viewer: &mut Viewer, exponent: f32) {
    viewer.distance = (viewer.distance * exponent.exp()).clamp(
        (viewer.radius * 0.06).clamp(0.000001, 0.35),
        (viewer.radius * 1000.0).max(80.0),
    );
}
