//! MCP access to the same bake jobs, caches and report data used by the model viewer.
use super::*;
use bevy::ecs::system::SystemParam;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Component, Path};

pub(super) enum Outcome {
    Ready { from_disk: bool },
    Failed(String),
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct BakeTarget {
    pub(in crate::editor::ui::model_viewer::paint) model_id: usize,
    /// Texture set index from get_paint_state. Defaults to the active set.
    pub(in crate::editor::ui::model_viewer::paint) set: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct BakeArgs {
    pub(in crate::editor::ui::model_viewer::paint) target: BakeTarget,
    quality: Option<Quality>,
    id_source: Option<IdSource>,
    maps: Option<Vec<BakeMap>>,
    /// GLB, OBJ or FBX path. Use an empty string to clear the high-poly source.
    high_poly: Option<String>,
    /// GLB, OBJ or FBX path. Use an empty string to clear the external cage.
    cage: Option<String>,
    front_percent: Option<u8>,
    rear_percent: Option<u8>,
    match_names: Option<bool>,
    fill_misses: Option<bool>,
    auto_fit: Option<bool>,
    /// Show the first available bake map in the editor when ready.
    #[serde(default)]
    preview: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct BakeLevelsArgs {
    pub(in crate::editor::ui::model_viewer::paint) target: BakeTarget,
    map: BakeMap,
    black_percent: Option<u16>,
    white_percent: Option<u16>,
    /// Gamma × 100. 100 leaves the curve linear.
    gamma_percent: Option<u16>,
    #[serde(default)]
    reset: bool,
}

#[derive(SystemParam)]
pub(in crate::editor) struct McpBake<'w> {
    state: Option<ResMut<'w, State>>,
    device: Option<Res<'w, RenderDevice>>,
    queue: Option<Res<'w, RenderQueue>>,
}

impl McpBake<'_> {
    pub(in crate::editor::ui::model_viewer::paint) fn set_levels(
        &mut self, model: usize, doc: &mut Document, session: &mut Session, args: BakeLevelsArgs,
    ) -> Result<Value, String> {
        let state = self.state.as_deref_mut().ok_or("The bake engine is unavailable")?;
        if state.running.is_some() || state.picking.is_some() || state.clearing_cache.is_some()
            || doc.paint_busy || doc.paint_pending || session.busy || session.resize_request.is_some()
        {
            return Err("Wait for the current bake or paint operation to finish".into());
        }
        let set = args.target.set.unwrap_or(session.set);
        let (object, slot) = doc.paint_target(set).ok_or("Unknown texture set")?;
        let binding = (object.id, slot);
        let material = &object.materials[slot];
        if !levels::supports(args.map)
            || !state.maps(model, doc, binding, doc.paint_size(material)).is_some_and(|maps| maps.has_map(args.map))
        {
            return Err("Choose an available baked scalar map that supports levels".into());
        }
        let new = args.resolve(levels::get(material, args.map))?;
        if state.apply_levels(model, doc, binding, args.map, new)? {
            session.revision += 1;
        }
        status(state, model, doc, set)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn start(
        &mut self,
        editor: &mut Editor,
        doc: &mut Document,
        session: &mut Session,
        args: BakeArgs,
    ) -> Result<Value, String> {
        let state = self
            .state
            .as_deref_mut()
            .ok_or("The bake engine is unavailable")?;
        let model = args.target.model_id;
        let set = session.set;
        if args.target.set.is_some_and(|expected| expected != set) {
            return Err("Select the requested texture set before baking".into());
        }
        if state.running.is_some()
            || state.picking.is_some()
            || state.clearing_cache.is_some()
            || doc.paint_busy
            || doc.paint_pending
            || session.busy
            || session.resize_request.is_some()
        {
            return Err("Wait for the current bake or paint operation to finish".into());
        }
        let (object, slot) = doc.paint_target(set).ok_or("Unknown texture set")?;
        if session.binding != Some((object.id, slot))
            || object.hidden
            || !engine::projection::has_uv(object, slot)
        {
            return Err("Bake requires a loaded texture set with UVs on a visible object".into());
        }
        let settings = settings(&editor.project.asset_root, &doc.bake_settings, &args)?;
        if session.bake_projection.saved() != settings {
            session.bake_projection.restore(&settings);
        }
        if settings != doc.bake_settings {
            let before = doc.snapshot();
            doc.bake_settings = settings;
            doc.checkpoint(before);
        }
        let gpu = self
            .device
            .as_deref()
            .zip(self.queue.as_deref())
            .map(|(device, queue)| mesh::RayDevice {
                device: device.clone(),
                queue: queue.clone(),
            });
        editor.status = state
            .start_manual(model, doc, session, gpu, args.preview)?
            .into();
        if args.preview {
            session.panel = Panel::Maps;
        }
        status(state, model, doc, set)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn cancel(
        &mut self,
        editor: &mut Editor,
        doc: &mut Document,
        model: usize,
        set: usize,
    ) -> Result<Value, String> {
        let state = self
            .state
            .as_deref_mut()
            .ok_or("The bake engine is unavailable")?;
        let (object, slot) = doc.paint_target(set).ok_or("Unknown texture set")?;
        if !state.running.as_ref().is_some_and(|job| {
            (job.key.model, job.key.object, job.key.slot) == (model, object.id, slot)
        }) {
            return Err("There is no running bake for the requested texture set".into());
        }
        state.cancel_running();
        state.automatic.remove(&model);
        doc.auto_bake_pending = false;
        editor.status = "Запекание отменено".into();
        status(state, model, doc, set)
    }

    pub(in crate::editor::ui::model_viewer::paint) fn status(
        &self,
        model: usize,
        doc: &Document,
        set: usize,
    ) -> Result<Value, String> {
        let state = self
            .state
            .as_deref()
            .ok_or("The bake engine is unavailable")?;
        status(state, model, doc, set)
    }
}

impl BakeLevelsArgs {
    fn resolve(&self, previous: levels::Levels) -> Result<levels::Levels, String> {
        let changed = self.black_percent.is_some() || self.white_percent.is_some() || self.gamma_percent.is_some();
        if self.reset && changed {
            return Err("Choose either reset or explicit level values".into());
        }
        let levels = if self.reset {
            levels::Levels::default()
        } else {
            levels::Levels {
                black: self.black_percent.unwrap_or(previous.black),
                white: self.white_percent.unwrap_or(previous.white),
                gamma: self.gamma_percent.unwrap_or(previous.gamma),
            }
        };
        if !levels.valid() {
            return Err("Black and white points must be 0–100 percent and gamma must be 10–400 percent".into());
        }
        Ok(levels)
    }
}

fn settings(root: &Path, saved: &Settings, args: &BakeArgs) -> Result<Settings, String> {
    let mut settings = saved.clone();
    if let Some(quality) = args.quality {
        settings.quality = quality;
    }
    if let Some(source) = args.id_source {
        settings.id_source = source;
    }
    if let Some(maps) = &args.maps {
        settings.map_bits = import_bake_map_bits(maps.iter().copied());
    }
    if let Some(path) = &args.high_poly {
        settings.high_poly = source_path(root, path)?;
        if settings.high_poly.is_none() {
            settings.cage = None;
        }
    }
    if let Some(path) = &args.cage {
        settings.cage = source_path(root, path)?;
    }
    if settings.cage.is_some() && settings.high_poly.is_none() {
        return Err("An external cage requires a high-poly source".into());
    }
    for (value, target) in [
        (args.front_percent, &mut settings.front),
        (args.rear_percent, &mut settings.rear),
    ] {
        if let Some(value) = value {
            if !(1..=100).contains(&value) {
                return Err("Projection distances must be between 1 and 100 percent".into());
            }
            *target = value;
        }
    }
    if let Some(value) = args.match_names {
        settings.match_names = value;
    }
    if let Some(value) = args.fill_misses {
        settings.fill_misses = value;
    }
    if let Some(value) = args.auto_fit {
        settings.auto_fit = value;
    }
    Ok(settings)
}

fn source_path(root: &Path, value: &str) -> Result<Option<PathBuf>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_owned()
    } else if path
        .components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
    {
        root.join(path)
    } else {
        return Err("Use an absolute path or a path relative to the project asset folder".into());
    };
    if ProjectionFormat::of(&path).is_none() || !path.is_file() {
        return Err("Choose an existing GLB, OBJ or FBX for the bake source".into());
    }
    Ok(Some(path))
}

fn status(state: &State, model: usize, doc: &Document, set: usize) -> Result<Value, String> {
    let (object, slot) = doc.paint_target(set).ok_or("Unknown texture set")?;
    let binding = (model, object.id, slot);
    let size = doc.paint_size(&object.materials[slot]);
    let running = state
        .running
        .as_ref()
        .filter(|job| (job.key.model, job.key.object, job.key.slot) == binding);
    let outcome = state.outcomes.get(&binding);
    let maps = state.maps(model, doc, (object.id, slot), size);
    let (phase, error, from_disk) = match (running, outcome, maps) {
        (Some(_), _, _) => ("running", None, None),
        (_, Some(Outcome::Failed(error)), _) => ("failed", Some(error), None),
        (_, Some(Outcome::Cancelled), _) => ("cancelled", None, None),
        (_, Some(Outcome::Ready { from_disk }), Some(_)) => ("ready", None, Some(*from_disk)),
        (_, _, Some(_)) => ("ready", None, None),
        _ => ("idle", None, None),
    };
    let available = maps.map_or_else(Vec::new, |maps| {
        BakeMap::ALL
            .into_iter()
            .chain(
                doc.custom_bakes
                    .iter()
                    .map(|recipe| BakeMap::Custom(recipe.id)),
            )
            .filter(|&map| maps.has_map(map))
            .collect()
    });
    Ok(json!({
        "model_id": model,
        "set": set,
        "resolution": size,
        "status": phase,
        "progress": running.and_then(|_| state.progress()),
        "engine_busy": state.running.is_some(),
        "error": error,
        "from_disk_cache": from_disk,
        "maps": available,
        "levels": available.iter().copied().filter(|&map| levels::supports(map)).map(|map| {
            let levels = levels::get(&object.materials[slot], map);
            json!({"map": map, "black_percent": levels.black, "white_percent": levels.white, "gamma_percent": levels.gamma})
        }).collect::<Vec<_>>(),
        "settings": doc.bake_settings,
        "report": maps.map(|maps| json!({
            "uv_texels": maps.filled,
            "uv_coverage_percent": maps.filled as f64 * 100. / f64::from(size).powi(2),
            "projection_hits": maps.hits,
            "projection_filled_misses": maps.filled_misses,
            "gpu_rays": maps.gpu_rays,
            "gpu_projection": maps.gpu_projection,
            "gpu_denoise": maps.gpu_denoise,
            "gpu_padding": maps.gpu_padding,
        })),
    }))
}
