//! Filled transform handles and a shared pivot in the editor viewport only.
use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    camera::CameraUpdateSystems,
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

use super::{
    AXES, AXIS_COLORS, Editor, EditorCamera, PointerState, Tool, TransformHandle, allowed,
    axis_color, axis_length, direction, handle_color, length, outer_ring_radius, plane_allowed,
    plane_corners, quad_contains, rotation_radius, view_rotation_allowed,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(in crate::editor) struct HandleMaterial {
    // Logical target size and visibility. Shader dimensions use logical pixels
    // too, so the heads and pivot retain their size across DPI changes.
    #[uniform(0)]
    viewport: Vec4,
    #[uniform(1)]
    pivot: Vec4,
    #[uniform(2)]
    tips: [Vec4; 3],
    #[uniform(3)]
    colors: [LinearRgba; 3],
    // Three visible quadrilateral faces per scale handle, four vertices each.
    #[uniform(4)]
    faces: [Vec4; 36],
    #[uniform(5)]
    planes: [Vec4; 12],
    #[uniform(6)]
    plane_colors: [LinearRgba; 3],
    #[uniform(7)]
    outer_ring: Vec4,
}

impl Default for HandleMaterial {
    fn default() -> Self {
        Self {
            viewport: Vec4::ZERO,
            pivot: Vec4::ZERO,
            tips: [Vec4::ZERO; 3],
            colors: [LinearRgba::WHITE; 3],
            faces: [Vec4::ZERO; 36],
            planes: [Vec4::ZERO; 12],
            plane_colors: [LinearRgba::WHITE; 3],
            outer_ring: Vec4::ZERO,
        }
    }
}

impl UiMaterial for HandleMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("transform_handles.wesl"))
                .with_source("embedded"),
        )
    }
}

pub(in crate::editor) struct HandlePlugin;

