//! Replay imported brush actions without requiring a render device in the asset importer.
use super::{Attributes, PolygonReplay, spp_polygon::ray};
use crate::editor::model_paint_data::{mask::fbm, texture_fill::Bitmap};
use crate::editor::ui::model_viewer::paint::tips::splatter_alpha;
use bevy::{math::Mat2, prelude::*};
use subcraft_spp::document::{BrushInfo, StrokeInfo};

const MAX_SAMPLES: usize = 131_072;
const MAX_TEXEL_TESTS: usize = 64 * 1024 * 1024;

pub(in crate::editor) struct Stencil<'a> {
    source: StencilSource<'a>,
    transform: Mat3,
}

enum StencilSource<'a> {
    Bitmap(&'a Bitmap),
    Splatter(Splatter),
}

struct Splatter {
    alpha: &'static image::GrayImage,
    flip: [bool; 2],
    inverse: bool,
    exponent: f32,
}

impl Splatter {
    fn new(hardness: f32, flip: [bool; 2], inverse: bool) -> Self {
        let hardness = if hardness.is_finite() {
            hardness.clamp(0., 1.)
        } else {
            0.5
        };
        Self {
            alpha: splatter_alpha(),
            flip,
            inverse,
            exponent: (2. - 4. * hardness).exp2(),
        }
    }

    fn sample(&self, raster: Vec2) -> f32 {
        let [x, y] = raster.to_array();
        let uv = Vec2::new(
            if self.flip[0] { 1. - x } else { x },
            if self.flip[1] { 1. - y } else { y },
        );
        let value = sample_gray(self.alpha, uv).powf(self.exponent);
        if self.inverse { 1. - value } else { value }
    }
}

impl<'a> Stencil<'a> {
    pub fn new(bitmap: &'a Bitmap, matrix: &[f32; 9]) -> Option<Self> {
        Self::with_source(StencilSource::Bitmap(bitmap), matrix)
    }

    pub fn procedural(url: &str, tweaks: &[(String, Vec<f32>)], matrix: &[f32; 9]) -> Option<Self> {
        let (path, query) = url.split_once('?')?;
        if !query
            .split('&')
            .any(|argument| argument.starts_with("version=") && argument.ends_with(".sbsar"))
            || !path
                .rsplit('/')
                .next()?
                .eq_ignore_ascii_case("drop_splatter")
        {
            return None;
        }
        let value = |name: &str, default: f32| {
            tweaks
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .and_then(|(_, values)| values.first().copied())
                .filter(|value| value.is_finite())
                .unwrap_or(default)
        };
        Self::with_source(
            StencilSource::Splatter(Splatter::new(
                value("hardness", 0.5),
                [
                    value("flip_horizontal", 0.) >= 0.5,
                    value("flip_vertical", 0.) >= 0.5,
                ],
                value("invert", 0.) >= 0.5,
            )),
            matrix,
        )
    }

    fn with_source(source: StencilSource<'a>, matrix: &[f32; 9]) -> Option<Self> {
        let transform = Mat3::from_cols_array(matrix);
        (transform.is_finite() && transform.determinant().abs() > 1e-12)
            .then_some(Self { source, transform })
    }

    fn sample(&self, clip: Vec4) -> f32 {
        if !clip.is_finite() || clip.w <= 1e-12 {
            return 0.;
        }
        let screen = clip.truncate().truncate() / clip.w;
        let image = self.transform * screen.extend(1.);
        if !image.is_finite() || image.z.abs() <= 1e-12 {
            return 0.;
        }
        let uv = image.truncate() / image.z;
        if uv.min_element() < 0. || uv.max_element() > 1. {
            return 0.;
        }
        let raster = Vec2::new(uv.x, 1. - uv.y);
        match &self.source {
            StencilSource::Bitmap(bitmap) => {
                let (width, height) = bitmap.pixels().dimensions();
                let margin = Vec2::splat(0.5) / Vec2::new(width as f32, height as f32);
                let pixel = bitmap.sample(raster.clamp(margin, Vec2::ONE - margin), 0.);
                pixel.truncate().dot(Vec3::new(0.2126, 0.7152, 0.0722)) * pixel.w
            }
            StencilSource::Splatter(splatter) => splatter.sample(raster),
        }
    }
}

