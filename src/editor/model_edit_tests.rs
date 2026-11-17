//! Complete model-document workflow through the real editor and renderer.
#[path = "model_wireframe_tests.rs"]
mod wireframe_tests;
use super::*;
use crate::EditorProject;
use crate::editor::{
    PendingAction,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    input_focus::{FocusCause, InputFocus},
    text::EditableText,
};
use std::path::{Path, PathBuf};

fn warm(app: &mut App, count: usize) {
    for _ in 0..count {
        frame(app);
    }
}

fn send(app: &mut App, action: EditAction) {
    app.world_mut()
        .write_message(Action::ModelView(ModelViewAction::Edit(action)));
    warm(app, 3);
}
fn key(app: &mut App, window: Entity, code: KeyCode, logical: Key) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code: code,
            logical_key: logical.clone(),
            state,
            text: None,
            repeat: false,
            window,
        });
        warm(app, 2);
    }
}
fn click_control(app: &mut App, window: Entity, predicate: impl Fn(Action) -> bool) {
    let position = {
        let world = app.world_mut();
        let (_, node, transform) = world
            .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(control, node, _)| node.size().min_element() > 0.0 && predicate(control.action))
            .expect("The real model control must be visible");
        transform.affine().translation * node.inverse_scale_factor()
    };
    move_pointer(app, window, position);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 3);
}
fn model_ready(app: &mut App, index: usize) {
    for _ in 0..240 {
        frame(app);
        if app
            .world()
            .resource::<State>()
            .documents
            .contains_key(&index)
        {
            warm(app, 5);
            return;
        }
    }
    panic!(
        "Editable model failed to initialize: {:?}",
        app.world().resource::<State>().errors
    );
}
fn canvas_rect(app: &mut App) -> Rect {
    let world = app.world_mut();
    let (node, transform) = world
        .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
        .single(world)
        .unwrap();
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}
fn edit_field(app: &mut App, field: controls::Field, content: &str) -> Entity {
    let entity = {
        let world = app.world_mut();
        world
            .query::<(Entity, &controls::Field)>()
            .iter(world)
            .find(|(_, f)| **f == field)
            .unwrap()
            .0
    };
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(entity, FocusCause::Auto);
    app.world_mut()
        .get_mut::<EditableText>(entity)
        .unwrap()
        .editor_mut()
        .set_text(content);
    warm(app, 3);
    entity
}
fn copy_directory(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &destination.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "requires GPU; compares unassigned GLTF materials in scenes and editable model tabs"]
fn gpu_unassigned_model_material_matches_scene_instead_of_pink_placeholder() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-model-material-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_directory(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let mut source = document::glb::Glb::load(&path).unwrap();
    source.json.as_object_mut().unwrap().remove("materials");
    for mesh in source.json["meshes"].as_array_mut().unwrap() {
        for primitive in mesh["primitives"].as_array_mut().unwrap() {
            primitive.as_object_mut().unwrap().remove("material");
        }
    }
    std::fs::write(&path, source.encode(&[], &[]).unwrap()).unwrap();
    let (mut app, _, target) = fixture_project(project);
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    let scene_handle = app
        .world()
        .resource::<AssetServer>()
        .get_handle::<StandardMaterial>("models/block.glb#DefaultMaterial/std")
        .unwrap();
    let state = app.world().resource::<State>();
    let materials = app.world().resource::<Assets<StandardMaterial>>();
    let scene = materials.get(&scene_handle).unwrap();
    assert_eq!(scene.base_color, Color::WHITE);
    for object in &state.documents[&0].objects {
        for slot in &object.materials {
            assert_ne!(slot.material, Handle::default());
            let shown = materials.get(&slot.material).unwrap();
            assert_eq!(shown.base_color, scene.base_color);
            assert_eq!(shown.metallic, scene.metallic);
            assert_eq!(shown.perceptual_roughness, scene.perceptual_roughness);
            assert_eq!(shown.alpha_mode, scene.alpha_mode);
        }
    }
    let rect = canvas_rect(&mut app);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/model-edit-qa/unassigned-material.png");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    capture_when(&mut app, &target, &output, |image| {
        image
            .enumerate_pixels()
            .filter(|(x, y, pixel)| {
                rect.contains(Vec2::new(*x as f32, *y as f32))
                    && pixel[0] > 180
                    && u16::from(pixel[0]) > u16::from(pixel[1]) + 70
                    && u16::from(pixel[2]) > u16::from(pixel[1]) + 40
            })
            .count()
            < 30
    });
}

