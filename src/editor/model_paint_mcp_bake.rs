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
    /// Prefer front-facing high-poly intersections.
    prefer_front_faces: Option<bool>,
    /// Average the low-poly normals used to cast projection rays.
    average_normals: Option<bool>,
    auto_fit: Option<bool>,
    rays: Option<RayPatch>,
    surface: Option<SurfacePatch>,
    /// Show the first available bake map in the editor when ready.
    #[serde(default)]
    preview: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor::ui::model_viewer::paint) struct ProjectionPatch {
    /// GLB, OBJ or FBX path. An empty string clears the source and cage.
    high_poly: Option<String>,
    /// GLB, OBJ or FBX path. An empty string clears the external cage.
    cage: Option<String>,
    /// Front ray distance as 1–100 percent of the model diagonal.
    front_percent: Option<u8>,
    /// Rear ray distance as 1–100 percent of the model diagonal.
    rear_percent: Option<u8>,
    match_names: Option<bool>,
    prefer_front_faces: Option<bool>,
    average_normals: Option<bool>,
    auto_fit: Option<bool>,
}

impl ProjectionPatch {
    pub(in crate::editor::ui::model_viewer::paint) fn resolve(
        &self, root: &Path, saved: &Settings,
    ) -> Result<Settings, String> {
        let mut next = saved.clone();
        update_projection(
            root, &mut next, self.high_poly.as_deref(), self.cage.as_deref(),
            self.front_percent, self.rear_percent,
        )?;
        next.match_names = self.match_names.unwrap_or(next.match_names);
        next.prefer_front_faces = self.prefer_front_faces.unwrap_or(next.prefer_front_faces);
        next.average_normals = self.average_normals.unwrap_or(next.average_normals);
        next.auto_fit = self.auto_fit.unwrap_or(next.auto_fit);
        Ok(next)
    }
}

