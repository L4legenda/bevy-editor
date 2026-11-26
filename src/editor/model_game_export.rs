//! Export a playable GLB without changing the open model or its editable source.
use super::*;
use crate::editor::{
    browser_files,
    model_document::{Document, glb::Glb, source_path},
};
use bevy::{
    tasks::{AsyncComputeTaskPool, IoTaskPool, Task, block_on, poll_once},
    ui::InteractionDisabled,
};
use std::path::{Path, PathBuf};

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct ExportButton;

pub(in crate::editor::ui::model_viewer) fn setup(
    commands: &mut Commands,
    toolbar: Entity,
    font: &Handle<Font>,
) {
    let button = super::super::super::button(
        commands,
        toolbar,
        font,
        "Игровой GLB…",
        Action::ModelView(ModelViewAction::ExportGame),
        Look::Regular,
    );
    commands
        .entity(button)
        .insert((ExportButton, InteractionDisabled));
}

struct Target {
    model: usize,
    root: PathBuf,
    asset: String,
    path: PathBuf,
}

impl Target {
    fn matches(&self, editor: &Editor, active: Option<usize>) -> bool {
        active == Some(self.model)
            && editor.project.asset_root == self.root
            && editor
                .project
                .catalog
                .get(self.model)
                .is_some_and(|model| model.asset == self.asset)
    }
}

struct Report {
    path: PathBuf,
    source_bytes: usize,
    export_bytes: usize,
}

enum Work {
    Choose(Task<std::io::Result<Option<PathBuf>>>),
    Wait(PathBuf),
    Write(Task<Result<Report, String>>),
}