pub(in crate::editor) struct StrokeCoverage {
    size: u32,
    values: Vec<f32>,
    min: UVec2,
    max: UVec2,
    budget: usize,
    pub partial: bool,
}

impl StrokeCoverage {
    fn new(size: u32) -> Self {
        Self {
            size,
            values: vec![0.; (size * size) as usize],
            min: UVec2::splat(size),
            max: UVec2::ZERO,
            budget: MAX_TEXEL_TESTS,
            partial: false,
        }
    }

    pub fn pixels(&self, opacity: f32) -> impl Iterator<Item = (usize, f32)> + '_ {
        (self.min.y..self.max.y).flat_map(move |y| {
            (self.min.x..self.max.x).filter_map(move |x| {
                let index = (y * self.size + x) as usize;
                let weight = self.values[index] * opacity;
                (weight > 0.).then_some((index * 4, weight))
            })
        })
    }

    pub fn pad(&mut self, sources: &[u32], distance: u32) {
        if !self.min.cmplt(self.max).all() {
            return;
        }
        let range = UVec2::splat(distance);
        let min = self.min.max(range) - range;
        let max = (self.max + range).min(UVec2::splat(self.size));
        for y in min.y..max.y {
            for x in min.x..max.x {
                let index = (y * self.size + x) as usize;
                let source = sources[index] as usize;
                if self.values[index] == 0.
                    && let Some(&value) = self.values.get(source)
                    && value > 0.
                {
                    self.values[index] = value;
                    self.min = self.min.min(UVec2::new(x, y));
                    self.max = self.max.max(UVec2::new(x + 1, y + 1));
                }
            }
        }
    }

    fn stamp(&mut self, bounds: Rect, mut sample: impl FnMut(Vec2, usize) -> f32) {
        let extent = Vec2::splat(self.size as f32);
        let min = (bounds.min * extent)
            .floor()
            .clamp(Vec2::ZERO, extent)
            .as_uvec2();
        let max = (bounds.max * extent)
            .ceil()
            .clamp(Vec2::ZERO, extent)
            .as_uvec2();
        let area = (max - min).element_product() as usize;
        if area > self.budget {
            self.partial = true;
            self.budget = 0;
            return;
        }
        self.budget -= area;
        for y in min.y..max.y {
            for x in min.x..max.x {
                let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / extent;
                let index = (y * self.size + x) as usize;
                let weight = sample(point, index);
                if weight > 0. {
                    self.min = self.min.min(UVec2::new(x, y));
                    self.max = self.max.max(UVec2::new(x + 1, y + 1));
                    let value = &mut self.values[index];
                    *value += (1. - *value) * weight;
                }
            }
        }
    }
}

