//! Model discovery and installation keep running independently of document tabs.
use super::*;
use crate::editor::generation_model::{self as model, Progress, Store};
use bevy::{
    tasks::{IoTaskPool, Task, block_on, poll_once},
    window::RequestRedraw,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

struct Job {
    progress: Arc<Progress>,
    task: Task<Result<PathBuf, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        if self.progress.phase.load(Ordering::Relaxed) != 3 {
            self.progress.cancel();
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct State {
    details_open: bool,
    store: Option<Store>,
    pub found: Option<PathBuf>,
    checked: bool,
    scan: Option<Task<Result<(Store, Option<PathBuf>, bool), String>>>,
    job: Option<Job>,
    error: String,
    resumable: bool,
}

#[cfg(test)]
impl State {
    pub(super) fn installed_fixture() -> Self {
        Self {
            found: Some(PathBuf::from("test-model")),
            checked: true,
            ..default()
        }
    }
}

#[derive(Resource)]
pub(super) struct InstallUi {
    normal: Entity,
    panel: Entity,
    badge: Entity,
    title: Entity,
    description: Entity,
    download: Entity,
    retry: Entity,
    progress: Entity,
    fill: Entity,
    amount: Entity,
    phase: Entity,
    cancel: Entity,
    error: Entity,
    location: Entity,
    details: Entity,
}

pub(super) fn setup(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    normal: Entity,
    badge: Entity,
) {
    let panel = node(
        commands,
        parent,
        Node {
            display: Display::None,
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.0,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::all(px(20)),
            ..column(18.0)
        },
    );
    let mark = surface(
        commands,
        panel,
        Node {
            width: px(84),
            height: px(84),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(20)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        WELL,
    );
    commands.entity(mark).insert(BorderColor::all(EDGE));
    icons::spawn(commands, mark, icons::Glyph::Box, 38.0, FOCUS);
    let title = text(commands, panel, handle, "Проверка модели…", 24.0, TEXT);
    let description = text(commands, panel, handle, "", 13.0, MUTED);
    commands.entity(description).insert((
        TextLayout::justify(Justify::Center),
        Node {
            max_width: px(540),
            ..default()
        },
    ));
    let download = action(
        commands,
        panel,
        handle,
        "Скачать · 3,82 ГБ",
        GenerationAction::DownloadModel,
        true,
    );
    commands.entity(download).insert(Node {
        height: px(42),
        padding: UiRect::horizontal(px(22)),
        border_radius: BorderRadius::all(px(8)),
        ..row(8.0)
    });
    disclosure(
        commands,
        panel,
        handle,
        "Сведения об установке",
        Section::Installation,
    );
    let details = node(
        commands,
        panel,
        Node {
            display: Display::None,
            max_width: px(660),
            align_items: AlignItems::Center,
            ..column(8.0)
        },
    );
    let retry = action(
        commands,
        details,
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
            width: px(440),
            max_width: percent(95),
            align_items: AlignItems::Stretch,
            ..column(12.0)
        },
    );
    let phase = text(
        commands,
        progress,
        handle,
        "Скачивание Hunyuan3D-2mini…",
        13.0,
        TEXT,
    );
    let (track, fill) = super::progress::bar(commands, progress, 8.0);
    commands
        .entity(track)
        .insert(AccessibleLabel("Прогресс скачивания модели".into()));
    let amount = text(
        commands,
        progress,
        handle,
        "0% · 0 МБ / 3,82 ГБ",
        12.0,
        MUTED,
    );
    let cancel = action(
        commands,
        progress,
        handle,
        "Отменить",
        GenerationAction::CancelModelDownload,
        false,
    );
    commands.entity(panel).insert_children(3, &[progress]);
    let error = text(commands, panel, handle, "", 12.0, ERROR);
    commands.entity(error).insert((
        TextLayout::justify(Justify::Center),
        Node {
            max_width: px(600),
            ..default()
        },
    ));
    let location = text(commands, details, handle, "", 11.0, MUTED);
    commands.entity(location).insert((
        TextLayout::justify(Justify::Center),
        Node {
            max_width: px(660),
            ..default()
        },
    ));
    text(
        commands,
        details,
        handle,
        "Системные файлы редактора. В проект и сборку игры не попадут.",
        11.0,
        MUTED,
    );
    commands.insert_resource(InstallUi {
        normal,
        panel,
        badge,
        title,
        description,
        download,
        retry,
        progress,
        fill,
        amount,
        phase,
        cancel,
        error,
        location,
        details,
    });
}

fn start_scan(state: &mut State, root: PathBuf) {
    let mut roots = vec![root];
    if let Ok(exe) = std::env::current_exe() {
        roots.push(exe);
    }
    state.checked = false;
    state.error.clear();
    state.scan = Some(IoTaskPool::get().spawn(async move {
        let store = Store::system()?;
        store.prepare()?;
        let found = store.discover(&roots);
        let resumable = store.root.join("Hunyuan3D-2mini").exists();
        Ok((store, found, resumable))
    }));
}

pub(super) fn update(
    mut events: MessageReader<Action>,
    editor: Res<Editor>,
    mut state: ResMut<State>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    if editor.generation.active && !state.checked && state.scan.is_none() && state.job.is_none() {
        start_scan(&mut state, editor.root.clone());
    }
    for event in events.read() {
        let Action::Generation(action) = event else {
            continue;
        };
        if editor.pending.is_some() || editor.browser_popup || editor.hdri_library_open {
            continue;
        }
        match action {
            GenerationAction::Section(Section::Installation) => {
                state.details_open = !state.details_open
            }
            GenerationAction::Toggle
                if editor.generation.active && state.scan.is_none() && state.job.is_none() =>
            {
                start_scan(&mut state, editor.root.clone())
            }
            GenerationAction::Open | GenerationAction::CheckModel
                if state.scan.is_none() && state.job.is_none() =>
            {
                start_scan(&mut state, editor.root.clone())
            }
            GenerationAction::DownloadModel
                if editor.generation.active
                    && state.checked
                    && state.scan.is_none()
                    && state.job.is_none()
                    && state.found.is_none() =>
            {
                if let Some(store) = state.store.clone() {
                    state.error.clear();
                    let progress = Arc::new(Progress::default());
                    let worker = progress.clone();
                    state.job = Some(Job {
                        progress,
                        task: IoTaskPool::get().spawn(async move { store.download(&worker) }),
                    });
                }
            }
            GenerationAction::CancelModelDownload => {
                if let Some(job) = &state.job {
                    job.progress.cancel();
                }
            }
            _ => {}
        }
    }
    if let Some(result) = state
        .scan
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        state.scan = None;
        state.checked = true;
        match result {
            Ok((store, found, resumable)) => {
                state.store = Some(store);
                state.found = found;
                state.resumable = resumable;
            }
            Err(error) => state.error = error,
        }
    }
    if let Some(result) = state
        .job
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)))
    {
        if let Some(job) = &state.job {
            state.resumable = job.progress.received.load(Ordering::Relaxed) > 0;
        }
        state.job = None;
        match result {
            Ok(path) => {
                state.found = Some(path);
                state.error.clear();
            }
            Err(error) => state.error = error,
        }
    }
    if state.scan.is_some() || state.job.is_some() {
        redraw.write(RequestRedraw);
    }
}

pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    state: Res<State>,
    ui: Res<InstallUi>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    children: Query<&Children>,
    mut folds: Query<(&FoldIcon, &mut UiTransform)>,
) {
    let missing = state.found.is_none();
    let checking = state.scan.is_some() || !state.checked;
    let downloading = state.job.is_some();
    let phase = state
        .job
        .as_ref()
        .map(|job| job.progress.phase.load(Ordering::Relaxed));
    for (entity, shown) in [
        (ui.normal, !missing && editor.generation.backend_ready),
        (ui.panel, missing),
        (
            ui.download,
            missing && !checking && !downloading && state.store.is_some(),
        ),
        (ui.retry, missing && !checking && !downloading),
        (ui.progress, downloading || checking),
        (ui.cancel, downloading),
        (ui.amount, downloading && phase == Some(0)),
        (ui.details, state.details_open),
        (ui.description, !checking && !downloading),
        (ui.error, !state.error.is_empty()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    for (FoldIcon(section), mut icon) in &mut folds {
        if *section == Section::Installation {
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
        if let Ok(mut text) = labels.get_mut(entity) {
            if text.0 != value {
                text.0 = value.into();
            }
        }
    };
    set(
        ui.badge,
        if !missing {
            if editor.generation.running {
                "Генерация"
            } else if editor.generation.backend_ready {
                "Готово"
            } else {
                if editor.generation.engine_loading {
                    "Запуск модели"
                } else {
                    "Выключено"
                }
            }
        } else if downloading {
            "Скачивание модели"
        } else if checking {
            "Проверка модели"
        } else {
            "Модель не установлена"
        },
    );
    set(
        ui.title,
        if checking {
            "Проверка модели…"
        } else if downloading {
            "Скачивание модели"
        } else {
            "Модель отсутствует"
        },
    );
    set(
        ui.description,
        if checking {
            "Ищем Hunyuan3D-2mini на этом компьютере."
        } else if downloading {
            "Можно переключить вкладку — скачивание продолжится."
        } else {
            "Hunyuan3D-2mini · 3,82 ГБ"
        },
    );
    set(ui.error, &state.error);
    if let Some(store) = &state.store {
        set(
            ui.location,
            &format!(
                "Папка моделей: {}\nИсточник: Tencent · Hugging Face",
                store.root.display()
            ),
        );
    }
    let download_caption = if state.resumable {
        "Продолжить скачивание"
    } else {
        "Скачать · 3,82 ГБ"
    };
    commands
        .entity(ui.download)
        .insert(AccessibleLabel(download_caption.into()));
    if let Ok(children) = children.get(ui.download) {
        for child in children.iter() {
            set(child, download_caption);
        }
    }
    if let Some(job) = &state.job {
        let received = job
            .progress
            .received
            .load(Ordering::Relaxed)
            .min(model::TOTAL);
        let ratio = received as f64 / model::TOTAL as f64;
        super::progress::set(
            &mut commands,
            ui.fill,
            if !editor.generation.active {
                super::progress::Value::Hidden
            } else if phase == Some(0) {
                super::progress::Value::Fraction(ratio as f32)
            } else {
                super::progress::Value::Busy
            },
        );
        set(
            ui.amount,
            &format!(
                "{:.1}% · {:.1} МБ / {:.2} ГБ",
                ratio * 100.0,
                received as f64 / 1_000_000.0,
                model::TOTAL as f64 / 1_000_000_000.0
            ),
        );
        let phase = job.progress.phase.load(Ordering::Relaxed);
        set(
            ui.phase,
            match phase {
                1 => "Проверка файлов…",
                2 => "Отмена скачивания…",
                _ => "Скачивание Hunyuan3D-2mini…",
            },
        );
        if job.progress.cancelled.load(Ordering::Relaxed) {
            commands.entity(ui.cancel).insert(InteractionDisabled);
        }
    } else {
        set(ui.phase, "Поиск файлов модели…");
        super::progress::set(
            &mut commands,
            ui.fill,
            if checking && editor.generation.active {
                super::progress::Value::Busy
            } else {
                super::progress::Value::Hidden
            },
        );
    }
}

#[cfg(test)]
#[path = "generation_install_tests.rs"]
mod tests;
