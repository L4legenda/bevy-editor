//! Built-in material values from Material Painter's shelf.
use super::{Layer, LayerKind, texture_fill};
use crate::editor::model_paint_data::solid;
use std::sync::Arc;

pub(super) struct MaterialPreset {
    pub name: &'static str,
    pub color: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub(super) variation: texture_fill::Variation,
}

const fn mat(name: &'static str, color: [f32; 3], roughness: f32, metallic: f32) -> MaterialPreset {
    rich(name, color, roughness, metallic, 0., 0., 0., 4.)
}

const fn rich(
    name: &'static str,
    color: [f32; 3],
    roughness: f32,
    metallic: f32,
    color_variation: f32,
    roughness_variation: f32,
    height_variation: f32,
    scale: f32,
) -> MaterialPreset {
    MaterialPreset {
        name,
        color,
        roughness,
        metallic,
        variation: texture_fill::Variation {
            color: color_variation,
            roughness: roughness_variation,
            height: height_variation,
            scale,
            seed: 0.,
            parts: 0.,
        },
    }
}

impl MaterialPreset {
    pub fn kind(&self) -> MaterialType {
        MaterialType::of(self.name, Some(self.metallic))
    }

    pub fn rgb(&self) -> [u8; 3] {
        self.color.map(|channel| (channel * 255.).round() as u8)
    }

    pub fn layer(&self, size: u32) -> Layer {
        let [r, g, b] = self.rgb();
        let mut layer = Layer::new(self.name.to_owned(), LayerKind::Fill);
        layer.channels[0] = Some(solid(size, [r, g, b, 255]));
        for (channel, value) in [(1, self.roughness), (2, self.metallic)] {
            let value = (value * 255.).round() as u8;
            layer.channels[channel] = Some(solid(size, [value, value, value, 255]));
        }
        if self.variation.height > 0. {
            layer.channels[3] = Some(solid(size, [128, 128, 128, 255]));
        }
        if self.variation != texture_fill::Variation::default() {
            let mut fill = texture_fill::Fill::default();
            fill.settings.variation = self.variation;
            layer.texture_fill = Some(Arc::new(fill));
        }
        layer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::editor) enum MaterialType {
    Paint,
    Metal,
    Wood,
    Stone,
    Fabric,
    Plastic,
    Glass,
    Organic,
    Nature,
    Other,
}

impl MaterialType {
    pub const ALL: [MaterialType; 10] = [
        MaterialType::Paint,
        MaterialType::Metal,
        MaterialType::Wood,
        MaterialType::Stone,
        MaterialType::Fabric,
        MaterialType::Plastic,
        MaterialType::Glass,
        MaterialType::Organic,
        MaterialType::Nature,
        MaterialType::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MaterialType::Paint => "Краски и покрытия",
            MaterialType::Metal => "Металл",
            MaterialType::Wood => "Дерево",
            MaterialType::Stone => "Камень и бетон",
            MaterialType::Fabric => "Ткань, кожа и бумага",
            MaterialType::Plastic => "Пластик, резина и воск",
            MaterialType::Glass => "Стекло и керамика",
            MaterialType::Organic => "Кожа и органика",
            MaterialType::Nature => "Природа и грунт",
            MaterialType::Other => "Прочие",
        }
    }