impl PolygonReplay {
    pub fn stroke(
        &self,
        strokes: &[StrokeInfo],
        brush: &BrushInfo,
        (slot, size, owners): (usize, u32, Option<&[u32]>),
        stencil: Option<&Stencil<'_>>,
        tip: Option<&Bitmap>,
    ) -> StrokeCoverage {
        let mut coverage = StrokeCoverage::new(size);
        let hardness = match brush.tip.as_str() {
            "shape" | "shape_v2" | "dirt_cloudy" => brush.hardness.unwrap_or(0.5).clamp(0., 0.99),
            "RoundBrush" | "SquareBrush" => 0.8,
            _ => 0.,
        };
        let cloudy = brush.tip == "dirt_cloudy";
        coverage.partial = !matches!(
            brush.tip.as_str(),
            "" | "shape" | "shape_v2" | "RoundBrush" | "SquareBrush"
        ) || brush.tip_text.is_some()
            || (brush.tip_bitmap.is_some() && tip.is_none())
            || (brush.tip_invert && !cloudy)
            || (brush.stencil.is_some()
                && (stencil
                    .is_none_or(|stencil| matches!(stencil.source, StencilSource::Splatter(_)))
                    || strokes.iter().any(|stroke| stroke.uv_space)))
            || brush.size_space != 1
            || brush.alignment != 0;
        let flow = brush.flow.clamp(0.01, 1.);
        let square = brush.tip == "SquareBrush";
        let cloud = (cloudy && tip.is_none()).then(|| cloudy_tip(brush, hardness));
        let splatter =
            (tip.is_none() && brush.tip.eq_ignore_ascii_case("drop_splatter")).then(|| {
                Splatter::new(
                    brush.hardness.unwrap_or(0.5),
                    brush.tip_flip,
                    brush.tip_invert,
                )
            });
        let square_bounds = square || tip.is_some() || splatter.is_some();
        let directional = square_bounds || cloud.is_some();
        let jitter = brush.rotation_jitter.clamp(0., std::f32::consts::PI);
        let scatter = brush.position_jitter.clamp(0., 1.);
        // The spatial settings are preserved, using the editor's deterministic random sequence.
        coverage.partial |= jitter > 0. || scatter > 0.;
        let extent = if square_bounds {
            3f32.sqrt()
        } else {
            std::f32::consts::SQRT_2
        };
        let shape = |offset: Vec2| {
            if offset.abs().max_element() > 1. {
                return 0.;
            }
            let uv = (offset + Vec2::ONE) * 0.5;
            if let Some(tip) = tip {
                let dims = Vec2::new(tip.pixels().width() as f32, tip.pixels().height() as f32);
                let margin = Vec2::splat(0.5) / dims;
                let rgba = tip.sample(uv.clamp(margin, Vec2::ONE - margin), 0.);
                return if tip.has_alpha() {
                    rgba.w
                } else {
                    rgba.truncate().dot(Vec3::new(0.2126, 0.7152, 0.0722))
                };
            }
            if let Some(cloud) = &cloud {
                return sample_gray(cloud, uv);
            }
            if let Some(splatter) = &splatter {
                return splatter.sample(uv);
            }
            let distance = if square {
                offset.abs().max_element()
            } else {
                offset.length()
            };
            1. - smoothstep(hardness, 1., distance)
        };
        let mut remaining = MAX_SAMPLES;
        let mut nearby = Vec::new();
        for stroke in strokes {
            if stroke.points.is_empty() || (!stroke.uv_space && owners.is_none()) {
                coverage.partial = true;
                continue;
            }
            let view = Mat4::from_cols_array(&stroke.view);
            let projection = Mat4::from_cols_array(&stroke.projection);
            let depth = if projection.w_axis.w.abs() > 0.5 {
                1.
            } else {
                (-view.transform_point3(self.center).z).max(1e-6)
            };
            let (span, diameter) = if stroke.uv_space {
                (Vec2::ONE, 0.15 * brush.size)
            } else {
                let span = Vec2::new(projection.x_axis.x.abs(), projection.y_axis.y.abs())
                    .max(Vec2::splat(1e-6))
                    .recip()
                    * (2. * depth);
                (span, 1.5 * brush.size * span.y)
            };
            if stroke.points.len() == 1
                && (brush.pressure_size && brush.pressure_size_min < 1.
                    || brush.pressure_flow && brush.pressure_flow_min < 1.)
                && !stroke
                    .pressure
                    .first()
                    .is_some_and(|pressure| pressure.is_finite())
            {
                coverage.partial = true;
            }
            let pressure = |index: usize| {
                // Empty tracks also occur for mouse taps with pen dynamics enabled.
                // Use the nominal brush size and flow when no pressure was saved.
                let pressure = stroke
                    .pressure
                    .get(index)
                    .copied()
                    .filter(|value| value.is_finite())
                    .unwrap_or(1.)
                    .clamp(0., 1.);
                let scale = |enabled, lower: f32| {
                    if enabled {
                        let lower = lower.clamp(0., 1.);
                        lower + pressure * (1. - lower)
                    } else {
                        1.
                    }
                };
                Vec2::new(
                    scale(brush.pressure_size, brush.pressure_size_min),
                    scale(brush.pressure_flow, brush.pressure_flow_min),
                )
            };
            let step = brush.spacing.max(0.02) * diameter;
            if !step.is_finite() || step <= 0. {
                coverage.partial = true;
                continue;
            }
            let copies = std::iter::once(Mat4::IDENTITY).chain(
                stroke
                    .duplicate
                    .map(|matrix| Mat4::from_cols_array(&matrix)),
            );
            for copy in copies {
                let view_proj = projection * view * copy;
                let inverse = view_proj.inverse();
                let right = inverse.x_axis.truncate().normalize_or_zero();
                if !copy.is_finite() || (!stroke.uv_space && !inverse.is_finite()) {
                    coverage.partial = true;
                    continue;
                }
                let uv_transform = if stroke.uv_space {
                    let linear = Mat2::from_cols(copy.x_axis.xy(), copy.y_axis.xy());
                    if linear.determinant().abs() <= 1e-12 {
                        coverage.partial = true;
                        continue;
                    }
                    Some((linear, linear.inverse(), copy.w_axis.xy()))
                } else {
                    None
                };
                let uv_offset = self.uv_offset(slot);
                let mut landed = false;
                let mut attempted = false;
                // Symmetry uses the same seed and stamp order as the original path.
                let mut noise = StampNoise(stroke.seed);
                let mut dab = |point: Vec2, pressure: Vec2, direction: Option<Vec2>| {
                    if remaining == 0 || coverage.budget == 0 {
                        return false;
                    }
                    remaining -= 1;
                    let radius = 0.5 * diameter * pressure.x;
                    let stamp_flow = flow * pressure.y;
                    if !point.is_finite() || !radius.is_finite() || radius <= 0. || stamp_flow <= 0.
                    {
                        return true;
                    }
                    attempted = true;
                    let heading = direction.unwrap_or(Vec2::X);
                    let angle = if brush.follow_path {
                        (-heading.y).atan2(heading.x)
                    } else {
                        0.
                    };
                    let angle = angle
                        + brush.rotation
                        + if jitter > 0. {
                            noise.signed() * jitter
                        } else {
                            0.
                        };
                    let offset = if scatter > 0. {
                        if brush.position_jitter_both_axes {
                            Vec2::new(noise.signed(), noise.signed())
                        } else {
                            heading.perp() * noise.signed()
                        }
                    } else {
                        Vec2::ZERO
                    };
                    let point = point + offset * (radius * scatter) / span;
                    let axes = if directional {
                        Mat2::from_angle(angle)
                    } else {
                        Mat2::IDENTITY
                    };
                    let local = axes.transpose();
                    if stroke.uv_space {
                        let mirrored = copy.transform_point3((point * 2. - Vec2::ONE).extend(0.));
                        let center =
                            Vec2::new(0.5 + mirrored.x * 0.5, 0.5 - mirrored.y * 0.5) - uv_offset;
                        let (linear, unmirror, shift) = uv_transform.unwrap();
                        let flip_y = Mat2::from_diagonal(Vec2::new(1., -1.));
                        let transform = flip_y * linear * flip_y * axes;
                        let sample = local * flip_y * unmirror * flip_y;
                        let extent = if directional {
                            transform.x_axis.abs() + transform.y_axis.abs()
                        } else {
                            Vec2::ONE
                        };
                        let bounds = Rect::from_center_size(center, extent * (2. * radius));
                        coverage.stamp(bounds, |uv, _| {
                            let normalized = (uv - center) / radius;
                            let weight = shape(if directional {
                                sample * normalized
                            } else {
                                normalized
                            }) * stamp_flow;
                            if weight > 0.
                                && let Some(stencil) = stencil
                            {
                                // UV strokes lack a usable view camera. Use the normalized UV
                                // canvas and sample symmetry copies in the original stencil frame.
                                let global = uv + uv_offset;
                                let clip = Vec2::new(2. * global.x - 1., 1. - 2. * global.y);
                                let original = unmirror * (clip - shift);
                                weight * stencil.sample(original.extend(0.).extend(1.))
                            } else {
                                weight
                            }
                        });
                        landed |=
                            bounds.min.cmplt(Vec2::ONE).all() && bounds.max.cmpgt(Vec2::ZERO).all();
                    } else if let Some(owners) = owners
                        && let Some((origin, direction)) = ray(inverse, point)
                        && let Some((center, normal, _, _)) =
                            self.trace.hit(origin, direction, 0, slot, false)
                    {
                        landed = true;
                        nearby.clear();
                        let reach = Vec3::splat(radius * extent);
                        let tangent = (right - normal * right.dot(normal))
                            .try_normalize()
                            .unwrap_or_else(|| normal.any_orthonormal_vector());
                        self.trace.bvh.triangles_in_box(
                            center - reach,
                            center + reach,
                            &mut nearby,
                        );
                        for &index in &nearby {
                            let triangle = &self.trace.attributes[index];
                            if triangle.slot == slot {
                                surface_dab(
                                    &mut coverage,
                                    triangle,
                                    |pixel| owners[pixel] == index as u32,
                                    SurfaceDab {
                                        center,
                                        normal,
                                        tangent,
                                        radius,
                                        extent,
                                    },
                                    |offset| shape(local * offset) * stamp_flow,
                                    |point| {
                                        stencil.map_or(1., |stencil| {
                                            stencil.sample(view_proj * point.extend(1.))
                                        })
                                    },
                                );
                            }
                            if coverage.budget == 0 {
                                break;
                            }
                        }
                    }
                    true
                };
                // Calculate spacing before the pressure adjustment, as in the recorded emitter.
                let first = Vec2::from_array(stroke.points[0]);
                if !brush.follow_path && !dab(first, pressure(0), None) {
                    coverage.partial = true;
                    return coverage;
                }
                for (index, pair) in stroke.points.windows(2).enumerate() {
                    let a = Vec2::from_array(pair[0]);
                    let b = Vec2::from_array(pair[1]);
                    let delta = (b - a) * span;
                    let direction = delta.try_normalize();
                    if direction.is_none() && brush.follow_path {
                        continue;
                    }
                    let count = (delta.length() / step).ceil().max(1.) as usize;
                    for k in 1..=count {
                        let t = k as f32 / count as f32;
                        if !dab(
                            a.lerp(b, t),
                            pressure(index).lerp(pressure(index + 1), t),
                            direction,
                        ) {
                            coverage.partial = true;
                            return coverage;
                        }
                    }
                }
                coverage.partial |= attempted && !landed;
            }
        }
        coverage
    }
}

