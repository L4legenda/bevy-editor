use super::*;
#[path = "asset_drop_tests.rs"]
mod asset_drop;
#[path = "confirmation_tests.rs"]
mod confirmation;
#[path = "scene_document_tests.rs"]
mod documents;
#[path = "lighting_render_tests.rs"]
mod lighting_render;
#[path = "lighting_ui_render_tests.rs"]
pub(super) mod lighting_ui_render;
use bevy::{
    camera::{RenderTarget, ScalingMode},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    input_focus::InputFocus,
    text::EditableText,
    time::TimeUpdateStrategy,
    ui_widgets::TextInput,
    world_serialization::WorldInstanceSpawner,
};

fn app() -> App {
    let project = EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(Editor::new(project, RoomEdits::default()))
        .init_resource::<Scene>()
        .init_resource::<viewport::PointerState>()
        .add_message::<Action>()
        .add_systems(Update, handle_actions);
    app
}

fn action(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    app.update();
}

#[test]
fn file_manager_starts_closed_and_toggles_without_editing_the_scene() {
    let mut app = app();
    assert!(!app.world().resource::<Editor>().drawer);
    action(&mut app, Action::Drawer);
    assert!(app.world().resource::<Editor>().drawer);
    assert!(!app.world().resource::<Editor>().dirty());
    action(&mut app, Action::Drawer);
    assert!(!app.world().resource::<Editor>().drawer);
}

fn focused_search_app() -> App {
    let mut app = app();
    app.world_mut().resource_mut::<Editor>().ready = true;
    action(&mut app, Action::Add(0));
    let input = app
        .world_mut()
        .spawn((TextInput, EditableText::new("wooden shelf")))
        .id();
    app.insert_resource(InputFocus::from_entity(input))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_systems(Update, ui::shortcuts.before(handle_actions));
    app.world_mut().resource_mut::<Editor>().drawer = true;
    app
}

#[test]
fn typing_in_asset_search_does_not_trigger_scene_shortcuts() {
    let mut app = focused_search_app();
    for key in [
        KeyCode::KeyW,
        KeyCode::KeyE,
        KeyCode::KeyR,
        KeyCode::KeyF,
        KeyCode::Delete,
    ] {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }
    app.update();
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.tool, Tool::Move);
    assert_eq!(editor.edits.models.len(), 1);
    assert!(!editor.focus);
    assert!(editor.drawer);
}

#[test]
fn escape_and_control_space_close_the_drawer_while_search_is_focused() {
    for keys in [
        vec![KeyCode::Escape],
        vec![KeyCode::ControlLeft, KeyCode::Space],
    ] {
        let mut app = focused_search_app();
        for key in keys {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        app.update();
        assert!(!app.world().resource::<Editor>().drawer);
        assert!(app.world().resource::<InputFocus>().get().is_none());
        assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    }
}

#[test]
fn keyboard_history_and_save_track_the_saved_scene_state() {
    let (mut app, scratch) = documents::workspace();
    app.init_resource::<InputFocus>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_systems(Update, ui::shortcuts.before(handle_actions));
    documents::lens(&mut app, 60.0);
    assert!(app.world().resource::<Editor>().document_dirty(0));

    for (key, lens, dirty) in [
        (KeyCode::KeyZ, 45.0, false),
        (KeyCode::KeyY, 60.0, true),
        (KeyCode::KeyS, 60.0, false),
        (KeyCode::KeyZ, 45.0, true),
        (KeyCode::KeyY, 60.0, false),
    ] {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(KeyCode::ControlLeft);
        keys.press(key);
        app.update();
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.camera(0).lens, lens, "{key:?}");
        assert_eq!(editor.document_dirty(0), dirty, "{key:?}");
    }
    let editor = app.world().resource::<Editor>();
    let saved: RoomEdits = RoomEdits::read(&scratch.0, &editor.definition().file).unwrap();
    assert_eq!(saved.cameras["main"].lens, 60.0);
}

