//! Prepare immutable paint exports on the editor thread and write them on a worker.
use super::*;
use bevy::{ecs::system::SystemParam, tasks::AsyncComputeTaskPool};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Component;

pub(in crate::editor) type OutputReply = Box<dyn FnOnce(Result<Value, String>) + Send + Sync>;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct ExportArgs {
    pub(in crate::editor::ui::model_viewer::paint) model_id: usize,
    /// Existing destination folder. Relative paths are resolved in the project asset folder.
    folder: PathBuf,
    #[serde(default)]
    all_sets: bool,
    prefix: Option<String>,
    template: Option<Template>,
    format: Option<Format>,
    /// Bits per channel: 8, 16, or 32 (float).
    bit_depth: Option<u8>,
    size: Option<u32>,
    padding: Option<Padding>,
    padding_width: Option<u32>,
}

#[derive(SystemParam)]
pub(in crate::editor) struct McpOutput<'w, 's> {
    images: Option<Res<'w, Assets<Image>>>,
    materials: Option<Res<'w, Assets<StandardMaterial>>>,
    baked: Option<Res<'w, bake::State>>,
    preferences: Option<Res<'w, prefs::Store>>,
    runtime: Option<Res<'w, engine::Runtime>>,
    worker: Local<'s, Option<Receiver<()>>>,
}

impl McpOutput<'_, '_> {
    pub(in crate::editor::ui::model_viewer::paint) fn stroke_active(&self) -> bool {
        self.runtime
            .as_deref()
            .is_some_and(engine::Runtime::stroke_active)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn write(
        &mut self,
        job: Result<impl FnOnce() -> Result<Value, String> + Send + 'static, String>,
        reply: OutputReply,
    ) {
        if self
            .worker
            .as_ref()
            .is_some_and(|task| matches!(task.try_recv(), Err(TryRecvError::Empty)))
        {
            reply(Err("Wait for the current paint export or GLB copy".into()));
            return;
        }
        match job {
            Ok(job) => {
                let (done, pending) = mpsc::channel();
                *self.worker = Some(pending);
                AsyncComputeTaskPool::get()
                    .spawn(async move {
                        let result = job();
                        let _ = done.send(());
                        reply(result);
                    })
                    .detach();
            }
            Err(error) => reply(Err(error)),
        }
    }

    pub(in crate::editor::ui::model_viewer::paint) fn prepare_export(
        &self,
        editor: &Editor,
        doc: &Document,
        binding: Option<(usize, usize)>,
        args: ExportArgs,
    ) -> Result<impl FnOnce() -> Result<Value, String> + Send + 'static, String> {
        let images = self
            .images
            .as_deref()
            .ok_or("Image assets are unavailable")?;
        let materials = self
            .materials
            .as_deref()
            .ok_or("Material assets are unavailable")?;
        let preferences = self
            .preferences
            .as_deref()
            .ok_or("Export preferences are unavailable")?;
        let settings = export_settings(preferences.settings(), &args)?;
        let recipe = match settings.template {
            Template::Custom(index) => Some(
                preferences
                    .recipe(index)
                    .ok_or("Unknown export template")?
                    .clone(),
            ),
            _ => None,
        };
        let folder = output_folder(&editor.project.asset_root, &args.folder)?;
        let selected = if args.all_sets {
            None
        } else {
            Some(binding.ok_or("No active texture set")?)
        };
        let sets = capture(
            doc,
            selected,
            images,
            materials,
            self.baked.as_deref().map(|baked| (baked, args.model_id)),
        );
        if sets.is_empty() {
            return Err("The model has no texture sets to export".into());
        }
        let model = args.prefix.unwrap_or_else(|| {
            doc.source
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Model")
                .to_owned()
        });
        let project = editor.project.title.clone();
        Ok(move || {
            let folder = folder.canonicalize().map_err(|error| error.to_string())?;
            if !folder.is_dir() {
                return Err("The export destination must be an existing folder".into());
            }
            let report = write_textures(&folder, &model, &project, sets, settings, recipe.as_ref())
                .map_err(|error| error.to_string())?;
            Ok(json!({"folder": report.path, "files": report.paths}))
        })
    }
}

fn output_folder(root: &Path, path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("Specify the destination folder".into());
    }
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err("Use an absolute folder or a path relative to the project asset folder".into());
    }
    Ok(root.join(path))
}

fn export_settings(
    mut settings: prefs::Settings,
    args: &ExportArgs,
) -> Result<prefs::Settings, String> {
    if let Some(template) = args.template {
        settings.template = template;
    }
    if let Some(format) = args.format {
        settings.file.set_format(format);
    }
    if let Some(depth) = args.bit_depth {
        settings.file.depth = match depth {
            8 => Depth::U8,
            16 => Depth::U16,
            32 => Depth::Float,
            _ => return Err("Bit depth must be 8, 16 or 32".into()),
        };
    }
    if !settings.file.format.depths().contains(&settings.file.depth) {
        return Err("That image format does not support the requested bit depth".into());
    }
    if let Some(size) = args.size {
        if !crate::editor::model_paint_data::RESOLUTIONS.contains(&size) {
            return Err("Export resolution must be 512, 1024, 2048 or 4096".into());
        }
        settings.file.size = Some(size);
    }
    if let Some(padding) = args.padding {
        settings.padding = padding;
    }
    if let Some(width) = args.padding_width {
        if !(1..=256).contains(&width) {
            return Err("Padding width must be between 1 and 256 pixels".into());
        }
        settings.width = width;
    }
    Ok(settings)
}