fn scene_workspace_interactions(app: &mut App, window: Entity) {
    use crate::editor::viewport::{
        TransformHandle as Handle,
        transform_handles::{HandleMaterial, ModelOverlay},
    };
    let expected = crate::SceneLighting::editor_default();
    {
        let world = app.world_mut();
        let (exposure, ambient) = world
            .query_filtered::<(&bevy::camera::Exposure, &AmbientLight), With<ViewCamera>>()
            .single(world)
            .unwrap();
        assert_eq!(exposure.ev100, expected.exposure_ev100);
        assert_eq!(ambient.brightness, expected.ambient_brightness);
        assert!(
            world
                .query_filtered::<&Visibility, With<ModelGrid>>()
                .iter(world)
                .any(|v| *v == Visibility::Visible)
        );
        let overlay = world
            .query_filtered::<&MaterialNode<HandleMaterial>, With<ModelOverlay>>()
            .single(world)
            .unwrap()
            .0
            .clone();
        let material = world
            .resource::<Assets<HandleMaterial>>()
            .get(&overlay)
            .unwrap();
        assert_eq!(
            material.viewport.z, 0.0,
            "Selection must hide model transform handles"
        );
    }
    let original = app.world().resource::<State>().documents[&0]
        .objects
        .clone();
    let rect = canvas_rect(app);
    let make_frame = |app: &App| {
        handles::Frame::new(
            &app.world().resource::<State>().documents[&0],
            app.world().resource::<Viewer>(),
            rect,
            false,
        )
        .unwrap()
    };
    let project = |app: &App, p| input::project(app.world().resource::<Viewer>(), rect, p).unwrap();
    let frame = make_frame(app);
    let center = project(app, frame.pivot);
    assert_eq!(
        frame.hit(
            app.world().resource::<Viewer>(),
            rect,
            Tool::Universal,
            center
        ),
        None
    );
    let history = app.world().resource::<State>().documents[&0].undo.len();
    move_pointer(app, window, center);
    mouse(app, window, ButtonState::Pressed);
    assert!(app.world().resource::<State>().box_start.is_some());
    assert!(app.world().resource::<State>().gesture.is_none());
    move_pointer(app, window, center + Vec2::new(28.0, -14.0));
    mouse(app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<State>().documents[&0].objects,
        original
    );
    assert_eq!(
        app.world().resource::<State>().documents[&0].undo.len(),
        history
    );
    send(app, EditAction::SelectAll);
    click_control(app, window, |a| matches!(a, Action::Tool(Tool::Move)));
    assert_eq!(
        frame.hit(app.world().resource::<Viewer>(), rect, Tool::Move, center),
        Some(Handle::ViewTranslation)
    );
    move_pointer(app, window, center);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, center + Vec2::new(28.0, -14.0));
    mouse(app, window, ButtonState::Released);
    assert_ne!(
        app.world().resource::<State>().documents[&0].objects,
        original
    );
    send(app, EditAction::Undo);
    assert_eq!(
        app.world().resource::<State>().documents[&0].objects,
        original
    );
    let frame = make_frame(app);
    let (normal, point) = (0..3)
        .find_map(|normal| {
            let point = frame
                .plane(normal)
                .map(|p| project(app, p))
                .into_iter()
                .sum::<Vec2>()
                * 0.25;
            (frame.hit(app.world().resource::<Viewer>(), rect, Tool::Move, point)
                == Some(Handle::Plane { normal }))
            .then_some((normal, point))
        })
        .expect("At least one visible scene-style plane must be draggable");
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, point + Vec2::new(20.0, 14.0));
    mouse(app, window, ButtonState::Released);
    let delta = app.world().resource::<State>().documents[&0]
        .active()
        .unwrap()
        .transform
        .translation
        - original[0].transform.translation;
    assert!(delta.length() > 0.01);
    assert!(
        delta[normal].abs() < 0.0001,
        "Plane dragging must lock its normal axis"
    );
    send(app, EditAction::Undo);
    click_control(app, window, |a| matches!(a, Action::Tool(Tool::Rotate)));
    let frame = make_frame(app);
    let (axis, point) = (0..3)
        .find_map(|axis| {
            frame.rings(axis).find_map(|(p, _)| {
                let point = project(app, p);
                (frame.hit(app.world().resource::<Viewer>(), rect, Tool::Rotate, point)
                    == Some(Handle::Axis {
                        tool: Tool::Rotate,
                        axis,
                    }))
                .then_some((axis, point))
            })
        })
        .expect("Rotate mode must expose the same rotation arcs as the scene");
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    assert_eq!(
        app.world()
            .resource::<State>()
            .gesture
            .as_ref()
            .unwrap()
            .axis,
        Some(axis)
    );
    move_pointer(app, window, point + Vec2::new(17.0, -13.0));
    mouse(app, window, ButtonState::Released);
    assert!(
        !app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .transform
            .rotation
            .abs_diff_eq(original[0].transform.rotation, 0.0001)
    );
    send(app, EditAction::Undo);
    click_control(app, window, |a| matches!(a, Action::Tool(Tool::Scale)));
    let frame = make_frame(app);
    let start = project(app, frame.tip(Tool::Scale, Tool::Scale, 0));
    let center = project(app, frame.pivot);
    move_pointer(app, window, start);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, start + (start - center).normalize() * 20.0);
    mouse(app, window, ButtonState::Released);
    let scale = app.world().resource::<State>().documents[&0]
        .active()
        .unwrap()
        .transform
        .scale;
    assert!(scale.x > original[0].transform.scale.x + 0.05);
    assert!(
        (scale.y - original[0].transform.scale.y).abs() < 0.0001
            && (scale.z - original[0].transform.scale.z).abs() < 0.0001
    );
    send(app, EditAction::Undo);
    click_control(app, window, |a| matches!(a, Action::Tool(Tool::Universal)));
    send(app, EditAction::Mode(Mode::Edit));
    let saved = {
        let v = app.world().resource::<Viewer>();
        (v.center, v.distance, v.yaw, v.pitch)
    };
    let before = view_pose(app.world().resource::<Viewer>()).translation;
    move_pointer(app, window, rect.center());
    app.world_mut()
        .write_message(bevy::input::mouse::MouseButtonInput {
            window,
            button: MouseButton::Right,
            state: ButtonState::Pressed,
        });
    warm(app, 2);
    move_pointer(app, window, rect.center() + Vec2::new(25.0, 10.0));
    assert!(
        view_pose(app.world().resource::<Viewer>())
            .translation
            .distance(before)
            < 0.0001,
        "RMB rotates from the eye rather than orbiting the object"
    );
    key(app, window, KeyCode::KeyW, Key::Character("w".into()));
    key(app, window, KeyCode::KeyE, Key::Character("e".into()));
    app.world_mut()
        .write_message(bevy::input::mouse::MouseButtonInput {
            window,
            button: MouseButton::Right,
            state: ButtonState::Released,
        });
    warm(app, 3);
    assert!(
        view_pose(app.world().resource::<Viewer>())
            .translation
            .distance(before)
            > 0.05
    );
    assert_eq!(
        app.world().resource::<State>().documents[&0].objects,
        original,
        "Fly keys must never edit the mesh"
    );
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    {
        let mut v = app.world_mut().resource_mut::<Viewer>();
        v.center = saved.0;
        v.distance = saved.1;
        v.yaw = saved.2;
        v.pitch = saved.3;
    }
    warm(app, 3);
    send(app, EditAction::Mode(Mode::Object));
}

