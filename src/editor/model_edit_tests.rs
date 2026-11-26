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

#[test]
#[ignore = "requires GPU; clicks the mesh-editing toolbar buttons"]
fn gpu_mesh_toolbar_starts_bevel_loop_cut_and_knife() {
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Edge));
    {
        let mut state = app.world_mut().resource_mut::<State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let edge = *doc.active().unwrap().geometry.edges().keys().next().unwrap();
        doc.selection.edges = [edge].into();
    }
    warm(&mut app, 3);
    for (label, started) in [
        ("Фаска (Bevel) · Ctrl+B", input::Gesture::is_beveling as fn(&input::Gesture) -> bool),
        ("Разрез петлёй (Loop Cut) · Ctrl+R", input::Gesture::is_loop_cutting),
        ("Нож (Knife) · K", input::Gesture::is_knifing),
    ] {
        let position = {
            let world = app.world_mut();
            let (_, node, transform) = world
                .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
                .iter(world)
                .find(|(name, node, _)| name.0 == label && !node.is_empty())
                .expect("Mesh tool must be visible in the toolbar");
            transform.affine().translation * node.inverse_scale_factor()
        };
        move_pointer(&mut app, window, position);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        warm(&mut app, 3);
        assert!(app.world().resource::<State>().gesture.as_ref().is_some_and(started), "{label}");
        key(&mut app, window, KeyCode::Escape, Key::Escape);
        assert!(app.world().resource::<State>().gesture.is_none());
    }
}