fn sample_gray(image: &image::GrayImage, uv: Vec2) -> f32 {
    let size = Vec2::new(image.width() as f32, image.height() as f32);
    let at = (uv * size - Vec2::splat(0.5)).clamp(Vec2::ZERO, size - Vec2::ONE);
    image::imageops::interpolate_bilinear(image, at.x, at.y)
        .map_or(0., |alpha| f32::from(alpha[0]) / 255.)
}

#[cfg(test)]
#[test]
fn procedural_splatter_stencil_respects_recorded_tweaks() {
    let url = "resource://starter_assets/drop_splatter/drop_splatter?version=test.sbsar";
    let identity = Mat3::IDENTITY.to_cols_array();
    let stencil = |values: &[(&str, f32)]| {
        let tweaks: Vec<_> = values
            .iter()
            .map(|(name, value)| (name.to_string(), vec![*value]))
            .collect();
        Stencil::procedural(url, &tweaks, &identity).unwrap()
    };
    let sample = |stencil: &Stencil<'_>, x, y| stencil.sample(Vec4::new(x, y, 0., 1.));
    let base = stencil(&[]);
    let inverted = stencil(&[("invert", 1.)]);
    let flip_x = stencil(&[("flip_horizontal", 1.)]);
    let flip_y = stencil(&[("flip_vertical", 1.)]);
    let soft = stencil(&[("hardness", 0.)]);
    let hard = stencil(&[("hardness", 1.)]);
    let mut soft_edge = false;
    let mut asymmetric = [false; 2];
    for row in 0..64 {
        for col in 0..64 {
            let x = (col as f32 + 0.5) / 64.;
            let y = (row as f32 + 0.5) / 64.;
            let value = sample(&base, x, y);
            let mirrored_x = sample(&base, 1. - x, y);
            let mirrored_y = sample(&base, x, 1. - y);
            assert!((sample(&inverted, x, y) + value - 1.).abs() < 0.001);
            assert!((sample(&flip_x, x, y) - mirrored_x).abs() < 0.001);
            assert!((sample(&flip_y, x, y) - mirrored_y).abs() < 0.001);
            asymmetric[0] |= value != mirrored_x;
            asymmetric[1] |= value != mirrored_y;
            if (0.01..0.99).contains(&value) {
                assert!(sample(&soft, x, y) < value && sample(&hard, x, y) > value);
                soft_edge = true;
            }
        }
    }
    assert!(soft_edge && asymmetric == [true; 2]);
    assert_eq!(sample(&inverted, 1.5, 0.5), 0.);
    assert!(Stencil::procedural("unlisted?version=test.sbsar", &[], &identity).is_none());
}

