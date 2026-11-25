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
    /// Built-in identifier, display name, or a saved recipe name. With template_file, selects a recipe from that file.
    template_name: Option<String>,
    /// A portable Material Painter export template JSON file, absolute or relative to the asset root.
    template_file: Option<PathBuf>,
    format: Option<Format>,
    /// Bits per channel: 8, 16, or 32 (float).
    bit_depth: Option<u8>,
    size: Option<u32>,
    #[serde(default)]
    native_size: bool,
    padding: Option<Padding>,
    padding_width: Option<u32>,
    normal: Option<NormalPatch>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct NormalPatch {
    height_strength: Option<f32>,
    use_baked: Option<bool>,
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
    pub(in crate::editor) fn template_catalog(&self) -> Vec<Value> {
        let builtins =
            Template::ALL.map(|template| json!({"id": template, "label": template.label()}));
        builtins
            .into_iter()
            .chain(self.preferences.iter().flat_map(|store| {
                store
                    .recipe_labels()
                    .map(|(template, label)| json!({"id": template, "label": label}))
            }))
            .collect()
    }

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
        let mut settings = export_settings(preferences.settings(), &args)?;
        if args.template.is_some() && (args.template_name.is_some() || args.template_file.is_some())
        {
            return Err("Choose template or template_name/template_file".into());
        }
        let source = args
            .template_file
            .as_deref()
            .map(|path| output_folder(&editor.project.asset_root, path))
            .transpose()?;
        if source.is_some() {
            settings.template = Template::Custom(0);
        } else if let Some(name) = &args.template_name {
            settings.template = named_template(preferences, name)?;
        }
        let recipe = match settings.template {
            Template::Custom(index) if source.is_none() => Some(
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
            let recipe = match source {
                Some(path) => Some(load_template(&path, args.template_name.as_deref())?),
                None => recipe,
            };
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

fn named_template(preferences: &prefs::Store, name: &str) -> Result<Template, String> {
    let name = name.trim().to_lowercase();
    if let Some(template) = Template::ALL.into_iter().find(|template| {
        serde_json::to_value(template)
            .ok()
            .as_ref()
            .and_then(Value::as_str)
            == Some(name.as_str())
    }) {
        return Ok(template);
    }
    let mut matches = Template::ALL
        .into_iter()
        .map(|template| (template, template.label()))
        .chain(preferences.recipe_labels())
        .filter(|(_, label)| label.to_lowercase() == name);
    let (template, _) = matches
        .next()
        .ok_or("Unknown export template name; use list_paint_presets")?;
    if matches.next().is_some() {
        return Err("The export template name is ambiguous; use a template id".into());
    }
    Ok(template)
}

fn load_template(path: &Path, name: Option<&str>) -> Result<recipe::Recipe, String> {
    let mut recipes = recipe::load(path)
        .map_err(|error| error.to_string())?
        .into_iter();
    if let Some(name) = name {
        let name = name.trim().to_lowercase();
        let mut matches = recipes.filter(|recipe| recipe.name.to_lowercase() == name);
        let recipe = matches
            .next()
            .ok_or("The template file has no recipe with that name")?;
        if matches.next().is_some() {
            return Err("The template file contains duplicate recipe names".into());
        }
        return Ok(recipe);
    }
    if recipes.len() != 1 {
        return Err("Supply template_name when the file contains multiple recipes".into());
    }
    Ok(recipes.next().unwrap())
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
        if args.native_size {
            return Err("Choose either size or native_size".into());
        }
        if !(16..=8192).contains(&size) {
            return Err("Export resolution must be between 16 and 8192 pixels".into());
        }
        settings.file.size = Some(size);
    }
    if args.native_size {
        settings.file.size = None;
    }
    if let Some(normal) = &args.normal {
        if let Some(strength) = normal.height_strength {
            if !(0.0..=8.0).contains(&strength) {
                return Err("Normal height strength must be between 0 and 8".into());
            }
            settings.file.normal.height_strength = strength;
        }
        if let Some(use_baked) = normal.use_baked {
            settings.file.normal.use_baked = use_baked;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_export_resolves_saved_templates_and_builtin_ids() {
        let scratch = tempfile::tempdir().unwrap();
        let profile = scratch.path().join("export.json");
        std::fs::write(
            &profile,
            br#"{"recipes":[{"name":"Saved Metal","maps":[
            {"name":"Metal","layout":"Grayscale","channels":[{"src":"Metallic"}]}
        ]}]}"#,
        )
        .unwrap();
        let preferences = prefs::Store::open(Some(profile));
        assert_eq!(
            named_template(&preferences, "SAVED Metal").unwrap(),
            Template::Custom(0)
        );
        assert_eq!(
            named_template(&preferences, "unity_urp").unwrap(),
            Template::UnityUrp
        );
        assert_eq!(
            named_template(&preferences, "Unreal Engine").unwrap(),
            Template::Unreal
        );
    }
}
