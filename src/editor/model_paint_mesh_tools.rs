use super::*;
use crate::editor::{model_document::Object, model_import_mesh};
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use std::path::PathBuf;

pub(in crate::editor::ui::model_viewer) struct Import {
    model: usize,
    document: PathBuf,
    task: Task<Result<Option<Vec<Object>>, String>>,
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut messages: MessageReader<Action>,
    mut import: Local<Option<Import>>,
    mut editor: ResMut<Editor>,
    mut editing: ResMut<editing::State>,
    mut paint: ResMut<PaintState>,
    mut runtime: ResMut<engine::Runtime>,
    mut baked: ResMut<bake::State>,
    mut queue: ResMut<engine::gpu::Queue>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut viewer: ResMut<Viewer>,
    uv: Res<editing::uv::UvState>,
) {
    let commands = messages.read().filter_map(|message| match message {
        Action::ModelView(ModelViewAction::Paint(PaintAction::MeshReplacement(command))) => {
            Some(*command)
        }
        _ => None,
    });
    let active = editor.viewing_model.filter(|index| {
        editor.pending.is_none()
            && editor.viewing_image.is_none()
            && editing
                .documents
                .get(index)
                .is_some_and(|doc| doc.mode == Mode::Paint)
    });
    if import.as_ref().is_some_and(|job| {
        active != Some(job.model)
            || editing
                .documents
                .get(&job.model)
                .is_none_or(|doc| doc.source.path != job.document)
    }) && let Some(job) = import.take()
        && let Some(session) = paint.sessions.get_mut(&job.model)
    {
        session.mesh_loading = false;
        session.revision += 1;
    }
    let Some(index) = active else { return };
    let saving = editing.saving_model(index);
    let Some(doc) = editing.documents.get_mut(&index) else {
        return;
    };
    let Some(session) = paint.sessions.get_mut(&index) else {
        return;
    };
    let idle = !saving
        && !doc.paint_busy
        && !doc.paint_pending
        && !runtime.stroke_active()
        && !session.busy
        && !session.smart_loading;
    let result = if idle {
        import
            .as_mut()
            .and_then(|job| block_on(poll_once(&mut job.task)))
    } else {
        None
    };
    let mut replacement = None;
    if let Some(result) = result {
        *import = None;
        session.mesh_loading = false;
        session.revision += 1;
        match result {
            Ok(Some(objects)) => replacement = Some((objects, false)),
            Ok(None) => {}
            Err(error) => editor.status = format!("Не удалось загрузить сетку · {error}"),
        }
    }
    let mut restored = false;
    let mut new_geometry = false;
    for command in commands {
        if editor.browser_popup || editor.hdri_library_open {
            continue;
        }
        if command == MeshCommand::Toggle {
            session.mesh_menu = !session.mesh_menu;
            session.revision += 1;
            continue;
        }
        if !idle || import.is_some() {
            editor.status = "Дождитесь завершения текущей операции".into();
            continue;
        }
        match command {
            MeshCommand::Primitive(shape) => match paint_mesh_replacement::primitive(shape) {
                Ok(objects) => replacement = Some((objects, true)),
                Err(error) => editor.status = format!("Не удалось создать сетку · {error}"),
            },
            MeshCommand::Import => {
                let directory = editor.project.asset_root.clone();
                let unwrap = uv.auto_unwrap();
                let task = IoTaskPool::get().spawn(async move {
                    crate::editor::asset_monitor::choose_file_path(&directory)
                        .map_err(|error| error.to_string())?
                        .map(|path| model_import_mesh::replacement(&path, unwrap))
                        .transpose()
                });
                *import = Some(Import {
                    model: index,
                    document: doc.source.path.clone(),
                    task,
                });
                session.mesh_loading = true;
                session.revision += 1;
            }
            MeshCommand::Undo if doc.can_restore_paint_mesh(false) => {
                doc.undo();
                doc.auto_bake_pending = false;
                editor.status = "Замена сетки отменена".into();
                restored = true;
            }
            MeshCommand::Redo if doc.can_restore_paint_mesh(true) => {
                doc.redo();
                doc.auto_bake_pending = uv.bake_on_load();
                editor.status = "Замена сетки повторена".into();
                restored = true;
                new_geometry = true;
            }
            _ => {}
        }
        break;
    }
    if let Some((objects, use_active_set)) = replacement {
        match doc.replace_paint_mesh(
            objects,
            use_active_set,
            uv.detect_udim_on_import(),
            uv.bake_on_load(),
            &mut materials,
        ) {
            Ok(report) => {
                editor.status = format!(
                    "Сетка заменена · Перенесено наборов: {}",
                    report.transferred
                );
                if report.omitted > 0 {
                    editor
                        .status
                        .push_str(&format!(" · Доступно через отмену: {}", report.omitted));
                }
                restored = true;
                new_geometry = true;
            }
            Err(error) => editor.status = format!("Не удалось заменить сетку · {error}"),
        }
    }
    if restored {
        baked.cancel_mesh_work(index);
        runtime.discard_mesh(index, &mut queue);
        session.binding = None;
        session.set = doc.paint_set_index();
        session.set_menu = false;
        session.set_appearance = default();
        session.uv_view = default();
        session.undo.clear();
        session.redo.clear();
        session.histories.clear();
        session.queued.clear();
        session.path_count = 0;
        session.path_command = None;
        session.folder_view = None;
        session.graph_surfaces.clear();
        session.mesh_graph_refresh.clear();
        if new_geometry {
            session.mesh_graph_refresh.extend(
                doc.objects.iter().flat_map(|object| {
                    (0..object.materials.len()).map(move |slot| (object.id, slot))
                }),
            );
        }
        session.graph_bake = None;
        session.bake_view = None;
        session.bake_fix = None;
        session.bake_inspect_cage = false;
        session.bake_paint_cage = false;
        session.bake_custom_gizmo = None;
        session.bake_ready = false;
        session.bake_option_edit = None;
        session.preview_3d = Preview3d::Material;
        session.revision += 1;
        viewer.fitted = false;
        viewer.restore = None;
        viewer.pending_document_camera = false;
    }
}
