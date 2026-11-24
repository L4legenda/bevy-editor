//! Presets and stroke previews prepared from the supplied Procreate library.
use super::*;
use photocraft_paint::{Control, Dynamic, GrayTile, Pattern};
use serde::Deserialize;
use std::{
    io::{Cursor, Read},
    sync::OnceLock,
};

#[cfg(test)]
#[path = "image_brush_stroke_tests.rs"]
mod tests;

const ARCHIVE: &[u8] = include_bytes!("procreate-catalog.zip");

#[derive(Deserialize)]
pub(super) struct Preset {
    pub name: String,
    category: String,
    sample: String,
    #[serde(skip)]
    decoded: OnceLock<Option<GrayTile>>,
    pub diameter: f32,
    pressure_size: f32,
    pressure_opacity: f32,
    canvas_grain: bool,
    cap_hardness: f32,
}

impl Preset {
    pub fn apply(&self, brush: &mut BrushSettings) {
        let sample = self
            .decoded
            .get_or_init(|| {
                let pixels = catalog_png(&self.sample)?.into_luma8();
                Some(GrayTile {
                    width: pixels.width(),
                    height: pixels.height(),
                    data: pixels
                        .into_raw()
                        .into_iter()
                        .map(|v| u16::from(v) * 257)
                        .collect(),
                })
            })
            .clone();
        if self.canvas_grain {
            if let Some(tile) = sample {
                brush.texture.enabled = true;
                brush.texture.pattern = Pattern::Tile(tile);
            }
            brush.hardness = 0.75;
        } else {
            brush.stroke_sample = sample;
            brush.hardness = self.cap_hardness;
        }
        // The measured strip includes the source brushes' spacing, scatter and blending.
        // Spacing here only controls how finely the continuous strip follows a curve.
        brush.spacing = 0.12;
        brush.pressure_size = false;
        brush.shape_dynamics.enabled = true;
        brush.shape_dynamics.angle.control = Control::Direction;
        brush.shape_dynamics.size = pressure_control(self.pressure_size);
        brush.transfer.enabled = true;
        brush.transfer.opacity = pressure_control(self.pressure_opacity);
    }
}

fn pressure_control(amount: f32) -> Dynamic {
    let control = if amount < 0.0 {
        Control::InversePenPressure
    } else {
        Control::PenPressure
    };
    Dynamic {
        minimum: 1.0 - amount.abs().min(1.0),
        ..Dynamic::controlled(control)
    }
}

#[derive(Default)]
pub(super) struct Library {
    pub categories: Vec<String>,
    pub groups: Vec<Vec<usize>>,
    pub presets: Vec<Preset>,
}

fn archive() -> zip::result::ZipResult<zip::ZipArchive<Cursor<&'static [u8]>>> {
    zip::ZipArchive::new(Cursor::new(ARCHIVE))
}

pub(super) fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let presets = archive()
            .ok()
            .and_then(|mut zip| serde_json::from_reader(zip.by_name("catalog.json").ok()?).ok())
            .unwrap_or_default();
        let mut library = Library {
            presets,
            ..default()
        };
        for (index, preset) in library.presets.iter().enumerate() {
            let category = if let Some(i) = library
                .categories
                .iter()
                .position(|name| *name == preset.category)
            {
                i
            } else {
                library.categories.push(preset.category.clone());
                library.groups.push(Vec::new());
                library.categories.len() - 1
            };
            library.groups[category].push(index);
        }
        library
    })
}

fn catalog_png(name: &str) -> Option<image::DynamicImage> {
    let mut zip = archive().ok()?;
    let mut png = Vec::new();
    zip.by_name(name).ok()?.read_to_end(&mut png).ok()?;
    image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()
}

pub(super) fn preview(index: usize) -> Option<Image> {
    let pixels = catalog_png(&format!("{index}.png"))?;
    Some(Image::from_dynamic(
        pixels,
        true,
        bevy::asset::RenderAssetUsages::default(),
    ))
}