#[test]
#[ignore = "requires GPU; exercises the real Loop Cut and Knife input paths"]
fn gpu_cut_tools_preview_cancel_and_commit_once() {
    let (mut app, window, _) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    send(&mut app, EditAction::Mode(Mode::Edit));
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.yaw = 0.0;
        viewer.pitch = 0.0;
    }
    warm(&mut app, 3);
    let rect = canvas_rect(&mut app);
    let (original, undo_len, front_face, front_center_y, rails, line) = {
        let world = app.world();
        let state = world.resource::<State>();
        let viewer = world.resource::<Viewer>();
        let doc = &state.documents[&0];
        let object = doc.active().unwrap();
        let mesh = &object.geometry;
        let (fi, face) = mesh.faces.iter().enumerate().max_by(|(_,a),(_,b)| {
            mesh.normal(a).normalize_or_zero().z.total_cmp(&mesh.normal(b).normalize_or_zero().z)
        }).unwrap();
        let show = |v| input::project(viewer, rect, object.transform.transform_point(v)).unwrap();
        let rails: Vec<_> = face.edges().take(2).map(|(a,b)| {
            let midpoint = (mesh.positions[a] + mesh.positions[b]) * 0.5;
            (show(midpoint), midpoint)
        }).collect();
        let screens: Vec<_> = face.corners.iter().map(|corner| show(mesh.positions[corner.vertex])).collect();
        let center: Vec2 = screens.iter().copied().sum::<Vec2>() / screens.len() as f32;
        let half = screens.iter().map(|p| (p.x-center.x).abs()).fold(0.0,f32::max);
        let line = [-1.0, 1.0].map(|side| center+Vec2::new(side*(half+12.0), 7.0));
        (doc.objects.clone(), doc.undo.len(), fi, mesh.center(face).y, rails, line)
    };
    let modifier = |app: &mut App, key_code, logical_key, state| {
        app.world_mut().write_message(KeyboardInput {
            key_code, logical_key, state, text: None, repeat: false, window,
        });
        warm(app, 2);
    };
    let click = |app: &mut App| {
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 2);
    };

    let begin_loop = |app: &mut App, rail| {
        move_pointer(app, window, rail);
        modifier(app, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
        key(app, window, KeyCode::KeyR, Key::Character("r".into()));
        modifier(app, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
        assert!(app.world().resource::<State>().gesture.as_ref().unwrap().is_loop_cutting());
    };
    for select_strip in [false,true] {
        begin_loop(&mut app, rails[0].0);
        if select_strip { click(&mut app); }
        key(&mut app, window, KeyCode::Escape, Key::Escape);
        let state = app.world().resource::<State>();
        assert!(state.gesture.is_none());
        assert_eq!(state.documents[&0].objects, original);
        assert_eq!(state.documents[&0].undo.len(), undo_len);
    }
    let mut centered_cuts = Vec::new();
    for &(rail, midpoint) in &rails {
        begin_loop(&mut app, rail);
        click(&mut app);
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Right, state, window,
            });
            warm(&mut app, 2);
        }
        {
            let state = app.world().resource::<State>();
            let doc = &state.documents[&0];
            let mesh = &doc.active().unwrap().geometry;
            assert!(state.gesture.is_none());
            assert!(mesh.positions.contains(&midpoint));
            assert_eq!(doc.undo.len(), undo_len + 1);
            centered_cuts.push(doc.objects.clone());
        }
        send(&mut app, EditAction::Undo);
    }
    assert_ne!(centered_cuts[0], centered_cuts[1]);
    begin_loop(&mut app, rails[0].0);
    click(&mut app);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);
    key(&mut app, window, KeyCode::Digit2, Key::Character("2".into()));
    key(&mut app, window, KeyCode::Digit5, Key::Character("5".into()));
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    let cut = app.world().resource::<State>().documents[&0].objects.clone();
    assert_ne!(cut, original);
    key(&mut app, window, KeyCode::F9, Key::F9);
    assert!(app.world().resource::<State>().gesture.as_ref().unwrap().is_loop_cutting());
    click_control(&mut app, window, |action| matches!(
        action, Action::ModelView(ModelViewAction::Edit(EditAction::LoopCut(LoopCutControl::Position)))
    ));
    key(&mut app, window, KeyCode::Digit7, Key::Character("7".into()));
    key(&mut app, window, KeyCode::Digit5, Key::Character("5".into()));
    click_control(&mut app, window, |action| matches!(
        action, Action::ModelView(ModelViewAction::Edit(EditAction::LoopCut(LoopCutControl::Confirm)))
    ));
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_ne!(doc.objects, cut);
        assert_eq!(doc.undo.len(), undo_len + 1);
        assert_eq!(doc.selection.edges.len(), 4);
    }
    send(&mut app, EditAction::Undo);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);

    let begin_knife = |app: &mut App| {
        modifier(app, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
        key(app, window, KeyCode::KeyK, Key::Character("k".into()));
        assert!(app.world().resource::<State>().gesture.as_ref().unwrap().is_knifing());
        for at in line {
            move_pointer(app, window, at);
            click(app);
        }
        modifier(app, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
        assert_eq!(app.world().resource::<State>().documents[&0].objects, original);
    };
    {
        let mut state = app.world_mut().resource_mut::<State>();
        let selection = &mut state.documents.get_mut(&0).unwrap().selection;
        selection.clear();
        selection.kind = Element::Face;
        selection.faces.insert(front_face);
    }
    begin_knife(&mut app);
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
    key(&mut app, window, KeyCode::KeyZ, Key::Character("z".into()));
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(app.world().resource::<State>().gesture.is_none());
    begin_knife(&mut app);
    key(&mut app, window, KeyCode::Space, Key::Space);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_ne!(doc.objects, original);
        assert_eq!(doc.undo.len(), undo_len + 1);
        let mesh = &doc.active().unwrap().geometry;
        assert_eq!(mesh.faces.len(), 7);
        assert!(!doc.selection.edges.is_empty());
        for edge in &doc.selection.edges {
            assert!([edge.0,edge.1].into_iter().all(|v| (mesh.positions[v].y-front_center_y).abs() < 1.0e-5));
        }
    }
    send(&mut app, EditAction::Undo);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);
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
#[ignore = "requires GPU; uses the real Ctrl+B, wheel, cancellation, F9 and undo input paths"]
fn gpu_bevel_shortcut_previews_segments_and_adjusts_one_undo_step() {
    use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
    let (mut app, window, target) = crate::editor::tests::lighting_ui_render::fixture();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Edge));
    let (original, original_selection, original_history) = {
        let mut state = app.world_mut().resource_mut::<State>();
        let doc = state.documents.get_mut(&0).unwrap();
        let edge = *doc.active().unwrap().geometry.edges().keys().next().unwrap();
        doc.selection.edges = [edge].into();
        (doc.objects.clone(), doc.selection.clone(), doc.undo.len())
    };
    let rect = canvas_rect(&mut app);
    let center = rect.center();
    move_pointer(&mut app, window, center);
    let begin = |app: &mut App| {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::ControlLeft,
                logical_key: Key::Control,
                state,
                text: None,
                repeat: false,
                window,
            });
            warm(app, 2);
            if state == ButtonState::Pressed {
                key(app, window, KeyCode::KeyB, Key::Character("b".into()));
            }
        }
        assert!(app.world().resource::<State>().gesture.as_ref().is_some_and(input::Gesture::is_beveling));
    };
    let type_width = |app: &mut App, digit, code| {
        key(app, window, KeyCode::Digit0, Key::Character("0".into()));
        key(app, window, KeyCode::Period, Key::Character(".".into()));
        key(app, window, code, Key::Character(digit));
    };
    let roll = |app: &mut App| {
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 2.0,
            window,
            phase: bevy::input::touch::TouchPhase::Moved,
        });
        warm(app, 3);
    };
    let modifier = |app: &mut App, key_code, logical_key, state| {
        app.world_mut().write_message(KeyboardInput {
            key_code, logical_key, state, text: None, repeat: false, window,
        });
        warm(app, 2);
    };
    let displayed_width = |app: &App| -> f32 {
        app.world().resource::<State>().gesture.as_ref().unwrap()
            .bevel_control_label(BevelControl::Amount).unwrap()
            .split_whitespace().last().unwrap().parse().unwrap()
    };
    let assert_beveled_edges_selected = |app: &App| {
        let doc = &app.world().resource::<State>().documents[&0];
        let mesh = &doc.active().unwrap().geometry;
        let expected: BTreeSet<_> = mesh.faces[original[0].geometry.faces.len()..]
            .iter()
            .flat_map(|face| face.edges().map(|(a, b)| Edge::new(a, b)))
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(doc.selection.kind, Element::Edge);
        assert_eq!(doc.selection.edges, expected);
        assert!(doc.selection.faces.is_empty());
    };
    begin(&mut app);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);
    let direction = {
        let state = app.world().resource::<State>();
        let viewer = app.world().resource::<Viewer>();
        let pivot = state.documents[&0].selection_center().unwrap();
        let delta = center - input::project(viewer, rect, pivot).unwrap_or(center);
        if delta.length() < 8.0 {
            Vec2::new(1.0, -1.0).normalize()
        } else {
            delta.normalize()
        }
    };
    move_pointer(&mut app, window, center + direction * 16.0);
    warm(&mut app, 3);
    assert_ne!(app.world().resource::<State>().documents[&0].objects, original);
    let width = displayed_width(&app);
    modifier(&mut app, KeyCode::ShiftLeft, Key::Shift, ButtonState::Pressed);
    move_pointer(&mut app, window, center + direction * 32.0);
    let fine_width = displayed_width(&app);
    assert!((fine_width - width * 1.1).abs() <= 0.0002);
    modifier(&mut app, KeyCode::ShiftLeft, Key::Shift, ButtonState::Released);
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, ButtonState::Pressed);
    let step = app.world().resource::<State>().documents[&0].amount;
    assert!((displayed_width(&app) - (fine_width / step).round() * step).abs() <= 0.0002);
    modifier(&mut app, KeyCode::ControlLeft, Key::Control, ButtonState::Released);
    type_width(&mut app, "1".into(), KeyCode::Digit1);
    roll(&mut app);
    assert_beveled_edges_selected(&app);
    let guide_image = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/model-edit-qa/bevel-guide.png");
    std::fs::create_dir_all(guide_image.parent().unwrap()).unwrap();
    capture_when(&mut app, &target, &guide_image, |_| true);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_ne!(doc.objects, original);
        assert_eq!(doc.undo.len(), original_history);
        assert_eq!(doc.active().unwrap().geometry.faces.len(), 11);
    }
    assert!(!app.world().resource::<Editor>().dirty_models.contains(&0));
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_eq!(doc.objects, original);
        assert_eq!(doc.selection, original_selection);
    }
    begin(&mut app);
    type_width(&mut app, "1".into(), KeyCode::Digit1);
    roll(&mut app);
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert_beveled_edges_selected(&app);
    let committed = app.world().resource::<State>().documents[&0].objects.clone();
    assert_eq!(app.world().resource::<State>().documents[&0].undo.len(), original_history + 1);
    key(&mut app, window, KeyCode::F9, Key::F9);
    assert!(app.world().resource::<State>().gesture.as_ref().is_some_and(input::Gesture::is_beveling));
    type_width(&mut app, "2".into(), KeyCode::Digit2);
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    assert_beveled_edges_selected(&app);
    assert_ne!(app.world().resource::<State>().documents[&0].objects, committed);
    assert_eq!(app.world().resource::<State>().documents[&0].undo.len(), original_history + 1);
    send(&mut app, EditAction::Undo);
    assert_eq!(app.world().resource::<State>().documents[&0].objects, original);

    send(&mut app, EditAction::Element(Element::Vertex));
    app.world_mut().resource_mut::<State>().documents.get_mut(&0).unwrap().selection.vertices =
        [0].into();
    for (code, logical_key) in [(KeyCode::ControlLeft, Key::Control), (KeyCode::ShiftLeft, Key::Shift)] {
        app.world_mut().write_message(KeyboardInput {
            key_code: code, logical_key, state: ButtonState::Pressed,
            text: None, repeat: false, window,
        });
    }
    warm(&mut app, 2);
    key(&mut app, window, KeyCode::KeyB, Key::Character("b".into()));
    for (code, logical_key) in [(KeyCode::ControlLeft, Key::Control), (KeyCode::ShiftLeft, Key::Shift)] {
        app.world_mut().write_message(KeyboardInput {
            key_code: code, logical_key, state: ButtonState::Released,
            text: None, repeat: false, window,
        });
    }
    warm(&mut app, 2);
    assert_eq!(app.world().resource::<State>().gesture.as_ref().unwrap()
        .bevel_control_label(BevelControl::Affect).as_deref(), Some("Вершины"));
    let custom_y = BevelControl::CustomCoordinate { point: 1, axis: 1 };
    send(&mut app, EditAction::Bevel(BevelControl::CustomProfile));
    type_width(&mut app, "1".into(), KeyCode::Digit1);
    roll(&mut app);
    let rounded = app.world().resource::<State>().documents[&0].objects.clone();
    send(&mut app, EditAction::Bevel(custom_y));
    type_width(&mut app, "2".into(), KeyCode::Digit2);
    assert_ne!(app.world().resource::<State>().documents[&0].objects, rounded);
    send(&mut app, EditAction::Bevel(BevelControl::Confirm));
    let state = app.world().resource::<State>();
    assert!(state.gesture.is_none());
    assert_eq!(state.documents[&0].selection.kind, Element::Vertex);
    assert!(!state.documents[&0].selection.vertices.is_empty());
    assert_eq!(state.documents[&0].undo.len(), original_history + 1);
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

