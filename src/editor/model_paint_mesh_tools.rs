use super::*;
use crate::editor::{model_document::Object, model_import_mesh};
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use std::path::PathBuf;

pub(in crate::editor) enum Operation {
    Primitive(paint_mesh_replacement::Shape),
    Import(Option<PathBuf>),
    Undo,
    Redo,
}

pub(in crate::editor) enum Outcome {
    Replaced(paint_mesh_replacement::Report),
    Undone,
    Redone,
}

pub(in crate::editor) trait Reply: Send + Sync {
    fn is_closed(&self) -> bool;
    fn finish(self: Box<Self>, result: Result<Outcome, String>);
}

type Completion = Box<dyn Reply>;

pub(in crate::editor) struct Request {
    pub model: usize,
    pub set: usize,
    pub revision: u64,
    pub operation: Operation,
    pub reply: Completion,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct Requests(pub Option<Request>);

pub(in crate::editor::ui::model_viewer) struct Import {
    model: usize,
    document: PathBuf,
    task: Task<Result<Option<Vec<Object>>, String>>,
    reply: Option<Completion>,
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut messages: MessageReader<Action>,
    mut requests: ResMut<Requests>,
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
            || job.reply.as_ref().is_some_and(|reply| reply.is_closed())
            || editing
                .documents
                .get(&job.model)
                .is_none_or(|doc| doc.source.path != job.document)
    }) && let Some(job) = import.take()
    {
        if let Some(session) = paint.sessions.get_mut(&job.model) {
            session.mesh_loading = false;
            session.revision += 1;
        }
        if let Some(reply) = job.reply {
            reply.finish(Err("The model changed during mesh import".into()));
        }
    }

    let request = requests.0.take().filter(|request| !request.reply.is_closed());
    let saving = active.is_some_and(|index| editing.saving_model(index));
    let Some((index, doc, session)) = active.and_then(|index| {
        let doc = editing.documents.get_mut(&index)?;
        let session = paint.sessions.get_mut(&index)?;
        Some((index, doc, session))
    }) else {
        if let Some(request) = request {
            request.reply.finish(Err("There is no active painting workspace".into()));
        }
        return;
    };
    let idle = !saving
        && !doc.paint_busy
        && !doc.paint_pending
        && !runtime.stroke_active()
        && !session.busy
        && !session.smart_loading
        && session.resize_request.is_none();
    let imported = if idle {
        import.as_mut().and_then(|job| block_on(poll_once(&mut job.task)))
    } else {
        None
    };
    let mut result = None;
    let mut replacement = None;
    let mut reply = None;
    if let Some(imported) = imported {
        reply = import.take().and_then(|job| job.reply);
        session.mesh_loading = false;
        session.revision += 1;
        match imported {
            Ok(Some(objects)) => replacement = Some((objects, false)),
            Ok(None) => {}
            Err(error) => {
                editor.status = format!("Не удалось загрузить сетку · {error}");
                result = Some(Err(error));
            }
        }
    }

    let busy = !idle || import.is_some() || replacement.is_some() || result.is_some();
    let blocked = editor.browser_popup || editor.hdri_library_open;
    let requested = request.and_then(|request| {
        if request.model != index
            || request.set != session.set
            || request.revision != session.revision
        {
            request.reply.finish(Err("The paint workspace changed; call get_paint_state again".into()));
            None
        } else if blocked || busy {
            request.reply.finish(Err("Finish the current paint, mesh or dialog operation first".into()));
            None
        } else {
            Some((request.operation, Some(request.reply)))
        }
    });
    let command = requested.or_else(|| {
        if blocked {
            return None;
        }
        for command in commands {
            if command == MeshCommand::Toggle {
                session.mesh_menu = !session.mesh_menu;
                session.revision += 1;
                continue;
            }
            if busy {
                editor.status = "Дождитесь завершения текущей операции".into();
                continue;
            }
            let operation = match command {
                MeshCommand::Primitive(shape) => Operation::Primitive(shape),
                MeshCommand::Import => Operation::Import(None),
                MeshCommand::Undo => Operation::Undo,
                MeshCommand::Redo => Operation::Redo,
                MeshCommand::Toggle => unreachable!(),
            };
            return Some((operation, None));
        }
        None
    });
    if let Some((operation, completion)) = command {
        reply = completion;
        match operation {
            Operation::Primitive(shape) => match paint_mesh_replacement::primitive(shape) {
                Ok(objects) => replacement = Some((objects, true)),
                Err(error) => {
                    editor.status = format!("Не удалось создать сетку · {error}");
                    result = Some(Err(error));
                }
            },
            Operation::Import(path) => {
                let directory = editor.project.asset_root.clone();
                let unwrap = uv.auto_unwrap();
                let task = IoTaskPool::get().spawn(async move {
                    let path = match path {
                        Some(path) => Some(path),
                        None => crate::editor::asset_monitor::choose_file_path(&directory)
                            .map_err(|error| error.to_string())?,
                    };
                    path.map(|path| model_import_mesh::replacement(&path, unwrap)).transpose()
                });
                *import = Some(Import {
                    model: index,
                    document: doc.source.path.clone(),
                    task,
                    reply: reply.take(),
                });
                session.mesh_loading = true;
                session.revision += 1;
            }
            Operation::Undo if doc.can_restore_paint_mesh(false) => {
                doc.undo();
                doc.auto_bake_pending = false;
                editor.status = "Замена сетки отменена".into();
                result = Some(Ok(Outcome::Undone));
            }
            Operation::Redo if doc.can_restore_paint_mesh(true) => {
                doc.redo();
                doc.auto_bake_pending = uv.bake_on_load();
                editor.status = "Замена сетки повторена".into();
                result = Some(Ok(Outcome::Redone));
            }
            _ => result = Some(Err("No mesh replacement is available to undo or redo".into())),
        }
    }
    if let Some((objects, use_active_set)) = replacement {
        if reply.as_ref().is_some_and(|reply| reply.is_closed()) {
            return;
        }
        result = Some(match doc.replace_paint_mesh(
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
                Ok(Outcome::Replaced(report))
            }
            Err(error) => {
                editor.status = format!("Не удалось заменить сетку · {error}");
                Err(error)
            }
        });
    }
    if let Some(Ok(outcome)) = result.as_ref() {
        let new_geometry = !matches!(outcome, Outcome::Undone);
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
        #[cfg(feature = "mcp")]
        {
            session.surface_selection = None;
        }
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
    if let (Some(reply), Some(result)) = (reply, result) {
        reply.finish(result);
    }
}
