use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::editor) enum BakeMap {
    Occlusion,
    Cavity,
    Thickness,
    Sky,
    PartRandom,
    PartGradient,
    BentNormal,
    WorldNormal,
    Position,
    PartId,
    Curvature,
    EdgeDistance,
    CreaseDistance,
    ProjectedNormal,
    ProjectedHeight,
    ProjectionCoverage,
    Custom(u64),
}

impl BakeMap {
    pub fn supports_fix(self) -> bool {
        matches!(
            self,
            Self::Occlusion
                | Self::Cavity
                | Self::Thickness
                | Self::Sky
                | Self::PartRandom
                | Self::PartGradient
                | Self::Curvature
                | Self::EdgeDistance
                | Self::CreaseDistance
                | Self::ProjectedHeight
                | Self::ProjectedNormal
        )
    }
}

pub(in crate::editor) const EXTRAS: &str = "bevy_editor_bake_fixes";
pub(in crate::editor) type Fixes = Vec<(BakeMap, Arc<Plane>)>;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::editor) struct Plane {
    pub size: u32,
    // A premultiplied grayscale value and its coverage.
    pub pixels: Vec<u8>,
}

impl Plane {
    pub fn byte_len(size: u32) -> Option<usize> {
        (size > 0 && size <= *super::RESOLUTIONS.last()?)
            .then_some(size as usize * size as usize * 2)
    }

    pub fn sample(&self, index: usize, size: u32) -> (f32, f32) {
        let at = |x: u32, y: u32| {
            let i = (y as usize * self.size as usize + x as usize) * 2;
            (self.pixels[i] as f32, self.pixels[i + 1] as f32 / 255.)
        };
        let x = index as u32 % size;
        let y = index as u32 / size;
        if size == self.size {
            return at(x, y);
        }
        let coordinate = |value: u32| {
            let pos = ((value as f32 + 0.5) * self.size as f32 / size as f32 - 0.5)
                .clamp(0., (self.size - 1) as f32);
            (pos as u32, (pos as u32 + 1).min(self.size - 1), pos.fract())
        };
        let (x0, x1, tx) = coordinate(x);
        let (y0, y1, ty) = coordinate(y);
        let mix = |(av, ac): (f32, f32), (bv, bc): (f32, f32), t: f32| {
            (av + (bv - av) * t, ac + (bc - ac) * t)
        };
        mix(
            mix(at(x0, y0), at(x1, y0), tx),
            mix(at(x0, y1), at(x1, y1), tx),
            ty,
        )
    }
}
