//! Geometry inputs and evaluation for procedural mask generators.
use crate::editor::{model_document::MeshData, model_mesh::Face, model_paint_data::custom};
use bevy::prelude::{Transform, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, OnceLock},
};

use super::spatial::{fbm, noise, smooth};

#[path = "model_paint_mask_patterns.rs"]
mod patterns;
#[path = "model_paint_mask_gradient.rs"]
mod gradient;
#[path = "model_paint_mask_tiles.rs"]
mod tiles;
pub(in crate::editor) use gradient::{GradientMapping, GradientProjection, GradientVolume};

#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "mcp", derive(Deserialize, Serialize, rmcp::schemars::JsonSchema), serde(rename_all = "snake_case"), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum GeneratorInput {
    Edges,
    Convexity,
    Concavity,
    Occlusion,
    Cavity,
    Sky,
    Up,
    Thin,
    PartGradient,
    Height,
    PartRandom,
    Creases,
    Streaks,
    Noise,
    Id,
    Overhang,
    Wood,
    Cells,
    Scratches,
    Front,
}

pub(in crate::editor) const GENERATOR_INPUT_COUNT: usize = 20;

impl GeneratorInput {
    pub const ALL: [Self; GENERATOR_INPUT_COUNT] = [
        Self::Edges,
        Self::Convexity,
        Self::Concavity,
        Self::Occlusion,
        Self::Cavity,
        Self::Sky,
        Self::Up,
        Self::Thin,
        Self::PartGradient,
        Self::Height,
        Self::PartRandom,
        Self::Creases,
        Self::Streaks,
        Self::Noise,
        Self::Id,
        Self::Overhang,
        Self::Wood,
        Self::Cells,
        Self::Scratches,
        Self::Front,
    ];

