//! Repeating masks assembled from independently placed shapes.
use bevy::prelude::{IVec2, Vec2};
use serde::{Deserialize, Serialize};

pub(super) type Control = (&'static str, f32, f32, f32);

pub(super) const CONTROLS: [Control; 24] = [
    ("Столбцы", 4., 1., 64.),
    ("Ряды", 4., 1., 64.),
    ("Ширина, %", 100., 0., 100.),
    ("Высота, %", 100., 0., 100.),
    ("Масштаб, %", 85., 0., 200.),
    ("Форма (0 — квадрат, 1 — диск, 2 — кирпич)", 0., 0., 2.),
    ("Скос кирпича, %", 50., 0., 100.),
    ("Сдвиг рядов, %", 0., -100., 100.),
    ("Разброс сдвига, %", 0., 0., 100.),
    ("Сдвигать столбцы", 0., 0., 1.),
    ("Разброс масштаба, %", 0., 0., 100.),
    ("Пропуск плиток, %", 0., 0., 100.),
    ("Зерно", 0., 0., 65535.),
    ("Зерно масштаба", 0., 0., 1000.),
    ("Зерно сдвига", 0., 0., 1000.),
    ("Смещение X, %", 0., -100., 100.),
    ("Смещение Y, %", 0., -100., 100.),
    ("Угол, °", 0., -360., 360.),
    ("Непрозрачность, %", 100., 0., 100.),
    ("Фон, %", 0., 0., 100.),
    ("Шахматная раскладка", 0., 0., 1.),
    ("Пропуск столбцов", 0., 0., 1.),
    ("Пропуск рядов", 0., 0., 1.),
    ("Инверсия пропусков", 0., 0., 1.),
];

const FIRST_OPTION: usize = 5;

pub(super) const fn discrete(index: usize) -> bool {
    matches!(index, 0 | 1 | 5 | 9 | 12..=14 | 20..=23)
}

pub(super) const EDITOR_CONTROLS: [Control; CONTROLS.len()] = {
    let mut controls = [("", 0., 0., 0.); CONTROLS.len()];
    let mut index = 0;
    while index < controls.len() {
        let (label, _, min, max) = CONTROLS[index];
        controls[index] = (label, min, max, if discrete(index) { 1. } else { 5. });
        index += 1;
    }
    controls
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct TileOptions {
    options: [f32; CONTROLS.len() - FIRST_OPTION],
}

impl Default for TileOptions {
    fn default() -> Self {
        Self {
            options: std::array::from_fn(|index| CONTROLS[index + FIRST_OPTION].1),
        }
    }
}

impl TileOptions {
    pub(super) fn value(self, index: usize) -> f32 {
        self.options[index - FIRST_OPTION]
    }

    pub(super) fn set(&mut self, index: usize, value: f32) {
        self.options[index - FIRST_OPTION] = value;
    }
}

pub(super) struct Sampler {
    shape: u32,
    bevel: f32,
    count: IVec2,
    width: Vec2,
    offset: Vec2,
    stagger: f32,
    stagger_random: f32,
    stagger_columns: bool,
    scale_random: f32,
    dropout: f32,
    seed: u32,
    scale_seed: u32,
    stagger_seed: u32,
    inverse_rotation: Vec2,
    opacity: f32,
    background: f32,
    skip: [bool; 4],
    neighbors: i32,
}

impl Sampler {
    pub(super) fn new(values: [f32; 5], options: TileOptions) -> Self {
        let [columns, rows, width, height, scale] = values;
        let [
            shape,
            bevel,
            stagger,
            stagger_random,
            stagger_columns,
            scale_random,
            dropout,
            seed,
            scale_seed,
            stagger_seed,
            offset_x,
            offset_y,
            rotation,
            opacity,
            background,
            checker,
            horizontal,
            vertical,
            invert,
        ] = options.options;
        let width = Vec2::new(width, height) * (scale / 10_000.);
        let inverse_rotation = Vec2::from_angle(-rotation.to_radians());
        let bounds = Vec2::new(
            width.dot(inverse_rotation.abs()),
            width.dot(inverse_rotation.perp().abs()),
        );
        Self {
            shape: shape as u32,
            bevel: bevel / 100.,
            count: IVec2::new(columns as i32, rows as i32),
            width,
            offset: Vec2::new(offset_x, offset_y) / 100.,
            stagger: stagger / 100.,
            stagger_random: stagger_random / 100.,
            stagger_columns: stagger_columns != 0.,
            scale_random: scale_random / 100.,
            dropout: dropout / 100.,
            seed: seed as u32,
            scale_seed: scale_seed as u32,
            stagger_seed: stagger_seed as u32,
            inverse_rotation,
            opacity: opacity / 100.,
            background: background / 100.,
            skip: [
                checker != 0.,
                horizontal != 0.,
                vertical != 0.,
                invert != 0.,
            ],
            neighbors: i32::from(bounds.max_element() > 1.00001),
        }
    }

    pub(super) fn sample(&self, uv: Vec2) -> f32 {
        if self.width.min_element() <= 0. || self.opacity == 0. {
            return self.background;
        }
        let grid = (uv - self.offset) * self.count.as_vec2();
        let orient = |p: Vec2| {
            if self.stagger_columns {
                Vec2::new(p.y, p.x)
            } else {
                p
            }
        };
        let grid = orient(grid);
        let count = if self.stagger_columns {
            IVec2::new(self.count.y, self.count.x)
        } else {
            self.count
        };
        let row = grid.y.floor() as i32;
        let mut coverage = 0.0_f32;
        for iy in row - self.neighbors..=row + self.neighbors {
            let line = iy.rem_euclid(count.y);
            let jitter = if self.stagger_random > 0. {
                random(
                    0,
                    line,
                    self.seed.wrapping_add(self.stagger_seed).wrapping_add(31),
                ) * self.stagger_random
            } else {
                0.
            };
            let stagger = line as f32 * self.stagger + jitter;
            let x = grid.x - stagger;
            let column = x.floor() as i32;
            for ix in column - self.neighbors..=column + self.neighbors {
                let mut id = IVec2::new(ix.rem_euclid(count.x), line);
                if self.stagger_columns {
                    id = IVec2::new(id.y, id.x);
                }
                let [checker, horizontal, vertical, invert] = self.skip;
                let hidden = (checker && (id.x + id.y) % 2 != 0)
                    || (horizontal && id.x % 2 != 0)
                    || (vertical && id.y % 2 != 0)
                    || (self.dropout > 0. && random(id.x, id.y, self.seed) < self.dropout);
                if hidden != invert {
                    continue;
                }
                let scale = if self.scale_random > 0. {
                    1. - self.scale_random
                        * random(
                            id.x,
                            id.y,
                            self.seed.wrapping_add(self.scale_seed).wrapping_add(79),
                        )
                } else {
                    1.
                };
                let local = orient(Vec2::new(x - ix as f32 - 0.5, grid.y - iy as f32 - 0.5));
                let distance = self.inverse_rotation.rotate(local) * 2.
                    / (self.width * scale).max(Vec2::splat(1e-6));
                let radius = if self.shape == 1 {
                    distance.length()
                } else {
                    distance.abs().max_element()
                };
                let value = if self.shape == 2 {
                    super::smooth(((1. - radius) / self.bevel.max(1e-5)).clamp(0., 1.))
                } else {
                    f32::from(u8::from(radius <= 1.))
                };
                coverage = coverage.max(value);
            }
        }
        self.background + (1. - self.background) * coverage * self.opacity
    }
}

fn random(x: i32, y: i32, seed: u32) -> f32 {
    let mut hash =
        (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca6b) ^ seed;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    ((hash ^ (hash >> 16)) >> 8) as f32 / 16_777_216.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_pattern_staggers_rows_and_wraps_seeded_dropout() {
        let mut options = TileOptions::default();
        options.set(7, 50.);
        let sampler = Sampler::new([4., 2., 100., 100., 50.], options);
        assert_eq!(sampler.sample(Vec2::new(0.125, 0.25)), 1.);
        assert_eq!(sampler.sample(Vec2::new(0.125, 0.75)), 0.);
        assert_eq!(sampler.sample(Vec2::new(0.25, 0.75)), 1.);
        options.set(11, 50.);
        options.set(12, 41.);
        let sampler = Sampler::new([4., 2., 100., 100., 50.], options);
        let samples = (0..4)
            .map(|column| Vec2::new((column as f32 + 0.5) / 4., 0.25))
            .map(|uv| {
                let value = sampler.sample(uv);
                assert_eq!(value, sampler.sample(uv + Vec2::ONE));
                value
            })
            .collect::<Vec<_>>();
        assert!(
            samples.contains(&0.) && samples.contains(&1.),
            "{samples:?}"
        );
    }
}