#[test]
fn save_shortcut_works_while_asset_search_has_focus() {
    let mut app = focused_search_app();
    let scratch = CameraSavePath::new();
    let editor = app.world().resource::<Editor>();
    let model = &editor.edits.models[0].asset;
    let target = scratch.0.join(model);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::copy(editor.root.join(model), target).unwrap();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.root = scratch.0.clone();
        // This shortcut test omits the renderer; supply the bounds a loaded solid model has.
        editor.edits.models[0].bounds = Some(BoxBounds {
            center: [0.0; 3],
            half_size: [0.5; 3],
        });
    }
    assert!(app.world().resource::<Editor>().dirty());
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.press(KeyCode::ControlLeft);
    keys.press(KeyCode::KeyS);
    app.update();

    let editor = app.world().resource::<Editor>();
    assert!(!editor.dirty(), "{}", editor.status);
    let saved: RoomEdits = RoomEdits::read(&scratch.0, &editor.definition().file).unwrap();
    assert_eq!(saved.models.len(), 1);
}

fn record_placements(app: &mut App, names: &[&str]) {
    // Obtain an actual instance ID without constructing or running a renderer.
    let mut assets = Assets::<WorldAsset>::default();
    let handle = assets.add(WorldAsset::new(World::new()));
    app.insert_resource(assets);
    let instance_id = WorldInstanceSpawner::default()
        .spawn_sync(app.world_mut(), handle.id())
        .unwrap();
    let root = app
        .world_mut()
        .spawn(Transform::IDENTITY)
        .observe(record_room)
        .id();
    let group = app
        .world_mut()
        .spawn((Name::new("Props"), Transform::IDENTITY, ChildOf(root)))
        .id();
    for name in names {
        app.world_mut().spawn((
            Name::new((*name).to_owned()),
            Transform::IDENTITY,
            ChildOf(group),
        ));
    }
    app.world_mut().resource_mut::<Scene>().root = Some(root);
    app.world_mut().trigger(WorldInstanceReady {
        entity: root,
        instance_id,
    });
}

#[test]
fn switching_scenes_accepts_different_placement_counts_and_resets_selection() {
    let mut app = app();
    record_placements(&mut app, &["One", "Two", "Three"]);
    assert!(app.world().resource::<Editor>().ready);
    assert_eq!(app.world().resource::<Editor>().imported.len(), 3);
    action(&mut app, Action::Select(Selection::Imported(2)));
    action(&mut app, Action::Open(1));
    assert!(app.world().resource::<Editor>().selection.is_none());
    record_placements(&mut app, &["Only"]);
    let editor = app.world().resource::<Editor>();
    assert!(editor.ready);
    assert_eq!(editor.imported.len(), 1);
    assert_eq!(editor.room, 1);
}

#[test]
fn duplicate_names_fail_cleanly_instead_of_editing_multiple_objects() {
    let mut app = app();
    record_placements(&mut app, &["Same", "Same"]);
    assert!(!app.world().resource::<Editor>().ready);
}

#[test]
fn models_keep_shared_asset_paths_through_duplicate_undo_and_redo() {
    let mut app = app();
    app.world_mut().resource_mut::<Editor>().ready = true;
    action(&mut app, Action::Add(0));
    action(&mut app, Action::Duplicate);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits.models.len(), 2);
    assert_eq!(editor.edits.models[0].asset, editor.edits.models[1].asset);
    assert_ne!(editor.edits.models[0].id, editor.edits.models[1].id);
    action(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    action(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 2);
    action(&mut app, Action::Open(1));
    assert_eq!(app.world().resource::<Editor>().room, 1);
    assert!(app.world().resource::<Editor>().document_dirty(0));
    action(&mut app, Action::Open(0));
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 2);
}