    pub fn label(self) -> &'static str {
        use GeneratorInput::*;
        match self {
            Edges => "Выпуклые рёбра",
            Convexity => "Выпуклая кривизна",
            Concavity => "Вогнутая кривизна",
            Occlusion => "Окклюзия",
            Cavity => "Впадины",
            Sky => "Открытость небу",
            Up => "Нормали вверх",
            Thin => "Тонкие участки",
            PartGradient => "Высота детали",
            Height => "Высота модели",
            PartRandom => "Случайно по деталям",
            Creases => "Складки",
            Streaks => "Вертикальные потёки",
            Noise => "Фрактальный шум",
            Id => "Цвет ID",
            Overhang => "Нормали вниз",
            Wood => "Волокна дерева",
            Cells => "Ячейки",
            Scratches => "Царапины",
            Front => "Угол от фронта",
        }
    }

    fn needs_bake(self) -> bool {
        !matches!(
            self,
            Self::Height
                | Self::Streaks
                | Self::Noise
                | Self::Wood
                | Self::Cells
                | Self::Scratches
                | Self::Front
        )
    }

    fn needs_mesh_normal(self, baked: bool) -> bool {
        matches!(self, Self::Wood | Self::Scratches)
            || (!baked && matches!(self, Self::Up | Self::Overhang))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_generator_mix_responds_to_width_and_procedural_inputs() {
        let data = BakedInputs {
            size: 1,
            edge_reach: 0.2,
            disabled_inputs: 0,
            texels: vec![[255, 255, 255, 255, 0, 0, 255, 128, 40, 255]],
            part_ids: None,
            custom: Vec::new(),
        };
        let mut sample = SurfaceSample {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            height: 0.5,
            extent: Vec3::new(2., 1., 1.),
            center: Vec3::ZERO,
            texel: 0.002,
            baked: Some(BakedSample {
                data: &data,
                index: 0,
            }),
            custom: &[],
            pixel: 0,
            size: 1,
        };
        let mut generator = Generator::new(GeneratorKind::EdgeWear);
        generator.edge_width = 20;
        assert_eq!(
            generator.base(&sample, &[(GeneratorInput::Edges, 1.)]),
            Some(0.)
        );
        generator.edge_width = 80;
        assert!(
            generator
                .base(&sample, &[(GeneratorInput::Edges, 1.)])
                .unwrap()
                > 0.5
        );
        sample.baked = None;
        for input in [
            GeneratorInput::Wood,
            GeneratorInput::Cells,
            GeneratorInput::Scratches,
            GeneratorInput::Front,
        ] {
            let values: Vec<_> = (0..64)
                .map(|i| {
                    sample.point = Vec3::new(i as f32 * 0.037, 0.1, 0.25);
                    generator.base(&sample, &[(input, 1.)]).unwrap()
                })
                .collect();
            assert!(values.iter().all(|v| v.is_finite()), "{input:?}");
            assert!(values.windows(2).any(|v| v[0] != v[1]), "{input:?}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum GeneratorKind {
    Clouds,
    Height,
    EdgeWear,
    Dirt,
    Dust,
    RainStreaks,
    Drips,
    CavityGrime,
    CreaseGrime,
    ThinAreas,
    PartGradient,
    PartRandom,
    IdMask,
    CustomMap,
    GradientLinear,
    GradientMirror,
    GradientPeak,
    ShapeDisc,
    ShapeCone,
    ShapeSquareBorder,
    ShapeCircleBorder,
    TilePattern,
}

impl GeneratorKind {
    pub const ALL: [Self; 22] = [
        Self::Clouds,
        Self::Height,
        Self::EdgeWear,
        Self::Dirt,
        Self::Dust,
        Self::RainStreaks,
        Self::Drips,
        Self::CavityGrime,
        Self::CreaseGrime,
        Self::ThinAreas,
        Self::PartGradient,
        Self::PartRandom,
        Self::IdMask,
        Self::CustomMap,
        Self::GradientLinear,
        Self::GradientMirror,
        Self::GradientPeak,
        Self::ShapeDisc,
        Self::ShapeCone,
        Self::ShapeSquareBorder,
        Self::ShapeCircleBorder,
        Self::TilePattern,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Clouds => "Облака",
            Self::Height => "Градиент высоты",
            Self::EdgeWear => "Износ рёбер",
            Self::Dirt => "Грязь",
            Self::Dust => "Пыль",
            Self::RainStreaks => "Следы дождя",
            Self::Drips => "Подтёки",
            Self::CavityGrime => "Грязь во впадинах",
            Self::CreaseGrime => "Грязь в складках",
            Self::ThinAreas => "Тонкие участки",
            Self::PartGradient => "Градиент по деталям",
            Self::PartRandom => "Случайно по деталям",
            Self::IdMask => "Маска по ID",
            Self::CustomMap => "По запечённой карте",
            Self::GradientLinear => "Линейный градиент",
            Self::GradientMirror => "Закруглённый градиент",
            Self::GradientPeak => "Треугольный градиент",
            Self::ShapeDisc => "Диск",
            Self::ShapeCone => "Конус",
            Self::ShapeSquareBorder => "Квадратная рамка",
            Self::ShapeCircleBorder => "Круглая рамка",
            Self::TilePattern => "Плитки",
        }
    }

    fn weights(self) -> [i16; GENERATOR_INPUT_COUNT] {
        use GeneratorInput as I;
        let inputs: &[(I, i16)] = match self {
            Self::Clouds => &[(I::Noise, 100)],
            Self::Height => &[(I::Height, 100)],
            Self::EdgeWear => &[(I::Edges, 100), (I::Convexity, 50)],
            Self::Dirt => &[(I::Occlusion, 100), (I::Cavity, 80), (I::Concavity, 40)],
            Self::Dust => &[(I::Sky, 80), (I::Up, 70)],
            Self::RainStreaks => &[(I::Sky, 40), (I::Streaks, 100), (I::Overhang, -40)],
            Self::Drips => &[
                (I::Creases, 60),
                (I::Streaks, 70),
                (I::Cavity, 40),
                (I::Overhang, 30),
            ],
            Self::CavityGrime => &[(I::Cavity, 150)],
            Self::CreaseGrime => &[(I::Creases, 100), (I::Concavity, 40)],
            Self::ThinAreas => &[(I::Thin, 100)],
            Self::PartGradient => &[(I::PartGradient, 100)],
            Self::PartRandom => &[(I::PartRandom, 100)],
            Self::IdMask => &[(I::Id, 100)],
            Self::CustomMap | Self::GradientLinear | Self::GradientMirror | Self::GradientPeak
            | Self::ShapeDisc | Self::ShapeCone | Self::ShapeSquareBorder | Self::ShapeCircleBorder
            | Self::TilePattern => &[],
        };
        let mut weights = [0; GENERATOR_INPUT_COUNT];
        for &(input, weight) in inputs {
            weights[input as usize] = weight;
        }
        weights
    }

    pub fn is_gradient(self) -> bool {
        matches!(self, Self::GradientLinear | Self::GradientMirror | Self::GradientPeak)
    }

    pub fn is_shape(self) -> bool {
        matches!(self, Self::ShapeDisc | Self::ShapeCone | Self::ShapeSquareBorder | Self::ShapeCircleBorder)
    }

    pub fn uses_pattern_mapping(self) -> bool {
        self.is_gradient() || self.is_shape() || self == Self::TilePattern
    }
}

pub(in crate::editor) const INPUT_CHANNELS: usize = 10;

#[derive(Debug, PartialEq)]
pub(in crate::editor) struct BakedInputs {
    pub size: u32,
    pub edge_reach: f32,
    pub disabled_inputs: u32,
    // AO, cavity, thickness, sky, part gradient, part random, normal Y,
    // curvature, edge distance and crease distance.
    pub texels: Vec<[u8; INPUT_CHANNELS]>,
    pub part_ids: Option<PartIds>,
    pub custom: Vec<custom::Plane>,
}

#[derive(Debug, PartialEq)]
pub(in crate::editor) struct PartIds {
    pub colors: Vec<[u8; 3]>,
    palette: Vec<[u8; 3]>,
}

impl PartIds {
    pub fn new(colors: Vec<[u8; 3]>) -> Self {
        let mut palette: Vec<_> = colors
            .iter()
            .copied()
            .filter(|color| *color != [0; 3])
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        palette.sort_unstable();
        Self { colors, palette }
    }
}

impl BakedInputs {
    fn index(&self, pixel: usize, size: u32) -> usize {
        sample_index(pixel, size, self.size)
    }
}

fn sample_index(pixel: usize, output_size: u32, input_size: u32) -> usize {
    let i = pixel as u32;
    let x = ((i % output_size) * 2 + 1) * input_size / (output_size * 2);
    let y = ((i / output_size) * 2 + 1) * input_size / (output_size * 2);
    (y * input_size + x) as usize
}

struct BakedSample<'a> {
    data: &'a BakedInputs,
    index: usize,
}

struct SurfaceSample<'a> {
    point: Vec3,
    normal: Vec3,
    height: f32,
    extent: Vec3,
    center: Vec3,
    texel: f32,
    baked: Option<BakedSample<'a>>,
    custom: &'a [(&'a [u8], u32, f32)],
    pixel: usize,
    size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct Generator {
    pub kind: GeneratorKind,
    pub values: [f32; 5],
    pub invert: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_color: Option<[u8; 3]>,
    #[serde(default = "default_id_tolerance")]
    pub id_tolerance: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<[i16; GENERATOR_INPUT_COUNT]>,
    #[serde(default = "default_edge_width")]
    pub edge_width: u16,
    #[serde(default, skip_serializing_if = "empty_custom_inputs")]
    pub custom: [custom::Input; custom::MAX_MIX_INPUTS],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_mapping: Option<GradientMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tile_options: Option<tiles::TileOptions>,
}