struct Job {
    target: Target,
    work: Work,
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct State {
    job: Option<Job>,
    #[cfg(test)]
    pub(super) test_path: Option<PathBuf>,
}

fn publish(
    mut requested: PathBuf,
    export: impl FnOnce(&Path) -> Result<(Vec<u8>, usize), String>,
) -> Result<Report, String> {
    if requested.extension().is_none() {
        requested.set_extension("glb");
    }
    if !requested
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
    {
        return Err("Используйте расширение .glb".into());
    }
    let name = requested
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Недопустимое имя GLB")?;
    browser_files::validate_name(name)?;
    let parent = requested
        .parent()
        .ok_or("Укажите папку для экспорта")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let path = parent.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => return Err("Файл с таким именем уже существует".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let (bytes, source_bytes) = export(&parent)?;
    save_copy::publish(&path, &bytes).map_err(|error| error.to_string())?;
    Ok(Report {
        path,
        source_bytes,
        export_bytes: bytes.len(),
    })
}

fn writable(
    model: usize,
    editing: &editing::State,
    paint: &PaintState,
    runtime: &engine::Runtime,
) -> bool {
    !editing.saving_model(model)
        && editing.gesture.is_none()
        && !runtime.stroke_active()
        && !editing
            .documents
            .get(&model)
            .is_some_and(|doc| doc.paint_busy || doc.paint_pending)
        && !paint
            .sessions
            .get(&model)
            .is_some_and(|session| session.busy || session.smart_loading)
}

fn report(editor: &mut Editor, editing: &mut editing::State, message: String) {
    editing.status.clone_from(&message);
    editor.status = message;
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut commands: Commands,
    mut messages: MessageReader<Action>,
    mut state: ResMut<State>,
    mut editor: ResMut<Editor>,
    viewer: Res<Viewer>,
    mut editing: ResMut<editing::State>,
    paint: Res<PaintState>,
    runtime: Res<engine::Runtime>,
    buttons: Query<(Entity, Has<InteractionDisabled>), With<ExportButton>>,
) {
    let requested = messages
        .read()
        .filter(|action| matches!(action, Action::ModelView(ModelViewAction::ExportGame)))
        .count()
        > 0;
    let active = editor.viewing_model.filter(|&model| {
        editor.pending.is_none()
            && editor.viewing_image.is_none()
            && !editor.browser_popup
            && !editor.hdri_library_open
            && viewer.model == Some(model)
            && viewer.root.is_some()
            && editor.project.catalog.get(model).is_some_and(|entry| {
                crate::data::ModelFormat::from_path(&entry.asset)
                    == Some(crate::data::ModelFormat::Glb)
            })
    });

    if let Some(mut job) = state.job.take() {
        if let Work::Choose(task) = &mut job.work
            && let Some(result) = block_on(poll_once(task))
        {
            match result {
                Ok(Some(path)) => job.work = Work::Wait(path),
                Ok(None) => return,
                Err(error) => {
                    report(
                        &mut editor,
                        &mut editing,
                        format!("Не удалось выбрать файл для игрового GLB · {error}"),
                    );
                    return;
                }
            }
        }
        if let Work::Wait(path) = &job.work {
            if !job.target.matches(&editor, active) {
                report(
                    &mut editor,
                    &mut editing,
                    "Экспорт отменён · активная модель изменилась".into(),
                );
                return;
            }
            if writable(job.target.model, &editing, &paint, &runtime) {
                let path = path.clone();
                let document = editing.documents.get(&job.target.model);
                match start_write(document, job.target.path.clone(), path) {
                    Ok(task) => {
                        job.work = Work::Write(task);
                        report(&mut editor, &mut editing, "Экспорт игрового GLB…".into());
                    }
                    Err(error) => {
                        report(
                            &mut editor,
                            &mut editing,
                            format!("Не удалось подготовить игровой GLB · {error}"),
                        );
                        return;
                    }
                }
            }
        }
        if let Work::Write(task) = &mut job.work
            && let Some(result) = block_on(poll_once(task))
        {
            let message = match result {
                Ok(report) => format!(
                    "Игровой GLB сохранён · {:.1} → {:.1} КиБ · {}",
                    report.source_bytes as f64 / 1024.0,
                    report.export_bytes as f64 / 1024.0,
                    report.path.display(),
                ),
                Err(error) => format!("Не удалось экспортировать игровой GLB · {error}"),
            };
            report(&mut editor, &mut editing, message);
        } else {
            state.job = Some(job);
        }
    } else if requested
        && let Some(model) = active
        && writable(model, &editing, &paint, &runtime)
    {
        let asset = editor.project.catalog[model].asset.clone();
        let root = editor.project.asset_root.clone();
        match source_path(&root, &asset) {
            Ok(path) => {
                let target = Target {
                    model,
                    root,
                    asset,
                    path,
                };
                #[cfg(test)]
                let selected = state.test_path.take();
                #[cfg(not(test))]
                let selected: Option<PathBuf> = None;
                let work = selected.map_or_else(
                    || {
                        let source = target.path.clone();
                        Work::Choose(IoTaskPool::get().spawn(async move {
                            let parent = source
                                .parent()
                                .ok_or_else(|| std::io::Error::other("У модели нет папки"))?;
                            let stem = source
                                .file_stem()
                                .and_then(|stem| stem.to_str())
                                .unwrap_or("Модель");
                            save_copy::choose_glb_path(
                                parent,
                                &format!("{stem}.game.glb"),
                                "Экспорт игрового GLB",
                            )
                        }))
                    },
                    Work::Wait,
                );
                state.job = Some(Job { target, work });
            }
            Err(error) => report(
                &mut editor,
                &mut editing,
                format!("Не удалось найти исходный GLB · {error}"),
            ),
        }
    }

    let enabled = state.job.is_none()
        && active.is_some_and(|model| writable(model, &editing, &paint, &runtime));
    for (button, disabled) in &buttons {
        if enabled && disabled {
            commands.entity(button).remove::<InteractionDisabled>();
        } else if !enabled && !disabled {
            commands.entity(button).insert(InteractionDisabled);
        }
    }
}

fn start_write(
    document: Option<&Document>,
    source: PathBuf,
    requested: PathBuf,
) -> Result<Task<Result<Report, String>>, String> {
    let snapshot = document
        .map(|doc| {
            doc.prepare_game_export()
                .map(|export| (export, doc.source.data().len()))
        })
        .transpose()?;
    Ok(AsyncComputeTaskPool::get().spawn(async move {
        publish(requested, |parent| {
            if let Some((write, size)) = snapshot {
                return Ok((write(parent)?, size));
            }
            let raw = Glb::load(&source)?;
            Ok((raw.game_export_to(parent)?, raw.data().len()))
        })
    }))
}