pub(in crate::editor::ui::model_viewer::paint) fn apply_settings(
    doc: &mut Document, session: &mut Session, settings: Settings,
) {
    if session.bake_projection.saved() != settings {
        session.bake_projection.restore(&settings);
        session.revision += 1;
    }
    if settings != doc.bake_settings {
        let before = doc.snapshot();
        doc.bake_settings = settings;
        doc.checkpoint(before);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct RayPatch {
    /// 4–512 samples. Null uses the quality preset.
    #[serde(default, deserialize_with = "optional_override", skip_serializing_if = "Option::is_none")]
    occlusion_samples: Option<Option<u16>>,
    /// 0–256 samples. Null uses the quality preset.
    #[serde(default, deserialize_with = "optional_override", skip_serializing_if = "Option::is_none")]
    thickness_samples: Option<Option<u16>>,
    /// 0–256 samples. Null uses the quality preset.
    #[serde(default, deserialize_with = "optional_override", skip_serializing_if = "Option::is_none")]
    sky_samples: Option<Option<u16>>,
    /// 1–100000 in hundredths of a percent of the model diagonal.
    occlusion_reach: Option<u32>,
    /// 1–100000 in hundredths of a percent of the model diagonal.
    cavity_reach: Option<u32>,
    /// 1–100000 in hundredths of a percent of the model diagonal.
    thickness_reach: Option<u32>,
    /// 10–890 in tenths of a degree.
    sky_cone: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct SurfacePatch {
    /// 1–8 samples per axis. Null uses the quality preset's antialiasing.
    #[serde(default, deserialize_with = "optional_override", skip_serializing_if = "Option::is_none")]
    grid: Option<Option<u8>>,
    /// 10–160 in tenths of a texel.
    radius: Option<u16>,
    /// 1–100000 in hundredths of a percent.
    contrast: Option<u32>,
    /// 1–100000 in hundredths of a percent of the model diagonal.
    edge_reach: Option<u32>,
    auto_edge_reach: Option<bool>,
    /// 50–950 in thousandths.
    sharpness: Option<u16>,
    /// 50–1200 in tenths of a degree.
    hard_angle: Option<u16>,
}

fn optional_override<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer).map(Some)
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
    pub(in crate::editor::ui::model_viewer::paint) fn material_preparation(
        &self, model: usize, doc: &Document, session: &Session,
    ) -> presets::MaterialPreparation {
        let binding = session.binding;
        presets::MaterialPreparation {
            maps: binding.and_then(|binding| {
                self.state.as_deref()?.maps(model, doc, binding, session.texture_size).cloned()
            }),
            surface: binding.and_then(|binding| material_graph::surface::Source::capture(doc, binding))
                .map(Arc::new),
            renderer: self.device.as_deref().zip(self.queue.as_deref())
                .map(|(device, queue)| (device.clone(), queue.clone())),
        }
    }

    pub(in crate::editor::ui::model_viewer::paint) fn set_levels(
        &mut self, model: usize, doc: &mut Document, session: &mut Session, args: BakeLevelsArgs,
    ) -> Result<Value, String> {
        let state = self.state.as_deref_mut().ok_or("The bake engine is unavailable")?;
        if state.configuration_busy()
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
        if state.configuration_busy()
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
        apply_settings(doc, session, settings);
        let gpu = state.ray_device(self.device.as_deref(), self.queue.as_deref());
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
    update_projection(
        root, &mut settings, args.high_poly.as_deref(), args.cage.as_deref(),
        args.front_percent, args.rear_percent,
    )?;
    if let Some(value) = args.match_names {
        settings.match_names = value;
    }
    if let Some(value) = args.fill_misses {
        settings.fill_misses = value;
    }
    settings.prefer_front_faces = args.prefer_front_faces.unwrap_or(settings.prefer_front_faces);
    settings.average_normals = args.average_normals.unwrap_or(settings.average_normals);
    if let Some(value) = args.auto_fit {
        settings.auto_fit = value;
    }
    if let Some(patch) = &args.rays {
        let previous = settings.rays;
        let rays = RayOptions {
            occlusion_samples: patch.occlusion_samples.unwrap_or(previous.occlusion_samples),
            thickness_samples: patch.thickness_samples.unwrap_or(previous.thickness_samples),
            sky_samples: patch.sky_samples.unwrap_or(previous.sky_samples),
            occlusion_reach: patch.occlusion_reach.unwrap_or(previous.occlusion_reach),
            cavity_reach: patch.cavity_reach.unwrap_or(previous.cavity_reach),
            thickness_reach: patch.thickness_reach.unwrap_or(previous.thickness_reach),
            sky_cone: patch.sky_cone.unwrap_or(previous.sky_cone),
        };
        if rays != rays.validated() {
            return Err("One or more ray settings are outside their allowed ranges".into());
        }
        settings.rays = rays;
    }
    if let Some(patch) = &args.surface {
        let previous = settings.surface;
        let surface = SurfaceOptions {
            grid: patch.grid.unwrap_or(previous.grid),
            radius: patch.radius.unwrap_or(previous.radius),
            contrast: patch.contrast.unwrap_or(previous.contrast),
            edge_reach: patch.edge_reach.unwrap_or(previous.edge_reach),
            auto_edge_reach: patch.auto_edge_reach.unwrap_or(previous.auto_edge_reach),
            sharpness: patch.sharpness.unwrap_or(previous.sharpness),
            hard_angle: patch.hard_angle.unwrap_or(previous.hard_angle),
        };
        if surface != surface.validated() {
            return Err("One or more surface settings are outside their allowed ranges".into());
        }
        settings.surface = surface;
    }
    Ok(settings)
}

fn update_projection(
    root: &Path,
    settings: &mut Settings,
    high_poly: Option<&str>,
    cage: Option<&str>,
    front_percent: Option<u8>,
    rear_percent: Option<u8>,
) -> Result<(), String> {
    if let Some(path) = high_poly {
        settings.high_poly = source_path(root, path)?;
        if settings.high_poly.is_none() {
            settings.cage = None;
        }
    }
    if let Some(path) = cage {
        settings.cage = source_path(root, path)?;
    }
    if settings.cage.is_some() && settings.high_poly.is_none() {
        return Err("An external cage requires a high-poly source".into());
    }
    for (value, target) in [(front_percent, &mut settings.front), (rear_percent, &mut settings.rear)] {
        if let Some(value) = value {
            if !(1..=100).contains(&value) {
                return Err("Projection distances must be between 1 and 100 percent".into());
            }
            *target = value;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_patches_preserve_omissions_and_reset_explicit_nulls() {
        let mut saved = Settings::default();
        saved.rays.occlusion_samples = Some(64);
        saved.rays.thickness_samples = Some(32);
        saved.surface.grid = Some(8);
        let args: BakeArgs = serde_json::from_value(json!({
            "target": {"model_id": 0},
            "rays": {"occlusion_samples": null, "sky_samples": 0},
            "surface": {"grid": null, "auto_edge_reach": true},
            "prefer_front_faces": false,
            "average_normals": false
        })).unwrap();
        let args = serde_json::from_value(serde_json::to_value(args).unwrap()).unwrap();
        let updated = settings(Path::new("."), &saved, &args).unwrap();
        assert_eq!(updated.rays.occlusion_samples, None);
        assert_eq!(updated.rays.thickness_samples, Some(32));
        assert_eq!(updated.rays.sky_samples, Some(0));
        assert_eq!(updated.surface.grid, None);
        assert_eq!(updated.surface.radius, saved.surface.radius);
        assert!(updated.surface.auto_edge_reach);
        assert!(!updated.prefer_front_faces);
        assert!(!updated.average_normals);
    }

    #[test]
    fn sampling_patches_reject_invalid_values() {
        for patch in [
            json!({"rays": {"occlusion_samples": 3}}),
            json!({"surface": {"grid": 9}}),
        ] {
            let mut request = json!({"target": {"model_id": 0}, "average_normals": false});
            request.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            let args = serde_json::from_value(request).unwrap();
            assert!(settings(Path::new("."), &Settings::default(), &args).is_err());
        }
    }
}
