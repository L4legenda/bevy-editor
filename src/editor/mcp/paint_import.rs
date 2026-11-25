//! Prepare an MPP on a worker and publish it only while the request is live.
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use tempfile::{Builder, NamedTempFile};

use super::{
    Editor, Job, PaintImportArgs, PaintOpenArgs, Payload, editing, json, open_paint_model,
};
use crate::editor::{
    browser::Browser,
    browser_files, model_paint_mpp,
    model_paint_spp::{ImportDetails, project_output_name},
    thumbnails::Thumbnails,
};

struct Staged {
    file: NamedTempFile,
    name: String,
    report: ImportDetails,
}

pub(super) struct Pending {
    job: Job,
    root: PathBuf,
    folder: PathBuf,
    destination: PathBuf,
    task: Task<io::Result<Staged>>,
}

pub(super) fn start(slot: &mut Option<Pending>, editor: &Editor, args: PaintImportArgs, job: Job) {
    let result = (|| {
        if slot.is_some() {
            return Err("Wait for the current paint project import to finish".into());
        }
        if !args.source.is_absolute()
            || !args
                .source
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mpp"))
        {
            return Err("Choose an absolute path to a Material Painter .mpp file".into());
        }
        let folder = args.folder.unwrap_or_default();
        if !crate::editor::browser::discovers_models_in(&folder) {
            return Err("Choose a destination folder indexed by the project asset browser".into());
        }
        let destination =
            browser_files::directory(&editor.root, &folder).map_err(|error| error.to_string())?;
        let target = destination.clone();
        let task = IoTaskPool::get().spawn(async move {
            let source = args.source.canonicalize()?;
            let (name, bytes, report) = model_paint_mpp::prepare_conversion(&source, &target)?;
            let mut file = Builder::new()
                .prefix(".mpp-import-")
                .suffix(".tmp")
                .tempfile_in(&target)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            Ok(Staged { file, name, report })
        });
        Ok((folder, destination, task))
    })();
    match result {
        Ok((folder, destination, task)) => {
            *slot = Some(Pending {
                job,
                root: editor.root.clone(),
                folder,
                destination,
                task,
            });
        }
        Err(error) => {
            let _ = job.reply.send(Err(error));
        }
    }
}

pub(super) fn poll(
    slot: &mut Option<Pending>,
    editor: &mut Editor,
    editing: &mut editing::State,
    browser: &mut Browser,
    previews: Option<&mut Thumbnails>,
    busy: bool,
) {
    let Some(pending) = slot.as_mut() else { return };
    if pending.job.reply.is_closed() {
        *slot = None;
        return;
    }
    if busy
        || editor.pending.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
        || editing.gesture.is_some()
        || !editor.ready
    {
        return;
    }
    let Some(result) = block_on(poll_once(&mut pending.task)) else {
        return;
    };
    let pending = slot.take().unwrap();
    if pending.job.reply.is_closed() {
        return;
    }
    let reply = (|| {
        if editor.root != pending.root
            || browser_files::directory(&editor.root, &pending.folder)
                .ok()
                .as_ref()
                != Some(&pending.destination)
        {
            return Err("The project or import destination changed while reading the MPP".into());
        }
        let Staged { file, name, report } = result.map_err(|error| error.to_string())?;
        let filename =
            publish(file, &pending.destination, &name).map_err(|error| error.to_string())?;
        browser.refresh(&mut editor.project);
        if let Some(previews) = previews {
            previews.extend(editor.project.catalog.len());
        }
        let path = pending.folder.join(filename);
        let asset = editor
            .project
            .catalog
            .iter()
            .find(|item| Path::new(&item.asset) == path)
            .map(|item| item.asset.clone())
            .ok_or_else(|| {
                format!(
                    "The imported GLB is outside the asset index: {}",
                    path.display()
                )
            })?;
        let mut opened = open_paint_model(editor, editing, PaintOpenArgs { asset })?;
        opened["report"] = json!({"label": report.label, "overview": report.overview, "warnings": report.warnings});
        Ok(Payload::Json(opened))
    })();
    let _ = pending.job.reply.send(reply);
}

fn publish(mut file: NamedTempFile, target: &Path, name: &str) -> io::Result<String> {
    for number in 1.. {
        let filename = project_output_name(name, "MPP", number);
        match file.persist_noclobber(target.join(&filename)) {
            Ok(_) => return Ok(filename),
            Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => file = err.file,
            Err(err) => return Err(err.error),
        }
    }
    unreachable!()
}