fn empty_custom_inputs(inputs: &[custom::Input; custom::MAX_MIX_INPUTS]) -> bool {
    inputs.iter().all(|input| input.id == 0)
}

type Control = (&'static str, f32, f32, f32);

const fn default_id_tolerance() -> u8 {
    15
}

const fn default_edge_width() -> u16 {
    40
}

impl Generator {
    pub const WEIGHT_LIMIT: i16 = 250;

    const CONTROLS: [Control; 5] = [
        ("Уровень, %", -100., 200., 5.),
        ("Переход, %", 0.1, 100., 5.),
        ("Масштаб", 0.1, 64., 0.5),
        ("Шум края, %", 0., 200., 5.),
        ("Зерно", 0., 100., 1.),
    ];

    const GRADIENT_CONTROLS: [Control; 5] = [
        ("Баланс, %", 0., 100., 5.),
        ("Контраст, %", 0., 100., 5.),
        ("Повторы", 1., 16., 1.),
        ("Угол, °", -360., 360., 15.),
        ("Вершина, %", 0., 100., 5.),
    ];

    const SHAPE_CONTROLS: [Control; 3] = [
        ("Размер, %", 0., 100., 5.),
        ("Жёсткость, %", 0., 100., 5.),
        ("Ширина, %", 0., 100., 1.),
    ];

    pub fn controls(self) -> &'static [Control] {
        match self.kind {
            GeneratorKind::GradientPeak => &Self::GRADIENT_CONTROLS,
            GeneratorKind::GradientLinear | GeneratorKind::GradientMirror => &Self::GRADIENT_CONTROLS[..4],
            GeneratorKind::ShapeDisc | GeneratorKind::ShapeCone => &Self::SHAPE_CONTROLS[..2],
            GeneratorKind::ShapeSquareBorder | GeneratorKind::ShapeCircleBorder => &Self::SHAPE_CONTROLS,
            GeneratorKind::TilePattern => &tiles::EDITOR_CONTROLS,
            _ => &Self::CONTROLS,
        }
    }

    pub fn new(kind: GeneratorKind) -> Self {
        let values = match kind {
            GeneratorKind::Clouds => [50., 10., 2.5, 0., 0.],
            GeneratorKind::Height => [50., 30., 2., 20., 0.],
            GeneratorKind::EdgeWear => [55., 10., 4., 60., 0.],
            GeneratorKind::Dirt => [40., 15., 3., 45., 0.],
            GeneratorKind::Dust => [95., 15., 3., 35., 0.],
            GeneratorKind::RainStreaks => [95., 12., 3., 15., 0.],
            GeneratorKind::Drips => [75., 10., 3., 25., 0.],
            GeneratorKind::CavityGrime => [25., 10., 4., 20., 0.],
            GeneratorKind::CreaseGrime => [45., 15., 4., 35., 0.],
            GeneratorKind::ThinAreas => [55., 20., 3., 0., 0.],
            GeneratorKind::PartGradient => [50., 50., 3., 0., 0.],
            GeneratorKind::PartRandom => [50., 1., 3., 0., 0.],
            GeneratorKind::IdMask => [50., 5., 3., 0., 0.],
            GeneratorKind::CustomMap => [35., 15., 3., 30., 0.],
            GeneratorKind::GradientLinear | GeneratorKind::GradientMirror | GeneratorKind::GradientPeak => {
                [50., 0., 1., 0., 50.]
            }
            GeneratorKind::ShapeDisc => [100., 50., 0., 0., 0.],
            GeneratorKind::ShapeCone => [100., 100., 0., 0., 0.],
            GeneratorKind::ShapeSquareBorder | GeneratorKind::ShapeCircleBorder => [100., 100., 5., 0., 0.],
            GeneratorKind::TilePattern => [4., 4., 100., 100., 85.],
        };
        Self {
            kind,
            values,
            invert: false,
            id_color: None,
            id_tolerance: default_id_tolerance(),
            weights: None,
            edge_width: default_edge_width(),
            custom: Default::default(),
            gradient_mapping: None,
            tile_options: None,
        }
    }

    pub fn value(self, index: usize) -> f32 {
        self.values.get(index).copied().unwrap_or_else(|| {
            if self.kind == GeneratorKind::TilePattern && index < tiles::EDITOR_CONTROLS.len() {
                self.tile_options.unwrap_or_default().value(index)
            } else {
                0.
            }
        })
    }

    pub fn set_value(&mut self, index: usize, value: f32) {
        let Some(&(_, min, max, _)) = self.controls().get(index) else { return };
        let value = value.clamp(min, max);
        let value = if self.kind == GeneratorKind::TilePattern && tiles::discrete(index) {
            value.round()
        } else {
            value
        };
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value;
        } else {
            let mut options = self.tile_options.unwrap_or_default();
            options.set(index, value);
            self.tile_options = (options != tiles::TileOptions::default()).then_some(options);
        }
    }

    pub fn valid(self) -> bool {
        (1..=100).contains(&self.id_tolerance)
            && self.gradient_mapping.is_none_or(|mapping| self.kind.uses_pattern_mapping() && mapping.valid())
            && (self.tile_options.is_none() || self.kind == GeneratorKind::TilePattern)
            && (2..=500).contains(&self.edge_width)
            && self.custom.iter().enumerate().all(|(i, input)| {
                if input.id == 0 {
                    input.weight == 0
                } else {
                    (-200..=200).contains(&input.weight)
                        && self.custom[..i].iter().all(|old| old.id != input.id)
                }
            })
            && self
                .weights
                .is_none_or(|weights| weights.into_iter().all(|w| (-Self::WEIGHT_LIMIT..=Self::WEIGHT_LIMIT).contains(&w)))
            && self.controls().iter().enumerate().all(|(index, &(_, min, max, _))| {
                let value = self.value(index);
                (min..=max).contains(&value)
                    && (self.kind != GeneratorKind::TilePattern || !tiles::discrete(index) || value.fract() == 0.)
            })
    }

    pub fn weights(self) -> [i16; GENERATOR_INPUT_COUNT] {
        self.weights.unwrap_or_else(|| self.kind.weights())
    }

    pub fn weight(self, input: GeneratorInput) -> i16 {
        self.weights()[input as usize]
    }

    pub fn set_weight(&mut self, input: GeneratorInput, value: i16) {
        let mut weights = self.weights();
        weights[input as usize] = value;
        self.weights = (weights != self.kind.weights()).then_some(weights);
    }

    pub fn needs_bake(self) -> bool {
        self.kind == GeneratorKind::CustomMap
            || !empty_custom_inputs(&self.custom)
            || GeneratorInput::ALL
                .into_iter()
                .zip(self.weights())
                .any(|(input, weight)| weight != 0 && input.needs_bake())
    }

    pub fn missing_bake_inputs(self, maps: Option<&BakedInputs>) -> bool {
        GeneratorInput::ALL.into_iter().zip(self.weights()).any(|(input, weight)| {
            weight != 0 && input.needs_bake()
                && maps.is_none_or(|maps| maps.disabled_inputs & (1 << input as u32) != 0)
        })
    }

    /// A lit preview sphere with synthetic bake inputs. The actual mask uses the model's maps.
    pub fn shelf_preview(self, size: u32) -> image::RgbaImage {
        if self.kind.uses_pattern_mapping() {
            let sample = gradient::Sampler::new(self);
            return image::RgbaImage::from_fn(size, size, |x, y| {
                let uv = (Vec2::new(x as f32 + 0.5, size as f32 - y as f32 - 0.5)) / size as f32;
                let [value, alpha] = sample.uv(uv);
                image::Rgba([value, value, value, alpha])
            });
        }
        let point = |x, y| {
            let half = size as f32 * 0.45;
            let x = (x as f32 + 0.5 - size as f32 * 0.5) / half;
            let y = (size as f32 * 0.5 - y as f32 - 0.5) / half;
            let z = 1. - x * x - y * y;
            (z > 0.).then(|| Vec3::new(x, y, z.sqrt()))
        };
        let pack = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        let mut inputs = BakedInputs {
            size, edge_reach: 0.12, disabled_inputs: 0,
            texels: Vec::with_capacity((size * size) as usize),
            part_ids: None, custom: Vec::new(),
        };
        let mut ids = Vec::with_capacity(inputs.texels.capacity());
        let mut custom = Vec::with_capacity(inputs.texels.capacity());
        for y in 0..size {
            for x in 0..size {
                let n = point(x, y).unwrap_or_default();
                let rim = 1. - n.z;
                inputs.texels.push([
                    pack(0.55 + 0.45 * n.y),
                    pack(0.65 + n.y * 0.2 + n.x * 0.1),
                    pack(n.z),
                    pack(0.45 + n.y * 0.55),
                    pack(0.5 + n.y * 0.5),
                    if n.x < -0.2 { 32 } else { 200 },
                    pack(0.5 + n.y * 0.5),
                    pack(0.5 + rim * 0.45 - (0.5 - n.y.abs()).max(0.) * 0.16),
                    pack(1. - rim),
                    pack((n.x + n.y * 0.28).abs()),
                ]);
                ids.push(if n.x + n.y * 0.35 < 0. {
                    [224, 68, 44]
                } else {
                    [242, 200, 63]
                });
                custom.push(pack(0.5 + 0.5 * n.x));
            }
        }
        inputs.part_ids = Some(PartIds::new(ids));
        let weights: Vec<_> = GeneratorInput::ALL.into_iter().zip(self.weights())
            .filter(|&(_, weight)| weight != 0)
            .map(|(input, weight)| (input, f32::from(weight) / 100.))
            .collect();
        let custom = [(custom.as_slice(), size, 1. / 255.)];
        image::RgbaImage::from_fn(size, size, |x, y| {
            let Some(n) = point(x, y) else {
                return image::Rgba([0; 4]);
            };
            let pixel = (y * size + x) as usize;
            let mask = self.sample(&SurfaceSample {
                point: n, normal: n, height: (n.y + 1.) * 0.5,
                extent: Vec3::splat(2.), center: Vec3::ZERO, texel: 2. / size as f32,
                baked: Some(BakedSample { data: &inputs, index: pixel }),
                custom: if self.kind == GeneratorKind::CustomMap { &custom } else { &[] },
                pixel, size,
            }, &weights);
            let light = n.dot(Vec3::new(-0.42, 0.55, 0.72)).max(0.);
            let gray = pack((0.13 + f32::from(mask) / 255. * 0.87) * (0.42 + light * 0.58));
            image::Rgba([gray, gray, gray, 255])
        })
    }

    fn id_match(self, sample: &BakedSample<'_>) -> Option<f32> {
        let ids = sample.data.part_ids.as_ref()?;
        let color = ids.colors[sample.index];
        if color == [0; 3] {
            return None;
        }
        let target = self.id_color.or_else(|| ids.palette.first().copied())?;
        let rgb = |value: [u8; 3]| Vec3::from_array(value.map(f32::from)) / 255.;
        let distance = rgb(color).distance(rgb(target));
        let tolerance = f32::from(self.id_tolerance) / 100.;
        Some(1. - patterns::transition(tolerance * 0.5, tolerance, distance))
    }

    fn base(self, surface: &SurfaceSample<'_>, weights: &[(GeneratorInput, f32)]) -> Option<f32> {
        use GeneratorInput::*;
        let baked = surface.baked.as_ref();
        let reach = baked.map_or(0., |sample| sample.data.edge_reach);
        let texel =
            baked.map(|sample| sample.data.texels[sample.index].map(|v| f32::from(v) / 255.));
        let up = texel.map_or(surface.normal.y, |v| v[6] * 2. - 1.);
        let point = surface.point;
        let scale = self.values[2];
        let seed = self.values[4];
        let q = point * scale + Vec3::splat(seed * 17.31);
        let mut combined = 0.;
        let mut available = weights.is_empty()
            && empty_custom_inputs(&self.custom)
            && self.kind != GeneratorKind::CustomMap;
        for &(input, weight) in weights {
            if baked.is_some_and(|sample| sample.data.disabled_inputs & (1 << input as u32) != 0) {
                continue;
            }
            let value = match input {
                Height => Some(surface.height),
                Up => Some(up.max(0.)),
                Overhang => Some((-up).max(0.)),
                Streaks => Some(patterns::transition(
                    0.45,
                    0.8,
                    fbm(point * Vec3::new(7., 0.45, 7.) * scale + Vec3::splat(seed)),
                )),
                Noise => Some(fbm(q)),
                Id => baked.and_then(|sample| self.id_match(sample)),
                Wood => Some(patterns::grain(
                    point,
                    surface.normal,
                    scale,
                    surface.extent,
                    seed,
                )),
                Cells => Some(patterns::cells(q * 4.)),
                Scratches => Some(patterns::scratches(
                    point,
                    surface.normal,
                    scale,
                    surface.texel,
                    seed,
                )),
                Front => {
                    let direction = (point - surface.center).normalize_or_zero();
                    Some(direction.z.clamp(-1., 1.).acos() / std::f32::consts::PI)
                }
                _ => texel.map(
                    |[
                        ao,
                        cavity,
                        thickness,
                        sky,
                        part_gradient,
                        part_random,
                        _,
                        curvature,
                        edge,
                        crease,
                    ]| {
                        let curvature = curvature * 2. - 1.;
                        let thick = 0.15 + 0.85 * patterns::transition(0.15, 0.75, thickness);
                        let distance_mask = |distance| {
                            1. - patterns::transition(
                                0.,
                                f32::from(self.edge_width) / 1000.,
                                distance * reach,
                            )
                        };
                        match input {
                            Edges => distance_mask(edge) * thick,
                            Convexity => curvature.max(0.) * thick,
                            Concavity => (-curvature).max(0.),
                            Occlusion => 1. - ao,
                            Cavity => 1. - cavity,
                            Sky => sky,
                            Thin => 1. - thickness,
                            PartGradient => part_gradient,
                            PartRandom => part_random,
                            Creases => distance_mask(crease),
                            _ => unreachable!(),
                        }
                    },
                ),
            };
            if let Some(value) = value {
                combined += value * weight;
                available = true;
            }
        }
        for &(values, size, weight) in surface.custom {
            let index = sample_index(surface.pixel, surface.size, size);
            combined += f32::from(values[index]) * weight;
            available = true;
        }
        available.then_some(combined)
    }

    fn sample(self, surface: &SurfaceSample<'_>, weights: &[(GeneratorInput, f32)]) -> u8 {
        self.base(surface, weights)
            .map_or(0, |v| self.finish(surface.point, v))
    }

    fn finish(self, point: Vec3, mut v: f32) -> u8 {
        let [level, contrast, scale, strength, seed] = self.values;
        if strength > 0. {
            let q = point * scale + Vec3::splat(seed * 17.31);
            let grit = q * 2.;
            let grunge = fbm(grit) * 0.6
                + fbm(grit * 5.3 + Vec3::new(4.1, 1.3, 7.9)) * 0.3
                + noise(grit * 23. + Vec3::new(9.4, 2.2, 5.6)) * 0.1;
            let broad = fbm(q * 0.35 + Vec3::new(11.7, 3.1, 5.9)) - 0.5;
            v += ((grunge - 0.5) * 1.3 + broad * 2.6) * strength / 100.;
        }
        let level = level / 100.;
        let contrast = contrast / 100.;
        let mask = smooth(((v - (level - contrast)) / (contrast * 2.)).clamp(0., 1.));
        ((if self.invert { 1. - mask } else { mask }) * 255.).round() as u8
    }
}