#[test]
#[ignore = "requires GPU; creates a window asset, unwraps it and loads the saved GLB into a scene"]
fn gpu_model_window_workflow_saves_beveled_frame_with_glass_and_uv_seam() {
    fn edit_document(app: &mut App, edit: impl FnOnce(&mut Document)) {
        let mut state = app.world_mut().resource_mut::<State>();
        edit(state.documents.get_mut(&0).unwrap());
    }
    let directory = std::env::temp_dir().join(format!(
        "bevy-window-model-{}", bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    copy_directory(&project.asset_root, &directory);
    project.asset_root = directory.clone();
    let path = directory.join("models/block.glb");
    let (mut app, window, _) = fixture_project(project);
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    send(&mut app, EditAction::SelectAll);
    send(&mut app, EditAction::Operate(Operation::Delete));

    let wood = bevy::color::Srgba::hex("#865328").unwrap();
    let glass = bevy::color::Srgba::hex("#89D0EA80").unwrap();
    let mut first_beam = None;
    for (name, position, size) in [
        ("Верх", Vec3::new(0.0, 2.0, 0.0), Vec3::new(2.0, 0.16, 0.14)),
        ("Низ", Vec3::new(0.0, 0.16, 0.0), Vec3::new(2.0, 0.16, 0.14)),
        ("Слева", Vec3::new(-0.92, 1.08, 0.0), Vec3::new(0.16, 1.84, 0.14)),
        ("Справа", Vec3::new(0.92, 1.08, 0.0), Vec3::new(0.16, 1.84, 0.14)),
        ("Стекло", Vec3::new(0.0, 1.08, 0.0), Vec3::new(1.84, 1.84, 0.02)),
    ] {
        let pane = name == "Стекло";
        send(&mut app, EditAction::Add(if pane { 2 } else { 0 }));
        edit_document(&mut app, |doc| {
            let object = doc.active_mut().unwrap();
            object.name = name.into();
            if !pane {
                first_beam.get_or_insert(object.id);
            }
            let positions = &object.geometry.positions;
            let (min, max) = positions.iter().fold((positions[0], positions[0]), |(min, max), &p| {
                (min.min(p), max.max(p))
            });
            object.transform = Transform::from_translation(position);
            object.transform.scale = if pane {
                object.transform.rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
                Vec3::new(size.x / (max.x - min.x), 1.0, size.y / (max.z - min.z))
            } else {
                size / (max - min)
            };
            doc.material_draft = document::MaterialDraft {
                color: if pane { glass } else { wood },
                name: if pane { "Стекло" } else { "Дерево" }.into(),
                roughness: if pane { 0.1 } else { 0.65 },
                metallic: 0.0,
            };
            doc.amount = 0.04;
        });
        send(&mut app, EditAction::Mode(Mode::Edit));
        if !pane {
            send(&mut app, EditAction::Element(Element::Edge));
            send(&mut app, EditAction::SelectAll);
            send(&mut app, EditAction::BeginBevel { vertices: false });
            for (code, digit) in [
                (KeyCode::Digit0, "0"),
                (KeyCode::Period, "."),
                (KeyCode::Digit0, "0"),
                (KeyCode::Digit4, "4"),
            ] {
                key(&mut app, window, code, Key::Character(digit.into()));
            }
            send(&mut app, EditAction::Bevel(BevelControl::Segments));
            key(&mut app, window, KeyCode::Digit2, Key::Character("2".into()));
            send(&mut app, EditAction::Bevel(BevelControl::Confirm));
            edit_document(&mut app, |doc| {
                assert_eq!(doc.active().unwrap().geometry.faces.len(), 86);
            });
        }
        send(&mut app, EditAction::Element(Element::Face));
        send(&mut app, EditAction::SelectAll);
        send(&mut app, EditAction::CreateMaterial);
        send(&mut app, EditAction::Mode(Mode::Object));
    }
    send(&mut app, EditAction::SelectObject(first_beam.unwrap()));
    send(&mut app, EditAction::Mode(Mode::Edit));
    send(&mut app, EditAction::Element(Element::Edge));
    edit_document(&mut app, |doc| {
        let edges = doc.active().unwrap().geometry.edges();
        doc.selection.edges = [*edges.keys().next().unwrap()].into();
    });
    send(&mut app, EditAction::Operate(Operation::MarkUvSeam));
    send(&mut app, EditAction::Mode(Mode::Uv));
    let revision = app.world().resource::<State>().documents[&0].revision;
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Uv(uv::UvAction::Unwrap)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while std::time::Instant::now() < deadline {
        frame(&mut app);
        if app.world().resource::<State>().documents[&0].revision != revision {
            break;
        }
    }
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_ne!(doc.revision, revision, "The unwrap must apply a real worker result");
        assert_eq!(doc.active().unwrap().geometry.uv_seams.len(), 1);
        let mesh = &doc.active().unwrap().geometry;
        let atlas = crate::editor::model_uv_mesh::Atlas::new(mesh);
        assert!(mesh.uv_seams.iter().all(|edge| atlas.seams.contains(edge)));
        assert_eq!(doc.objects.len(), 5);
        assert!(doc.objects.iter().filter(|o| o.name != "Стекло").all(|o| o.geometry.faces.len() >= 86));
    }
    send(&mut app, EditAction::Save);
    for _ in 0..240 {
        frame(&mut app);
        if !app.world().resource::<State>().documents[&0].dirty() {
            break;
        }
    }
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    let saved = document::glb::Glb::load(&path).unwrap();
    let gltf = bevy::gltf::gltf::Gltf::from_slice(saved.data()).unwrap();
    assert_eq!(gltf.default_scene().unwrap().nodes().filter(|node| node.mesh().is_some()).count(), 5);
    assert!(gltf.materials().any(|m| m.name() == Some("Стекло")
        && m.alpha_mode() == bevy::gltf::gltf::material::AlphaMode::Blend));
    app.world_mut().write_message(Action::Open(0));
    warm(&mut app, 12);
    app.world_mut().write_message(Action::Add(0));
    let mut loaded_in_scene = false;
    for _ in 0..180 {
        frame(&mut app);
        let world = app.world_mut();
        let handles: Vec<_> = world.query::<(&MeshMaterial3d<StandardMaterial>, Option<&RenderLayers>, &InheritedVisibility)>()
            .iter(world)
            .filter(|(_, layers, visibility)| visibility.get() && layers.is_none_or(|layers| layers.intersects(&RenderLayers::layer(0))))
            .map(|(material, _, _)| material.0.clone())
            .collect();
        let materials = world.resource::<Assets<StandardMaterial>>();
        loaded_in_scene = handles.iter().any(|handle| {
            materials.get(handle).is_some_and(|m| {
                m.alpha_mode == AlphaMode::Blend
                    && (m.base_color.alpha() - glass.alpha).abs() < 0.001
            })
        });
        if loaded_in_scene {
            break;
        }
    }
    assert!(loaded_in_scene, "The scene must render the saved glass material");
}

fn scene_workspace_interactions(app: &mut App, window: Entity) {
    use crate::editor::viewport::{
        TransformHandle as Handle,
        transform_handles::{HandleMaterial, ModelOverlay},
    };
    {
        let world = app.world_mut();
        let ambient = world
            .query_filtered::<&AmbientLight, With<ViewCamera>>()
            .single(world)
            .unwrap();
        assert_eq!(ambient.brightness, 0.0);
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
            .find(|(_, label, _)| label.0 == "Экструдировать вершины, рёбра и грани · E")
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
            .is_none()
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
            if node.is_empty() {
                continue;
            }
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
    let (center, handle) = {
        let state = app.world().resource::<State>();
        assert!(state.extrude_tool && state.gesture.is_none());
        let doc = &state.documents[&0];
        assert_eq!(doc.active().unwrap().geometry.faces.len(), 6);
        let viewer = app.world().resource::<Viewer>();
        let (_, center, handle) = input::extrusion_handle(doc, viewer, rect).unwrap();
        (center, handle)
    };
    move_pointer(&mut app, window, handle);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    move_pointer(&mut app, window, handle);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, handle + (handle - center) * 0.5);
    mouse(&mut app, window, ButtonState::Released);
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
    // E starts the modal gesture without a handle press. Escape restores the mesh.
    move_pointer(&mut app, window, rect.center());
    key(&mut app, window, KeyCode::KeyE, Key::Character("e".into()));
    assert!(
        app.world()
            .resource::<State>()
            .gesture
            .as_ref()
            .is_some_and(input::Gesture::is_extruding)
    );
    move_pointer(&mut app, window, rect.center() + (handle - center) * 0.5);
    assert!(app.world().resource::<State>().documents[&0].dirty());
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    assert!(!app.world().resource::<State>().documents[&0].dirty());
    move_pointer(&mut app, window, rect.center());
    key(&mut app, window, KeyCode::KeyE, Key::Character("e".into()));
    move_pointer(&mut app, window, rect.center() + (handle - center) * 0.5);
    key(&mut app, window, KeyCode::Enter, Key::Enter);
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
    let active_x = |app: &App| {
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .transform
            .translation
            .x
    };
    let original_x = {
        let doc = &app.world().resource::<State>().documents[&0];
        doc.saved
            .iter()
            .find(|o| Some(o.id) == doc.active)
            .unwrap()
            .transform
            .translation
            .x
    };
    capture_when(
        &mut app,
        &target,
        &output.join("model-inspector.png"),
        |_| true,
    );
    click_control(&mut app, window, |action| {
        matches!(
            action,
            Action::ModelView(ModelViewAction::Edit(EditAction::RevertTransform(
                TransformProperty::Position
            )))
        )
    });
    assert_eq!(active_x(&app), original_x);
    send(&mut app, EditAction::Undo);
    assert_eq!(active_x(&app), 2.5);

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
        let (mesh, material) = world
            .query::<(&Mesh3d, &MeshMaterial3d<selection::FaceMaterial>)>()
            .single(world)
            .unwrap();
        assert!(world.resource::<Assets<Mesh>>().get(&mesh.0).unwrap().count_vertices() > 0);
        assert_eq!(
            world.resource::<Assets<selection::FaceMaterial>>().get(&material.0).unwrap().depth_bias(),
            f32::INFINITY,
        );
    }
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
    let first = capture_when(&mut app, &target, &output.join("02-xray-selection.png"), |_| true);
    warm(&mut app, 8);
    let last = capture_when(&mut app, &target, &output.join("02-xray-selection-later.png"), |_| true);
    assert!(differences(&first, &last) < 20, "Stationary X-Ray edges must remain stable");
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
            let assets = world.resource::<Assets<StandardMaterial>>();
            let original = assets.get(&source.0).unwrap();
            let restored = assets.get(&displayed.0).unwrap();
            assert!(restored.unlit);
            assert!(restored.double_sided);
            assert!(restored.cull_mode.is_none());
            assert_eq!(restored.base_color, original.base_color);
            assert_eq!(restored.alpha_mode, original.alpha_mode);
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
#[ignore = "requires GPU; verifies the seven model component selection states"]
fn gpu_model_selection_feedback_identifies_faces_edges_and_vertices() {
    use crate::editor::model_mesh::ComponentHit;
    use bevy::mesh::VertexAttributeValues;

    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    ).unwrap();
    let (mut app, window, target) = fixture_project(project);
    let scene = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::ViewModel(0));
    model_ready(&mut app, 0);
    app.world_mut().write_message(Action::ModelView(ModelViewAction::Shading(
        shading::ShadingAction::Set(shading::Mode::Solid),
    )));
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.center = Vec3::Y * 0.5;
        viewer.distance = 3.6;
        viewer.yaw = 0.55;
        viewer.pitch = 0.48;
    }
    send(&mut app, EditAction::Mode(Mode::Edit));
    let rect = canvas_rect(&mut app);
    let (objects, vertices, edges, faces) = {
        let state = app.world().resource::<State>();
        let doc = &state.documents[&0];
        let object = doc.active().unwrap();
        let viewer = app.world().resource::<Viewer>();
        let bvh = input::Bvh::build(doc);
        let point = |p| {
            let world = object.transform.transform_point(p);
            input::visible(&bvh, viewer, rect, world)
                .then(|| input::project(viewer, rect, world)).flatten()
        };
        let mesh = &object.geometry;
        let vertices: Vec<_> = mesh.positions.iter().enumerate().filter_map(|(i, &p)| {
            Some((ComponentHit::Vertex(i), point(p)?))
        }).take(2).collect();
        let edges: Vec<_> = mesh.edges().into_keys().filter_map(|e| {
            Some((ComponentHit::Edge(e), point((mesh.positions[e.0] + mesh.positions[e.1]) * 0.5)?))
        }).take(2).collect();
        let faces: Vec<_> = mesh.faces.iter().enumerate().filter_map(|(i, face)| {
            let center = object.transform.transform_point(mesh.center(face));
            let cursor = input::project(viewer, rect, center)?;
            let (origin, direction) = input::ray(viewer, rect, cursor);
            let (_, hit, _) = bvh.cast(origin, direction, Some(object.id))?;
            (hit == i).then_some((ComponentHit::Face(i), cursor))
        }).take(2).collect();
        (doc.objects.clone(), vertices, edges, faces)
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/model-selection-qa");
    std::fs::create_dir_all(&output).unwrap();
    let colors = |app: &mut App| {
        let world = app.world_mut();
        let mesh = world.query_filtered::<&Mesh3d, With<MeshMaterial3d<selection::FaceMaterial>>>()
            .single(world).unwrap();
        let assets = world.resource::<Assets<Mesh>>();
        let VertexAttributeValues::Float32x4(colors) = assets.get(mesh).unwrap()
            .attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
        else {
            panic!("Selection feedback requires per-vertex colors");
        };
        colors.clone()
    };
    let capture = |app: &mut App, file: &str| {
        move_pointer(app, window, Vec2::new(20., 110.));
        capture_when(app, &target, &output.join(file), |_| true);
    };
    send(&mut app, EditAction::SelectAll);
    let all = colors(&mut app);
    assert_eq!(all.iter().filter(|c| c[3] == 0.0).count(), 36);
    assert_eq!(all.iter().filter(|c| c[3] == 0.5).count(), 48);
    assert!(all.iter().all(|c| c[..3] != [1.0; 3]));
    capture(&mut app, "01-all.png");

    for (kind, hits, tag, one, two) in [
        (Element::Vertex, vertices, 0.5, "02-vertex.png", "03-two-vertices.png"),
        (Element::Edge, edges, 1.0, "04-edge.png", "05-two-edges.png"),
        (Element::Face, faces, 1.0, "06-face.png", "07-two-faces.png"),
    ] {
        assert_eq!(hits.len(), 2);
        send(&mut app, EditAction::Element(kind));
        send(&mut app, EditAction::Deselect);
        for (index, (hit, point)) in hits.into_iter().enumerate() {
            if index == 1 {
                app.world_mut().write_message(KeyboardInput {
                    key_code: KeyCode::ShiftLeft,
                    logical_key: Key::Shift,
                    state: ButtonState::Pressed,
                    text: None,
                    repeat: false,
                    window,
                });
                warm(&mut app, 2);
            }
            move_pointer(&mut app, window, point);
            mouse(&mut app, window, ButtonState::Pressed);
            mouse(&mut app, window, ButtonState::Released);
            if index == 1 {
                app.world_mut().write_message(KeyboardInput {
                    key_code: KeyCode::ShiftLeft,
                    logical_key: Key::Shift,
                    state: ButtonState::Released,
                    text: None,
                    repeat: false,
                    window,
                });
            }
            warm(&mut app, 3);
            let state = app.world().resource::<State>();
            assert_eq!(state.documents[&0].selection.active(), Some(hit));
            assert!(state.selection_label(Some(0)).unwrap().contains(&format!("выбрано {} из", index + 1)));
            let colors = colors(&mut app);
            assert!(colors.iter().any(|c| c[3] == tag && c[..3] == [1.0; 3]));
            if index == 1 {
                assert!(colors.iter().any(|c| c[3] == tag && c[0] == 1.0 && c[1] < 0.5));
            }
            if kind == Element::Face {
                assert_eq!(colors.iter().filter(|c| c[3] == 0.0).count(), (index + 1) * 6);
            }
            capture(&mut app, if index == 0 { one } else { two });
        }
    }
    let state = app.world().resource::<State>();
    assert_eq!(state.documents[&0].objects, objects);
    assert!(!state.documents[&0].dirty());
    assert!(state.documents[&0].undo.is_empty());
    assert_eq!(app.world().resource::<Editor>().edits, scene);
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
    send(&mut app, EditAction::Element(Element::Vertex));
    send(&mut app, EditAction::Deselect);
    click_control(&mut app, window, |action| {
        matches!(
            action,
            Action::ModelView(ModelViewAction::Edit(EditAction::ExtrudeTool))
        )
    });
    let vertex_position = {
        let state = app.world().resource::<State>();
        assert!(state.extrude_tool && state.gesture.is_none());
        let viewer = app.world().resource::<Viewer>();
        let object = state.documents[&0].active().unwrap();
        object
            .geometry
            .positions
            .iter()
            .find_map(|&local| {
                let world = object.transform.transform_point(local);
                let screen = input::project(viewer, rect, world)?;
                (rect.contains(screen) && input::visible(&state.bvh, viewer, rect, world))
                    .then_some(screen)
            })
            .unwrap()
    };
    move_pointer(&mut app, window, vertex_position);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<State>().documents[&0].selection.vertices.len(),
        1
    );
    let handle = |app: &App| {
        let state = app.world().resource::<State>();
        let viewer = app.world().resource::<Viewer>();
        let (_, center, handle) =
            input::extrusion_handle(&state.documents[&0], viewer, rect).unwrap();
        (center, handle)
    };
    let (center, plus) = handle(&app);
    let delta = (plus - center).perp() * 0.5;
    move_pointer(&mut app, window, plus);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, plus + delta);
    mouse(&mut app, window, ButtonState::Released);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        assert_eq!(doc.active().unwrap().geometry.loose_edges.len(), 1);
        let viewer = app.world().resource::<Viewer>();
        let new_center = input::project(viewer, rect, doc.selection_center().unwrap()).unwrap();
        assert!(new_center.distance(center + delta) < 1.0);
    }
    send(&mut app, EditAction::Undo);
    let (center, plus) = handle(&app);
    move_pointer(&mut app, window, rect.center());
    key(&mut app, window, KeyCode::KeyE, Key::Character("e".into()));
    // E moves a point with the cursor even perpendicular to the handle.
    let delta = (plus - center).perp() * 0.5;
    move_pointer(&mut app, window, rect.center() + delta);
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    let edge = {
        let state = app.world().resource::<State>();
        let doc = &state.documents[&0];
        let mesh = &doc.active().unwrap().geometry;
        assert_eq!(mesh.faces.len(), 6);
        assert_eq!(mesh.loose_edges.len(), 1);
        let viewer = app.world().resource::<Viewer>();
        let new_center = input::project(viewer, rect, doc.selection_center().unwrap()).unwrap();
        assert!(new_center.distance(center + delta) < 1.0);
        *mesh.loose_edges.first().unwrap()
    };
    send(&mut app, EditAction::Element(Element::Edge));
    app.world_mut()
        .resource_mut::<State>()
        .documents
        .get_mut(&0)
        .unwrap()
        .selection
        .edges
        .insert(edge);
    let (center, plus) = handle(&app);
    move_pointer(&mut app, window, plus);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, plus + (plus - center) * 0.5);
    mouse(&mut app, window, ButtonState::Released);
    {
        let state = app.world().resource::<State>();
        let mesh = &state.documents[&0].active().unwrap().geometry;
        assert_eq!(mesh.faces.len(), 7);
        assert!(mesh.loose_edges.is_empty());
    }
    send(&mut app, EditAction::Undo);
    assert_eq!(
        app.world().resource::<State>().documents[&0]
            .active()
            .unwrap()
            .geometry
            .loose_edges
            .len(),
        1
    );
    send(&mut app, EditAction::Undo);
    send(&mut app, EditAction::Element(Element::Edge));
    let edge_position = {
        let state = app.world().resource::<State>();
        assert!(state.extrude_tool);
        assert_eq!(state.tool, Tool::Move);
        let viewer = app.world().resource::<Viewer>();
        let object = state.documents[&0].active().unwrap();
        object
            .geometry
            .edges()
            .into_keys()
            .find_map(|edge| {
                let local = (object.geometry.positions[edge.0]
                    + object.geometry.positions[edge.1])
                    * 0.5;
                let world = object.transform.transform_point(local);
                let screen = input::project(viewer, rect, world)?;
                (rect.contains(screen) && input::visible(&state.bvh, viewer, rect, world))
                    .then_some(screen)
            })
            .unwrap()
    };
    move_pointer(&mut app, window, edge_position);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<State>().documents[&0].selection.edges.len(),
        1
    );
    let (pivot, normal, tangent) = {
        let doc = &app.world().resource::<State>().documents[&0];
        let object = doc.active().unwrap();
        let mesh = &object.geometry;
        let edge = doc.selection.edges.first().unwrap();
        let edges = mesh.extrusion_edges(&doc.selection);
        let local = mesh.extrusion_direction(&doc.selection, &edges).unwrap();
        let transform = object.transform.to_matrix();
        let normal = transform.transform_vector3(local).normalize();
        let tangent = transform
            .transform_vector3(mesh.positions[edge.1] - mesh.positions[edge.0])
            .normalize();
        (doc.selection_center().unwrap(), normal, tangent)
    };
    // Check both a depth-only extrusion and a strip where the normal would
    // appear parallel to the edge on screen.
    for view in [normal, (normal + tangent).normalize()] {
        {
            let mut viewer = app.world_mut().resource_mut::<Viewer>();
            viewer.center = pivot;
            viewer.yaw = view.x.atan2(view.z);
            viewer.pitch = view.y.asin();
            viewer.orthographic = true;
        }
        warm(&mut app, 5);
        let (center, plus) = handle(&app);
        let delta = (plus - center) * 0.5 + (plus - center).perp() * 0.4;
        move_pointer(&mut app, window, plus);
        mouse(&mut app, window, ButtonState::Pressed);
        move_pointer(&mut app, window, plus + delta);
        mouse(&mut app, window, ButtonState::Released);
        {
            let viewer = app.world().resource::<Viewer>();
            let doc = &app.world().resource::<State>().documents[&0];
            assert_eq!(doc.active().unwrap().geometry.faces.len(), 7);
            assert_eq!(doc.selection.edges.len(), 1);
            let new_center = input::project(viewer, rect, doc.selection_center().unwrap()).unwrap();
            assert!(new_center.distance(center + delta) < 1.0);
        }
        send(&mut app, EditAction::Undo);
        if view.dot(tangent).abs() < 0.001 {
            let edge_along = {
                let viewer = app.world().resource::<Viewer>();
                (input::project(viewer, rect, pivot + tangent).unwrap() - center).normalize()
            };
            move_pointer(&mut app, window, plus);
            mouse(&mut app, window, ButtonState::Pressed);
            move_pointer(&mut app, window, plus + edge_along * 24.0);
            mouse(&mut app, window, ButtonState::Released);
            {
                let state = app.world().resource::<State>();
                let doc = &state.documents[&0];
                let viewer = app.world().resource::<Viewer>();
                assert_eq!(doc.active().unwrap().geometry.faces.len(), 7);
                let moved = input::project(viewer, rect, doc.selection_center().unwrap()).unwrap();
                assert!(edge_along.perp_dot(moved - center).abs() > 10.0);
            }
            send(&mut app, EditAction::Undo);
        }
        move_pointer(&mut app, window, rect.center());
        key(&mut app, window, KeyCode::KeyE, Key::Character("e".into()));
        assert!(app.world().resource::<State>().gesture.as_ref().unwrap().is_extruding());
        let edge_along = {
            let viewer = app.world().resource::<Viewer>();
            (input::project(viewer, rect, pivot + tangent).unwrap() - center).normalize()
        };
        if view.dot(tangent).abs() < 0.001 {
            move_pointer(&mut app, window, rect.center() + edge_along * 24.0);
            let state = app.world().resource::<State>();
            let doc = &state.documents[&0];
            assert_eq!(doc.active().unwrap().geometry.faces.len(), 7);
            assert!(state.gesture.as_ref().unwrap().is_extruding());
            let viewer = app.world().resource::<Viewer>();
            let moved = input::project(viewer, rect, doc.selection_center().unwrap()).unwrap();
            assert!(edge_along.perp_dot(moved - center).abs() > 10.0);
        }
        move_pointer(&mut app, window, rect.center() + delta);
        key(&mut app, window, KeyCode::Enter, Key::Enter);
        {
            let viewer = app.world().resource::<Viewer>();
            let doc = &app.world().resource::<State>().documents[&0];
            assert_eq!(doc.active().unwrap().geometry.faces.len(), 7);
            assert_eq!(doc.selection.edges.len(), 1);
            let new_center = input::project(viewer, rect, doc.selection_center().unwrap()).unwrap();
            assert!(new_center.distance(center + delta) < 1.0);
            assert!(edge_along.perp_dot(new_center - center).abs() > 10.0);
        }
        send(&mut app, EditAction::Undo);
    }
    {
        let mut viewer = app.world_mut().resource_mut::<Viewer>();
        viewer.yaw = 0.65;
        viewer.pitch = 0.45;
    }
    warm(&mut app, 5);
    for (index, (axis, code, label)) in [
        (Vec3::X, KeyCode::KeyX, "x"),
        (Vec3::Y, KeyCode::KeyY, "y"),
        (Vec3::Z, KeyCode::KeyZ, "z"),
    ]
    .into_iter()
    .enumerate()
    {
        // A strip extruded parallel to its source edge has zero area.
        if axis.dot(tangent).abs() > 0.99 {
            continue;
        }
        let direction = {
            let viewer = app.world().resource::<Viewer>();
            (input::project(viewer, rect, pivot + axis).unwrap()
                - input::project(viewer, rect, pivot).unwrap())
            .normalize()
        };
        move_pointer(&mut app, window, rect.center());
        key(&mut app, window, KeyCode::KeyE, Key::Character("e".into()));
        key(&mut app, window, code, Key::Character(label.into()));
        let state = app.world().resource::<State>();
        assert_eq!(state.gesture.as_ref().unwrap().axis, Some(index));
        move_pointer(&mut app, window, rect.center() + direction * 30.0 + direction.perp() * 12.0);
        key(&mut app, window, KeyCode::Enter, Key::Enter);
        let doc = &app.world().resource::<State>().documents[&0];
        let moved = doc.selection_center().unwrap() - pivot;
        assert!(moved.dot(axis) > 0.001);
        assert!(moved.cross(axis).length() < 0.0001);
        send(&mut app, EditAction::Undo);
    }
    send(&mut app, EditAction::Element(Element::Vertex));
    warm(&mut app, 3);
    let (center, plus) = handle(&app);
    move_pointer(&mut app, window, plus);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, plus + (plus - center) * 0.5);
    mouse(&mut app, window, ButtonState::Released);
    {
        let doc = &app.world().resource::<State>().documents[&0];
        let mesh = &doc.active().unwrap().geometry;
        assert_eq!(mesh.faces.len(), 7);
        assert_eq!(doc.selection.kind, Element::Vertex);
        assert_eq!(doc.selection.vertices.len(), 2);
        assert!(mesh.loose_edges.is_empty());
    }
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}
