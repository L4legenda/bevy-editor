//! Native project launcher. Each editor owns its own window and event loop.
mod effects;
mod native;
pub(crate) use native::{choose_folder, choose_folder_in, reveal as reveal_folder};
mod projects;
#[cfg(test)]
mod tests;
mod ui;
mod update;

use crate::{EditorProject, project::resolve_project_file};
use bevy::{
    input_focus::{InputFocus, tab_navigation::TabNavigationPlugin},
    prelude::*,
    ui_widgets::Button,
    window::WindowResizeConstraints,
};
use projects::{RecentProjects, Template};
use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

#[derive(Clone, Message)]
enum Action {
    New,
    Import,
    Close,
    PickOpen,
    PickParent,
    Submit,
    Template(Template),
    Subfolder,
    Select(PathBuf),
    Context(PathBuf, Vec2),
    DismissContext,
    Open(PathBuf),
    Favorite,
    Forget,
    Reveal,
    Sort,
    ClearFilter,
    UpdateEditor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialog {
    Create,
    Open,
}

#[derive(Clone, PartialEq)]
struct ContextMenu {
    file: PathBuf,
    position: Vec2,
}

struct Form {
    dialog: Dialog,
    title: String,
    folder: String,
    template: Template,
    subfolder: bool,
    error: String,
}
impl Form {
    fn destination(&self) -> io::Result<PathBuf> {
        let folder = projects::input_path(&self.folder)?;
        Ok(if self.subfolder {
            folder.join(projects::folder_name(&self.title))
        } else {
            folder
        })
    }
}

struct Launch {
    file: PathBuf,
    child: Child,
    reply: tempfile::TempDir,
}

impl Launch {
    fn result(&mut self) -> io::Result<Option<Result<String, String>>> {
        let status = self.child.try_wait()?;
        // The child renames the complete message into place before entering its event loop.
        match std::fs::read(self.reply.path().join("ready")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(status
                .map(|code| Err(format!("Редактор завершился при открытии проекта ({code})")))),
            Err(e) => Err(e),
        }
    }
}

#[derive(Resource)]
struct Manager {
    recent: RecentProjects,
    registry: PathBuf,
    selected: Option<PathBuf>,
    filter: String,
    alphabetic: bool,
    revision: u64,
    dialog_revision: u64,
    form: Option<Form>,
    context: Option<ContextMenu>,
    status: String,
    last_click: Option<(PathBuf, f64)>,
    launch: Option<Launch>,
    executable: PathBuf,
}
impl Manager {
    fn visible_projects(&self) -> Vec<&projects::RecentProject> {
        let filter = self.filter.trim().to_lowercase();
        let mut entries: Vec<_> = self
            .recent
            .projects
            .iter()
            .filter(|project| {
                filter.is_empty()
                    || project.title.to_lowercase().contains(&filter)
                    || project
                        .file
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect();
        entries.sort_by(|a, b| {
            b.favorite.cmp(&a.favorite).then_with(|| {
                if self.alphabetic {
                    a.title.to_lowercase().cmp(&b.title.to_lowercase())
                } else {
                    b.last_opened.cmp(&a.last_opened)
                }
            })
        });
        entries
    }
    fn reconcile_selection(&mut self) {
        let entries = self.visible_projects();
        if !entries
            .iter()
            .any(|project| Some(&project.file) == self.selected.as_ref())
        {
            self.selected = entries.first().map(|project| project.file.clone());
        }
    }
    fn set_filter(&mut self, value: String) {
        if self.filter != value {
            self.filter = value;
            self.status.clear();
            self.last_click = None;
            self.reconcile_selection();
            self.revision += 1;
        }
    }
    fn load(registry: PathBuf, executable: PathBuf) -> Self {
        let (recent, status) = match RecentProjects::read(&registry) {
            Ok(recent) => (recent, String::new()),
            Err(error) => (
                RecentProjects::default(),
                format!("Не удалось прочитать историю: {error}"),
            ),
        };
        let selected = recent.projects.first().map(|project| project.file.clone());
        Self {
            recent,
            registry,
            selected,
            filter: String::new(),
            alphabetic: false,
            revision: 1,
            dialog_revision: 0,
            form: None,
            context: None,
            status,
            last_click: None,
            launch: None,
            executable,
        }
    }
    fn save(&mut self) {
        if let Err(error) = self.recent.save(&self.registry) {
            self.status = format!("Не удалось сохранить список проектов: {error}");
        }
        self.revision += 1;
    }
    fn open(&mut self, path: &Path) -> io::Result<()> {
        let file = resolve_project_file(path)?;
        let reply = tempfile::tempdir()?;
        let mut command = Command::new(&self.executable);
        command
            .arg("--managed-project")
            .arg(&file)
            .arg(reply.path())
            .stdin(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        self.launch = Some(Launch { file, child, reply });
        self.status = "Открытие проекта…".into();
        self.revision += 1;
        Ok(())
    }
    fn show(&mut self, dialog: Dialog) {
        self.status.clear();
        let folder = projects::home()
            .map(|path| path.join("Projects"))
            .unwrap_or_default();
        self.form = Some(Form {
            dialog,
            title: "Новый проект".into(),
            folder: folder.to_string_lossy().into_owned(),
            template: Template::Starter,
            subfolder: true,
            error: String::new(),
        });
        self.dialog_revision += 1;
    }
    fn error(&mut self, error: impl std::fmt::Display) {
        if let Some(form) = &mut self.form {
            form.error = error.to_string();
        } else {
            self.status = error.to_string();
        }
    }
}

/// Store a successfully opened project in the user's recent-project list.
pub fn remember_project(path: impl AsRef<Path>) -> io::Result<()> {
    let file = resolve_project_file(path)?;
    let project = EditorProject::load(&file)?;
    remember_opened_project(&file, &project.title)
}

/// Record metadata already validated by the editor during launch.
pub fn remember_opened_project(path: &Path, title: &str) -> io::Result<()> {
    let file = resolve_project_file(path)?;
    let registry = projects::config_file()?;
    let mut recent = RecentProjects::read(&registry).unwrap_or_default();
    recent.remember(&file, title);
    recent.save(&registry)
}

#[cfg(test)]
pub(crate) fn create_test_project(directory: &Path) -> io::Result<EditorProject> {
    let file = projects::create_project(directory, "Test project", Template::Starter)?;
    EditorProject::load(file)
}

/// Start here when launching the standalone editor without a project argument.
pub fn create_project_manager_app() -> Result<App, Box<dyn std::error::Error>> {
    let manager = Manager::load(projects::config_file()?, std::env::current_exe()?);
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Bevy Editor · Менеджер проектов".into(),
            decorations: false,
            resolution: (1100, 760).into(),
            resize_constraints: WindowResizeConstraints {
                min_width: 880.0,
                min_height: 620.0,
                ..default()
            },
            ..default()
        }),
        ..default()
    }));
    let mut updates = update::Updates::default();
    updates.check();
    install(&mut app, manager);
    app.insert_resource(updates);
    Ok(app)
}

