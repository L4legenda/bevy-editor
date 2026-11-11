//! Convert a browser drop into a bounded position on the actual scene geometry.

use bevy::{
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
    shape::Ray3d,
};

use super::Scene;

const MAX_DISTANCE: f32 = 200.0;
const POSITION_LIMIT: f32 = 1000.0;
const FALLBACK_DISTANCE: f32 = 5.0;
const SNAP_STEP: f32 = 0.25;

fn bounded(point: Vec3) -> bool {
    point.is_finite() && point.abs().max_element() <= POSITION_LIMIT
}

/// Prefer an actual mesh intersection. Otherwise use the ground beneath the
/// cursor, or a short distance along the ray when the ground is behind the eye
/// or too close to parallel. Near-horizon rays never create distant objects.
pub(super) fn position_from_ray(ray: Ray3d, surface: Option<Vec3>) -> Option<Vec3> {
    if !ray.origin.is_finite() || !(*ray.direction).is_finite() {
        return None;
    }
    if let Some(point) = surface.filter(|&point| {
        let offset = point - ray.origin;
        bounded(point)
            && offset.dot(*ray.direction) >= 0.0
            && offset.length_squared() <= MAX_DISTANCE * MAX_DISTANCE
    }) {
        return Some(point);
    }
    if ray.direction.y.abs() > 1e-4 {
        let distance = -ray.origin.y / ray.direction.y;
        if (0.0..=MAX_DISTANCE).contains(&distance) {
            let point = ray.get_point(distance).with_y(0.0);
            if bounded(point) {
                return Some(point);
            }
        }
    }
    let point = ray.get_point(FALLBACK_DISTANCE);
    bounded(point).then_some(point)
}

pub(super) fn snap_position(point: Vec3, enabled: bool) -> Vec3 {
    if enabled {
        (point / SNAP_STEP).round() * SNAP_STEP
    } else {
        point
    }
}

/// `cursor` is in logical coordinates of the scene's render target, including
/// any camera viewport offset. The caller subtracts the editor toolbar offset
/// from the window cursor and rejects drops on editor overlays first.
pub(super) fn drop_point(
    camera: &Camera,
    view: &Transform,
    cursor: Vec2,
    meshes: &mut MeshRayCast,
    scene: &Scene,
    parents: &Query<&ChildOf>,
    snap: bool,
) -> Option<Vec3> {
    if !cursor.is_finite()
        || !camera
            .logical_viewport_rect()
            .is_some_and(|viewport| viewport.contains(cursor))
    {
        return None;
    }
    let ray = camera
        .viewport_to_world(&GlobalTransform::from(*view), cursor)
        .ok()?;
    // Room geometry includes fixed architecture as well as editable nodes.
    // Thumbnail instances, preview-only models and other editor meshes have
    // independent roots, so they can never become placement surfaces.
    let filter = |entity| {
        std::iter::once(entity)
            .chain(parents.iter_ancestors(entity))
            .any(|ancestor| {
                Some(ancestor) == scene.root
                    || scene.models.values().any(|(root, _)| *root == ancestor)
            })
    };
    let settings = MeshRayCastSettings::default().with_filter(&filter);
    let surface = meshes
        .cast_ray(ray, &settings)
        .first()
        .map(|(_, hit)| hit.point);
    let point = position_from_ray(ray, surface)?;
    let snapped = snap_position(point, snap);
    let offset = snapped - ray.origin;
    // At the near plane, rounding can cross behind the ray origin. Keep the
    // unsnapped point in that edge case rather than placing behind the camera.
    Some(
        if bounded(snapped)
            && offset.dot(*ray.direction) >= 0.0
            && offset.length_squared() <= MAX_DISTANCE * MAX_DISTANCE
        {
            snapped
        } else {
            point
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray(origin: Vec3, direction: Vec3) -> Ray3d {
        Ray3d::new(origin, Dir3::new(direction).unwrap())
    }

    #[test]
    fn a_scene_surface_wins_over_the_ground_plane() {
        let ray = ray(Vec3::new(1.0, 5.0, 2.0), Vec3::NEG_Y);
        let surface = Vec3::new(1.0, 3.0, 2.0);
        assert_eq!(position_from_ray(ray, Some(surface)), Some(surface));
    }

    #[test]
    fn an_empty_drop_intersects_ground_at_the_cursor_ray() {
        let ray = ray(Vec3::new(2.0, 4.0, 6.0), Vec3::new(0.0, -1.0, -1.0));
        let point = position_from_ray(ray, None).unwrap();
        assert!(point.abs_diff_eq(Vec3::new(2.0, 0.0, 2.0), 1e-5));
    }

    #[test]
    fn parallel_and_upward_rays_fall_back_in_front_of_the_camera() {
        let origin = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(
            position_from_ray(ray(origin, Vec3::X), None),
            Some(origin + Vec3::X * FALLBACK_DISTANCE)
        );
        assert_eq!(
            position_from_ray(ray(origin, Vec3::Y), None),
            Some(origin + Vec3::Y * FALLBACK_DISTANCE)
        );
    }

    #[test]
    fn behind_eye_invalid_and_out_of_range_surfaces_are_ignored() {
        let horizontal = ray(Vec3::new(0.0, 2.0, 0.0), Vec3::X);
        let fallback = Some(Vec3::new(FALLBACK_DISTANCE, 2.0, 0.0));
        for surface in [
            Vec3::new(-2.0, 2.0, 0.0),
            Vec3::new(MAX_DISTANCE + 1.0, 2.0, 0.0),
            Vec3::new(1.0, 2.0, MAX_DISTANCE + 1.0),
            Vec3::NAN,
        ] {
            assert_eq!(position_from_ray(horizontal, Some(surface)), fallback);
        }
        let grazing = ray(Vec3::new(0.0, 2.0, 0.0), Vec3::new(1.0, -0.001, 0.0));
        let point = position_from_ray(grazing, None).unwrap();
        assert!((point - grazing.origin).length() < FALLBACK_DISTANCE + 1e-4);
    }

    #[test]
    fn non_finite_or_unbounded_placements_are_rejected() {
        assert!(position_from_ray(ray(Vec3::NAN, Vec3::X), None).is_none());
        assert!(position_from_ray(ray(Vec3::splat(POSITION_LIMIT + 1.0), Vec3::Y), None).is_none());
    }

    #[test]
    fn snapping_uses_the_existing_quarter_metre_grid_on_every_axis() {
        let point = Vec3::new(0.12, 1.13, -0.38);
        assert_eq!(snap_position(point, false), point);
        assert_eq!(snap_position(point, true), Vec3::new(0.0, 1.25, -0.5));
    }
}
