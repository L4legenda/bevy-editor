use super::*;
use bevy::gltf::gltf;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "bevy-editor-projects-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn templates_produce_loadable_portable_projects_and_valid_glb_assets() {
    let scratch = Scratch::new();
    for (name, template, nodes) in [
        ("empty", Template::Empty, 1),
        ("starter", Template::Starter, 3),
    ] {
        let root = scratch.0.join(name);
        let file = projects::create_project(&root, "Мой проект", template).unwrap();
        assert_eq!(file.file_name().unwrap(), crate::project::PROJECT_FILE_NAME);
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(raw["asset_root"], "assets");
        let project = EditorProject::load(&root).unwrap();
        assert_eq!(project.title, "Мой проект");
        let edits = project.read_scene(0).unwrap();
        assert_eq!(edits.lights.is_empty(), template == Template::Empty);
        for asset in [&project.scenes[0].asset, &project.catalog[0].asset] {
            let bytes = fs::read(project.asset_root.join(asset)).unwrap();
            let parsed = gltf::Gltf::from_slice(&bytes)
                .expect("A template must produce a valid glTF 2.0 binary");
            if asset.starts_with("scenes/") {
                assert_eq!(parsed.nodes().count(), nodes);
            }
            for mesh in parsed.meshes() {
                for primitive in mesh.primitives() {
                    let reader = primitive.reader(|_| parsed.blob.as_deref());
                    let positions: Vec<_> = reader.read_positions().unwrap().collect();
                    let indices: Vec<_> = reader.read_indices().unwrap().into_u32().collect();
                    assert_eq!(positions.len(), 24);
                    assert_eq!(indices.len(), 36);
                    assert!(
                        indices
                            .iter()
                            .all(|index| (*index as usize) < positions.len())
                    );
                }
            }
        }
        // Moving the entire project must preserve every relative reference.
        let moved = scratch.0.join(format!("{name}-moved"));
        fs::rename(&root, &moved).unwrap();
        assert_eq!(EditorProject::load(&moved).unwrap().title, project.title);
    }
}

#[test]
fn creating_a_project_never_overwrites_existing_files() {
    let scratch = Scratch::new();
    let existing = scratch.0.join("important.txt");
    fs::write(&existing, b"user content").unwrap();
    assert!(projects::create_project(&scratch.0, "Keep", Template::Starter).is_err());
    assert_eq!(fs::read(&existing).unwrap(), b"user content");
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    let root = scratch.0.join("new");
    let file = projects::create_project(&root, "First", Template::Empty).unwrap();
    let bytes = fs::read(&file).unwrap();
    assert!(projects::create_project(&root, "Second", Template::Starter).is_err());
    assert_eq!(fs::read(file).unwrap(), bytes);
    assert!(projects::create_project(&scratch.0.join("invalid"), " ", Template::Empty).is_err());
    assert!(!scratch.0.join("invalid").exists());
}

#[test]
fn folder_resolution_keeps_legacy_projects_compatible_and_prefers_standard_manifest() {
    let scratch = Scratch::new();
    let root = scratch.0.join("project");
    let file = projects::create_project(&root, "Legacy", Template::Empty).unwrap();
    let legacy = root.join("project.json");
    fs::rename(&file, &legacy).unwrap();
    assert_eq!(
        resolve_project_file(&root).unwrap(),
        legacy.canonicalize().unwrap()
    );
    assert_eq!(EditorProject::load(&root).unwrap().title, "Legacy");
    fs::copy(&legacy, &file).unwrap();
    assert_eq!(
        resolve_project_file(&root).unwrap(),
        file.canonicalize().unwrap()
    );
    assert!(resolve_project_file(&scratch.0).is_err());
}

#[test]
fn recents_round_trip_deduplicate_preserve_favorites_and_back_up_corrupt_history() {
    let scratch = Scratch::new();
    let registry = scratch.0.join("config/recent-projects.json");
    let mut recent = RecentProjects::read(&registry).unwrap();
    let first = scratch.0.join("first/project.bevyeditor");
    recent.remember(&first, "Первый");
    recent.projects[0].favorite = true;
    recent.remember(&scratch.0.join("second/project.bevyeditor"), "Второй");
    recent.remember(&first, "Переименован");
    recent.save(&registry).unwrap();
    recent.save(&registry).unwrap();
    let loaded = RecentProjects::read(&registry).unwrap();
    assert_eq!(loaded.projects.len(), 2);
    assert!(loaded.projects[0].favorite);
    assert_eq!(loaded.projects[0].title, "Переименован");
    fs::write(&registry, b"corrupt history").unwrap();
    recent.save(&registry).unwrap();
    assert_eq!(RecentProjects::read(&registry).unwrap().projects.len(), 2);
    let backup = fs::read_dir(registry.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".backup.json")
        })
        .unwrap();
    assert_eq!(fs::read(backup.path()).unwrap(), b"corrupt history");
}

fn controller(scratch: &Scratch) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(Manager::load(
            scratch.0.join("history.json"),
            PathBuf::from("/usr/bin/true"),
        ))
        .init_resource::<InputFocus>()
        .init_resource::<native::Picker>()
        .add_message::<Action>()
        .add_systems(Update, (actions, poll_editors).chain());
    app
}
fn action(app: &mut App, action: Action) {
    app.world_mut().write_message(action);
    app.update();
}