fn install(app: &mut App, manager: Manager) {
    app.insert_resource(manager)
        .add_plugins((
            crate::editor::icons::SvgIconsPlugin,
            crate::editor::starfield::StarfieldPlugin,
            crate::editor::orbital::OrbitalPlugin,
            effects::EffectsPlugin,
            crate::window_chrome::WindowChromePlugin,
        ))
        .init_resource::<native::Picker>()
        .init_resource::<update::Updates>()
        .add_message::<Action>()
        .add_plugins(TabNavigationPlugin)
        .add_observer(ui::activate)
        .add_systems(Startup, ui::setup)
        .add_systems(
            Update,
            (
                ui::inputs,
                shortcuts,
                ui::context_input,
                native::poll,
                actions,
                update::poll,
                ui::update_status,
                poll_editors,
                ui::refresh,
                ui::refresh_context,
                ui::styles,
                ui::scroll,
            )
                .chain(),
        );
}

fn actions(
    mut messages: MessageReader<Action>,
    mut manager: ResMut<Manager>,
    mut picker: ResMut<native::Picker>,
    time: Res<Time<Real>>,
    mut focus: ResMut<InputFocus>,
    mut updates: ResMut<update::Updates>,
) {
    for action in messages.read() {
        if manager.launch.is_some() || updates.downloading {
            break;
        }
        if picker.busy() && !matches!(action, Action::Close | Action::DismissContext) {
            continue;
        }
        // Modal dialogs own all interactions until closed.
        if manager.form.is_some()
            && matches!(
                action,
                Action::New
                    | Action::Import
                    | Action::Select(_)
                    | Action::Context(_, _)
                    | Action::Open(_)
                    | Action::Favorite
                    | Action::Forget
                    | Action::Reveal
                    | Action::Sort
                    | Action::ClearFilter
                    | Action::UpdateEditor
            )
        {
            continue;
        }
        match action {
            Action::Context(file, position) => {
                if manager
                    .visible_projects()
                    .iter()
                    .any(|project| &project.file == file)
                {
                    manager.selected = Some(file.clone());
                    manager.last_click = None;
                    manager.context = Some(ContextMenu {
                        file: file.clone(),
                        position: *position,
                    });
                }
            }
            Action::DismissContext => manager.context = None,
            Action::New => manager.show(Dialog::Create),
            Action::Import => manager.show(Dialog::Open),
            Action::Close => {
                manager.form = None;
                manager.dialog_revision += 1;
            }
            Action::PickOpen => picker.start(Dialog::Open),
            Action::PickParent => picker.start(Dialog::Create),
            Action::Template(template) => {
                if let Some(form) = &mut manager.form {
                    form.template = *template;
                }
            }
            Action::Subfolder => {
                if let Some(form) = &mut manager.form {
                    form.subfolder = !form.subfolder;
                }
            }
            Action::Select(file) => {
                manager.status.clear();
                let double = manager.last_click.as_ref().is_some_and(|(previous, at)| {
                    previous == file && time.elapsed_secs_f64() - at < 0.35
                });
                manager.last_click = Some((file.clone(), time.elapsed_secs_f64()));
                manager.selected = Some(file.clone());
                if double && let Err(error) = manager.open(file) {
                    manager.error(error);
                }
            }
            Action::Open(file) => {
                if let Err(error) = manager.open(file) {
                    manager.error(error);
                }
            }
            Action::Submit => {
                let result = if let Some(form) = &manager.form {
                    match form.dialog {
                        Dialog::Open => {
                            projects::input_path(&form.folder).and_then(|path| manager.open(&path))
                        }
                        Dialog::Create => {
                            let result = form.destination().and_then(|path| {
                                projects::create_project(&path, &form.title, form.template)
                            });
                            match result {
                                Ok(file) => {
                                    // Remember a created project even if the OS cannot start the editor.
                                    let title = manager
                                        .form
                                        .as_ref()
                                        .map(|form| form.title.trim().to_owned())
                                        .unwrap_or_default();
                                    manager.recent.remember(&file, &title);
                                    manager.selected = Some(file.clone());
                                    manager.save();
                                    manager.open(&file).map_err(|error| io::Error::other(format!("Проект создан в {}. Не удалось запустить редактор: {error}", file.display())))
                                }
                                Err(error) => Err(error),
                            }
                        }
                    }
                } else {
                    Ok(())
                };
                if let Err(error) = result {
                    manager.error(error);
                }
            }
            Action::Favorite => {
                let selected = manager.selected.clone();
                if let Some(project) = manager
                    .recent
                    .projects
                    .iter_mut()
                    .find(|project| Some(&project.file) == selected.as_ref())
                {
                    project.favorite = !project.favorite;
                    manager.save();
                }
            }
            Action::Forget => {
                let selected = manager.selected.clone();
                manager
                    .recent
                    .projects
                    .retain(|project| Some(&project.file) != selected.as_ref());
                manager.reconcile_selection();
                manager.status.clear();
                manager.save();
            }
            Action::Reveal => {
                if let Some(file) = &manager.selected
                    && let Err(error) = native::reveal(file.parent().unwrap_or(file))
                {
                    manager.error(error);
                }
            }
            Action::Sort => {
                manager.alphabetic = !manager.alphabetic;
                manager.revision += 1;
            }
            Action::ClearFilter => manager.set_filter(String::new()),
            Action::UpdateEditor if manager.context.is_none() && manager.launch.is_none() => {
                updates.install(manager.executable.clone());
            }
            Action::UpdateEditor => {}
        }
        if !matches!(action, Action::Context(_, _)) {
            manager.context = None;
        }
        if !matches!(action, Action::Template(_) | Action::Subfolder) {
            focus.clear();
        }
    }
}