impl Plugin for HandlePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "transform_handles.wesl");
        app.add_plugins(UiMaterialPlugin::<HandleMaterial>::default())
            .add_systems(
                PostUpdate,
                sync.after(CameraUpdateSystems)
                    .after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

pub(in crate::editor) fn spawn_overlay(
    commands: &mut Commands,
    materials: &mut Assets<HandleMaterial>,
    viewport: Entity,
) {
    commands.spawn((
        MaterialNode(materials.add(HandleMaterial::default())),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(viewport),
    ));
}

fn sync(
    editor: Res<Editor>,
    pointer: Res<PointerState>,
    camera: Query<(&Camera, &Transform, &GlobalTransform), With<EditorCamera>>,
    overlays: Query<&MaterialNode<HandleMaterial>>,
    mut materials: ResMut<Assets<HandleMaterial>>,
) {
    for overlay in &overlays {
        let Some(mut material) = materials.get_mut(&overlay.0) else {
            continue;
        };
        // Disable immediately on deselection or entering a camera.
        material.viewport.z = 0.0;
        material.planes = [Vec4::ZERO; 12];
        material.outer_ring = Vec4::ZERO;
        if !editor.has_scene() || !editor.ready || editor.pilot {
            continue;
        }
        let Some(selection) = editor.selection else {
            continue;
        };
        if editor.object_hidden(selection) {
            continue;
        }
        let Some(pose) = editor.pose(selection) else {
            continue;
        };
        let Ok((camera, view, global)) = camera.single() else {
            continue;
        };
        if !camera.is_active {
            continue;
        }
        let Some(size) = camera.logical_target_size() else {
            continue;
        };
        let pose = pose.transform();
        let frame = editor.gizmo_rotation(pose);
        let pivot = pose.translation;
        let Ok(start) = camera.world_to_viewport(global, pivot) else {
            continue;
        };
        let mode = match editor.tool {
            Tool::Universal => 3.0,
            Tool::Move => 0.0,
            Tool::Scale => 1.0,
            Tool::Rotate => 2.0,
        };
        material.viewport = Vec4::new(size.x, size.y, 1.0, mode);
        material.pivot = start.extend(0.0).extend(0.0);
        let size = length(camera, view, pivot);
        material.tips = [Vec4::ZERO; 3];
        for &tool in editor.tool.operations() {
            if tool == Tool::Rotate {
                continue;
            }
            for axis in 0..3 {
                if !allowed(&editor, selection, tool, axis) {
                    continue;
                }
                material.colors[axis] =
                    axis_color(&pointer, tool, axis, AXIS_COLORS[axis]).to_linear();
                let center =
                    pivot + direction(&editor, pose, axis) * axis_length(editor.tool, tool, size);
                let Ok(end) = camera.world_to_viewport(global, center) else {
                    continue;
                };
                // Match the hit test: an axis seen end-on has no handle.
                if end.distance(start) < 13.0 {
                    continue;
                }
                material.tips[axis] = Vec4::new(end.x, end.y, 1.0, 0.0);
                if tool == Tool::Scale {
                    let local_eye = frame.inverse() * (view.translation - center);
                    let half = size * 0.06;
                    for face in 0..3 {
                        let normal = AXES[face] * if local_eye[face] >= 0.0 { 1.0 } else { -1.0 };
                        let u = AXES[(face + 1) % 3];
                        let v = AXES[(face + 2) % 3];
                        for (corner, offset) in
                            [u + v, u - v, -u - v, -u + v].into_iter().enumerate()
                        {
                            let position = center + frame * (normal + offset) * half;
                            if let Ok(point) = camera.world_to_viewport(global, position) {
                                material.faces[axis * 12 + face * 4 + corner] =
                                    point.extend(0.0).extend(0.0);
                            } else {
                                material.tips[axis].z = 0.0;
                            }
                        }
                    }
                }
            }
        }
        if editor.tool == Tool::Universal {
            for normal in 0..3 {
                if !plane_allowed(&editor, selection, normal) {
                    continue;
                }
                let [Ok(a), Ok(b), Ok(c), Ok(d)] = plane_corners(pivot, normal, size, frame)
                    .map(|point| camera.world_to_viewport(global, point))
                else {
                    continue;
                };
                let corners = [a, b, c, d];
                if !quad_contains((a + b + c + d) * 0.25, corners) {
                    continue;
                }
                for (index, corner) in corners.into_iter().enumerate() {
                    material.planes[normal * 4 + index] = corner.extend(1.0).extend(0.0);
                }
                material.plane_colors[normal] = handle_color(
                    &pointer,
                    TransformHandle::Plane { normal },
                    AXIS_COLORS[normal],
                )
                .to_linear();
            }
        }
        if editor.tool.operations().contains(&Tool::Rotate) {
            if view_rotation_allowed(&editor, selection)
                && let Some(radius) =
                    outer_ring_radius(camera, view, pivot, rotation_radius(camera, view, pivot))
            {
                let highlighted = pointer.hover == Some(TransformHandle::ViewRotation)
                    || pointer
                        .drag
                        .as_ref()
                        .is_some_and(|drag| drag.handle == TransformHandle::ViewRotation);
                material.outer_ring =
                    Vec4::new(radius, if highlighted { 1.0 } else { 0.0 }, 1.0, 0.0);
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::super::{handle_at, rotation_direction, universal_ring_segments};
    use super::*;
    use crate::editor::{
        Selection,
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::fixture,
        ui,
    };
    use bevy::input::{ButtonState, mouse::MouseButtonInput};

    fn handle_material(app: &mut App) -> HandleMaterial {
        let world = app.world_mut();
        let mut query = world.query::<&MaterialNode<HandleMaterial>>();
        let handle = query.single(world).unwrap().0.clone();
        world
            .resource::<Assets<HandleMaterial>>()
            .get(&handle)
            .unwrap()
            .clone()
    }

    #[test]
    fn universal_plane_handles_require_both_unlocked_translation_axes() {
        let project = crate::project::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, default());
        editor.imported.push(crate::editor::Imported {
            entity: Entity::PLACEHOLDER,
            name: "Constrained".into(),
            original: Transform::IDENTITY.into(),
            parent: GlobalTransform::IDENTITY,
            local: Transform::IDENTITY,
        });
        editor.project.scenes[0].node_policies.insert(
            "Constrained".into(),
            crate::project::NodePolicy {
                translation: [true, false, true],
                rotation: [false, true, true],
                ..default()
            },
        );
        let selection = Selection::Imported(0);
        assert!(!plane_allowed(&editor, selection, 0));
        assert!(plane_allowed(&editor, selection, 1));
        assert!(!plane_allowed(&editor, selection, 2));
        assert!(!view_rotation_allowed(&editor, selection));
        assert!(view_rotation_allowed(&editor, Selection::Camera(0)));
    }

    #[test]
    fn rotation_tools_use_front_half_arcs() {
        let view = Transform::from_xyz(4.0, 3.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        for axis in 0..3 {
            let points: Vec<_> =
                super::super::display_ring_points(Vec3::ZERO, axis, 2.0, &view).collect();
            assert!(
                points
                    .first()
                    .unwrap()
                    .abs_diff_eq(*points.last().unwrap(), 1e-5)
            );
            let circumference: f32 = points
                .windows(2)
                .map(|pair| pair[0].distance(pair[1]))
                .sum();
            let arc: f32 = universal_ring_segments(Vec3::ZERO, axis, 2.0, &view, Quat::IDENTITY)
                .map(|(a, b)| a.distance(b))
                .sum();
            assert!(
                (arc * 2.0 - circumference).abs() < 0.01,
                "Universal mode must show the near half, matching its reference"
            );
        }
    }

    #[test]
    #[ignore = "requires GPU adapter; renders and drags each universal transform operation"]
    fn gpu_transform_handles_universal_mode_edits_each_operation_independently() {
        let (mut app, window, target) = fixture();
        let selection = Selection::Imported(0);
        {
            let mut editor = app.world_mut().resource_mut::<Editor>();
            editor.select(Some(selection));
            editor.hidden_objects.clear();
        }
        app.world_mut()
            .write_message(crate::editor::Action::Tool(Tool::Universal));
        for _ in 0..4 {
            frame(&mut app);
        }
        let initial = app.world().resource::<Editor>().edits.clone();
        let material = handle_material(&mut app);
        assert_eq!(material.viewport.w, 3.0);
        assert!(material.tips.iter().all(|tip| tip.z == 1.0));
        let output = std::env::temp_dir().join(format!(
            "bevy-editor-universal-handles-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&output).unwrap();
        println!("Universal gizmo GPU artifacts: {}", output.display());
        let to_canvas = |p: Vec2| (p + Vec2::Y * ui::TOP).round().as_uvec2();
        assert!(material.outer_ring.x > 0.0 && material.outer_ring.z == 1.0);
        assert!((0..3).all(|plane| material.planes[plane * 4].z == 1.0));
        let start = material.pivot.truncate().truncate();
        let tip = material.tips[1].truncate().truncate();
        let head = to_canvas(tip - (tip - start).normalize() * 8.0);
        let image = capture_when(&mut app, &target, &output.join("universal.png"), |image| {
            image.get_pixel(head.x, head.y).0[1] > 180
        });
        for axis in 0..3 {
            let start = material.pivot.truncate().truncate();
            let tip = material.tips[axis].truncate().truncate();
            let sample = to_canvas(tip - (tip - start).normalize() * 8.0);
            assert!(
                image.get_pixel(sample.x, sample.y).0[axis] > 180,
                "Move arrow {axis} must render alongside the rotation arcs"
            );
            let face = &material.planes[axis * 4..axis * 4 + 4];
            let sample =
                to_canvas(face.iter().map(|p| p.truncate().truncate()).sum::<Vec2>() / 4.0);
            assert!(
                image.get_pixel(sample.x, sample.y).0[axis] > 150,
                "Translation plane {axis} must have a flat colored face"
            );
        }
        for expected in [
            TransformHandle::Axis {
                tool: Tool::Move,
                axis: 1,
            },
            TransformHandle::Axis {
                tool: Tool::Rotate,
                axis: 1,
            },
            TransformHandle::Plane { normal: 1 },
            TransformHandle::ViewRotation,
        ] {
            let material = handle_material(&mut app);
            let before = app
                .world()
                .resource::<Editor>()
                .pose(selection)
                .unwrap()
                .transform();
            let (start, end) = if expected
                == (TransformHandle::Axis {
                    tool: Tool::Move,
                    axis: 1,
                }) {
                let start = material.tips[1].truncate().truncate();
                let direction = (start - material.pivot.truncate().truncate()).normalize();
                (start, start + direction * 25.0)
            } else {
                let world = app.world_mut();
                let mut query = world
                    .query_filtered::<(&Camera, &Transform, &GlobalTransform), With<EditorCamera>>(
                    );
                let (camera, view, global) = query.single(world).unwrap();
                let editor = world.resource::<Editor>();
                let radius = rotation_radius(camera, view, before.translation);
                if expected == (TransformHandle::Plane { normal: 1 }) {
                    let size = length(camera, view, before.translation);
                    let center = plane_corners(before.translation, 1, size, Quat::IDENTITY)
                        .into_iter()
                        .sum::<Vec3>()
                        * 0.25;
                    let end = center + Vec3::new(size * 0.22, 0.0, size * 0.28);
                    (
                        camera.world_to_viewport(global, center).unwrap(),
                        camera.world_to_viewport(global, end).unwrap(),
                    )
                } else if expected == TransformHandle::ViewRotation {
                    let center = material.pivot.truncate().truncate();
                    (0..96)
                        .filter_map(|index| {
                            let angle = std::f32::consts::TAU * index as f32 / 96.0;
                            let start = center + Vec2::from_angle(angle) * material.outer_ring.x;
                            if handle_at(editor, selection, before, camera, view, start, radius)
                                != Some(expected)
                            {
                                return None;
                            }
                            let end =
                                center + Vec2::from_angle(angle + 0.4) * material.outer_ring.x;
                            Some((start, end))
                        })
                        .next()
                        .expect("The outer circle must be independently selectable")
                } else {
                    universal_ring_segments(before.translation, 1, radius, view, Quat::IDENTITY)
                        .filter_map(|(a, b)| {
                            let point = a.lerp(b, 0.5);
                            let start = camera.world_to_viewport(global, point).ok()?;
                            if handle_at(editor, selection, before, camera, view, start, radius)
                                != Some(expected)
                            {
                                return None;
                            }
                            let direction = rotation_direction(
                                camera,
                                view,
                                start,
                                before.translation,
                                Vec3::Y,
                            )?;
                            let offset = Quat::from_rotation_y(0.45) * direction * radius;
                            let end = before.translation + offset
                                - *view.forward() * offset.dot(*view.forward());
                            Some((start, camera.world_to_viewport(global, end).ok()?))
                        })
                        .find(|(start, end)| start.distance(*end) > 12.0)
                        .expect("A rotation ring must remain independently selectable")
                }
            };
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(start + Vec2::Y * ui::TOP));
            frame(&mut app);
            assert_eq!(app.world().resource::<PointerState>().hover, Some(expected));
            app.world_mut().write_message(MouseButtonInput {
                window,
                button: MouseButton::Left,
                state: ButtonState::Pressed,
            });
            frame(&mut app);
            assert_eq!(
                app.world()
                    .resource::<PointerState>()
                    .drag
                    .as_ref()
                    .map(|drag| drag.handle),
                Some(expected)
            );
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(end + Vec2::Y * ui::TOP));
            frame(&mut app);
            app.world_mut().write_message(MouseButtonInput {
                window,
                button: MouseButton::Left,
                state: ButtonState::Released,
            });
            frame(&mut app);
            let editor = app.world().resource::<Editor>();
            let after = editor.pose(selection).unwrap().transform();
            assert_eq!(
                editor.tool,
                Tool::Universal,
                "Dragging must keep the shared mode selected"
            );
            assert_eq!(
                after.scale, before.scale,
                "Shared-mode gestures must not change scale"
            );
            match expected {
                TransformHandle::Axis {
                    tool: Tool::Move, ..
                } => {
                    assert!(after.translation.y > before.translation.y + 0.1);
                    assert_eq!(after.rotation, before.rotation);
                    assert_eq!(after.scale, before.scale);
                }
                TransformHandle::Axis {
                    tool: Tool::Rotate, ..
                }
                | TransformHandle::ViewRotation => {
                    assert!(after.rotation.angle_between(before.rotation) > 0.1);
                    assert_eq!(after.translation, before.translation);
                    assert_eq!(after.scale, before.scale);
                }
                TransformHandle::Plane { normal: 1 } => {
                    assert!(after.translation.x > before.translation.x + 0.1);
                    assert!(after.translation.z > before.translation.z + 0.1);
                    assert_eq!(after.translation.y, before.translation.y);
                    assert_eq!(after.rotation, before.rotation);
                }
                _ => unreachable!(),
            }
            app.world_mut().write_message(crate::editor::Action::Undo);
            for _ in 0..4 {
                frame(&mut app);
            }
            assert_eq!(
                app.world().resource::<Editor>().edits,
                initial,
                "Undo must restore the whole gesture"
            );
        }
        app.world_mut()
            .write_message(crate::editor::Action::Select(Selection::Camera(0)));
        frame(&mut app);
        {
            let mut editor = app.world_mut().resource_mut::<Editor>();
            let position = editor.camera(0).pose.transform().translation;
            editor.view = Transform::from_translation(position + Vec3::new(4.0, 3.0, 5.0))
                .looking_at(position, Vec3::Y);
        }
        for _ in 0..4 {
            frame(&mut app);
        }
        let material = handle_material(&mut app);
        assert_eq!(material.viewport.z, 1.0, "The camera gizmo must be in view");
        assert_eq!(material.viewport.w, 3.0);
        assert_eq!(material.tips.len(), 3);
    }

    #[test]
    #[ignore = "requires GPU adapter; captures and exercises all transform handles"]
    fn gpu_transform_handles_are_filled_share_a_pivot_and_remain_draggable() {
        let (mut app, window, target) = fixture();
        app.world_mut().resource_mut::<Editor>().selection = Some(Selection::Imported(0));
        app.world_mut().resource_mut::<Editor>().tool = Tool::Move;
        for _ in 0..4 {
            frame(&mut app);
        }
        let material = {
            let world = app.world_mut();
            let mut query = world.query::<&MaterialNode<HandleMaterial>>();
            let handle = query.single(world).unwrap().0.clone();
            world
                .resource::<Assets<HandleMaterial>>()
                .get(&handle)
                .unwrap()
                .clone()
        };
        assert_eq!(material.viewport.z, 1.0);
        let output =
            std::env::temp_dir().join(format!("bevy-editor-move-handles-{}", std::process::id()));
        std::fs::create_dir_all(&output).unwrap();
        println!("Move gizmo GPU artifacts: {}", output.display());
        let to_canvas = |p: Vec2| (p + Vec2::Y * ui::TOP).round().as_uvec2();
        let pivot = to_canvas(material.pivot.truncate().truncate());
        let image = capture_when(&mut app, &target, &output.join("move.png"), |image| {
            let pixel = image.get_pixel(pivot.x, pivot.y).0;
            pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245
        });
        let start = material.pivot.truncate().truncate();
        for axis in 0..3 {
            let tip = material.tips[axis];
            assert_eq!(tip.z, 1.0);
            let direction = (tip.truncate().truncate() - start).normalize();
            // Inside the triangle but off its centerline and edges: wire tips
            // and an unfilled V cannot satisfy this color check.
            let sample = to_canvas(
                tip.truncate().truncate() - direction * 9.0
                    + Vec2::new(-direction.y, direction.x) * 2.0,
            );
            let pixel = image.get_pixel(sample.x, sample.y).0;
            // World X/Y/Z use red/green/blue respectively.
            assert!(
                pixel[axis] > 180,
                "Axis {axis} has no filled head: {pixel:?}"
            );
            assert!(
                pixel[axis] as i16 - pixel[(axis + 1) % 3] as i16 > 35,
                "Axis {axis} interior has the wrong color: {pixel:?}"
            );
        }
        // The visual overlay must not intercept the existing head hit test.
        let selection = Selection::Imported(0);
        let before = app
            .world()
            .resource::<Editor>()
            .pose(selection)
            .unwrap()
            .transform();
        let head = material.tips[1].truncate().truncate();
        let direction = (head - start).normalize();
        let cursor = head - direction * 7.0 + Vec2::Y * ui::TOP;
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(cursor));
        frame(&mut app);
        app.world_mut()
            .write_message(bevy::input::mouse::MouseButtonInput {
                window,
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Pressed,
            });
        frame(&mut app);
        assert!(
            app.world().resource::<PointerState>().dragging(),
            "Filled head must remain draggable"
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(cursor + direction * 25.0));
        frame(&mut app);
        app.world_mut()
            .write_message(bevy::input::mouse::MouseButtonInput {
                window,
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Released,
            });
        frame(&mut app);
        let after = app
            .world()
            .resource::<Editor>()
            .pose(selection)
            .unwrap()
            .transform();
        assert!(after.translation.y > before.translation.y + 0.1);
        assert_eq!(after.translation.x, before.translation.x);
        assert_eq!(after.translation.z, before.translation.z);
        app.world_mut().write_message(crate::editor::Action::Undo);
        frame(&mut app);
        assert!(
            app.world()
                .resource::<Editor>()
                .pose(selection)
                .unwrap()
                .transform()
                .translation
                .abs_diff_eq(before.translation, 1e-5)
        );

        app.world_mut().resource_mut::<Editor>().tool = Tool::Scale;
        for _ in 0..4 {
            frame(&mut app);
        }
        let scale_material = {
            let world = app.world_mut();
            let mut query = world.query::<&MaterialNode<HandleMaterial>>();
            let handle = query.single(world).unwrap().0.clone();
            world
                .resource::<Assets<HandleMaterial>>()
                .get(&handle)
                .unwrap()
                .clone()
        };
        assert_eq!(scale_material.viewport.w, 1.0);
        let image = capture_when(&mut app, &target, &output.join("scale.png"), |image| {
            let pixel = image.get_pixel(pivot.x, pivot.y).0;
            pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245
        });
        for axis in 0..3 {
            // Sample inside a visible face, away from the old wire cube edges.
            let face = &scale_material.faces[axis * 12..axis * 12 + 4];
            let center = face.iter().map(|p| p.truncate().truncate()).sum::<Vec2>() / 4.0;
            let sample = to_canvas(center);
            let pixel = image.get_pixel(sample.x, sample.y).0;
            assert!(
                pixel[axis] > 150 && pixel[axis] as i16 - pixel[(axis + 1) % 3] as i16 > 30,
                "Axis {axis} must have a solid colored cube face: {pixel:?}"
            );
        }
        app.world_mut().resource_mut::<Editor>().tool = Tool::Rotate;
        for _ in 0..4 {
            frame(&mut app);
        }
        capture_when(&mut app, &target, &output.join("rotate.png"), |image| {
            let pixel = image.get_pixel(pivot.x, pivot.y).0;
            pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245
        });
        app.world_mut().resource_mut::<Editor>().selection = None;
        for _ in 0..4 {
            frame(&mut app);
        }
        capture_when(&mut app, &target, &output.join("deselected.png"), |image| {
            let pixel = image.get_pixel(pivot.x, pivot.y).0;
            pixel[0] < 230 || pixel[1] < 230 || pixel[2] < 230
        });
    }
}