#[test]
fn camera_keys_and_node_constraints_come_from_the_project() {
    let mut app = app();
    app.world_mut().resource_mut::<Editor>().project.scenes[0]
        .node_policies
        .insert(
            "Seat".into(),
            crate::NodePolicy {
                translation: [true, false, true],
                rotation: [false, true, false],
                scale: [false; 3],
                ..default()
            },
        );
    record_placements(&mut app, &["Seat"]);
    action(&mut app, Action::Select(Selection::Imported(0)));
    action(&mut app, Action::Number(Field::Position(1), 5.0));
    assert!(!app.world().resource::<Editor>().dirty());
    action(&mut app, Action::Select(Selection::Camera(0)));
    action(&mut app, Action::Number(Field::Lens, 60.0));
    assert_eq!(
        app.world().resource::<Editor>().edits.cameras["main"].lens,
        60.0
    );
}

fn camera_app() -> (App, Entity, Entity) {
    let mut app = app();
    let mut images = Assets::<Image>::default();
    let surface = viewport::Surface::new(&mut images);
    let camera = app
        .world_mut()
        .spawn((
            viewport::EditorCamera,
            Camera::default(),
            Transform::IDENTITY,
            Projection::default(),
            RenderTarget::Image(surface.0.clone().into()),
        ))
        .id();
    let mut window = Window {
        focused: true,
        resolution: (1280, 760).into(),
        ..default()
    };
    window.set_cursor_position(Some(Vec2::new(120.0, 180.0)));
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    app.insert_resource(images)
        .insert_resource(surface)
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(20),
        ))
        .init_resource::<ui::DrawerMotion>()
        .init_resource::<InputFocus>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<AccumulatedMouseScroll>()
        .add_systems(Update, ui::shortcuts.before(handle_actions))
        .add_systems(Update, viewport::navigate.after(handle_actions));
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.ready = true;
        editor.reload = false;
        editor.view = Transform::from_xyz(-4.0, 5.0, 9.0).looking_at(Vec3::ZERO, Vec3::Y);
    }
    app.update(); // Warm up the manually controlled time resource.
    action(&mut app, Action::Select(Selection::Camera(0)));
    action(&mut app, Action::Pilot);
    (app, window, camera)
}

fn camera_frame(app: &mut App, motion: Vec2) {
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = motion;
    app.update();
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::ZERO;
    app.world_mut()
        .resource_mut::<AccumulatedMouseScroll>()
        .delta = Vec2::ZERO;
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn begin_camera_flight(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    camera_frame(app, Vec2::new(8.0, -3.0));
}

#[test]
fn camera_flight_is_one_undoable_edit_and_preserves_the_free_view() {
    let (mut app, _, camera) = camera_app();
    let original = app.world().resource::<Editor>().camera(0);
    let free_view = app.world().resource::<Editor>().view;
    begin_camera_flight(&mut app);
    camera_frame(&mut app, Vec2::new(4.0, 2.0));
    let moved = app.world().resource::<Editor>().camera(0);
    assert_ne!(moved.pose.position, original.pose.position);
    assert_ne!(moved.pose.rotation, original.pose.rotation);
    assert_eq!(moved.lens, original.lens);
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        moved.pose.transform()
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Right);
    camera_frame(&mut app, Vec2::ZERO);
    assert_eq!(app.world().resource::<Editor>().undo.len(), 1);
    action(&mut app, Action::Pilot);
    assert_eq!(app.world().resource::<Editor>().view, free_view);
    assert_eq!(*app.world().get::<Transform>(camera).unwrap(), free_view);
    action(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().camera(0), original);
    assert!(!app.world().resource::<Editor>().dirty());
    action(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().camera(0), moved);
}

#[test]
fn losing_focus_or_exiting_the_camera_finishes_the_live_edit() {
    for lose_focus in [true, false] {
        let (mut app, window, _) = camera_app();
        begin_camera_flight(&mut app);
        let moved = app.world().resource::<Editor>().camera(0);
        if lose_focus {
            app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
            camera_frame(&mut app, Vec2::new(100.0, 100.0));
        } else {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            camera_frame(&mut app, Vec2::ZERO);
            assert!(!app.world().resource::<Editor>().pilot);
        }
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.camera(0), moved);
        assert_eq!(editor.undo.len(), 1);
        assert!(editor.camera_navigation.is_none());
    }
}

