//! Portable approximations of Painter's grayscale height sources.
use super::{Bitmap, byte};
use bevy::prelude::Vec2;
use image::{GrayImage, Luma, Rgba, RgbaImage};
use std::f32::consts::TAU;
use subcraft_spp::document::GraphRef;

#[derive(Clone, Copy)]
pub(super) enum Pattern {
    Checker,
    Gaussian,
    Scratches,
}

impl Pattern {
    pub(super) fn of(graph: &GraphRef) -> Option<Self> {
        match graph.name().to_ascii_lowercase().as_str() {
            "gradient_checker" => Some(Self::Checker),
            "noise_gaussian_noise" => Some(Self::Gaussian),
            "noise_directional_scratches" => Some(Self::Scratches),
            _ => None,
        }
    }

    pub(super) fn size(self, graph: &GraphRef) -> u32 {
        let requested = 1 << scalar(graph, "$outputsize", 8.).round().clamp(7., 10.) as u32;
        if matches!(self, Self::Gaussian) {
            requested.max(gaussian_scale(graph) as u32 * 4)
        } else {
            requested
        }
    }

    pub(super) fn bitmap(self, graph: &GraphRef, size: u32) -> Result<Bitmap, String> {
        let seed = scalar(graph, "$randomseed", 0.) as i32 as u32;
        let checker = matches!(self, Self::Checker);
        let balance = unit(graph, if checker { "Position" } else { "balance" }, 0.5);
        let contrast = unit(graph, "contrast", 0.);
        let invert = unit(graph, "invert", 0.) >= 0.5;
        let levels = |gray: f32| {
            let width = (1. - contrast).max(1. / 255.);
            let v = if balance == 0. || balance == 1. {
                balance
            } else {
                ((gray + balance - 1.) / width + 0.5).clamp(0., 1.)
            };
            let v = byte(if invert { 1. - v } else { v });
            Rgba([v, v, v, 255])
        };
        let image = match self {
            Self::Checker => {
                let tiles = scalar(graph, "tile_x", 2.).round().clamp(1., 16.);
                sample_image(size, |uv| {
                    let cell = uv * tiles;
                    let phase = cell.fract();
                    let parity = (cell.x.floor() as i32 + cell.y.floor() as i32) % 2;
                    levels(if parity == 0 { phase.x } else { phase.y })
                })
            }
            Self::Gaussian => {
                let scale = gaussian_scale(graph);
                let phase = unit(graph, "disorder", 0.) * 7.;
                sample_image(size, |uv| levels(gaussian(uv, scale, phase, seed)))
            }
            Self::Scratches => {
                let gray = scratches(graph, size, seed);
                RgbaImage::from_fn(size, size, |x, y| levels(f32::from(gray[(x, y)][0]) / 255.))
            }
        };
        Bitmap::from_rgba(format!("{} (SPP).png", graph.name()), image)
    }
}

fn scalar(graph: &GraphRef, name: &str, default: f32) -> f32 {
    graph
        .value(name)
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

fn unit(graph: &GraphRef, name: &str, default: f32) -> f32 {
    scalar(graph, name, default).clamp(0., 1.)
}

fn vector(graph: &GraphRef, name: &str, default: Vec2) -> Vec2 {
    let Some((_, values)) = graph
        .tweaks
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
    else {
        return default;
    };
    match values.as_slice() {
        [x, y] if x.is_finite() && y.is_finite() => Vec2::new(*x, *y),
        _ => default,
    }
}

fn sample_image(size: u32, sample: impl Fn(Vec2) -> Rgba<u8>) -> RgbaImage {
    RgbaImage::from_fn(size, size, |x, y| {
        sample((Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / size as f32)
    })
}

fn hash(mut n: u32) -> u32 {
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846c_a68b);
    n ^ (n >> 16)
}

fn gaussian_scale(graph: &GraphRef) -> i32 {
    // Older Painter projects use values above 64.
    scalar(graph, "scale", 32.).round().clamp(1., 256.) as i32
}

fn gaussian(uv: Vec2, scale: i32, phase: f32, seed: u32) -> f32 {
    let p = uv * scale as f32 + Vec2::new(phase, -phase);
    let cell = p.floor().as_ivec2();
    let t = p - p.floor();
    let t = t * t * (Vec2::splat(3.) - 2. * t);
    let at = |x, y| {
        let x = (cell.x + x as i32).rem_euclid(scale) as u32;
        let y = (cell.y + y as i32).rem_euclid(scale) as u32;
        let samples = hash(seed ^ (x + y * scale as u32)).to_le_bytes();
        samples.into_iter().map(f32::from).sum::<f32>() / 1020.
    };
    let lerp = |a, b, weight| a + (b - a) * weight;
    lerp(
        lerp(at(0, 0), at(1, 0), t.x),
        lerp(at(0, 1), at(1, 1), t.x),
        t.y,
    )
}

fn scratches(graph: &GraphRef, size: u32, seed: u32) -> GrayImage {
    let mut image = GrayImage::new(size, size);
    let scale = scalar(graph, "scale", 4.).round().clamp(1., 16.);
    let angle = unit(graph, "angle", 0.);
    let jitter = unit(graph, "angle_random", 0.);
    let disorder = unit(graph, "disorder", 0.);
    let amount = unit(graph, "pattern_amount", 1.);
    let extent =
        vector(graph, "pattern_size", Vec2::new(0.5, 20.)).clamp(Vec2::ZERO, Vec2::splat(40.));
    if extent.min_element() == 0. {
        return image;
    }
    let variation = vector(graph, "pattern_size_random", Vec2::ZERO).clamp(Vec2::ZERO, Vec2::ONE);
    let size_i = size as i32;
    let max_radii = (extent * size as f32 / (128. * scale.sqrt())).max(Vec2::splat(0.5));
    let max_box = (2. * max_radii.element_sum() + 4.).ceil() as u32;
    let count = ((scale * 1_600. * amount).round() as u32).min(24_000_000 / max_box.pow(2));
    for i in 0..count {
        let rand = |stream: u32| {
            (hash(seed ^ i.wrapping_mul(13).wrapping_add(stream)) >> 8) as f32 / 16_777_216.
        };
        let angle = (angle + (rand(2) - 0.5) * jitter) * TAU;
        let direction = Vec2::from_angle(angle);
        let cross = direction.perp();
        let center =
            (Vec2::new(rand(0), rand(1)) + Vec2::new(rand(3), rand(4)) * disorder) * size as f32;
        let random_size = Vec2::ONE - variation * Vec2::new(rand(5), rand(6));
        let radii = (max_radii * random_size).max(Vec2::splat(0.5));
        let bounds = cross.abs() * radii.x + direction.abs() * radii.y + Vec2::ONE;
        let low = (center - bounds).floor().as_ivec2();
        let high = (center + bounds).ceil().as_ivec2();
        let strength = 0.35 + 0.65 * rand(7);
        for y in low.y..high.y {
            for x in low.x..high.x {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - center;
                let local = Vec2::new(p.dot(cross), p.dot(direction)) / radii;
                let coverage = (1. - local.length()).max(0.);
                let pixel =
                    image.get_pixel_mut(x.rem_euclid(size_i) as u32, y.rem_euclid(size_i) as u32);
                *pixel = Luma([pixel[0].max(byte(coverage * strength))]);
            }
        }
    }
    image
}
