//! Editable texture sets. Brush coverage/straight-alpha over follow Material Painter's
//! stamp.wgsl and apply.wgsl; adapted for the editor's Bevy assets and worker pool.
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[path = "model_paint_custom_maps.rs"]
pub(super) mod custom;
#[path = "model_paint_mask.rs"]
pub(super) mod mask;
#[path = "model_paint_map_fixes.rs"]
pub(super) mod map_fixes;
#[path = "model_paint_resize.rs"]
mod resize;

pub(super) const CHANNEL_COUNT: usize = 8;
pub(super) const MASK_TARGET: usize = CHANNEL_COUNT;
pub(super) const DEFAULT_SIZE: u32 = 1024;
pub(super) const RESOLUTIONS: [u32; 4] = [512, 1024, 2048, 4096];

// Material Painter expresses dielectric F0 as 0.08 * level.
pub(super) const SPECULAR_MAX_F0: f32 = 0.08;

pub(super) fn dielectric_f0(material: &serde_json::Value) -> f32 {
    let ior = material["extensions"]["KHR_materials_ior"]["ior"]
        .as_f64()
        .filter(|&ior| ior >= 1.)
        .unwrap_or(1.5) as f32;
    ((ior - 1.) / (ior + 1.)).powi(2)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum LayerKind {
    Paint,
    Fill,
    Folder,
}

#[path = "model_paint_texture_fill.rs"]
pub(super) mod texture_fill;
// Discriminants match model_paint_gpu_blend.wgsl.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::editor) enum Blend {
    #[default]
    Normal = 0,
    Multiply = 1,
    Screen = 2,
    Overlay = 3,
    Add = 4,
    Subtract = 5,
    Darken = 6,
    Lighten = 7,
    AddHeight = 10,
}
impl Blend {
    pub(super) const ALL: [Self; 9] = [
        Self::Normal,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Add,
        Self::Subtract,
        Self::Darken,
        Self::Lighten,
        Self::AddHeight,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Normal => "Нормальный",
            Self::Multiply => "Умножение",
            Self::Screen => "Экран",
            Self::Overlay => "Перекрытие",
            Self::Add => "Сложение",
            Self::Subtract => "Вычитание",
            Self::Darken => "Затемнение",
            Self::Lighten => "Осветление",
            Self::AddHeight => "Сложение высоты",
        }
    }

    // Normal vectors have their own composition and must not use RGB modes.
    pub(super) fn for_channel(self, channel: usize) -> Self {
        match (channel, self) {
            (6, _) => Self::Normal,
            (3, Self::AddHeight) => Self::AddHeight,
            (_, Self::AddHeight) => Self::Add,
            _ => self,
        }
    }

    fn mix_pixel<T: CompositeSample>(self, dst: &mut [T], src: &[u8], opacity: f32) {
        let coverage = src[3] as f32 / 255. * opacity / 100.;
        if coverage == 0. {
            return;
        }
        if self == Self::Normal {
            for c in 0..3 {
                dst[c] = T::interpolate(dst[c], T::from_byte(src[c]), coverage);
            }
            return;
        }
        for c in 0..3 {
            let base = dst[c].unit();
            let source = src[c] as f32 / 255.;
            let blended = match self {
                Self::Normal => source,
                Self::Multiply => base * source,
                Self::Screen => 1. - (1. - base) * (1. - source),
                Self::Overlay if base < 0.5 => 2. * base * source,
                Self::Overlay => 1. - 2. * (1. - base) * (1. - source),
                Self::Add => base + source,
                Self::AddHeight => base + source - 128. / 255.,
                Self::Subtract => base - source,
                Self::Darken => base.min(source),
                Self::Lighten => base.max(source),
            };
            dst[c] = T::from_unit(base * (1. - coverage) + blended * coverage);
        }
    }
}

pub(super) trait CompositeSample: Copy + PartialEq {
    fn from_byte(value: u8) -> Self;
    fn from_unit(value: f32) -> Self;
    fn unit(self) -> f32;
    fn interpolate(a: Self, b: Self, weight: f32) -> Self;
}

impl CompositeSample for u8 {
    fn from_byte(value: u8) -> Self {
        value
    }

    fn from_unit(value: f32) -> Self {
        (value * 255.).round() as u8
    }

    fn unit(self) -> f32 {
        self as f32 / 255.
    }

