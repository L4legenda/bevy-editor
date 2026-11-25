//! Source images and projection settings for editable material fill layers.
use super::CHANNEL_COUNT;
#[path = "model_paint_fill_variation.rs"]
mod variation;
use crate::editor::model_document::MeshData;
use bevy::prelude::*;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{io::Cursor, path::Path, sync::Arc};
pub(in crate::editor) use variation::Variation;
pub(in crate::editor) use variation::CHANNELS as VARIATION_CHANNELS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum Projection {
    #[default]
    Uv,
    Triplanar,
    PlanarX,
    PlanarY,
    PlanarZ,
    Spherical,
    Cylindrical,
}

impl Projection {
    pub(in crate::editor) const ALL: [Self; 7] = [
        Self::Uv,
        Self::Triplanar,
        Self::PlanarX,
        Self::PlanarY,
        Self::PlanarZ,
        Self::Spherical,
        Self::Cylindrical,
    ];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::Uv => "UV",
            Self::Triplanar => "Трипланарная",
            Self::PlanarX => "Плоскость X",
            Self::PlanarY => "Плоскость Y",
            Self::PlanarZ => "Плоскость Z",
            Self::Spherical => "Сферическая",
            Self::Cylindrical => "Цилиндрическая",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(in crate::editor) struct Settings {
    pub projection: Projection,
    pub tiling: f32,
    pub rotation: f32,
    pub offset: [f32; 2],
    pub sharpness: f32,
    pub normal_strength: f32,
    pub height_strength: f32,
    pub height_offset: f32,
    pub opacity_source: OpacitySource,
    pub opacity_strength: f32,
    pub emission_tint: [f32; 3],
    pub specular_strength: f32,
    pub variation: Variation,
    pub variation_channels: [bool; 3],
    pub image_alpha_coverage: bool,
    pub graph_channels: [bool; CHANNEL_COUNT],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            projection: Projection::Uv,
            tiling: 1.,
            rotation: 0.,
            offset: [0.; 2],
            sharpness: 4.,
            normal_strength: 1.,
            height_strength: 1.,
            height_offset: 0.,
            opacity_source: OpacitySource::Auto,
            opacity_strength: 1.,
            emission_tint: [1.; 3],
            specular_strength: 1.,
            variation: Variation::default(),
            variation_channels: [true; 3],
            image_alpha_coverage: true,
            graph_channels: [true; CHANNEL_COUNT],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(crate = "rmcp::schemars"))]
pub(in crate::editor) enum OpacitySource {
    #[default]
    Auto,
    Alpha,
    Brightness,
    Red,
    Green,
    Blue,
}

impl OpacitySource {
    pub(in crate::editor) const ALL: [Self; 6] = [
        Self::Auto,
        Self::Alpha,
        Self::Brightness,
        Self::Red,
        Self::Green,
        Self::Blue,
    ];

    pub(in crate::editor) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Авто",
            Self::Alpha => "Альфа",
            Self::Brightness => "Яркость",
            Self::Red => "R",
            Self::Green => "G",
            Self::Blue => "B",
        }
    }

    fn value(self, rgba: Vec4, has_alpha: bool) -> f32 {
        let luma = || rgba.truncate().dot(Vec3::new(0.2126, 0.7152, 0.0722));
        match self {
            Self::Auto if has_alpha => rgba.w,
            Self::Auto | Self::Brightness => luma(),
            Self::Alpha => rgba.w,
            Self::Red => rgba.x,
            Self::Green => rgba.y,
            Self::Blue => rgba.z,
        }
    }
}

impl Settings {
    fn varied_channels(self) -> [bool; 3] {
        let enabled = self.variation.enabled();
        std::array::from_fn(|i| enabled[i] && self.variation_channels[i])
    }

    pub fn valid(self) -> bool {
        (0.05..=64.).contains(&self.tiling)
            && (-180.0..=180.0).contains(&self.rotation)
            && self.offset.iter().all(|v| (-1.0..=1.0).contains(v))
            && (1.0..=16.0).contains(&self.sharpness)
            && (0.0..=2.0).contains(&self.normal_strength)
            && (-10.0..=10.0).contains(&self.height_strength)
            && (-100.0..=100.0).contains(&self.height_offset)
            && (0.0..=1.0).contains(&self.opacity_strength)
            && self.emission_tint.iter().all(|v| (0.0..=1.0).contains(v))
            && (0.0..=2.0).contains(&self.specular_strength)
            && self.variation.valid()
    }
}

