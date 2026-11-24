//! Spatial inputs for smart masks, following Material Painter's wear.wgsl.
use super::{fbm, smooth};
use bevy::prelude::{Vec2, Vec3};

pub(super) fn transition(low: f32, high: f32, value: f32) -> f32 {
    smooth(((value - low) / (high - low)).clamp(0., 1.))
}

pub(super) fn grain(p: Vec3, normal: Vec3, scale: f32, extent: Vec3, seed: f32) -> f32 {
    let axis = if extent.y > extent.x && extent.y >= extent.z {
        1
    } else if extent.z > extent.x {
        2
    } else {
        0
    };
    let axis = if normal[axis].abs() > 0.7 {
        usize::from(axis != 1)
    } else {
        axis
    };
    let [a, b, c] = [p[axis], p[(axis + 1) % 3], p[(axis + 2) % 3]];
    let seed = Vec3::splat(seed * 17.31);
    let warp = fbm(Vec3::new(a * 0.35, b * 1.5, c * 1.5) * scale + seed);
    let rings = (b + c) * scale * 2.5 + warp * 5.0;
    let ring = transition(0.6, 1., (rings.rem_euclid(1.) * 2. - 1.).abs());
    let fiber = fbm(Vec3::new(a * 0.5, b * 30., c * 30.) * scale + seed + 3.7);
    (ring * 0.7 + transition(0.35, 0.75, fiber) * 0.45).clamp(0., 1.)
}

fn hash3(point: Vec3) -> Vec3 {
    let [mut x, mut y, mut z] = point.floor().to_array().map(|v| {
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
    x = x.wrapping_add(y.wrapping_mul(z));
    y = y.wrapping_add(z.wrapping_mul(x));
    z = z.wrapping_add(x.wrapping_mul(y));
    Vec3::new(x as f32, y as f32, z as f32) * (1. / 4294967295.)
}

pub(super) fn cells(point: Vec3) -> f32 {
    let cell = point.floor();
    let fraction = point - cell;
    let mut nearest = 8.0_f32;
    for z in -1..=1 {
        for y in -1..=1 {
            for x in -1..=1 {
                let offset = Vec3::new(x as f32, y as f32, z as f32);
                let delta = offset + hash3(cell + offset) - fraction;
                nearest = nearest.min(delta.length_squared());
            }
        }
    }
    nearest.sqrt()
}

fn scratch_plane(point: Vec2, seed: f32, footprint: f32) -> f32 {
    let cell = point.floor();
    let mut mask = 0.0_f32;
    for y in -1..=1 {
        for x in -1..=1 {
            let cell = cell + Vec2::new(x as f32, y as f32);
            let position = hash3(cell.extend(seed));
            if position.z < 0.55 {
                continue;
            }
            let style = hash3((cell + Vec2::new(17., 31.)).extend(seed + 5.));
            let (sin, cos) = (style.x * std::f32::consts::PI).sin_cos();
            let direction = Vec2::new(cos, sin);
            let half_length = 0.35 + 1.25 * position.x.powi(2);
            let relative = point - (cell + position.truncate());
            let along = relative.dot(direction) / half_length;
            if along.abs() > 1. {
                continue;
            }
            let bend = (style.y - 0.5) * 0.25 * half_length;
            let distance = (relative.dot(Vec2::new(-sin, cos)) - bend * along.powi(2)).abs();
            let width = (0.006 + 0.03 * style.z.powi(2)) * (1. - transition(0.3, 1.0, along.abs()));
            let edge = footprint.max(1e-4);
            let line = (1. - distance / (width + edge)).clamp(0., 1.);
            let depth = 0.3 + 0.7 * (style.x * 13.7 + position.y * 7.1).fract();
            mask = mask.max(line.powi(2) * depth * (width / edge + 0.35).min(1.));
        }
    }
    mask
}

pub(super) fn scratches(point: Vec3, normal: Vec3, scale: f32, texel: f32, seed: f32) -> f32 {
    let weights = normal.abs().to_array().map(|n| n.powi(4));
    let sum = weights.iter().sum::<f32>().max(1e-4);
    let scale = scale * 5.;
    let point = point * scale;
    let footprint = texel * scale;
    let seed = seed * 3.7;
    let planes = [
        Vec2::new(point.y, point.z),
        Vec2::new(point.x, point.z),
        point.truncate(),
    ];
    weights
        .into_iter()
        .zip(planes)
        .enumerate()
        .filter_map(|(axis, (weight, plane))| {
            let weight = weight / sum;
            (weight >= 0.01).then(|| {
                let seed = seed + axis as f32;
                let bold = scratch_plane(plane, seed, footprint);
                let fine = scratch_plane(
                    plane * 2.7 + Vec2::new(3.1, 7.7),
                    seed + 11.,
                    footprint * 2.7,
                );
                weight * bold.max(fine * 0.55)
            })
        })
        .sum()
}
