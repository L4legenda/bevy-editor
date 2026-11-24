//! CPU reconstruction of Painter's stock 3D Perlin Noise stencil.
use bevy::prelude::*;
use image::{GrayImage, Luma};
use std::sync::Arc;

#[derive(Default)]
pub(in crate::editor) struct StencilCache {
    perlin: Option<(Perlin, Arc<GrayImage>)>,
}

impl StencilCache {
    pub(super) fn perlin(&mut self, tweaks: &[(String, Vec<f32>)]) -> Arc<GrayImage> {
        let settings = Perlin::from_tweaks(tweaks);
        if let Some((previous, image)) = &self.perlin
            && *previous == settings
        {
            return image.clone();
        }
        let image = Arc::new(settings.render());
        self.perlin = Some((settings, image.clone()));
        image
    }
}

#[derive(PartialEq)]
struct Perlin {
    resolution: [u32; 2],
    frequency: Vec3,
    offset: Vec3,
    position: f32,
    seed: u32,
    baseline: f32,
    contrast: f32,
    distortion: f32,
    distortion_scale: f32,
    absolute: bool,
    invert: bool,
    tile: bool,
}

impl Perlin {
    fn from_tweaks(tweaks: &[(String, Vec<f32>)]) -> Self {
        let values = |name: &str| {
            tweaks
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, values)| values.as_slice())
        };
        let component = |name: &str, index: usize, default: f32| {
            values(name)
                .and_then(|values| values.get(index).copied())
                .filter(|value| value.is_finite())
                .unwrap_or(default)
        };
        let value = |name, default| component(name, 0, default);
        let vector =
            |name, default| Vec3::from_array(std::array::from_fn(|i| component(name, i, default)));
        let frequency = vector("size", 1.).clamp(Vec3::splat(0.01), Vec3::splat(16.))
            * value("scale", 32.).clamp(1., 256.);
        Self {
            resolution: std::array::from_fn(|i| {
                1 << component("$outputsize", i, 8.).clamp(6., 9.) as u32
            }),
            frequency: frequency.clamp(Vec3::ONE, Vec3::splat(256.)),
            offset: vector("offset", 0.).clamp(Vec3::splat(-64.), Vec3::splat(64.)),
            position: value("Position", 0.5).clamp(0., 1.),
            seed: value("$randomseed", 0.) as u32,
            baseline: value("baseline", 0.5).clamp(0., 1.),
            contrast: value("Contrast", 0.).clamp(0., 1.),
            distortion: value("distortion_intensity", 0.).clamp(0., 1.),
            distortion_scale: value("distortion_scale_multiplier", 1.).clamp(0.125, 8.),
            absolute: value("absolute", 0.) >= 0.5,
            invert: value("invert", 0.) >= 0.5,
            tile: value("enable_tiling", 0.) >= 0.5,
        }
    }

    fn render(&self) -> GrayImage {
        let [width, height] = self.resolution;
        let prepare_frequency = |frequency: Vec3| {
            if self.tile {
                frequency.round().max(Vec3::ONE)
            } else {
                frequency
            }
        };
        let frequency = prepare_frequency(self.frequency);
        let distortion_frequency = prepare_frequency(frequency * self.distortion_scale);
        GrayImage::from_fn(width, height, |x, y| {
            let uv = Vec3::new(
                (x as f32 + 0.5) / width as f32,
                1. - (y as f32 + 0.5) / height as f32,
                self.position,
            ) + self.offset;
            let mut point = uv * frequency;
            if self.distortion > 0. {
                let sample = |seed| {
                    gradient_noise(
                        uv * distortion_frequency,
                        self.tile.then_some(distortion_frequency),
                        seed,
                    )
                };
                point.x += sample(self.seed.wrapping_add(31)) * self.distortion * 4.;
                point.y += sample(self.seed.wrapping_add(79)) * self.distortion * 4.;
            }
            let noise = gradient_noise(point, self.tile.then_some(frequency), self.seed);
            let noise = if self.absolute { noise.abs() } else { noise };
            let gray = ((self.baseline + noise * 0.7 - 0.5) * (1. + 4. * self.contrast) + 0.5)
                .clamp(0., 1.);
            let gray = if self.invert { 1. - gray } else { gray };
            Luma([(gray * 255.).round() as u8])
        })
    }
}

fn gradient_noise(point: Vec3, period: Option<Vec3>, seed: u32) -> f32 {
    use super::stamp_hash;
    let origin = point.floor();
    let fraction = point - origin;
    let [x, y, z] = fraction
        .to_array()
        .map(|t| t * t * t * (t * (t * 6. - 15.) + 10.));
    let at = |corner: Vec3| {
        let p = origin + corner;
        let p = period.map_or(p, |period| p.rem_euclid(period)).as_ivec3();
        let hash = stamp_hash(
            stamp_hash(p.x as u32 ^ seed)
                ^ (p.y as u32).rotate_left(13)
                ^ (p.z as u32).rotate_left(25),
        );
        let offset = fraction - corner;
        let [u, v] = match (hash >> 2) % 3 {
            0 => [offset.x, offset.y],
            1 => [offset.y, offset.z],
            _ => [offset.z, offset.x],
        };
        (if hash & 1 == 0 { u } else { -u }) + (if hash & 2 == 0 { v } else { -v })
    };
    let row = |y, z| {
        Vec2::new(at(Vec3::new(0., y, z)), at(Vec3::new(1., y, z))).dot(Vec2::new(1. - x, x))
    };
    let plane = |z| Vec2::new(row(0., z), row(1., z)).dot(Vec2::new(1. - y, y));
    Vec2::new(plane(0.), plane(1.)).dot(Vec2::new(1. - z, z))
}

#[cfg(test)]
#[test]
fn perlin_slice_seed_and_tiling_are_preserved() {
    let mut cache = StencilCache::default();
    let defaults = cache.perlin(&[]);
    assert!(Arc::ptr_eq(&defaults, &cache.perlin(&[])));
    assert_ne!(defaults, cache.perlin(&[("$randomseed".into(), vec![1.])]));
    assert_ne!(defaults, cache.perlin(&[("Position".into(), vec![0.7])]));
    let point = Vec3::new(-0.4, 2.1, 1.3);
    let period = Vec3::splat(4.);
    let a = gradient_noise(point, Some(period), 42);
    let b = gradient_noise(point + Vec3::X * 4., Some(period), 42);
    assert!((a - b).abs() < 1e-6);
}
