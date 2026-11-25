//! Bake parameters stored with the editable model. Pixel maps use the bake cache.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const FIELD: &str = "bevy_editor_bake_settings";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub(in crate::editor) struct Settings {
    pub high_poly: Option<PathBuf>,
    pub cage: Option<PathBuf>,
    pub quality: Quality,
    pub map_bits: u8,
    pub id_source: IdSource,
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
            map_bits: 0b111_1111,
            id_source: IdSource::default(),
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
    let mut settings = root["extras"]
        .get(FIELD)
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
    settings.map_bits &= Settings::default().map_bits;
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
    let value = serde_json::to_value(stored).map_err(|error| error.to_string())?;
    extras.insert(FIELD.into(), value);
    Ok(())
}
