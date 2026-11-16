use super::*;
use crate::editor::{
    Action,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
};
use bevy::input::{
    ButtonState,
    keyboard::{Key, KeyboardInput},
};
use std::path::PathBuf;

fn warm(app: &mut App, n: usize) {
    for _ in 0..n {
        frame(app);
    }
}
fn modifier(app: &mut App, window: Entity, code: KeyCode, down: bool) {
    app.world_mut().write_message(KeyboardInput {
        key_code: code,
        logical_key: if code == KeyCode::ControlLeft {
            Key::Control
        } else {
            Key::Shift
        },
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
}
fn drag(app: &mut App, window: Entity, rect: Rect) {
    move_pointer(app, window, rect.min);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, rect.max);
    mouse(app, window, ButtonState::Released);
    warm(app, 2);
}
fn bounds(app: &mut App, xs: &[f32]) -> Rect {
    let world = app.world_mut();
    let (camera, view) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<EditorCamera>>()
        .single(world)
        .unwrap();
    let points = xs.iter().flat_map(|x| {
        (0..8).map(move |i| {
            Vec3::new(
                x + if i & 1 == 0 { -0.5 } else { 0.5 },
                if i & 2 == 0 { 0.0 } else { 1.0 },
                if i & 4 == 0 { -0.5 } else { 0.5 },
            )
        })
    });
    let r = crate::editor::area_selection::projected_bounds(points, |p| {
        camera
            .world_to_viewport(view, p)
            .ok()
            .map(|p| p + Vec2::Y * ui::TOP)
    })
    .unwrap();
    Rect::from_corners(r.min - Vec2::splat(16.0), r.max + Vec2::splat(16.0))
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires GPU; verifies scene rectangle selection, modifiers, grouped transforms, undo and cancellation"]
fn gpu_scene_cursor_area_selects_and_moves_a_group() {
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "bevy-scene-area-{}",
        bevy::asset::uuid::Uuid::new_v4()
    )));
    std::fs::create_dir_all(scratch.0.join("models")).unwrap();
    std::fs::create_dir_all(scratch.0.join("scenes")).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/assets/models/block.glb"),
        scratch.0.join("models/block.glb"),
    )
    .unwrap();
    let pose = Transform::from_xyz(0.0, 3.5, 8.0).looking_at(Vec3::Y * 0.5, Vec3::Y);
    let scene = serde_json::json!({"scene":{"label":"Area QA","cameras":[{"id":"main","label":"Camera","placement":crate::CameraPlacement{pose:pose.into(),lens:58.0}}]},
        "models":[{"id":1,"asset":"models/block.glb","pose":crate::Pose::from(Transform::from_xyz(-1.4,0.0,0.0))},
                  {"id":2,"asset":"models/block.glb","pose":crate::Pose::from(Transform::from_xyz(1.4,0.0,0.0))}],
        "lighting":crate::SceneLighting::editor_default()});
    std::fs::write(
        scratch.0.join("scenes/main.scene"),
        serde_json::to_vec(&scene).unwrap(),
    )
    .unwrap();
    let manifest = scratch.0.join("project.bevyeditor");
    std::fs::write(&manifest,serde_json::to_vec(&serde_json::json!({"title":"Area QA","asset_root":".","scenes":[{"file":"scenes/main.scene"}],"catalog":[{"label":"Cube","asset":"models/block.glb","category":"Models","solid":true}]})).unwrap()).unwrap();
    let (mut app, window, target) = fixture_project(crate::EditorProject::load(manifest).unwrap());
    warm(&mut app, 15);
    let original = app.world().resource::<Editor>().edits.clone();
    let area = bounds(&mut app, &[-1.4, 1.4]);
    move_pointer(&mut app, window, area.min);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, area.max);
    assert!(
        app.world()
            .resource::<PointerState>()
            .selection_rect()
            .is_some()
    );
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/area-selection-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("01-scene-drag.png"), |_| {
        true
    });
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 3);
    let selected = app.world().resource::<Editor>().selected_targets();
    assert_eq!(selected, vec![Selection::Model(1), Selection::Model(2)]);
    assert_eq!(app.world().resource::<Editor>().edits, original);
    capture_when(
        &mut app,
        &target,
        &output.join("02-scene-group.png"),
        |_| true,
    );

    // A drag starting on the old gizmo center must begin rectangle selection.
    let center = {
        let world = app.world_mut();
        let editor = world.resource::<Editor>();
        let pivot = editor
            .pose(editor.selection.unwrap())
            .unwrap()
            .transform()
            .translation;
        let (camera, view) = world
            .query_filtered::<(&Camera, &GlobalTransform), With<EditorCamera>>()
            .single(world)
            .unwrap();
        camera.world_to_viewport(view, pivot).unwrap() + Vec2::Y * ui::TOP
    };
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    assert!(app.world().resource::<PointerState>().drag.is_none());
    assert!(app.world().resource::<PointerState>().selecting());
    move_pointer(&mut app, window, area.min);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<Editor>().edits, original);
    assert!(app.world().resource::<Editor>().undo.is_empty());
    app.world_mut().resource_mut::<Editor>().select_targets(
        vec![Selection::Model(1), Selection::Model(2)],
        crate::editor::area_selection::Mode::Replace,
    );

    app.world_mut().write_message(Action::Tool(Tool::Move));
    warm(&mut app, 3);
    let (start, end) = {
        let world = app.world_mut();
        let editor = world.resource::<Editor>();
        let pivot = editor
            .pose(Selection::Model(2))
            .unwrap()
            .transform()
            .translation;
        let view = editor.view;
        let (camera, _) = world
            .query_filtered::<(&Camera, &GlobalTransform), With<EditorCamera>>()
            .single(world)
            .unwrap();
        let global = GlobalTransform::from(view);
        let at = |p| camera.world_to_viewport(&global, p).unwrap() + Vec2::Y * ui::TOP;
        let start = at(pivot + Vec3::X * length(camera, &view, pivot) * 0.7);
        (start, start + at(pivot + Vec3::X) - at(pivot))
    };
    move_pointer(&mut app, window, start);
    mouse(&mut app, window, ButtonState::Pressed);
    assert!(
        app.world().resource::<PointerState>().drag.is_some(),
        "The group gizmo must be draggable"
    );
    move_pointer(&mut app, window, end);
    mouse(&mut app, window, ButtonState::Released);
    for (a, b) in app
        .world()
        .resource::<Editor>()
        .edits
        .models
        .iter()
        .zip(&original.models)
    {
        assert!((a.pose.position[0] - b.pose.position[0] - 1.0).abs() < 0.01);
    }
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 3);
    assert_eq!(app.world().resource::<Editor>().edits, original);
    app.world_mut().write_message(Action::Tool(Tool::Universal));
    warm(&mut app, 3);
    let left = bounds(&mut app, &[-1.4]);
    modifier(&mut app, window, KeyCode::ControlLeft, true);
    drag(&mut app, window, left);
    modifier(&mut app, window, KeyCode::ControlLeft, false);
    assert_eq!(
        app.world().resource::<Editor>().selected_targets(),
        vec![Selection::Model(2)]
    );
    modifier(&mut app, window, KeyCode::ShiftLeft, true);
    drag(&mut app, window, left);
    modifier(&mut app, window, KeyCode::ShiftLeft, false);
    assert_eq!(app.world().resource::<Editor>().selected_targets().len(), 2);
    app.world_mut()
        .resource_mut::<Editor>()
        .select(Some(Selection::Model(1)));
    move_pointer(&mut app, window, area.min);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, area.max);
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Escape,
        logical_key: Key::Escape,
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window,
    });
    warm(&mut app, 2);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<Editor>().selected_targets(),
        vec![Selection::Model(1)]
    );
    assert!(!app.world().resource::<PointerState>().selecting());
    assert_eq!(app.world().resource::<Editor>().edits, original);
    eprintln!("Area selection screenshots: {}", output.display());
}
