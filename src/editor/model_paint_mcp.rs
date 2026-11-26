//! MCP access to the material slots, layer edits and history of the paint workspace.
use super::*;
use crate::editor::model_document::Document;
use crate::editor::model_paint_data::mask::MaskBlend;
pub(in crate::editor) use bake::{BakeArgs, BakeLevelsArgs, BakeTarget, McpBake};
pub(in crate::editor) use engine::McpPaintInput;
pub(in crate::editor) use export::{ExportArgs, McpOutput, OutputReply};
pub(in crate::editor) use mcp_preview::offscreen::{RenderArgs, render};
pub(in crate::editor) use mcp_preview::{
    CageArgs, CaptureArgs, CaptureReply, McpPreview, ViewArgs as PaintViewArgs, cage, capture,
    set_view,
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::{Path, PathBuf}};

pub(super) const CHANNEL_KEYS: [&str; 8] = [
    "base_color",
    "roughness",
    "metallic",
    "height",
    "opacity",
    "emission",
    "normal",
    "specular",
];

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct EditArgs {
    /// model_id returned by get_paint_state.
    model_id: usize,
    /// Expected active texture set from get_paint_state.
    set: usize,
    /// Optional workspace revision from get_paint_state to reject stale layer indices.
    revision: Option<u64>,
    edit: Edit,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct SaveArgs {
    model_id: usize,
    /// Save a new GLB in the source model's directory instead of updating the open GLB.
    copy_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct SavePresetArgs {
    workspace: PaintWorkspace,
    layer: usize,
    kind: presets::CaptureKind,
    /// Defaults to the existing layer or group name.
    name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct ImportPresetArgs {
    /// Absolute path to a local .mpsmart file or a Material Painter ZIP package.
    source: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
enum PresetFormat {
    #[default]
    Mpsmart,
    Zip,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct ExportPresetArgs {
    workspace: PaintWorkspace,
    layer: usize,
    /// Absolute folder or a path relative to the project's asset folder.
    folder: PathBuf,
    #[serde(default)]
    format: PresetFormat,
    name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    SelectSet {
        index: usize,
    },
    UpdateTextureSet {
        /// Index from texture_sets in get_paint_state.
        index: usize,
        /// Resize the active set in the background to 512, 1024, 2048 or 4096.
        resolution: Option<u32>,
        visible: Option<bool>,
        /// Pin or clear the set's highlight in the viewport.
        highlighted: Option<bool>,
    },
    SelectLayer {
        index: usize,
        #[serde(default)]
        mask: bool,
    },
    AddLayer {
        kind: LayerKind,
        name: Option<String>,
        target: Option<MaterialTarget>,
    },
    AddMaterial {
        preset: String,
        name: Option<String>,
        target: Option<MaterialTarget>,
    },
    AddTexture {
        preset: String,
        name: Option<String>,
        target: Option<MaterialTarget>,
    },
    AddSmartMaterial {
        preset: String,
        name: Option<String>,
        target: Option<MaterialTarget>,
    },
    AddLocalMaterial {
        /// Index of a material in local_presets from list_paint_presets.
        preset: usize,
        name: Option<String>,
        target: Option<MaterialTarget>,
    },
    ApplyLocalMask {
        /// Index of a mask in local_presets from list_paint_presets.
        preset: usize,
        layer: usize,
    },
    UpdateLayer {
        index: usize,
        name: Option<String>,
        visible: Option<bool>,
        opacity_percent: Option<f32>,
        blend: Option<Blend>,
        fill: Option<Box<FillPatch>>,
    },
    DuplicateLayer {
        index: usize,
    },
    RemoveLayer {
        index: usize,
    },
    MoveLayer {
        from: usize,
        to: usize,
        placement: Placement,
    },
    SetMask {
        index: usize,
        mask: MaskBase,
    },
    ApplyGenerator {
        index: usize,
        generator: GeneratorKind,
        #[serde(default)]
        parameters: GeneratorPatch,
    },
    UpdateGenerator {
        index: usize,
        /// Omit to edit the mask's main generator.
        mask_layer: Option<usize>,
        parameters: GeneratorPatch,
    },
    AddMaskLayer {
        index: usize,
        source: MaskLayerSource,
        #[serde(default)]
        parameters: MaskLayerPatch,
    },
    UpdateMaskLayer {
        index: usize,
        mask_layer: usize,
        parameters: MaskLayerPatch,
    },
    MoveMaskLayer {
        index: usize,
        from: usize,
        to: usize,
    },
    RemoveMaskLayer {
        index: usize,
        mask_layer: usize,
    },
    AddMaskFilter {
        index: usize,
        filter: FilterKind,
        #[serde(default)]
        parameters: FilterPatch,
    },
    UpdateMaskFilter {
        index: usize,
        filter_index: usize,
        parameters: FilterPatch,
    },
    MoveMaskFilter {
        index: usize,
        from: usize,
        to: usize,
    },
    RemoveMaskFilter {
        index: usize,
        filter_index: usize,
    },
    Undo { steps: Option<usize> },
    Redo { steps: Option<usize> },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
enum Placement {
    Above,
    Below,
    Inside,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct MaterialTarget {
    layer: usize,
    placement: Placement,
}

impl MaterialTarget {
    fn shelf(self) -> ShelfPlacement {
        match self.placement {
            Placement::Above => ShelfPlacement::Above(self.layer),
            Placement::Below => ShelfPlacement::Below(self.layer),
            Placement::Inside => ShelfPlacement::Inside(self.layer),
        }
    }

    fn insertion(session: &Session, target: Option<Self>) -> Result<(usize, usize), String> {
        match target {
            Some(target) => session.shelf_insertion(target.shelf())
                .ok_or_else(|| "Unknown layer or invalid layer placement".into()),
            None => Ok(session.insertion()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
enum MaskBase {
    White,
    Black,
    Remove,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct GeneratorPatch {
    /// Inspector control indices, ranges and defaults are listed in list_paint_presets.
    controls: Option<BTreeMap<String, f32>>,
    invert: Option<bool>,
    edge_width: Option<u16>,
    id_tolerance: Option<u8>,
    id_color: Option<[u8; 3]>,
    /// Index in the target mask's available baked ID palette.
    id_region: Option<usize>,
    weights: Option<BTreeMap<String, i16>>,
}

fn control_index(key: &str) -> Result<usize, String> {
    key.parse::<usize>().ok().filter(|index| index.to_string() == key)
        .ok_or_else(|| format!("Invalid control index '{key}'"))
}

impl GeneratorPatch {
    fn apply(&self, mut generator: Generator, palette: &[[u8; 3]]) -> Result<Generator, String> {
        if self.id_color.is_some() && self.id_region.is_some() {
            return Err("Choose either id_color or id_region".into());
        }
        for (key, &value) in self.controls.iter().flatten() {
            let index = control_index(key)?;
            let Some(&(_, min, max, _)) = generator.controls().get(index) else {
                return Err(format!("Unknown control {index} for this generator"));
            };
            if !(min..=max).contains(&value) {
                return Err(format!("Generator control {index} must be between {min} and {max}"));
            }
            generator.set_value(index, value);
        }
        generator.invert = self.invert.unwrap_or(generator.invert);
        generator.edge_width = self.edge_width.unwrap_or(generator.edge_width);
        generator.id_tolerance = self.id_tolerance.unwrap_or(generator.id_tolerance);
        generator.id_color = match self.id_region {
            Some(index) => Some(*palette.get(index).ok_or("Unknown baked ID region for this mask")?),
            None => self.id_color.or(generator.id_color),
        };
        for (name, &weight) in self.weights.iter().flatten() {
            let input = serde_json::from_value::<GeneratorInput>(Value::String(name.clone()))
                .map_err(|_| format!("Unknown generator input '{name}'"))?;
            if !(-Generator::WEIGHT_LIMIT..=Generator::WEIGHT_LIMIT).contains(&weight) {
                return Err(format!("Weight for '{name}' must be between -250 and 250"));
            }
            generator.set_weight(input, weight);
        }
        if !generator.valid() {
            return Err("Invalid generator settings; edge_width must be 2–500 and id_tolerance 1–100".into());
        }
        Ok(generator)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct FilterPatch {
    enabled: Option<bool>,
    /// Inspector control indices and ranges are listed in list_paint_presets.
    controls: Option<BTreeMap<String, f32>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum MaskLayerSource {
    Fill,
    Generator {
        generator: GeneratorKind,
        #[serde(default)]
        parameters: GeneratorPatch,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct MaskLayerPatch {
    enabled: Option<bool>,
    blend: Option<MaskBlend>,
    opacity_percent: Option<u8>,
    value_percent: Option<u8>,
}

impl MaskLayerPatch {
    fn apply(self, layer: &mut MaskLayer) -> Result<(), String> {
        if self.value_percent.is_some() && layer.generator.is_some() {
            return Err("value_percent is only available for fill mask layers".into());
        }
        layer.enabled = self.enabled.unwrap_or(layer.enabled);
        layer.blend = self.blend.unwrap_or(layer.blend);
        layer.opacity = self.opacity_percent.unwrap_or(layer.opacity);
        layer.value = self.value_percent.unwrap_or(layer.value);
        if layer.opacity > 100 || layer.value > 100 {
            return Err("Mask layer opacity and value must be between 0 and 100 percent".into());
        }
        Ok(())
    }
}

impl FilterPatch {
    fn apply(&self, mut filter: Filter) -> Result<Filter, String> {
        filter.enabled = self.enabled.unwrap_or(filter.enabled);
        for (key, &value) in self.controls.iter().flatten() {
            let index = control_index(key)?;
            let Some(&(_, _, min, max)) = filter.kind.controls().get(index) else {
                return Err(format!("Unknown control {index} for this filter"));
            };
            if !(min..=max).contains(&value) {
                return Err(format!("Filter control {index} must be between {min} and {max}"));
            }
            filter.set_value(index, value);
            if filter.value(index) != value {
                return Err(format!("Filter control {index} requires a discrete value"));
            }
        }
        if !filter.valid() {
            return Err("Invalid mask filter settings".into());
        }
        Ok(filter)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct StrokeArgs {
    workspace: PaintWorkspace,
    target: RasterTarget,
    /// Pointer samples in order. Coordinates and pressure are in 0–1.
    samples: Vec<StrokeSample>,
    #[serde(default)]
    brush: BrushValues,
    /// Override stroke behavior for this command without changing the editor's brush.
    style: Option<StrokeOptions>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct FillArgs {
    workspace: PaintWorkspace,
    target: RasterTarget,
    /// Normalized position used to pick the starting face. Use either position or region.
    position: Option<[f32; 2]>,
    region: Option<FillRegion>,
    mode: Option<FillMode>,
    angle_degrees: Option<f32>,
    padding_px: Option<f32>,
    #[serde(default)]
    brush: BrushValues,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum FillRegion {
    Rectangle { start: [f32; 2], end: [f32; 2], through: Option<bool> },
    Lasso { points: Vec<[f32; 2]>, through: Option<bool> },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct PaintWorkspace {
    model_id: usize,
    set: usize,
    revision: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct MeshArgs {
    workspace: PaintWorkspace,
    replacement: MeshOperation,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum MeshOperation {
    Primitive { shape: paint_mesh_replacement::Shape },
    File {
        /// Absolute OBJ or FBX path, or a path relative to the project asset root.
        path: std::path::PathBuf,
    },
    Undo,
    Redo,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct SurfaceArgs {
    workspace: PaintWorkspace,
    layer: usize,
    source: SurfaceSeed,
    #[serde(default)]
    mode: surface_selection::MaskMode,
    #[serde(default)]
    stop_at: surface_selection::StopAt,
    crease_angle: Option<f32>,
    /// Optional token returned by list_paint_surface_parts. Rejects stale part indices.
    topology: Option<SurfaceTopology>,
    /// Wait for the mask and selection report to be published before replying.
    #[serde(default)]
    pub wait: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct SurfaceTopology {
    geometry_revision: u64,
    visibility_revision: u64,
}

impl SurfaceTopology {
    fn new(doc: &Document, session: &Session) -> Self {
        Self {
            geometry_revision: doc.revision,
            visibility_revision: session.set_appearance.visibility.revision,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct SurfacePartsArgs {
    workspace: PaintWorkspace,
    #[serde(default)]
    offset: usize,
    /// Number of results, 1–250. Defaults to 100.
    limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SurfaceSeed {
    Parts { indices: Vec<usize> },
    Point { position: [f32; 3] },
    Viewport { position: [f32; 2], view: Option<RasterView> },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct RasterTarget {
    pub layer: usize,
    #[serde(default)]
    pub mask: bool,
    pub space: PaintSpace,
    /// Use an independent 3D camera and pixel dimensions for this operation.
    pub view: Option<RasterView>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct RasterView {
    pub width: u32,
    pub height: u32,
    /// Override camera fields for this request only.
    pub camera: Option<mcp_preview::CameraPatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub(super) enum PaintSpace {
    Viewport,
    Uv,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct StrokeSample {
    pub position: [f32; 2],
    pub pressure: Option<f32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub(super) struct BrushValues {
    pub size_px: Option<f32>,
    pub flow_percent: Option<f32>,
    pub hardness_percent: Option<f32>,
    /// Legacy opacity override. With only the opacity map active, this sets the map value.
    pub opacity_percent: Option<f32>,
    /// Strength of the whole stroke or fill, independent of the map value.
    pub stroke_opacity_percent: Option<f32>,
    pub spacing_percent: Option<f32>,
    pub erase: bool,
    pub mask_value: Option<u8>,
    pub channels: Option<PaintColors>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct StrokeOptions {
    pub projection: Option<BrushProjection>,
    pub symmetry: Option<Symmetry>,
    pub paint_through: Option<bool>,
    pub angle_limit_degrees: Option<f32>,
    /// Tip name from list_paint_presets, or "round" for the built-in round stamp.
    pub tip: Option<String>,
    pub tip_angle_degrees: Option<f32>,
    pub follow_stroke: Option<bool>,
    /// Grain name from list_paint_presets, or "none" to disable grain.
    pub grain: Option<String>,
    pub grain_strength_percent: Option<f32>,
    pub grain_scale: Option<f32>,
    pub grain_angle_degrees: Option<f32>,
    pub grain_invert: Option<bool>,
    pub grain_uv: Option<bool>,
    pub scatter_percent: Option<f32>,
    pub size_jitter_percent: Option<f32>,
    pub angle_jitter_percent: Option<f32>,
    pub stabilizer_px: Option<f32>,
    pub pressure_size: Option<bool>,
    pub pressure_flow: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub(super) struct PaintColors {
    base_color: Option<[u8; 3]>,
    roughness: Option<u8>,
    metallic: Option<u8>,
    height: Option<u8>,
    opacity: Option<u8>,
    emission: Option<[u8; 3]>,
    normal: Option<[u8; 3]>,
    specular: Option<u8>,
}

impl PaintColors {
    pub fn colors(&self) -> [Option<[u8; 3]>; 8] {
        [
            self.base_color,
            self.roughness.map(|v| [v; 3]),
            self.metallic.map(|v| [v; 3]),
            self.height.map(|v| [v; 3]),
            self.opacity.map(|v| [v; 3]),
            self.emission,
            self.normal,
            self.specular.map(|v| [v; 3]),
        ]
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct FillPatch {
    /// Solid channel values in the same RGB8 or scalar byte format as a brush.
    channels: Option<PaintColors>,
    /// Channel name to image path relative to the asset root or absolute. Null removes the image.
    images: Option<BTreeMap<String, Option<PathBuf>>>,
    projection: Option<texture_fill::Projection>,
    tiling: Option<f32>,
    rotation: Option<f32>,
    offset: Option<[f32; 2]>,
    sharpness: Option<f32>,
    normal_strength: Option<f32>,
    height_strength: Option<f32>,
    height_offset: Option<f32>,
    opacity_source: Option<texture_fill::OpacitySource>,
    opacity_strength: Option<f32>,
    emission_tint: Option<[f32; 3]>,
    specular_strength: Option<f32>,
    variation: Option<VariationPatch>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct VariationPatch {
    color: Option<f32>,
    roughness: Option<f32>,
    height: Option<f32>,
    scale: Option<f32>,
    seed: Option<f32>,
    parts: Option<f32>,
}

type FillImages = Vec<(usize, Option<Arc<texture_fill::Bitmap>>)>;

impl FillPatch {
    fn apply(self, layer: &mut Layer, size: u32, images: FillImages) -> Result<(), String> {
        if layer.kind != LayerKind::Fill {
            return Err("Fill settings require a fill layer".into());
        }
        let before = layer.texture_fill.as_ref().map(|fill| fill.settings).unwrap_or_default();
        let mut settings = before;
        settings.projection = self.projection.unwrap_or(settings.projection);
        settings.tiling = self.tiling.unwrap_or(settings.tiling);
        settings.rotation = self.rotation.unwrap_or(settings.rotation);
        settings.offset = self.offset.unwrap_or(settings.offset);
        settings.sharpness = self.sharpness.unwrap_or(settings.sharpness);
        settings.normal_strength = self.normal_strength.unwrap_or(settings.normal_strength);
        settings.height_strength = self.height_strength.unwrap_or(settings.height_strength);
        settings.height_offset = self.height_offset.unwrap_or(settings.height_offset);
        settings.opacity_source = self.opacity_source.unwrap_or(settings.opacity_source);
        settings.opacity_strength = self.opacity_strength.unwrap_or(settings.opacity_strength);
        settings.emission_tint = self.emission_tint.unwrap_or(settings.emission_tint);
        settings.specular_strength = self.specular_strength.unwrap_or(settings.specular_strength);
        if let Some(patch) = self.variation {
            let variation = &mut settings.variation;
            variation.color = patch.color.unwrap_or(variation.color);
            variation.roughness = patch.roughness.unwrap_or(variation.roughness);
            variation.height = patch.height.unwrap_or(variation.height);
            variation.scale = patch.scale.unwrap_or(variation.scale);
            variation.seed = patch.seed.unwrap_or(variation.seed);
            variation.parts = patch.parts.unwrap_or(variation.parts);
        }
        if !settings.valid() {
            return Err("Fill settings are outside the supported ranges".into());
        }
        if !images.is_empty() && layer.texture_fill.as_ref().is_some_and(|fill| fill.graph.is_some()) {
            return Err("Edit image sources in the fill graph".into());
        }
        let mut images_changed = false;
        for (index, image) in images {
            let before = layer.texture_fill.as_ref().and_then(|fill| fill.images[index].as_ref());
            let same = match (before, &image) {
                (None, None) => true,
                (Some(before), Some(after)) => before.name == after.name && before.bytes == after.bytes,
                _ => false,
            };
            if !same {
                Arc::make_mut(layer.texture_fill.get_or_insert_with(Arc::default)).images[index] = image;
                images_changed = true;
            }
        }
        if let Some(channels) = self.channels {
            let colors = channels.colors();
            let supplied = |index: usize| layer.texture_fill.as_ref().is_some_and(|fill| fill.images[index].is_some());
            if colors.iter().enumerate().any(|(index, value)| value.is_some() && supplied(index)) {
                return Err("A texture or graph supplies one of the requested solid channels".into());
            }
            for (index, color) in colors.into_iter().enumerate() {
                let Some([r, g, b]) = color else { continue };
                let rgba = [r, g, b, 255];
                let channel = &mut layer.channels[index];
                let same = channel.as_ref().is_some_and(|pixels| {
                    pixels.len() == size as usize * size as usize * 4
                        && pixels.chunks_exact(4).all(|pixel| pixel == rgba)
                });
                if !same {
                    *channel = Some(crate::editor::model_paint_data::solid(size, rgba));
                }
            }
        }
        if settings.variation.height != before.variation.height && settings.variation.height != 0.
            && layer.channels[3].is_none()
            && layer.texture_fill.as_ref().is_none_or(|fill| fill.images[3].is_none())
        {
            layer.channels[3] = Some(crate::editor::model_paint_data::solid(size, [128, 128, 128, 255]));
        }
        if images_changed || settings != before {
            let fill = Arc::make_mut(layer.texture_fill.get_or_insert_with(Arc::default));
            fill.settings = settings;
            fill.invalidate();
            if fill.settings == texture_fill::Settings::default() && fill.graph.is_none()
                && fill.images.iter().all(Option::is_none)
            {
                layer.texture_fill = None;
            }
        }
        Ok(())
    }
}

pub(in crate::editor) struct PaintEditJob {
    target: EditTarget,
    task: bevy::tasks::Task<Result<PaintEditData, String>>,
}

pub(in crate::editor) struct PreparedPaintEdit {
    target: EditTarget,
    data: PaintEditData,
}

pub(in crate::editor) struct PaintEditData(Prepared);

enum Prepared {
    Images(FillImages),
    Mask { layer: usize, mask: Arc<Mask> },
    Material {
        preset: ShelfMaterial,
        insertion: (usize, usize),
        selection: Option<usize>,
        layers: Vec<Layer>,
    },
}

struct EditTarget {
    root: PathBuf,
    source: PathBuf,
    binding: Option<(usize, usize)>,
    size: u32,
    stack: Stack,
    document_revision: u64,
}

impl EditTarget {
    fn capture(editor: &Editor, doc: &Document, session: &Session) -> Result<Self, String> {
        if doc.paint_pending || session.smart_loading {
            return Err("Wait for the current paint operation to finish".into());
        }
        Ok(Self {
            root: editor.project.asset_root.clone(), source: doc.source.path.clone(),
            binding: session.binding, size: session.texture_size, stack: session.stack.clone(),
            document_revision: doc.revision,
        })
    }
}

fn load_fill_images(root: &Path, files: BTreeMap<String, Option<PathBuf>>) -> Result<FillImages, String> {
    let mut bytes = 0;
    let mut texels = 0;
    let mut loaded = HashMap::<PathBuf, Arc<texture_fill::Bitmap>>::new();
    files.into_iter().map(|(key, path)| {
        let channel = CHANNEL_KEYS.iter().position(|name| *name == key)
            .ok_or_else(|| format!("Unknown fill image channel '{key}'; use list_paint_presets"))?;
        let image = if let Some(path) = path {
            let path = root.join(path);
            if let Some(image) = loaded.get(&path) {
                Some(image.clone())
            } else {
                let bitmap = texture_fill::Bitmap::open(&path)
                    .map_err(|error| format!("Could not load {key} image '{}' · {error}", path.display()))?;
                material_graph::file::check_size(&mut bytes, &mut texels, loaded.len() + 1, &bitmap)
                    .map_err(|_| "The combined fill images exceed the import limit")?;
                let bitmap = Arc::new(bitmap);
                loaded.insert(path, bitmap.clone());
                Some(bitmap)
            }
        } else {
            None
        };
        Ok((channel, image))
    }).collect()
}

impl PaintEditJob {
    pub(in crate::editor) fn poll(&mut self) -> Option<Result<PaintEditData, String>> {
        bevy::tasks::block_on(bevy::tasks::poll_once(&mut self.task))
    }

    pub(in crate::editor) fn finish(self, data: PaintEditData) -> PreparedPaintEdit {
        PreparedPaintEdit { target: self.target, data }
    }
}

impl EditArgs {
    pub(in crate::editor) fn is_history_edit(&self) -> bool {
        self.history_request().is_some()
    }

    fn history_request(&self) -> Option<(PaintAction, usize)> {
        match self.edit {
            Edit::Undo { steps } => Some((PaintAction::Undo, steps.unwrap_or(1).max(1))),
            Edit::Redo { steps } => Some((PaintAction::Redo, steps.unwrap_or(1).max(1))),
            _ => None,
        }
    }

    pub(in crate::editor) fn is_history_batch(&self) -> bool {
        self.history_request().is_some_and(|(_, steps)| steps > 1)
    }

    pub(in crate::editor) fn needs_preparation(&self) -> bool {
        match &self.edit {
            Edit::AddTexture { .. } | Edit::AddSmartMaterial { .. }
            | Edit::AddLocalMaterial { .. } | Edit::ApplyLocalMask { .. } => true,
            Edit::UpdateLayer { fill: Some(fill), .. } => fill.images.as_ref().is_some_and(|images| !images.is_empty()),
            _ => false,
        }
    }

    fn workspace(&self) -> PaintWorkspace {
        PaintWorkspace { model_id: self.model_id, set: self.set, revision: self.revision }
    }
}

pub(in crate::editor) struct PaintHistoryEdit {
    model: usize,
    source: PathBuf,
    active_set: usize,
    binding: (usize, usize),
    steps: Vec<HistoryStep>,
    requested: usize,
    applied: usize,
    action: PaintAction,
}

impl PaintHistoryEdit {
    pub(in crate::editor) fn start(
        editor: &Editor,
        editing: &mut editing::State,
        paint: &mut PaintState,
        args: &EditArgs,
    ) -> Result<Self, String> {
        let (model, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, args.workspace())?;
        if session.smart_loading || session.editing_shader().is_some() || !session.queued.is_empty() {
            return Err("Wait for the active paint operation or close the shader editor".into());
        }
        let (action, requested) = args.history_request().ok_or("Expected undo or redo")?;
        let steps = session.plan_history(doc, action, requested);
        if steps.is_empty() {
            return Err("There is no such history step for this model".into());
        }
        let binding = session.binding.ok_or("Wait for the active texture set to load")?;
        Ok(Self {
            model, source: doc.source.path.clone(), active_set: session.set,
            binding, steps, requested, applied: 0, action,
        })
    }

    pub(in crate::editor) fn poll(
        &mut self,
        editor: &Editor,
        editing: &mut editing::State,
        paint: &mut PaintState,
        now: f64,
    ) -> Option<Result<Value, String>> {
        let applied = self.applied;
        let changed = || Some(Err(format!("The paint workspace changed after {applied} history steps were applied")));
        if editor.viewing_model != Some(self.model) || editor.viewing_image.is_some() {
            return changed();
        }
        let Some(doc) = editing.documents.get_mut(&self.model) else { return changed() };
        let Some(session) = paint.sessions.get_mut(&self.model) else { return changed() };
        if doc.mode != Mode::Paint || doc.source.path != self.source || session.set != self.active_set
            || doc.paint_set_index() != self.active_set || session.editing_shader().is_some()
            || !doc.paint_target(self.active_set).is_some_and(|(object, slot)| (object.id, slot) == self.binding)
        {
            return changed();
        }
        if doc.paint_busy || doc.paint_pending || session.busy || session.binding != Some(self.binding)
            || session.smart_loading || session.mesh_loading || session.resize_request.is_some()
        {
            return None;
        }
        let Some(&step) = self.steps.get(self.applied) else {
            let mut state = snapshot(self.model, doc, Some(session), &paint.tips);
            state["history_steps_applied"] = json!(self.applied);
            state["history_steps_requested"] = json!(self.requested);
            return Some(Ok(state));
        };
        if session.history_step_set(doc, self.action) != Some(step) {
            return changed();
        }
        if step.binding != self.binding {
            session.choose_set(doc, step.set, now);
            session.busy = true;
            self.binding = step.binding;
            self.active_set = step.set;
            return None;
        }
        if let Err(error) = apply_action(session, self.action) {
            return Some(Err(error));
        }
        doc.paint_pending = true;
        self.applied += 1;
        None
    }
}

pub(in crate::editor) fn prepare_edit(
    editor: &Editor, editing: &mut editing::State, paint: &mut PaintState, bake: &McpBake, args: &EditArgs,
) -> Result<PaintEditJob, String> {
    let (model, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, args.workspace())?;
    let target = EditTarget::capture(editor, doc, session)?;
    let task = match &args.edit {
        Edit::UpdateLayer { index, fill: Some(fill), .. } => {
            let files = fill.images.clone().ok_or("No fill images were requested")?;
            check_layer(session, *index)?;
            let layer = &session.stack.layers[*index];
            if layer.kind != LayerKind::Fill {
                return Err("Fill settings require a fill layer".into());
            }
            if layer.texture_fill.as_ref().is_some_and(|fill| fill.graph.is_some()) {
                return Err("Edit image sources in the fill graph".into());
            }
            let root = target.root.clone();
            bevy::tasks::IoTaskPool::get().spawn(async move {
                load_fill_images(&root, files).map(|images| PaintEditData(Prepared::Images(images)))
            })
        }
        Edit::AddTexture { preset, name, target } => {
            let index = tips::texture_index(preset).ok_or("Unknown texture preset; use list_paint_presets")?;
            prepare_material(session, ShelfMaterial::Texture(index), name.as_deref(), *target,
                move |_| tips::texture_layer(index))?
        }
        Edit::AddSmartMaterial { preset, name, target } => {
            let index = SMART_MATERIALS.iter().position(|item| item.name.eq_ignore_ascii_case(preset))
                .ok_or("Unknown smart material; use list_paint_presets")?;
            prepare_material(session, ShelfMaterial::Smart(index), name.as_deref(), *target,
                move |size| SMART_MATERIALS[index].prepare(size))?
        }
        Edit::AddLocalMaterial { preset, name, target } => {
            let entry = paint.library.entry(*preset).ok_or("Unknown local preset; use list_paint_presets")?;
            if !matches!(entry.kind, presets::Kind::Material { .. }) {
                return Err("The local preset is a mask".into());
            }
            let path = entry.path.clone();
            let preparation = if presets::is_mpsmart(&path) {
                bake.material_preparation(model, doc, session)
            } else {
                presets::MaterialPreparation::default()
            };
            prepare_material(session, ShelfMaterial::User(*preset), name.as_deref(), *target,
                move |size| preparation.load(&path, size))?
        }
        Edit::ApplyLocalMask { preset, layer } => {
            check_layer(session, *layer)?;
            let entry = paint.library.entry(*preset).ok_or("Unknown local preset; use list_paint_presets")?;
            if !matches!(entry.kind, presets::Kind::Mask(_)) {
                return Err("The local preset is a material".into());
            }
            let path = entry.path.clone();
            let size = session.texture_size;
            let layer = *layer;
            bevy::tasks::IoTaskPool::get().spawn(async move {
                let mask = presets::load_mask(&path, size)?;
                Ok(PaintEditData(Prepared::Mask { layer, mask }))
            })
        }
        _ => return Err("This paint edit does not require background preparation".into()),
    };
    Ok(PaintEditJob { target, task })
}

fn prepare_material(
    session: &Session, preset: ShelfMaterial, name: Option<&str>, target: Option<MaterialTarget>,
    load: impl FnOnce(u32) -> Result<Vec<Layer>, String> + Send + 'static,
) -> Result<bevy::tasks::Task<Result<PaintEditData, String>>, String> {
    let name = name.map(Session::valid_layer_name).transpose()?.map(str::to_owned);
    let insertion = MaterialTarget::insertion(session, target)?;
    if !session.can_insert_material(preset, Some(insertion)) {
        return Err("The material would exceed the layer count or group depth limit".into());
    }
    let selection = target.is_none().then_some(session.stack.selected);
    let size = session.texture_size;
    Ok(bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let mut layers = load(size)?;
        let root = layers.last_mut().ok_or("The material preset has no layers")?;
        if let Some(name) = name {
            root.name = name;
        }
        Ok(PaintEditData(Prepared::Material { preset, insertion, selection, layers }))
    }))
}

pub(super) enum RasterOperation<'a> {
    Stroke(&'a [StrokeSample], Option<&'a StrokeOptions>),
    Fill(FillSource<'a>, FillSettings),
}

pub(super) enum FillSource<'a> {
    Point([f32; 2]),
    Region(&'a FillRegion),
}

pub(super) struct FillSettings {
    pub mode: Option<FillMode>,
    pub angle: Option<f32>,
    pub padding: Option<f32>,
}

pub(super) fn model_index(editor: &Editor) -> Result<usize, String> {
    editor
        .viewing_model
        .filter(|_| editor.viewing_image.is_none())
        .ok_or_else(|| "Open a 3D model in the editor first".to_string())
}

fn bake_model_index(editor: &Editor, target: BakeTarget) -> Result<usize, String> {
    let index = model_index(editor)?;
    if target.model_id != index {
        return Err("The active model changed; call get_paint_state again".into());
    }
    Ok(index)
}

pub(in crate::editor) fn bake_status(
    input: &McpBake,
    editor: &Editor,
    editing: &editing::State,
    paint: &PaintState,
    target: BakeTarget,
) -> Result<Value, String> {
    let index = bake_model_index(editor, target)?;
    let doc = editing
        .documents
        .get(&index)
        .ok_or("Wait for the model to load")?;
    let set = target.set.unwrap_or_else(|| {
        paint
            .sessions
            .get(&index)
            .filter(|_| doc.mode == Mode::Paint)
            .map_or_else(|| doc.paint_set_index(), |session| session.set)
    });
    input.status(index, doc, set)
}

pub(in crate::editor) fn start_bake(
    input: &mut McpBake,
    editor: &mut Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: BakeArgs,
) -> Result<(usize, Value), String> {
    let index = bake_model_index(editor, args.target)?;
    let doc = editing
        .documents
        .get_mut(&index)
        .ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = paint
        .sessions
        .get_mut(&index)
        .ok_or("Wait for Painting mode to load")?;
    input.start(editor, doc, session, args)
}

pub(in crate::editor) fn cancel_bake(
    input: &mut McpBake,
    editor: &mut Editor,
    editing: &mut editing::State,
    paint: &PaintState,
    target: BakeTarget,
) -> Result<Value, String> {
    let index = bake_model_index(editor, target)?;
    let doc = editing
        .documents
        .get_mut(&index)
        .ok_or("Wait for the model to load")?;
    let set = target.set.unwrap_or_else(|| {
        paint
            .sessions
            .get(&index)
            .map_or_else(|| doc.paint_set_index(), |session| session.set)
    });
    input.cancel(editor, doc, index, set)
}

pub(in crate::editor) fn set_bake_levels(
    input: &mut McpBake,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: BakeLevelsArgs,
) -> Result<Value, String> {
    let index = bake_model_index(editor, args.target)?;
    let doc = editing.documents.get_mut(&index).ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = paint.sessions.get_mut(&index).ok_or("Wait for Painting mode to load")?;
    input.set_levels(index, doc, session, args)
}

fn output_document<'a>(
    output: &McpOutput,
    editor: &Editor,
    editing: &'a editing::State,
    paint: &'a PaintState,
    requested_model: usize,
) -> Result<(&'a Document, &'a Session), String> {
    let index = model_index(editor)?;
    if index != requested_model {
        return Err("The active model changed; call get_paint_state again".into());
    }
    let doc = editing
        .documents
        .get(&index)
        .ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = paint
        .sessions
        .get(&index)
        .ok_or("Wait for Painting mode to load")?;
    if doc.paint_busy
        || doc.paint_pending
        || session.busy
        || session.smart_loading
        || output.stroke_active()
    {
        return Err("Wait for the current paint operation to finish before writing files".into());
    }
    Ok((doc, session))
}

pub(in crate::editor) fn save_model(
    output: &mut McpOutput,
    editor: &mut Editor,
    editing: &mut editing::State,
    paint: &PaintState,
    args: SaveArgs,
    reply: OutputReply,
) {
    let (doc, _) = match output_document(output, editor, editing, paint, args.model_id) {
        Ok(document) => document,
        Err(error) => return reply(Err(error)),
    };
    if let Some(name) = args.copy_name.as_deref() {
        let task = save_copy::prepare_copy(doc, &editor.project.asset_root, name)
            .map(|write| move || write().map(|path| json!({"path": path, "copy": true})));
        output.write(task, reply);
    } else {
        editing.start_save_with_reply(
            args.model_id,
            editor,
            Some(Box::new(move |result| {
                reply(result.map(|path| json!({"path": path, "copy": false})))
            })),
        );
    }
}

pub(in crate::editor) fn export_textures(
    output: &mut McpOutput,
    editor: &Editor,
    editing: &editing::State,
    paint: &PaintState,
    args: ExportArgs,
    reply: OutputReply,
) {
    let job = output_document(output, editor, editing, paint, args.model_id).and_then(
        |(doc, session)| {
            let binding = doc
                .paint_target(session.set)
                .map(|(object, slot)| (object.id, slot))
                .filter(|key| Some(*key) == session.binding);
            output.prepare_export(editor, doc, binding, args)
        },
    );
    output.write(job, reply);
}

pub(in crate::editor) fn state(
    editor: &Editor,
    editing: &editing::State,
    paint: &PaintState,
    preview: &McpPreview,
) -> Result<Value, String> {
    let index = model_index(editor)?;
    let doc = editing
        .documents
        .get(&index)
        .ok_or_else(|| editing.errors.get(&index).cloned()
            .unwrap_or_else(|| "Wait for the model to load".into()))?;
    let session = paint.sessions.get(&index);
    let mut response = snapshot(index, doc, session, &paint.tips);
    if let Some(session) = session {
        response["view"]["trace_progress"] = preview.trace_progress(index, doc, session);
        response["view"]["live_preview"] = preview.live_progress(index, doc, session);
    }
    Ok(response)
}

pub(in crate::editor) fn save_preset(
    editor: &Editor, editing: &mut editing::State, paint: &mut PaintState,
    args: SavePresetArgs, reply: OutputReply,
) {
    let capture = capture_preset(editor, editing, paint, args.workspace, args.layer, args.kind, args.name.as_deref());
    match capture {
        Ok(capture) => paint.library.save_with_reply(capture, Box::new(move |result| {
            reply(result.map(|(index, entry, omitted)| json!({
                "index": index, "name": entry.name, "kind": args.kind, "omitted": omitted,
            })));
        })),
        Err(error) => reply(Err(error)),
    }
}

pub(in crate::editor) fn import_preset(paint: &mut PaintState, args: ImportPresetArgs, reply: OutputReply) {
    if !args.source.is_absolute() {
        reply(Err("Use an absolute path to a Material Painter preset or ZIP package".into()));
        return;
    }
    paint.library.import_with_reply(args.source, Box::new(move |result| {
        reply(result.map(|(index, entry, omitted)| json!({
            "index": index, "name": entry.name, "kind": "material", "omitted": omitted,
        })));
    }));
}

pub(in crate::editor) fn export_preset(
    editor: &Editor, editing: &mut editing::State, paint: &mut PaintState,
    args: ExportPresetArgs, reply: OutputReply,
) {
    let prepared = (|| -> Result<_, String> {
        let folder = export::output_folder(&editor.project.asset_root, &args.folder)?;
        let capture = capture_preset(
            editor, editing, paint, args.workspace, args.layer, presets::CaptureKind::Material, args.name.as_deref(),
        )?;
        Ok((folder, capture))
    })();
    match prepared {
        Ok((folder, capture)) => paint.library.export_with_reply(
            capture, matches!(args.format, PresetFormat::Zip), folder, Box::new(move |result| {
                reply(result.map(|file| json!({"path": file.path, "format": args.format,
                    "omitted": file.omitted, "adjusted": file.adjusted})));
            }),
        ),
        Err(error) => reply(Err(error)),
    }
}

fn capture_preset(
    editor: &Editor, editing: &mut editing::State, paint: &mut PaintState,
    workspace: PaintWorkspace, layer: usize, kind: presets::CaptureKind, name: Option<&str>,
) -> Result<presets::Capture, String> {
    let (_, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, workspace)?;
    if doc.paint_pending || session.smart_loading {
        return Err("Wait for the current paint operation to finish".into());
    }
    let name = name.map(Session::valid_layer_name).transpose()?;
    Ok(session.capture_shelf_preset(layer, kind, name)?)
}

#[cfg(test)]
pub(in crate::editor) fn seed_local_presets(paint: &mut PaintState, directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    let source = directory.join("source.mpsmart");
    presets::mpsmart_graph_fixture(&source);
    let (entry, _) = presets::import_material(&directory.join("shelf"), &source).unwrap();
    paint.library.upsert(entry);
    let mask = Arc::new(Mask::new(crate::editor::model_paint_data::solid(16, [96, 96, 96, 255]), Vec::new()));
    let (entry, _) = presets::save(
        &directory.join("shelf"), presets::Capture::Mask { name: "Local gray mask".into(), mask },
    ).unwrap();
    paint.library.upsert(entry);
    paint.library.root = Some(directory.join("shelf"));
}

#[cfg(test)]
fn apply(
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: EditArgs,
    now: f64,
) -> Result<Value, String> {
    apply_prepared(editor, editing, paint, args, now, None)
}

pub(in crate::editor) fn apply_prepared(
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: EditArgs,
    now: f64,
    prepared: Option<PreparedPaintEdit>,
) -> Result<Value, String> {
    let (index, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, args.workspace())?;
    let prepared = match prepared {
        Some(PreparedPaintEdit { target, data }) => {
            if target.root != editor.project.asset_root || target.source != doc.source.path
                || target.binding != session.binding || target.size != session.texture_size
                || doc.paint_pending || session.smart_loading
                || !crate::editor::model_paint_data::same_layer_list(&target.stack, &session.stack)
            {
                return Err("The paint workspace changed during preparation; call get_paint_state again".into());
            }
            if matches!(data.0, Prepared::Material { preset: ShelfMaterial::User(_), .. })
                && target.document_revision != doc.revision
            {
                return Err("The model changed during material preparation; call get_paint_state again".into());
            }
            Some(data.0)
        }
        None if args.needs_preparation() => return Err("This paint edit has not been prepared".into()),
        None => None,
    };
    match prepared {
        Some(Prepared::Images(images)) => perform(doc, session, args.edit, now, images)?,
        Some(Prepared::Mask { layer, mask }) => {
            session.apply_shelf_mask(mask, Some(layer));
            doc.paint_pending = true;
        }
        Some(Prepared::Material { preset, insertion, selection, layers }) => {
            if selection.is_some_and(|index| session.stack.selected != index)
                || !session.can_insert_material(preset, Some(insertion))
            {
                return Err("The material destination changed; call get_paint_state again".into());
            }
            if !session.can_insert_layers(&layers, insertion) {
                return Err("The material would exceed the layer count or group depth limit".into());
            }
            session.insert_smart_material(layers, insertion);
            doc.paint_pending = true;
        }
        None => perform(doc, session, args.edit, now, Vec::new())?,
    }
    Ok(snapshot(index, doc, Some(session), &paint.tips))
}

fn bound_workspace<'a>(
    editor: &Editor,
    editing: &'a mut editing::State,
    sessions: &'a mut HashMap<usize, Session>,
    workspace: PaintWorkspace,
) -> Result<(usize, &'a mut Document, &'a mut Session), String> {
    let index = model_index(editor)?;
    if index != workspace.model_id {
        return Err("The active model changed; call get_paint_state again".into());
    }
    let doc = editing
        .documents
        .get_mut(&index)
        .ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = sessions
        .get_mut(&index)
        .ok_or("Wait for Painting mode to load")?;
    if session.set != workspace.set
        || workspace
            .revision
            .is_some_and(|revision| revision != session.revision)
    {
        return Err("The paint workspace changed; call get_paint_state again".into());
    }
    if doc.paint_busy
        || session.busy
        || session.mesh_loading
        || session.resize_request.is_some()
        || !doc
            .paint_target(workspace.set)
            .is_some_and(|(object, slot)| session.binding == Some((object.id, slot)))
    {
        return Err("Wait for the active texture set or paint operation to finish".into());
    }
    Ok((index, doc, session))
}

pub(in crate::editor) fn replace_mesh(
    requests: &mut mesh_tools::Requests,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: MeshArgs,
    reply: Box<dyn mesh_tools::Reply>,
) {
    use mesh_tools::Operation;

    if requests.0.is_some() {
        reply.finish(Err("Wait for the current mesh replacement to finish".into()));
        return;
    }
    let (index, _, session) = match bound_workspace(editor, editing, &mut paint.sessions, args.workspace) {
        Ok(workspace) => workspace,
        Err(error) => {
            reply.finish(Err(error));
            return;
        }
    };
    let operation = match args.replacement {
        MeshOperation::Primitive { shape } => Operation::Primitive(shape),
        MeshOperation::File { path } => Operation::Import(Some(editor.project.asset_root.join(path))),
        MeshOperation::Undo => Operation::Undo,
        MeshOperation::Redo => Operation::Redo,
    };
    requests.0 = Some(mesh_tools::Request {
        model: index,
        set: session.set,
        revision: session.revision,
        operation,
        reply,
    });
}

pub(in crate::editor) fn stroke(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: StrokeArgs,
) -> Result<Value, String> {
    raster(
        input,
        editor,
        editing,
        paint,
        args.workspace,
        args.target,
        args.brush,
        RasterOperation::Stroke(&args.samples, args.style.as_ref()),
    )
}

pub(in crate::editor) fn fill(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: FillArgs,
) -> Result<Value, String> {
    let source = match (args.position, &args.region) {
        (Some(position), None) => FillSource::Point(position),
        (None, Some(region)) => FillSource::Region(region),
        _ => return Err("Specify either a fill position or a region".into()),
    };
    raster(
        input,
        editor,
        editing,
        paint,
        args.workspace,
        args.target,
        args.brush,
        RasterOperation::Fill(source, FillSettings {
            mode: args.mode, angle: args.angle_degrees, padding: args.padding_px,
        }),
    )
}

pub(in crate::editor) enum SurfaceResponse {
    Running(Value),
    Wait(SurfaceWait),
}

pub(in crate::editor) struct SurfaceWait {
    model: usize,
    set: usize,
    binding: sets::SetKey,
}

impl SurfaceWait {
    pub fn poll(&self, editor: &Editor, editing: &editing::State, paint: &PaintState) -> Option<Result<Value, String>> {
        let workspace = editing.documents.get(&self.model).zip(paint.sessions.get(&self.model))
            .filter(|(doc, session)| {
                model_index(editor).ok() == Some(self.model) && doc.mode == Mode::Paint
                    && session.set == self.set && session.binding == Some(self.binding)
            });
        let Some((doc, session)) = workspace else {
            return Some(Err("The paint workspace changed while waiting for surface selection".into()));
        };
        if paint_work_pending(doc, Some(session)) {
            return None;
        }
        let result = match session.surface_selection.as_ref() {
            Some((binding, Ok(_))) if *binding == self.binding => {
                Ok(snapshot(self.model, doc, Some(session), &paint.tips))
            }
            Some((binding, Err(error))) if *binding == self.binding => Err(error.clone()),
            _ => Err("The surface selection was interrupted before publishing its result".into()),
        };
        Some(result)
    }
}

pub(in crate::editor) fn select_surface(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: SurfaceArgs,
) -> Result<SurfaceResponse, String> {
    if editor.browser_popup || editor.hdri_library_open {
        return Err("Close the open editor overlay before selecting a surface".into());
    }
    let crease_angle = args.crease_angle.unwrap_or(30.);
    if !(1. ..=179.).contains(&crease_angle) {
        return Err("The crease angle must be between 1 and 179 degrees".into());
    }
    let (index, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, args.workspace)?;
    let binding = session.binding.ok_or("Wait for the active texture set to load")?;
    if args.topology.is_some_and(|topology| topology != SurfaceTopology::new(doc, session)) {
        return Err("The surface topology changed; call list_paint_surface_parts again".into());
    }
    let source = input.surface_source(index, session, args.source)?;
    input.submit_surface(index, doc, session, args.layer, surface_selection::Selection {
        source,
        crease_angle,
        stop_at: args.stop_at,
        mode: args.mode,
    })?;
    if args.wait {
        Ok(SurfaceResponse::Wait(SurfaceWait { model: index, set: args.workspace.set, binding }))
    } else {
        let mut result = snapshot(index, doc, Some(session), &paint.tips);
        result["surface_selection"] = json!({"status": "running"});
        Ok(SurfaceResponse::Running(result))
    }
}

pub(in crate::editor) fn list_surface_parts(
    editor: &Editor, editing: &editing::State, paint: &PaintState, args: SurfacePartsArgs, reply: OutputReply,
) {
    let snapshot = (|| {
        let model_id = model_index(editor)?;
        let workspace = args.workspace;
        if model_id != workspace.model_id {
            return Err("The active model changed; call get_paint_state again".into());
        }
        let doc = editing.documents.get(&model_id).filter(|doc| doc.mode == Mode::Paint)
            .ok_or("Switch the model to Painting mode first")?;
        let session = paint.sessions.get(&model_id).ok_or("Wait for Painting mode to load")?;
        if session.set != workspace.set || workspace.revision.is_some_and(|rev| rev != session.revision) {
            return Err("The paint workspace changed; call get_paint_state again".into());
        }
        let (object, slot) = doc.paint_target(workspace.set).ok_or("Unknown texture set")?;
        let target = (object.id, slot);
        if session.binding != Some(target) || session.mesh_loading {
            return Err("Wait for the active texture set to load".into());
        }
        let limit = args.limit.unwrap_or(100);
        if !(1..=250).contains(&limit) {
            return Err("The surface part limit must be between 1 and 250".into());
        }
        Ok((doc.objects.clone(), target, session.set_appearance.visibility.clone(),
            SurfaceTopology::new(doc, session), limit, session.revision))
    })();
    let (objects, target, visibility, topology, limit, revision) = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => return reply(Err(error)),
    };
    bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let (parts, total) = surface_selection::list_parts(&objects, target, &visibility, args.offset, limit);
        let next = args.offset.saturating_add(parts.len());
        reply(Ok(json!({
            "workspace": {"model_id": args.workspace.model_id, "set": args.workspace.set, "revision": revision},
            "topology": topology, "parts": parts, "total": total,
            "next_offset": (next < total).then_some(next),
        })));
    }).detach();
}

fn raster(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    workspace: PaintWorkspace,
    target: RasterTarget,
    brush: BrushValues,
    operation: RasterOperation,
) -> Result<Value, String> {
    if editor.browser_popup || editor.hdri_library_open {
        return Err("Close the open editor overlay before painting".into());
    }
    let (index, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, workspace)?;
    input.submit(index, doc, session, &mut paint.tips, target, brush, operation)?;
    Ok(snapshot(index, doc, Some(session), &paint.tips))
}

fn perform(doc: &mut Document, session: &mut Session, edit: Edit, now: f64, images: FillImages) -> Result<(), String> {
    let before = session.stack.clone();
    match edit {
        Edit::SelectSet { index } => {
            if !session.choose_set(doc, index, now) {
                return Err("Unknown texture set index".into());
            }
        }
        Edit::UpdateTextureSet { index, resolution, visible, highlighted } => {
            update_texture_set(doc, session, index, resolution, visible, highlighted)?;
        }
        Edit::SelectLayer { index, mask } => {
            check_layer(session, index)?;
            let action = if mask {
                PaintAction::SelectLayerMask(index)
            } else {
                PaintAction::Select(index)
            };
            apply_action(session, action)?;
        }
        Edit::AddLayer { kind, name, target } => {
            let name = name.as_deref().map(Session::valid_layer_name).transpose()?;
            let insertion = MaterialTarget::insertion(session, target)?;
            if !session.can_insert_layer(insertion) {
                return Err("The layer would exceed the layer count or group depth limit".into());
            }
            session.checkpoint(history::Label::AddLayer);
            session.insert_new_layer(kind, name, insertion);
            session.revision += 1;
        }
        Edit::AddMaterial { preset, name, target } => {
            let index = MATERIALS
                .iter()
                .position(|material| material.name.eq_ignore_ascii_case(&preset))
                .ok_or("Unknown material preset; use list_paint_presets")?;
            let name = name.as_deref().map(Session::valid_layer_name).transpose()?;
            let insertion = MaterialTarget::insertion(session, target)?;
            if !session.can_insert_material(ShelfMaterial::Basic(index), Some(insertion)) {
                return Err("The material would exceed the layer count or group depth limit".into());
            }
            session.checkpoint(history::Label::AddMaterial);
            session.insert_basic_material(index, name, insertion);
            session.revision += 1;
        }
        Edit::AddTexture { .. } | Edit::AddSmartMaterial { .. }
        | Edit::AddLocalMaterial { .. } | Edit::ApplyLocalMask { .. } => {
            return Err("The material preset has not been prepared".into());
        }
        Edit::UpdateLayer {
            index,
            name,
            visible,
            opacity_percent,
            blend,
            fill,
        } => {
            check_layer(session, index)?;
            let name = name.as_deref().map(Session::valid_layer_name).transpose()?;
            if opacity_percent.is_some_and(|value| !(0. ..=100.).contains(&value)) {
                return Err("Layer opacity must be between 0 and 100 percent".into());
            }
            let layer = &session.stack.layers[index];
            if blend.is_some() && layer.kind == LayerKind::Folder {
                return Err("Folders do not have a layer blend mode".into());
            }
            let mut updated = layer.clone();
            if let Some(name) = name {
                updated.name = name.to_owned();
            }
            if let Some(value) = visible {
                updated.visible = value;
            }
            if let Some(value) = opacity_percent {
                updated.opacity = value;
            }
            if let Some(value) = blend {
                updated.blend = value;
            }
            if let Some(fill) = fill {
                fill.apply(&mut updated, session.texture_size, images)?;
            }
            if updated != *layer {
                session.checkpoint(history::Label::LayerProperties);
                session.stack.layers[index] = updated;
                session.revision += 1;
            }
        }
        Edit::DuplicateLayer { index } => apply_to_layer(session, index, PaintAction::Duplicate)?,
        Edit::RemoveLayer { index } => apply_to_layer(session, index, PaintAction::Remove)?,
        Edit::MoveLayer {
            from,
            to,
            placement,
        } => {
            let target = match placement {
                Placement::Above => ShelfPlacement::Above(to),
                Placement::Below => ShelfPlacement::Below(to),
                Placement::Inside => ShelfPlacement::Inside(to),
            };
            apply_action(session, PaintAction::MoveLayer(from, target))?;
        }
        Edit::SetMask { index, mask } => {
            let action = match mask {
                MaskBase::White => PaintAction::AddMask(true),
                MaskBase::Black => PaintAction::AddMask(false),
                MaskBase::Remove => PaintAction::RemoveMask,
            };
            apply_to_layer(session, index, action)?;
        }
        Edit::ApplyGenerator { index, generator, parameters } => {
            check_layer(session, index)?;
            if !session.available(PaintAction::ApplyGenerator(generator, Some(index))) {
                return Err("The target layer is unavailable for generator changes".into());
            }
            let mask = session.stack.layers[index].mask.as_deref();
            let value = parameters.apply(new_generator(doc, generator)?, mask_palette(mask))?;
            if mask.and_then(|mask| mask.generator) != Some(value) {
                session.apply_generator_value(value, Some(index));
            }
            session.stack.reveal_selection();
        }
        Edit::UpdateGenerator { index, mask_layer, parameters } => {
            edit_mask(session, index, false, history::Label::Mask, |mask| {
                let previous = match mask_layer {
                    Some(index) => mask.layers.get(index).ok_or("Unknown mask layer index")?.generator,
                    None => mask.generator,
                }.ok_or("There is no generator on the requested mask target")?;
                let value = parameters.apply(previous, mask_palette(Some(mask)))?;
                match mask_layer {
                    Some(index) => mask.edit_layers()[index].generator = Some(value),
                    None => *mask.edit_generator() = Some(value),
                }
                Ok(())
            })?;
        }
        Edit::AddMaskLayer { index, source, parameters } => {
            edit_mask(session, index, true, history::Label::Mask, |mask| {
                if mask.layers.len() >= MAX_MASK_LAYERS {
                    return Err("The mask already has the maximum number of layers".into());
                }
                let mut layer = MaskLayer::default();
                if let MaskLayerSource::Generator { generator, parameters } = source {
                    layer.generator = Some(parameters.apply(new_generator(doc, generator)?, mask_palette(Some(mask)))?);
                    layer.blend = MaskBlend::Multiply;
                }
                parameters.apply(&mut layer)?;
                mask.edit_layers().push(layer);
                Ok(())
            })?;
        }
        Edit::UpdateMaskLayer { index, mask_layer, parameters } => {
            edit_mask(session, index, false, history::Label::Mask, |mask| {
                let layer = mask.edit_layers().get_mut(mask_layer).ok_or("Unknown mask layer index")?;
                parameters.apply(layer)
            })?;
        }
        Edit::MoveMaskLayer { index, from, to } => {
            edit_mask(session, index, false, history::Label::Mask, |mask| {
                let layers = mask.edit_layers();
                if from >= layers.len() || to >= layers.len() {
                    return Err("Unknown mask layer index".into());
                }
                let layer = layers.remove(from);
                layers.insert(to, layer);
                Ok(())
            })?;
        }
        Edit::RemoveMaskLayer { index, mask_layer } => {
            edit_mask(session, index, false, history::Label::Mask, |mask| {
                let layers = mask.edit_layers();
                if mask_layer >= layers.len() {
                    return Err("Unknown mask layer index".into());
                }
                layers.remove(mask_layer);
                Ok(())
            })?;
        }
        Edit::AddMaskFilter { index, filter, parameters } => {
            let filter = parameters.apply(Filter::new(filter))?;
            edit_mask_filters(session, index, true, |filters| {
                if filters.len() >= MAX_FILTERS {
                    return Err("The mask already has the maximum number of filters".into());
                }
                filters.push(filter);
                Ok(())
            })?;
        }
        Edit::UpdateMaskFilter { index, filter_index, parameters } => {
            edit_mask_filters(session, index, false, |filters| {
                let filter = filters.get_mut(filter_index).ok_or("Unknown mask filter index")?;
                *filter = parameters.apply(*filter)?;
                Ok(())
            })?;
        }
        Edit::MoveMaskFilter { index, from, to } => {
            edit_mask_filters(session, index, false, |filters| {
                if from >= filters.len() || to >= filters.len() {
                    return Err("Unknown mask filter index".into());
                }
                let filter = filters.remove(from);
                filters.insert(to, filter);
                Ok(())
            })?;
        }
        Edit::RemoveMaskFilter { index, filter_index } => {
            edit_mask_filters(session, index, false, |filters| {
                if filter_index >= filters.len() {
                    return Err("Unknown mask filter index".into());
                }
                filters.remove(filter_index);
                Ok(())
            })?;
        }
        Edit::Undo { .. } => apply_action(session, PaintAction::Undo)?,
        Edit::Redo { .. } => apply_action(session, PaintAction::Redo)?,
    }
    if !crate::editor::model_paint_data::same_layer_list(&before, &session.stack) {
        doc.paint_pending = true;
    }
    Ok(())
}

fn mask_palette(mask: Option<&Mask>) -> &[[u8; 3]] {
    mask.and_then(|mask| mask.context.as_deref()).map_or(&[], |context| context.id_palette())
}

fn update_texture_set(
    doc: &mut Document, session: &mut Session, index: usize, resolution: Option<u32>,
    visible: Option<bool>, highlighted: Option<bool>,
) -> Result<(), String> {
    use crate::editor::model_paint_data::RESOLUTIONS;
    let (object, slot) = doc.paint_target(index).ok_or("Unknown texture set index")?;
    let key = (object.id, slot);
    let visibility_change = if let Some(visible) = visible {
        if visible && object.hidden {
            return Err("Show the parent object before showing its texture set".into());
        }
        !object.hidden && visible != session.set_appearance.visibility.shown(key.0, key.1)
    } else {
        false
    };
    let resize = if let Some(size) = resolution {
        if !RESOLUTIONS.contains(&size) {
            return Err("Texture set resolution must be 512, 1024, 2048 or 4096".into());
        }
        if index != session.set || session.binding != Some(key) {
            return Err("Select the requested texture set before resizing it".into());
        }
        (size != session.texture_size).then_some(PaintAction::Resolution(size))
    } else {
        None
    };
    if resize.is_some_and(|action| doc.paint_pending || session.smart_loading || !session.available(action)) {
        return Err("Wait for the current paint operation to finish".into());
    }
    if visibility_change {
        session.set_appearance.count(doc);
        if !session.available(PaintAction::SetHide(key.0, key.1)) {
            return Err("At least one texture set must remain visible".into());
        }
        session.edit(PaintAction::SetHide(key.0, key.1));
        session.set_appearance.count(doc);
    }
    if highlighted.is_some_and(|highlighted| highlighted != session.set_appearance.pinned.contains(&key)) {
        session.edit(PaintAction::SetPin(key.0, key.1));
    }
    if let Some(action) = resize {
        session.edit(action);
        session.mark_resize_pending(doc);
    }
    Ok(())
}

fn new_generator(doc: &Document, kind: GeneratorKind) -> Result<Generator, String> {
    let mut generator = Generator::new(kind);
    if kind == GeneratorKind::CustomMap {
        let recipe = doc.custom_bakes.first().ok_or("Create a custom bake map before using that generator")?;
        generator.custom[0] = custom::Input { id: recipe.id, weight: 100 };
    }
    Ok(generator)
}

fn edit_mask_filters(
    session: &mut Session,
    index: usize,
    select: bool,
    edit: impl FnOnce(&mut Vec<Filter>) -> Result<(), String>,
) -> Result<(), String> {
    edit_mask(session, index, select, history::Label::Filter, |mask| edit(mask.edit_filters()))
}

fn edit_mask(
    session: &mut Session,
    index: usize,
    select: bool,
    label: history::Label,
    edit: impl FnOnce(&mut Mask) -> Result<(), String>,
) -> Result<(), String> {
    check_layer(session, index)?;
    let mask = session.stack.layers[index].mask.as_ref().ok_or("The layer has no mask")?;
    let mut updated = (**mask).clone();
    edit(&mut updated)?;
    if !updated.same_settings(mask) {
        if select && session.stack.selected != index {
            session.select_layer(index);
        }
        session.checkpoint(label);
        session.stack.layers[index].mask = Some(Arc::new(updated));
        session.revision += 1;
    }
    Ok(())
}

fn check_layer(session: &Session, index: usize) -> Result<(), String> {
    if index >= session.stack.layers.len() {
        return Err("Unknown layer index".into());
    }
    Ok(())
}

fn apply_action(session: &mut Session, action: PaintAction) -> Result<(), String> {
    if !session.available(action) {
        return Err("That operation is unavailable for the current paint stack".into());
    }
    session.edit(action);
    Ok(())
}

fn apply_to_layer(session: &mut Session, layer: usize, action: PaintAction) -> Result<(), String> {
    check_layer(session, layer)?;
    let previous = std::mem::replace(&mut session.stack.selected, layer);
    let available = session.available(action);
    session.stack.selected = previous;
    if !available {
        return Err("That operation is unavailable for the requested layer".into());
    }
    if previous != layer {
        session.select_layer(layer);
    }
    session.edit(action);
    Ok(())
}

fn paint_work_pending(doc: &Document, session: Option<&Session>) -> bool {
    doc.paint_busy || doc.paint_pending || session.is_some_and(|session| {
        session.busy || session.smart_loading || session.mesh_loading || session.resize_request.is_some()
    })
}

fn snapshot(
    index: usize,
    doc: &Document,
    session: Option<&Session>,
    tips: &tips::Library,
) -> Value {
    let session = session.filter(|_| doc.mode == Mode::Paint);
    let active = session.map_or_else(|| doc.paint_set_index(), |session| session.set);
    let target = doc.paint_target(active);
    let bound = session.filter(|session| {
        session.binding.is_some()
            && session.binding == target.map(|(object, slot)| (object.id, slot))
    });
    let texture_sets = doc.objects.iter()
        .flat_map(|object| object.materials.iter().enumerate().map(move |(slot, material)| (object, slot, material)))
        .enumerate()
        .map(|(index, (object, slot, material))| {
            let key = (object.id, slot);
            let shown = session.is_none_or(|s| s.set_appearance.visibility.shown(key.0, key.1));
            json!({
                "index": index,
                "object_id": object.id,
                "object": object.name,
                "material_slot": slot,
                "material": material.name(&doc.source.json).unwrap_or("Материал"),
                "udim": material.udim().map(|tile| tile.number()),
                "resolution": bound.filter(|_| index == active).map_or(doc.paint_size(material), |s| s.texture_size),
                "visible": !object.hidden && shown,
                "highlighted": session.is_some_and(|s| s.set_appearance.pinned.contains(&key)),
            })
        })
        .collect::<Vec<_>>();
    let saved_stack = target
        .and_then(|(object, slot)| object.materials[slot].paint.as_deref())
        .map(|surface| &surface.stack);
    let stack = bound.map(|s| &s.stack).or(saved_stack);
    let triangles: usize = doc
        .objects
        .iter()
        .flat_map(|o| o.geometry.faces.iter())
        .map(|face| face.corners.len().saturating_sub(2))
        .sum();
    let history = |action| {
        let steps = session.map_or_else(Vec::new, |session| session.plan_history(doc, action, 10));
        steps.into_iter().map(|step| {
            let (operation, label) = step.label.names();
            json!({"set": step.set, "operation": operation, "label": label})
        }).collect::<Vec<_>>()
    };
    let undo = history(PaintAction::Undo);
    let redo = history(PaintAction::Redo);
    json!({
        "model_id": index,
        "revision": session.map(|s| s.revision),
        "mesh": {
            "name": doc.source.path.file_name().map(|name| name.to_string_lossy()),
            "triangles": triangles,
            "loading": session.is_some_and(|s| s.mesh_loading),
            "can_undo_replacement": doc.can_restore_paint_mesh(false),
            "can_redo_replacement": doc.can_restore_paint_mesh(true),
        },
        "mode": match doc.mode { Mode::Object => "object", Mode::Edit => "edit", Mode::Uv => "uv", Mode::Paint => "paint" },
        "dirty": doc.dirty(),
        "paint_busy": paint_work_pending(doc, session),
        "surface_selection": bound.and_then(|session| {
            let (binding, result) = session.surface_selection.as_ref()?;
            (session.binding == Some(*binding)).then(|| match result {
                Ok(report) => json!({
                    "status": "complete",
                    "triangles_selected": report.selected,
                    "triangles_total": report.total,
                    "triangles_in_active_set": report.active_set,
                    "bounds": report.bounds.map(|bounds| bounds.map(|point| point.to_array())),
                }),
                Err(error) => json!({"status": "failed", "error": error}),
            })
        }),
        "texture_sets": texture_sets,
        "active_set": target.map(|_| active),
        "resolution": texture_sets.get(active).map(|set| &set["resolution"]),
        "layers_bottom_to_top": stack.map_or_else(Vec::new, layer_rows),
        "view": session.map(|s| json!({"mode": s.view, "path_trace": s.path_trace})),
        "brush": session.map(|s| brush(s, tips)),
        "bake": {
            "busy": bound.is_some_and(|s| s.bake_busy),
            "ready": bound.is_some_and(|s| s.bake_ready),
            "quality": doc.bake_settings.quality,
            "id_source": doc.bake_settings.id_source,
            "match_names": doc.bake_settings.match_names,
            "has_high_poly": doc.bake_settings.high_poly.is_some(),
            "has_cage": doc.bake_settings.cage.is_some(),
        },
        "undo_steps": bound.map_or(0, |s| s.undo.len()),
        "redo_steps": bound.map_or(0, |s| s.redo.len()),
        "next_undo_set": undo.first().map(|step| &step["set"]),
        "next_redo_set": redo.first().map(|step| &step["set"]),
        "undo_history": undo,
        "redo_history": redo,
    })
}

fn layer_rows(stack: &Stack) -> Vec<Value> {
    stack.layers.iter().enumerate().map(|(index, layer)| {
        let channels = CHANNEL_KEYS.iter().enumerate()
            .filter_map(|(i, key)| layer.channel(i).is_some().then_some(key)).collect::<Vec<_>>();
        let mask = layer.mask.as_ref().map(|mask| json!({
            "generator": mask.generator,
            "filter_count": mask.filters.len(),
            "filters": mask.filters.iter().enumerate().map(|(index, filter)| json!({
                "index": index, "kind": filter.kind, "enabled": filter.enabled,
                "controls": (0..filter.kind.controls().len()).map(|index| filter.value(index)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "layer_count": mask.layers.len(),
            "layer_generators": mask.layers.iter().map(|layer| layer.generator).collect::<Vec<_>>(),
            "layers_bottom_to_top": mask.layers.iter().enumerate().map(|(index, layer)| json!({
                "index": index, "kind": if layer.generator.is_some() { "generator" } else { "fill" },
                "enabled": layer.enabled, "blend": layer.blend, "opacity_percent": layer.opacity,
                "value_percent": layer.generator.is_none().then_some(layer.value), "generator": layer.generator,
            })).collect::<Vec<_>>(),
            "id_palette": mask_palette(Some(mask)),
            "has_graph": mask.graph.is_some(),
        }));
        json!({
            "index": index,
            "name": layer.name,
            "visible": layer.visible,
            "shown": stack.shown(index),
            "opacity_percent": layer.opacity,
            "blend": layer.blend,
            "kind": match layer.kind { LayerKind::Paint => "paint", LayerKind::Fill => "fill", LayerKind::Folder => "folder" },
            "folder": stack.parents(index).next(),
            "layers_inside": (layer.kind == LayerKind::Folder).then(|| stack.block(index).len() - 1),
            "collapsed": layer.collapsed,
            "channels": channels,
            "fill": (layer.kind == LayerKind::Fill).then(|| {
                let settings = layer.texture_fill.as_ref().map(|fill| fill.settings).unwrap_or_default();
                let textured_channels = layer.texture_fill.as_ref().map(|fill| {
                    CHANNEL_KEYS.iter().zip(&fill.images)
                        .filter_map(|(name, image)| image.as_ref().map(|_| *name)).collect::<Vec<_>>()
                }).unwrap_or_default();
                json!({"settings": settings, "textured_channels": textured_channels})
            }),
            "mask": mask,
            "selected": index == stack.selected,
        })
    }).collect()
}

fn brush(session: &Session, tips: &tips::Library) -> Value {
    let channels = CHANNEL_KEYS
        .iter()
        .enumerate()
        .filter_map(|(index, key)| (session.stroke_channels() & (1 << index) != 0).then_some(key))
        .collect::<Vec<_>>();
    json!({
        "tool": session.tool,
        "size_px": session.size,
        "flow_percent": session.flow,
        "opacity_percent": session.opacity,
        "stroke_opacity_percent": session.opacity,
        "opacity_channel_percent": session.brush_alpha,
        "hardness_percent": session.hardness,
        "spacing_percent": session.spacing,
        "stabilizer": session.stabilizer,
        "color_rgb8": session.brush_color(),
        "pressure_size": session.pressure_size,
        "pressure_flow": session.pressure_flow,
        "projection": session.projection,
        "symmetry": session.symmetry,
        "paint_through": session.paint_through,
        "angle_limit_degrees": session.angle_limit,
        "tip": session.tip.and_then(|i| tips.entries.get(i)).map(|tip| &tip.name),
        "tip_angle_degrees": session.tip_angle,
        "follow_stroke": session.follow_stroke,
        "grain": session.grain.and_then(|i| tips.grains.get(i)).map(|grain| &grain.name),
        "grain_strength_percent": session.grain_strength,
        "grain_scale": session.grain_scale,
        "grain_angle_degrees": session.grain_angle,
        "grain_invert": session.grain_invert,
        "grain_uv": session.grain_uv,
        "scatter_percent": session.brush_scatter,
        "size_jitter_percent": session.brush_size_jitter,
        "angle_jitter_percent": session.brush_angle_jitter,
        "target": if session.editing_mask && session.has_mask() { "mask" } else { "material" },
        "channels": channels,
        "fill_mode": session.fill_mode,
    })
}

pub(in crate::editor) fn catalog(paint: &PaintState, export_presets: Vec<Value>) -> Value {
    json!({
        "channels": CHANNEL_KEYS,
        "materials": MATERIALS.iter().map(|preset| preset.name).collect::<Vec<_>>(),
        "smart_materials": SMART_MATERIALS.iter().map(|preset| preset.name).collect::<Vec<_>>(),
        "textures": tips::texture_names(),
        "local_presets": paint.library.entries().map(|(index, entry)| {
            let kind = match entry.kind {
                presets::Kind::Material { .. } => "material",
                presets::Kind::Mask(_) => "mask",
            };
            json!({"index": index, "name": entry.name, "kind": kind})
        }).collect::<Vec<_>>(),
        "generators": GeneratorKind::ALL.map(|kind| {
            let generator = Generator::new(kind);
            json!({"id": kind, "label": kind.label(), "controls": generator.controls().iter().enumerate()
                .map(|(index, &(label, min, max, _))| json!({"index": index, "label": label,
                    "min": min, "max": max, "default": generator.value(index)})).collect::<Vec<_>>()})
        }),
        "generator_inputs": GeneratorInput::ALL.map(|input| json!({"id": input as usize, "key": input, "label": input.label()})),
        "mask_blend_modes": MaskBlend::ALL.map(|mode| json!({"id": mode, "label": mode.label()})),
        "mask_filters": FilterKind::ALL.map(|kind| json!({"id": kind, "label": kind.label(),
            "controls": kind.controls().iter().enumerate().map(|(index, &(label, default, min, max))|
                json!({"index": index, "label": label, "min": min, "max": max, "default": default})
            ).collect::<Vec<_>>()
        })),
        "blend_modes": Blend::ALL.map(|blend| json!({"id": blend, "label": blend.label()})),
        "brush_tips": paint.tips.names(),
        "brush_grains": paint.tips.grain_names(),
        "brush_projections": [BrushProjection::Screen, BrushProjection::Surface, BrushProjection::Uv],
        "brush_symmetry": [Symmetry::Off, Symmetry::X, Symmetry::Y, Symmetry::Z],
        "fill_modes": FillMode::OPTIONS.map(|(label, mode)| json!({"id": mode, "label": label})),
        "texture_set_resolutions": crate::editor::model_paint_data::RESOLUTIONS,
        "fill_projections": texture_fill::Projection::ALL,
        "fill_opacity_sources": texture_fill::OpacitySource::ALL,
        "paint_tone_maps": crate::ToneMap::ALL,
        "bake_qualities": crate::editor::model_bake_settings::Quality::ALL,
        "bake_maps": bake::BakeMap::ALL.map(|map| json!({"id": map, "label": map.label(), "requires_high_poly": map.requires_high_poly()})),
        "export_presets": export_presets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        model_document::{MaterialSlot, Object, glb::Glb},
        model_mesh::Geometry,
    };

    fn fixture() -> (Document, Session) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/assets/models/block.glb");
        let source = Glb::load(&path).unwrap();
        let material = |name, tile, paint| MaterialSlot {
            material: Handle::default(),
            source: json!({"name": name, "extras": {"bevy_editor_udim": tile}}),
            authored: None,
            paint,
            bake_fixes: Default::default(),
        };
        let pixels = Arc::new(vec![0; 4]);
        let saved = Arc::new(Surface {
            size: 512,
            base: std::array::from_fn(|_| pixels.clone()),
            stack: Stack {
                layers: vec![Layer::new("Saved paint".into(), LayerKind::Paint)],
                selected: 0,
            },
            material: json!({}),
            baked: None,
        });
        let object = Object {
            id: 7,
            name: "Cube".into(),
            source_node: None,
            geometry: Geometry::default().into(),
            transform: Transform::IDENTITY,
            materials: vec![
                material("Stone", 1001, Some(saved)),
                material("Stone", 1013, None),
            ],
            hidden: false,
        };
        let mut document = Document::new(source, vec![object]);
        document.mode = Mode::Paint;
        let mut child = Layer::new("Live paint".into(), LayerKind::Paint);
        child.depth = 1;
        child.channels[0] = Some(pixels);
        let mut group = Layer::new("Group".into(), LayerKind::Folder);
        group.visible = false;
        let mut session = Session {
            set: 1,
            binding: Some((7, 1)),
            texture_size: 2048,
            stack: Stack {
                layers: vec![child, group],
                selected: 0,
            },
            size: 64.,
            projection: BrushProjection::Surface,
            ..default()
        };
        session.set_appearance.toggle_visibility((7, 0));
        session.set_appearance.toggle_pin((7, 1));
        (document, session)
    }

    #[test]
    fn snapshot_tracks_live_set_and_falls_back_to_saved_layers_until_rebound() {
        let (mut document, mut session) = fixture();
        let paint = PaintState::default();
        let live = snapshot(0, &document, Some(&session), &paint.tips);
        assert_eq!(live["active_set"], 1);
        assert_eq!(live["resolution"], 2048);
        assert_eq!(live["texture_sets"][0]["visible"], false);
        assert_eq!(live["texture_sets"][1]["udim"], 1013);
        assert_eq!(live["texture_sets"][1]["highlighted"], true);
        assert_eq!(live["layers_bottom_to_top"][0]["name"], "Live paint");
        assert_eq!(live["layers_bottom_to_top"][0]["folder"], 1);
        assert_eq!(live["layers_bottom_to_top"][0]["shown"], false);
        assert_eq!(
            live["layers_bottom_to_top"][0]["channels"],
            json!(["base_color"])
        );
        assert_eq!(live["layers_bottom_to_top"][1]["layers_inside"], 1);
        assert_eq!(live["brush"]["size_px"].as_f64(), Some(64.));
        assert_eq!(live["brush"]["projection"], "surface");

        session.set = 0;
        let loading = snapshot(0, &document, Some(&session), &paint.tips);
        assert_eq!(loading["resolution"], 512);
        assert_eq!(loading["layers_bottom_to_top"][0]["name"], "Saved paint");
        document.mode = Mode::Object;
        let object_view = snapshot(0, &document, Some(&session), &paint.tips);
        assert_eq!(object_view["mode"], "object");
        assert!(object_view["brush"].is_null());
        assert_eq!(
            object_view["layers_bottom_to_top"][0]["name"],
            "Saved paint"
        );
    }

    #[test]
    fn fill_settings_and_solid_values_share_layer_undo() {
        let (mut doc, mut session) = fixture();
        session.texture_size = 4;
        let layer = &mut session.stack.layers[0];
        layer.kind = LayerKind::Fill;
        let edit = |fill: Value| {
            serde_json::from_value(json!({
                "operation": "update_layer", "index": 0, "name": "Glazed",
                "opacity_percent": 72.0, "fill": fill,
            })).unwrap()
        };
        let initial = session.stack.clone();
        assert!(perform(&mut doc, &mut session, edit(json!({"tiling": 0.0})), 0., Vec::new()).is_err());
        assert_eq!(session.stack, initial);
        assert!(session.undo.is_empty());
        assert!(!doc.paint_pending);

        let patch = json!({
            "channels": {"base_color": [20, 40, 80], "roughness": 100},
            "projection": "Triplanar", "tiling": 3.0, "offset": [0.25, 0.0],
            "variation": {"height": 0.4, "scale": 8.0}
        });
        perform(&mut doc, &mut session, edit(patch.clone()), 0., Vec::new()).unwrap();
        let layer = &session.stack.layers[0];
        assert_eq!(layer.name, "Glazed");
        assert_eq!(layer.opacity, 72.);
        assert_eq!(&layer.channels[0].as_ref().unwrap()[..4], &[20, 40, 80, 255]);
        assert_eq!(&layer.channels[3].as_ref().unwrap()[..4], &[128, 128, 128, 255]);
        let settings = layer.texture_fill.as_ref().unwrap().settings;
        assert_eq!(settings.projection, texture_fill::Projection::Triplanar);
        assert_eq!(settings.variation.scale, 8.);
        assert!(doc.paint_pending);
        assert_eq!(session.undo.len(), 1);
        perform(&mut doc, &mut session, edit(patch), 0., Vec::new()).unwrap();
        assert_eq!(session.undo.len(), 1);
        perform(&mut doc, &mut session, Edit::Undo { steps: None }, 0., Vec::new()).unwrap();
        assert_eq!(session.stack, initial);

        let layer = &mut session.stack.layers[0];
        let bitmap = texture_fill::Bitmap::from_rgba(
            "Fixture".into(), image::RgbaImage::from_pixel(2, 2, image::Rgba([90, 20, 40, 255])),
        ).unwrap();
        layer.edit_texture_fill(|fill| fill.images[0] = Some(Arc::new(bitmap)));
        let textured = session.stack.clone();
        assert!(perform(&mut doc, &mut session, edit(json!({
            "channels": {"base_color": [0, 0, 0]}, "tiling": 2.0,
        })), 0., Vec::new()).is_err());
        assert_eq!(session.stack, textured);
        assert!(session.undo.is_empty());
    }

    #[test]
    fn texture_set_updates_validate_before_changing_the_session() {
        let (mut doc, mut session) = fixture();
        let edit = |doc: &mut Document, session: &mut Session, data| {
            perform(doc, session, serde_json::from_value(data).unwrap(), 0., Vec::new())
        };
        let active = (7, 1);
        let other = (7, 0);
        for data in [
            json!({"operation":"update_texture_set", "index":1, "visible":false, "highlighted":false, "resolution":512}),
            json!({"operation":"update_texture_set", "index":0, "visible":true, "resolution":512}),
            json!({"operation":"update_texture_set", "index":1, "resolution":123, "highlighted":false}),
        ] {
            assert!(edit(&mut doc, &mut session, data).is_err());
        }
        assert!(session.set_appearance.visibility.shown(active.0, active.1));
        assert!(!session.set_appearance.visibility.shown(other.0, other.1));
        assert!(session.set_appearance.pinned.contains(&active));
        assert_eq!(session.resize_request, None);
        assert!(!doc.paint_busy);

        edit(&mut doc, &mut session, json!({"operation":"update_texture_set", "index":0, "visible":true})).unwrap();
        edit(&mut doc, &mut session, json!({
            "operation":"update_texture_set", "index":1, "visible":false, "highlighted":false, "resolution":512
        })).unwrap();
        assert!(session.set_appearance.visibility.shown(other.0, other.1));
        assert!(!session.set_appearance.visibility.shown(active.0, active.1));
        assert!(!session.set_appearance.pinned.contains(&active));
        assert_eq!(session.resize_request, Some(512));
        assert_eq!(session.texture_size, 2048);
        assert!(doc.paint_busy && doc.paint_pending);
        assert!(session.undo.is_empty());
    }

    #[test]
    fn layer_creation_names_and_places_each_layer_in_one_history_step() {
        let (mut doc, mut session) = fixture();
        session.texture_size = 16;
        let edit = |doc: &mut Document, session: &mut Session, data| {
            perform(doc, session, serde_json::from_value(data).unwrap(), 0., Vec::new())
        };
        let initial = session.stack.clone();
        for data in [
            json!({"operation":"add_layer", "kind":"Paint", "name":" "}),
            json!({"operation":"add_layer", "kind":"Folder", "target":{"layer":0,"placement":"inside"}}),
            json!({"operation":"add_material", "preset":MATERIALS[0].name, "target":{"layer":99,"placement":"above"}}),
        ] {
            assert!(edit(&mut doc, &mut session, data).is_err());
        }
        assert_eq!(session.stack, initial);
        assert!(session.undo.is_empty());
        assert!(!doc.paint_pending);

        edit(&mut doc, &mut session, json!({
            "operation":"add_layer", "kind":"Folder", "name":" Details ",
            "target":{"layer":1,"placement":"above"}
        })).unwrap();
        assert_eq!(session.stack.layers[2].name, "Details");
        assert_eq!(session.stack.layers[2].depth, 0);
        assert_eq!(session.undo.len(), 1);

        edit(&mut doc, &mut session, json!({
            "operation":"add_material", "preset":MATERIALS[0].name, "name":"Metal",
            "target":{"layer":2,"placement":"inside"}
        })).unwrap();
        let metal = &session.stack.layers[2];
        assert_eq!(metal.name, "Metal");
        assert_eq!(metal.depth, 1);
        assert_eq!(metal.channels, MATERIALS[0].layer(session.texture_size).channels);
        assert_eq!(session.stack.selected, 2);
        assert_eq!(session.undo.len(), 2);

        edit(&mut doc, &mut session, json!({
            "operation":"add_layer", "kind":"Paint", "name":"Highlights",
            "target":{"layer":1,"placement":"below"}
        })).unwrap();
        assert_eq!(session.stack.layers[0].name, "Highlights");
        assert_eq!(session.stack.layers[0].depth, 0);
        assert_eq!(session.stack.layers[1].name, "Live paint");
        assert_eq!(session.undo.len(), 3);
        assert!(doc.paint_pending);
        let after = session.stack.clone();
        for _ in 0..3 {
            edit(&mut doc, &mut session, json!({"operation":"undo"})).unwrap();
        }
        assert_eq!(session.stack, initial);
        for _ in 0..3 {
            edit(&mut doc, &mut session, json!({"operation":"redo"})).unwrap();
        }
        assert_eq!(session.stack, after);
    }

    #[test]
    fn paint_edit_validates_workspace_and_keeps_atomic_shared_history() {
        let (document, mut session) = fixture();
        session.texture_size = 16;
        let project_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json");
        let project = crate::EditorProject::load(project_path).unwrap();
        let scene = project.read_scene(0).unwrap();
        let mut editor = Editor::new(project, scene);
        editor.viewing_model = Some(0);
        let mut editing = editing::State::default();
        editing.documents.insert(0, document);
        let mut paint = PaintState::default();
        paint.sessions.insert(0, session);
        let request = |operation| EditArgs {
            model_id: 0,
            set: 1,
            revision: None,
            edit: operation,
        };
        let edited = |name, opacity| Edit::UpdateLayer {
            index: 0,
            name: Some(name),
            visible: Some(false),
            opacity_percent: Some(opacity),
            blend: Some(Blend::Multiply),
            fill: None,
        };
        let invalid = request(edited("Ignored".into(), 120.));
        assert!(apply(&editor, &mut editing, &mut paint, invalid, 0.).is_err());
        assert!(paint.sessions[&0].undo.is_empty());
        assert_eq!(paint.sessions[&0].stack.layers[0].name, "Live paint");

        let patch = request(edited("Polished".into(), 55.));
        let updated = apply(&editor, &mut editing, &mut paint, patch, 0.).unwrap();
        let row = &updated["layers_bottom_to_top"][0];
        assert_eq!(row["name"], "Polished");
        assert_eq!(row["blend"], "Multiply");
        assert_eq!(row["visible"], false);
        assert_eq!(row["opacity_percent"].as_f64(), Some(55.));
        assert_eq!(updated["undo_steps"], 1);
        assert_eq!(updated["dirty"], true);
        let stale = EditArgs {
            revision: Some(updated["revision"].as_u64().unwrap() - 1),
            ..request(Edit::RemoveLayer { index: 0 })
        };
        assert!(apply(&editor, &mut editing, &mut paint, stale, 0.).is_err());
        assert_eq!(paint.sessions[&0].stack.layers.len(), 2);

        apply(&editor, &mut editing, &mut paint, request(Edit::Undo { steps: None }), 0.).unwrap();
        assert_eq!(paint.sessions[&0].stack.layers[0].name, "Live paint");
        apply(&editor, &mut editing, &mut paint, request(Edit::Redo { steps: None }), 0.).unwrap();
        assert_eq!(paint.sessions[&0].stack.layers[0].name, "Polished");
        for edit in [
            Edit::SetMask {
                index: 0,
                mask: MaskBase::Black,
            },
            Edit::ApplyGenerator {
                index: 0,
                generator: GeneratorKind::Clouds,
                parameters: GeneratorPatch::default(),
            },
            Edit::AddMaskFilter {
                index: 0,
                filter: FilterKind::Levels,
                parameters: FilterPatch::default(),
            },
            Edit::MoveLayer {
                from: 0,
                to: 1,
                placement: Placement::Above,
            },
        ] {
            apply(&editor, &mut editing, &mut paint, request(edit), 0.).unwrap();
        }
        let session = &paint.sessions[&0];
        assert!(session.stack.valid_hierarchy());
        assert_eq!(session.stack.layers[1].depth, 0);
        let mask = session.stack.layers[1].mask.as_ref().unwrap();
        assert_eq!(mask.generator.unwrap().kind, GeneratorKind::Clouds);
        assert_eq!(mask.filters.len(), 1);
        let changed = apply(
            &editor,
            &mut editing,
            &mut paint,
            request(Edit::SelectSet { index: 0 }),
            0.,
        )
        .unwrap();
        assert_eq!(changed["active_set"], 0);
        assert_eq!(editing.documents[&0].paint_set, Some(0));
        assert_eq!(changed["layers_bottom_to_top"][0]["name"], "Saved paint");
        assert!(apply(&editor, &mut editing, &mut paint, request(Edit::Undo { steps: None }), 0.).is_err());
    }
}