#[derive(Debug)]
pub(in crate::editor) struct GeneratorSurface {
    mesh: MeshData,
    slot: usize,
    uv_offset: Vec2,
    size: u32,
    transform: Transform,
    baked: Option<Arc<BakedInputs>>,
    live: Option<Arc<custom::LivePlane>>,
    geometry: OnceLock<Coordinates>,
    normals: OnceLock<Vec<[i8; 3]>>,
    maps: Mutex<Vec<(Generator, GeneratorMap)>>,
}

#[derive(Clone, Debug)]
pub(in crate::editor) struct GeneratorMap {
    pub values: Arc<Vec<u8>>,
    pub coverage: Option<Arc<Vec<u8>>>,
}

#[derive(Debug)]
struct Coordinates {
    points: Vec<Vec3>,
    min: Vec3,
    extent: Vec3,
    world_min: Vec3,
    world_extent: Vec3,
}

impl GeneratorSurface {
    pub fn disabled_inputs(&self) -> u32 {
        self.baked.as_ref().map_or(0, |inputs| inputs.disabled_inputs)
    }

    pub fn id_palette(&self) -> &[[u8; 3]] {
        self.baked
            .as_ref()
            .and_then(|input| input.part_ids.as_ref())
            .map_or(&[], |ids| ids.palette.as_slice())
    }

