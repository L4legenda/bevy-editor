//! The model workspace uses the same fly/orbit/pan gestures as the scene view.
use super::*;
use super::super::image_viewer::paint_input::PenPan;

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
) {
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
        let nav = if mouse.just_pressed(MouseButton::Right) {
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
                        viewer.yaw -= delta.x * 0.006;
                        viewer.pitch = (viewer.pitch + delta.y * 0.006).clamp(-1.5, 1.5);
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
        zoom(viewer, -scroll.delta.y * factor);
    }
}

pub(super) fn zoom(viewer: &mut Viewer, exponent: f32) {
    viewer.distance = (viewer.distance * exponent.exp()).clamp(
        (viewer.radius * 0.06).clamp(0.000001, 0.35),
        (viewer.radius * 1000.0).max(80.0),
    );
}