#[derive(Debug, PartialEq)]
pub(in crate::editor) struct Bitmap {
    pub name: String,
    pub bytes: Vec<u8>,
    mips: Vec<RgbaImage>,
    has_alpha: bool,
}

// A 4096² RGBA raster can take 64 MiB plus PNG filter and container bytes.
const MAX_BYTES: usize = 65 * 1024 * 1024;

impl Bitmap {
    pub(in crate::editor) fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    pub(in crate::editor) fn pixels(&self) -> &RgbaImage {
        &self.mips[0]
    }

    pub(in crate::editor) fn preview_pixels(&self) -> &RgbaImage {
        self.mips.iter().find(|image| image.width().max(image.height()) <= 144)
            .unwrap_or_else(|| self.pixels())
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Self::decode(name, &bytes)
    }

    pub fn decode(name: String, bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Размер изображения превышает 65 МиБ".into());
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| error.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let pixels = reader
            .decode()
            .map_err(|error| error.to_string())?
            .into_rgba8();
        Ok(Self::with_pixels(name, bytes.to_vec(), pixels))
    }

    pub(in crate::editor) fn from_rgba(name: String, pixels: RgbaImage) -> Result<Self, String> {
        let bytes = encode_png(&pixels)?;
        if bytes.len() > MAX_BYTES {
            return Err("Размер изображения превышает 65 МиБ".into());
        }
        Ok(Self::with_pixels(name, bytes, pixels))
    }

    fn with_pixels(name: String, bytes: Vec<u8>, pixels: RgbaImage) -> Self {
        let has_alpha = pixels.pixels().any(|pixel| pixel[3] != 255);
        let mut mips = vec![pixels];
        loop {
            let level = mips.last().unwrap();
            let (w, h) = level.dimensions();
            if w == 1 && h == 1 {
                break;
            }
            mips.push(image::imageops::resize(
                level,
                (w / 2).max(1),
                (h / 2).max(1),
                image::imageops::FilterType::Triangle,
            ));
        }
        Self {
            name,
            bytes,
            mips,
            has_alpha,
        }
    }

    pub(in crate::editor) fn sample(&self, uv: Vec2, footprint: f32) -> Vec4 {
        self.sample_filtered(uv, footprint, true)
    }

    pub(in crate::editor) fn sample_clamped(&self, uv: Vec2, footprint: f32) -> Vec4 {
        self.sample_filtered(uv, footprint, false)
    }

    fn sample_filtered(&self, uv: Vec2, footprint: f32, repeat: bool) -> Vec4 {
        let max_side = self.mips[0].width().max(self.mips[0].height()) as f32;
        let lod = (footprint * max_side)
            .max(1.)
            .log2()
            .min((self.mips.len() - 1) as f32);
        let sample = |level: usize| {
            let image = &self.mips[level];
            let w = image.width() as i32;
            let h = image.height() as i32;
            let uv = if repeat { uv.rem_euclid(Vec2::ONE) } else { uv.clamp(Vec2::ZERO, Vec2::ONE) };
            let p = uv * Vec2::new(w as f32, h as f32) - Vec2::splat(0.5);
            let cell = p.floor();
            let f = p - cell;
            let (x, y) = (cell.x as i32, cell.y as i32);
            let at = |x: i32, y: i32| {
                let edge = |x: i32, size: i32| if repeat { x.rem_euclid(size) } else { x.clamp(0, size - 1) };
                Vec4::from_array(
                    image
                        .get_pixel(edge(x, w) as u32, edge(y, h) as u32)
                        .0
                        .map(f32::from),
                ) / 255.
            };
            at(x, y)
                .lerp(at(x + 1, y), f.x)
                .lerp(at(x, y + 1).lerp(at(x + 1, y + 1), f.x), f.y)
        };
        sample(lod.floor() as usize).lerp(sample(lod.ceil() as usize), lod.fract())
    }
}

pub(in crate::editor) fn encode_png(pixels: &RgbaImage) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            pixels.as_raw(), pixels.width(), pixels.height(), image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[derive(Clone, Debug)]
struct Stamp {
    mesh: MeshData,
    slot: usize,
    size: u32,
    offset: Vec2,
    transform: Transform,
    variation_base: [Option<Arc<Vec<u8>>>; 3],
}

type Channels = [Option<Arc<Vec<u8>>>; CHANNEL_COUNT];
pub(in crate::editor) type GraphImages = std::collections::BTreeMap<String, Arc<Bitmap>>;