#[test]
fn switching_cameras_finishes_the_edit_and_requires_a_new_navigation_gesture() {
    let (mut app, _, _) = camera_app();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let mut second = editor.definition().cameras[0].clone();
        second.id = "detail".into();
        second.placement.pose.position[0] += 3.0;
        editor.project.scenes[0].cameras.push(second);
    }
    let second = app.world().resource::<Editor>().camera(1);
    begin_camera_flight(&mut app);
    let first = app.world().resource::<Editor>().camera(0);
    action(&mut app, Action::Select(Selection::Camera(1)));
    camera_frame(&mut app, Vec2::new(20.0, 20.0));
    let editor = app.world().resource::<Editor>();
    assert!(editor.pilot);
    assert_eq!(editor.camera(0), first);
    assert_eq!(editor.camera(1), second);
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.camera_navigation.is_none());
}

#[test]
fn undo_during_camera_flight_includes_the_current_gesture() {
    let (mut app, _, _) = camera_app();
    let original = app.world().resource::<Editor>().camera(0);
    begin_camera_flight(&mut app);
    let moved = app.world().resource::<Editor>().camera(0);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyZ);
    }
    camera_frame(&mut app, Vec2::ZERO);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0), original);
    assert!(editor.undo.is_empty());
    assert_eq!(editor.redo.len(), 1);
    assert!(editor.camera_navigation.is_none());
    // Held movement keys must not immediately overwrite the undo while Ctrl is down.
    camera_frame(&mut app, Vec2::ZERO);
    assert_eq!(app.world().resource::<Editor>().camera(0), original);
    action(&mut app, Action::Redo);
    assert_eq!(app.world().resource::<Editor>().camera(0), moved);
}

struct CameraSavePath(PathBuf);

impl CameraSavePath {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "bevy-editor-camera-save-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
    }
}

impl Drop for CameraSavePath {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn saving_during_flight_writes_the_current_camera_and_finishes_its_edit() {
    let (mut app, _, _) = camera_app();
    let scratch = CameraSavePath::new();
    app.world_mut().resource_mut::<Editor>().root = scratch.0.clone();
    begin_camera_flight(&mut app);
    let moved = app.world().resource::<Editor>().camera(0);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyS);
    }
    camera_frame(&mut app, Vec2::ZERO);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.camera(0), moved);
    assert!(!editor.dirty(), "{}", editor.status);
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.camera_navigation.is_none());
    let saved: RoomEdits = RoomEdits::read(&scratch.0, &editor.definition().file).unwrap();
    assert_eq!(saved.cameras["main"], moved);
}

#[test]
fn orthographic_camera_scroll_changes_framing_and_is_undoable() {
    let (mut app, _, camera) = camera_app();
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.project.scenes[0].cameras[0].projection = crate::project::LensKind::Orthographic;
        editor.project.scenes[0].cameras[0].placement.lens = 8.0;
    }
    let original = app.world().resource::<Editor>().camera(0);
    app.world_mut()
        .resource_mut::<AccumulatedMouseScroll>()
        .delta = Vec2::Y;
    camera_frame(&mut app, Vec2::ZERO);
    let edited = app.world().resource::<Editor>().camera(0);
    assert!(edited.lens < original.lens);
    assert_eq!(edited.pose, original.pose);
    let Projection::Orthographic(projection) = app.world().get::<Projection>(camera).unwrap()
    else {
        panic!("pilot view must use the selected camera's orthographic projection");
    };
    let ScalingMode::FixedVertical { viewport_height } = projection.scaling_mode else {
        panic!("orthographic lens must describe the frame's vertical height");
    };
    assert_eq!(viewport_height, edited.lens);
    action(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().camera(0), original);

    for (scroll, expected) in [(1000.0, 0.05), (-1000.0, 20.0)] {
        app.world_mut()
            .resource_mut::<AccumulatedMouseScroll>()
            .delta = Vec2::new(0.0, scroll);
        camera_frame(&mut app, Vec2::ZERO);
        assert_eq!(app.world().resource::<Editor>().camera(0).lens, expected);
    }
}

