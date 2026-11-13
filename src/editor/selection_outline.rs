//! Orange corner brackets around the selected hierarchy, in its local axes.
use bevy::{
    camera::{primitives::Aabb, visibility::RenderLayers},
    gizmos::config::{GizmoConfigGroup, GizmoConfigStore},
    math::Affine3A,
    prelude::*,
};

use super::{Editor, Scene, Selection, viewport::EditorCamera};

#[cfg(test)]
#[path = "selection_outline_render_tests.rs"]
pub(crate) mod render_tests;

pub(super) const LAYER: usize = 4;
const COLOR: Color = Color::srgb(1.0, 0.64, 0.17);
const CORNER_FRACTION: f32 = 0.16;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct SelectionGizmos;

pub(super) fn setup(mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<SelectionGizmos>();
    config.render_layers = RenderLayers::layer(LAYER);
    config.depth_bias = -1.0;
    config.line.width = 1.5;
}

/// Bounds live in the selected root's space so rotating a model rotates its
/// brackets too. Transform all child bounds before merging, preserving shear.
fn bounds(
    root: Entity,
    layers: &RenderLayers,
    children: &Query<&Children>,
    transforms: &Query<&GlobalTransform>,
    meshes: &Query<
        (
            &Aabb,
            &GlobalTransform,
            &InheritedVisibility,
            Option<&RenderLayers>,
        ),
        With<Mesh3d>,
    >,
) -> Option<(Vec3, Vec3, Affine3A)> {
    let world = transforms.get(root).ok()?.affine();
    if !world.is_finite() || world.matrix3.determinant().abs() < 1e-8 {
        return None;
    }
    let inverse = world.inverse();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
        let Ok((aabb, global, visibility, mesh_layers)) = meshes.get(entity) else {
            continue;
        };
        if !visibility.get() || !layers.intersects(mesh_layers.unwrap_or(&RenderLayers::default()))
        {
            continue;
        }
        let local = inverse * global.affine();
        let center = local.transform_point3(aabb.center.into());
        let half = Vec3::from(local.matrix3.abs() * aabb.half_extents.abs());
        if center.is_finite() && half.is_finite() {
            min = min.min(center - half);
            max = max.max(center + half);
        }
    }
    (min.is_finite() && max.is_finite()).then_some((min, max, world))
}

fn corner_segments(min: Vec3, max: Vec3, world: Affine3A) -> impl Iterator<Item = (Vec3, Vec3)> {
    (0..8).flat_map(move |corner| {
        let start = Vec3::new(
            if corner & 1 == 0 { min.x } else { max.x },
            if corner & 2 == 0 { min.y } else { max.y },
            if corner & 4 == 0 { min.z } else { max.z },
        );
        (0..3).map(move |axis| {
            let mut end = start;
            let sign = if corner & (1 << axis) == 0 { 1.0 } else { -1.0 };
            end[axis] += (max[axis] - min[axis]) * CORNER_FRACTION * sign;
            (world.transform_point3(start), world.transform_point3(end))
        })
    })
}