#[derive(Clone, Debug, Default)]
pub(in crate::editor) struct Fill {
    pub settings: Settings,
    pub graph: Option<Arc<serde_json::Value>>,
    pub graph_images: Arc<GraphImages>,
    pub images: [Option<Arc<Bitmap>>; CHANNEL_COUNT],
    pub sampled: [Option<Arc<Vec<u8>>>; CHANNEL_COUNT],
    stamp: Option<Stamp>,
}

impl PartialEq for Fill {
    fn eq(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.graph == other.graph
            && (Arc::ptr_eq(&self.graph_images, &other.graph_images)
                || self.graph_images == other.graph_images)
            && self
                .images
                .iter()
                .zip(&other.images)
                .all(|(a, b)| match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                })
    }
}

impl Fill {
    pub fn invalidate(&mut self) {
        self.stamp = None;
    }

    pub fn matches(
        &self,
        mesh: &MeshData,
        slot: usize,
        size: u32,
        offset: Vec2,
        transform: Transform,
        base: &Channels,
    ) -> bool {
        let varied = self.settings.varied_channels();
        self.stamp.as_ref().is_some_and(|s| {
            s.size == size
                && ((self.settings.projection == Projection::Uv && !varied.contains(&true))
                    || (s.mesh.shares_storage_with(mesh)
                        && s.slot == slot
                        && s.offset == offset
                        && s.transform == transform))
                && variation::CHANNELS
                    .into_iter()
                    .zip(varied)
                    .zip(&s.variation_base)
                    .all(|((channel, enabled), cached)| {
                        !enabled
                            || self.images[channel].is_some()
                            || match (&base[channel], cached) {
                                (None, None) => true,
                                (Some(current), Some(cached)) => Arc::ptr_eq(current, cached),
                                _ => false,
                            }
                    })
        })
    }