    /// From a material's name (painted surfaces count as paint: "Painted Steel" is paint
    /// first); `metal` (its metallic value) decides for names that say nothing.
    pub fn of(name: &str, metal: Option<f32>) -> MaterialType {
        const WORDS: [(MaterialType, &[&str]); 9] = [
            (
                MaterialType::Organic,
                &[
                    "skin", "flesh", "eye", "scale", "fungus", "mushroom", "chitin", "insect",
                    "organic", "bone",
                ],
            ),
            (
                MaterialType::Paint,
                &["paint", "lacquer", "enamel", "coating", "powder coat"],
            ),
            (
                MaterialType::Metal,
                &[
                    "metal", "steel", "iron", "rust ", "rusty", "rusted", "copper", "brass",
                    "bronze", "gold", "silver", "alumin", "chrome", "tin ", "zinc", "nickel",
                    "titanium", "galvan", "pewter",
                ],
            ),
            (
                MaterialType::Wood,
                &[
                    "wood", "oak", "pine", "walnut", "plank", "bark", "birch", "maple", "bamboo",
                    "plywood", "mahogany", "ebony", "teak", "cedar", "cherry",
                ],
            ),
            (
                MaterialType::Stone,
                &[
                    "stone", "concrete", "brick", "marble", "granite", "rock", "cobble", "tile",
                    "slate", "asphalt", "plaster", "cement", "terrazzo", "pavement", "gravel",
                    "clay", "obsidian", "jade",
                ],
            ),
            (
                MaterialType::Fabric,
                &[
                    "fabric",
                    "cloth",
                    "leather",
                    "denim",
                    "cotton",
                    "wool",
                    "silk",
                    "carpet",
                    "canvas",
                    "felt",
                    "velvet",
                    "linen",
                    "suede",
                    "paper",
                    "cardboard",
                ],
            ),
            (
                MaterialType::Plastic,
                &[
                    "plastic",
                    "rubber",
                    "vinyl",
                    "silicone",
                    "acrylic",
                    "polymer",
                    "foam",
                    "carbon fiber",
                    "wax",
                ],
            ),
            (
                MaterialType::Glass,
                &["glass", "ceramic", "porcelain", "crystal", "terracotta"],
            ),
            (
                MaterialType::Nature,
                &[
                    "ground", "soil", "dirt", "mud", "sand", "grass", "moss", "snow", "ice",
                    "leaf", "leaves", "forest", "rock",
                ],
            ),
        ];
        let n = format!("{} ", name.to_lowercase());
        for (kind, words) in WORDS {
            if words.iter().any(|w| n.contains(w)) {
                return kind;
            }
        }
        match metal {
            Some(m) if m > 0.5 => MaterialType::Metal,
            _ => MaterialType::Other,
        }
    }
}

