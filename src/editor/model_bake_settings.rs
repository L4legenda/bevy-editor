//! Bake parameters stored with the editable model. Pixel maps use the bake cache.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const FIELD: &str = "bevy_editor_bake_settings";
const MAP_SELECTION_VERSION: &str = "map_selection_version";
const LEGACY_MAP_BITS: u16 = (1 << 7) - 1;
pub(in crate::editor) const BENT_NORMAL_BIT: u16 = 1 << 9;
pub(in crate::editor) const ALL_MAP_BITS: u16 = (1 << 13) - 1;
pub(in crate::editor) const BASIC_MAP_BITS: u16 = ALL_MAP_BITS & !(LEGACY_MAP_BITS | BENT_NORMAL_BIT);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum Quality {
    #[default]
    Draft,
    Standard,
    High,
    Ultra,
}

impl Quality {
    pub(in crate::editor) const ALL: [Self; 4] =
        [Self::Draft, Self::Standard, Self::High, Self::Ultra];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::Draft => "Черновик",
            Self::Standard => "Стандарт",
            Self::High => "Высокое",
            Self::Ultra => "Максимум",
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            Self::Draft => Self::Standard,
            Self::Standard => Self::High,
            Self::High => Self::Ultra,
            Self::Ultra => Self::Draft,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum IdSource {
    #[default]
    MeshParts,
    Materials,
    VertexColors,
}

impl IdSource {
    pub(in crate::editor) const ALL: [Self; 3] = [Self::MeshParts, Self::Materials, Self::VertexColors];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::MeshParts => "Части сетки",
            Self::Materials => "Материалы",
            Self::VertexColors => "Цвета вершин",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(in crate::editor) struct Defaults {
    pub quality: Quality,
    pub match_names: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            quality: Quality::default(),
            match_names: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(in crate::editor) struct RayOptions {
    pub occlusion_samples: Option<u16>,
    pub thickness_samples: Option<u16>,
    pub sky_samples: Option<u16>,
    // Reach in hundredths of a percent of the model's diagonal.
    pub occlusion_reach: u32,
    pub cavity_reach: u32,
    pub thickness_reach: u32,
    pub sky_cone: u16, // tenths of a degree
}

impl Default for RayOptions {
    fn default() -> Self {
        Self {
            occlusion_samples: None,
            thickness_samples: None,
            sky_samples: None,
            occlusion_reach: 6000,
            cavity_reach: 400,
            thickness_reach: 5000,
            sky_cone: 400,
        }
    }
}

impl RayOptions {
    pub const MAX_REACH: u32 = 100_000;

    pub fn validated(mut self) -> Self {
        self.occlusion_samples = self.occlusion_samples.map(|count| count.clamp(4, 512));
        self.thickness_samples = self.thickness_samples.map(|count| count.min(256));
        self.sky_samples = self.sky_samples.map(|count| count.min(256));
        self.occlusion_reach = self.occlusion_reach.clamp(1, Self::MAX_REACH);
        self.cavity_reach = self.cavity_reach.clamp(1, Self::MAX_REACH);
        self.thickness_reach = self.thickness_reach.clamp(1, Self::MAX_REACH);
        self.sky_cone = self.sky_cone.clamp(10, 890);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(in crate::editor) struct SurfaceOptions {
    pub grid: Option<u8>,
    pub radius: u16, // tenths of a texel
    // Distances and the curvature multiplier in hundredths of a percent of the diagonal.
    pub contrast: u32,
    pub edge_reach: u32,
    pub auto_edge_reach: bool,
    pub sharpness: u16, // thousandths
    pub hard_angle: u16, // tenths of a degree
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        Self {
            grid: None,
            radius: 20,
            contrast: 2500,
            edge_reach: 2000,
            auto_edge_reach: false,
            sharpness: 350,
            hard_angle: 350,
        }
    }
}

impl SurfaceOptions {
    pub fn validated(mut self) -> Self {
        self.grid = self.grid.map(|grid| grid.clamp(1, 8));
        self.radius = self.radius.clamp(10, 160);
        self.contrast = self.contrast.clamp(1, RayOptions::MAX_REACH);
        self.edge_reach = self.edge_reach.clamp(1, RayOptions::MAX_REACH);
        self.sharpness = self.sharpness.clamp(50, 950);
        self.hard_angle = self.hard_angle.clamp(50, 1200);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(in crate::editor) struct Settings {
    pub high_poly: Option<PathBuf>,
    pub cage: Option<PathBuf>,
    pub quality: Quality,
    pub map_bits: u16,
    pub id_source: IdSource,
    pub rays: RayOptions,
    pub surface: SurfaceOptions,
    pub front: u8,
    pub rear: u8,
    pub match_names: bool,
    pub fill_misses: bool,
    pub prefer_front_faces: bool,
    pub average_normals: bool,
    pub auto_fit: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let defaults = Defaults::default();
        Self {
            high_poly: None,
            cage: None,
            quality: defaults.quality,
            map_bits: ALL_MAP_BITS,
            id_source: IdSource::default(),
            rays: RayOptions::default(),
            surface: SurfaceOptions::default(),
            front: 5,
            rear: 5,
            match_names: defaults.match_names,
            fill_misses: true,
            prefer_front_faces: true,
            average_normals: true,
            auto_fit: false,
        }
    }
}

pub(super) fn accepts_defaults(root: &Value) -> bool {
    let extras = &root["extras"];
    (extras.is_null() || extras.is_object()) && extras.get(FIELD).is_none()
}

pub(super) fn read(root: &Value, model: &Path) -> Settings {
    let stored = root["extras"].get(FIELD);
    let mut settings = stored
        .and_then(|value| serde_json::from_value::<Settings>(value.clone()).ok())
        .unwrap_or_default();
    let parent = model.parent().unwrap_or_else(|| Path::new(""));
    for path in [&mut settings.high_poly, &mut settings.cage]
        .into_iter()
        .flatten()
    {
        if path.is_relative() {
            *path = parent.join(path.as_path());
        }
    }
    settings.front = settings.front.clamp(1, 100);
    settings.rear = settings.rear.clamp(1, 100);
    if stored.is_some_and(|value| value.get("map_bits").is_some()
        && value.get(MAP_SELECTION_VERSION).is_none())
    {
        // Older files always baked these maps and coupled the bent normal to AO.
        settings.map_bits &= LEGACY_MAP_BITS;
        settings.map_bits |= BASIC_MAP_BITS;
        if settings.map_bits & 1 != 0 {
            settings.map_bits |= BENT_NORMAL_BIT;
        }
    }
    settings.map_bits &= ALL_MAP_BITS;
    settings.rays = settings.rays.validated();
    settings.surface = settings.surface.validated();
    settings
}

pub(super) fn write(root: &mut Value, model: &Path, settings: &Settings) -> Result<(), String> {
    if *settings == Settings::default() && !root["extras"].is_object() && !root["extras"].is_null()
    {
        // A glTF may use non-object extras. Preserve that payload when there are no custom settings.
        return Ok(());
    }
    let mut stored = settings.clone();
    if let Some(parent) = model.parent() {
        for path in [&mut stored.high_poly, &mut stored.cage]
            .into_iter()
            .flatten()
        {
            if let Ok(relative) = path.strip_prefix(parent) {
                *path = relative.to_owned();
            }
        }
    }
    if root["extras"].is_null() {
        root["extras"] = serde_json::json!({});
    }
    let extras = root["extras"]
        .as_object_mut()
        .ok_or("Некорректные дополнительные данные GLB")?;
    let mut value = serde_json::to_value(stored).map_err(|error| error.to_string())?;
    value[MAP_SELECTION_VERSION] = Value::from(1);
    extras.insert(FIELD.into(), value);
    Ok(())
}