    pub fn bind(
        &mut self,
        mesh: &MeshData,
        slot: usize,
        size: u32,
        offset: Vec2,
        transform: Transform,
        base: &Channels,
    ) {
        if self.matches(mesh, slot, size, offset, transform, base) {
            return;
        }
        let mut output = self.images.each_ref().map(|image| {
            image
                .as_ref()
                .map(|_| vec![0; size as usize * size as usize * 4])
        });
        let projection = self.settings.projection;
        let tiling = self.settings.tiling;
        let slide = Vec2::from_array(self.settings.offset);
        let rotation = Vec2::from_angle(self.settings.rotation.to_radians());
        let rotate = |point: Vec2| rotation.rotate(point);
        let centered = |uv: Vec2| rotate(uv - Vec2::splat(0.5)) * tiling + Vec2::splat(0.5) + slide;
        let varied = self.settings.varied_channels();
        let positions: Vec<_> = if projection == Projection::Uv && !varied.contains(&true) {
            Vec::new()
        } else {
            mesh.positions
                .iter()
                .map(|&p| transform.transform_point(p))
                .collect()
        };
        let (min, max) = positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        let extent = (max - min).max_element().max(1e-4);
        let center = (min + max) * 0.5;
        let mut write = |index: usize,
                         q: [Vec2; 3],
                         weights: Vec3,
                         footprint: f32,
                         axes: Option<(Vec3, Vec3)>,
                         normal: Vec3| {
            for (channel, (source, target)) in self.images.iter().zip(&mut output).enumerate() {
                let Some((source, target)) = source.as_ref().zip(target.as_mut()) else {
                    continue;
                };
                let mut samples = [source.sample(q[0], footprint), Vec4::ZERO, Vec4::ZERO];
                if projection == Projection::Triplanar {
                    samples[1] = source.sample(q[1], footprint);
                    samples[2] = source.sample(q[2], footprint);
                }
                let mixed =
                    samples[0] * weights.x + samples[1] * weights.y + samples[2] * weights.z;
                let bytes = if channel == 6 {
                    let slope = |sample: Vec4| {
                        let n = sample.truncate() * 2. - 1.;
                        Vec2::new(rotation.x, -rotation.y)
                            .rotate(Vec2::new(-n.x, n.y) / n.z.max(0.2))
                    };
                    let a = slope(samples[0]);
                    let gradient = match projection {
                        Projection::Triplanar => {
                            let b = slope(samples[1]);
                            let c = slope(samples[2]);
                            Some(
                                Vec3::new(0., -a.y, a.x) * weights.x
                                    + Vec3::new(b.x, 0., b.y) * weights.y
                                    + Vec3::new(c.x, -c.y, 0.) * weights.z,
                            )
                        }
                        Projection::PlanarX => Some(Vec3::new(0., -a.y, -a.x)),
                        Projection::PlanarY => Some(Vec3::new(a.x, 0., a.y)),
                        Projection::PlanarZ => Some(Vec3::new(a.x, -a.y, 0.)),
                        _ => None,
                    };
                    let a = gradient.zip(axes).map_or_else(
                        || slope(mixed),
                        |(gradient, (u, v))| {
                            let tangent = (u - normal * normal.dot(u))
                                .try_normalize()
                                .unwrap_or_else(|| normal.any_orthonormal_vector());
                            let bitangent = normal.cross(tangent);
                            Vec2::new(
                                gradient.dot(tangent),
                                gradient.dot(bitangent) * bitangent.dot(v).signum(),
                            )
                        },
                    ) * self.settings.normal_strength;
                    (Vec3::new(-a.x, a.y, 1.).normalize() * 0.5 + 0.5).extend(1.)
                } else if channel == 4 {
                    let opacity = self.settings.opacity_source.value(mixed, source.has_alpha);
                    Vec3::splat(opacity * self.settings.opacity_strength).extend(1.)
                } else if channel == 3 {
                    Vec3::splat(0.5 + (mixed.x - 0.5) * self.settings.height_strength
                        + self.settings.height_offset * 0.5).extend(1.)
                } else if channel == 5 {
                    (mixed.truncate() * Vec3::from_array(self.settings.emission_tint))
                        .extend(if self.settings.image_alpha_coverage { mixed.w } else { 1. })
                } else if channel == 7 {
                    Vec3::splat(mixed.x * self.settings.specular_strength).extend(1.)
                } else if channel == 0 {
                    if self.settings.image_alpha_coverage { mixed } else { mixed.truncate().extend(1.) }
                } else {
                    Vec3::splat(mixed.x).extend(1.)
                };
                target[index * 4..index * 4 + 4].copy_from_slice(
                    &bytes
                        .to_array()
                        .map(|v| (v.clamp(0., 1.) * 255.).round() as u8),
                );
            }
        };
        if projection == Projection::Uv && self.images.iter().any(Option::is_some) {
            for y in 0..size {
                for x in 0..size {
                    let uv = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
                    write(
                        (y * size + x) as usize,
                        [centered(uv); 3],
                        Vec3::X,
                        tiling / size as f32,
                        None,
                        Vec3::Z,
                    );
                }
            }
        } else if self.images.iter().any(Option::is_some) {
            let mut covered = vec![0; size as usize * size as usize];
            raster(
                mesh,
                slot,
                size,
                offset,
                transform,
                &positions,
                &mut covered,
                |index, _, p, n, axes, pixel_width| {
                    let (coords, weights, width) = if projection == Projection::Triplanar {
                        let (coords, weights) = triplanar_coordinates(p * tiling, n, self.settings.sharpness);
                        (
                            coords.map(|uv| rotate(uv) + slide),
                            weights,
                            pixel_width * tiling,
                        )
                    } else {
                        let d = (p - center) / extent;
                        let q = match projection {
                            Projection::PlanarX => Vec2::new(-d.z, -d.y) + 0.5,
                            Projection::PlanarY => Vec2::new(d.x, d.z) + 0.5,
                            Projection::PlanarZ => Vec2::new(d.x, -d.y) + 0.5,
                            Projection::Spherical => Vec2::new(
                                d.x.atan2(d.z) / std::f32::consts::TAU + 0.5,
                                (d.y / d.length().max(1e-6)).clamp(-1., 1.).acos()
                                    / std::f32::consts::PI,
                            ),
                            _ => Vec2::new(d.x.atan2(d.z) / std::f32::consts::TAU + 0.5, 0.5 - d.y),
                        };
                        ([centered(q); 3], Vec3::X, pixel_width * tiling / extent)
                    };
                    write(index, coords, weights, width, axes, n);
                },
            );
            pad(size as usize, &mut covered, &mut output);
        }
        if varied.contains(&true) {
            let mut changed: [Option<Vec<u8>>; CHANNEL_COUNT] = Default::default();
            for (channel, enabled) in variation::CHANNELS.into_iter().zip(varied) {
                if enabled {
                    changed[channel] = Some(output[channel].take().unwrap_or_else(|| {
                        base[channel].as_ref().map_or_else(
                            || {
                                let value = if channel == 0 { 255 } else { 128 };
                                [value, value, value, 255].repeat(size as usize * size as usize)
                            },
                            |plane| plane.as_ref().clone(),
                        )
                    }));
                }
            }
            let parts = (self.settings.variation.parts != 0.).then(|| mesh.vertex_components());
            let mut covered = vec![0; size as usize * size as usize];
            raster(
                mesh,
                slot,
                size,
                offset,
                transform,
                &positions,
                &mut covered,
                |index, vertex, position, _, _, _| {
                    let part = parts
                        .as_ref()
                        .map_or(0.5, |parts| variation::part_value(parts[vertex]));
                    let start = index * 4;
                    for channel in variation::CHANNELS {
                        if let Some(plane) = &mut changed[channel] {
                            self.settings.variation.apply(
                                channel,
                                &mut plane[start..start + 4],
                                position,
                                part,
                            );
                        }
                    }
                },
            );
            pad(size as usize, &mut covered, &mut changed);
            for channel in variation::CHANNELS {
                if changed[channel].is_some() {
                    output[channel] = changed[channel].take();
                }
            }
        }
        self.sampled = output.map(|plane| plane.map(Arc::new));
        self.stamp = Some(Stamp {
            mesh: mesh.clone(),
            slot,
            size,
            offset,
            transform,
            variation_base: std::array::from_fn(|i| {
                let channel = variation::CHANNELS[i];
                if varied[i] && self.images[channel].is_none() {
                    base[channel].clone()
                } else {
                    None
                }
            }),
        });
    }
}

