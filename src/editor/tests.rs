use super::*;
use bevy::world_serialization::WorldInstanceSpawner;

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
    assert_eq!(
        app.world().resource::<Editor>().room,
        0,
        "unsaved edits prevent switching"
    );
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
