//! Save the current editable model into a new asset, including work not saved to the source file.
use super::*;
use crate::editor::{
    browser::Browser,
    browser_files, file_moves,
    model_document::{Document, Object, glb::Glb},
    thumbnails::Thumbnails,
};
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task, block_on, poll_once};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

struct Snapshot {
    source: Glb,
    objects: Vec<Object>,
    original: Vec<Object>,
    custom: Arc<Vec<custom::Recipe>>,
    settings: crate::editor::model_bake_settings::Settings,
    cage: Arc<crate::editor::model_cage_data::Edits>,
}

impl Snapshot {
    fn capture(doc: &Document) -> Result<Self, String> {
        doc.validate()?;
        Ok(Self {
            source: doc.source_for_save(),
            objects: doc.objects.clone(),
            original: doc.original.clone(),
            custom: doc.custom_bakes.clone(),
            settings: doc.bake_settings.clone(),
            cage: doc.cage_edits.clone(),
        })
    }

    fn write(mut self, asset_root: &Path, requested: &Path) -> Result<PathBuf, String> {
        let root = asset_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let parent = self
            .source
            .path
            .parent()
            .ok_or("У модели нет папки")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let requested_parent = requested
            .parent()
            .ok_or("Укажите папку копии")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        // GLB may refer to external textures and other resources by relative URI.
        if requested_parent != parent || !parent.starts_with(&root) {
            return Err("Сохраните копию в папке исходной модели внутри проекта".into());
        }
        let mut destination = requested.to_owned();
        if destination.extension().is_none() {
            destination.set_extension("glb");
        }
        if !destination
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
        {
            return Err("Используйте расширение .glb".into());
        }
        let name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Недопустимое имя GLB")?;
        browser_files::validate_name(name)?;
        let absolute = parent.join(name);
        let relative = absolute
            .strip_prefix(&root)
            .map_err(|error| error.to_string())?
            .to_owned();
        let redirects =
            crate::asset_paths::Redirects::read(&root).map_err(|error| error.to_string())?;
        for path in [&relative, &file_moves::meta_path(&relative)] {
            if redirects.resolve(path).map_err(|error| error.to_string())? != *path {
                return Err("Это имя уже используется в проекте".into());
            }
            match std::fs::symlink_metadata(root.join(path)) {
                Ok(_) => return Err("Файл с таким именем уже существует".into()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        // The copy stays in the original directory, so the original path gives
        // the metadata encoder the same relative base as a regular save.
        editing::write_model_metadata(&mut self.source, &self.custom, &self.settings)?;
        let bytes =
            self.source
                .encode_with_cage(&self.objects, &self.original, Some(&self.cage))?;
        publish(&absolute, &bytes).map_err(|error| error.to_string())?;
        Ok(relative)
    }
}

#[cfg(feature = "mcp")]
pub(super) fn prepare_copy(doc: &Document, root: &Path, name: &str) -> Result<impl FnOnce() -> Result<PathBuf, String> + Send + 'static, String> {
    browser_files::validate_name(name)?;
    let path = doc.source.path.with_file_name(name);
    if !path.extension().is_none_or(|ext| ext.eq_ignore_ascii_case("glb")) {
        return Err("Используйте расширение .glb".into());
    }
    let snapshot = Snapshot::capture(doc)?;
    let root = root.to_owned();
    Ok(move || snapshot.write(&root, &path).map(|relative| root.join(relative)))
}

fn publish(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let staged = destination.with_file_name(format!(
        ".bevy-editor-model-{}.tmp",
        bevy::asset::uuid::Uuid::new_v4()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let result = written.and_then(|_| file_moves::move_no_replace(&staged, destination));
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

fn choose_path(source: &Path) -> io::Result<Option<PathBuf>> {
    let folder = source
        .parent()
        .ok_or_else(|| io::Error::other("У модели нет папки"))?;
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Модель");
    let filename = format!("{stem} — копия.glb");
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        Command::new("powershell.exe")
            .creation_flags(0x08000000)
            .current_dir(folder)
            .env("BEVY_EDITOR_MODEL_COPY_NAME", &filename)
            .args(["-NoProfile", "-STA", "-Command", concat!(
                "[Console]::OutputEncoding=[Text.Encoding]::UTF8; ",
                "Add-Type -AssemblyName System.Windows.Forms; ",
                "$d=New-Object Windows.Forms.SaveFileDialog; ",
                "$d.Title='Сохранить копию в папке модели'; $d.InitialDirectory=(Get-Location).Path; ",
                "$d.Filter='GLB (*.glb)|*.glb'; $d.DefaultExt='glb'; $d.OverwritePrompt=$false; ",
                "$d.FileName=$env:BEVY_EDITOR_MODEL_COPY_NAME; ",
                "if($d.ShowDialog() -eq 'OK'){[Console]::Write($d.FileName)}; $d.Dispose()",
            )])
            .output()?
    };
    #[cfg(target_os = "macos")]
    let output = Command::new("osascript").args(["-e", concat!(
        "on run argv\ntry\n",
        "return POSIX path of (choose file name with prompt \"Save a copy in the model folder\" ",
        "default name (item 1 of argv) default location (POSIX file (item 2 of argv)))\n",
        "on error number -128\nreturn \"\"\nend try\nend run",
    ), &filename]).arg(folder).output()?;
    #[cfg(not(any(windows, target_os = "macos")))]
    let output = Command::new("zenity")
        .args([
            "--file-selection",
            "--save",
            "--title=Сохранить копию в папке модели",
            "--file-filter=GLB | *.glb",
        ])
        .arg(format!("--filename={}", folder.join(&filename).display()))
        .output()
        .or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Command::new("kdialog")
                    .arg("--getsavefilename")
                    .arg(folder.join(&filename))
                    .arg("*.glb")
                    .output()
            } else {
                Err(error)
            }
        })?;
    if !output.status.success() {
        return if output.status.code() == Some(1) && output.stderr.is_empty() {
            Ok(None)
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        };
    }
    let path = String::from_utf8(output.stdout).map_err(io::Error::other)?;
    let path = path.trim_end_matches(['\r', '\n']);
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

enum Work {
    Choose(Task<io::Result<Option<PathBuf>>>),
    Waiting(PathBuf),
    Write(Task<Result<PathBuf, String>>),
}

struct Job {
    model: usize,
    root: PathBuf,
    source: PathBuf,
    work: Work,
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct State {
    job: Option<Job>,
    activate: Option<usize>,
    #[cfg(test)]
    pub(super) test_path: Option<PathBuf>,
}

pub(in crate::editor::ui::model_viewer) fn update(
    mut messages: MessageReader<Action>,
    mut state: ResMut<State>,
    mut editor: ResMut<Editor>,
    mut editing: ResMut<editing::State>,
    paint: Res<PaintState>,
    runtime: Res<engine::Runtime>,
    mut browser: ResMut<Browser>,
    mut previews: ResMut<Thumbnails>,
) {
    let requested = messages
        .read()
        .filter(|action| {
            matches!(
                action,
                Action::ModelView(ModelViewAction::Paint(PaintAction::SaveCopy))
            )
        })
        .count()
        > 0;
    let active = editor
        .viewing_model
        .filter(|_| editor.pending.is_none() && editor.viewing_image.is_none());
    if let Some(index) = state.activate {
        if active != Some(index) {
            state.activate = None;
        } else if let Some(doc) = editing.documents.get_mut(&index) {
            doc.set_mode(Mode::Paint);
            state.activate = None;
        }
    }
    if let Some(mut job) = state.job.take() {
        let mut keep = true;
        if let Work::Choose(task) = &mut job.work
            && let Some(result) = block_on(poll_once(task))
        {
            match result {
                Ok(Some(path)) => job.work = Work::Waiting(path),
                Ok(None) => keep = false,
                Err(error) => {
                    editor.status = format!("Не удалось выбрать имя копии · {error}");
                    keep = false;
                }
            }
        }
        if let Work::Waiting(path) = &job.work {
            let doc = editing.documents.get(&job.model).filter(|doc| {
                active == Some(job.model)
                    && doc.mode == Mode::Paint
                    && doc.source.path == job.source
                    && editor.project.asset_root == job.root
            });
            match doc {
                Some(doc)
                    if !doc.paint_busy
                        && !doc.paint_pending
                        && !runtime.stroke_active()
                        && !paint
                            .sessions
                            .get(&job.model)
                            .is_some_and(|session| session.smart_loading)
                        && !editing.saving_model(job.model) =>
                {
                    match Snapshot::capture(doc) {
                        Ok(snapshot) => {
                            let root = job.root.clone();
                            let path = path.clone();
                            job.work = Work::Write(
                                AsyncComputeTaskPool::get()
                                    .spawn(async move { snapshot.write(&root, &path) }),
                            );
                            editor.status = "Сохранение копии GLB…".into();
                        }
                        Err(error) => {
                            editor.status = format!("Не удалось подготовить копию · {error}");
                            keep = false;
                        }
                    }
                }
                Some(_) => {}
                None => keep = false,
            }
        }
        if let Work::Write(task) = &mut job.work
            && let Some(result) = block_on(poll_once(task))
        {
            keep = false;
            match result {
                Ok(path) if editor.project.asset_root == job.root => {
                    browser.refresh(&mut editor.project);
                    previews.extend(editor.project.catalog.len());
                    if active == Some(job.model)
                        && let Some(copy) = editor
                            .project
                            .catalog
                            .iter()
                            .position(|model| Path::new(&model.asset) == path)
                    {
                        editor.open_model_tab(copy);
                        state.activate = Some(copy);
                    }
                    editor.status = format!("Копия сохранена · {}", path.display());
                }
                Ok(_) => {}
                Err(error) => editor.status = format!("Не удалось сохранить копию · {error}"),
            }
        }
        if keep {
            state.job = Some(job);
        }
        return;
    }
    if requested
        && !editor.browser_popup
        && !editor.hdri_library_open
        && let Some(index) = active
        && let Some(doc) = editing
            .documents
            .get(&index)
            .filter(|doc| doc.mode == Mode::Paint)
    {
        let source = doc.source.path.clone();
        #[cfg(test)]
        let path = state.test_path.take();
        #[cfg(not(test))]
        let path: Option<PathBuf> = None;
        let work = path.map_or_else(
            || {
                let source = source.clone();
                Work::Choose(IoTaskPool::get().spawn(async move { choose_path(&source) }))
            },
            Work::Waiting,
        );
        state.job = Some(Job {
            model: index,
            root: editor.project.asset_root.clone(),
            source,
            work,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_paint_data::solid;
    use serde_json::json;

    #[test]
    fn paint_save_copy_preserves_unsaved_layers_and_refuses_occupied_paths() {
        let root = std::env::temp_dir().join(format!(
            "bevy-model-copy-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let source_path = root.join("source.glb");
        let mut objects =
            paint_mesh_replacement::primitive(paint_mesh_replacement::Shape::Plane).unwrap();
        let original = Glb::new_asset(&objects).unwrap();
        fs::write(&source_path, &original).unwrap();
        objects[0].source_node = Some(0);
        let mut doc = Document::new(Glb::load(&source_path).unwrap(), objects);
        let mut layer = Layer::new("Несохранённые штрихи".into(), LayerKind::Paint);
        layer.channels[0] = Some(solid(16, [13, 71, 181, 255]));
        layer.mask = Some(Mask::new(solid(16, [179; 4]), Vec::new()).into());
        doc.objects[0].materials[0].paint = Some(Arc::new(Surface {
            size: 16,
            base: std::array::from_fn(|_| solid(16, [128; 4])),
            stack: Stack {
                layers: vec![layer],
                selected: 0,
            },
            material: json!({}),
            baked: None,
        }));
        doc.paint_uv.show_seams_2d = true;
        assert!(doc.dirty());
        let path = root.join("Копия");
        let saved = Snapshot::capture(&doc)
            .unwrap()
            .write(&root, &path)
            .unwrap();
        assert_eq!(saved, Path::new("Копия.glb"));
        let absolute = root.join(saved);
        let saved_bytes = fs::read(&absolute).unwrap();
        assert!(
            Snapshot::capture(&doc)
                .unwrap()
                .write(&root, &path)
                .is_err()
        );
        assert!(
            Snapshot::capture(&doc)
                .unwrap()
                .write(&root, &source_path)
                .is_err()
        );
        assert_eq!(fs::read(&absolute).unwrap(), saved_bytes);
        assert_eq!(fs::read(&source_path).unwrap(), original);
        assert!(doc.dirty());
        let glb = Glb::load(&absolute).unwrap();
        let mesh = glb.json["nodes"][0]["mesh"].as_u64().unwrap() as usize;
        let primitive = &glb.json["meshes"][mesh]["primitives"][0];
        let paint = glb.paint_surface(primitive).unwrap().unwrap();
        assert_eq!(paint.stack.layers[0].name, "Несохранённые штрихи");
        assert_eq!(
            paint.stack.layers[0].channels[0].as_ref().unwrap()[..4],
            [13, 71, 181, 255]
        );
        assert_eq!(paint.stack.layers[0].mask.as_ref().unwrap().pixels[0], 179);
        assert_eq!(
            glb.json["extras"][crate::editor::model_document::PAINT_UV_EXTRA]["show_seams_2d"],
            true
        );
        fs::remove_dir_all(root).unwrap();
    }
}
