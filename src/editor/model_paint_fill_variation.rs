//! Object-space noise for solid and textured fill layers.
use crate::editor::model_paint_data::mask::fbm;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub(in crate::editor) const CHANNELS: [usize; 3] = [0, 1, 3];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::editor) struct Variation {
    pub color: f32,
    pub roughness: f32,
    pub height: f32,
    pub scale: f32,
    pub seed: f32,
    pub parts: f32,
}

impl Default for Variation {
    fn default() -> Self {
        Self {
            color: 0.,
            roughness: 0.,
            height: 0.,
            scale: 4.,
            seed: 0.,
            parts: 0.,
        }
    }
}

impl Variation {
    pub(super) fn valid(self) -> bool {
        [self.color, self.roughness, self.height, self.parts]
            .iter()
            .all(|v| (0.0..=1.0).contains(v))
            && (0.5..=60.).contains(&self.scale)
            && (0.0..=100.).contains(&self.seed)
    }

    pub(super) fn enabled(self) -> [bool; 3] {
        [
            self.color != 0. || self.parts != 0.,
            self.roughness != 0. || self.parts != 0.,
            self.height != 0.,
        ]
    }

    pub(super) fn apply(self, channel: usize, pixel: &mut [u8], position: Vec3, part: f32) {
        let seed = Vec3::splat(self.seed * 13.7);
        let pack = |v: f32| v.clamp(0., 255.).round() as u8;
        let noise = |frequency: f32, phase: f32, amount: f32| {
            if amount == 0. {
                0.
            } else {
                (fbm(position * self.scale * frequency + seed + Vec3::splat(phase)) - 0.5) * amount
            }
        };
        if channel == 0 {
            let factor =
                (1. + noise(1., 0., self.color) * 2.) * (1. + (part - 0.5) * self.parts * 2.);
            for component in &mut pixel[..3] {
                *component = pack(f32::from(*component) * factor);
            }
        } else {
            let offset = if channel == 1 {
                noise(1.7, 5.2, self.roughness) + ((part * 7.31).fract() - 0.5) * self.parts * 0.6
            } else {
                // Bevy packs signed layer height around the midpoint of its byte channel.
                noise(3.1, 11.9, self.height) * 0.5
            };
            let value = pack(f32::from(pixel[0]) + offset * 255.);
            pixel[..3].fill(value);
        }
    }
}

pub(super) fn part_value(part: usize) -> f32 {
    let mut id = part as u32 ^ 1;
    id = (id ^ (id >> 16)).wrapping_mul(0x7feb352d);
    id = (id ^ (id >> 15)).wrapping_mul(0x846ca68b);
    (id ^ (id >> 16)) as u8 as f32 / 255.
}