#[test]
#[ignore = "requires GPU; verifies object/edit modes, picking, numeric transforms, undo, GLB saving and dirty-tab decisions"]
fn gpu_model_edit_workflow_preserves_scene_and_saves_real_glb() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-model-edit-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_directory(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-edit-qa");
    std::fs::create_dir_all(&output).unwrap();
    let path = directory.join("models/block.glb");
    let original_bytes = std::fs::read(&path).unwrap();
    let original_source = document::glb::Glb::load(&path).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let scene_edits = app.world().resource::<Editor>().edits.clone();
    let scene_history = app.world().resource::<Editor>().undo.clone();
    let scene_toolbar = {
        let editor = app.world().resource::<Editor>();
        (editor.tool, editor.local_space, editor.snap)
    };
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    assert_eq!(app.world().resource::<State>().mode(Some(0)), Mode::Object);
    let extrude_button = {
        let world = app.world_mut();
        let (entity, _, node) = world
            .query::<(Entity, &AccessibleLabel, &ComputedNode)>()
            .iter(world)
            .find(|(_, label, _)| label.0 == "Экструдировать · E")
            .expect("The shared header must contain the extrusion button");
        assert_eq!(node.size(), Vec2::ZERO, "Hide extrusion in Object mode");
        entity
    };
    {
        let world = app.world_mut();
        assert!(
            world
                .query::<&Control>()
                .iter(world)
                .all(|control| !matches!(
                    control.action,
                    Action::ModelView(ModelViewAction::Edit(
                        EditAction::Tool(_) | EditAction::Local | EditAction::Snap
                    ))
                )),
            "Transform controls must exist only in the shared header"
        );
    }
    // The real header buttons select model tools without changing scene state.
    for tool in [Tool::Move, Tool::Rotate, Tool::Scale, Tool::Universal] {
        click_control(
            &mut app,
            window,
            |a| matches!(a, Action::Tool(t) if t == tool),
        );
        assert_eq!(app.world().resource::<State>().tool, tool);
        let world = app.world_mut();
        let (_, color, disabled) = world
            .query::<(
                &Control,
                &BackgroundColor,
                Has<bevy::ui::InteractionDisabled>,
            )>()
            .iter(world)
            .find(|(c, _, _)| matches!(c.action, Action::Tool(t) if t == tool))
            .unwrap();
        assert!(!disabled);
        assert_eq!(
            color.0, ACCENT,
            "The selected header tool must be highlighted"
        );
        let mut controls = world.query::<(&Control, &Children)>();
        for (control, children) in controls.iter(world) {
            let Action::Tool(button_tool) = control.action else {
                continue;
            };
            let icon = children
                .iter()
                .find_map(|child| world.get::<ImageNode>(child))
                .expect("Transform buttons must contain an SVG icon");
            assert_eq!(
                icon.color,
                if button_tool == tool {
                    ON_ACCENT
                } else {
                    MUTED
                },
                "The {button_tool:?} icon must match the active model tool {tool:?}"
            );
        }
        if tool == Tool::Move {
            capture_when(
                &mut app,
                &target,
                &output.join("00-move-tool-icons.png"),
                |_| true,
            );
        }
    }
    click_control(&mut app, window, |a| matches!(a, Action::LocalSpace));
    click_control(&mut app, window, |a| matches!(a, Action::Snap));
    assert!(app.world().resource::<State>().local);
    assert!(app.world().resource::<State>().snap);
    {
        let editor = app.world().resource::<Editor>();
        assert_eq!(
            (editor.tool, editor.local_space, editor.snap),
            scene_toolbar
        );
    }
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    click_control(&mut app, window, |a| matches!(a, Action::LocalSpace));
    click_control(&mut app, window, |a| matches!(a, Action::Snap));
    {
        let world = app.world_mut();
        let (_, _, transform) = world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(label, _, _)| label.0 == "Режим модели")
            .unwrap();
        let mode_x = transform.affine().translation.x;
        let (_, canvas_transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Canvas>>()
            .single(world)
            .unwrap();
        assert!(
            mode_x < canvas_transform.affine().translation.x,
            "The mode selector must remain aligned to the left"
        );
    }
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .positions
            .len(),
        8
    );
    {
        let doc = &app.world().resource::<State>().documents[&0];
        let geometry = &doc.active().unwrap().geometry;
        assert_eq!(
            geometry.faces.len(),
            6,
            "Imported cube must expose six quad faces"
        );
        assert!(geometry.faces.iter().all(|face| face.corners.len() == 4));
        assert_eq!(
            geometry.edges().len(),
            12,
            "No triangulation diagonals in edit mode"
        );
        assert!(
            !doc.dirty(),
            "Restoring editable quads must not dirty the source"
        );
    }
    capture_when(
        &mut app,
        &target,
        &output.join("01-object-mode.png"),
        |image| {
            image
                .pixels()
                .filter(|p| p[0] > 150 && p[0] > p[2].saturating_add(30))
                .count()
                > 300
        },
    );

    scene_workspace_interactions(&mut app, window);

    // Use the actual dropdown, then the visible component selector.
    click_control(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(EditAction::Menu))
        )
    });
    assert!(app.world().resource::<State>().menu);
    click_control(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(EditAction::Mode(Mode::Edit)))
        )
    });
    assert_eq!(app.world().resource::<State>().mode(Some(0)), Mode::Edit);
    assert!(!app.world().resource::<State>().menu);
    click_control(&mut app, window, |a| matches!(a, Action::Tool(Tool::Scale)));
    assert_eq!(app.world().resource::<State>().tool, Tool::Scale);
    click_control(&mut app, window, |a| {
        matches!(a, Action::Tool(Tool::Universal))
    });
    click_control(&mut app, window, |a| {
        matches!(
            a,
            Action::ModelView(ModelViewAction::Edit(EditAction::Element(Element::Face)))
        )
    });
    capture_when(&mut app, &target, &output.join("02-edit-quads.png"), |_| {
        true
    });
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .faces
            .len(),
        6
    );
    send(&mut app, EditAction::Deselect);
    assert!(
        app.world()
            .get::<bevy::ui::InteractionDisabled>(extrude_button)
            .is_some()
    );
    let rect = canvas_rect(&mut app);
    move_pointer(&mut app, window, rect.center());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .faces
            .len(),
        1,
        "Click must pick exactly one visible polygon"
    );
    let extrude_position = {
        let world = app.world_mut();
        assert!(
            world
                .get::<bevy::ui::InteractionDisabled>(extrude_button)
                .is_none()
        );
        let node = world.get::<ComputedNode>(extrude_button).unwrap();
        let position = world
            .get::<UiGlobalTransform>(extrude_button)
            .unwrap()
            .affine()
            .translation
            * node.inverse_scale_factor();
        for (control, node, transform) in world
            .query::<(&Control, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
        {
            let center = transform.affine().translation * node.inverse_scale_factor();
            match control.action {
                Action::Tool(Tool::Scale) => assert!(center.x < position.x),
                Action::LocalSpace => assert!(position.x < center.x),
                _ => continue,
            }
            assert!((center.y - position.y).abs() < 1.0);
        }
        position
    };
    move_pointer(&mut app, window, extrude_position);
    capture_when(
        &mut app,
        &target,
        &output.join("02-extrude-toolbar.png"),
        |_| true,
    );
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 3);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .faces
            .len(),
        10
    );
    send(&mut app, EditAction::Undo);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .faces
            .len(),
        6
    );
    // The original shortcut must produce the same topology as the new header button.
    move_pointer(&mut app, window, rect.center());
    key(&mut app, window, KeyCode::KeyE, Key::Character("e".into()));
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .faces
            .len(),
        10
    );
    let before_move = app.world().resource::<State>().documents[&0]
        .active()
        .unwrap()
        .geometry
        .positions
        .clone();
    let selected = {
        let doc = &app.world().resource::<State>().documents[&0];
        doc.selection
            .selected_vertices(&doc.active().unwrap().geometry)
    };
    key(&mut app, window, KeyCode::KeyG, Key::Character("g".into()));
    assert!(app.world().resource::<State>().gesture.is_some());
    key(&mut app, window, KeyCode::KeyX, Key::Character("x".into()));
    key(
        &mut app,
        window,
        KeyCode::Digit1,
        Key::Character("1".into()),
    );
    assert!(
        app.world().resource::<State>().gesture.is_some(),
        "Typed digits must not switch component mode during a transform"
    );
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert!(app.world().resource::<State>().gesture.is_none());
    let after_move = app.world().resource::<State>().documents[&0]
        .active()
        .unwrap()
        .geometry
        .positions
        .clone();
    for (i, (before, after)) in before_move.iter().zip(&after_move).enumerate() {
        assert!(
            (*after
                - *before
                - if selected.contains(&i) {
                    Vec3::X
                } else {
                    Vec3::ZERO
                })
            .length()
                < 0.0001
        );
    }
    key(&mut app, window, KeyCode::KeyG, Key::Character("g".into()));
    move_pointer(&mut app, window, rect.center() + Vec2::new(40.0, -20.0));
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .positions,
        after_move
    );
    assert!(app.world().resource::<Editor>().dirty_models.contains(&0));
    click_control(&mut app, window, |a| matches!(a, Action::Tool(Tool::Move)));
    key(&mut app, window, KeyCode::Home, Key::Home);
    warm(&mut app, 5);
    capture_when(
        &mut app,
        &target,
        &output.join("02-edit-mode.png"),
        |image| {
            image
                .pixels()
                .filter(|p| p[0] > 180 && p[1] > 90 && p[2] < 120)
                .count()
                > 300
        },
    );

    // Object fields are real editable text controls, including failed validation.
    send(&mut app, EditAction::Mode(Mode::Object));
    edit_field(&mut app, controls::Field::Transform(0), "2.5");
    move_pointer(&mut app, window, rect.center());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .transform
            .translation
            .x,
        2.5
    );
    edit_field(&mut app, controls::Field::Transform(6), "0");
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert!(app.world().resource::<State>().input_error.is_some());
    send(&mut app, EditAction::Save);
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(app.world().resource::<State>().input_error.is_none());
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .transform
            .scale,
        Vec3::ONE
    );

    // A second model has its own mode, mesh selection, history and dirty state.
    app.world_mut().write_message(Action::ViewModel(1));
    model_ready(&mut app, 1);
    assert_eq!(app.world().resource::<State>().mode(Some(1)), Mode::Object);
    assert!(!app.world().resource::<State>().documents[&1].dirty());
    app.world_mut().write_message(Action::ViewModel(0));
    warm(&mut app, 12);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .transform
            .translation
            .x,
        2.5
    );
    send(&mut app, EditAction::Save);
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<State>().saving.is_none()
            && !app.world().resource::<State>().documents[&0].dirty()
        {
            break;
        }
    }
    assert!(
        !app.world().resource::<State>().documents[&0].dirty(),
        "{}",
        app.world().resource::<Editor>().status
    );
    let saved_bytes = std::fs::read(&path).unwrap();
    assert_ne!(saved_bytes, original_bytes);
    bevy::gltf::gltf::Gltf::from_slice(&saved_bytes).unwrap();
    let saved_source = document::glb::Glb::load(&path).unwrap();
    assert_eq!(
        saved_source.json["materials"],
        original_source.json["materials"]
    );
    assert_eq!(
        saved_source.json["textures"],
        original_source.json["textures"]
    );
    assert_eq!(saved_source.json["images"], original_source.json["images"]);
    assert_eq!(app.world().resource::<Editor>().edits, scene_edits);
    assert_eq!(app.world().resource::<Editor>().undo, scene_history);

    // Reopening reads the saved container, then close/cancel/discard protects edits.
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 8);
    assert!(!app.world().resource::<State>().documents.contains_key(&0));
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .transform
            .translation
            .x,
        2.5
    );
    send(&mut app, EditAction::Operate(Operation::Duplicate));
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 4);
    assert!(matches!(
        app.world().resource::<Editor>().pending,
        Some(PendingAction::CloseModelTab(0))
    ));
    app.world_mut().write_message(Action::CancelPending);
    warm(&mut app, 4);
    assert!(app.world().resource::<Editor>().pending.is_none());
    assert!(app.world().resource::<State>().documents[&0].dirty());
    app.world_mut().write_message(Action::CloseModelTab(0));
    warm(&mut app, 4);
    app.world_mut().write_message(Action::DiscardAndContinue);
    warm(&mut app, 8);
    assert!(!app.world().resource::<State>().documents.contains_key(&0));
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);

    app.world_mut().write_message(Action::ViewModel(1));
    warm(&mut app, 8);
    send(&mut app, EditAction::Operate(Operation::Duplicate));
    app.world_mut().write_message(Action::CloseModelTab(1));
    warm(&mut app, 4);
    app.world_mut().write_message(Action::SaveAndContinue);
    for _ in 0..240 {
        frame(&mut app);
        if !app.world().resource::<Editor>().model_tabs.contains(&1) {
            break;
        }
    }
    assert!(
        !app.world().resource::<Editor>().model_tabs.contains(&1),
        "{}",
        app.world().resource::<Editor>().status
    );
    assert!(app.world().resource::<Editor>().dirty_models.is_empty());
    let blue = bevy::gltf::gltf::Gltf::from_slice(
        &std::fs::read(directory.join("models/blue_block.glb")).unwrap(),
    )
    .unwrap();
    assert_eq!(blue.nodes().filter(|n| n.mesh().is_some()).count(), 2);
    drop(app);
    // This directory is the unique temporary project created above.
    assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
    std::fs::remove_dir_all(directory).unwrap();
    eprintln!("Model edit screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; verifies transparent X-Ray, hidden-face picking, Alt+Z and material restoration"]