struct StampNoise(u32);

impl StampNoise {
    fn signed(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e3779b9);
        let mut value = self.0;
        value = (value ^ (value >> 16)).wrapping_mul(0x21f0aaad);
        value = (value ^ (value >> 15)).wrapping_mul(0x735a2d97);
        let value = (value ^ (value >> 15)) >> 8;
        value as f32 * (2. / 16_777_216.) - 1.
    }
}

fn cloudy_tip(brush: &BrushInfo, hardness: f32) -> image::GrayImage {
    const SIDE: u32 = 128;
    let shape = brush
        .tip_shape
        .filter(|v| v.is_finite())
        .unwrap_or(0.75)
        .clamp(0., 1.);
    // Use the editor's noise to reconstruct a textured alpha for this stock graph.
    // The saved Substance graph itself is not evaluated.
    let seed = Vec3::new(
        (brush.tip_seed & 255) as f32,
        ((brush.tip_seed >> 8) & 255) as f32,
        11.,
    );
    image::GrayImage::from_fn(SIDE, SIDE, |x, y| {
        let point =
            (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) * (2. / SIDE as f32) - Vec2::ONE;
        let edge = 1. - smoothstep(hardness, 1., point.length());
        let noise = fbm(point.extend(0.) * (4. + 6. * shape) + seed);
        let density = smoothstep(0.15 + 0.3 * shape, 0.82, noise);
        let density = if brush.tip_invert {
            1. - density
        } else {
            density
        };
        image::Luma([(edge * density * 255.).round() as u8])
    })
}