pub(in crate::editor) fn triplanar_coordinates(point: Vec3, normal: Vec3, sharpness: f32) -> ([Vec2; 3], Vec3) {
    let [x, y, z] = point.to_array();
    let weight = normal.abs().powf(sharpness);
    ([Vec2::new(z, -y), Vec2::new(x, z), Vec2::new(x, -y)], weight / weight.element_sum().max(1e-4))
}

pub(in crate::editor) fn raster(
    mesh: &MeshData,
    slot: usize,
    size: u32,
    offset: Vec2,
    transform: Transform,
    positions: &[Vec3],
    covered: &mut [u8],
    mut write: impl FnMut(usize, usize, Vec3, Vec3, Option<(Vec3, Vec3)>, f32),
) {
    let extent = Vec2::splat(size as f32);
    let mut smooth = vec![Vec3::ZERO; mesh.positions.len()];
    for face in mesh.faces.iter().filter(|face| face.smooth) {
        let normal = mesh.normal(face);
        for corner in &face.corners {
            smooth[corner.vertex] += normal;
        }
    }
    for face in mesh.faces.iter().filter(|face| face.material == slot) {
        for tri in mesh.triangles(face) {
            let corners = tri.map(|i| &face.corners[i]);
            let uv = corners.map(|corner| (corner.uv - offset) * extent);
            let p = corners.map(|corner| positions[corner.vertex]);
            let [a, b, c] = p;
            let Some(geometric) = (b - a).cross(c - a).try_normalize() else {
                continue;
            };
            let n = corners.map(|corner| {
                let normal = corner
                    .source_normal
                    .or_else(|| face.smooth.then(|| smooth[corner.vertex]));
                normal
                    .and_then(|n| (transform.rotation * (n / transform.scale)).try_normalize())
                    .unwrap_or(geometric)
            });
            let du = uv[1] - uv[0];
            let dv = uv[2] - uv[0];
            let det = du.perp_dot(dv);
            if det.abs() < 1e-10 {
                continue;
            }
            let tangent = ((b - a) * dv.y - (c - a) * du.y) / det;
            let bitangent = ((c - a) * du.x - (b - a) * dv.x) / det;
            let pixel_width = tangent.length().max(bitangent.length());
            let min = uv[0]
                .min(uv[1])
                .min(uv[2])
                .floor()
                .clamp(Vec2::ZERO, extent)
                .as_uvec2();
            let max = uv[0]
                .max(uv[1])
                .max(uv[2])
                .ceil()
                .clamp(Vec2::ZERO, extent)
                .as_uvec2();
            for y in min.y..max.y {
                for x in min.x..max.x {
                    let index = (y * size + x) as usize;
                    if covered[index] != 0 {
                        continue;
                    }
                    let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - uv[0];
                    let v = q.perp_dot(dv) / det;
                    let w = du.perp_dot(q) / det;
                    let u = 1. - v - w;
                    if u.min(v).min(w) < -1e-5 {
                        continue;
                    }
                    covered[index] = 1;
                    let normal = (n[0] * u + n[1] * v + n[2] * w)
                        .try_normalize()
                        .unwrap_or(geometric);
                    write(
                        index,
                        corners[0].vertex,
                        a * u + b * v + c * w,
                        normal,
                        Some((tangent, bitangent)),
                        pixel_width,
                    );
                }
            }
        }
    }
}