    fn interpolate(a: Self, b: Self, weight: f32) -> Self {
        (a as f32 * (1. - weight) + b as f32 * weight).round() as u8
    }
}

impl CompositeSample for f32 {
    fn from_byte(value: u8) -> Self {
        value as f32 / 255.
    }

    fn from_unit(value: f32) -> Self {
        value.clamp(0., 1.)
    }

    fn unit(self) -> f32 {
        self
    }

    fn interpolate(a: Self, b: Self, weight: f32) -> Self {
        a * (1. - weight) + b * weight
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layer {
    pub name: String,
    pub graph_id: Option<u64>,
    pub folder_graph: Option<Arc<serde_json::Value>>,
    pub folder_images: Arc<texture_fill::GraphImages>,
    pub kind: LayerKind,
    pub visible: bool,
    pub collapsed: bool,
    pub opacity: f32,
    pub blend: Blend,
    pub depth: usize,
    pub mask: Option<Arc<mask::Mask>>,
    pub channels: [Option<Arc<Vec<u8>>>; CHANNEL_COUNT],
    pub texture_fill: Option<Arc<texture_fill::Fill>>,
}
impl Layer {
    pub fn edit_texture_fill(&mut self, edit: impl FnOnce(&mut texture_fill::Fill)) {
        let fill = Arc::make_mut(self.texture_fill.get_or_insert_with(Arc::default));
        edit(fill);
        fill.invalidate();
        if fill.graph.is_none() && fill.images.iter().all(Option::is_none)
            && fill.settings.variation == texture_fill::Variation::default()
        {
            self.texture_fill = None;
        }
    }

    pub fn isolated(&self) -> bool {
        self.opacity < 100. || self.mask.is_some()
    }

    pub fn new(name: String, kind: LayerKind) -> Self {
        Self {
            name,
            graph_id: None,
            folder_graph: None,
            folder_images: Arc::default(),
            kind,
            visible: true,
            collapsed: false,
            opacity: 100.,
            blend: Blend::Normal,
            depth: 0,
            mask: None,
            channels: Default::default(),
            texture_fill: None,
        }
    }

    pub fn channel(&self, index: usize) -> Option<&Arc<Vec<u8>>> {
        self.texture_fill.as_ref().and_then(|fill| fill.sampled[index].as_ref())
            .or(self.channels[index].as_ref())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stack {
    pub layers: Vec<Layer>,
    pub selected: usize,
}
pub(super) const MAX_LAYER_DEPTH: usize = 8;
pub(super) const MAX_LAYERS: usize = 256;

#[derive(Clone, Copy)]
pub(super) enum CompositeOp {
    Save { slot: usize },
    Layer,
    Fade { slot: usize },
}

impl Stack {
    // Children are contiguous below their folder, matching compositing order.
    pub fn valid_hierarchy(&self) -> bool {
        let mut folders = 0;
        for layer in self.layers.iter().rev() {
            if layer.depth > folders || layer.depth > MAX_LAYER_DEPTH {
                return false;
            }
            folders = layer.depth + usize::from(layer.kind == LayerKind::Folder);
        }
        true
    }

    pub fn block(&self, index: usize) -> std::ops::Range<usize> {
        let layer = &self.layers[index];
        let start = if layer.kind == LayerKind::Folder {
            self.layers[..index]
                .iter()
                .rposition(|l| l.depth <= layer.depth)
                .map_or(0, |i| i + 1)
        } else {
            index
        };
        start..index + 1
    }

    pub fn parents(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let mut depth = self.layers[index].depth;
        (index + 1..self.layers.len()).filter(move |&i| {
            if self.layers[i].depth < depth {
                depth = self.layers[i].depth;
                true
            } else {
                false
            }
        })
    }

    pub fn shown(&self, index: usize) -> bool {
        self.layers.get(index).is_some_and(|layer| layer.visible)
            && self.parents(index).all(|i| self.layers[i].visible)
    }

    pub fn listed(&self, index: usize) -> bool {
        index < self.layers.len() && self.parents(index).all(|i| !self.layers[i].collapsed)
    }

    pub fn reveal_selection(&mut self) {
        if self.selected >= self.layers.len() {
            return;
        }
        let mut depth = self.layers[self.selected].depth;
        for layer in &mut self.layers[self.selected + 1..] {
            if layer.depth < depth {
                depth = layer.depth;
                layer.collapsed = false;
            }
        }
    }

    pub fn composite_ops(&self) -> Vec<(usize, CompositeOp)> {
        let mut opens = vec![Vec::new(); self.layers.len()];
        for (i, layer) in self.layers.iter().enumerate() {
            if layer.kind == LayerKind::Folder && layer.isolated() && self.shown(i) {
                let start = self.block(i).start;
                if start != i {
                    opens[start].push(i);
                }
            }
        }
        let mut waiting = Vec::new();
        let mut ops = Vec::new();
        for (i, layer) in self.layers.iter().enumerate() {
            for &folder in opens[i].iter().rev() {
                ops.push((
                    i,
                    CompositeOp::Save {
                        slot: waiting.len(),
                    },
                ));
                waiting.push(folder);
            }
            if waiting.last() == Some(&i) {
                waiting.pop();
                ops.push((
                    i,
                    CompositeOp::Fade {
                        slot: waiting.len(),
                    },
                ));
            } else if layer.kind != LayerKind::Folder && self.shown(i) {
                ops.push((i, CompositeOp::Layer));
            }
        }
        ops
    }

    pub fn move_block(&mut self, destination: usize, depth: usize) {
        let block = self.block(self.selected);
        let old_depth = self.layers[self.selected].depth;
        let index = if destination >= block.end {
            destination - block.len()
        } else {
            destination
        };
        let mut layers: Vec<_> = self.layers.drain(block).collect();
        for layer in &mut layers {
            layer.depth = layer.depth - old_depth + depth;
        }
        self.selected = index + layers.len() - 1;
        self.layers.splice(index..index, layers);
    }

    pub fn can_move_into(&self, folder: usize) -> bool {
        let Some(target) = self.layers.get(folder) else {
            return false;
        };
        let block = self.block(self.selected);
        target.kind == LayerKind::Folder
            && !block.contains(&folder)
            && self.parents(self.selected).next() != Some(folder)
            && self.layers[block].iter().all(|layer| {
                layer.depth - self.layers[self.selected].depth + target.depth < MAX_LAYER_DEPTH
            })
    }

    pub fn adjacent(&self, direction: i8) -> Option<usize> {
        let block = self.block(self.selected);
        let depth = self.layers[self.selected].depth;
        if direction > 0 {
            (block.end..self.layers.len())
                .find(|&i| self.layers[i].depth <= depth)
                .filter(|&i| self.layers[i].depth == depth)
                .map(|i| i + 1)
        } else {
            block
                .start
                .checked_sub(1)
                .filter(|&i| self.layers[i].depth == depth)
                .map(|i| self.block(i).start)
        }
    }
}
impl Default for Stack {
    fn default() -> Self {
        Self {
            layers: vec![Layer::new("Слой 1".into(), LayerKind::Paint)],
            selected: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Surface {
    pub size: u32,
    pub base: [Arc<Vec<u8>>; CHANNEL_COUNT],
    pub stack: Stack,
    pub material: serde_json::Value,
    pub baked: Option<Arc<mask::BakedInputs>>,
}
pub(super) fn solid(size: u32, rgba: [u8; 4]) -> Arc<Vec<u8>> {
    Arc::new(rgba.repeat(size as usize * size as usize))
}
#[cfg(test)]
pub(super) fn coverage(distance: f32, hardness: f32) -> f32 {
    if distance >= 1. {
        return 0.;
    }
    let t = ((distance - hardness) / (1. - hardness).max(1.0e-5)).clamp(0., 1.);
    1. - t * t * (3. - 2. * t)
}
pub(super) fn over(base: [u8; 4], color: [u8; 3], amount: f32, erase: bool) -> [u8; 4] {
    let a = base[3] as f32 / 255.;
    if erase {
        return [
            base[0],
            base[1],
            base[2],
            (255. * a * (1. - amount)).round() as u8,
        ];
    }
    let alpha = amount + a * (1. - amount);
    let mut out = [0; 4];
    if alpha > 1.0e-6 {
        for i in 0..3 {
            out[i] = ((color[i] as f32 * amount + base[i] as f32 * a * (1. - amount)) / alpha)
                .round() as u8;
        }
    }
    out[3] = (alpha * 255.).round() as u8;
    out
}
impl Surface {
    pub fn has_texture_fills(&self) -> bool {
        self.stack.layers.iter().any(|layer| layer.texture_fill.is_some())
    }

    pub fn texture_fills_need_binding(&self, mesh: &super::model_document::MeshData, slot: usize, offset: bevy::prelude::Vec2, transform: bevy::prelude::Transform) -> bool {
        self.stack.layers.iter().any(|layer| layer.texture_fill.as_ref()
            .is_some_and(|fill| !fill.matches(mesh, slot, self.size, offset, transform, &layer.channels)))
    }

    pub fn bind_texture_fills(&mut self, mesh: &super::model_document::MeshData, slot: usize, offset: bevy::prelude::Vec2, transform: bevy::prelude::Transform) {
        for layer in &mut self.stack.layers {
            if let Some(fill) = &mut layer.texture_fill
                && !fill.matches(mesh, slot, self.size, offset, transform, &layer.channels)
            {
                Arc::make_mut(fill).bind(mesh, slot, self.size, offset, transform, &layer.channels);
            }
        }
    }

    pub fn resize(&mut self, size: u32) {
        if self.size != size {
            resize::apply(self, size);
        }
    }

    pub fn needs_bake(&self) -> bool {
        self.stack.layers.iter().any(|layer| layer.mask.as_ref().is_some_and(|mask| mask.needs_bake()))
    }

    pub fn same_bake(&self, other: Option<&Surface>) -> bool {
        match (&self.baked, other.and_then(|surface| surface.baked.as_ref())) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    pub fn bind_generators(&mut self, mesh: &super::model_document::MeshData, slot: usize, uv_offset: bevy::prelude::Vec2, transform: bevy::prelude::Transform) -> bool {
        let matching = |mask: &mask::Mask| {
            mask.context
                .as_ref()
                .is_some_and(|c| c.matches(mesh, slot, self.size, uv_offset, transform, self.baked.as_ref()))
        };
        let masks = || {
            self.stack
                .layers
                .iter()
                .filter_map(|layer| layer.mask.as_ref())
        };
        if !masks().any(|mask| mask.uses_geometry() && !matching(mask)) {
            return false;
        }
        let context = masks()
            .find(|mask| matching(mask))
            .and_then(|mask| mask.context.clone())
            .unwrap_or_else(|| {
                Arc::new(mask::GeneratorSurface::new(mesh.clone(), slot, self.size, uv_offset, transform, self.baked.clone()))
            });
        for mask in self
            .stack
            .layers
            .iter_mut()
            .filter_map(|layer| layer.mask.as_mut())
        {
            if mask.uses_geometry() && !matching(mask) {
                Arc::make_mut(mask).bind(context.clone());
            }
        }
        true
    }

    pub fn same(&self, other: &Self) -> bool {
        self.size == other.size
            && self.material == other.material
            && (!self.needs_bake() || self.same_bake(Some(other)))
            && self
                .base
                .iter()
                .zip(&other.base)
                .all(|(a, b)| Arc::ptr_eq(a, b))
            && same_stack(&self.stack, &other.stack)
    }
    pub fn composite_rect(&self, channel: usize, rect: bevy::prelude::URect) -> Vec<u8> {
        self.composite_samples(channel, rect)
    }

    pub fn composite_samples<T: CompositeSample>(
        &self, channel: usize, rect: bevy::prelude::URect,
    ) -> Vec<T> {
        let width = rect.width() as usize;
        let mut out = Vec::with_capacity(width * rect.height() as usize * 4);
        let size = self.size as usize;
        for y in rect.min.y..rect.max.y {
            let start = (y as usize * size + rect.min.x as usize) * 4;
            out.extend(self.base[channel][start..start + width * 4].iter().copied().map(T::from_byte));
        }
        let mut saved = Vec::new();
        for (index, op) in self.stack.composite_ops() {
            let layer = &self.stack.layers[index];
            let mask = layer.mask.as_ref().map(|mask| mask.effective());
            let opacity =
                |byte| layer.opacity * mask.map_or(1., |pixels| pixels[byte] as f32 / 255.);
            match op {
                CompositeOp::Save { .. } => {
                    saved.push(out.clone());
                    continue;
                }
                CompositeOp::Fade { .. } => {
                    let before = saved.pop().unwrap();
                    for (row, y) in (rect.min.y..rect.max.y).enumerate() {
                        let start = row * width * 4;
                        for (x, (dst, src)) in out[start..start + width * 4]
                            .chunks_exact_mut(4)
                            .zip(before[start..start + width * 4].chunks_exact(4))
                            .enumerate()
                        {
                            let weight =
                                opacity((y as usize * size + rect.min.x as usize + x) * 4) / 100.;
                            for c in 0..3 {
                                dst[c] = T::interpolate(src[c], dst[c], weight);
                            }
                        }
                    }
                    continue;
                }
                CompositeOp::Layer => {}
            }
            let Some(pixels) = layer.channel(channel) else {
                continue;
            };
            let blend = layer.blend.for_channel(channel);
            for (row, y) in (rect.min.y..rect.max.y).enumerate() {
                let start = (y as usize * size + rect.min.x as usize) * 4;
                for (x, (dst, src)) in out[row * width * 4..(row + 1) * width * 4]
                    .chunks_exact_mut(4)
                    .zip(pixels[start..start + width * 4].chunks_exact(4))
                    .enumerate()
                {
                    blend.mix_pixel(dst, src, opacity(start + x * 4));
                }
            }
        }
        out
    }
    pub fn composite(&self, channel: usize) -> Vec<u8> {
        self.composite_rect(
            channel,
            bevy::prelude::URect {
                min: bevy::prelude::UVec2::ZERO,
                max: bevy::prelude::UVec2::splat(self.size),
            },
        )
    }
    pub fn composites(&self) -> [Vec<u8>; CHANNEL_COUNT] {
        std::array::from_fn(|c| self.composite(c))
    }
    pub fn has_channel(&self, c: usize) -> bool {
        self.stack.layers.iter().any(|l| l.channel(c).is_some())
    }
    pub fn normal_map(&self, maps: &[Vec<u8>; CHANNEL_COUNT]) -> Vec<u8> {
        self.normal_with_height(maps[6].clone(), &maps[3])
    }
    pub fn normal_with_height(&self, normal: Vec<u8>, heights: &[u8]) -> Vec<u8> {
        self.normal_with_height_scaled(normal, heights, 4.)
    }
    pub fn normal_with_height_scaled(&self, mut normal: Vec<u8>, heights: &[u8], scale: f32) -> Vec<u8> {
        if scale == 0. || !self.has_channel(3) {
            return normal;
        }
        let n = self.size as usize;
        for y in 0..n {
            for x in 0..n {
                let height = |x: usize, y: usize| heights[(y * n + x) * 4] as f32 / 255.;
                let dx = (height((x + 1).min(n - 1), y) - height(x.saturating_sub(1), y)) * scale;
                let dy = (height(x, (y + 1).min(n - 1)) - height(x, y.saturating_sub(1))) * scale;
                let i = (y * n + x) * 4;
                let v = bevy::prelude::Vec3::new(
                    normal[i] as f32 / 127.5 - 1. - dx,
                    normal[i + 1] as f32 / 127.5 - 1. + dy,
                    (normal[i + 2] as f32 / 127.5 - 1.).max(0.01),
                )
                .normalize();
                for c in 0..3 {
                    normal[i + c] = ((v[c] * 0.5 + 0.5) * 255.).round() as u8;
                }
            }
        }
        normal
    }
}
pub(super) fn same_stack(a: &Stack, b: &Stack) -> bool {
    a.selected == b.selected
        && compare_layers(a, b, true)
}

pub(super) fn same_composite(a: &Stack, b: &Stack) -> bool {
    compare_layers(a, b, false)
}

fn compare_layers(a: &Stack, b: &Stack, metadata: bool) -> bool {
    a.layers.len() == b.layers.len()
        && a.layers.iter().zip(&b.layers).all(|(a, b)| {
            (!metadata || (a.name == b.name && a.collapsed == b.collapsed
                && a.graph_id == b.graph_id && a.folder_graph == b.folder_graph
                && (Arc::ptr_eq(&a.folder_images, &b.folder_images) || a.folder_images == b.folder_images)))
                && a.kind == b.kind
                && a.visible == b.visible
                && a.opacity == b.opacity
                && a.blend == b.blend
                && a.depth == b.depth
                && a.texture_fill == b.texture_fill
                && match (&a.mask, &b.mask) {
                    (None, None) => true,
                    (Some(a), Some(b)) => {
                        Arc::ptr_eq(a, b)
                            || (Arc::ptr_eq(&a.pixels, &b.pixels) && a.same_settings(b))
                    }
                    _ => false,
                }
                && a.channels
                    .iter()
                    .zip(&b.channels)
                    .all(|(a, b)| match (a, b) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        _ => false,
                    })
        })
}
