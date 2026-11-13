use super::*;
use crate::editor::browser::{Browser, Entry, FileKind};

fn workspace() -> (App, Browser) {
    let mut app = app();
    let browser = {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.ready = true;
        editor.reload = false;
        Browser::new(&mut editor.project)
    };
    (app, browser)
}

fn drop_at(app: &mut App, browser: &Browser, entry: Entry, point: Vec3) -> bool {
    let mut editor = app.world_mut().resource_mut::<Editor>();
    apply_asset_drop(&mut editor, browser, entry, point, None)
}

fn assert_rejected(app: &mut App, browser: &Browser, entry: Entry, point: Vec3) {
    let editor = app.world().resource::<Editor>();
    let edits = editor.edits.clone();
    let saved = editor.saved.clone();
    let undo = editor.undo.clone();
    let redo = editor.redo.clone();
    let room = editor.room;
    let tabs = editor.open_tabs.clone();
    let selection = editor.selection;
    assert!(
        !drop_at(app, browser, entry, point),
        "{entry:?} at {point:?}"
    );
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits, edits);
    assert_eq!(editor.saved, saved);
    assert_eq!(editor.undo, undo);
    assert_eq!(editor.redo, redo);
    assert_eq!(editor.room, room);
    assert_eq!(editor.open_tabs, tabs);
    assert_eq!(editor.selection, selection);
}

#[test]
fn dropping_a_model_places_one_instance_at_the_drop_point_with_one_undo_step() {
    let (mut app, browser) = workspace();
    let asset = app.world().resource::<Editor>().project.catalog[0]
        .asset
        .clone();
    let point = Vec3::new(2.25, 1.5, -3.75);
    assert!(drop_at(&mut app, &browser, Entry::Model(0), point));
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits.models.len(), 1);
    let placed = editor.edits.models[0].clone();
    assert_eq!(placed.asset, asset);
    assert_eq!(placed.pose.position, point.to_array());
    assert!(placed.bounds.is_none());
    assert_eq!(editor.selection, Some(Selection::Model(placed.id)));
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.undo[0].models.is_empty());
    assert!(editor.redo.is_empty());
    assert!(editor.document_dirty(0));

    action(&mut app, Action::Undo);
    let editor = app.world().resource::<Editor>();
    assert!(editor.edits.models.is_empty());
    assert!(editor.undo.is_empty());
    assert_eq!(editor.redo.len(), 1);
    assert!(!editor.document_dirty(0));

    action(&mut app, Action::Redo);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.edits.models, [placed]);
    assert_eq!(editor.undo.len(), 1);
    assert!(editor.redo.is_empty());
    assert!(editor.document_dirty(0));
}

#[test]
fn glbs_previously_used_as_sources_are_placeable_models() {
    for scene_card in [true, false] {
        let (mut app, browser) = workspace();
        let editor = app.world().resource::<Editor>();
        let source = editor.project.scenes[1].asset.clone();
        let catalog_count = editor.project.catalog.len();
        let index = editor
            .project
            .catalog
            .iter()
            .position(|model| model.asset == source)
            .unwrap();
        let _ = scene_card;
        let entry = Entry::Model(index);
        let point = Vec3::new(-1.0, 0.0, 2.5);
        assert!(drop_at(&mut app, &browser, entry, point));
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits.models.len(), 1);
        assert_eq!(editor.edits.models[0].asset, source);
        assert_eq!(editor.edits.models[0].pose.position, point.to_array());
        assert_eq!(editor.room, 0);
        assert_eq!(editor.open_tabs, [0]);
        assert!(editor.documents.is_empty());
        assert!(!editor.reload);
        assert_eq!(editor.project.catalog.len(), catalog_count);
        assert_eq!(editor.undo.len(), 1);
        assert!(editor.document_dirty(0));
        assert!(!editor.document_dirty(1));
    }
}

#[test]
fn folders_images_generic_non_models_and_stale_indices_do_not_change_the_document() {
    let (mut app, mut browser) = workspace();
    for entry in [
        Entry::Folder(0),
        Entry::Folder(usize::MAX),
        Entry::Image(0),
        Entry::Image(usize::MAX),
        Entry::Model(usize::MAX),
        Entry::File(usize::MAX),
        Entry::Scene {
            file: usize::MAX,
            room: 1,
        },
    ] {
        assert_rejected(&mut app, &browser, entry, Vec3::ZERO);
    }
    // A scanner snapshot of an ordinary text file must never become a model,
    // even if a caller accidentally submits it through the scene-card variant.
    browser.files[0].path = PathBuf::from("notes/readme.txt");
    browser.files[0].filename = "readme.txt".into();
    browser.files[0].extension = "txt".into();
    browser.files[0].kind = FileKind::Other;
    for entry in [Entry::File(0), Entry::Scene { file: 0, room: 1 }] {
        assert_rejected(&mut app, &browser, entry, Vec3::ZERO);
    }
}

#[test]
fn malformed_model_paths_and_invalid_drop_coordinates_are_rejected() {
    let (mut app, browser) = workspace();
    for point in [
        Vec3::NAN,
        Vec3::INFINITY,
        Vec3::new(1000.25, 0.0, 0.0),
        Vec3::new(0.0, -1000.25, 0.0),
        Vec3::new(0.0, 0.0, 1000.25),
    ] {
        assert_rejected(&mut app, &browser, Entry::Model(0), point);
    }
    for path in ["../outside.glb", "/absolute.glb", "models/readme.txt"] {
        app.world_mut().resource_mut::<Editor>().project.catalog[0].asset = path.into();
        assert_rejected(&mut app, &browser, Entry::Model(0), Vec3::ZERO);
    }
}

#[test]
fn an_unloaded_scene_closed_workspace_or_pending_confirmation_blocks_a_drop() {
    let (mut app, browser) = workspace();
    app.world_mut().resource_mut::<Editor>().ready = false;
    assert_rejected(&mut app, &browser, Entry::Model(0), Vec3::ZERO);

    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.ready = true;
        editor.open_tabs.clear();
    }
    assert_rejected(&mut app, &browser, Entry::Model(0), Vec3::ZERO);

    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.open_tabs.push(0);
        editor.pending = Some(PendingAction::CloseTab(0));
    }
    assert_rejected(&mut app, &browser, Entry::Model(0), Vec3::ZERO);
}

#[test]
fn measured_preview_bounds_preserve_the_catalog_collision_setting() {
    let bounds = BoxBounds {
        center: [0.0, 0.5, 0.0],
        half_size: [0.5; 3],
    };
    for solid in [false, true] {
        let (mut app, browser) = workspace();
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.project.catalog[0].solid = solid;
        assert!(apply_asset_drop(
            &mut editor,
            &browser,
            Entry::Model(0),
            Vec3::new(1.0, 0.0, 2.0),
            Some(bounds)
        ));
        assert_eq!(editor.edits.models[0].bounds, solid.then_some(bounds));
        assert_eq!(editor.undo.len(), 1);
    }
}
