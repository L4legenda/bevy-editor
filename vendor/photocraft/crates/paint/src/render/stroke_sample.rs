//! Sweep a measured strip of pigment along the input path. Travel advances the
//! sample continuously, and pressure scales the cross-section around the path.
use super::{Dab, Mips, Rect};

pub(super) fn rasterize(
    sample: &Mips,
    diameter: f32,
    hardness: f32,
    dab: &Dab,
    rect: Rect,
    out: &mut [f32],
) {
    let width = rect.width() as usize;
    if width == 0 {
        return;
    }
    let (sw, sh, _) = &sample.levels[0];
    let period = diameter * *sw as f32 / *sh as f32;
    let radius = dab.radius;
    let level = sample.level_for(2.0 * radius * (*sw).max(*sh) as f32 / *sh as f32);
    let half_texel = 0.5 / sample.levels[level].0 as f32;
    let phase = (dab.sample_position / f64::from(period)).rem_euclid(2.0) as f32;
    let (sin, cos) = dab.angle.sin_cos();
    let reach = radius + 0.5;
    let shoulder = (radius * (1.0 - hardness.clamp(0.0, 1.0))).max(0.5);
    for (y, row) in out.chunks_exact_mut(width).enumerate() {
        let dy = (f64::from(rect.y0) + y as f64 + 0.5 - dab.center.y) as f32;
        for (x, pixel) in row.iter_mut().enumerate() {
            let dx = (f64::from(rect.x0) + x as f64 + 0.5 - dab.center.x) as f32;
            let distance_sq = dx * dx + dy * dy;
            if distance_sq >= reach * reach {
                continue;
            }
            let along = dx * cos - dy * sin;
            let across = dx * sin + dy * cos;
            // Mirror at the ends so the seam is continuous even for widely spaced bristles.
            let u = 1.0 - ((phase + along / period).rem_euclid(2.0) - 1.0).abs();
            let v = 0.5 + across / (2.0 * radius);
            let remaining = reach - distance_sq.sqrt();
            // The sample already has its transverse falloff. Attenuate only the extra
            // distance to the end of the dab, so soft strokes also have soft end caps.
            let cap = (remaining / (reach - across.abs()).clamp(0.5, shoulder)).min(1.0);
            *pixel = sample.sample_clamped(level, u.clamp(half_texel, 1.0 - half_texel), v)
                * cap
                * dab.alpha;
        }
    }
}