    pub fn new(
        mesh: MeshData,
        slot: usize,
        size: u32,
        uv_offset: Vec2,
        transform: Transform,
        baked: Option<Arc<BakedInputs>>,
    ) -> Self {
        Self {
            mesh,
            slot,
            uv_offset,
            size,
            transform,
            baked,
            live: None,
            geometry: OnceLock::new(),
            normals: OnceLock::new(),
            maps: Mutex::new(Vec::new()),
        }
    }

    pub fn with_live(mut self, plane: Arc<custom::LivePlane>) -> Self {
        self.live = Some(plane);
        self
    }

    pub fn matches(
        &self,
        mesh: &MeshData,
        slot: usize,
        size: u32,
        uv_offset: Vec2,
        transform: Transform,
        baked: Option<&Arc<BakedInputs>>,
    ) -> bool {
        self.size == size
            && self.live.is_none()
            && self.slot == slot
            && self.uv_offset == uv_offset
            && std::ptr::eq(&*self.mesh, &**mesh)
            && self.transform == transform
            && match (self.baked.as_ref(), baked) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }

    pub fn map(&self, generator: Generator) -> GeneratorMap {
        let mut cache = self.maps.lock().unwrap();
        if let Some(i) = cache.iter().position(|(cached, _)| *cached == generator) {
            let entry = cache.remove(i);
            let pixels = entry.1.clone();
            cache.push(entry);
            return pixels;
        }
        let pixels = if generator.kind.uses_pattern_mapping() {
            self.pattern_map(generator)
        } else {
            GeneratorMap { values: self.geometry_map(generator), coverage: None }
        };
        // Recent settings are reused during strokes and undo.
        if cache.len() == 8 {
            cache.remove(0);
        }
        cache.push((generator, pixels.clone()));
        pixels
    }

