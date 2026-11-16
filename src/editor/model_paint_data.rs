//! Editable texture sets. Brush coverage/straight-alpha over follow Material Painter's
//! stamp.wgsl and apply.wgsl; adapted for the editor's Bevy assets and worker pool.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(super) const CHANNEL_COUNT: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum LayerKind {
    Paint,
    Fill,
    Folder,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layer {
    pub name: String,
    pub kind: LayerKind,
    pub visible: bool,
    pub opacity: f32,
    pub channels: [Option<Arc<Vec<u8>>>; CHANNEL_COUNT],
}
impl Layer {
    pub fn new(name: String, kind: LayerKind) -> Self {
        Self {
            name,
            kind,
            visible: true,
            opacity: 100.,
            channels: Default::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stack {
    pub layers: Vec<Layer>,
    pub selected: usize,
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
}
pub(super) fn solid(size: u32, rgba: [u8; 4]) -> Arc<Vec<u8>> {
    Arc::new(rgba.repeat(size as usize * size as usize))
}
#[cfg(test)] pub(super) fn coverage(distance: f32, hardness: f32) -> f32 {
    if distance >= 1. {
        return 0.;
    }
    let t = ((distance - hardness) / (1. - hardness).max(1.0e-5)).clamp(0., 1.);
    1. - t * t * (3. - 2. * t)
}
#[cfg(test)] pub(super) fn over(base: [u8; 4], color: [u8; 3], amount: f32, erase: bool) -> [u8; 4] {
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
    pub fn same(&self, other: &Self) -> bool {
        self.size == other.size
            && self.material == other.material
            && self
                .base
                .iter()
                .zip(&other.base)
                .all(|(a, b)| Arc::ptr_eq(a, b))
            && same_stack(&self.stack, &other.stack)
    }
    pub fn composite_rect(&self, channel: usize, rect: bevy::prelude::URect) -> Vec<u8> {
        let width = rect.width() as usize;
        let mut out = Vec::with_capacity(width * rect.height() as usize * 4);
        let size = self.size as usize;
        for y in rect.min.y..rect.max.y {
            let start = (y as usize * size + rect.min.x as usize) * 4;
            out.extend_from_slice(&self.base[channel][start..start + width * 4]);
        }
        for layer in &self.stack.layers {
            if !layer.visible || layer.kind == LayerKind::Folder {
                continue;
            }
            let Some(pixels) = &layer.channels[channel] else {
                continue;
            };
            let opacity = layer.opacity / 25500.;
            for (row, y) in (rect.min.y..rect.max.y).enumerate() {
                let start = (y as usize * size + rect.min.x as usize) * 4;
                for (dst, src) in out[row * width * 4..(row + 1) * width * 4]
                    .chunks_exact_mut(4)
                    .zip(pixels[start..start + width * 4].chunks_exact(4))
                {
                    let a = src[3] as f32 * opacity;
                    if a == 0. {
                        continue;
                    }
                    for c in 0..3 {
                        dst[c] = (dst[c] as f32 * (1. - a) + src[c] as f32 * a).round() as u8;
                    }
                }
            }
        }
        out
    }
    pub fn composite(&self, channel: usize) -> Vec<u8> {
        let mut out = self.base[channel].as_ref().clone();
        for layer in &self.stack.layers {
            if !layer.visible || layer.kind == LayerKind::Folder {
                continue;
            }
            let Some(pixels) = &layer.channels[channel] else {
                continue;
            };
            for (dst, src) in out.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                let a = src[3] as f32 / 255. * layer.opacity / 100.;
                for c in 0..3 {
                    dst[c] = (dst[c] as f32 * (1. - a) + src[c] as f32 * a).round() as u8;
                }
            }
        }
        out
    }
    pub fn composites(&self) -> [Vec<u8>; CHANNEL_COUNT] {
        std::array::from_fn(|c| self.composite(c))
    }
    pub fn has_channel(&self, c: usize) -> bool {
        self.stack.layers.iter().any(|l| l.channels[c].is_some())
    }
    pub fn normal_map(&self, maps: &[Vec<u8>; CHANNEL_COUNT]) -> Vec<u8> {
        let mut normal = maps[6].clone();
        if !self.has_channel(3) {
            return normal;
        }
        let n = self.size as usize;
        for y in 0..n {
            for x in 0..n {
                let height = |x: usize, y: usize| maps[3][(y * n + x) * 4] as f32 / 255.;
                let dx = (height((x + 1).min(n - 1), y) - height(x.saturating_sub(1), y)) * 4.;
                let dy = (height(x, (y + 1).min(n - 1)) - height(x, y.saturating_sub(1))) * 4.;
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
        && a.layers.len() == b.layers.len()
        && a.layers.iter().zip(&b.layers).all(|(a, b)| {
            a.name == b.name
                && a.kind == b.kind
                && a.visible == b.visible
                && a.opacity == b.opacity
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
