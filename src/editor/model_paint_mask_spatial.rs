//! CPU spatial mask processing for layer edits and saved material textures.
use super::{Filter, FilterKind};
use bevy::prelude::{IVec2, Vec2, Vec3};

pub(in crate::editor) struct Spatial {
    pub kind: FilterKind,
    pub vertical: bool,
    pub params: [f32; 4],
    pub weights: Vec<f32>,
}

impl Spatial {
    pub fn reach(&self, size: u32) -> u32 {
        match self.kind {
            // A changed texel can affect the opposite edge of a repeated mask.
            FilterKind::Blur if self.params[1] != 0. => size,
            FilterKind::Blur | FilterKind::Outline => self.params[0] as u32,
            FilterKind::DirectionalBlur => {
                let [samples, x, y, _] = self.params;
                (samples * x.abs().max(y.abs())).ceil() as u32 + 1
            }
            FilterKind::Pixelate => size.div_ceil(self.params[0] as u32),
            FilterKind::Sharpen => self.params[1] as u32,
            FilterKind::Warp => (self.params[0] * size as f32 * 0.5).ceil() as u32 + 1,
            _ => 0,
        }
    }

    pub(super) fn passes(filter: Filter, size: u32) -> Vec<Self> {
        let [a, b, c] = filter.values;
        let scale = size as f32 / 2048.;
        let pass = |vertical, params, weights| Self {
            kind: filter.kind,
            vertical,
            params,
            weights,
        };
        match filter.kind {
            FilterKind::Blur if a > 0. => {
                let radius = a * scale;
                let sigma = (radius * 0.5).max(0.5);
                let n = (radius.ceil() as i32).min(64);
                let mut weights: Vec<_> = (-n..=n)
                    .map(|k| (-(k * k) as f32 / (2. * sigma * sigma)).exp())
                    .collect();
                let sum: f32 = weights.iter().sum();
                for weight in &mut weights {
                    *weight /= sum;
                }
                let params = [n as f32, f32::from(u8::from(filter.wrap)), 0., 0.];
                vec![
                    pass(false, params, weights.clone()),
                    pass(true, params, weights),
                ]
            }
            FilterKind::DirectionalBlur if a > 0. => {
                // The Painter preset is authored at 256 px. Keep the width relative to the mask.
                let radius = a * (size as f32 / 256.);
                let samples = (radius.ceil() as i32).clamp(1, 64);
                let step = Vec2::from_angle(b.to_radians()) * (radius / samples as f32);
                vec![pass(false, [samples as f32, step.x, step.y, 0.], Vec::new())]
            }
            FilterKind::Pixelate if a >= 1. && a < size as f32 => {
                vec![pass(false, [a, 0., 0., 0.], Vec::new())]
            }
            FilterKind::Sharpen if a > 0. => vec![pass(
                false,
                [a, (b * scale).round().max(1.), 0., 0.],
                Vec::new(),
            )],
            FilterKind::Warp if a > 0. => {
                vec![pass(false, [a / 100. * 0.2, b, c * 1.37, 0.], Vec::new())]
            }
            FilterKind::Noise if a > 0. => {
                vec![pass(false, [a / 100., b, c * 1.37, 0.], Vec::new())]
            }
            FilterKind::Outline => {
                let radius = (b * size as f32 / 100.).round() as u32;
                let threshold = (a * 255. / 100.).ceil().max(1.);
                let first = radius.min(1);
                let mut passes = vec![pass(false, [first as f32, threshold, c, 0.], Vec::new())];
                for (vertical, mut covered) in [(false, first), (true, 0)] {
                    // Three overlapping intervals grow the neighborhood without scanning every texel.
                    while covered < radius {
                        let offset = (radius - covered).min(2 * covered + 1);
                        covered += offset;
                        let stage = if vertical && covered == radius {
                            2.
                        } else {
                            1.
                        };
                        passes.push(pass(
                            vertical,
                            [offset as f32, threshold, c, stage],
                            Vec::new(),
                        ));
                    }
                }
                if radius == 0 {
                    passes.push(pass(true, [0., threshold, c, 2.], Vec::new()));
                }
                let mut blur = Filter::new(FilterKind::Blur);
                blur.set_value(0, filter.outline.blur * 2048. / 100.);
                passes.extend(Self::passes(blur, size));
                passes
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn apply(&self, input: &[u8], output: &mut [u8], size: usize) {
        let [a, b, c, stage] = self.params;
        let wrap = self.kind == FilterKind::Blur && b != 0.;
        let raw_at = |p: IVec2| {
            let size = size as i32;
            let p = if wrap {
                IVec2::new(p.x.rem_euclid(size), p.y.rem_euclid(size))
            } else {
                p.clamp(IVec2::ZERO, IVec2::splat(size - 1))
            };
            input[p.y as usize * size as usize + p.x as usize]
        };
        let at = |p| f32::from(raw_at(p)) / 255.;
        let bilinear = |pos: Vec2| {
            let lo = pos.floor().as_ivec2();
            let t = pos - lo.as_vec2();
            mix(
                mix(at(lo), at(lo + IVec2::X), t.x),
                mix(at(lo + IVec2::Y), at(lo + IVec2::ONE), t.x),
                t.y,
            )
        };
        let dir = if self.vertical { IVec2::Y } else { IVec2::X };
        for (i, value) in output.iter_mut().enumerate() {
            let p = IVec2::new((i % size) as i32, (i / size) as i32);
            let s = input[i] as f32 / 255.;
            let q = (p.as_vec2() + Vec2::splat(0.5)) * (b / size as f32);
            let v = match self.kind {
                FilterKind::Blur => self
                    .weights
                    .iter()
                    .enumerate()
                    .map(|(k, weight)| at(p + dir * (k as i32 - a as i32)) * weight)
                    .sum(),
                FilterKind::DirectionalBlur => {
                    let step = Vec2::new(b, c);
                    let samples = a as i32;
                    let sum: f32 = (-samples..=samples)
                        .map(|k| bilinear(p.as_vec2() + step * k as f32))
                        .sum();
                    sum / (2 * samples + 1) as f32
                }
                FilterKind::Pixelate => {
                    let cells = a as i32;
                    let size = size as i32;
                    let cell = ((p * 2 + IVec2::ONE) * cells) / (2 * size);
                    at(((cell * 2 + IVec2::ONE) * size) / (2 * cells))
                }
                FilterKind::Sharpen => {
                    let x = IVec2::X * b as i32;
                    let y = IVec2::Y * b as i32;
                    let ring = (at(p + x) + at(p - x) + at(p + y) + at(p - y)) * 0.25;
                    s + a * (s - ring)
                }
                FilterKind::Warp => {
                    let offset = Vec2::new(
                        fbm(q.extend(1.3 + c)),
                        fbm((q + Vec2::new(5.2, 1.7)).extend(7.1 + c)),
                    );
                    let pos = p.as_vec2() + (offset - Vec2::splat(0.5)) * (a * size as f32);
                    bilinear(pos)
                }
                FilterKind::Noise => {
                    let t = ((fbm(q.extend(c)) - 0.35) / (0.65 - 0.35)).clamp(0., 1.);
                    s * (1. - a * (1. - smooth(t)))
                }
                FilterKind::Outline => {
                    let radius = a as i32;
                    let mut neighborhood = 0;
                    for k in [-radius, 0, radius] {
                        let sample = raw_at(p + dir * k);
                        neighborhood |= if stage != 0. {
                            sample & 3
                        } else if sample >= b as u8 {
                            1
                        } else {
                            2
                        };
                        if neighborhood == 3 {
                            break;
                        }
                    }
                    let inside = if stage == 0. {
                        input[i] >= b as u8
                    } else {
                        input[i] & 4 != 0
                    };
                    if stage == 2. {
                        let on_outline = neighborhood == 3
                            && match c as u32 {
                                1 => !inside,
                                2 => inside,
                                _ => true,
                            };
                        *value = if on_outline { 255 } else { 0 };
                    } else {
                        // Store inside/outside presence in bits 0/1 and the center in bit 2.
                        *value = neighborhood | (u8::from(inside) << 2);
                    }
                    continue;
                }
                _ => unreachable!(),
            };
            *value = (v.clamp(0., 1.) * 255.).round() as u8;
        }
    }
}

#[cfg(test)]
#[test]
fn directional_blur_spreads_an_impulse_only_along_its_angle() {
    let size = 256;
    let mut input = vec![0; size * size];
    let center = 100 * size + 100;
    input[center] = 255;
    let mut output = input.clone();
    let mut filter = Filter::new(FilterKind::DirectionalBlur);
    filter.set_value(0, 4.);
    for (angle, axis, other) in [(0., 1, size), (90., size, 1)] {
        filter.set_value(1, angle);
        let passes = Spatial::passes(filter, size as u32);
        assert_eq!(passes.len(), 1);
        passes[0].apply(&input, &mut output, size);
        assert_eq!(output[center], 28);
        assert_eq!(output[center - 4 * axis], 28);
        assert_eq!(output[center + 4 * axis], 28);
        assert_eq!(output[center + 5 * axis], 0);
        assert_eq!(output[center + other], 0);
    }
    filter.set_value(0, 0.);
    assert!(Spatial::passes(filter, size as u32).is_empty());
}

#[cfg(test)]
#[test]
fn pixelate_reads_the_center_of_each_cell_on_a_nondivisible_map() {
    let size = 8;
    let input: Vec<_> = (0..64).collect();
    let mut output = vec![0; size * size];
    let mut filter = Filter::new(FilterKind::Pixelate);
    filter.set_value(0, 3.);
    let passes = Spatial::passes(filter, size as u32);
    passes[0].apply(&input, &mut output, size);
    assert_eq!(&output[..8], &[9, 9, 9, 12, 12, 14, 14, 14]);
    assert_eq!(&output[7 * size..], &[49, 49, 49, 52, 52, 54, 54, 54]);
    assert_eq!(passes[0].reach(size as u32), 3);
    filter.set_value(0, 8.);
    assert!(Spatial::passes(filter, size as u32).is_empty());
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1. - t) + b * t
}

pub(super) fn smooth(t: f32) -> f32 {
    t * t * (3. - 2. * t)
}

fn hash(p: Vec3) -> f32 {
    let [mut x, mut y, mut z] = p.to_array().map(|v| {
        (v as i32 as u32)
            .wrapping_mul(1664525)
            .wrapping_add(1013904223)
    });
    x = x.wrapping_add(y.wrapping_mul(z));
    y = y.wrapping_add(z.wrapping_mul(x));
    z = z.wrapping_add(x.wrapping_mul(y));
    x ^= x >> 16;
    y ^= y >> 16;
    z ^= z >> 16;
    x.wrapping_add(y.wrapping_mul(z)) as f32 * (1. / 4294967295.)
}

pub(super) fn noise(p: Vec3) -> f32 {
    let origin = p.floor();
    let [x, y, z] = (p - origin).to_array().map(smooth);
    let at = |x, y, z| hash(origin + Vec3::new(x, y, z));
    let plane = |z| {
        mix(
            mix(at(0., 0., z), at(1., 0., z), x),
            mix(at(0., 1., z), at(1., 1., z), x),
            y,
        )
    };
    mix(plane(0.), plane(1.), z)
}

pub(in crate::editor) fn fbm(mut p: Vec3) -> f32 {
    let mut sum = 0.;
    let mut amplitude = 0.5;
    for _ in 0..5 {
        sum += amplitude * noise(p);
        p = p * 2.03 + Vec3::new(1.7, 9.2, 3.1);
        amplitude *= 0.5;
    }
    sum / 0.96875
}