    fn pattern_map(&self, generator: Generator) -> GeneratorMap {
        use crate::editor::model_paint_data::texture_fill::{pad_texels, raster};
        let size = self.size as usize;
        let sampler = gradient::Sampler::new(generator);
        let mapping = generator.gradient_mapping.unwrap_or_default();
        if mapping.full_coverage() {
            let values = (0..size * size).map(|i| {
                let uv = Vec2::new((i % size) as f32 + 0.5, size as f32 - (i / size) as f32 - 0.5);
                sampler.uv(uv / size as f32)[0]
            }).collect();
            return GeneratorMap { values: Arc::new(values), coverage: None };
        }
        let mut samples = vec![[0; 2]; size * size];
        if mapping.volume.is_some() {
            let mut covered = vec![0; samples.len()];
            raster(
                &self.mesh, self.slot, self.size, self.uv_offset, Transform::IDENTITY,
                &self.mesh.positions, &mut covered, |index, _, point, normal, _, _| {
                    samples[index] = sampler.projected(point, normal);
                },
            );
            pad_texels(size, &mut covered, |to, from| samples[to] = samples[from]);
        } else {
            for (i, pixel) in samples.iter_mut().enumerate() {
                let uv = Vec2::new((i % size) as f32 + 0.5, size as f32 - (i / size) as f32 - 0.5);
                *pixel = sampler.uv(uv / size as f32);
            }
        }
        let (values, coverage) = samples.into_iter().map(|[v, a]| (v, a)).unzip();
        GeneratorMap { values: Arc::new(values), coverage: Some(Arc::new(coverage)) }
    }

