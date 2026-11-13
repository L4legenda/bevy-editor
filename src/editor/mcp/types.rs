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
    /// Exact project-relative GLB asset path returned by list_assets.
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

impl Command {
    pub fn mutates(&self) -> bool {
        !matches!(
            self,
            Self::State | Self::Scenes | Self::Objects(_) | Self::Assets(_) | Self::Capture
        )
    }
}