fn pad(size: usize, covered: &mut [u8], output: &mut [Option<Vec<u8>>; CHANNEL_COUNT]) {
    pad_texels(size, covered, |target, source| {
        for plane in output.iter_mut().flatten() {
            plane.copy_within(source * 4..source * 4 + 4, target * 4);
        }
    });
}

pub(in crate::editor) fn pad_texels(size: usize, covered: &mut [u8], mut copy: impl FnMut(usize, usize)) {
    for ring in 1..=2 {
        for y in 0..size {
            for x in 0..size {
                let p = y * size + x;
                if covered[p] != 0 {
                    continue;
                }
                let neighbors = [
                    (x > 0).then(|| p - 1),
                    (x + 1 < size).then(|| p + 1),
                    (y > 0).then(|| p - size),
                    (y + 1 < size).then(|| p + size),
                ];
                if let Some(source) = neighbors
                    .into_iter()
                    .flatten()
                    .find(|&i| (1..=ring).contains(&covered[i]))
                {
                    covered[p] = ring + 1;
                    copy(p, source);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    #[test]
    fn clamped_bitmap_edges_sample_only_nearest_border() {
        let pixels = RgbaImage::from_raw(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        let bitmap = Bitmap::from_rgba("Edges".into(), pixels).unwrap();
        assert_eq!(bitmap.sample(Vec2::ZERO, 0.).x, 0.5);
        assert_eq!(bitmap.sample_clamped(Vec2::ZERO, 0.).x, 0.);
        assert_eq!(bitmap.sample_clamped(Vec2::ONE, 0.).x, 1.);
    }

    #[test]
    fn fill_variation_tracks_connected_parts_seed_and_solid_values() {
        let geometry = MeshData::from(Geometry {
            positions: vec![
                Vec3::ZERO,
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                Vec3::X + Vec3::Z,
                Vec3::Y + Vec3::Z,
            ],
            faces: (0..2)
                .map(|part| Face {
                    corners: [Vec2::ZERO, Vec2::X, Vec2::Y]
                        .into_iter()
                        .enumerate()
                        .map(|(i, uv)| {
                            let mut corner = Corner::new(part * 3 + i);
                            corner.uv = (uv + Vec2::new(part as f32, 0.)) * 0.5;
                            corner
                        })
                        .collect(),
                    material: 0,
                    smooth: false,
                })
                .collect(),
            ..default()
        });
        let plane = |value| Arc::new([value, value, value, 201].repeat(16 * 16));
        let mut base = std::array::from_fn(|_| None);
        base[0] = Some(plane(100));
        base[1] = Some(plane(128));
        let mut fill = Fill::default();
        fill.settings.variation.parts = 0.6;
        let bind = |fill: &mut Fill, base: &Channels| {
            fill.bind(&geometry, 0, 16, Vec2::ZERO, Transform::IDENTITY, base);
        };
        bind(&mut fill, &base);
        let at = |fill: &Fill, channel: usize, x: usize, y: usize| {
            let index = (y * 16 + x) * 4;
            fill.sampled[channel].as_ref().unwrap()[index..index + 4].to_vec()
        };
        let a = at(&fill, 0, 1, 1);
        assert_eq!(a, at(&fill, 0, 2, 2));
        assert_ne!(a, at(&fill, 0, 10, 2));
        assert_eq!(a[3], 201);
        assert_ne!(at(&fill, 1, 1, 1), at(&fill, 1, 10, 2));
        assert!(fill.sampled[2].is_none() && fill.sampled[3].is_none());
        base[0] = Some(plane(74));
        bind(&mut fill, &base);
        assert_ne!(a, at(&fill, 0, 1, 1));

        fill.settings.variation.height = 0.8;
        fill.invalidate();
        bind(&mut fill, &base);
        let first = fill.sampled[3].clone().unwrap();
        assert!(first.chunks_exact(4).any(|p| p[0] != 128));
        fill.invalidate();
        bind(&mut fill, &base);
        assert!(first == fill.sampled[3].clone().unwrap());
        fill.settings.variation.seed = 2.75;
        fill.invalidate();
        bind(&mut fill, &base);
        assert!(first != fill.sampled[3].clone().unwrap());
    }
}
