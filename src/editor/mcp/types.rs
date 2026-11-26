use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct Page {
    #[serde(default)]
    pub offset: usize,
    /// Maximum number of results, 1–250. Defaults to 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct ObjectArgs {
    /// Opaque scene-qualified ID from list_objects, not a Bevy entity index.
    pub object_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct SceneArgs {
    /// Scene ID returned by list_scenes (the configured document path).
    pub scene_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct TransformPatch {
    /// Absolute world position in metres; each component must be within ±1000.
    pub position: Option<[f32; 3]>,
    /// Pitch X, yaw Y, roll Z in degrees, matching the editor inspector.
    pub rotation_degrees: Option<[f32; 3]>,
    /// Absolute scale; each absolute component must be between 0.01 and 100.
    pub scale: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct TransformArgs {
    pub object_id: String,
    pub transform: TransformPatch,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct AddModelArgs {
    /// Exact project-relative model path returned by list_assets (GLB or FBX).
    pub asset: String,
    /// Defaults to the origin, no rotation and unit scale.
    pub transform: Option<TransformPatch>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct AddLightArgs {
    /// bulb, candle, softbox, neon, spot, sun, or moon. Defaults to bulb.
    pub preset: Option<String>,
    pub position: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct LightArgs {
    pub object_id: String,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    /// sRGB tint, each component between 0 and 1.
    pub color: Option<[f32; 3]>,
    /// Lumens for local lights, lux for a directional light.
    pub intensity: Option<f32>,
    pub range: Option<f32>,
    pub shadows: Option<bool>,
    /// Set false to remove the temperature tint, true to enable it.
    pub use_temperature: Option<bool>,
    pub temperature_kelvin: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct VisibilityArgs {
    pub object_id: String,
    pub visible: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct ViewArgs {
    pub position: [f32; 3],
    pub target: [f32; 3],
}

#[derive(Debug, Clone)]
pub(super) enum Command {
    State,
    PaintState,
    PaintOpen(PaintOpenArgs),
    PaintImport(PaintImportArgs),
    PaintReplaceMesh(super::mcp_state::MeshArgs),
    PaintPresets,
    PaintEdit(super::mcp_state::EditArgs),
    PaintStroke(super::mcp_state::StrokeArgs),
    PaintFill(super::mcp_state::FillArgs),
    PaintSelectSurface(super::mcp_state::SurfaceArgs),
    PaintSave(super::mcp_state::SaveArgs),
    PaintPresetSave(super::mcp_state::SavePresetArgs),
    PaintPresetImport(super::mcp_state::ImportPresetArgs),
    PaintPresetExport(super::mcp_state::ExportPresetArgs),
    PaintExport(super::mcp_state::ExportArgs),
    BakeStatus(super::mcp_state::BakeTarget),
    BakeStart(super::mcp_state::BakeArgs),
    BakeCancel(super::mcp_state::BakeTarget),
    BakeLevels(super::mcp_state::BakeLevelsArgs),
    PaintView(super::mcp_state::PaintViewArgs),
    CagePreview(super::mcp_state::CageArgs),
    PaintCapture(super::mcp_state::CaptureArgs),
    PaintRender(super::mcp_state::RenderArgs),
    Scenes,
    Objects(Page),
    Assets(Page),
    Open(SceneArgs),
    Select(ObjectArgs),
    Transform(TransformArgs),
    AddModel(AddModelArgs),
    AddLight(AddLightArgs),
    Light(LightArgs),
    Remove(ObjectArgs),
    Visibility(VisibilityArgs),
    View(ViewArgs),
    Save,
    Undo,
    Redo,
    Capture,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct PaintOpenArgs {
    /// Exact project-relative model path returned by list_assets.
    pub asset: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct PaintImportArgs {
    /// Absolute local path of a Material Painter .mpp project.
    pub source: std::path::PathBuf,
    /// Existing directory relative to the current project's assets root. Defaults to that root.
    pub folder: Option<std::path::PathBuf>,
}

impl Command {
    pub fn mutates(&self) -> bool {
        if let Self::CagePreview(args) = self {
            return args.mutates();
        }
        !matches!(
            self,
            Self::State | Self::PaintState | Self::PaintPresets | Self::Scenes | Self::Objects(_) | Self::Assets(_) | Self::Capture | Self::BakeStatus(_) | Self::PaintCapture(_) | Self::PaintRender(_)
        )
    }
}
