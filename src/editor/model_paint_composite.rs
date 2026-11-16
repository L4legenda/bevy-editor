//! Like Material Painter's stroke_region/scissor, emit only damaged channels and texels.
use super::*;
pub(super) type Dirty = [Option<URect>; 8];
pub(super) struct Patch {
    pub rect: URect,
    pub rgba: Vec<u8>,
}
pub(super) struct Updates {
    // color/opacity, packed RM, normal, emission, then scalar preview textures.
    pub textures: [Option<Patch>; 9],
    pub transparent: Option<bool>,
    pub reflectance: Option<f32>,
}
pub(super) const PREVIEW: [usize; 8] = [0, 4, 5, 6, 7, 3, 2, 8];
pub(super) fn union(a: Option<URect>, b: Option<URect>) -> Option<URect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(URect {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        }),
        (None, b) => b,
        (a, None) => a,
    }
}
pub(super) fn prepare(surface: &Surface, dirty: Dirty) -> Updates {
    let mut updates = Updates {
        textures: std::array::from_fn(|_| None),
        transparent: None,
        reflectance: None,
    };
    let rects = [
        union(dirty[0], dirty[4]),
        union(dirty[1], dirty[2]),
        union(dirty[3], dirty[6]),
        dirty[5],
        dirty[1],
        dirty[2],
        dirty[3],
        dirty[4],
        dirty[7],
    ];
    for (target, rect) in rects.into_iter().enumerate() {
        let Some(mut rect) = rect else {
            continue;
        };
        if target == 2 {
            rect.min = rect.min.saturating_sub(UVec2::ONE);
            rect.max = (rect.max + UVec2::ONE).min(UVec2::splat(surface.size));
        }
        let rgba = match target {
            0 => {
                let mut color = surface.composite_rect(0, rect);
                let opacity = surface.composite_rect(4, rect);
                for (p, a) in color.chunks_exact_mut(4).zip(opacity.chunks_exact(4)) {
                    p[3] = a[0];
                }
                color
            }
            1 => {
                let mut rm = surface.composite_rect(1, rect);
                let metal = surface.composite_rect(2, rect);
                for (p, m) in rm.chunks_exact_mut(4).zip(metal.chunks_exact(4)) {
                    p[1] = p[0];
                    p[0] = 255;
                    p[2] = m[0];
                    p[3] = 255;
                }
                rm
            }
            2 => normal_rect(surface, rect),
            3 => surface.composite_rect(5, rect),
            _ => surface.composite_rect([1, 2, 3, 4, 7][target - 4], rect),
        };
        updates.textures[target] = Some(Patch { rect, rgba });
    }
    if dirty[4].is_some() {
        updates.transparent = Some(surface.composite(4).chunks_exact(4).any(|p| p[0] < 255));
    }
    if dirty[7].is_some() {
        updates.reflectance = Some(
            surface
                .composite(7)
                .chunks_exact(4)
                .map(|p| p[0] as f32 / 255.)
                .sum::<f32>()
                / (surface.size * surface.size) as f32,
        );
    }
    updates
}
fn normal_rect(surface: &Surface, rect: URect) -> Vec<u8> {
    let mut out = surface.composite_rect(6, rect);
    if !surface.has_channel(3) {
        return out;
    }
    let padded = URect {
        min: rect.min.saturating_sub(UVec2::ONE),
        max: (rect.max + UVec2::ONE).min(UVec2::splat(surface.size)),
    };
    let height = surface.composite_rect(3, padded);
    let width = rect.width() as usize;
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            let h = |x: u32, y: u32| {
                height[((y - padded.min.y) * padded.width() + x - padded.min.x) as usize * 4] as f32
                    / 255.
            };
            let dx = (h((x + 1).min(surface.size - 1), y) - h(x.saturating_sub(1), y)) * 4.;
            let dy = (h(x, (y + 1).min(surface.size - 1)) - h(x, y.saturating_sub(1))) * 4.;
            let i = ((y - rect.min.y) as usize * width + (x - rect.min.x) as usize) * 4;
            let n = Vec3::new(
                out[i] as f32 / 127.5 - 1. - dx,
                out[i + 1] as f32 / 127.5 - 1. + dy,
                (out[i + 2] as f32 / 127.5 - 1.).max(0.01),
            )
            .normalize();
            for c in 0..3 {
                out[i + c] = ((n[c] * 0.5 + 0.5) * 255.).round() as u8;
            }
        }
    }
    out
}
