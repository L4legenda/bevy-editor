//! Replace a flat flood colour and its contribution to the antialiased edge.
use super::*;
use crate::selection::{Region, rgba8_image, wand_region_bucket};

const EDGE_WIDTH: i32 = 2;
const DIRECTIONS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// An exact contiguous bucket with edge recolouring. Edge pixels are never
/// used to expand the flood into another region.
pub fn bucket_recolor_edges(
    surface: &mut Surface,
    area: Rect,
    seed: (i32, i32),
    color: [f32; 4],
    opacity: f32,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> bool {
    if !area.contains(seed.0, seed.1) {
        return false;
    }
    let img = rgba8_image(surface, area);
    let Some(region) = wand_region_bucket(&img, area, seed, 0.0, true, false) else {
        return false;
    };
    let sample = |x, y| {
        area.contains(x, y)
            .then(|| img[(y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize])
    };
    let original = surface.rgba(seed.0, seed.1);
    let mode = BlendMode::Normal;
    let painted = if lock_transparency {
        blend::composite_atop(mode, original, color, opacity)
    } else {
        blend::composite(mode, original, color, opacity)
    };
    let seed_color = premultiply(original);
    let painted = premultiply(painted);
    let change: [f32; 4] = std::array::from_fn(|c| painted[c] - seed_color[c]);
    if change == [0.0; 4] {
        return false;
    }
    let bounds = region.bbox.inflate(EDGE_WIDTH).intersect(&area);
    let format = surface.format();
    let channels = format.channels();
    for tile in bounds.tiles() {
        let rect = tile.rect().intersect(&bounds);
        let width = rect.width() as usize;
        let mut data = None;
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                if region.at(x, y) > 0.0 {
                    continue;
                }
                let selection = selection.map_or(1.0, |mask| mask.sample_channel(x, y, 0));
                if selection <= 0.0 {
                    continue;
                }
                let Some(weight) = edge_weight(x, y, &region, &sample, seed_color) else {
                    continue;
                };
                let buffer = data.get_or_insert_with(|| surface.read_region(rect));
                let i = ((y - rect.y0) as usize * width + (x - rect.x0) as usize) * channels;
                let pixel = &mut buffer[i..i + channels];
                let before = premultiply(photocraft_raster::to_rgba(&format, pixel));
                let mut after: [f32; 4] =
                    std::array::from_fn(|c| before[c] + change[c] * weight * selection);
                if lock_transparency {
                    after[3] = before[3];
                }
                let alpha = after[3].clamp(0.0, 1.0);
                let rgba = if alpha > 0.0 {
                    [after[0] / alpha, after[1] / alpha, after[2] / alpha, alpha]
                } else {
                    [0.0; 4]
                };
                photocraft_raster::from_rgba_into(&format, rgba, pixel);
            }
        }
        if let Some(buffer) = data {
            surface.write_region(rect, &buffer);
        }
    }
    fill_region(
        surface,
        &region,
        opacity,
        selection,
        lock_transparency,
        |_, _| color,
        None,
        FillMode::Blend(mode),
        false,
    );
    true
}

fn premultiply(mut rgba: [f32; 4]) -> [f32; 4] {
    let alpha = rgba[3];
    rgba[..3].iter_mut().for_each(|c| *c *= alpha);
    rgba
}

fn edge_weight(
    x: i32,
    y: i32,
    region: &Region,
    sample: &impl Fn(i32, i32) -> Option<[u8; 4]>,
    seed: [f32; 4],
) -> Option<f32> {
    let color = |p: [u8; 4]| premultiply(p.map(|c| f32::from(c) / 255.0));
    let pixel = color(sample(x, y)?);
    let mut best: Option<(f32, f32)> = None;
    for (dx, dy) in DIRECTIONS {
        let Some(inner) =
            (1..=EDGE_WIDTH).find(|step| region.at(x - dx * step, y - dy * step) > 0.0)
        else {
            continue;
        };
        for outer in 1..=EDGE_WIDTH {
            let end = (x + dx * outer, y + dy * outer);
            let Some(outside) = sample(end.0, end.1) else {
                break;
            };
            if region.at(end.0, end.1) > 0.0 {
                break;
            }
            // Require a stable outside colour. A hard edge against that
            // colour has zero weight, so it stays pixel-sharp.
            if sample(end.0 + dx, end.1 + dy) != Some(outside) {
                continue;
            }
            let outside = color(outside);
            let delta: [f32; 4] = std::array::from_fn(|c| seed[c] - outside[c]);
            let length: f32 = delta.iter().map(|d| d * d).sum();
            if length < (4.0f32 / 255.0).powi(2) {
                continue;
            }
            let project = |p: [f32; 4]| {
                let weight = (0..4).map(|c| (p[c] - outside[c]) * delta[c]).sum::<f32>() / length;
                let error = (0..4)
                    .map(|c| (p[c] - outside[c] - delta[c] * weight).abs())
                    .fold(0.0f32, f32::max);
                (weight, error)
            };
            let (weight, error) = project(pixel);
            let max_error = 1.6 / 255.0;
            if !(0.001..0.999).contains(&weight) || error > max_error {
                continue;
            }
            let mut previous = 1.0;
            let monotonic = (-inner + 1..outer).all(|step| {
                let Some(p) = sample(x + dx * step, y + dy * step) else {
                    return false;
                };
                let (part, residual) = project(color(p));
                let valid = part > 0.0 && part < previous + 0.001 && residual <= max_error;
                previous = part;
                valid
            });
            if monotonic && best.is_none_or(|(score, _)| error < score) {
                best = Some((error, weight));
            }
            break;
        }
    }
    best.map(|(_, weight)| weight)
}
