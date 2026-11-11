//! Screen-sized light handles and the selected light's influence volume.
use bevy::prelude::*;

use super::{
    Editor, Selection,
    viewport::{self, TransformGizmos},
};
use crate::lighting::{LightKind, SceneLight};

pub(super) fn hit(editor: &Editor, camera: &Camera, view: &Transform, cursor: Vec2) -> Option<u32> {
    if !editor.show_lights {
        return None;
    }
    let global = GlobalTransform::from(*view);
    editor
        .edits
        .lights
        .iter()
        .filter_map(|light| {
            let position = Vec3::from(light.pose.position);
            if position.distance_squared(view.translation) < 0.04 {
                return None;
            }
            let point = camera.world_to_viewport(&global, position).ok()?;
            let distance = point.distance_squared(cursor);
            (distance <= 13.0_f32.powi(2)).then_some((light.id, distance))
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(id, _)| id)
}

fn color(light: &SceneLight, selected: bool) -> Color {
    if selected {
        return Color::srgb(1.0, 0.81, 0.38);
    }
    if !light.enabled {
        return Color::srgb(0.46, 0.47, 0.53);
    }
    let rgb = light.resolved_rgb();
    Color::srgb(rgb[0].max(0.30), rgb[1].max(0.30), rgb[2].max(0.30))
}

pub(super) fn draw(
    editor: &Editor,
    camera: &Camera,
    view: &Transform,
    gizmos: &mut Gizmos<TransformGizmos>,
) {
    if !editor.show_lights {
        return;
    }
    for light in &editor.edits.lights {
        let pose = light.pose.transform();
        let origin = pose.translation;
        if origin.distance_squared(view.translation) < 0.04 {
            continue;
        }
        let selected = editor.selection == Some(Selection::Light(light.id));
        let radius = viewport::length(camera, view, origin) * if selected { 0.17 } else { 0.12 };
        let color = color(light, selected);
        let ring = |radius: f32| {
            (0..24).map(move |step| {
                let angle = step as f32 * std::f32::consts::TAU / 24.0;
                origin + view.rotation * Vec3::new(angle.cos() * radius, angle.sin() * radius, 0.0)
            })
        };
        gizmos.lineloop(ring(radius * 0.55), color);
        for ray in 0..8 {
            let angle = ray as f32 * std::f32::consts::TAU / 8.0;
            let direction = view.rotation * Vec3::new(angle.cos(), angle.sin(), 0.0);
            gizmos.line(
                origin + direction * radius * 0.75,
                origin + direction * radius,
                color,
            );
        }
        if light.kind != LightKind::Point {
            gizmos
                .arrow(origin, origin + *pose.forward() * radius * 2.5, color)
                .with_tip_length(radius * 0.45);
        }
        if !selected {
            continue;
        }
        let faint = color.with_alpha(0.32);
        match light.kind {
            LightKind::Point => {
                for axis in 0..3 {
                    gizmos.lineloop(
                        (0..48).map(|step| {
                            let angle = step as f32 * std::f32::consts::TAU / 48.0;
                            let mut point = Vec3::ZERO;
                            point[(axis + 1) % 3] = angle.cos() * light.range;
                            point[(axis + 2) % 3] = angle.sin() * light.range;
                            origin + point
                        }),
                        faint,
                    );
                }
            }
            LightKind::Spot => {
                for (angle, color) in [
                    (light.outer_angle, faint),
                    (light.inner_angle, color.with_alpha(0.18)),
                ] {
                    let distance = light.range;
                    let width = distance * angle.to_radians().tan();
                    let points: Vec<_> = (0..32)
                        .map(|step| {
                            let theta = step as f32 * std::f32::consts::TAU / 32.0;
                            origin
                                + pose.rotation
                                    * Vec3::new(theta.cos() * width, theta.sin() * width, -distance)
                        })
                        .collect();
                    for index in [0, 8, 16, 24] {
                        gizmos.line(origin, points[index], color);
                    }
                    gizmos.lineloop(points, color);
                }
            }
            LightKind::Directional => {
                gizmos
                    .arrow(origin, origin + *pose.forward() * radius * 6.0, faint)
                    .with_tip_length(radius * 0.7);
            }
            LightKind::Rect => {
                let right = light.width * 0.5;
                let up = light.height * 0.5;
                gizmos.lineloop(
                    [
                        Vec3::new(-right, -up, 0.0),
                        Vec3::new(right, -up, 0.0),
                        Vec3::new(right, up, 0.0),
                        Vec3::new(-right, up, 0.0),
                    ]
                    .map(|point| origin + pose.rotation * point),
                    color,
                );
            }
        }
    }
}
