use super::*;
use bevy::camera::{CameraProjection, ComputedCameraValues, RenderTargetInfo};

fn editor() -> Editor {
    let project = crate::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    Editor::new(project, default())
}

#[test]
fn rotated_objects_use_world_axes_by_default_and_object_axes_in_local_space() {
    let mut editor = editor();
    let pose = Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
        .with_scale(Vec3::new(4.0, 2.0, 3.0));
    assert_eq!(direction(&editor, pose, 0), Vec3::X);
    assert_eq!(direction(&editor, pose, 2), Vec3::Z);
    editor.local_space = true;
    assert!(direction(&editor, pose, 0).abs_diff_eq(Vec3::NEG_Z, 1e-5));
    assert!(direction(&editor, pose, 2).abs_diff_eq(Vec3::X, 1e-5));
    assert!((direction(&editor, pose, 0).length() - 1.0).abs() < 1e-5);
}

#[test]
fn gizmo_hit_testing_follows_the_same_rotated_axes_as_the_handles() {
    let mut editor = editor();
    editor.tool = Tool::Move;
    let pose = Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    let camera = Camera {
        computed: ComputedCameraValues {
            clip_from_view: PerspectiveProjection {
                aspect_ratio: 1.0,
                fov: EDITOR_FOV,
                ..default()
            }
            .get_clip_from_view(),
            target_info: Some(RenderTargetInfo {
                physical_size: UVec2::splat(1000),
                scale_factor: 1.0,
            }),
            ..default()
        },
        ..default()
    };
    let view = Transform::from_xyz(4.0, 3.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
    let global = GlobalTransform::from(view);
    let extent = length(&camera, &view, Vec3::ZERO) * 0.9;
    let world_x = camera.world_to_viewport(&global, Vec3::X * extent).unwrap();
    let local_x = camera
        .world_to_viewport(&global, Vec3::NEG_Z * extent)
        .unwrap();
    let expected = |axis| {
        Some(TransformHandle::Axis {
            tool: Tool::Move,
            axis,
        })
    };
    assert_eq!(
        handle_at(
            &editor,
            Selection::Model(1),
            pose,
            &camera,
            &view,
            world_x,
            1.0
        ),
        expected(0)
    );
    editor.local_space = true;
    assert_eq!(
        handle_at(
            &editor,
            Selection::Model(1),
            pose,
            &camera,
            &view,
            local_x,
            1.0
        ),
        expected(0)
    );
    assert_eq!(
        handle_at(
            &editor,
            Selection::Model(1),
            pose,
            &camera,
            &view,
            world_x,
            1.0
        ),
        expected(2)
    );
}

#[test]
fn plane_geometry_and_scale_operations_use_the_selected_space() {
    let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let start = Transform::from_xyz(3.0, 2.0, 1.0)
        .with_rotation(rotation)
        .with_scale(Vec3::new(2.0, 3.0, 4.0));
    for point in plane_corners(start.translation, 0, 2.0, rotation) {
        assert!((point - start.translation).dot(Vec3::NEG_Z).abs() < 1e-5);
    }
    let global = scaled_pose(start, Quat::IDENTITY, 0, 2.0);
    let local = scaled_pose(start, rotation, 0, 2.0);
    assert!(global.scale.abs_diff_eq(Vec3::new(2.0, 3.0, 8.0), 1e-4));
    assert!(local.scale.abs_diff_eq(Vec3::new(4.0, 3.0, 4.0), 1e-4));
    assert_eq!(global.translation, start.translation);
    assert_eq!(local.translation, start.translation);
    assert!(global.rotation.angle_between(rotation).abs() < 1e-3);
    assert_eq!(local.rotation, rotation);
    assert!(crate::Pose::from(global).valid() && crate::Pose::from(local).valid());
}

#[test]
fn world_scaling_keeps_arbitrarily_rotated_and_mirrored_objects_valid() {
    for scale in [Vec3::new(2.0, 3.0, 4.0), Vec3::new(2.0, -3.0, 4.0)] {
        let start = Transform::from_xyz(3.0, 2.0, 1.0)
            .with_rotation(Quat::from_euler(EulerRot::YXZ, 0.7, 0.4, 0.3))
            .with_scale(scale);
        for axis in 0..3 {
            for factor in [0.25, 2.0, 10.0] {
                let scaled = scaled_pose(start, Quat::IDENTITY, axis, factor);
                assert!(crate::Pose::from(scaled).valid());
                assert_eq!(scaled.translation, start.translation);
                assert_eq!(
                    scaled.to_matrix().determinant().is_sign_negative(),
                    start.to_matrix().determinant().is_sign_negative()
                );
            }
        }
    }
}