/// Run after current mesh bounds, transforms and inherited visibility are ready.
/// No mesh proxies or extra cameras: brackets never change the source material.
pub(super) fn draw(
    editor: Res<Editor>,
    scene: Res<Scene>,
    camera: Query<(&Camera, &GlobalTransform, Option<&RenderLayers>), With<EditorCamera>>,
    children: Query<&Children>,
    transforms: Query<&GlobalTransform>,
    meshes: Query<
        (
            &Aabb,
            &GlobalTransform,
            &InheritedVisibility,
            Option<&RenderLayers>,
        ),
        With<Mesh3d>,
    >,
    mut gizmos: Gizmos<SelectionGizmos>,
) {
    if !editor.has_scene() || !editor.ready || editor.pilot {
        return;
    }
    let root = match editor.selection {
        Some(Selection::Imported(index)) => editor.imported.get(index).map(|node| node.entity),
        Some(Selection::Model(id)) => scene.models.get(&id).map(|(entity, _)| *entity),
        _ => None,
    };
    let Some(root) = root else { return };
    let Ok((camera, view, layers)) = camera.single() else {
        return;
    };
    if !camera.is_active {
        return;
    }
    let Some((mut min, mut max, world)) = bounds(
        root,
        layers.unwrap_or(&RenderLayers::default()),
        &children,
        &transforms,
        &meshes,
    ) else {
        return;
    };
    // Leave a small screen-sized gap between the geometry and the brackets,
    // including thin/flat models. Derive it from the actual camera projection.
    let center = world.transform_point3((min + max) * 0.5);
    let Ok(screen) = camera.world_to_viewport(view, center) else {
        return;
    };
    let padding = camera
        .viewport_to_world(view, screen)
        .ok()
        .zip(camera.viewport_to_world(view, screen + Vec2::X * 3.0).ok())
        .map(|(a, b)| {
            let normal = *view.forward();
            let depth = (center - a.origin).dot(normal);
            let point = |ray: bevy::shape::Ray3d| {
                ray.origin + *ray.direction * (depth / ray.direction.dot(normal).max(1e-5))
            };
            point(a).distance(point(b))
        })
        .unwrap_or(0.0);
    let axis_lengths = Vec3::new(
        world.matrix3.x_axis.length(),
        world.matrix3.y_axis.length(),
        world.matrix3.z_axis.length(),
    );
    let gap = Vec3::splat(padding) / axis_lengths;
    min -= gap;
    max += gap;
    for (start, end) in corner_segments(min, max, world) {
        gizmos.line(start, end, COLOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    fn measure(world: &mut World, root: Entity) -> Option<(Vec3, Vec3, Affine3A)> {
        let mut state: SystemState<(
            Query<&Children>,
            Query<&GlobalTransform>,
            Query<
                (
                    &Aabb,
                    &GlobalTransform,
                    &InheritedVisibility,
                    Option<&RenderLayers>,
                ),
                With<Mesh3d>,
            >,
        )> = SystemState::new(world);
        let (children, transforms, meshes) = state.get(world).unwrap();
        bounds(
            root,
            &RenderLayers::layer(0),
            &children,
            &transforms,
            &meshes,
        )
    }

    fn mesh(world: &mut World, global: GlobalTransform) -> Entity {
        world
            .spawn((
                Mesh3d::default(),
                global,
                InheritedVisibility::VISIBLE,
                Aabb::from_min_max(Vec3::splat(-1.0), Vec3::ONE),
            ))
            .id()
    }

    #[test]
    fn bounds_include_root_mesh_and_children_in_rotated_scaled_root_space() {
        let mut world = World::new();
        let root_global = GlobalTransform::from(
            Transform::from_xyz(3.0, 2.0, -4.0)
                .with_rotation(Quat::from_rotation_z(0.6))
                .with_scale(Vec3::new(2.0, 0.5, -3.0)),
        );
        let root = mesh(&mut world, root_global);
        let child_pose = Transform::from_xyz(4.0, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4));
        let child = mesh(&mut world, root_global.mul_transform(child_pose));
        world.entity_mut(child).insert(ChildOf(root));
        let (min, max, affine) = measure(&mut world, root).unwrap();
        let extent = 2.0_f32.sqrt();
        assert!(min.abs_diff_eq(Vec3::new(-1.0, -1.0, -extent), 1e-4));
        assert!(max.abs_diff_eq(Vec3::new(4.0 + extent, 1.0, extent), 1e-4));
        assert!(affine.abs_diff_eq(root_global.affine(), 1e-5));
        // Removing asynchronously loaded geometry must shrink the next frame.
        world.entity_mut(child).despawn();
        let (min, max, _) = measure(&mut world, root).unwrap();
        assert!(min.abs_diff_eq(Vec3::splat(-1.0), 1e-4));
        assert!(max.abs_diff_eq(Vec3::ONE, 1e-4));
    }

    #[test]
    fn hidden_and_other_layer_geometry_never_expand_the_selection() {
        let mut world = World::new();
        let root = mesh(&mut world, GlobalTransform::IDENTITY);
        for hidden in [true, false] {
            let child = mesh(&mut world, GlobalTransform::from_xyz(100.0, 0.0, 0.0));
            world.entity_mut(child).insert(ChildOf(root));
            if hidden {
                world.entity_mut(child).insert(InheritedVisibility::HIDDEN);
            } else {
                world.entity_mut(child).insert(RenderLayers::layer(9));
            }
        }
        let (min, max, _) = measure(&mut world, root).unwrap();
        assert_eq!(min, Vec3::splat(-1.0));
        assert_eq!(max, Vec3::ONE);
        world.entity_mut(root).insert(InheritedVisibility::HIDDEN);
        assert!(measure(&mut world, root).is_none());
        world
            .entity_mut(root)
            .insert(GlobalTransform::from(Transform::from_scale(Vec3::ZERO)));
        assert!(measure(&mut world, root).is_none());
    }
}
