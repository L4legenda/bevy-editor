use super::*;
use crate::editor::{generation_backend as backend, generation_model::Store};
use bevy::{
    tasks::{IoTaskPool, Task, block_on, poll_once},
    window::RequestRedraw,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

#[path = "generation_preview.rs"]
mod preview;

struct Job {
    started: std::time::Instant,
    progress: Arc<backend::Progress>,
    task: Task<Result<backend::ResultModel, String>>,
}
struct Loading {
    progress: Arc<backend::Progress>,
    task: Task<Result<(), String>>,
}

#[derive(Resource, Default)]
pub(super) struct State {
    details_open: bool,
    runtime: Option<backend::Runtime>,
    service: Option<backend::Service>,
    loading: Option<Loading>,
    probe: Option<Task<Result<backend::Runtime, String>>>,
    checked: bool,
    job: Option<Job>,
    pub result: Option<backend::ResultModel>,
    imported: Option<(PathBuf, usize)>,
    import_task: Option<Task<Result<PathBuf, String>>>,
    error: String,
    #[cfg(test)]
    fixture_ready: bool,
}

impl State {
    fn ready(&self) -> bool {
        #[cfg(test)]
        if self.fixture_ready {
            return true;
        }
        self.service.as_ref().is_some_and(backend::Service::ready)
    }
    fn stop(&mut self) {
        if let Some(loading) = &self.loading {
            loading.progress.shutdown();
        }
        if let Some(job) = &self.job {
            job.progress.shutdown();
        }
        if let Some(service) = self.service.take() {
            service.shutdown();
        }
        self.loading = None;
        self.job = None;
        #[cfg(test)]
        {
            self.fixture_ready = false;
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Resource)]
pub(super) struct ServiceUi {
    panel: Entity,
    title: Entity,
    description: Entity,
    start: Entity,
    retry: Entity,
    progress: Entity,
    fill: Entity,
    stage: Entity,
    error: Entity,
}
#[derive(Resource)]
pub(super) struct FooterUi {
    root: Entity,
    label: Entity,
}

pub(super) fn service_setup(
    commands: &mut Commands,
    parent: Entity,
    _normal: Entity,
    handle: &Handle<Font>,
) {
    let panel = node(
        commands,
        parent,
        Node {
            display: Display::None,
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..column(16.0)
        },
    );
    let mark = surface(
        commands,
        panel,
        Node {
            width: px(84),
            height: px(84),
            border_radius: BorderRadius::all(px(22)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        WELL,
    );
    icons::spawn(commands, mark, icons::Glyph::Box, 36.0, FOCUS);
    let title = text(commands, panel, handle, "3D генератор выключен", 24.0, TEXT);
    let description = text(
        commands,
        panel,
        handle,
        "Включите модель, чтобы создавать 3D объекты по изображению",
        13.0,
        MUTED,
    );
    commands
        .entity(description)
        .insert(TextLayout::justify(Justify::Center));
    let start = action(
        commands,
        panel,
        handle,
        "Включить модель",
        GenerationAction::StartModel,
        true,
    );
    commands
        .entity(start)
        .insert(AccessibleLabel("Включить 3D модель".into()));
    let retry = action(
        commands,
        panel,
        handle,
        "Проверить снова",
        GenerationAction::CheckModel,
        false,
    );
    let progress = node(
        commands,
        panel,
        Node {
            display: Display::None,
            width: px(360),
            max_width: percent(90),
            ..column(10.0)
        },
    );
    let stage = text(commands, progress, handle, "Загрузка модели…", 12.0, MUTED);
    let (track, fill) = super::progress::bar(commands, progress, 6.0);
    commands
        .entity(track)
        .insert(AccessibleLabel("Прогресс запуска 3D генератора".into()));
    action(
        commands,
        progress,
        handle,
        "Отменить запуск",
        GenerationAction::StopModel,
        false,
    );
    let error = text(commands, panel, handle, "", 12.0, ERROR);
    commands.entity(error).insert((
        TextLayout::justify(Justify::Center),
        Node {
            max_width: px(600),
            ..default()
        },
    ));
    commands.insert_resource(ServiceUi {
        panel,
        title,
        description,
        start,
        retry,
        progress,
        fill,
        stage,
        error,
    });
}
pub(super) fn service_footer(commands: &mut Commands, parent: Entity, handle: &Handle<Font>) {
    let root = node(
        commands,
        parent,
        Node {
            display: Display::None,
            flex_shrink: 0.0,
            ..row(8.0)
        },
    );
    let label = text(
        commands,
        root,
        handle,
        "● Работает 3D генератор",
        11.0,
        GREEN,
    );
    action(
        commands,
        root,
        handle,
        "Остановить",
        GenerationAction::StopModel,
        false,
    );
    commands.insert_resource(FooterUi { root, label });
}

#[cfg(test)]
impl State {
    pub(super) fn unavailable_fixture() -> Self {
        let mut state = Self::default();
        state.checked = true;
        state
    }
    pub(super) fn ready_fixture() -> Self {
        let mut state = Self::unavailable_fixture();
        state.fixture_ready = true;
        state.runtime = Some(backend::Runtime {
            executable: "fixture".into(),
        });
        state
    }
}

#[derive(Resource)]
pub(super) struct RunUi {
    hint: Entity,
    progress: Entity,
    stage: Entity,
    fill: Entity,
    bar: Entity,
    cancel: Entity,
    notice: Entity,
    stats: Entity,
    details: Entity,
    details_text: Entity,
    import: Entity,
    open: Entity,
    result_tools: Entity,
    placeholder: Entity,
    wireframe: Entity,
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<State>();
    preview::register(app);
}

pub(super) fn setup(
    commands: &mut Commands,
    launch: Entity,
    result: Entity,
    placeholder: Entity,
    handle: &Handle<Font>,
    hint: Entity,
    heading: Entity,
) {
    let wireframe = action(
        commands,
        heading,
        handle,
        "Сетка",
        GenerationAction::ToggleWireframe,
        false,
    );
    let progress = node(
        commands,
        launch,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(7.0)
        },
    );
    let stage = text(commands, progress, handle, "", 12.0, TEXT);
    let (bar, fill) = super::progress::bar(commands, progress, 6.0);
    commands
        .entity(bar)
        .insert(AccessibleLabel("Прогресс обработки 3D модели".into()));
    let cancel = action(
        commands,
        progress,
        handle,
        "Отменить генерацию",
        GenerationAction::CancelGeneration,
        false,
    );
    let notice = text(commands, launch, handle, "", 11.0, ERROR);
    let stats = text(commands, result, handle, "", 12.0, MUTED);
    let result_tools = node(
        commands,
        result,
        Node {
            display: Display::None,
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            ..row(8.0)
        },
    );
    let import = action(
        commands,
        result_tools,
        handle,
        "Добавить в проект",
        GenerationAction::ImportResult,
        true,
    );
    let open = action(
        commands,
        result_tools,
        handle,
        "Открыть модель",
        GenerationAction::OpenResult,
        false,
    );
    action(
        commands,
        result_tools,
        handle,
        "Сбросить вид",
        GenerationAction::FitResult,
        false,
    );
    disclosure(
        commands,
        result_tools,
        handle,
        "Подробности",
        Section::Details,
    );
    let details = node(
        commands,
        result,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(5.0)
        },
    );
    let details_text = text(commands, details, handle, "", 11.0, MUTED);
    text(
        commands,
        details,
        handle,
        "ЛКМ — вращение · колесо — масштаб",
        11.0,
        MUTED,
    );
    preview::setup(commands, result, handle);
    commands.insert_resource(RunUi {
        hint,
        progress,
        stage,
        fill,
        bar,
        cancel,
        notice,
        stats,
        details,
        details_text,
        import,
        open,
        result_tools,
        placeholder,
        wireframe,
    });
}

pub(super) fn update(
    mut events: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    installation: Res<installation::State>,
    mut state: ResMut<State>,
    mut redraw: MessageWriter<RequestRedraw>,
    mut thumbnails: ResMut<crate::editor::thumbnails::Thumbnails>,
) {
    if editor.generation.active && !state.checked && state.probe.is_none() {
        let mut roots = vec![editor.root.clone()];
        if let Ok(exe) = std::env::current_exe() {
            roots.push(exe);
        }
        state.probe =
            Some(IoTaskPool::get().spawn(async move { backend::Runtime::discover(&roots) }));
    }
    if let Some(result) = state
        .probe
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        state.probe = None;
        state.checked = true;
        match result {
            Ok(runtime) => {
                state.runtime = Some(runtime);
                state.error.clear();
            }
            Err(error) => state.error = error,
        }
    }
    for event in events.read() {
        let Action::Generation(action) = event else {
            continue;
        };
        if !matches!(action, GenerationAction::StopModel)
            && (!editor.generation.active || editor.pending.is_some() || editor.browser_popup)
        {
            continue;
        }
        match action {
            GenerationAction::StopModel => {
                state.stop();
                state.error.clear();
            }
            GenerationAction::StartModel if state.service.is_none() => {
                let (Some(runtime), Some(model)) =
                    (state.runtime.clone(), installation.found.clone())
                else {
                    continue;
                };
                let store = match Store::system().and_then(|store| {
                    store.prepare()?;
                    Ok(store)
                }) {
                    Ok(store) => store,
                    Err(error) => {
                        state.error = error;
                        continue;
                    }
                };
                match runtime.launch(&model, &store.root) {
                    Ok(service) => {
                        let progress = Arc::new(backend::Progress::default());
                        let worker = service.clone();
                        let monitor = progress.clone();
                        state.loading = Some(Loading {
                            progress,
                            task: IoTaskPool::get()
                                .spawn(async move { worker.wait_ready(&monitor) }),
                        });
                        state.service = Some(service);
                        state.error.clear();
                    }
                    Err(error) => state.error = error,
                }
            }
            GenerationAction::Generate if state.job.is_none() && state.import_task.is_none() => {
                if !editor.generation.source_ready || !editor.generation.config_valid {
                    continue;
                }
                let (Some(runtime), Some(model), Some(source)) = (
                    state.service.clone(),
                    installation.found.clone(),
                    editor.generation.source.clone(),
                ) else {
                    continue;
                };
                let request = match backend::Request::from_draft(&editor.generation, model) {
                    Ok(request) => request,
                    Err(error) => {
                        state.error = error;
                        continue;
                    }
                };
                let store = match Store::system().and_then(|store| {
                    store.prepare()?;
                    Ok(store)
                }) {
                    Ok(store) => store,
                    Err(error) => {
                        state.error = error;
                        continue;
                    }
                };
                let progress = Arc::new(backend::Progress::default());
                let worker = progress.clone();
                state.error.clear();
                state.job = Some(Job {
                    started: std::time::Instant::now(),
                    progress,
                    task: IoTaskPool::get().spawn(async move {
                        backend::run(&runtime, &request, &source, &store.root, worker)
                    }),
                });
            }
            GenerationAction::CancelGeneration => {
                if let Some(job) = &state.job {
                    job.progress.cancelled.store(true, Ordering::Relaxed);
                }
            }
            GenerationAction::Section(Section::Details) => state.details_open = !state.details_open,
            GenerationAction::CheckModel if state.job.is_none() => state.checked = false,
            GenerationAction::ImportResult
                if state.job.is_none()
                    && state.import_task.is_none()
                    && state.imported.is_none() =>
            {
                if let Some(result) = state.result.clone() {
                    let root = editor.root.clone();
                    state.error.clear();
                    state.import_task = Some(
                        IoTaskPool::get().spawn(async move { backend::import(&result, &root) }),
                    );
                }
            }
            GenerationAction::OpenResult => {
                if let Some((_, index)) = &state.imported {
                    editor.open_model_tab(*index);
                }
            }
            _ => {}
        }
    }
    if let Some(result) = state
        .loading
        .as_mut()
        .and_then(|load| block_on(poll_once(&mut load.task)))
    {
        state.loading = None;
        if let Err(error) = result {
            state.stop();
            state.error = error;
        }
    }
    if state
        .service
        .as_ref()
        .is_some_and(|service| !service.alive())
    {
        state.stop();
        state.error = "3D генератор завершился. Включите модель снова. Подробности — в системном generator-stderr.log.".into();
    }
    if let Some(result) = state
        .job
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)))
    {
        state.job = None;
        match result {
            Ok(result) => {
                state.result = Some(result);
                state.imported = None;
                state.error.clear();
            }
            Err(error) => state.error = error,
        }
    }
    if let Some(result) = state
        .import_task
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        state.import_task = None;
        match result {
            Ok(path) => {
                browser.refresh(&mut editor.project);
                thumbnails.extend(editor.project.catalog.len());
                let asset = path.to_string_lossy().replace('\\', "/");
                if let Some(index) = editor
                    .project
                    .catalog
                    .iter()
                    .position(|item| item.asset == asset)
                {
                    state.imported = Some((path.clone(), index));
                    if let Some(folder) = browser
                        .folders
                        .iter()
                        .position(|folder| Some(folder.path.as_path()) == path.parent())
                    {
                        browser.navigate(folder);
                    }
                } else {
                    state.error = "Файл создан, но файловый менеджер не смог найти модель".into();
                }
            }
            Err(error) => state.error = error,
        }
    }
    editor.generation.backend_ready = state.ready() && installation.found.is_some();
    editor.generation.engine_active = state.service.is_some();
    editor.generation.engine_loading = state.loading.is_some();
    editor.generation.running = state.job.is_some();
    editor.generation.has_result = state.result.is_some();
    editor.generation.result_imported = state.imported.is_some() || state.import_task.is_some();
    if state.job.is_some()
        || state.loading.is_some()
        || state.probe.is_some()
        || state.import_task.is_some()
    {
        redraw.write(RequestRedraw);
    }
}

pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    state: Res<State>,
    ui: Res<RunUi>,
    service_ui: Res<ServiceUi>,
    footer_ui: Res<FooterUi>,
    installation: Res<installation::State>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut folds: Query<(&FoldIcon, &mut UiTransform)>,
) {
    let busy = state.job.is_some();
    let checking = state.probe.is_some();
    let importing = state.import_task.is_some();
    let active = busy || checking || importing;
    let loading = state.loading.is_some();
    let off = installation.found.is_some() && !state.ready();
    for (entity, shown) in [
        (ui.progress, active),
        (ui.bar, active),
        (ui.cancel, busy),
        (ui.notice, !state.error.is_empty()),
        (ui.placeholder, state.result.is_none()),
        (ui.wireframe, state.result.is_some()),
        (ui.result_tools, state.result.is_some()),
        (ui.open, state.imported.is_some()),
        (ui.import, state.imported.is_none()),
        (ui.details, state.result.is_some() && state.details_open),
        (ui.hint, !active && state.runtime.is_none()),
        (service_ui.panel, off),
        (service_ui.start, off && !loading),
        (service_ui.retry, off && !loading && state.runtime.is_none()),
        (service_ui.progress, loading),
        (service_ui.description, !loading),
        (service_ui.error, off && !state.error.is_empty()),
        (footer_ui.root, state.service.is_some()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    for (FoldIcon(section), mut icon) in &mut folds {
        if *section == Section::Details {
            icon.set_if_neq(UiTransform::from_rotation(Rot2::radians(
                if state.details_open {
                    std::f32::consts::FRAC_PI_2
                } else {
                    0.0
                },
            )));
        }
    }
    let mut set = |entity, value: &str| {
        if let Ok(mut label) = labels.get_mut(entity) {
            if label.0 != value {
                label.0 = value.into();
            }
        }
    };
    set(
        ui.hint,
        "Rust-генератор недоступен. Откройте блок «Система».",
    );
    set(
        service_ui.title,
        if loading {
            "Включение 3D генератора"
        } else {
            "3D генератор выключен"
        },
    );
    set(service_ui.error, &state.error);
    set(
        footer_ui.label,
        if loading {
            "● Загрузка 3D генератора…"
        } else if busy {
            "● Генерация 3D модели…"
        } else {
            "● Работает 3D генератор"
        },
    );
    let mut load_value = super::progress::Value::Hidden;
    if let Some(load) = &state.loading {
        let status = load.progress.snapshot();
        load_value = if status.total > 0 {
            super::progress::Value::Fraction(status.current as f32 / status.total as f32)
        } else {
            super::progress::Value::Busy
        };
        let caption = if status.total > 0 {
            format!("Загрузка модели · {}%", status.current * 100 / status.total)
        } else {
            "Запуск CUDA…".into()
        };
        set(service_ui.stage, &caption);
    }
    super::progress::set(
        &mut commands,
        service_ui.fill,
        if editor.generation.active {
            load_value
        } else {
            super::progress::Value::Hidden
        },
    );
    if loading || state.runtime.is_none() || checking {
        commands
            .entity(service_ui.start)
            .insert(InteractionDisabled);
    } else {
        commands
            .entity(service_ui.start)
            .remove::<InteractionDisabled>();
    }
    set(ui.notice, &state.error);
    let mut value = super::progress::Value::Hidden;
    if let Some(job) = &state.job {
        let stage = job.progress.snapshot();
        let elapsed = job.started.elapsed().as_secs();
        let caption = if job.progress.cancelled.load(Ordering::Relaxed) {
            "Остановка…".into()
        } else {
            match stage.stage.as_str() {
                "loading" | "" => "Загрузка Mini…".into(),
                "conditioning" => "Анализ изображения…".into(),
                "diffusion" => format!("Генерация формы · {} / {}", stage.current, stage.total),
                "surface" => "Восстановление поверхности…".into(),
                "optimizing" => "Оптимизация сетки…".into(),
                "exporting" => "Сохранение модели…".into(),
                _ => "Завершение…".into(),
            }
        };
        set(
            ui.stage,
            &format!("{caption}  ·  {}:{:02}", elapsed / 60, elapsed % 60),
        );
        value = if stage.total > 0 && !job.progress.cancelled.load(Ordering::Relaxed) {
            super::progress::Value::Fraction(stage.current as f32 / stage.total as f32)
        } else {
            super::progress::Value::Busy
        };
        if job.progress.cancelled.load(Ordering::Relaxed) {
            commands.entity(ui.cancel).insert(InteractionDisabled);
        }
    } else if checking {
        set(ui.stage, "Проверка Rust-генератора…");
        value = super::progress::Value::Busy;
    } else if importing {
        set(ui.stage, "Добавление в проект…");
        value = super::progress::Value::Busy;
    }
    super::progress::set(
        &mut commands,
        ui.fill,
        if editor.generation.active {
            value
        } else {
            super::progress::Value::Hidden
        },
    );
    if let Some(result) = &state.result {
        let summary = if let Some((path, _)) = &state.imported {
            format!("В проекте · {}", path.display())
        } else {
            format!("{} · {} треугольников", result.name, result.faces)
        };
        set(ui.stats, &summary);
        set(
            ui.details_text,
            &format!(
                "{} вершин · {} шагов · CFG {} · разрешение {}\nSeed {} · {:.0} с · исходно {} граней",
                result.vertices,
                result.steps,
                result.guidance,
                result.resolution,
                result.seed,
                result.seconds,
                result.raw_faces
            ),
        );
    } else {
        set(ui.stats, "");
    }
}

#[cfg(test)]
#[path = "generation_run_tests.rs"]
mod tests;