#[test]
fn forgetting_removes_only_the_recent_entry_and_modal_guards_background_actions() {
    let scratch = Scratch::new();
    let file = projects::create_project(&scratch.0.join("keep"), "Keep", Template::Empty).unwrap();
    let mut app = controller(&scratch);
    {
        let mut manager = app.world_mut().resource_mut::<Manager>();
        manager.recent.remember(&file, "Keep");
        manager.selected = Some(file.clone());
    }
    action(&mut app, Action::New);
    action(&mut app, Action::Forget);
    assert_eq!(app.world().resource::<Manager>().recent.projects.len(), 1);
    action(&mut app, Action::Close);
    action(&mut app, Action::Favorite);
    assert!(app.world().resource::<Manager>().recent.projects[0].favorite);
    action(&mut app, Action::Forget);
    assert!(app.world().resource::<Manager>().recent.projects.is_empty());
    assert!(EditorProject::load(&file).is_ok());
}

#[cfg(unix)]
#[test]
fn create_and_open_remembers_project_and_launch_failure_keeps_created_files() {
    let scratch = Scratch::new();
    let mut app = controller(&scratch);
    action(&mut app, Action::Open(scratch.0.join("missing-project")));
    assert!(app.should_exit().is_none());
    action(&mut app, Action::New);
    {
        let mut manager = app.world_mut().resource_mut::<Manager>();
        let form = manager.form.as_mut().unwrap();
        form.folder = scratch.0.to_string_lossy().into_owned();
        form.title = "Created".into();
    }
    action(&mut app, Action::Submit);
    assert!(matches!(app.should_exit(), Some(AppExit::Success)));
    app.world_mut().resource_mut::<Messages<AppExit>>().clear();
    let file = scratch
        .0
        .join("Created/project.bevyeditor")
        .canonicalize()
        .unwrap();
    {
        let manager = app.world().resource::<Manager>();
        assert!(manager.form.is_none());
        assert_eq!(manager.running.len(), 1);
        assert_eq!(
            RecentProjects::read(&manager.registry).unwrap().projects[0].file,
            file
        );
    }
    for (_, mut child) in app.world_mut().resource_mut::<Manager>().running.drain(..) {
        child.wait().unwrap();
    }
    app.world_mut().resource_mut::<Manager>().executable = scratch.0.join("missing-executable");
    action(&mut app, Action::New);
    {
        let mut manager = app.world_mut().resource_mut::<Manager>();
        let form = manager.form.as_mut().unwrap();
        form.folder = scratch.0.to_string_lossy().into_owned();
        form.title = "Saved despite launch error".into();
    }
    action(&mut app, Action::Submit);
    let manager = app.world().resource::<Manager>();
    assert!(
        manager
            .form
            .as_ref()
            .unwrap()
            .error
            .contains("Проект создан")
    );
    assert_eq!(manager.recent.projects.len(), 2);
    assert!(EditorProject::load(scratch.0.join("Saved despite launch error")).is_ok());
    assert!(app.should_exit().is_none());
}

#[test]
fn folder_names_and_manual_paths_handle_spaces_unicode_and_relative_input() {
    assert_eq!(projects::folder_name("  Мой / проект:  "), "Мой - проект-");
    assert_eq!(projects::folder_name("..."), "New Project");
    assert_eq!(
        projects::input_path("\"folder with spaces\"").unwrap(),
        std::env::current_dir().unwrap().join("folder with spaces")
    );
    assert_eq!(
        projects::input_path("~/Проекты").unwrap(),
        projects::home().unwrap().join("Проекты")
    );
}

#[path = "render_tests.rs"]
mod render_tests;

#[test]
fn filtering_moves_selection_to_a_visible_result_and_clears_stale_selection() {
    let scratch = Scratch::new();
    let mut manager = Manager::load(scratch.0.join("history.json"), "/usr/bin/true".into());
    let alpha = scratch.0.join("alpha/project.bevyeditor");
    let beta = scratch.0.join("beta/project.bevyeditor");
    manager.recent.remember(&alpha, "Альфа");
    manager.recent.remember(&beta, "Бета");
    manager.selected = Some(alpha.clone());
    manager.set_filter("бета".into());
    assert_eq!(manager.selected.as_ref(), Some(&beta));
    assert_eq!(manager.visible_projects().len(), 1);
    manager.set_filter("нет совпадений".into());
    assert!(manager.visible_projects().is_empty());
    assert!(manager.selected.is_none());
    manager.set_filter(String::new());
    assert!(
        manager
            .visible_projects()
            .iter()
            .any(|project| Some(&project.file) == manager.selected.as_ref())
    );
}

#[cfg(unix)]
#[test]
fn enter_in_search_opens_the_matching_project_and_resets_the_filter() {
    let scratch = Scratch::new();
    let alpha =
        projects::create_project(&scratch.0.join("alpha"), "Альфа", Template::Empty).unwrap();
    let beta = projects::create_project(&scratch.0.join("beta"), "Бета", Template::Empty).unwrap();
    let mut app = controller(&scratch);
    app.init_resource::<ButtonInput<KeyCode>>()
        .add_systems(Update, shortcuts.before(actions));
    {
        let mut manager = app.world_mut().resource_mut::<Manager>();
        manager.recent.remember(&alpha, "Альфа");
        manager.recent.remember(&beta, "Бета");
        manager.selected = Some(alpha);
        manager.set_filter("бета".into());
    }
    let search = app
        .world_mut()
        .spawn((ui::Input::Search, bevy::text::EditableText::new("бета")))
        .id();
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(search, bevy::input_focus::FocusCause::Pressed);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Enter);
    app.update();
    assert!(matches!(app.should_exit(), Some(AppExit::Success)));
    {
        let manager = app.world().resource::<Manager>();
        assert_eq!(manager.running[0].0, beta);
        assert!(manager.filter.is_empty());
        assert!(manager.status.is_empty());
    }
    for (_, mut child) in app.world_mut().resource_mut::<Manager>().running.drain(..) {
        child.wait().unwrap();
    }
}