    fn geometry_map(&self, generator: Generator) -> Arc<Vec<u8>> {
        let weights: Vec<_> = GeneratorInput::ALL
            .into_iter()
            .zip(generator.weights())
            .filter(|&(_, weight)| weight != 0)
            .map(|(input, weight)| (input, f32::from(weight) / 100.))
            .collect();
        let needs_bake = generator.needs_bake();
        let baked = self.baked.as_ref().filter(|_| needs_bake);
        let custom: Vec<_> = generator
            .custom
            .iter()
            .filter(|input| input.id != 0 && input.weight != 0)
            .filter_map(|input| {
                let weight = f32::from(input.weight) / 25500.;
                if let Some(live) = self.live.as_ref().filter(|live| live.id == input.id) {
                    return Some((live.values.as_slice(), live.size, weight));
                }
                let baked = baked?;
                let plane = baked.custom.iter().find(|plane| plane.recipe.id == input.id)?;
                Some((plane.values.as_slice(), baked.size, weight))
            })
            .collect();
        let missing_preset_maps = generator.weights.is_none()
            && baked.is_none()
            && needs_bake
            && empty_custom_inputs(&generator.custom);
        let coordinates = self.geometry.get_or_init(|| self.coordinates());
        let normals = weights
            .iter()
            .any(|(input, _)| input.needs_mesh_normal(baked.is_some()))
            .then(|| self.normals.get_or_init(|| self.raster_normals()));
        let use_local = matches!(
            generator.kind,
            GeneratorKind::Clouds | GeneratorKind::Height
        );
        let (min, extent) = if use_local {
            (coordinates.min, coordinates.extent)
        } else {
            (coordinates.world_min, coordinates.world_extent)
        };
        let center = min + extent * 0.5;
        let footprint = extent.length().max(1e-4) / self.size as f32;
        let simple =
            use_local && generator.weights.is_none() && empty_custom_inputs(&generator.custom);
        let height_scale = extent.y.max(1e-4);
        let position = |point| {
            if use_local {
                point
            } else {
                self.transform.transform_point(point)
            }
        };
        let use_footprint = generator.weight(GeneratorInput::Scratches) != 0;
        Arc::new(
            coordinates
                .points
                .iter()
                .enumerate()
                .map(|(i, &p)| {
                    if !p.is_finite() || missing_preset_maps {
                        return 0;
                    }
                    if simple {
                        let value = if generator.kind == GeneratorKind::Clouds {
                            fbm(p * generator.values[2] + Vec3::splat(generator.values[4] * 17.31))
                        } else {
                            (p.y - min.y) / height_scale
                        };
                        return generator.finish(p, value);
                    }
                    let point = position(p);
                    let normal = normals.map_or(Vec3::ZERO, |normals| {
                        let n = Vec3::from_array(normals[i].map(f32::from)).normalize_or_zero();
                        if use_local {
                            self.transform.rotation.inverse() * n
                        } else {
                            n
                        }
                    });
                    let right = i % self.size as usize + 1 < self.size as usize;
                    let texel = if use_footprint && right && coordinates.points[i + 1].is_finite() {
                        position(coordinates.points[i + 1]).distance(point)
                    } else {
                        footprint
                    };
                    generator.sample(
                        &SurfaceSample {
                            point,
                            normal,
                            height: (point.y - min.y) / height_scale,
                            extent,
                            center,
                            texel,
                            baked: baked.map(|data| BakedSample {
                                data,
                                index: data.index(i, self.size),
                            }),
                            custom: &custom,
                            pixel: i,
                            size: self.size,
                        },
                        &weights,
                    )
                })
                .collect(),
        )
    }

