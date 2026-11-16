//! Shared layer presets. The native envelope stores pattern pixels without JSON encoding.
use super::*;
use photocraft_engine::{
    commands::pattern_cmds::PatternLibrary,
    presets::{
        Group, PresetState, gradients::GradientPreset, shapes::ShapePreset, styles::StylePreset,
    },
};
use serde::{Deserialize, Serialize};

const KEY: &str = "bevy-editor.layer-libraries.v1";
#[derive(Clone)]
pub(in crate::editor) struct LayerLibraries {
    pub presets: PresetState,
    pub patterns: PatternLibrary,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    styles: Vec<Group<StylePreset>>,
    gradients: Vec<Group<GradientPreset>>,
    gradient: GradientPreset,
    shapes: Vec<Group<ShapePreset>>,
    pattern_groups: Vec<Group<String>>,
    pattern: Option<String>,
    layer_defaults: std::collections::BTreeMap<String, serde_json::Value>,
    contours: Vec<photocraft_doc::Contour>,
}
impl Default for LayerLibraries {
    fn default() -> Self {
        let engine = photocraft_engine::Session::new();
        Self {
            presets: engine.presets,
            patterns: engine.patterns,
        }
    }
}
impl LayerLibraries {
    pub fn load(path: &Path) -> Result<Self, String> {
        let doc = photocraft_format::load_path_with(
            path,
            &photocraft_format::LoadOptions {
                max_total_bytes: 256 << 20,
                max_blob_bytes: 128 << 20,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let text = doc
            .metadata
            .text
            .iter()
            .find(|(key, _)| key == KEY)
            .ok_or("Нет данных библиотеки слоёв")?;
        let manifest: Manifest = serde_json::from_str(&text.1).map_err(|e| e.to_string())?;
        if manifest.version != 1 {
            return Err("Неизвестная версия библиотеки слоёв".into());
        }
        let mut result = Self::default();
        result.presets.styles = manifest.styles;
        result.presets.gradients = manifest.gradients;
        result.presets.gradient = manifest.gradient;
        result.presets.shapes = manifest.shapes;
        result.presets.pattern_groups = manifest.pattern_groups;
        result.presets.pattern = manifest.pattern;
        result.presets.layer_defaults = manifest.layer_defaults;
        result.presets.contours = manifest.contours;
        result.patterns.items = doc.patterns;
        result.presets.rev += 1;
        Ok(result)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let p = &self.presets;
        let manifest = Manifest {
            version: 1,
            styles: p.styles.clone(),
            gradients: p.gradients.clone(),
            gradient: p.gradient.clone(),
            shapes: p.shapes.clone(),
            pattern_groups: p.pattern_groups.clone(),
            pattern: p.pattern.clone(),
            layer_defaults: p.layer_defaults.clone(),
            contours: p.contours.clone(),
        };
        let mut doc = Document::new(
            "Библиотеки слоёв",
            Size::new(1, 1),
            ColorMode::Rgb,
            SampleType::U8,
        );
        doc.patterns = self.patterns.items.clone();
        doc.metadata.text.push((
            KEY.into(),
            serde_json::to_string(&manifest).map_err(|e| e.to_string())?,
        ));
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default())
            .map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        photocraft_format::atomic_write_no_follow(path, &bytes).map_err(|e| e.to_string())
    }
}
impl PhotoDocument {
    pub fn libraries(&self) -> LayerLibraries {
        if let Some(engine) = self.engine.as_ref().and_then(|e| e.lock().ok()) {
            LayerLibraries {
                presets: engine.presets.clone(),
                patterns: engine.patterns.clone(),
            }
        } else {
            LayerLibraries::default()
        }
    }
    pub fn set_libraries(&mut self, libraries: &LayerLibraries) {
        let engine = self.engine.get_or_insert_with(|| {
            std::sync::Mutex::new(Box::new(photocraft_engine::Session::new()))
        });
        if let Ok(engine) = engine.get_mut() {
            let p = &libraries.presets;
            engine.presets.styles = p.styles.clone();
            engine.presets.gradients = p.gradients.clone();
            engine.presets.gradient = p.gradient.clone();
            engine.presets.shapes = p.shapes.clone();
            engine.presets.pattern_groups = p.pattern_groups.clone();
            engine.presets.pattern = p.pattern.clone();
            engine.presets.layer_defaults = p.layer_defaults.clone();
            engine.presets.contours = p.contours.clone();
            engine.presets.rev = p.rev;
            engine.patterns = libraries.patterns.clone();
        }
    }
    pub fn effect_defaults(&self, kind: &str) -> serde_json::Value {
        let libraries = self.libraries();
        let mut defaults = photocraft_engine::layer_style::effect_defaults(kind);
        if let Some(values) = libraries
            .presets
            .layer_defaults
            .get(kind)
            .and_then(serde_json::Value::as_object)
        {
            for (key, value) in values {
                defaults[key] = value.clone();
            }
        }
        if kind == "patternOverlay"
            && defaults["pattern"].as_str().is_none_or(str::is_empty)
            && let Some(pattern) = libraries.patterns.items.first()
        {
            defaults["pattern"] = serde_json::json!(pattern.name);
        }
        defaults
    }
    pub fn library_options(&self, key: &str, command: &str) -> Option<Vec<serde_json::Value>> {
        if !matches!(key, "pattern" | "texture" | "preset" | "group") {
            return None;
        }
        let libraries = self.libraries();
        let p = libraries.presets;
        let names: Vec<String> = match key {
            "pattern" | "texture" => self
                .document
                .patterns
                .iter()
                .map(|p| p.name.clone())
                .chain(libraries.patterns.items.into_iter().map(|p| p.name))
                .collect(),
            "preset" if command.starts_with("style.") => p
                .styles
                .into_iter()
                .flat_map(|g| g.items.into_iter().map(|s| s.name))
                .collect(),
            "preset" if command.starts_with("gradient.") => p
                .gradients
                .into_iter()
                .flat_map(|g| g.items.into_iter().map(|s| s.name))
                .collect(),
            "preset" if command.starts_with("shape.") => p
                .shapes
                .into_iter()
                .flat_map(|g| g.items.into_iter().map(|s| s.name))
                .collect(),
            "group" if command.starts_with("style.") => {
                p.styles.into_iter().map(|g| g.name).collect()
            }
            "group" if command.starts_with("gradient.") => {
                p.gradients.into_iter().map(|g| g.name).collect()
            }
            "group" if command.starts_with("shape.") => {
                p.shapes.into_iter().map(|g| g.name).collect()
            }
            _ => return None,
        };
        (!names.is_empty()).then(|| names.into_iter().map(serde_json::Value::String).collect())
    }
}
