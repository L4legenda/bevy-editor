//! Frame thumbnail subjects rather than their scene's potentially huge backdrop.
use bevy::{camera::primitives::Aabb, prelude::*};

use crate::BoxBounds;

use super::scene;

const FIELD_OF_VIEW: f32 = 35.0_f32.to_radians();
const FRAME_MARGIN: f32 = 1.15;

/// Find world bounds while the job is still hidden. Editable roots and their
/// descendants take precedence; geometry outside them remains the fallback.
pub(super) fn bounds(
    root: Entity,
    subjects: &Query<(), With<scene::Subject>>,
    children: &Query<&Children>,
    meshes: &Query<(&Aabb, &GlobalTransform)>,
) -> Option<BoxBounds> {
    let empty = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut all = empty;
    let mut focused = empty;
    let mut pending = vec![(root, false)];
    while let Some((entity, ancestor_is_subject)) = pending.pop() {
        let subject = ancestor_is_subject || subjects.contains(entity);
        if let Ok(nodes) = children.get(entity) {
            pending.extend(nodes.iter().map(|child| (child, subject)));
        }
        let Ok((local_bounds, world)) = meshes.get(entity) else {
            continue;
        };
        let affine = world.affine();
        let center = affine.transform_point3(local_bounds.center.into());
        let half = Vec3::from(affine.matrix3.abs() * local_bounds.half_extents.abs());
        let min = center - half;
        let max = center + half;
        if !min.is_finite() || !max.is_finite() {
            continue;
        }
        for (lower, upper) in std::iter::once(&mut all).chain(subject.then_some(&mut focused)) {
            *lower = lower.min(min);
            *upper = upper.max(max);
        }
    }
    let (min, max) = if focused.0.is_finite() && focused.1.is_finite() {
        focused
    } else {
        all
    };
    (min.is_finite() && max.is_finite()).then(|| BoxBounds {
        center: ((min + max) * 0.5).to_array(),
        half_size: ((max - min) * 0.5).max(Vec3::splat(0.00001)).to_array(),
    })
}