fn gpu_model_xray_reveals_hidden_faces_without_changing_materials() {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    let scene = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::Y * 0.5;
        viewer.distance = 3.8;
        viewer.yaw = 0.55;
        viewer.pitch = 0.32;
    }
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Face));
    send(&mut app, EditAction::Deselect);
    let objects = app.world().resource::<State>().documents[&0]
        .objects
        .clone();
    let rect = canvas_rect(&mut app);
    move_pointer(&mut app, window, rect.center());
    let (hidden_face, hidden_point, region) = {
        let state = app.world().resource::<State>();
        let viewer = app.world().resource::<Viewer>();
        let object = state.documents[&0].active().unwrap();
        let origin = view_pose(viewer).translation;
        let (face, point) = object
            .geometry
            .faces
            .iter()
            .enumerate()
            .find_map(|(i, f)| {
                let point = object.transform.transform_point(object.geometry.center(f));
                let distance = point.distance(origin);
                let (_, _, surface) = state.bvh.cast(origin, (point - origin).normalize(), None)?;
                (surface + 0.01 < distance).then_some((i, input::project(viewer, rect, point)?))
            })
            .expect("The cube must have a hidden face with a projected center");
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for p in &object.geometry.positions {
            let p = input::project(viewer, rect, object.transform.transform_point(*p)).unwrap();
            min = min.min(p);
            max = max.max(p);
        }
        (face, point, Rect::from_corners(min, max))
    };
    move_pointer(&mut app, window, hidden_point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert!(
        !app.world().resource::<State>().documents[&0]
            .selection
            .faces
            .contains(&hidden_face)
    );
    send(&mut app, EditAction::Deselect);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-xray-qa");
    std::fs::create_dir_all(&output).unwrap();
    let solid = capture_when(&mut app, &target, &output.join("01-solid.png"), |_| true);
    let sources: Vec<_> = {
        let world = app.world_mut();
        world
            .query::<&xray::SourceMaterial>()
            .iter(world)
            .map(|m| m.0.clone())
            .collect()
    };
    assert!(!sources.is_empty());
    let original: Vec<_> = sources
        .iter()
        .map(|handle| {
            let m = app
                .world()
                .resource::<Assets<StandardMaterial>>()
                .get(handle)
                .unwrap();
            (m.base_color, m.alpha_mode, m.cull_mode, m.double_sided)
        })
        .collect();
    click_control(&mut app, window, |action| {
        matches!(
            action,
            Action::ModelView(ModelViewAction::Edit(EditAction::Xray))
        )
    });
    warm(&mut app, 20);
    assert!(app.world().resource::<State>().xray);
    {
        let world = app.world_mut();
        let assets = world.resource::<Assets<StandardMaterial>>();
        for (handle, expected) in sources.iter().zip(&original) {
            let m = assets.get(handle).unwrap();
            assert_eq!(
                (m.base_color, m.alpha_mode, m.cull_mode, m.double_sided),
                *expected
            );
        }
        let mut query = world.query::<(&xray::SourceMaterial, &MeshMaterial3d<StandardMaterial>)>();
        for (source, displayed) in query.iter(world) {
            assert_ne!(source.0, displayed.0);
            let m = world
                .resource::<Assets<StandardMaterial>>()
                .get(&displayed.0)
                .unwrap();
            assert_eq!(m.alpha_mode, AlphaMode::Blend);
            assert!(m.base_color.alpha() <= 0.22);
        }
    }
    let transparent = capture_when(&mut app, &target, &output.join("02-edit-xray.png"), |_| {
        true
    });
    let differences = |a: &image::RgbaImage, b: &image::RgbaImage| {
        let mut count = 0;
        for y in region.min.y.ceil() as u32..region.max.y.floor() as u32 {
            for x in region.min.x.ceil() as u32..region.max.x.floor() as u32 {
                if a.get_pixel(x, y).0[..3]
                    .iter()
                    .zip(&b.get_pixel(x, y).0[..3])
                    .map(|(a, b)| a.abs_diff(*b) as u32)
                    .sum::<u32>()
                    > 35
                {
                    count += 1;
                }
            }
        }
        count
    };
    let changed = differences(&solid, &transparent);
    assert!(
        changed > 1000,
        "X-Ray must visibly reveal the background through the surface"
    );
    move_pointer(&mut app, window, hidden_point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert!(
        app.world().resource::<State>().documents[&0]
            .selection
            .faces
            .contains(&hidden_face),
        "X-Ray must pick a hidden face by its center"
    );
    send(&mut app, EditAction::Mode(Mode::Object));
    capture_when(
        &mut app,
        &target,
        &output.join("03-object-xray.png"),
        |_| true,
    );
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Deselect);
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::AltLeft,
            logical_key: Key::Alt,
            state,
            text: None,
            repeat: false,
            window,
        });
        warm(&mut app, 2);
        if state == ButtonState::Pressed {
            key(&mut app, window, KeyCode::KeyZ, Key::Character("z".into()));
        }
    }
    assert!(!app.world().resource::<State>().xray);
    {
        let world = app.world_mut();
        for (source, displayed) in world
            .query::<(&xray::SourceMaterial, &MeshMaterial3d<StandardMaterial>)>()
            .iter(world)
        {
            assert_eq!(source.0, displayed.0);
        }
    }
    let restored = capture_when(&mut app, &target, &output.join("04-restored.png"), |_| true);
    assert!(differences(&solid, &restored) < changed / 5);
    let state = app.world().resource::<State>();
    assert_eq!(state.documents[&0].objects, objects);
    assert!(!state.documents[&0].dirty());
    assert!(state.documents[&0].undo.is_empty());
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    eprintln!("Model X-Ray screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; verifies selected face fill, vertex/edge contrast, accurate labels and X-Ray feedback"]
fn gpu_model_selection_feedback_identifies_faces_edges_and_vertices() {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    let scene = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::Y * 0.5;
        viewer.distance = 2.2;
        viewer.yaw = 0.55;
        viewer.pitch = 0.48;
    }
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Face));
    send(&mut app, EditAction::Deselect);
    let rect = canvas_rect(&mut app);
    move_pointer(&mut app, window, rect.center());
    let (objects, top, top_point, samples, vertex, edge) = {
        let state = app.world().resource::<State>();
        let doc = &state.documents[&0];
        let object = doc.active().unwrap();
        let world = |p| object.transform.transform_point(p);
        let top = object
            .geometry
            .faces
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                world(object.geometry.center(a))
                    .y
                    .total_cmp(&world(object.geometry.center(b)).y)
            })
            .unwrap()
            .0;
        let center = world(object.geometry.center(&object.geometry.faces[top]));
        let viewer = app.world().resource::<Viewer>();
        let samples: Vec<_> = object.geometry.faces[top]
            .corners
            .iter()
            .map(|c| {
                input::project(
                    viewer,
                    rect,
                    center.lerp(world(object.geometry.positions[c.vertex]), 0.55),
                )
                .unwrap()
            })
            .collect();
        let eye = view_pose(viewer).translation;
        let vertex = object
            .geometry
            .positions
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                world(**a)
                    .distance_squared(eye)
                    .total_cmp(&world(**b).distance_squared(eye))
            })
            .unwrap()
            .0;
        let edge = object
            .geometry
            .edges()
            .into_keys()
            .min_by(|a, b| {
                let midpoint = |e: &Edge| {
                    world((object.geometry.positions[e.0] + object.geometry.positions[e.1]) * 0.5)
                };
                midpoint(a)
                    .distance_squared(eye)
                    .total_cmp(&midpoint(b).distance_squared(eye))
            })
            .unwrap();
        (
            doc.objects.clone(),
            top,
            input::project(viewer, rect, center).unwrap(),
            samples,
            vertex,
            edge,
        )
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-selection-qa");
    std::fs::create_dir_all(&output).unwrap();
    let before = capture_when(
        &mut app,
        &target,
        &output.join("01-no-selection.png"),
        |_| true,
    );
    move_pointer(&mut app, window, top_point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 12);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .faces,
        BTreeSet::from([top])
    );
    let face_image = capture_when(&mut app, &target, &output.join("02-face.png"), |_| true);
    let nebula = |pixel: &image::Rgba<u8>| {
        let [r, g, b, _] = pixel.0.map(u16::from);
        b + 8 > r && b > g + 15
    };
    let stellar = |pixel: &image::Rgba<u8>| {
        let [r, g, b, _] = pixel.0.map(u16::from);
        r > 205 && g > 200 && b > 145 && r > g + 4 && g > b + 12
    };
    let sample = |image: &image::RgbaImage, p: Vec2| {
        *image.get_pixel(p.x.round() as u32, p.y.round() as u32)
    };
    assert!(
        samples
            .iter()
            .filter(|&&p| nebula(&sample(&face_image, p)) && !nebula(&sample(&before, p)))
            .count()
            >= 2,
        "The selected face must have an amethyst interior, independently of the GLTF material"
    );
    warm(&mut app, 120);
    let flowing = capture_when(&mut app, &target, &output.join("02-face-flow.png"), |_| {
        true
    });
    let animated = samples
        .iter()
        .map(|&p| {
            (-3..=3)
                .flat_map(|y| (-3..=3).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let p = p + Vec2::new(x as f32, y as f32);
                    sample(&face_image, p)
                        .0
                        .into_iter()
                        .zip(sample(&flowing, p).0)
                        .take(3)
                        .any(|(a, b)| a.abs_diff(b) > 2)
                })
                .count()
        })
        .sum::<usize>();
    assert!(
        animated > 30,
        "The nebula must drift gently within the selected face: {animated}"
    );
    let label = app
        .world()
        .resource::<State>()
        .selection_label(Some(0))
        .unwrap();
    assert!(label.contains("Грани · выбрано 1 из 6"), "{label}");
    let status = app.world().resource::<ViewerUi>().status;
    assert!(
        app.world()
            .get::<Text>(status)
            .unwrap()
            .0
            .contains("Грани · выбрано 1 из 6")
    );

    send(&mut app, EditAction::Element(Element::Vertex));
    let converted_face = capture_when(
        &mut app,
        &target,
        &output.join("03-face-to-vertices.png"),
        |_| true,
    );
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .vertices
            .len(),
        4
    );
    assert!(
        samples
            .iter()
            .all(|&p| !nebula(&sample(&converted_face, p)) || nebula(&sample(&before, p))),
        "Vertex mode must remove the selected face fill after conversion"
    );
    send(&mut app, EditAction::Deselect);
    let vertex_point = {
        let state = app.world().resource::<State>();
        let object = state.documents[&0].active().unwrap();
        input::project(
            app.world().resource::<Viewer>(),
            rect,
            object
                .transform
                .transform_point(object.geometry.positions[vertex]),
        )
        .unwrap()
    };
    move_pointer(&mut app, window, vertex_point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    let vertex_image = capture_when(&mut app, &target, &output.join("03-vertex.png"), |_| true);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .vertices,
        BTreeSet::from([vertex])
    );
    assert!(
        app.world()
            .resource::<State>()
            .selection_label(Some(0))
            .unwrap()
            .contains("Вершины · выбрано 1 из 8")
    );
    let mut colored = 0;
    for y in -12..=12 {
        for x in -12..=12 {
            if stellar(&sample(
                &vertex_image,
                vertex_point + Vec2::new(x as f32, y as f32),
            )) {
                colored += 1;
            }
        }
    }
    assert!(
        colored >= 30,
        "A selected vertex must have a luminous orbit visible around the transform pivot: {colored} pixels"
    );

    send(&mut app, EditAction::Element(Element::Edge));
    send(&mut app, EditAction::Deselect);
    let edge_point = {
        let state = app.world().resource::<State>();
        let object = state.documents[&0].active().unwrap();
        let p = (object.geometry.positions[edge.0] + object.geometry.positions[edge.1]) * 0.5;
        input::project(
            app.world().resource::<Viewer>(),
            rect,
            object.transform.transform_point(p),
        )
        .unwrap()
    };
    move_pointer(&mut app, window, edge_point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    send(&mut app, EditAction::Tool(Tool::Rotate));
    let edge_image = capture_when(&mut app, &target, &output.join("04-edge.png"), |_| true);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .edges,
        BTreeSet::from([edge])
    );
    assert!(
        app.world()
            .resource::<State>()
            .selection_label(Some(0))
            .unwrap()
            .contains("Рёбра · выбрано 1 из 12")
    );

    click_control(&mut app, window, |action| {
        matches!(
            action,
            Action::ModelView(ModelViewAction::Edit(EditAction::Element(Element::Vertex)))
        )
    });
    let converted_edge = capture_when(
        &mut app,
        &target,
        &output.join("04-edge-to-vertices.png"),
        |_| true,
    );
    let (a, b) = {
        let state = app.world().resource::<State>();
        let doc = &state.documents[&0];
        assert_eq!(doc.selection.kind, Element::Vertex);
        assert_eq!(doc.selection.vertices, BTreeSet::from([edge.0, edge.1]));
        assert!(doc.selection.edges.is_empty());
        assert!(doc.selection.faces.is_empty());
        let object = doc.active().unwrap();
        let viewer = app.world().resource::<Viewer>();
        let project = |v| {
            input::project(
                viewer,
                rect,
                object
                    .transform
                    .transform_point(object.geometry.positions[v]),
            )
            .unwrap()
        };
        (project(edge.0), project(edge.1))
    };
    let colored_patch = |image: &image::RgbaImage, p: Vec2, radius: i32| {
        (-radius..=radius)
            .flat_map(|y| (-radius..=radius).map(move |x| (x, y)))
            .filter(|&(x, y)| stellar(&sample(image, p + Vec2::new(x as f32, y as f32))))
            .count()
    };
    for p in [a, b] {
        let count = colored_patch(&converted_edge, p, 6);
        assert!(
            count >= 20,
            "Each converted endpoint must have a visible filled vertex marker: {p:?}, {count} pixels"
        );
    }
    let interior = |image: &image::RgbaImage| {
        [0.15, 0.25, 0.35, 0.65, 0.75, 0.85]
            .into_iter()
            .map(|t| colored_patch(image, a.lerp(b, t), 2))
            .sum::<usize>()
    };
    assert!(
        interior(&edge_image) > 40,
        "Edge mode must highlight the line"
    );
    let normal = (b - a).normalize().perp();
    let contrast_sections = [0.15, 0.25, 0.35, 0.65, 0.75, 0.85]
        .into_iter()
        .filter(|&t| {
            let point = a.lerp(b, t);
            let dark = (-7..=7)
                .filter(|&offset| {
                    let point = point + normal * offset as f32;
                    let selected = sample(&edge_image, point).0.map(i32::from);
                    let original = sample(&before, point).0.map(i32::from);
                    selected[2] > selected[0] + 8
                        && selected[2] > selected[1] + 18
                        && (0..3).map(|i| original[i] - selected[i]).sum::<i32>() > 100
                })
                .count();
            dark >= 2 && colored_patch(&edge_image, point, 2) >= 4
        })
        .count();
    assert!(
        contrast_sections >= 5,
        "The luminous edge needs a contrasting plum body along its length on the bright model: {contrast_sections}/6 sections"
    );
    assert!(
        interior(&converted_edge) < interior(&edge_image) / 4,
        "Switching to vertices must remove the connecting edge highlight"
    );

    send(&mut app, EditAction::Xray);
    send(&mut app, EditAction::Element(Element::Face));
    send(&mut app, EditAction::Deselect);
    let (hidden, point) = {
        let state = app.world().resource::<State>();
        let object = state.documents[&0].active().unwrap();
        let viewer = app.world().resource::<Viewer>();
        let eye = view_pose(viewer).translation;
        object
            .geometry
            .faces
            .iter()
            .enumerate()
            .find_map(|(i, f)| {
                let p = object.transform.transform_point(object.geometry.center(f));
                let (_, _, near) = state.bvh.cast(eye, (p - eye).normalize(), None)?;
                (near + 0.01 < p.distance(eye)).then_some((i, input::project(viewer, rect, p)?))
            })
            .unwrap()
    };
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    capture_when(&mut app, &target, &output.join("05-xray-face.png"), |_| {
        true
    });
    assert!(
        app.world().resource::<State>().documents[&0]
            .selection
            .faces
            .contains(&hidden)
    );
    let state = app.world().resource::<State>();
    assert_eq!(state.documents[&0].objects, objects);
    assert!(!state.documents[&0].dirty());
    assert!(state.documents[&0].undo.is_empty());
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    eprintln!("Model selection screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; verifies continuous model edges at shallow slopes and different zoom levels"]
fn gpu_model_wire_edges_remain_continuous_after_antialiasing() {
    use bevy::gizmos::config::GizmoConfigStore;
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, _, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Vertex));
    send(&mut app, EditAction::Deselect);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-wire-qa");
    std::fs::create_dir_all(&output).unwrap();
    let (a, b) = {
        let state = app.world().resource::<State>();
        let object = state.documents[&0].active().unwrap();
        let world = |p| object.transform.transform_point(p);
        let edge = object
            .geometry
            .edges()
            .into_keys()
            .filter(|e| {
                let delta = object.geometry.positions[e.0] - object.geometry.positions[e.1];
                delta.x.abs() > 0.9 && delta.y.abs() < 0.001 && delta.z.abs() < 0.001
            })
            .max_by(|a, b| {
                let score = |e: &Edge| {
                    let p = world(
                        (object.geometry.positions[e.0] + object.geometry.positions[e.1]) * 0.5,
                    );
                    p.y + p.z
                };
                score(a).total_cmp(&score(b))
            })
            .unwrap();
        (
            world(object.geometry.positions[edge.0]),
            world(object.geometry.positions[edge.1]),
        )
    };
    let width = app
        .world()
        .resource::<GizmoConfigStore>()
        .config::<input::ModelGizmos>()
        .0
        .line
        .width;
    for (label, line_width) in [("thin-control", 1.25), ("solid", width)] {
        app.world_mut()
            .resource_mut::<GizmoConfigStore>()
            .config_mut::<input::ModelGizmos>()
            .0
            .line
            .width = line_width;
        for (case, distance, yaw, pitch, ortho) in [
            ("close", 1.7, 0.009, 0.4, false),
            ("medium", 2.6, 0.009, 0.4, false),
            ("distant", 5.0, 0.035, 0.4, false),
            ("diagonal", 2.6, 0.48, 0.45, false),
            ("ortho", 2.6, 0.009, 0.4, true),
        ] {
            {
                let mut viewer = app.world_mut().resource_mut::<Viewer>();
                viewer.center = Vec3::splat(0.5);
                viewer.distance = distance;
                viewer.yaw = yaw;
                viewer.pitch = pitch;
                viewer.orthographic = ortho;
            }
            warm(&mut app, 6);
            let image = capture_when(
                &mut app,
                &target,
                &output.join(format!("{label}-{case}.png")),
                |_| true,
            );
            let rect = canvas_rect(&mut app);
            let viewer = app.world().resource::<Viewer>();
            let pa = input::project(viewer, rect, a).unwrap();
            let pb = input::project(viewer, rect, b).unwrap();
            let length = pa.distance(pb).ceil() as usize;
            let normal = (pb - pa).normalize().perp();
            let mut present = 0;
            let mut gap = 0;
            let mut longest_gap = 0;
            let samples = length * 8 / 10;
            for i in 0..samples {
                let p = pa.lerp(pb, 0.1 + i as f32 / length as f32);
                let dark = [-1.0, 0.0, 1.0].into_iter().any(|offset| {
                    let p = p + normal * offset;
                    let pixel = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
                    pixel[0] < 145 && pixel[1] < 145 && pixel[2] < 145
                });
                if dark {
                    present += 1;
                    gap = 0;
                } else {
                    gap += 1;
                    longest_gap = longest_gap.max(gap);
                }
            }
            eprintln!(
                "{label}-{case}: dark coverage {present}/{samples}, longest gap {longest_gap}"
            );
            if label == "solid" {
                assert!(
                    present * 100 >= samples * 97,
                    "Visible edges must have continuous dark coverage: {case}"
                );
                assert!(
                    longest_gap <= 2,
                    "Visible edges must not have dashed gaps: {case}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires GPU; verifies automatic cursor rectangles for faces, visible/X-Ray vertices and model objects"]
fn gpu_model_cursor_area_selects_components_and_objects() {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.distance = 3.5;
        viewer.center = Vec3::Y * 0.5;
        viewer.yaw = 0.55;
        viewer.pitch = 0.48;
    }
    let rect = canvas_rect(&mut app);
    move_pointer(&mut app, window, rect.center());
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Face));
    send(&mut app, EditAction::Deselect);
    let top = {
        let state = app.world().resource::<State>();
        let object = state.documents[&0].active().unwrap();
        object
            .geometry
            .faces
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                object
                    .geometry
                    .center(a)
                    .y
                    .total_cmp(&object.geometry.center(b).y)
            })
            .unwrap()
            .0
    };
    let center = {
        let state = app.world().resource::<State>();
        let o = state.documents[&0].active().unwrap();
        input::project(
            app.world().resource::<Viewer>(),
            rect,
            o.transform
                .transform_point(o.geometry.center(&o.geometry.faces[top])),
        )
        .unwrap()
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/area-selection-qa");
    std::fs::create_dir_all(&output).unwrap();
    let drag = |app: &mut App, area: Rect| {
        move_pointer(app, window, area.min);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, area.max);
        mouse(app, window, ButtonState::Released);
        warm(app, 2);
    };
    move_pointer(&mut app, window, center - Vec2::splat(12.0));
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::splat(12.0));
    assert!(app.world().resource::<State>().box_start.is_some());
    capture_when(
        &mut app,
        &target,
        &output.join("03-model-face-drag.png"),
        |_| true,
    );
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 3);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .faces,
        BTreeSet::from([top]),
        "Face rectangles select centers rather than requiring every corner inside"
    );
    send(&mut app, EditAction::Element(Element::Vertex));
    send(&mut app, EditAction::Deselect);
    let all = {
        let state = app.world().resource::<State>();
        let o = state.documents[&0].active().unwrap();
        let viewer = app.world().resource::<Viewer>();
        let r = crate::editor::area_selection::projected_bounds(
            o.geometry
                .positions
                .iter()
                .map(|p| o.transform.transform_point(*p)),
            |p| input::project(viewer, rect, p),
        )
        .unwrap();
        Rect::from_corners(r.min - Vec2::splat(18.0), r.max + Vec2::splat(18.0))
    };
    drag(&mut app, all);
    let visible = app.world().resource::<State>().documents[&0]
        .selection
        .vertices
        .len();
    assert!(
        visible > 0 && visible < 8,
        "Solid mode must keep hidden vertices out of rectangle selection; got {visible}"
    );
    send(&mut app, EditAction::Xray);
    send(&mut app, EditAction::Deselect);
    drag(&mut app, all);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .vertices
            .len(),
        8
    );
    let (index, p) = {
        let state = app.world().resource::<State>();
        let doc = &state.documents[&0];
        let o = doc.active().unwrap();
        let viewer = app.world().resource::<Viewer>();
        let pivot = input::project(viewer, rect, doc.pivot().unwrap()).unwrap();
        o.geometry
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    i,
                    input::project(viewer, rect, o.transform.transform_point(*p)).unwrap(),
                )
            })
            .max_by(|(_, a), (_, b)| {
                a.distance_squared(pivot)
                    .total_cmp(&b.distance_squared(pivot))
            })
            .unwrap()
    };
    let modifier = |app: &mut App, code, logical: Key, down| {
        app.world_mut().write_message(KeyboardInput {
            key_code: code,
            logical_key: logical,
            state: if down {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
            text: None,
            repeat: false,
            window,
        });
        warm(app, 2);
    };
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, true);
    drag(&mut app, Rect::from_center_size(p, Vec2::splat(12.0)));
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, false);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .vertices
            .len(),
        7
    );
    assert!(
        !app.world().resource::<State>().documents[&0]
            .selection
            .vertices
            .contains(&index)
    );
    modifier(&mut app, KeyCode::ShiftLeft, Key::Shift, true);
    drag(&mut app, Rect::from_center_size(p, Vec2::splat(12.0)));
    modifier(&mut app, KeyCode::ShiftLeft, Key::Shift, false);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .selection
            .vertices
            .len(),
        8
    );
    send(&mut app, EditAction::Mode(Mode::Object));
    send(&mut app, EditAction::SelectAll);
    send(&mut app, EditAction::Operate(Operation::Duplicate));
    {
        let mut state = app.world_mut().resource_mut::<State>();
        let doc = state.documents.get_mut(&0).unwrap();
        doc.objects.last_mut().unwrap().transform.translation.x += 1.5;
        doc.revision += 1;
        doc.ui_revision += 1;
    }
    send(&mut app, EditAction::Deselect);
    let (objects, history, area) = {
        let state = app.world().resource::<State>();
        let doc = &state.documents[&0];
        let viewer = app.world().resource::<Viewer>();
        let r = crate::editor::area_selection::projected_bounds(
            doc.objects.iter().flat_map(|o| {
                o.geometry
                    .positions
                    .iter()
                    .map(move |p| o.transform.transform_point(*p))
            }),
            |p| input::project(viewer, rect, p),
        )
        .unwrap();
        (
            doc.objects.clone(),
            doc.undo.len(),
            Rect::from_corners(r.min - Vec2::splat(18.0), r.max + Vec2::splat(18.0)),
        )
    };
    drag(&mut app, area);
    assert_eq!(
        app.world().resource::<State>().documents[&0].selected.len(),
        2
    );
    assert_eq!(
        app.world().resource::<State>().documents[&0].objects,
        objects
    );
    assert_eq!(
        app.world().resource::<State>().documents[&0].undo.len(),
        history
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-model-object-group.png"),
        |_| true,
    );
    eprintln!("Model area screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; verifies the actual previously edited cube has six non-planar quad faces"]
