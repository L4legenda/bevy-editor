//! Texture-space and projected patterns for procedural masks.
use super::{Generator, GeneratorKind};
use bevy::prelude::{EulerRot, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct GradientMapping {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub rotation: f32,
    pub repeat: [bool; 2],
    pub volume: Option<GradientVolume>,
}

impl Default for GradientMapping {
    fn default() -> Self {
        Self {
            scale: [1.; 2],
            offset: [0.; 2],
            rotation: 0.,
            repeat: [true; 2],
            volume: None,
        }
    }
}

impl GradientMapping {
    pub fn full_coverage(self) -> bool {
        self.repeat == [true; 2] && self.volume.is_none()
    }

    pub fn valid(self) -> bool {
        Vec2::from_array(self.scale).is_finite()
            && Vec2::from_array(self.offset).is_finite()
            && self.rotation.is_finite()
            && self.volume.is_none_or(GradientVolume::valid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) struct GradientVolume {
    pub center: [f32; 3],
    pub size: [f32; 3],
    pub rotation: [f32; 3],
    pub crop: bool,
    pub projection: GradientProjection,
    pub backface: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::editor) enum GradientProjection {
    Planar { depth: Option<f32> },
    Triplanar { hardness: f32 },
}

impl GradientVolume {
    fn valid(self) -> bool {
        let size = Vec3::from_array(self.size);
        let unit = |v| (0. ..=1.).contains(&v);
        size.is_finite()
            && size.min_element() > 0.
            && Vec3::from_array(self.center).is_finite()
            && Vec3::from_array(self.rotation).is_finite()
            && self
                .backface
                .is_none_or(|params| params.into_iter().all(unit))
            && match self.projection {
                GradientProjection::Planar { depth } => depth.is_none_or(unit),
                GradientProjection::Triplanar { hardness } => unit(hardness),
            }
    }
}

pub(super) struct Sampler {
    generator: Generator,
    mapping: GradientMapping,
    uv_angle: Vec2,
    direction: Vec2,
    inverse_rotation: Quat,
    tiles: Option<super::tiles::Sampler>,
}

impl Sampler {
    pub fn new(generator: Generator) -> Self {
        let mapping = generator.gradient_mapping.unwrap_or_default();
        let inverse_rotation = mapping.volume.map_or(Quat::IDENTITY, |volume| {
            let [x, y, z] = volume.rotation;
            Quat::from_euler(EulerRot::XYZ, x, y, z).inverse()
        });
        Self {
            generator,
            mapping,
            uv_angle: Vec2::from_angle(mapping.rotation),
            direction: Vec2::from_angle(generator.values[3].to_radians()),
            inverse_rotation,
            tiles: (generator.kind == GeneratorKind::TilePattern)
                .then(|| super::tiles::Sampler::new(generator.values, generator.tile_options.unwrap_or_default())),
        }
    }

    pub fn uv(&self, uv: Vec2) -> [u8; 2] {
        self.sample(uv).map_or([0; 2], |value| [pack(value), 255])
    }

    fn sample(&self, uv: Vec2) -> Option<f32> {
        let center = Vec2::splat(0.5);
        let uv = center
            + self.uv_angle.rotate(
                (uv - center - Vec2::from_array(self.mapping.offset))
                    * Vec2::from_array(self.mapping.scale),
            );
        let mut uv = uv.to_array();
        for (value, repeat) in uv.iter_mut().zip(self.mapping.repeat) {
            if repeat {
                *value = value.rem_euclid(1.);
            } else if !(0. ..=1.).contains(value) {
                return None;
            }
        }
        if let Some(tiles) = &self.tiles {
            let value = tiles.sample(Vec2::from_array(uv));
            return Some(if self.generator.invert { 1. - value } else { value });
        }
        if self.generator.kind.is_shape() {
            let value = self.shape(Vec2::from_array(uv) - center);
            return Some(if self.generator.invert { 1. - value } else { value });
        }
        let [position, contrast, repeats, _, peak] = self.generator.values;
        let axis = 0.5 + (Vec2::from_array(uv) - center).dot(self.direction.perp());
        let t = if repeats == 1. {
            axis.clamp(0., 1.)
        } else {
            (axis * repeats).rem_euclid(1.)
        };
        let value = match self.generator.kind {
            GeneratorKind::GradientMirror => (std::f32::consts::PI * t).sin(),
            GeneratorKind::GradientPeak => {
                let peak = peak / 100.;
                if peak == 0. {
                    1. - t
                } else if peak == 1. {
                    t
                } else if t <= peak {
                    t / peak
                } else {
                    (1. - t) / (1. - peak)
                }
            }
            _ => t,
        };
        let width = (1. - contrast / 100.).max(1. / 255.);
        let value = if position == 0. || position == 100. {
            position / 100.
        } else {
            ((value + position / 100. - 1.) / width + 0.5).clamp(0., 1.)
        };
        Some(if self.generator.invert {
            1. - value
        } else {
            value
        })
    }

    fn shape(&self, point: Vec2) -> f32 {
        let [size, hardness, width, ..] = self.generator.values;
        if size == 0. {
            return 0.;
        }
        let radius = match self.generator.kind {
            GeneratorKind::ShapeSquareBorder => point.abs().max_element(),
            _ => point.length(),
        } * 200. / size;
        if self.generator.kind == GeneratorKind::ShapeCone {
            return (1. - radius).max(0.).powf(2. - hardness / 100.);
        }
        let edge = (1. - hardness / 100.).max(1e-5);
        if self.generator.kind == GeneratorKind::ShapeDisc {
            return super::smooth(((1. - radius) / edge).clamp(0., 1.));
        }
        let width = width / 100.;
        if width == 0. {
            return 0.;
        }
        let softness = edge.min(width * 0.5);
        let outer = super::smooth(((1. - radius) / softness).clamp(0., 1.));
        if width == 1. {
            return outer;
        }
        let inner = super::smooth(((radius - (1. - width)) / softness).clamp(0., 1.));
        outer * inner
    }

    pub fn projected(&self, point: Vec3, normal: Vec3) -> [u8; 2] {
        let Some(volume) = self.mapping.volume else {
            return [0; 2];
        };
        let point = (self.inverse_rotation * (point - Vec3::from_array(volume.center)))
            / Vec3::from_array(volume.size);
        let normal = self.inverse_rotation * normal;
        match volume.projection {
            GradientProjection::Planar { depth } => {
                if volume.crop && point.truncate().abs().max_element() > 0.5 {
                    return [0; 2];
                }
                let mut coverage =
                    depth.map_or(1., |hardness| fade(1. - 2. * point.z.abs(), hardness));
                if let Some([angle, hardness]) = volume.backface {
                    coverage *= fade(normal.z + 1. - 2. * angle, hardness);
                }
                self.sample(point.truncate() + Vec2::splat(0.5))
                    .map_or([0; 2], |value| [pack(value), pack(coverage)])
            }
            GradientProjection::Triplanar { hardness } => {
                if volume.crop && point.abs().max_element() > 0.5 {
                    return [0; 2];
                }
                let axes = [
                    Vec2::new(-point.z, point.y),
                    Vec2::new(point.x, -point.z),
                    point.truncate(),
                ];
                let weights = normal.abs().powf(1. + 31. * hardness);
                let mut value = 0.;
                let mut coverage = 0.;
                for (uv, weight) in axes.into_iter().zip(weights.to_array()) {
                    if weight > 0.
                        && let Some(sample) = self.sample(uv + Vec2::splat(0.5))
                    {
                        value += sample * weight;
                        coverage += weight;
                    }
                }
                if coverage > 0. {
                    [
                        pack(value / coverage),
                        pack(coverage / weights.element_sum()),
                    ]
                } else {
                    [0; 2]
                }
            }
        }
    }
}

fn fade(distance: f32, hardness: f32) -> f32 {
    super::smooth((distance / (1. - hardness).max(1e-5)).clamp(0., 1.))
}

fn pack(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_applies_texture_transform_and_single_axis_repeat() {
        let mut generator = Generator::new(GeneratorKind::GradientLinear);
        generator.gradient_mapping = Some(GradientMapping {
            scale: [2., 1.],
            offset: [0.125, 0.],
            rotation: std::f32::consts::FRAC_PI_2,
            repeat: [false, true],
            volume: None,
        });
        let sampler = Sampler::new(generator);
        assert_eq!(sampler.uv(Vec2::new(0.75, 0.25)), [191, 255]);
        assert_eq!(sampler.uv(Vec2::new(0.25, 0.25)), [191, 255]);
        assert_eq!(sampler.uv(Vec2::new(0.625, 1.25)), [0, 0]);
    }

    #[test]
    fn planar_gradient_clips_in_projector_space_and_fades_near_the_depth_boundary() {
        let center = Vec3::new(2., -1., 3.);
        let mut generator = Generator::new(GeneratorKind::GradientLinear);
        generator.gradient_mapping = Some(GradientMapping {
            volume: Some(GradientVolume {
                center: center.to_array(),
                size: [2., 2., 2.],
                rotation: [0., std::f32::consts::FRAC_PI_2, 0.],
                crop: true,
                projection: GradientProjection::Planar { depth: Some(0.5) },
                backface: Some([0.5, 0.5]),
            }),
            ..Default::default()
        });
        let sampler = Sampler::new(generator);
        assert_eq!(
            sampler.projected(center + Vec3::Y * 0.5, Vec3::X),
            [191, 255]
        );
        assert_eq!(
            sampler.projected(center + Vec3::X * 0.75, Vec3::X),
            [128, 128]
        );
        assert_eq!(sampler.projected(center + Vec3::Z * 1.5, Vec3::X), [0, 0]);
        assert_eq!(sampler.projected(center, -Vec3::X)[1], 0);
    }
}