#[test]
fn perspective_camera_scroll_dollies_without_changing_the_lens() {
    let (mut app, _, camera) = camera_app();
    let original = app.world().resource::<Editor>().camera(0);
    app.world_mut()
        .resource_mut::<AccumulatedMouseScroll>()
        .delta = Vec2::Y;
    camera_frame(&mut app, Vec2::ZERO);
    let edited = app.world().resource::<Editor>().camera(0);
    assert_ne!(edited.pose.position, original.pose.position);
    assert_eq!(edited.lens, original.lens);
    let Projection::Perspective(projection) = app.world().get::<Projection>(camera).unwrap() else {
        panic!("pilot view must use the selected camera's perspective projection");
    };
    assert!((projection.fov.to_degrees() - original.lens).abs() < 1e-4);
    action(&mut app, Action::Undo);
    assert_eq!(app.world().resource::<Editor>().camera(0), original);
}

#[test]
fn camera_frame_stays_centered_and_has_the_same_aspect_for_different_surfaces() {
    for size in [
        UVec2::new(964, 664),
        UVec2::new(1928, 1328),
        UVec2::new(400, 900),
        UVec2::new(2000, 500),
        UVec2::ONE,
    ] {
        let frame = viewport::frame_viewport(size);
        assert!(frame.physical_size.cmpge(UVec2::ONE).all());
        assert!(frame.physical_size.cmple(size).all());
        let occupied = frame.physical_position * 2 + frame.physical_size;
        assert!(size.saturating_sub(occupied).cmple(UVec2::ONE).all());
        let ratio = frame.physical_size.x as f32 / frame.physical_size.y as f32;
        let pixel_tolerance = viewport::CAMERA_ASPECT / frame.physical_size.y as f32;
        assert!((ratio - viewport::CAMERA_ASPECT).abs() <= pixel_tolerance);
    }
}

#[test]
fn focused_text_input_prevents_camera_motion_without_leaving_the_camera_view() {
    let (mut app, _, camera) = camera_app();
    let original = app.world().resource::<Editor>().camera(0);
    let input = app
        .world_mut()
        .spawn((TextInput, EditableText::new("12.0")))
        .id();
    app.insert_resource(InputFocus::from_entity(input));
    begin_camera_flight(&mut app);
    let editor = app.world().resource::<Editor>();
    assert!(editor.pilot);
    assert_eq!(editor.camera(0), original);
    assert!(editor.undo.is_empty());
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        original.pose.transform()
    );
}

#[test]
fn camera_control_ignores_numpad_zero_and_pilot_keys_do_not_change_tools() {
    let (mut app, _, _) = camera_app();
    app.world_mut().resource_mut::<Editor>().tool = Tool::Scale;
    for key in [KeyCode::KeyE, KeyCode::KeyW, KeyCode::KeyF] {
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.reset_all();
            keys.press(key);
        }
        camera_frame(&mut app, Vec2::ZERO);
        let editor = app.world().resource::<Editor>();
        assert!(editor.pilot);
        assert_eq!(editor.tool, Tool::Scale);
        assert!(!editor.focus);
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Numpad0);
    camera_frame(&mut app, Vec2::ZERO);
    assert!(app.world().resource::<Editor>().pilot);
    action(&mut app, Action::Pilot);
    assert!(!app.world().resource::<Editor>().pilot);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Numpad0);
    camera_frame(&mut app, Vec2::ZERO);
    assert!(!app.world().resource::<Editor>().pilot);
    action(&mut app, Action::Pilot);
    let editor = app.world().resource::<Editor>();
    assert!(editor.pilot);
    assert!(!editor.dirty());
    assert!(editor.undo.is_empty());
}