/// Preserve the supplied target-to-camera direction and move the camera just
/// far enough to contain every world-AABB corner, with fifteen percent margin.
pub(super) fn fit(bounds: BoxBounds, direction: Vec3, aspect: f32) -> (Transform, Projection) {
    let center = Vec3::from(bounds.center);
    let center = if center.is_finite() {
        center
    } else {
        Vec3::ZERO
    };
    let half = Vec3::from(bounds.half_size);
    let half = if half.is_finite() {
        half.abs().max(Vec3::splat(0.00001))
    } else {
        Vec3::splat(0.5)
    };
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let back = direction
        .try_normalize()
        .unwrap_or_else(|| Vec3::new(1.0, 0.7, 1.0).normalize());
    let up = if back.dot(Vec3::Y).abs() > 0.98 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let mut view = Transform::IDENTITY.looking_to(-back, up);
    let right = *view.right();
    let up = *view.up();
    let tan_vertical = (FIELD_OF_VIEW * 0.5).tan();
    let tan_horizontal = tan_vertical * aspect;
    let radius = half.length();
    let near = (radius * 0.002).max(0.0000001);
    let depth_padding = (radius * 0.03).max(near * 2.0);
    let mut distance = depth_padding;
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let corner = half * Vec3::new(x, y, z);
                let horizontal = corner.dot(right).abs() / tan_horizontal;
                let vertical = corner.dot(up).abs() / tan_vertical;
                distance = distance.max(
                    corner.dot(back) + FRAME_MARGIN * horizontal.max(vertical) + depth_padding,
                );
            }
        }
    }
    view.translation = center + back * distance;
    let far = distance + radius + depth_padding;
    (
        view,
        Projection::Perspective(PerspectiveProjection {
            fov: FIELD_OF_VIEW,
            aspect_ratio: aspect,
            near,
            far,
            near_clip_plane: Vec4::new(0.0, 0.0, -1.0, -near),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{camera::CameraProjection, ecs::system::SystemState};

    fn measured(world: &mut World, root: Entity) -> Option<BoxBounds> {
        let mut state = SystemState::<(
            Query<(), With<scene::Subject>>,
            Query<&Children>,
            Query<(&Aabb, &GlobalTransform)>,
        )>::new(world);
        let (subjects, children, meshes) = state.get(world).unwrap();
        bounds(root, &subjects, &children, &meshes)
    }

    fn cube(half: f32) -> Aabb {
        Aabb::from_min_max(Vec3::splat(-half), Vec3::splat(half))
    }

    #[test]
    fn hidden_subject_bounds_include_root_meshes_and_transformed_descendants() {
        let mut world = World::new();
        let root = world.spawn(Visibility::Hidden).id();
        world.spawn((cube(1000.0), GlobalTransform::IDENTITY, ChildOf(root)));
        let subject = world
            .spawn((
                scene::Subject,
                Aabb::from_min_max(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5)),
                GlobalTransform::from(
                    Transform::from_xyz(4.0, 1.0, -2.0)
                        .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
                        .with_scale(Vec3::new(2.0, 1.0, 3.0)),
                ),
                ChildOf(root),
            ))
            .id();
        world.spawn((
            cube(0.25),
            GlobalTransform::from_translation(Vec3::new(7.0, 1.0, -2.0)),
            ChildOf(subject),
        ));

        let bounds = measured(&mut world, root).unwrap();
        assert!(Vec3::from(bounds.center).abs_diff_eq(Vec3::new(4.875, 1.375, -2.0), 1e-5));
        assert!(Vec3::from(bounds.half_size).abs_diff_eq(Vec3::new(2.375, 0.625, 1.0), 1e-5));
    }

    #[test]
    fn empty_subjects_fall_back_to_all_geometry_including_the_job_root() {
        let mut world = World::new();
        let root = world
            .spawn((cube(1.0), GlobalTransform::IDENTITY, Visibility::Hidden))
            .id();
        world.spawn((scene::Subject, ChildOf(root)));
        world.spawn((
            cube(0.5),
            GlobalTransform::from_translation(Vec3::new(4.0, 0.0, 0.0)),
            ChildOf(root),
        ));
        world.spawn((
            cube(1000.0),
            GlobalTransform::from_translation(Vec3::splat(f32::NAN)),
            ChildOf(root),
        ));

        let bounds = measured(&mut world, root).unwrap();
        assert_eq!(bounds.center, [1.75, 0.0, 0.0]);
        assert_eq!(bounds.half_size, [2.75, 1.0, 1.0]);
        assert!(measured(&mut world, Entity::PLACEHOLDER).is_none());
    }

    fn assert_corners_fit(bounds: BoxBounds, direction: Vec3, aspect: f32) -> f32 {
        let (view, projection) = fit(bounds, direction, aspect);
        let Projection::Perspective(perspective) = projection else {
            panic!("expected perspective")
        };
        let view_from_world = view.compute_affine().inverse();
        let clip_from_view = perspective.get_clip_from_view();
        let center = Vec3::from(bounds.center);
        let half = Vec3::from(bounds.half_size);
        let mut largest: f32 = 0.0;
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    let corner = center + half * Vec3::new(x, y, z);
                    let camera_point = view_from_world.transform_point3(corner);
                    assert!(-camera_point.z > perspective.near);
                    assert!(-camera_point.z < perspective.far);
                    let projected = clip_from_view.project_point3(camera_point);
                    assert!(projected.is_finite());
                    let edge = projected.x.abs().max(projected.y.abs());
                    assert!(
                        edge <= 1.0 / FRAME_MARGIN + 1e-4,
                        "corner outside margin: {edge}"
                    );
                    largest = largest.max(edge);
                }
            }
        }
        largest
    }

    #[test]
    fn perspective_fit_contains_wide_tall_flat_and_deep_assets_with_a_tight_margin() {
        for (half, direction, aspect) in [
            ([9.0, 0.1, 0.15], Vec3::Z, 0.55),
            ([0.1, 7.0, 0.1], Vec3::new(1.0, 0.4, 1.0), 1.6),
            ([4.0, 0.01, 3.0], Vec3::new(1.0, 0.7, 1.0), 1.6),
            ([0.3, 0.5, 12.0], Vec3::new(1.0, 0.2, 0.5), 2.0),
        ] {
            let largest = assert_corners_fit(
                BoxBounds {
                    center: [10.0, 2.0, -5.0],
                    half_size: half,
                },
                direction,
                aspect,
            );
            assert!(
                largest > 0.83,
                "asset was framed unnecessarily far away: {largest}"
            );
        }
    }

    #[test]
    fn framing_preserves_vertical_directions_and_scales_clip_planes_with_asset_size() {
        for scale in [0.0001, 1.0, 1_000_000.0] {
            let bounds = BoxBounds {
                center: [0.0; 3],
                half_size: [scale, scale * 0.5, scale * 2.0],
            };
            for direction in [Vec3::Y, Vec3::NEG_Y, Vec3::new(-1.0, 0.8, 2.0)] {
                assert_corners_fit(bounds, direction, 1.6);
                let (view, projection) = fit(bounds, direction, 1.6);
                assert!(view.back().dot(direction.normalize()) > 0.9999);
                let Projection::Perspective(projection) = projection else {
                    unreachable!()
                };
                assert!(projection.far > scale * 2.0);
                assert!(projection.near > 0.0 && projection.near < projection.far);
            }
        }
        let (view, projection) = fit(
            BoxBounds {
                center: [0.0; 3],
                half_size: [1.0; 3],
            },
            Vec3::ZERO,
            f32::NAN,
        );
        assert!(view.translation.is_finite() && view.rotation.is_finite());
        let Projection::Perspective(projection) = projection else {
            unreachable!()
        };
        assert_eq!(projection.aspect_ratio, 1.0);
    }
}