pub(super) const MATERIALS: &[MaterialPreset] = &[
    mat("Gold", [1.0, 0.77, 0.34], 0.22, 1.0),
    mat("Silver", [0.97, 0.96, 0.92], 0.18, 1.0),
    mat("Copper", [0.96, 0.64, 0.54], 0.28, 1.0),
    rich(
        "Iron Raw",
        [0.56, 0.57, 0.58],
        0.45,
        1.0,
        0.08,
        0.15,
        0.0,
        6.0,
    ),
    mat("Aluminium", [0.91, 0.92, 0.92], 0.3, 1.0),
    mat("Chrome", [0.55, 0.56, 0.55], 0.05, 1.0),
    rich(
        "Steel Brushed",
        [0.66, 0.66, 0.67],
        0.32,
        1.0,
        0.04,
        0.12,
        0.0,
        14.0,
    ),
    mat("Plastic Red", [0.72, 0.07, 0.05], 0.38, 0.0),
    mat("Plastic White", [0.86, 0.86, 0.84], 0.35, 0.0),
    mat("Plastic Black", [0.04, 0.04, 0.04], 0.45, 0.0),
    mat("Car Paint Blue", [0.05, 0.18, 0.55], 0.12, 0.0),
    mat("Rubber", [0.05, 0.05, 0.05], 0.88, 0.0),
    mat("Ceramic Teal", [0.18, 0.55, 0.55], 0.15, 0.0),
    rich(
        "Leather Brown",
        [0.36, 0.2, 0.11],
        0.6,
        0.0,
        0.18,
        0.15,
        0.25,
        22.0,
    ),
    rich(
        "Concrete",
        [0.56, 0.55, 0.52],
        0.9,
        0.0,
        0.2,
        0.1,
        0.35,
        9.0,
    ),
    rich("Rust", [0.44, 0.24, 0.13], 0.9, 0.0, 0.25, 0.15, 0.3, 7.0),
    rich(
        "Fabric Olive",
        [0.32, 0.34, 0.2],
        0.95,
        0.0,
        0.1,
        0.0,
        0.15,
        40.0,
    ),
    rich(
        "Stone Dark",
        [0.25, 0.24, 0.23],
        0.82,
        0.0,
        0.3,
        0.2,
        0.5,
        5.0,
    ),
    mat("Paint Red", [0.62, 0.08, 0.05], 0.45, 0.0),
    rich(
        "Paint Olive",
        [0.27, 0.29, 0.17],
        0.6,
        0.0,
        0.08,
        0.1,
        0.0,
        6.0,
    ),
    rich("Wax", [0.93, 0.88, 0.76], 0.35, 0.0, 0.03, 0.05, 0.05, 5.0),
    rich("Oak", [0.34, 0.21, 0.12], 0.72, 0.0, 0.18, 0.15, 0.0, 3.0),
    rich("Brass", [0.62, 0.49, 0.28], 0.45, 1.0, 0.18, 0.4, 0.0, 5.0),
    rich(
        "Iron Dark",
        [0.27, 0.27, 0.29],
        0.58,
        1.0,
        0.15,
        0.3,
        0.0,
        8.0,
    ),
    rich("Bone", [0.82, 0.77, 0.64], 0.55, 0.0, 0.06, 0.1, 0.0, 10.0),
    rich("Moss", [0.24, 0.32, 0.11], 0.95, 0.0, 0.25, 0.05, 0.3, 25.0),
    // Metals: F0 albedo (sRGB) from measured reflectance.
    mat("Titanium", [0.76, 0.73, 0.69], 0.3, 1.0),
    mat("Nickel", [0.83, 0.80, 0.74], 0.25, 1.0),
    rich("Bronze", [0.70, 0.50, 0.30], 0.35, 1.0, 0.1, 0.2, 0.0, 6.0),
    rich(
        "Zinc Galvanized",
        [0.78, 0.80, 0.82],
        0.42,
        1.0,
        0.1,
        0.3,
        0.0,
        12.0,
    ),
    rich(
        "Gunmetal",
        [0.33, 0.34, 0.36],
        0.3,
        1.0,
        0.05,
        0.1,
        0.0,
        8.0,
    ),
    mat("Rose Gold", [0.95, 0.70, 0.60], 0.22, 1.0),
    // Dielectrics.
    mat("Car Paint Red", [0.55, 0.03, 0.03], 0.1, 0.0),
    mat("Porcelain", [0.90, 0.89, 0.86], 0.08, 0.0),
    rich(
        "Carbon Fiber",
        [0.06, 0.06, 0.065],
        0.22,
        0.0,
        0.3,
        0.1,
        0.05,
        60.0,
    ),
    mat("Obsidian", [0.03, 0.03, 0.035], 0.06, 0.0),
    rich(
        "Marble White",
        [0.88, 0.87, 0.84],
        0.12,
        0.0,
        0.08,
        0.05,
        0.0,
        3.0,
    ),
    rich(
        "Granite",
        [0.45, 0.43, 0.42],
        0.55,
        0.0,
        0.45,
        0.2,
        0.1,
        60.0,
    ),
    rich(
        "Sandstone",
        [0.76, 0.62, 0.45],
        0.9,
        0.0,
        0.15,
        0.05,
        0.3,
        14.0,
    ),
    rich(
        "Brick Red",
        [0.55, 0.24, 0.16],
        0.88,
        0.0,
        0.2,
        0.08,
        0.3,
        10.0,
    ),
    rich(
        "Terracotta",
        [0.70, 0.38, 0.24],
        0.8,
        0.0,
        0.1,
        0.08,
        0.15,
        8.0,
    ),
    rich(
        "Asphalt",
        [0.12, 0.12, 0.12],
        0.92,
        0.0,
        0.35,
        0.05,
        0.35,
        45.0,
    ),
    rich("Jade", [0.30, 0.58, 0.40], 0.2, 0.0, 0.2, 0.05, 0.0, 4.0),
    rich("Pine", [0.70, 0.52, 0.32], 0.7, 0.0, 0.12, 0.12, 0.0, 3.0),
    rich("Walnut", [0.24, 0.14, 0.08], 0.6, 0.0, 0.2, 0.15, 0.0, 3.0),
    rich("Ebony", [0.07, 0.05, 0.04], 0.35, 0.0, 0.15, 0.1, 0.0, 6.0),
    rich("Bark", [0.25, 0.19, 0.14], 0.92, 0.0, 0.25, 0.05, 0.5, 12.0),
    rich("Skin", [0.80, 0.58, 0.47], 0.5, 0.0, 0.06, 0.1, 0.05, 20.0),
    rich("Denim", [0.16, 0.24, 0.40], 0.9, 0.0, 0.15, 0.0, 0.1, 50.0),
    rich(
        "Velvet Red",
        [0.40, 0.04, 0.07],
        0.95,
        0.0,
        0.1,
        0.0,
        0.05,
        30.0,
    ),
    rich(
        "Paper",
        [0.88, 0.86, 0.80],
        0.85,
        0.0,
        0.03,
        0.05,
        0.03,
        40.0,
    ),
    rich(
        "Cardboard",
        [0.60, 0.46, 0.30],
        0.88,
        0.0,
        0.08,
        0.05,
        0.08,
        30.0,
    ),
    rich("Snow", [0.92, 0.93, 0.95], 0.6, 0.0, 0.03, 0.15, 0.2, 10.0),
    mat("Ice", [0.70, 0.80, 0.86], 0.08, 0.0),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_paint_data::Stack;

    #[test]
    fn paint_material_catalog_applies_to_brush_or_adds_an_undoable_fill() {
        use super::super::{PaintAction, PaintTool, Parameter, Session};

        let index = MATERIALS
            .iter()
            .position(|preset| preset.name == "Concrete")
            .unwrap();
        let preset = &MATERIALS[index];
        let mut session = Session {
            binding: Some((0, 0)),
            texture_size: 4,
            tool: PaintTool::Eraser,
            stack: Stack {
                layers: vec![Layer::new("Folder".into(), LayerKind::Folder)],
                selected: 0,
            },
            ..Session::default()
        };
        let before = session.stack.clone();
        session.edit(PaintAction::Preset(index));
        assert_eq!(session.stack, before);
        assert_eq!(session.tool, PaintTool::Brush);
        assert_eq!(session.brush_color(), preset.rgb());

        session.edit(PaintAction::AddMaterial(index));
        assert!(session.stack.valid_hierarchy());
        let layer = &session.stack.layers[session.stack.selected];
        assert_eq!(layer.depth, 1);
        assert_eq!(layer.name, "Concrete");
        assert_eq!(
            &layer.channels[1].as_ref().unwrap()[..4],
            &[230, 230, 230, 255]
        );
        assert_eq!(
            &layer.channels[3].as_ref().unwrap()[..4],
            &[128, 128, 128, 255]
        );
        let variation = layer.texture_fill.as_ref().unwrap().settings.variation;
        assert_eq!(
            (
                variation.color,
                variation.roughness,
                variation.height,
                variation.scale
            ),
            (0.2, 0.1, 0.35, 9.)
        );
        let added = session.stack.clone();
        session.edit(PaintAction::Preset(0));
        session.set_value(Parameter::Roughness, 65.);
        let edited = &session.stack.layers[session.stack.selected];
        let original = &added.layers[added.selected];
        assert_ne!(edited.channels[1], original.channels[1]);
        for channel in [0, 2, 3] {
            assert_eq!(edited.channels[channel], original.channels[channel]);
        }
        session.edit(PaintAction::Undo);
        assert_eq!(session.stack, added);
        session.edit(PaintAction::Undo);
        assert_eq!(session.stack, before);
    }
}