    fn coordinates(&self) -> Coordinates {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut world_min = min;
        let mut world_max = max;
        for &p in &self.mesh.positions {
            min = min.min(p);
            max = max.max(p);
            let world = self.transform.transform_point(p);
            world_min = world_min.min(world);
            world_max = world_max.max(world);
        }
        let points = self.raster(
            Vec3::NAN,
            |face, triangle| triangle.map(|i| self.mesh.positions[face.corners[i].vertex]),
            |point| point,
        );
        Coordinates {
            points,
            min,
            extent: max - min,
            world_min,
            world_extent: world_max - world_min,
        }
    }

    fn raster_normals(&self) -> Vec<[i8; 3]> {
        let mut vertex_normals = vec![Vec3::ZERO; self.mesh.positions.len()];
        for face in self.mesh.faces.iter().filter(|face| face.smooth) {
            let normal = self.mesh.normal(face);
            for corner in &face.corners {
                vertex_normals[corner.vertex] += normal;
            }
        }
        let inverse_scale =
            self.transform.scale.signum() / self.transform.scale.abs().max(Vec3::splat(1e-6));
        self.raster(
            [0; 3],
            |face, triangle| {
                let flat = self.mesh.normal(face).normalize_or_zero();
                triangle.map(|i| {
                    let corner = &face.corners[i];
                    let normal = corner.source_normal.unwrap_or_else(|| {
                        if face.smooth {
                            vertex_normals[corner.vertex]
                                .try_normalize()
                                .unwrap_or(flat)
                        } else {
                            flat
                        }
                    });
                    self.transform.rotation * (normal * inverse_scale).normalize_or_zero()
                })
            },
            |normal| {
                normal
                    .normalize_or_zero()
                    .to_array()
                    .map(|x| (x * 127.).round() as i8)
            },
        )
    }

    fn raster<T: Clone>(
        &self,
        empty: T,
        triangle_values: impl Fn(&Face, [usize; 3]) -> [Vec3; 3],
        pack: impl Fn(Vec3) -> T,
    ) -> Vec<T> {
        let size = self.size as usize;
        let mut samples = vec![empty; size * size];
        let extent = Vec2::splat(self.size as f32);
        for face in self
            .mesh
            .faces
            .iter()
            .filter(|face| face.material == self.slot)
        {
            for triangle in self.mesh.triangles(face) {
                let uv = triangle.map(|i| (face.corners[i].uv - self.uv_offset) * extent);
                let values = triangle_values(face, triangle);
                let a = uv[1] - uv[0];
                let b = uv[2] - uv[0];
                let area = a.perp_dot(b);
                if !area.is_finite() || area.abs() <= 1e-10 {
                    continue;
                }
                let lo = uv[0]
                    .min(uv[1])
                    .min(uv[2])
                    .floor()
                    .clamp(Vec2::ZERO, extent)
                    .as_uvec2();
                let hi = uv[0]
                    .max(uv[1])
                    .max(uv[2])
                    .ceil()
                    .clamp(Vec2::ZERO, extent)
                    .as_uvec2();
                for y in lo.y..hi.y {
                    for x in lo.x..hi.x {
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - uv[0];
                        let w1 = p.perp_dot(b) / area;
                        let w2 = a.perp_dot(p) / area;
                        let w0 = 1. - w1 - w2;
                        if w0.min(w1).min(w2) >= -1e-6 {
                            samples[y as usize * size + x as usize] =
                                pack(values[0] * w0 + values[1] * w1 + values[2] * w2);
                        }
                    }
                }
            }
        }
        samples
    }
}