fn gpu_model_edit_warped_cube_has_whole_quad_faces_without_diagonals() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-warped-quad-{}",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy_directory(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model-edit/warped-cube.glb"),
        directory.join("models/block.glb"),
    )
    .unwrap();
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        let geometry = &doc.active().unwrap().geometry;
        assert_eq!(geometry.faces.len(), 6);
        assert_eq!(geometry.edges().len(), 12);
        assert!(geometry.faces.iter().all(|f| f.corners.len() == 4));
        assert!(!doc.dirty());
    }
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Face));
    send(&mut app, EditAction::Tool(Tool::Move));
    send(&mut app, EditAction::Deselect);
    app.world_mut().resource_mut::<Viewer>().distance *= 0.7;
    warm(&mut app, 5);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/model-edit-qa/warped-cube-quads.png");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    capture_when(&mut app, &target, &output, |_| true);
    let rect = canvas_rect(&mut app);
    move_pointer(&mut app, window, rect.center());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_eq!(doc.selection.faces.len(), 1);
        let selected = *doc.selection.faces.first().unwrap();
        assert_eq!(
            doc.active().unwrap().geometry.faces[selected].corners.len(),
            4
        );
    }
    send(&mut app, EditAction::Operate(Operation::Extrude));
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .faces
            .len(),
        10
    );
    send(&mut app, EditAction::Undo);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .faces
            .len(),
        6
    );
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    println!("Warped quad screenshot: {}", output.display());
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
