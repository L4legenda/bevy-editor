//! Painted mask data and non-destructive filters.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

use super::texture_fill::{Bitmap, GraphImages};

#[path = "model_paint_mask_spatial.rs"]
mod spatial;
pub(in crate::editor) use spatial::Spatial;
pub(in crate::editor) use spatial::fbm;
#[path = "model_paint_mask_generator.rs"]
mod generator;
pub(in crate::editor) use generator::{
    BakedInputs, Generator, GeneratorInput, GeneratorKind, GeneratorSurface, GradientMapping,
    GradientProjection, GradientVolume, INPUT_CHANNELS, PartIds,
};
use generator::GeneratorMap;

pub(in crate::editor) const MAX_FILTERS: usize = 16;
pub(in crate::editor) const MAX_MASK_LAYERS: usize = 16;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema), schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum FilterKind {
    Levels = 0,
    Invert = 1,
    Contrast = 2,
    Blur = 3,
    Sharpen = 4,
    Warp = 5,
    Noise = 6,
    Outline = 9,
    Threshold = 10,
    HistogramScan = 11,
    ContrastLuminosity = 12,
    DirectionalBlur = 13,
    Clamp = 14,
    Smoothstep = 15,
    Posterize = 16,
    Pixelate = 17,
}

impl FilterKind {
    pub(in crate::editor) const ALL: [Self; 16] = [
        Self::Levels,
        Self::Invert,
        Self::Contrast,
        Self::Blur,
        Self::Sharpen,
        Self::Warp,
        Self::Noise,
        Self::Outline,
        Self::Threshold,
        Self::HistogramScan,
        Self::ContrastLuminosity,
        Self::DirectionalBlur,
        Self::Clamp,
        Self::Smoothstep,
        Self::Posterize,
        Self::Pixelate,
    ];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::Levels => "Уровни",
            Self::Invert => "Инверсия",
            Self::Contrast => "Контраст",
            Self::Blur => "Размытие",
            Self::Sharpen => "Резкость",
            Self::Warp => "Искажение",
            Self::Noise => "Шум",
            Self::Outline => "Контур",
            Self::Threshold => "Порог",
            Self::HistogramScan => "Сканирование гистограммы",
            Self::ContrastLuminosity => "Контраст и яркость",
            Self::DirectionalBlur => "Направленное размытие",
            Self::Clamp => "Ограничение диапазона",
            Self::Smoothstep => "Плавный порог",
            Self::Posterize => "Постеризация",
            Self::Pixelate => "Пикселизация",
        }
    }

    /// (Label, initial value, min, max) in the units shown in the inspector.
    pub(in crate::editor) fn controls(self) -> &'static [(&'static str, f32, f32, f32)] {
        match self {
            Self::Levels => &[
                ("Чёрная точка, %", 0., 0., 100.),
                ("Белая точка, %", 100., 0., 100.),
                ("Гамма, %", 100., 10., 400.),
                ("Выходной чёрный, %", 0., 0., 100.),
                ("Выходной белый, %", 100., 0., 100.),
            ],
            Self::Invert => &[],
            Self::Contrast => &[("Центр, %", 50., 0., 100.), ("Контраст, %", 80., 0., 100.)],
            Self::Blur => &[("Радиус", 6., 0., 64.), ("Повтор на границах", 0., 0., 1.)],
            Self::Sharpen => &[("Сила", 1., 0., 4.), ("Радиус", 1.5, 0.5, 8.)],
            Self::Warp => &[
                ("Сила, %", 15., 0., 100.),
                ("Масштаб", 8., 1., 64.),
                ("Зерно", 0., 0., 65535.),
            ],
            Self::Noise => &[
                ("Сила, %", 60., 0., 100.),
                ("Масштаб", 12., 1., 64.),
                ("Зерно", 0., 0., 100.),
            ],
            Self::Outline => &[
                ("Порог, %", 50., 0., 100.),
                ("Ширина, %", 2., 0., 100.),
                ("Положение", 3., 1., 3.),
                ("Размытие, %", 0., 0., 100.),
                ("Кривая, %", 0., -100., 100.),
                ("Инверсия", 0., 0., 1.),
            ],
            Self::Threshold => &[("Порог, %", 1., 0., 100.), ("Сравнение", 0., 0., 3.)],
            Self::HistogramScan => &[
                ("Позиция, %", 50., 0., 100.),
                ("Контраст, %", 0., 0., 100.),
                ("Инверсия", 0., 0., 1.),
            ],
            Self::ContrastLuminosity => &[
                ("Контраст, %", 0., -100., 100.),
                ("Яркость, %", 0., -100., 100.),
            ],
            Self::DirectionalBlur => &[("Интенсивность", 1., 0., 16.), ("Угол, °", 0., 0., 360.)],
            Self::Clamp | Self::Smoothstep => &[("Минимум, %", 0., 0., 100.), ("Максимум, %", 100., 0., 100.)],
            Self::Posterize => &[("Градации", 8., 1., 64.)],
            Self::Pixelate => &[("Ячеек на сторону", 32., 1., 128.)],
        }
    }

    pub fn step(self, slot: usize) -> f32 {
        match (self, slot) {
            (Self::Blur, _) | (Self::Warp | Self::Noise, 1..=2) => 1.,
            (Self::Sharpen, 0) => 0.25,
            (Self::Sharpen, _) => 0.5,
            (Self::Outline, 1 | 3) => 0.25,
            (Self::Outline, 2 | 5) => 1.,
            (Self::Threshold, 1) | (Self::HistogramScan, 2) => 1.,
            (Self::DirectionalBlur, 0) => 0.25,
            (Self::Clamp | Self::Smoothstep | Self::Posterize | Self::Pixelate, _) => 1.,
            _ => 5.,
        }
    }

    pub(in crate::editor) fn spatial(self) -> bool {
        matches!(
            self,
            Self::Blur | Self::Sharpen | Self::Warp | Self::Noise | Self::Outline | Self::DirectionalBlur | Self::Pixelate
        )
    }

    fn discrete(self, slot: usize) -> bool {
        matches!((self, slot),
            (Self::Threshold, 1) | (Self::HistogramScan, 2) | (Self::Posterize | Self::Pixelate, 0)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct Filter {
    pub kind: FilterKind,
    pub enabled: bool,
    pub values: [f32; 3],
    #[serde(
        default = "default_levels_output",
        skip_serializing_if = "is_default_levels_output"
    )]
    pub output: [f32; 2],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wrap: bool,
    #[serde(default, skip_serializing_if = "OutlineOptions::is_default")]
    outline: OutlineOptions,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct OutlineOptions {
    blur: f32,
    curve: f32,
    inverted: bool,
}

impl OutlineOptions {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

const fn default_levels_output() -> [f32; 2] {
    [0., 100.]
}

fn is_default_levels_output(output: &[f32; 2]) -> bool {
    *output == default_levels_output()
}

impl Filter {
    pub fn new(kind: FilterKind) -> Self {
        let mut values = [0.; 3];
        for (v, &(_, initial, _, _)) in values.iter_mut().zip(kind.controls()) {
            *v = initial;
        }
        Self {
            kind,
            enabled: true,
            values,
            output: default_levels_output(),
            wrap: false,
            outline: OutlineOptions::default(),
        }
    }

    pub fn valid(&self) -> bool {
        self.values.iter().all(|v| v.is_finite())
            && (self.kind != FilterKind::Outline || self.values[2].fract() == 0.)
            && self
                .kind
                .controls()
                .iter()
                .enumerate()
                .all(|(slot, &(_, _, min, max))| {
                    let value = self.value(slot);
                    (min..=max).contains(&value)
                        && (!self.kind.discrete(slot) || value.fract() == 0.)
                })
    }

    pub fn value(&self, slot: usize) -> f32 {
        if self.kind == FilterKind::Blur && slot == 1 {
            return f32::from(u8::from(self.wrap));
        }
        if self.kind == FilterKind::Outline {
            return match slot {
                3 => self.outline.blur,
                4 => self.outline.curve,
                5 => f32::from(u8::from(self.outline.inverted)),
                _ => self.values.get(slot).copied().unwrap_or_default(),
            };
        }
        self.values
            .iter()
            .chain(&self.output)
            .nth(slot)
            .copied()
            .unwrap_or(0.)
    }

    pub fn set_value(&mut self, slot: usize, value: f32) {
        if self.kind == FilterKind::Blur && slot == 1 {
            self.wrap = value >= 0.5;
            return;
        }
        let Some(&(_, initial, min, max)) = self.kind.controls().get(slot) else {
            return;
        };
        let value = if value.is_finite() {
            value.clamp(min, max)
        } else {
            initial
        };
        if self.kind == FilterKind::Outline {
            match slot {
                2 => self.values[slot] = value.round(),
                3 => self.outline.blur = value,
                4 => self.outline.curve = value,
                5 => self.outline.inverted = value >= 0.5,
                _ => self.values[slot] = value,
            }
        } else if let Some(target) = self.values.iter_mut().chain(&mut self.output).nth(slot) {
            *target = if self.kind.discrete(slot) { value.round() } else { value };
        }
    }

    fn apply(self, value: u8) -> u8 {
        let [a, b, c] = self.values.map(|v| v / 100.);
        let s = value as f32 / 255.;
        let value = match self.kind {
            FilterKind::Levels => {
                let value = ((s - a) / (b - a).max(0.0001))
                    .clamp(0., 1.)
                    .powf(1. / c.max(0.001));
                let [black, white] = self.output.map(|v| v / 100.);
                (black + (white - black) * value).clamp(0., 1.)
            }
            FilterKind::Invert => 1. - s,
            FilterKind::Contrast => {
                let w = ((1. - b) * 0.5).max(0.0001);
                let t = ((s - (a - w)) / (2. * w)).clamp(0., 1.);
                t * t * (3. - 2. * t)
            }
            FilterKind::Threshold => {
                let passes = match self.values[1] as u32 {
                    0 => s > a,
                    1 => s >= a,
                    2 => s < a,
                    _ => s <= a,
                };
                f32::from(u8::from(passes))
            }
            FilterKind::HistogramScan => {
                let value = if a == 0. || a == 1. {
                    a
                } else {
                    let width = (1. - b).max(0.0001);
                    ((s - (1. - a)) / width + 0.5).clamp(0., 1.)
                };
                if c > 0. { 1. - value } else { value }
            }
            FilterKind::ContrastLuminosity => {
                let slope = if a < 0. { 1. + a } else { 1. / (1. - a).max(0.0001) };
                ((s - 0.5) * slope + 0.5 + b).clamp(0., 1.)
            }
            FilterKind::Clamp => s.clamp(a.min(b), a.max(b)),
            FilterKind::Smoothstep if a == b => f32::from(u8::from(s >= a)),
            FilterKind::Smoothstep => spatial::smooth(((s - a) / (b - a)).clamp(0., 1.)),
            FilterKind::Posterize => {
                let intervals = self.values[0] - 1.;
                if intervals > 0. { (s * intervals).round() / intervals } else { 0.5 }
            }
            _ => unreachable!("spatial filters use a separate pass"),
        };
        (value * 255.).round() as u8
    }
}

#[cfg(test)]
#[test]
fn threshold_modes_preserve_equality_and_direction() {
    let mut filter = Filter::new(FilterKind::Threshold);
    filter.set_value(0, 0.);
    for (mode, equal, higher) in [(0., 0, 255), (1., 255, 255), (2., 0, 0), (3., 255, 0)] {
        filter.set_value(1, mode);
        assert_eq!([filter.apply(0), filter.apply(128)], [equal, higher]);
    }
}

#[cfg(test)]
#[test]
fn mask_ranges_clamp_or_remap_and_posterize_has_discrete_steps() {
    let mut clamp = Filter::new(FilterKind::Clamp);
    clamp.set_value(0, 25.);
    clamp.set_value(1, 75.);
    assert_eq!([clamp.apply(0), clamp.apply(100), clamp.apply(255)], [64, 100, 191]);
    let mut smooth = Filter::new(FilterKind::Smoothstep);
    smooth.set_value(0, 25.);
    smooth.set_value(1, 75.);
    assert_eq!([smooth.apply(60), smooth.apply(128), smooth.apply(200)], [0, 129, 255]);
    smooth.set_value(1, 25.);
    assert_eq!([smooth.apply(63), smooth.apply(64)], [0, 255]);
    let mut posterize = Filter::new(FilterKind::Posterize);
    posterize.set_value(0, 3.7);
    assert_eq!(posterize.value(0), 4.);
    assert_eq!([0, 70, 130, 255].map(|gray| posterize.apply(gray)), [0, 85, 170, 255]);
}

pub(in crate::editor) enum MaskPass {
    Lookup(Box<[u32; 256]>),
    Spatial(Spatial),
    Affine(Option<Arc<Vec<u8>>>),
    GeneratorLayer(MaskLayer, GeneratorMap),
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) enum MaskBlend {
    #[default]
    Normal,
    Multiply,
    Add,
    Subtract,
    Lighten,
    Darken,
}

impl MaskBlend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Обычный",
            Self::Multiply => "Умножение",
            Self::Add => "Сложение",
            Self::Subtract => "Вычитание",
            Self::Lighten => "Светлее",
            Self::Darken => "Темнее",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Normal => Self::Multiply,
            Self::Multiply => Self::Add,
            Self::Add => Self::Subtract,
            Self::Subtract => Self::Lighten,
            Self::Lighten => Self::Darken,
            Self::Darken => Self::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct MaskLayer {
    pub value: u8,
    pub opacity: u8,
    pub blend: MaskBlend,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,
}

impl Default for MaskLayer {
    fn default() -> Self {
        Self {
            value: 100,
            opacity: 100,
            blend: MaskBlend::Normal,
            enabled: true,
            generator: None,
        }
    }
}

impl MaskLayer {
    pub fn valid(self) -> bool {
        self.value <= 100 && self.opacity <= 100 && self.generator.is_none_or(Generator::valid)
    }
    fn apply(self, base: u8, l: f32) -> u8 {
        self.apply_covered(base, l, 255)
    }

    fn apply_covered(self, base: u8, l: f32, coverage: u8) -> u8 {
        let s = base as f32 / 255.;
        let w = self.opacity as f32 / 100. * (coverage as f32 / 255.);
        let mixed = match self.blend {
            MaskBlend::Normal => l,
            MaskBlend::Multiply => s * l,
            MaskBlend::Add => s + l,
            MaskBlend::Subtract => s - l,
            MaskBlend::Lighten => s.max(l),
            MaskBlend::Darken => s.min(l),
        };
        ((s * (1. - w) + mixed * w).clamp(0., 1.) * 255.).round() as u8
    }
}

#[derive(Debug)]
pub(in crate::editor) struct Mask {
    pub pixels: Arc<Vec<u8>>,
    pub filters: Vec<Filter>,
    pub layers: Vec<MaskLayer>,
    pub generator: Option<Generator>,
    pub affine: bool,
    pub context: Option<Arc<GeneratorSurface>>,
    pub graph: Option<Arc<GraphMask>>,
    geometry_coverage: Option<Arc<Vec<u8>>>,
    effective: OnceLock<Arc<Vec<u8>>>,
}

type CachedPlane = Arc<Mutex<Option<(u32, Arc<Vec<u8>>)>>>;

/// The graph supplies the base multiplier. Painted mask channels stay editable.
#[derive(Clone, Debug)]
pub(in crate::editor) struct GraphMask {
    pub payload: Arc<serde_json::Value>,
    pub images: Arc<GraphImages>,
    pub output: Option<Arc<Bitmap>>,
    plane: CachedPlane,
}

impl PartialEq for GraphMask {
    fn eq(&self, other: &Self) -> bool {
        self.payload == other.payload && self.images == other.images && self.output == other.output
    }
}

impl GraphMask {
    pub fn new(
        payload: Arc<serde_json::Value>,
        images: Arc<GraphImages>,
        output: Option<Arc<Bitmap>>,
    ) -> Self {
        Self {
            payload,
            images,
            output,
            plane: Default::default(),
        }
    }

    fn plane(&self, size: u32) -> Option<Arc<Vec<u8>>> {
        let pixels = self.output.as_ref()?.pixels();
        let mut cache = self.plane.lock().unwrap();
        if let Some((_, plane)) = cache.as_ref().filter(|(edge, _)| *edge == size) {
            return Some(plane.clone());
        }
        let samples = |pixel, source| (2 * pixel + 1) * source / (2 * size);
        let plane: Arc<Vec<u8>> = Arc::new(
            (0..size)
                .flat_map(|y| {
                    (0..size).map(move |x| {
                        pixels
                            .get_pixel(samples(x, pixels.width()), samples(y, pixels.height()))
                            .0[0]
                    })
                })
                .collect(),
        );
        *cache = Some((size, plane.clone()));
        Some(plane)
    }
}

impl PartialEq for Mask {
    fn eq(&self, other: &Self) -> bool {
        self.same_settings(other) && self.pixels == other.pixels
    }
}

impl Clone for Mask {
    fn clone(&self) -> Self {
        // Copy-on-write callers mutate the pixels or filters after cloning.
        self.with_pixels(self.pixels.clone())
    }
}

impl Mask {
    pub fn new(pixels: Arc<Vec<u8>>, filters: Vec<Filter>) -> Self {
        Self {
            pixels,
            filters,
            layers: Vec::new(),
            generator: None,
            affine: false,
            context: None,
            graph: None,
            geometry_coverage: None,
            effective: OnceLock::new(),
        }
    }

    pub fn edit_filters(&mut self) -> &mut Vec<Filter> {
        self.effective.take();
        &mut self.filters
    }

    pub fn geometry_coverage(&self) -> Option<&[u8]> {
        self.geometry_coverage.as_deref().map(Vec::as_slice)
    }

    pub fn set_geometry_coverage(&mut self, coverage: Vec<u8>) {
        debug_assert_eq!(coverage.len(), self.pixels.len() / 4);
        self.geometry_coverage = Some(Arc::new(coverage));
        self.effective.take();
    }

    pub fn edit_layers(&mut self) -> &mut Vec<MaskLayer> {
        self.effective.take();
        &mut self.layers
    }

    pub fn edit_generator(&mut self) -> &mut Option<Generator> {
        self.effective.take();
        &mut self.generator
    }

    pub fn use_affine(&mut self) {
        if !self.affine {
            for pixel in Arc::make_mut(&mut self.pixels).chunks_exact_mut(4) {
                pixel[1] = 0;
                pixel[2] = 0;
            }
            self.affine = true;
            self.effective.take();
        }
    }

    pub fn set_graph(&mut self, graph: Arc<GraphMask>) {
        self.use_affine();
        self.graph = Some(graph);
        self.effective.take();
    }

    pub fn clear_graph(&mut self) {
        self.graph = None;
        self.effective.take();
    }

    pub fn uses_geometry(&self) -> bool {
        self.generator.is_some() || self.layers.iter().any(|layer| layer.generator.is_some())
    }

    pub fn needs_bake(&self) -> bool {
        self.generator
            .into_iter()
            .chain(self.layers.iter().filter_map(|layer| layer.generator))
            .any(Generator::needs_bake)
    }

    pub fn uses_custom_map(&self, id: u64) -> bool {
        self.generator
            .filter(|_| self.affine)
            .into_iter()
            .chain(
                self.layers
                    .iter()
                    .filter(|layer| layer.enabled && layer.opacity != 0)
                    .filter_map(|layer| layer.generator),
            )
            .any(|generator| {
                generator
                    .custom
                    .iter()
                    .any(|input| input.id == id && input.weight != 0)
            })
    }

    pub fn bind(&mut self, context: Arc<GeneratorSurface>) {
        self.context = Some(context);
        self.effective.take();
    }

    pub fn same_settings(&self, other: &Self) -> bool {
        self.affine == other.affine
            && self.generator == other.generator
            && self.filters == other.filters
            && self.layers == other.layers
            && self.geometry_coverage == other.geometry_coverage
            && match (&self.graph, &other.graph) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                (None, None) => true,
                _ => false,
            }
    }

    pub fn with_pixels(&self, pixels: Arc<Vec<u8>>) -> Self {
        let geometry_coverage = self.geometry_coverage.as_ref().map(|coverage| {
            let area = pixels.len() / 4;
            if coverage.len() == area {
                return coverage.clone();
            }
            let old = coverage.len().isqrt();
            let new = area.isqrt();
            let sample = |p| (2 * p + 1) * old / (2 * new);
            Arc::new(
                (0..new)
                    .flat_map(|y| (0..new).map(move |x| coverage[sample(y) * old + sample(x)]))
                    .collect(),
            )
        });
        Self {
            pixels,
            filters: self.filters.clone(),
            layers: self.layers.clone(),
            generator: self.generator,
            affine: self.affine,
            context: self.context.clone(),
            graph: self.graph.clone(),
            geometry_coverage,
            effective: OnceLock::new(),
        }
    }

    fn has_operations(&self) -> bool {
        self.affine
            || self.geometry_coverage.is_some()
            || self.filters.iter().any(|f| f.enabled)
            || self.layers.iter().any(|l| l.enabled && l.opacity != 0)
    }

    pub fn passes(&self, size: u32) -> Vec<MaskPass> {
        let mut passes = Vec::new();
        let mut lookup = None;
        let identity = || Box::new(std::array::from_fn(|i| i as u32));
        if self.affine {
            let generator = self
                .generator
                .zip(self.context.as_ref())
                .map(|(g, c)| c.map(g).values);
            let graph = self.graph.as_ref().and_then(|graph| graph.plane(size));
            let map = match (graph, generator) {
                (Some(graph), Some(generator)) => Some(Arc::new(
                    graph
                        .iter()
                        .zip(generator.iter())
                        .map(|(&a, &b)| ((u16::from(a) * u16::from(b) + 127) / 255) as u8)
                        .collect(),
                )),
                (graph, generator) => graph.or(generator),
            };
            passes.push(MaskPass::Affine(map));
        }
        for layer in self.layers.iter().filter(|l| l.enabled && l.opacity != 0) {
            if let Some(generator) = layer.generator {
                if let Some(context) = &self.context {
                    if let Some(table) = lookup.take() {
                        passes.push(MaskPass::Lookup(table));
                    }
                    passes.push(MaskPass::GeneratorLayer(*layer, context.map(generator)));
                }
            } else {
                for value in lookup.get_or_insert_with(identity).iter_mut() {
                    *value = layer.apply(*value as u8, layer.value as f32 / 100.) as u32;
                }
            }
        }
        for &filter in self.filters.iter().filter(|f| f.enabled) {
            if filter.kind.spatial() {
                if let Some(table) = lookup.take() {
                    passes.push(MaskPass::Lookup(table));
                }
                passes.extend(
                    Spatial::passes(filter, size)
                        .into_iter()
                        .map(MaskPass::Spatial),
                );
                if filter.kind == FilterKind::Outline
                    && (filter.outline.curve != 0. || filter.outline.inverted)
                {
                    let exponent = (filter.outline.curve / 100.).exp2();
                    for value in lookup.get_or_insert_with(identity).iter_mut() {
                        let shaped = ((*value as f32 / 255.).powf(exponent) * 255.).round() as u32;
                        *value = if filter.outline.inverted {
                            255 - shaped
                        } else {
                            shaped
                        };
                    }
                }
            } else {
                for value in lookup.get_or_insert_with(identity).iter_mut() {
                    *value = filter.apply(*value as u8) as u32;
                }
            }
        }
        if let Some(table) = lookup {
            passes.push(MaskPass::Lookup(table));
        }
        if let Some(coverage) = &self.geometry_coverage {
            passes.push(MaskPass::GeneratorLayer(
                MaskLayer {
                    blend: MaskBlend::Multiply,
                    ..MaskLayer::default()
                },
                GeneratorMap { values: coverage.clone(), coverage: None },
            ));
        }
        passes
    }

    pub fn effective(&self) -> &[u8] {
        if !self.has_operations() {
            return &self.pixels;
        }
        self.effective.get_or_init(|| {
            let size = (self.pixels.len() / 4).isqrt();
            let mut gray: Vec<_> = self.pixels.chunks_exact(4).map(|p| p[0]).collect();
            let mut scratch = Vec::new();
            for pass in self.passes(size as u32) {
                match pass {
                    MaskPass::Lookup(table) => {
                        for value in &mut gray {
                            *value = table[*value as usize] as u8;
                        }
                    }
                    MaskPass::Spatial(pass) => {
                        scratch.resize(gray.len(), 0);
                        pass.apply(&gray, &mut scratch, size);
                        std::mem::swap(&mut gray, &mut scratch);
                    }
                    MaskPass::Affine(map) => {
                        for (i, (value, pixel)) in
                            gray.iter_mut().zip(self.pixels.chunks_exact(4)).enumerate()
                        {
                            let multiplier = map.as_ref().map_or(255, |map| map[i]);
                            let base = (u32::from(pixel[0]) * u32::from(multiplier) + 127) / 255;
                            *value = (base + u32::from(pixel[1])).min(255) as u8;
                        }
                    }
                    MaskPass::GeneratorLayer(layer, map) => {
                        for (i, (value, &sample)) in gray.iter_mut().zip(map.values.iter()).enumerate() {
                            let coverage = map.coverage.as_ref().map_or(255, |coverage| coverage[i]);
                            *value = layer.apply_covered(*value, sample as f32 / 255., coverage);
                        }
                    }
                }
            }
            Arc::new(gray.into_iter().flat_map(|v| [v, v, v, 255]).collect())
        })
    }
}