fn poll_editors(mut manager: ResMut<Manager>, mut exit: MessageWriter<AppExit>) {
    let Some(launch) = &mut manager.launch else {
        return;
    };
    let result = match launch.result() {
        Ok(Some(result)) => result,
        Ok(None) => return,
        Err(e) => Err(e.to_string()),
    };
    let file = manager.launch.take().unwrap().file;
    match result {
        Ok(title) => {
            manager.recent.remember(&file, &title);
            manager.selected = Some(file);
            manager.filter.clear();
            manager.status.clear();
            manager.save();
            manager.form = None;
            manager.dialog_revision += 1;
            exit.write(AppExit::Success);
        }
        Err(error) => {
            manager.status.clear();
            manager.error(format!("Не удалось открыть проект · {error}"));
            manager.revision += 1;
        }
    }
}

fn shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    buttons: Query<(), With<Button>>,
    manager: Res<Manager>,
    mut messages: MessageWriter<Action>,
) {
    let modified = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let action = if keys.just_pressed(KeyCode::Escape) && manager.context.is_some() {
        Some(Action::DismissContext)
    } else if keys.just_pressed(KeyCode::Escape) && manager.form.is_some() {
        Some(Action::Close)
    } else if keys.just_pressed(KeyCode::Escape) && !manager.filter.is_empty() {
        Some(Action::ClearFilter)
    } else if modified && keys.just_pressed(KeyCode::KeyN) && manager.form.is_none() {
        Some(Action::New)
    } else if modified && keys.just_pressed(KeyCode::KeyO) && manager.form.is_none() {
        Some(Action::Import)
    } else if keys.just_pressed(KeyCode::Enter)
        && !focus.get().is_some_and(|entity| buttons.contains(entity))
    {
        if manager.form.is_some() {
            Some(Action::Submit)
        } else {
            manager.selected.clone().map(Action::Open)
        }
    } else {
        None
    };
    if let Some(action) = action {
        messages.write(action);
    }
}