fn smoothstep(lo: f32, hi: f32, value: f32) -> f32 {
    let t = ((value - lo) / (hi - lo)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

struct SurfaceDab {
    center: Vec3,
    normal: Vec3,
    tangent: Vec3,
    radius: f32,
    extent: f32,
}

fn surface_dab(
    coverage: &mut StrokeCoverage,
    triangle: &Attributes,
    owns: impl Fn(usize) -> bool,
    dab: SurfaceDab,
    falloff: impl Fn(Vec2) -> f32,
    stencil: impl Fn(Vec3) -> f32,
) {
    let SurfaceDab {
        center,
        normal,
        tangent,
        radius,
        extent,
    } = dab;
    let bitangent = normal.cross(tangent);
    let [a, b, c] = triangle.points;
    let [u, v, w] = triangle.uv;
    let (ab, ac) = (b - a, c - a);
    let n = ab.cross(ac);
    let area = n.length_squared();
    if area <= 1e-20 {
        return;
    }
    // Project the stamp's enclosing sphere into UV to bound per-texel work.
    let u_gradient = ac.cross(n) / area;
    let v_gradient = n.cross(ab) / area;
    let target =
        u + (v - u) * (center - a).dot(u_gradient) + (w - u) * (center - a).dot(v_gradient);
    let gradient = |s: f32, t: f32| (u_gradient * s + v_gradient * t).length();
    let reach = Vec2::new(
        gradient(v.x - u.x, w.x - u.x),
        gradient(v.y - u.y, w.y - u.y),
    ) * (radius * extent);
    let min = u.min(v).min(w).max(target - reach);
    let max = u.max(v).max(w).min(target + reach);
    if !min.cmplt(max).all() {
        return;
    }
    coverage.stamp(Rect { min, max }, |uv, pixel| {
        if !owns(pixel) {
            return 0.;
        }
        let Some(weights) = super::super::projection::weights(uv, triangle.uv) else {
            return 0.;
        };
        let point = a * weights.x + b * weights.y + c * weights.z;
        let offset = point - center;
        let height = offset.dot(normal);
        if height.abs() > radius {
            return 0.;
        }
        let [a, b, c] = triangle.normals;
        let face = (a * weights.x + b * weights.y + c * weights.z).normalize_or_zero();
        falloff(Vec2::new(offset.dot(tangent), offset.dot(bitangent)) / radius)
            * smoothstep(0., 0.15, face.dot(normal))
            * stencil(point)
    });
}
