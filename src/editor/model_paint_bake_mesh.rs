//! UV baking. Distances are relative to the visible mesh bounds.
use super::{BakeMap, IdSource, MapSelection, Progress, ProjectionSettings, Sampling, material_id_color};
use crate::editor::{
    model_document::Object,
    model_paint_data::custom::Plane,
    model_paint_data::mask::{BakedInputs, PartIds},
    model_uv_mesh::Atlas,
    ui::model_viewer::editing::input::{Bvh, PickTriangle},
};
use bevy::prelude::*;
use std::{collections::{BTreeSet, HashMap, VecDeque}, sync::{Arc, OnceLock, atomic::Ordering}};

#[path = "model_paint_bake_aa.rs"]
mod aa;
#[path = "model_paint_bake_custom.rs"]
mod custom;
#[path = "model_paint_bake_denoise.rs"]
mod denoise;
#[path = "model_paint_bake_padding.rs"]
mod padding;
#[path = "model_paint_bake_probe.rs"]
pub(super) mod probe;
#[path = "model_paint_bake_project.rs"]
mod project;
#[path = "model_paint_bake_relief.rs"]
mod relief;
#[path = "model_paint_bake_raster.rs"]
mod raster;
#[path = "model_paint_bake_rays_gpu.rs"]
mod rays_gpu;
pub(super) use rays_gpu::Device as RayDevice;
#[path = "model_paint_bake_live_mesh.rs"]
pub(super) mod live;

pub(super) use project::FitReport;
pub(super) const COMPONENTS: usize = 21;
pub(super) const GENERATOR_MAPS: [Option<BakeMap>; 10] = [
    Some(BakeMap::Occlusion), Some(BakeMap::Cavity), Some(BakeMap::Thickness), Some(BakeMap::Sky),
    Some(BakeMap::PartGradient), Some(BakeMap::PartRandom), None, Some(BakeMap::Curvature),
    Some(BakeMap::EdgeDistance), Some(BakeMap::CreaseDistance),
];

pub(in crate::editor::ui::model_viewer::paint) struct Maps {
    pub size: u32,
    pub filled: usize,
    pub(super) values: Vec<[u8; COMPONENTS]>,
    pub(super) covered: Vec<u8>,
    pub(super) projection: Option<Vec<[u8; 5]>>,
    pub hits: usize,
    pub filled_misses: usize,
    pub cage_fit: Option<FitReport>,
    pub gpu_rays: bool,
    pub gpu_projection: bool,
    pub gpu_denoise: bool,
    pub gpu_padding: bool,
    pub(super) selection: super::MapSelection,
    pub(super) edge_reach: f32,
    pub(super) inputs: OnceLock<Arc<BakedInputs>>,
    pub custom: Vec<Plane>,
}

impl Maps {
    pub(in crate::editor::ui::model_viewer::paint) fn island_texels(&self) -> impl Iterator<Item = bool> + '_ {
        self.covered.iter().map(|&coverage| coverage == 1)
    }

    pub fn has_generator_inputs(&self, existing: Option<&Arc<BakedInputs>>) -> bool {
        self.inputs
            .get()
            .zip(existing)
            .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
    }

    pub fn generator_inputs(&self) -> &Arc<BakedInputs> {
        self.inputs.get_or_init(|| self.build_generator_inputs(|_, _, value| value))
    }

    pub(super) fn build_generator_inputs(&self, mut remap: impl FnMut(usize, usize, u8) -> u8) -> Arc<BakedInputs> {
            Arc::new(BakedInputs {
                size: self.size,
                edge_reach: self.edge_reach,
                disabled_inputs: self.selection.disabled_inputs(),
                texels: self
                    .values
                    .iter()
                    .enumerate()
                    .map(|(pixel, v)| {
                        let values = [
                            v[0], v[1], v[2], v[3], v[5], v[4], v[10], v[18], v[19], v[20],
                        ];
                        std::array::from_fn(|channel| remap(channel, pixel, values[channel]))
                    })
                    .collect(),
                part_ids: self.selection.contains(BakeMap::PartId).then(|| PartIds::new(
                    self.values.iter().map(|v| [v[15], v[16], v[17]]).collect(),
                )),
                custom: self.custom.clone(),
            })
    }

    pub fn bytes(&self) -> usize {
        self.values.len() * COMPONENTS
            + self.covered.len()
            + self
                .projection
                .as_ref()
                .map_or(0, |values| values.len() * 5)
            + self
                .custom
                .iter()
                .map(|plane| plane.values.len())
                .sum::<usize>()
    }

    pub fn has_map(&self, map: BakeMap) -> bool {
        if let BakeMap::Custom(id) = map {
            return self.custom.iter().any(|plane| plane.recipe.id == id);
        }
        self.selection.contains(map) && (!map.requires_high_poly() || self.projection.is_some())
    }

    pub(in crate::editor::ui::model_viewer::paint) fn projected_tangent_normals(
        &self,
    ) -> Option<impl Fn(usize) -> Option<[u8; 3]> + '_> {
        if !self.selection.contains(BakeMap::ProjectedNormal) {
            return None;
        }
        let projection = self.projection.as_deref()?;
        Some(move |index: usize| {
            (self.covered[index] != 0).then(|| {
                let [x, y, z, ..] = projection[index];
                [x, y, z]
            })
        })
    }

    pub fn rgba(&self, map: BakeMap) -> Vec<u8> {
        if let BakeMap::Custom(id) = map {
            return self
                .custom
                .iter()
                .find(|plane| plane.recipe.id == id)
                .map_or_else(Vec::new, |plane| {
                    plane
                        .values
                        .iter()
                        .zip(&self.covered)
                        .flat_map(|(&v, &coverage)| [v, v, v, if coverage == 0 { 0 } else { 255 }])
                        .collect()
                });
        }
        if map.requires_high_poly() {
            return self
                .projection
                .as_ref()
                .map_or_else(Vec::new, |projection| {
                    projection
                        .iter()
                        .zip(&self.covered)
                        .flat_map(|(&value, &covered)| {
                            let [r, g, b] = match map {
                                BakeMap::ProjectedHeight => [value[3]; 3],
                                BakeMap::ProjectionCoverage => [value[4]; 3],
                                _ => [value[0], value[1], value[2]],
                            };
                            [r, g, b, if covered == 0 { 0 } else { 255 }]
                        })
                        .collect()
                });
        }
        let (offset, scalar) = Self::component(map);
        let mut rgba = Vec::with_capacity(self.values.len() * 4);
        for (pixel, &covered) in self.values.iter().zip(&self.covered) {
            let alpha = if covered == 0 { 0 } else { 255 };
            let rgb = if scalar {
                [pixel[offset]; 3]
            } else {
                [pixel[offset], pixel[offset + 1], pixel[offset + 2]]
            };
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
        }
        rgba
    }

    fn component(map: BakeMap) -> (usize, bool) {
        match map {
            BakeMap::Occlusion => (0, true),
            BakeMap::Cavity => (1, true),
            BakeMap::Thickness => (2, true),
            BakeMap::Sky => (3, true),
            BakeMap::PartRandom => (4, true),
            BakeMap::PartGradient => (5, true),
            BakeMap::BentNormal => (6, false),
            BakeMap::WorldNormal => (9, false),
            BakeMap::Position => (12, false),
            BakeMap::PartId => (15, false),
            BakeMap::Curvature => (18, true),
            BakeMap::EdgeDistance => (19, true),
            BakeMap::CreaseDistance => (20, true),
            _ => unreachable!(),
        }
    }

    pub(in crate::editor::ui::model_viewer::paint) fn graph_component(&self, map: BakeMap, channel: usize) -> Option<impl Fn(usize) -> u8 + '_> {
        if channel > 2 {
            return None;
        }
        if !self.has_map(map) {
            return None;
        }
        let (bytes, stride, component) = match map {
            BakeMap::Custom(id) => (&self.custom.iter().find(|plane| plane.recipe.id == id)?.values[..], 1, 0),
            BakeMap::ProjectedHeight => (self.projection.as_ref()?.as_flattened(), 5, 3),
            BakeMap::ProjectedNormal => (self.projection.as_ref()?.as_flattened(), 5, channel),
            BakeMap::ProjectionCoverage => (self.projection.as_ref()?.as_flattened(), 5, 4),
            _ => {
                let (offset, scalar) = Self::component(map);
                (self.values.as_flattened(), COMPONENTS, offset + if scalar { 0 } else { channel })
            }
        };
        Some(move |pixel| bytes[pixel * stride + component])
    }

    pub(in crate::editor::ui::model_viewer::paint) fn edge_reach(&self) -> f32 {
        self.edge_reach
    }

    fn pad(&mut self, progress: &Progress, device: Option<&RayDevice>) -> Result<(), String> {
        let size = self.size as usize;
        let offsets = if let Some(device) = device {
            padding::offsets(device, size, &self.covered, progress)?
        } else {
            None
        };
        let copy_texel = |i: usize, other: usize| {
            // Copy the same texel in all bake outputs at UV seams.
            self.values[i] = self.values[other];
            if let Some(projection) = &mut self.projection {
                projection[i] = projection[other];
            }
            for plane in &mut self.custom {
                let values = Arc::get_mut(&mut plane.values).unwrap();
                values[i] = values[other];
            }
        };
        if let Some(offsets) = offsets {
            offsets.apply(&mut self.covered, progress, copy_texel)?;
            self.gpu_padding = true;
            Ok(())
        } else {
            pad_uv(size, &mut self.covered, progress, copy_texel)
        }
    }
}

fn pad_uv(
    size: usize,
    covered: &mut [u8],
    progress: &Progress,
    mut copy_texel: impl FnMut(usize, usize),
) -> Result<(), String> {
    for previous in 1..=2 {
        for y in 0..size {
            progress.check()?;
            for x in 0..size {
                let i = y * size + x;
                if covered[i] != 0 {
                    continue;
                }
                let neighbors = [
                    (x > 0).then(|| i - 1),
                    (y > 0).then(|| i - size),
                    (x + 1 < size).then(|| i + 1),
                    (y + 1 < size).then(|| i + size),
                ];
                if let Some(other) = neighbors.into_iter().flatten()
                    .find(|&other| (1..=previous).contains(&covered[other]))
                {
                    copy_texel(i, other);
                    covered[i] = previous + 1;
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    min: Vec3,
    max: Vec3,
}
impl Default for Bounds {
    fn default() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }
}
impl Bounds {
    pub(super) fn of_scene(objects: &[Object]) -> Option<Self> {
        let mut bounds = Self::default();
        for object in objects.iter().filter(|object| !object.hidden) {
            for face in &object.geometry.faces {
                for corner in &face.corners {
                    bounds.add(object.transform.transform_point(object.geometry.positions[corner.vertex]));
                }
            }
        }
        (bounds.min.is_finite() && bounds.max.is_finite()).then_some(bounds)
    }

    pub(super) fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub(super) fn custom_unit(self) -> f32 {
        ((self.max - self.min).max_element() * 0.5).max(1e-6)
    }

    pub(super) fn radius(self) -> f32 {
        ((self.max - self.min).length() * 0.5).max(1e-6)
    }

    fn add(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    fn normalize(&self, p: Vec3) -> Vec3 {
        let extent = self.max - self.min;
        let normalized = (p - self.min) / extent.max(Vec3::splat(1e-8));
        Vec3::select(
            extent.cmple(Vec3::splat(1e-8)),
            Vec3::splat(0.5),
            normalized,
        )
    }
}

fn root(parents: &mut [usize], mut v: usize) -> usize {
    while v != parents[v] {
        parents[v] = parents[parents[v]];
        v = parents[v];
    }
    v
}

struct Parts {
    faces: Vec<usize>,
    bounds: Vec<Bounds>,
}
impl Parts {
    fn build(object: &Object) -> Self {
        let mesh = &object.geometry;
        let components = mesh.vertex_components();
        let mut bounds = vec![Bounds::default(); mesh.positions.len()];
        for (v, &p) in mesh.positions.iter().enumerate() {
            bounds[components[v]].add(object.transform.transform_point(p));
        }
        let faces = mesh
            .faces
            .iter()
            .map(|face| components[face.corners[0].vertex])
            .collect();
        Self { faces, bounds }
    }
}

struct Rays<'a> {
    scene: &'a Bvh,
    scale: f32,
    sampling: Sampling,
    maps: MapSelection,
    resolution: u32,
    flow_normals: Option<custom::FlowNormals<'a>>,
}

#[derive(Clone, Copy)]
struct RaySurface {
    point: Vec3,
    normal: Vec3,
    geometric: Vec3,
    object: usize,
    face: usize,
    part: usize,
    bounds: Bounds,
}

impl RaySurface {
    fn projected(scene: &Bvh, parts: &project::HighRelief, hit: project::Hit) -> Self {
        let (part, bounds) = parts.component(hit.triangle);
        Self {
            point: hit.point,
            normal: hit.normal,
            geometric: (hit.geometric + hit.normal)
                .try_normalize()
                .unwrap_or(hit.geometric),
            object: scene.triangle(hit.triangle).object,
            face: hit.triangle,
            part,
            bounds,
        }
    }
}

fn world_normal(transform: Transform) -> impl Fn(Vec3) -> Vec3 {
    let inverse_scale = transform.scale.signum() / transform.scale.abs().max(Vec3::splat(1e-6));
    move |normal| transform.rotation * (normal * inverse_scale).normalize_or_zero()
}

fn mesh_normals(
    object: &Object,
    progress: &Progress,
) -> Result<(Vec<Vec3>, Vec<Vec<Vec3>>), String> {
    let mesh = &object.geometry;
    let mut normals = vec![Vec3::ZERO; mesh.positions.len()];
    for face in mesh.faces.iter().filter(|face| face.smooth) {
        progress.check()?;
        let normal = mesh.normal(face);
        for corner in &face.corners {
            normals[corner.vertex] += normal;
        }
    }
    let normal_to_world = world_normal(object.transform);
    let mut geometric = Vec::with_capacity(mesh.faces.len());
    let mut corners = Vec::with_capacity(mesh.faces.len());
    for face in &mesh.faces {
        progress.check()?;
        let normal = mesh.normal(face);
        geometric.push(normal_to_world(normal));
        corners.push(
            face.corners
                .iter()
                .map(|corner| {
                    normal_to_world(corner.source_normal.unwrap_or(if face.smooth {
                        normals[corner.vertex]
                    } else {
                        normal
                    }))
                })
                .collect(),
        );
    }
    Ok((geometric, corners))
}

fn projection_directions(
    object: &Object,
    positions: &[Vec3],
    generated: bool,
    authored: Option<&[Vec<Vec3>]>,
    progress: &Progress,
) -> Result<Vec<Vec3>, String> {
    let mesh = &object.geometry;
    let to_world = world_normal(object.transform);
    let mut directions = vec![Vec3::ZERO; positions.len()];
    for (index, face) in mesh.faces.iter().enumerate() {
        progress.check()?;
        let world = to_world(mesh.normal(face));
        if let Some(normals) = authored {
            for (corner, &normal) in face.corners.iter().zip(&normals[index]) {
                directions[corner.vertex] += normal.try_normalize().unwrap_or(world);
            }
        } else if generated {
            for triangle in mesh.triangles(face) {
                let indices = triangle.map(|i| face.corners[i].vertex);
                let points = indices.map(|vertex| positions[vertex]);
                let Some(mut normal) = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .try_normalize()
                else {
                    continue;
                };
                if normal.dot(world) < 0. {
                    normal = -normal;
                }
                for i in 0..3 {
                    let a = (points[(i + 1) % 3] - points[i]).normalize_or_zero();
                    let b = (points[(i + 2) % 3] - points[i]).normalize_or_zero();
                    directions[indices[i]] += normal * a.dot(b).clamp(-1., 1.).acos();
                }
            }
        } else {
            for corner in &face.corners {
                directions[corner.vertex] += world;
            }
        }
    }
    if generated && authored.is_none() {
        let key = |p: Vec3| p.to_array().map(|v| if v == 0. { 0 } else { v.to_bits() });
        let mut sums = std::collections::HashMap::<[u32; 3], Vec3>::new();
        for (i, (&point, &direction)) in positions.iter().zip(&directions).enumerate() {
            if i % 4096 == 0 {
                progress.check()?;
            }
            *sums.entry(key(point)).or_default() += direction;
        }
        for (direction, &point) in directions.iter_mut().zip(positions) {
            *direction = sums[&key(point)];
        }
    }
    Ok(directions)
}

fn triangle_ray_directions(
    vertices: [usize; 3],
    normals: [Vec3; 3],
    averaged: Option<&[Vec3]>,
    geometric: Vec3,
) -> [Vec3; 3] {
    let directions = averaged.map_or(normals, |values| {
        std::array::from_fn(|i| values.get(vertices[i]).copied().unwrap_or(normals[i]))
    });
    std::array::from_fn(|i| {
        directions[i].try_normalize().or_else(|| normals[i].try_normalize()).unwrap_or(geometric)
    })
}

fn projection_cage(
    projector: &project::Projection,
    object: &Object,
    positions: &[Vec3],
    directions: &[Vec3],
    scale: f32,
    settings: &ProjectionSettings,
    progress: &Progress,
) -> Result<(Option<project::Cage>, Option<project::FitReport>), String> {
    if let Some(path) = &settings.cage {
        return Ok((
            Some(project::Cage::load(path, object, positions, progress)?),
            None,
        ));
    }
    let (cage, report) = if settings.auto_fit {
        let (cage, report) = projector.fit_cage(positions, directions, scale, progress)?;
        (Some(cage), Some(report))
    } else {
        (None, None)
    };
    let painted = settings
        .manual
        .as_ref()
        .map(|manual| {
            projector.paint_cage(cage.as_ref(), directions, &manual.values, scale, progress)
        })
        .transpose()?
        .flatten();
    Ok((painted.or(cage), report))
}

fn byte(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

fn rgb(vector: Vec3) -> [u8; 3] {
    vector.to_array().map(byte)
}

fn triangle_id_color(colors: [Option<Vec3>; 3]) -> Option<[u8; 3]> {
    colors.into_iter().sum::<Option<Vec3>>().map(|sum| rgb(sum / 3.))
}

#[derive(PartialEq, Eq, Hash)]
enum MaterialKey {
    Shared(u64, Option<u32>),
    Local(usize, usize),
}

fn low_material_color(objects: &[Object], target: usize, slot: usize) -> Option<[u8; 3]> {
    let mut palette = HashMap::new();
    let mut selected = None;
    for object in objects.iter().filter(|object| !object.hidden) {
        let used: BTreeSet<_> = object.geometry.faces.iter().map(|face| face.material).collect();
        for index in used {
            let key = object.materials.get(index)
                .and_then(|slot| slot.source["material"].as_u64().map(|id| {
                    MaterialKey::Shared(id, slot.udim().map(|tile| tile.number()))
                }))
                .unwrap_or(MaterialKey::Local(object.id, index));
            let next = palette.len() as u32;
            let id = *palette.entry(key).or_insert(next);
            if (object.id, index) == (target, slot) {
                selected = Some(id);
            }
        }
    }
    selected.filter(|_| palette.len() > 1).map(material_id_color)
}

fn two_texel_width(points: [Vec3; 3], uv: [Vec2; 3]) -> f32 {
    let a = uv[1] - uv[0];
    let b = uv[2] - uv[0];
    let area = a.perp_dot(b).abs();
    if area <= 1e-10 {
        return 0.;
    }
    let ab = points[1] - points[0];
    let ac = points[2] - points[0];
    let du = ab * b.y - ac * a.y;
    let dv = ac * a.x - ab * b.x;
    2. * du.length().max(dv.length()) / area
}

fn hash(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846ca68b);
    value ^ (value >> 16)
}

fn hemisphere(normal: Vec3, sample: u32, count: u32, phase: f32) -> Vec3 {
    let radius = ((sample as f32 + 0.5) / count as f32).sqrt();
    let angle = phase + sample as f32 * 2.3999632;
    let (sin, cos) = angle.sin_cos();
    let tangent = normal.any_orthonormal_vector();
    let bitangent = normal.cross(tangent);
    normal * (1. - radius * radius).sqrt() + radius * (tangent * cos + bitangent * sin)
}

impl Rays<'_> {
    fn metadata(&self, surface: RaySurface) -> [u8; COMPONENTS] {
        let id = hash((surface.object as u32).wrapping_mul(0x9e3779b9) ^ surface.part as u32 ^ 1);
        let mut result = [0; COMPONENTS];
        result[..4].fill(255);
        result[4] = if self.maps.contains(BakeMap::PartRandom) { id as u8 } else { 128 };
        result[5] = if self.maps.contains(BakeMap::PartGradient) {
            byte(surface.bounds.normalize(surface.point).y)
        } else {
            128
        };
        result[9..12].copy_from_slice(&rgb(surface.normal * 0.5 + 0.5));
        let normal = [result[9], result[10], result[11]];
        result[6..9].copy_from_slice(&normal);
        if self.maps.contains(BakeMap::PartId) {
            let [_, r, g, b] = id.to_le_bytes();
            result[15..18].copy_from_slice(&[r, g, b].map(|value| 48 + value % 176));
        }
        result[18..21].copy_from_slice(&[128, 255, 255]);
        result
    }

    fn sample(
        &self,
        surface: RaySurface,
        pixel: usize,
        thickness_accepts: impl Fn(&PickTriangle) -> bool,
    ) -> [u8; COMPONENTS] {
        let mut result = self.metadata(surface);
        if !self.maps.visibility() {
            return result;
        }
        let ao_enabled = self.maps.contains(BakeMap::Occlusion);
        let bent_enabled = self.maps.contains(BakeMap::BentNormal);
        let broad_visibility = ao_enabled || bent_enabled;
        let cavity_enabled = self.maps.contains(BakeMap::Cavity);
        let thickness_enabled = self.maps.contains(BakeMap::Thickness) && self.sampling.thickness != 0;
        let sky_enabled = self.maps.contains(BakeMap::Sky) && self.sampling.sky != 0;
        let RaySurface {
            point: p,
            normal,
            geometric: geo,
            object,
            face,
            ..
        } = surface;
        let phase = (hash(pixel as u32) >> 8) as f32 * (std::f32::consts::TAU / 16777216.);
        let bias = self.scale * 0.00002;
        let outside = p + geo * bias;
        let other_face =
            |triangle: &PickTriangle| triangle.object != object || triangle.face != face;
        let mut ao = 0;
        let mut cavity = 0;
        let mut bent = Vec3::ZERO;
        let visibility_count = if broad_visibility || cavity_enabled { self.sampling.occlusion } else { 0 };
        let [ao_reach, cavity_reach, thickness_reach, cone_cos] = self.sampling.visibility;
        let ao_reach = self.scale * ao_reach;
        let cavity_reach = self.scale * cavity_reach;
        let visibility_reach = match (broad_visibility, cavity_enabled) {
            (true, true) => ao_reach.max(cavity_reach),
            (true, false) => ao_reach,
            _ => cavity_reach,
        };
        for i in 0..visibility_count {
            let mut direction = hemisphere(normal, i, self.sampling.occlusion, phase);
            if direction.dot(geo) < 0. {
                direction = (direction - 2. * direction.dot(geo) * geo).normalize_or_zero();
            }
            let distance = self
                .scene
                .cast_triangle_up_to(outside, direction, visibility_reach, other_face)
                .map_or(f32::INFINITY, |(_, distance)| distance);
            if distance > cavity_reach {
                cavity += 1;
            }
            if broad_visibility && distance > ao_reach {
                ao += 1;
                bent += direction;
            }
        }
        let reach = self.scale * thickness_reach;
        let mut thickness = 0.;
        let thickness_count = if thickness_enabled { self.sampling.thickness } else { 0 };
        for i in 0..thickness_count {
            let mut direction = hemisphere(-normal, i, self.sampling.thickness, phase + 1.);
            if direction.dot(geo) > 0. {
                direction = (direction - 2. * direction.dot(geo) * geo).normalize_or_zero();
            }
            thickness += self
                .scene
                .cast_triangle_up_to(p - geo * bias, direction, reach, |triangle| {
                    other_face(triangle) && thickness_accepts(triangle)
                })
                .map_or(reach, |(_, distance)| distance);
        }
        let mut sky = 0;
        let sky_count = if sky_enabled { self.sampling.sky } else { 0 };
        for i in 0..sky_count {
            let y = 1. - (1. - cone_cos) * ((i as f32 + 0.5) / self.sampling.sky as f32);
            let radius = (1. - y * y).sqrt();
            let (sin, cos) = (phase + i as f32 * 2.3999632).sin_cos();
            let direction = Vec3::new(radius * cos, y, radius * sin);
            if normal.dot(direction) > 0.
                && geo.dot(direction) > 0.
                && self
                    .scene
                    .cast_where(outside, direction, other_face)
                    .is_none()
            {
                sky += 1;
            }
        }
        if ao_enabled {
            result[0] = byte(ao as f32 / self.sampling.occlusion as f32);
        }
        if bent_enabled {
            result[6..9].copy_from_slice(&rgb(bent.try_normalize().unwrap_or(normal) * 0.5 + 0.5));
        }
        if cavity_enabled {
            result[1] = byte(cavity as f32 / self.sampling.occlusion as f32);
        }
        if thickness_enabled {
            result[2] = byte(thickness / (reach * self.sampling.thickness as f32));
        }
        if sky_enabled {
            result[3] = byte(sky as f32 / self.sampling.sky as f32);
        }
        result
    }
}

fn set_relief(texel: &mut [u8; COMPONENTS], sample: relief::Sample, selection: MapSelection) {
    let maps = [BakeMap::EdgeDistance, BakeMap::CreaseDistance];
    for ((output, value), map) in texel[19..21].iter_mut().zip(sample.distance).zip(maps) {
        if selection.contains(map) {
            *output = byte(value);
        }
    }
}

struct BakeGuides {
    surface: Vec<denoise::Guide>,
    curvature: Vec<relief::Curvature>,
}

fn neutral_texel(normal: Vec3) -> [u8; COMPONENTS] {
    let mut texel = [0; COMPONENTS];
    texel[..4].fill(255);
    let normal = rgb(normal * 0.5 + 0.5);
    texel[6..9].copy_from_slice(&normal);
    texel[9..12].copy_from_slice(&normal);
    texel[18..21].copy_from_slice(&[128, 255, 255]);
    texel
}

struct ProjectionPixel {
    pixel: usize,
    point: Vec3,
    normal: Vec3,
    width: f32,
    island: usize,
}

#[derive(Default)]
struct ProjectionWork {
    probes: Vec<aa::Probe>,
    pixels: Vec<(ProjectionPixel, aa::Group)>,
}

struct ProjectionPass<'a> {
    projector: &'a project::Projection,
    relief: &'a project::HighRelief,
    rays: &'a Rays<'a>,
    custom: &'a [custom::Prepared],
    bounds: Bounds,
    pattern: aa::SamplePattern,
    batches: Option<rays_gpu::ProjectBatch>,
    work: ProjectionWork,
    pending: VecDeque<(rays_gpu::ProjectionReadback, ProjectionWork)>,
    spare: Vec<ProjectionWork>,
    hits: Vec<Option<project::Hit>>,
    next_slot: usize,
}

impl ProjectionPass<'_> {
    fn write(
        &self,
        item: &ProjectionPixel,
        sample: Option<aa::Texel>,
        maps: &mut Maps,
        guides: &mut BakeGuides,
        batch: &mut Option<rays_gpu::Batch>,
        progress: &Progress,
    ) -> Result<(), String> {
        let pixel = item.pixel;
        let mut texel;
        let packed = if let Some(sample) = sample {
            let hit = sample.center;
            maps.hits += usize::from(hit.ray_hit);
            maps.filled_misses += usize::from(!hit.ray_hit);
            let surface = RaySurface::projected(self.rays.scene, self.relief, hit);
            if let Some(guide) = guides.surface.get_mut(pixel) {
                *guide = denoise::Guide::new(hit.point, surface.part, item.island, item.width);
            }
            texel = if let Some(batch) = batch.as_mut() {
                batch.push(surface, pixel, item.width);
                self.rays.metadata(surface)
            } else {
                self.rays.sample(surface, pixel, |_| true)
            };
            if maps.selection.contains(BakeMap::PartId)
                && let Some(color) = self.projector.id_color(hit.triangle)
            {
                texel[15..18].copy_from_slice(&color);
            }
            if batch.is_none() {
                for (recipe, plane) in self.custom.iter().zip(&mut maps.custom) {
                    Arc::get_mut(&mut plane.values).unwrap()[pixel] =
                        recipe.sample(self.rays, surface, item.width, &|triangle| {
                            self.relief.part(triangle.face) != surface.part
                        });
                }
            }
            if maps.selection.relief() {
                let sample = self.relief.sample(&hit, item.width);
                set_relief(&mut texel, sample, maps.selection);
                if let Some(curvature) = guides.curvature.get_mut(pixel) {
                    *curvature = sample.curvature;
                }
            }
            let mut packed = [128, 128, 255, 128, if hit.ray_hit { 255 } else { 0 }];
            if maps.selection.contains(BakeMap::ProjectedNormal) {
                packed[..3].copy_from_slice(&rgb(sample.normal * 0.5 + 0.5));
            }
            if maps.selection.contains(BakeMap::ProjectedHeight) {
                packed[3] = byte(sample.height * 0.5 + 0.5);
            }
            packed
        } else {
            texel = neutral_texel(item.normal);
            [128, 128, 255, 128, 0]
        };
        maps.projection.as_mut().unwrap()[pixel] = packed;
        texel[12..15].copy_from_slice(&rgb(self.bounds.normalize(item.point)));
        maps.values[pixel] = texel;
        if let Some(batch) = batch.as_mut() && batch.full() {
            let calculated = batch.flush(maps, progress, false)?;
            maps.gpu_rays |= calculated;
        }
        Ok(())
    }

    fn submit(
        &mut self, maps: &mut Maps, guides: &mut BakeGuides,
        batch: &mut Option<rays_gpu::Batch>, progress: &Progress,
    ) -> Result<(), String> {
        if self.work.probes.is_empty() {
            return Ok(());
        }
        let slots = self.batches.as_ref().unwrap().slots(progress);
        if self.pending.len() == slots {
            self.retire(maps, guides, batch, progress)?;
        }
        let readback = self.batches.as_mut().unwrap().submit(
            &self.work.probes, self.next_slot, progress,
        )?;
        self.next_slot = (self.next_slot + 1) % slots;
        let work = std::mem::replace(&mut self.work, self.spare.pop().unwrap_or_default());
        self.pending.push_back((readback, work));
        #[cfg(test)]
        progress.projection_peak.fetch_max(self.pending.len() as u32, Ordering::Relaxed);
        if slots == 1 {
            self.retire(maps, guides, batch, progress)?;
        }
        Ok(())
    }

    fn retire(
        &mut self, maps: &mut Maps, guides: &mut BakeGuides,
        batch: &mut Option<rays_gpu::Batch>, progress: &Progress,
    ) -> Result<(), String> {
        let (readback, mut work) = self.pending.pop_front().unwrap();
        self.batches.as_ref().unwrap().resolve(self.projector, readback, &mut self.hits, progress)?;
        for (item, group) in work.pixels.drain(..) {
            let texel = group.resolve(&work.probes, &self.hits);
            self.write(&item, texel, maps, guides, batch, progress)?;
        }
        work.probes.clear();
        self.spare.push(work);
        maps.gpu_projection = true;
        Ok(())
    }

    fn finish(
        &mut self, maps: &mut Maps, guides: &mut BakeGuides,
        batch: &mut Option<rays_gpu::Batch>, progress: &Progress,
    ) -> Result<(), String> {
        self.submit(maps, guides, batch, progress)?;
        while !self.pending.is_empty() {
            self.retire(maps, guides, batch, progress)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn bake(
    objects: Vec<Object>,
    target: usize,
    slot: usize,
    size: u32,
    progress: &Progress,
) -> Result<Maps, String> {
    bake_with_projection(
        objects,
        target,
        slot,
        size,
        &ProjectionSettings::default(),
        progress,
    )
}

struct PreparedScene {
    bounds: Bounds,
    scale: f32,
    geometric: Vec<Vec3>,
    corner_normals: Vec<Vec<Vec3>>,
    triangle_normals: Vec<[Vec3; 3]>,
    low_scene: Option<Bvh>,
    projection: Option<project::Projection>,
    parts: Option<Parts>,
    positions: Vec<Vec3>,
    projection_normals: Vec<Vec3>,
    cage: Option<project::Cage>,
    cage_fit: Option<FitReport>,
}

fn prepare_scene(
    objects: &[Object],
    object: &Object,
    settings: &ProjectionSettings,
    flow: bool,
    progress: &Progress,
) -> Result<PreparedScene, String> {
    let target = object.id;
    let mesh = &object.geometry;
    let mut bounds = Bounds::default();
    let mut triangles = Vec::new();
    let (geometric, corner_normals) = mesh_normals(object, progress)?;
    let mut triangle_normals = Vec::new();
    for visible in objects.iter().filter(|o| !o.hidden) {
        let other_normals = (flow && settings.path.is_none() && visible.id != target)
            .then(|| mesh_normals(visible, progress).map(|(_, corners)| corners))
            .transpose()?;
        let normals = if visible.id == target {
            Some(&corner_normals)
        } else {
            other_normals.as_ref()
        };
        for (face, polygon) in visible.geometry.faces.iter().enumerate() {
            progress.check()?;
            if settings.path.is_some() {
                for corner in &polygon.corners {
                    bounds.add(
                        visible
                            .transform
                            .transform_point(visible.geometry.positions[corner.vertex]),
                    );
                }
                continue;
            }
            for triangle in visible.geometry.triangles(polygon) {
                let points = triangle.map(|i| {
                    visible
                        .transform
                        .transform_point(visible.geometry.positions[polygon.corners[i].vertex])
                });
                for &p in &points {
                    bounds.add(p);
                }
                triangles.push(PickTriangle {
                    object: visible.id,
                    face,
                    points,
                });
                if flow && let Some(normals) = normals {
                    triangle_normals.push(triangle.map(|i| normals[face][i]));
                }
            }
        }
    }
    let scale = (bounds.max - bounds.min).length().max(1e-5);
    let projection = settings
        .path
        .as_ref()
        .map(|path| project::Projection::load(path, &object.name, scale, settings, progress))
        .transpose()?;
    let low_scene = projection.is_none().then(|| Bvh::from_triangles(triangles));
    let parts = projection.is_none().then(|| Parts::build(object));
    let transform = object.transform;
    let positions: Vec<_> = mesh
        .positions
        .iter()
        .map(|&point| transform.transform_point(point))
        .collect();
    let projection_normals = if projection.is_some() {
        projection_directions(
            object,
            &positions,
            settings.cage.is_none(),
            (!settings.average_normals).then_some(corner_normals.as_slice()),
            progress,
        )?
    } else {
        Vec::new()
    };
    let (cage, cage_fit) = projection
        .as_ref()
        .map(|projector| {
            projection_cage(
                projector,
                object,
                &positions,
                &projection_normals,
                scale,
                settings,
                progress,
            )
        })
        .transpose()?
        .unwrap_or_default();
    Ok(PreparedScene {
        bounds, scale, geometric, corner_normals, triangle_normals,
        low_scene, projection, parts, positions, projection_normals, cage, cage_fit,
    })
}

#[cfg(test)]
pub(super) fn bake_with_projection(
    objects: Vec<Object>,
    target: usize,
    slot: usize,
    size: u32,
    settings: &ProjectionSettings,
    progress: &Progress,
) -> Result<Maps, String> {
    bake_on_device(objects, target, slot, size, settings, progress, None)
}

pub(super) fn bake_on_device(
    objects: Vec<Object>,
    target: usize,
    slot: usize,
    size: u32,
    settings: &ProjectionSettings,
    progress: &Progress,
    device: Option<&RayDevice>,
) -> Result<Maps, String> {
    #[cfg(test)]
    let started = std::time::Instant::now();
    let object = objects
        .iter()
        .find(|o| o.id == target && !o.hidden)
        .ok_or("Объект скрыт")?;
    let mesh = &object.geometry;
    let extent = Vec2::splat(size as f32);
    let offset = object.uv_offset(slot);
    let mut triangles = Vec::new();
    for (face, polygon) in mesh.faces.iter().enumerate().filter(|(_, face)| face.material == slot) {
        progress.check()?;
        triangles.extend(mesh.triangles(polygon).into_iter().map(|corners| {
            let uv = corners.map(|i| (polygon.corners[i].uv - offset) * extent);
            raster::Triangle::new(face, corners, uv, size)
        }));
    }
    if triangles.is_empty() {
        return Err("В наборе нет граней".into());
    }
    let flow = settings.custom.iter().any(|recipe| recipe.kind == crate::editor::model_paint_data::custom::Kind::Flow);
    let PreparedScene {
        bounds, scale, geometric, corner_normals, triangle_normals,
        low_scene, projection, parts, positions, projection_normals, cage, cage_fit,
    } = prepare_scene(&objects, object, settings, flow, progress)?;
    let sampling = settings.sampling();
    let relief_params = relief::Parameters::new(bounds, scale, settings.surface);
    let material_color = if settings.maps.contains(BakeMap::PartId)
        && projection.is_none() && settings.id_source == IdSource::Materials
    {
        low_material_color(&objects, target, slot)
    } else {
        None
    };
    let low_rays = low_scene.as_ref().map(|scene| Rays {
        scene,
        scale,
        sampling,
        maps: settings.maps,
        resolution: size,
        flow_normals: flow.then_some(custom::FlowNormals::Mesh(&triangle_normals)),
    });
    let high_rays = projection.as_ref().map(|projector| Rays {
        scene: projector.scene(),
        scale,
        sampling,
        maps: settings.maps,
        resolution: size,
        flow_normals: flow.then_some(custom::FlowNormals::High(projector)),
    });
    let features = parts
        .as_ref()
        .filter(|_| settings.maps.relief())
        .map(|parts| {
            relief::Features::build(
                mesh,
                &positions,
                &corner_normals,
                &geometric,
                &parts.faces,
                relief_params,
                progress,
            )
        })
        .transpose()?;
    let source_relief = projection
        .as_ref()
        .map(|projector| projector.build_relief(relief_params, settings.maps.relief(), progress))
        .transpose()?;
    let count = (size * size) as usize;
    let custom: Vec<_> = settings
        .custom
        .iter()
        .map(|recipe| custom::Prepared::new(recipe, bounds))
        .collect();
    let visibility = settings.maps.visibility();
    let trace_maps = visibility || !custom.is_empty();
    let passes = sampling.denoise_passes();
    let use_resident = visibility;
    #[cfg(test)]
    let use_resident = use_resident && !progress.legacy_visibility_readback;
    let visibility_atlas = device
        .filter(|_| use_resident)
        .and_then(|device| denoise::visibility_atlas(device, size, passes));
    let mut batch = device.filter(|_| trace_maps || projection.is_some()).and_then(|device| {
        let labels = if let (Some(relief), Some(projector)) = (&source_relief, &projection) {
            rays_gpu::PartsOfScene::High(relief, projector)
        } else {
            rays_gpu::PartsOfScene::Low(parts.as_ref()?, target)
        };
        rays_gpu::Batch::new(device, high_rays.as_ref().or(low_rays.as_ref())?, labels, &custom, visibility, visibility_atlas.as_ref())
    });
    let visibility_atlas = visibility_atlas.filter(|_| batch.is_some());
    #[cfg(test)]
    progress.resident_visibility.store(visibility_atlas.is_some(), Ordering::Relaxed);
    let mut projected_pass = projection.as_ref().zip(source_relief.as_ref()).zip(high_rays.as_ref())
        .map(|((projector, relief), rays)| ProjectionPass {
            projector, relief, rays, bounds, custom: &custom,
            pattern: aa::SamplePattern::new(sampling.grid),
            batches: batch.as_ref().and_then(rays_gpu::Batch::project_batch),
            work: ProjectionWork::default(),
            pending: VecDeque::new(), spare: Vec::new(), hits: Vec::new(),
            next_slot: 0,
        });
    let islands = Atlas::with_offsets(mesh, |material| object.uv_offset(material)).islands;
    progress.check()?;
    if !trace_maps {
        batch = None;
    }
    let curve = settings.maps.contains(BakeMap::Curvature);
    let mut guides = BakeGuides {
        surface: vec![denoise::Guide::default(); if visibility || curve { count } else { 0 }],
        curvature: vec![relief::Curvature::default(); if curve { count } else { 0 }],
    };
    let mut maps = Maps {
        size,
        filled: 0,
        values: vec![[0; COMPONENTS]; count],
        covered: vec![0; count],
        projection: projection.as_ref().map(|_| vec![[0; 5]; count]),
        hits: 0,
        filled_misses: 0,
        cage_fit,
        gpu_rays: false,
        gpu_projection: false,
        gpu_denoise: false,
        gpu_padding: false,
        selection: settings.maps,
        edge_reach: relief_params.reach,
        inputs: OnceLock::new(),
        custom: settings
            .custom
            .iter()
            .cloned()
            .map(|recipe| Plane {
                recipe,
                values: Arc::new(vec![0; count]),
            })
            .collect(),
    };
    #[cfg(test)]
    let raster_start = std::time::Instant::now();
    let gpu_pixels = if let Some(device) = device.filter(|_| progress.gpu_raster) {
        let gpu_pixels = raster::pixels(device, size, &triangles, progress)?;
        #[cfg(test)]
        if gpu_pixels.is_some() {
            let _ = progress.gpu_raster_ms.set(raster_start.elapsed().as_secs_f64() * 1000.);
        }
        gpu_pixels
    } else {
        None
    };
    for (ordinal, triangle) in triangles.iter().enumerate() {
        progress.check()?;
        let face = triangle.face;
        let polygon = &mesh.faces[face];
        let geo = geometric[face];
        let corners = triangle.corners.map(|i| &polygon.corners[i]);
        let id_color = match settings.id_source {
            IdSource::Materials => material_color,
            IdSource::VertexColors if projection.is_none() => triangle_id_color(corners.map(|corner| corner.id_color)),
            _ => None,
        };
        let uv = triangle.uv;
        let points = corners.map(|corner| positions[corner.vertex]);
        let cage_triangle = cage
            .as_ref()
            .map(|cage| cage.triangle(corners.map(|corner| corner.vertex)));
        let normals = triangle.corners.map(|i| corner_normals[face][i]);
        let directions = triangle_ray_directions(
            corners.map(|corner| corner.vertex),
            normals,
            settings.average_normals.then_some(projection_normals.as_slice()),
            geo,
        );
        let axes = projection
            .as_ref()
            .and_then(|_| project::tangent_axes(points, uv));
        let projection_frame = axes.zip(triangle.barycentric).map(|(axes, barycentric)| aa::ProjectTriangle {
            barycentric,
            points,
            normals,
            directions,
            geometric: geo,
            axes,
            cage: cage_triangle.as_ref(),
        });
        let curvature = features
            .as_ref()
            .map(|features| Vec3::from_array(triangle.corners.map(|i| features.curvature[face][i])));
        let width = two_texel_width(points, uv);
        progress.rows.store(
            (ordinal * denoise::RASTER_END as usize / triangles.len()) as u32,
            Ordering::Relaxed,
        );
        let cpu_rows = if gpu_pixels.is_none() { triangle.lo.y..triangle.hi.y } else { 0..0 };
        let gpu_triangle = gpu_pixels.as_ref().map_or(&[][..], |pixels| pixels[ordinal].as_slice());
        let cpu_pixels = cpu_rows.flat_map(|y| (triangle.lo.x..triangle.hi.x).map(move |x| (x, y)));
        let pixels = cpu_pixels.chain(gpu_triangle.iter().map(|&i| (i % size, i / size)));
        let mut previous_y = None;
        for (x, y) in pixels {
            if previous_y != Some(y) {
                progress.check()?;
                previous_y = Some(y);
                let fraction = (y - triangle.lo.y) as f32 / (triangle.hi.y - triangle.lo.y) as f32;
                progress.rows.store(
                    ((ordinal as f32 + fraction) / triangles.len() as f32 * denoise::RASTER_END as f32) as u32,
                    Ordering::Relaxed,
                );
            }
            if flow || ((sampling.grid > 2 || !custom.is_empty()) && x % 32 == 0) {
                progress.check()?;
            }
            let pixel = (y * size + x) as usize;
            if maps.covered[pixel] != 0 {
                continue;
            }
            let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let Some(w) = triangle.weights(center)
            else {
                continue;
            };
            let p = points[0] * w.x + points[1] * w.y + points[2] * w.z;
            let normal = (normals[0] * w.x + normals[1] * w.y + normals[2] * w.z)
                .try_normalize()
                .unwrap_or(geo);
            if let Some(pass) = &mut projected_pass {
                let item = ProjectionPixel { pixel, point: p, normal, width, island: islands[face] };
                if let Some(frame) = &projection_frame
                    && let Some(gpu) = &pass.batches
                {
                    if pass.work.probes.len() + pass.pattern.max_probes() > gpu.capacity {
                        pass.submit(&mut maps, &mut guides, &mut batch, progress)?;
                    }
                    let group = frame.enqueue(pass.projector, center, w, &pass.pattern, &mut pass.work.probes);
                    pass.work.pixels.push((item, group));
                } else {
                    let result = projection_frame.as_ref().and_then(|frame| frame.sample(pass.projector, center, w, &pass.pattern));
                    pass.write(&item, result, &mut maps, &mut guides, &mut batch, progress)?;
                }
                maps.covered[pixel] = 1;
                maps.filled += 1;
                continue;
            }
            let mut texel;
            if let (Some(rays), Some(parts)) = (&low_rays, &parts) {
                let part = parts.faces[face];
                if let Some(guide) = guides.surface.get_mut(pixel) {
                    *guide = denoise::Guide::new(p, part, islands[face], width);
                }
                let surface = RaySurface {
                    point: p,
                    normal,
                    geometric: geo,
                    object: target,
                    face,
                    part,
                    bounds: parts.bounds[part],
                };
                let same_part = |triangle: &PickTriangle| {
                    triangle.object == target && parts.faces[triangle.face] == part
                };
                texel = if let Some(batch) = &mut batch {
                    batch.push(surface, pixel, width);
                    rays.metadata(surface)
                } else {
                    rays.sample(surface, pixel, same_part)
                };
                if batch.is_none() {
                    for (recipe, plane) in custom.iter().zip(&mut maps.custom) {
                        Arc::get_mut(&mut plane.values).unwrap()[pixel] =
                            recipe.sample(rays, surface, width, &|triangle| !same_part(triangle));
                    }
                }
                if let (Some(features), Some(curvature)) = (&features, curvature) {
                    let sample = features.sample(part, p, curvature.dot(w), width);
                    set_relief(&mut texel, sample, settings.maps);
                    if let Some(curvature) = guides.curvature.get_mut(pixel) {
                        *curvature = sample.curvature;
                    }
                }
            } else {
                texel = neutral_texel(normal);
            }
            if settings.maps.contains(BakeMap::PartId) && let Some(color) = id_color {
                texel[15..18].copy_from_slice(&color);
            }
            texel[12..15].copy_from_slice(&rgb(bounds.normalize(p)));
            maps.values[pixel] = texel;
            if let Some(batch) = &mut batch && batch.full() {
                let calculated = batch.flush(&mut maps, progress, false)?;
                maps.gpu_rays |= calculated;
            }
            maps.covered[pixel] = 1;
            maps.filled += 1;
        }
    }
    if let Some(pass) = &mut projected_pass {
        pass.finish(&mut maps, &mut guides, &mut batch, progress)?;
    }
    if maps.filled == 0 {
        return Err("Ни один тексель набора не покрыт UV-развёрткой".into());
    }
    if let Some(batch) = &mut batch {
        let calculated = batch.flush(&mut maps, progress, true)?;
        maps.gpu_rays |= calculated;
    }
    drop(projected_pass);
    drop(batch);
    #[cfg(test)]
    let denoise_start = std::time::Instant::now();
    if visibility {
        denoise::filter(&mut maps, &guides.surface, passes, progress, device, visibility_atlas.as_ref())?;
    }
    drop(visibility_atlas);
    if curve {
        denoise::filter_curvature(&mut maps, &guides.surface, &guides.curvature, relief_params.radius, progress, device)?;
    }
    drop(guides);
    #[cfg(test)]
    let padding_start = std::time::Instant::now();
    maps.pad(progress, device)?;
    #[cfg(test)]
    if progress.profile {
        let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.;
        let _ = progress.stage_ms.set([
            ms(raster_start - started),
            ms(denoise_start - raster_start),
            ms(padding_start - denoise_start),
            ms(padding_start.elapsed()),
        ]);
    }
    progress.rows.store(1000, Ordering::Relaxed);
    Ok(maps)
}

#[cfg(test)]
#[path = "model_paint_bake_perf_tests.rs"]
mod perf_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    fn quad(id: usize, y: f32) -> Object {
        let mut mesh = Geometry::default();
        mesh.positions = [
            Vec3::new(0., y, 0.),
            Vec3::new(1., y, 0.),
            Vec3::new(1., y, -1.),
            Vec3::new(0., y, -1.),
        ]
        .into();
        mesh.faces.push(Face {
            corners: [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y]
                .into_iter()
                .enumerate()
                .map(|(index, uv)| {
                    let mut corner = Corner::new(index);
                    corner.uv = uv;
                    corner
                })
                .collect(),
            material: 0,
            smooth: false,
        });
        Object {
            id,
            source_node: None,
            name: format!("Plane {id}"),
            geometry: mesh.into(),
            transform: Transform::IDENTITY,
            materials: Vec::new(),
            hidden: false,
        }
    }

    #[test]
    fn vertex_ids_use_triangle_rgb_and_fall_back_for_uncolored_meshes() {
        let mut object = quad(0, 0.);
        let settings = ProjectionSettings { id_source: IdSource::VertexColors, ..default() };
        let progress = Progress::default();
        let bake = |object| bake_with_projection(vec![object], 0, 0, 16, &settings, &progress).unwrap();
        let original = bake(object.clone());
        let color = Vec3::new(0.25, 0.5, 0.75);
        for corner in &mut object.geometry.faces[0].corners {
            corner.id_color = Some(color);
        }
        let painted = bake(object);
        let center = (8 * 16 + 8) * 4;
        assert_eq!(&painted.rgba(BakeMap::PartId)[center..center + 3], &[64, 128, 191]);
        assert_ne!(painted.rgba(BakeMap::PartId), original.rgba(BakeMap::PartId));
        assert_eq!(painted.rgba(BakeMap::PartRandom), original.rgba(BakeMap::PartRandom));
        assert_eq!(triangle_id_color([Some(Vec3::X), Some(Vec3::Y), Some(Vec3::Z)]), Some([85; 3]));
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_projected_id_uses_glb_colors_without_changing_visible_vertex_colors() {
        use crate::editor::model_document::glb::Glb;
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice::new(
            app.world().resource::<bevy::render::renderer::RenderDevice>(),
            app.world().resource::<bevy::render::renderer::RenderQueue>(),
        );
        let path = std::env::temp_dir().join(format!("bevy-bake-vertex-id-{}.glb", bevy::asset::uuid::Uuid::new_v4()));
        let piece = |id, color| {
            let mut object = quad(id, 0.02);
            for position in &mut object.geometry.positions {
                position.x = (position.x + id as f32) * 0.5;
            }
            for corner in &mut object.geometry.faces[0].corners {
                corner.id_color = Some(color);
            }
            object
        };
        std::fs::write(&path, Glb::new_asset(&[piece(0, Vec3::X), piece(1, Vec3::Z)]).unwrap()).unwrap();
        let settings = ProjectionSettings {
            path: Some(path.clone()),
            id_source: IdSource::VertexColors,
            match_names: false,
            ..default()
        };
        let progress = Progress::default();
        let objects = vec![quad(0, 0.)];
        let cpu = bake_with_projection(objects.clone(), 0, 0, 16, &settings, &progress).unwrap();
        let gpu = bake_on_device(objects, 0, 0, 16, &settings, &progress, Some(&device)).unwrap();
        assert!(gpu.gpu_projection);
        let ids = cpu.rgba(BakeMap::PartId);
        assert_eq!(ids, gpu.rgba(BakeMap::PartId));
        for (column, expected) in [(3, [255, 0, 0]), (12, [0, 0, 255])] {
            let offset = (8 * 16 + column) * 4;
            assert_eq!(&ids[offset..offset + 3], &expected);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn material_ids_use_low_poly_slots_without_changing_the_mesh_part() {
        let mut object = quad(0, 0.);
        let mut other = object.geometry.faces[0].clone();
        other.material = 1;
        object.geometry.faces.push(other);
        let settings = ProjectionSettings { id_source: IdSource::Materials, ..Default::default() };
        let progress = Progress::default();
        let first = bake_with_projection(vec![object.clone()], 0, 0, 16, &settings, &progress).unwrap();
        let second = bake_with_projection(vec![object], 0, 1, 16, &settings, &progress).unwrap();
        let center = (8 * 16 + 8) * 4;
        assert_eq!(&first.rgba(BakeMap::PartId)[center..center + 3], &material_id_color(0));
        assert_eq!(&second.rgba(BakeMap::PartId)[center..center + 3], &material_id_color(1));
        assert_eq!(first.rgba(BakeMap::PartRandom), second.rgba(BakeMap::PartRandom));
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_projected_id_map_uses_high_poly_material_assignments() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice::new(
            app.world().resource::<bevy::render::renderer::RenderDevice>(),
            app.world().resource::<bevy::render::renderer::RenderQueue>(),
        );
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-material-ids-{}.obj",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        std::fs::write(&path, concat!(
            "o Source\n",
            "v 0 0.02 0\nv 0.5 0.02 0\nv 1 0.02 0\n",
            "v 0 0.02 -1\nv 0.5 0.02 -1\nv 1 0.02 -1\n",
            "usemtl Copper\nf 1 2 5 4\nusemtl Rubber\nf 2 3 6 5\n",
        )).unwrap();
        let settings = ProjectionSettings {
            path: Some(path.clone()),
            match_names: false,
            id_source: IdSource::Materials,
            ..Default::default()
        };
        let progress = Progress::default();
        let low = vec![quad(0, 0.)];
        let cpu = bake_with_projection(low.clone(), 0, 0, 16, &settings, &progress).unwrap();
        let gpu = bake_on_device(low.clone(), 0, 0, 16, &settings, &progress, Some(&device)).unwrap();
        assert!(gpu.gpu_projection);
        let ids = cpu.rgba(BakeMap::PartId);
        assert_eq!(ids, gpu.rgba(BakeMap::PartId));
        for (column, material) in [(3, 0), (12, 1)] {
            let offset = (8 * 16 + column) * 4;
            assert_eq!(&ids[offset..offset + 3], &material_id_color(material));
        }
        let parts = bake_with_projection(low, 0, 0, 16, &ProjectionSettings {
            id_source: IdSource::MeshParts,
            ..settings
        }, &progress).unwrap();
        assert_ne!(parts.rgba(BakeMap::PartId), ids);
        assert_eq!(parts.rgba(BakeMap::PartRandom), cpu.rgba(BakeMap::PartRandom));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_baked_projection_uses_face_priority_and_authored_directions() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice::new(
            app.world().resource::<bevy::render::renderer::RenderDevice>(),
            app.world().resource::<bevy::render::renderer::RenderQueue>(),
        );
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-ray-options-{}.obj",
            bevy::asset::uuid::Uuid::new_v4(),
        ));
        std::fs::write(&path, concat!(
            "o Stacked\n",
            "v 0 0 0\nv 1 0 0\nv 1 0 -1\nv 0 0 -1\n",
            "v 0 0.03 0\nv 1 0.03 0\nv 1 0.03 -1\nv 0 0.03 -1\n",
            "f 1 2 3 4\nf 8 7 6 5\n",
        )).unwrap();
        let mut low = quad(0, 0.);
        for corner in &mut low.geometry.faces[0].corners {
            corner.source_normal = Some(Vec3::X);
        }
        let settings = ProjectionSettings {
            path: Some(path.clone()),
            match_names: false,
            fill_misses: false,
            surface: crate::editor::model_bake_settings::SurfaceOptions {
                grid: Some(3),
                ..default()
            },
            ..Default::default()
        };
        let progress = Progress::default();
        let compare = |settings: &ProjectionSettings| {
            let cpu = bake_with_projection(vec![low.clone()], 0, 0, 16, settings, &progress).unwrap();
            let gpu = bake_on_device(vec![low.clone()], 0, 0, 16, settings, &progress, Some(&device)).unwrap();
            let center = (8 * 16 + 8) * 4;
            let cpu_height = cpu.rgba(BakeMap::ProjectedHeight)[center];
            let gpu_height = gpu.rgba(BakeMap::ProjectedHeight)[center];
            assert!(gpu.gpu_projection);
            assert_eq!(cpu.hits, gpu.hits);
            assert!(cpu_height.abs_diff(gpu_height) <= 1);
            (cpu.hits, cpu_height)
        };
        let (hits, floor) = compare(&settings);
        assert!(hits > 0);
        assert!(floor.abs_diff(128) <= 1);
        let (_, lid) = compare(&ProjectionSettings { prefer_front_faces: false, ..settings.clone() });
        assert!(lid > floor + 20);
        let maps = MapSelection::MINIMUM.toggle(BakeMap::ProjectedHeight);
        let height_only = ProjectionSettings { maps, prefer_front_faces: false, ..settings.clone() };
        assert_eq!(compare(&height_only).1, lid);
        let output = bake_on_device(vec![low.clone()], 0, 0, 16, &height_only, &progress, Some(&device)).unwrap();
        assert!(output.has_map(BakeMap::ProjectedHeight));
        assert!(output.has_map(BakeMap::ProjectionCoverage));
        assert!(!output.has_map(BakeMap::ProjectedNormal));
        assert!(output.projected_tangent_normals().is_none());
        assert_eq!(&output.projection.as_ref().unwrap()[8 * 16 + 8][..3], &[128, 128, 255]);
        let back_faces = std::fs::read_to_string(&path).unwrap().replace("f 1 2 3 4", "f 4 3 2 1");
        std::fs::write(&path, back_faces).unwrap();
        let (fallback_hits, fallback_height) = compare(&settings);
        assert_eq!(fallback_hits, hits);
        assert_eq!(fallback_height, lid);
        let unaveraged = ProjectionSettings { average_normals: false, ..settings };
        assert_eq!(compare(&unaveraged).0, 0);
        let probe = probe::build(vec![low], 0, 0, &unaveraged, &progress).unwrap();
        assert!(probe.summary().contains("промахи 100%"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_baked_projection_keeps_ties_across_leaves() {
        use std::fmt::Write;

        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice::new(
            app.world().resource::<bevy::render::renderer::RenderDevice>(),
            app.world().resource::<bevy::render::renderer::RenderQueue>(),
        );
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-bvh-tie-{}.obj", bevy::asset::uuid::Uuid::new_v4(),
        ));
        let mut obj = String::from("o Stacked\nvn 1 1 0\nvn 0 1 1\n");
        for (i, x) in [-12., -11., -10., -9., 0., 0., 8., 9., 10., 11.].into_iter().enumerate() {
            let (lo_x, hi_x, lo_z, hi_z, y) = if i == 4 || i == 5 {
                (-2., 6., -6., 2., 0.03)
            } else {
                (x, x + 0.25, -1., 1., if i < 4 { 0.06 } else { 0.03 })
            };
            writeln!(obj, "v {lo_x} {y} {hi_z}\nv {hi_x} {y} {hi_z}\nv {lo_x} {y} {lo_z}").unwrap();
            let first = i * 3 + 1;
            let normal = if i < 5 { 1 } else { 2 };
            writeln!(obj, "f {first}//{normal} {}//{normal} {}//{normal}", first + 1, first + 2).unwrap();
        }
        std::fs::write(&path, obj).unwrap();
        let progress = Progress::default();
        let settings = ProjectionSettings {
            path: Some(path.clone()),
            match_names: false,
            fill_misses: false,
            ..Default::default()
        };
        let projection = project::Projection::load(&path, "Stacked", 2_f32.sqrt(), &settings, &progress).unwrap();
        let hit = projection.sample(Vec3::new(0.5, 0., -0.5), Vec3::Y, Vec3::Y, None).unwrap();
        // The left box is encountered earlier along the ray but the right leaf wins the tie.
        assert!(hit.normal.z > 0.6 && hit.normal.x.abs() < 0.01);
        let objects = vec![quad(0, 0.)];
        let cpu = bake_with_projection(objects.clone(), 0, 0, 16, &settings, &progress).unwrap();
        let gpu = bake_on_device(objects, 0, 0, 16, &settings, &progress, Some(&device)).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(gpu.gpu_projection && gpu.hits == gpu.filled);
        assert_eq!(cpu.rgba(BakeMap::ProjectedNormal), gpu.rgba(BakeMap::ProjectedNormal));
    }

    #[test]
    fn bake_ray_options_use_independent_reaches_and_skip_zero_counts() {
        check_bake_ray_options(None);
    }

    #[test]
    fn disabled_part_maps_have_neutral_bytes_and_generator_inputs() {
        use crate::editor::model_paint_data::mask::GeneratorInput;

        let mut tilted = quad(0, 0.);
        for point in &mut tilted.geometry.positions {
            point.y = point.x;
        }
        let settings = ProjectionSettings {
            maps: MapSelection::MINIMUM.toggle(BakeMap::PartGradient),
            ..default()
        };
        let output = bake_with_projection(vec![tilted], 0, 0, 16, &settings, &Progress::default()).unwrap();
        assert!(output.has_map(BakeMap::PartGradient));
        assert!(!output.has_map(BakeMap::PartRandom));
        assert!(!output.has_map(BakeMap::PartId));
        assert!(!output.has_map(BakeMap::BentNormal));
        assert!(output.graph_component(BakeMap::PartRandom, 0).is_none());
        let baked = output.generator_inputs();
        assert!(baked.part_ids.is_none());
        assert_ne!(baked.disabled_inputs & (1 << GeneratorInput::Id as u32), 0);
        assert_ne!(baked.disabled_inputs & (1 << GeneratorInput::PartRandom as u32), 0);
        assert_eq!(baked.disabled_inputs & (1 << GeneratorInput::PartGradient as u32), 0);
        for i in [8 * 16 + 3, 8 * 16 + 12] {
            assert_eq!(output.values[i][4], 128);
            assert_eq!(&output.values[i][15..18], &[0; 3]);
        }
        assert_ne!(output.values[8 * 16 + 3][5], output.values[8 * 16 + 12][5]);
    }

    fn check_bake_ray_options(device: Option<&RayDevice>) {
        let objects = vec![quad(0, 0.), quad(1, 0.09)];
        let mut settings = ProjectionSettings::default();
        settings.rays.occlusion_samples = Some(23);
        settings.rays.occlusion_reach = 100;
        settings.rays.cavity_reach = 6000;
        settings.rays.thickness_samples = Some(0);
        settings.rays.sky_samples = Some(0);
        let center = 16 * 8 + 8;
        let bake = |settings: &ProjectionSettings| {
            let progress = Progress::default();
            let result = bake_on_device(objects.clone(), 0, 0, 16, settings, &progress, device).unwrap();
            assert_eq!(result.gpu_rays, device.is_some());
            result.values[center]
        };
        let short_ao = bake(&settings);
        assert_eq!(short_ao[0], 255);
        assert!(short_ao[1] < 70);
        assert_eq!(short_ao[2..4], [255, 255]);
        std::mem::swap(&mut settings.rays.occlusion_reach, &mut settings.rays.cavity_reach);
        let short_cavity = bake(&settings);
        assert!(short_cavity[0] < 70);
        assert_eq!(short_cavity[1], 255);

        settings.maps = MapSelection::MINIMUM.toggle(BakeMap::BentNormal);
        let bent_only = bake(&settings);
        assert_eq!(bent_only[0], 255);
        settings.maps = settings.maps.toggle(BakeMap::Occlusion);
        let ao_and_bent = bake(&settings);
        assert_eq!(bent_only[6..9], ao_and_bent[6..9]);
        assert!(ao_and_bent[0] < 70);
        settings.maps = settings.maps.toggle(BakeMap::BentNormal);
        let ao_only = bake(&settings);
        assert_eq!(ao_only[0], ao_and_bent[0]);
        assert_eq!(ao_only[6..9], ao_only[9..12]);
    }

    #[test]
    #[ignore = "requires Bevy's native GPU device"]
    fn gpu_baked_visibility_matches_cpu_on_connected_and_separate_parts() {
        let (app, _, _) = crate::editor::tests::lighting_ui_render::fixture();
        let device = RayDevice::new(
            app.world().resource::<bevy::render::renderer::RenderDevice>(),
            app.world().resource::<bevy::render::renderer::RenderQueue>(),
        );
        check_bake_ray_options(Some(&device));
        let selected = ProjectionSettings {
            maps: MapSelection::MINIMUM.toggle(BakeMap::Cavity),
            ..Default::default()
        };
        let planes = vec![quad(0, 0.), quad(1, 0.01)];
        let progress = Progress::default();
        let cpu = bake_with_projection(planes.clone(), 0, 0, 16, &selected, &progress).unwrap();
        let gpu = bake_on_device(planes, 0, 0, 16, &selected, &progress, Some(&device)).unwrap();
        assert!(gpu.gpu_rays && gpu.has_map(BakeMap::Cavity));
        assert!(!gpu.has_map(BakeMap::Occlusion) && !gpu.has_map(BakeMap::BentNormal));
        assert!(gpu.values.iter().any(|pixel| pixel[1] < 64));
        for (a, b) in cpu.values.iter().zip(&gpu.values) {
            assert!(a[1].abs_diff(b[1]) <= 18);
            assert_eq!([b[0], b[2], b[3]], [255; 3]);
            assert_eq!(b[18..21], [128, 255, 255]);
        }
        let mut slab = quad(0, 0.);
        let bottom = quad(0, -0.06);
        slab.geometry.positions.extend_from_slice(&bottom.geometry.positions);
        let mut lower = bottom.geometry.faces[0].clone();
        lower.material = 1;
        for corner in &mut lower.corners {
            corner.vertex += 4;
        }
        lower.corners.reverse();
        slab.geometry.faces.push(lower);
        slab.geometry.faces.push(Face {
            corners: [1, 0, 4, 5].map(Corner::new).to_vec(),
            material: 1,
            smooth: false,
        });
        let objects = vec![slab, quad(1, 0.18)];
        use crate::editor::model_paint_data::custom::{Kind, Recipe};
        let recipes: Vec<_> = [Kind::Exposure, Kind::Flow, Kind::Deposit, Kind::Contact, Kind::Point]
            .into_iter().map(|kind| Recipe {
                distance: 90,
                elevation: 90,
                spread: 30,
                ..Recipe::new(kind, &[])
            }).collect();
        let settings = ProjectionSettings { custom: recipes.clone().into(), ..default() };
        let cpu = bake_with_projection(objects.clone(), 0, 0, 64, &settings, &Progress::default()).unwrap();
        let scene = live::Scene::prepare(objects.clone(), 0, 0, &settings, &Progress::default()).unwrap();
        let legacy = Progress { legacy_visibility_readback: true, ..default() };
        let old = bake_on_device(objects.clone(), 0, 0, 64, &settings, &legacy, Some(&device)).unwrap();
        let resident = Progress::default();
        let gpu = bake_on_device(objects, 0, 0, 64, &settings, &resident, Some(&device)).unwrap();
        assert!(resident.resident_visibility.load(Ordering::Relaxed));
        assert_eq!(old.values, gpu.values);
        assert_eq!(old.covered, gpu.covered);
        assert_eq!(old.custom, gpu.custom);
        assert_eq!(resident.visibility_readback_bytes.load(Ordering::Relaxed), (gpu.filled * recipes.len() * 4) as u64);
        assert!(gpu.gpu_rays && gpu.gpu_padding);
        assert_eq!(gpu.covered, cpu.covered);
        assert!(cpu.values.iter().any(|pixel| pixel[2] < 200));
        assert!(cpu.values.iter().any(|pixel| pixel[0] < 200 && pixel[3] < 200));
        for channel in [0, 1, 2, 3, 6, 7, 8] {
            let differences: Vec<_> = cpu.values.iter().zip(&gpu.values)
                .map(|(a, b)| a[channel].abs_diff(b[channel])).collect();
            let mean = differences.iter().copied().map(f32::from).sum::<f32>() / differences.len() as f32;
            assert!(mean < 2., "visibility channel {channel}, mean error {mean}");
            assert!(differences.iter().filter(|&&value| value > 18).count() * 100 < differences.len(),
                "visibility channel {channel} differs by more than one Draft ray");
        }
        for (index, recipe) in recipes.iter().enumerate() {
            let (preview, gpu_used) = scene.shade_on_device(
                recipe, 64, super::super::Quality::Draft, &Progress::default(), Some(&device),
            ).unwrap();
            assert!(gpu_used);
            assert!(scene.has_gpu_padding(64));
            assert_eq!(preview, gpu.rgba(BakeMap::Custom(recipe.id)));
            let left = &cpu.custom[index].values;
            let right = &gpu.custom[index].values;
            let differences: Vec<_> = left.iter().zip(right.iter()).map(|(a, b)| a.abs_diff(*b)).collect();
            let mean = differences.iter().copied().map(f32::from).sum::<f32>() / differences.len() as f32;
            assert!(mean < 2., "custom map {:?}, mean error {mean}", recipe.kind);
            if recipe.kind != Kind::Flow {
                assert!(left.iter().any(|&value| value > 10), "empty test map {:?}", recipe.kind);
            }
        }
        let path = std::env::temp_dir().join(format!("bevy-gpu-custom-{}.obj", bevy::asset::uuid::Uuid::new_v4()));
        std::fs::write(&path, concat!(
            "o Tray\nv 0 0.03 0\nv 1 0.03 0\nv 1 0.03 -1\nv 0 0.03 -1\nf 1 2 3 4\n",
            "o Roof\nv 0 0.06 0\nv 1 0.06 0\nv 1 0.06 -1\nv 0 0.06 -1\nf 5 6 7 8\n",
        )).unwrap();
        let settings = ProjectionSettings { path: Some(path.clone()), match_names: true, ..settings };
        let mut target = quad(0, 0.);
        target.name = "Tray_low".into();
        for corner in &mut target.geometry.faces[0].corners {
            corner.uv = corner.uv * 0.75 + Vec2::splat(0.125);
        }
        let low = vec![target];
        let cpu = bake_with_projection(low.clone(), 0, 0, 32, &settings, &Progress::default()).unwrap();
        let scene = live::Scene::prepare(low.clone(), 0, 0, &settings, &Progress::default()).unwrap();
        let gpu = bake_on_device(low.clone(), 0, 0, 32, &settings, &Progress::default(), Some(&device)).unwrap();
        assert!(gpu.gpu_rays && gpu.gpu_projection && gpu.hits > 0);
        assert!(gpu.gpu_padding && gpu.covered.contains(&3));
        assert_eq!(cpu.covered, gpu.covered);
        let projection_maps = [
            BakeMap::ProjectedNormal,
            BakeMap::ProjectedHeight,
            BakeMap::ProjectionCoverage,
        ];
        for map in projection_maps {
            assert_eq!(cpu.rgba(map), gpu.rgba(map), "direct projection for {map:?}");
        }
        for recipe in &recipes {
            let (preview, used_gpu) = scene.shade_on_device(
                recipe, 32, super::super::Quality::Draft, &Progress::default(), Some(&device),
            ).unwrap();
            assert!(used_gpu);
            assert!(scene.has_gpu_raster(32));
            assert!(scene.has_gpu_padding(32));
            let map = BakeMap::Custom(recipe.id);
            assert_eq!(preview, gpu.rgba(map));
            let reference = cpu.rgba(map);
            let difference: u32 = preview.chunks_exact(4).zip(reference.chunks_exact(4))
                .map(|(a, b)| u32::from(a[0].abs_diff(b[0]))).sum();
            assert!(difference < 2 * 32 * 32, "high-poly custom map {:?}", recipe.kind);
        }
        let serial_scene = live::Scene::prepare(low.clone(), 0, 0, &settings, &Progress::default()).unwrap();
        let serial = Progress { serial_projection: true, ..default() };
        let queued = Progress::default();
        let preview = |scene: &live::Scene, progress: &Progress| {
            let (pixels, used_gpu) = scene.shade_on_device(
                &recipes[4], 256, super::super::Quality::Draft, progress, Some(&device),
            ).unwrap();
            assert!(used_gpu && scene.has_gpu_raster(256));
            pixels
        };
        assert_eq!(preview(&serial_scene, &serial), preview(&scene, &queued));
        assert_eq!(serial.projection_peak.load(Ordering::Relaxed), 1);
        assert_eq!(queued.projection_peak.load(Ordering::Relaxed), 3);
        let settings = ProjectionSettings { front: 1, fill_misses: true, ..settings };
        let cpu = bake_with_projection(low.clone(), 0, 0, 32, &settings, &Progress::default()).unwrap();
        let scene = live::Scene::prepare(low.clone(), 0, 0, &settings, &Progress::default()).unwrap();
        let gpu = bake_on_device(low, 0, 0, 32, &settings, &Progress::default(), Some(&device)).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(gpu.gpu_projection);
        assert_eq!(gpu.hits, 0);
        assert_eq!(gpu.filled_misses, gpu.filled);
        for map in projection_maps {
            assert_eq!(cpu.rgba(map), gpu.rgba(map), "filled projection for {map:?}");
        }
        let recipe = &recipes[0];
        let (preview, used_gpu) = scene.shade_on_device(
            recipe, 32, super::super::Quality::Draft, &Progress::default(), Some(&device),
        ).unwrap();
        assert!(used_gpu && scene.has_gpu_raster(32));
        assert_eq!(preview, gpu.rgba(BakeMap::Custom(recipe.id)));
        let point = recipes.iter().find(|recipe| recipe.kind == Kind::Point).unwrap();
        let (preview, used_gpu) = scene.shade_on_device(
            point, 96, super::super::Quality::Draft, &Progress::default(), Some(&device),
        ).unwrap();
        assert!(used_gpu && scene.has_gpu_raster(96));
        let reference = scene.shade(point, 96, super::super::Quality::Draft, &Progress::default()).unwrap();
        assert!(preview.iter().zip(reference).all(|(a, b)| a.abs_diff(b) <= 1));

        let bounds = Bounds { min: Vec3::splat(-1.), max: Vec3::ONE };
        let recipe = Recipe {
            azimuth: 180, elevation: 35, spread: 0, distance: 200, wander: 0,
            ..Recipe::new(Kind::Flow, &[])
        };
        let prepared = custom::Prepared::new(&recipe, bounds);
        let floor = PickTriangle {
            object: 0, face: 0, points: [Vec3::ZERO, Vec3::Z, Vec3::X],
        };
        let texel = RaySurface {
            point: Vec3::new(0.35, 0., 0.05), normal: Vec3::Y, geometric: Vec3::Y,
            object: 0, face: 0, part: 0, bounds,
        };
        let compute = |height: Option<f32>, roof: bool| {
            let triangles: Vec<_> = std::iter::once(floor)
                .chain(height.map(|height| PickTriangle {
                    object: 0, face: 1, points: [Vec3::ZERO, Vec3::X, Vec3::Y * height],
                }))
                .chain(roof.then_some(PickTriangle {
                    object: 1, face: 0,
                    points: [Vec3::new(-1., 0.08, -1.), Vec3::new(-1., 0.08, 2.), Vec3::new(2., 0.08, -1.)],
                })).collect();
            let normals: Vec<_> = triangles.iter().map(|triangle| {
                let [a, b, c] = triangle.points;
                [(b - a).cross(c - a).normalize(); 3]
            }).collect();
            let scene = Bvh::from_triangles(triangles);
            let rays = Rays {
                scene: &scene, scale: 1., sampling: super::super::Quality::Draft.sampling(),
                maps: MapSelection::ALL, resolution: 32, flow_normals: Some(custom::FlowNormals::Mesh(&normals)),
            };
            let parts = Parts { faces: vec![0, 0], bounds: vec![bounds] };
            let mut batch = rays_gpu::Batch::new(
                &device, &rays, rays_gpu::PartsOfScene::Low(&parts, 0), std::slice::from_ref(&prepared), false, None,
            ).unwrap();
            batch.push(texel, 0, 0.02);
            let mut pixels = [[0; 4]];
            batch.flush_preview(&mut pixels, &Progress::default(), true).unwrap();
            let cpu = prepared.sample(&rays, texel, 0.02, &|_| true);
            assert!(pixels[0][0].abs_diff(cpu) <= 2, "folded flow: GPU {:?}, CPU {cpu}", pixels[0]);
            pixels[0][0]
        };
        let open = compute(None, false);
        let concave = compute(Some(1.), false);
        let convex = compute(Some(-1.), false);
        assert!(concave < open && convex > open, "open {open}, concave {concave}, convex {convex}");
        assert!(compute(None, true) < open);
    }

    #[test]
    fn baked_custom_maps_respond_to_blockers_and_distance() {
        use crate::editor::model_paint_data::custom::{Kind, Recipe};
        let mut recipes: Vec<_> = [Kind::Exposure, Kind::Deposit, Kind::Contact, Kind::Point, Kind::Flow]
            .iter()
            .map(|&kind| Recipe::new(kind, &[]))
            .collect();
        for recipe in &mut recipes[..2] {
            recipe.elevation = 90;
            recipe.spread = 0;
        }
        let settings = ProjectionSettings {
            custom: Arc::new(recipes.clone()),
            ..Default::default()
        };
        let bake = |objects| {
            bake_with_projection(objects, 0, 0, 16, &settings, &Progress::default()).unwrap()
        };
        let open = bake(vec![quad(0, 0.)]);
        let blocked = bake(vec![quad(0, 0.), quad(1, 0.02)]);
        let center = (8 * 16 + 8) * 4;
        let at = |maps: &Maps, index: usize| maps.rgba(BakeMap::Custom(recipes[index].id))[center];
        assert_eq!((at(&open, 0), at(&blocked, 0)), (255, 0));
        assert_eq!((at(&open, 1), at(&blocked, 1)), (0, 255));
        assert_eq!(at(&open, 2), 0);
        assert!(at(&blocked, 2) > 20);
        let point = open.rgba(BakeMap::Custom(recipes[3].id));
        assert!(point[center] > 240);
        assert_eq!(point[0], 0);
        assert_eq!(open.generator_inputs().custom, open.custom);
        let live = live::Scene::prepare(vec![quad(0, 0.), quad(1, 0.02)], 0, 0, &settings, &Progress::default()).unwrap();
        for recipe in &recipes {
            let preview = live.shade(recipe, 16, super::super::Quality::Draft, &Progress::default()).unwrap();
            assert_eq!(&preview[center..center + 4], &blocked.rgba(BakeMap::Custom(recipe.id))[center..center + 4]);
        }
    }

    #[test]
    fn baked_flow_collects_upstream_and_respects_cover() {
        use crate::editor::model_paint_data::custom::{Kind, Recipe};
        let mut ramp = quad(0, 0.);
        for point in &mut ramp.geometry.positions {
            point.y = -point.z;
        }
        let mut roof = ramp.clone();
        roof.id = 1;
        roof.transform.translation.y += 1.;
        let recipe = Recipe {
            spread: 0,
            wander: 0,
            ..Recipe::new(Kind::Flow, &[])
        };
        let map = BakeMap::Custom(recipe.id);
        let settings = ProjectionSettings {
            custom: vec![recipe].into(),
            ..Default::default()
        };
        let bake = |objects, settings| {
            bake_with_projection(objects, 0, 0, 32, settings, &Progress::default()).unwrap()
        };
        let open = bake(vec![ramp.clone()], &settings).rgba(map);
        let covered = bake(vec![ramp.clone(), roof], &settings).rgba(map);
        let lower = (4 * 32 + 16) * 4;
        let upper = (31 * 32 + 16) * 4;
        assert!(open[lower] > 170);
        assert!(open[upper] < 65);
        assert_eq!(covered[lower], 0);
        let mut reverse = settings.clone();
        Arc::make_mut(&mut reverse.custom)[0].elevation = -90;
        assert_eq!(bake(vec![ramp], &reverse).rgba(map)[lower], 0);
    }

    #[test]
    fn baked_mesh_maps_respond_to_geometry_and_cancellation() {
        let target = quad(0, 0.);
        let progress = Progress::default();
        let open = bake(vec![target.clone()], 0, 0, 16, &progress).unwrap();
        let blocked = bake(vec![target.clone(), quad(1, 0.01)], 0, 0, 16, &progress).unwrap();
        let center = (8 * 16 + 8) * 4;
        let pixel = |maps: &Maps, map| -> [u8; 4] {
            maps.rgba(map)[center..center + 4].try_into().unwrap()
        };
        assert_eq!(open.filled, 16 * 16);
        assert_eq!(pixel(&open, BakeMap::Occlusion), [255; 4]);
        assert!(pixel(&blocked, BakeMap::Occlusion)[0] < 20);
        assert!(pixel(&blocked, BakeMap::Cavity)[0] < 20);
        assert_eq!(pixel(&blocked, BakeMap::Sky), [0, 0, 0, 255]);
        assert_eq!(pixel(&blocked, BakeMap::WorldNormal), [128, 255, 128, 255]);
        assert_eq!(
            pixel(&blocked, BakeMap::PartId),
            pixel(&open, BakeMap::PartId)
        );
        progress.cancelled.store(true, Ordering::Relaxed);
        assert!(bake(vec![target], 0, 0, 16, &progress).is_err());
    }

    #[test]
    fn baked_high_poly_projects_named_glb_parts_in_world_space() {
        use crate::editor::model_document::{MaterialSlot, glb::Glb};

        let mut low = quad(0, 0.);
        low.name = "Plate_low".into();
        let mut high = quad(1, 0.);
        high.name = "Plate_high.001".into();
        high.transform.translation.y = 0.04;
        let mut unrelated = high.clone();
        unrelated.id = 2;
        unrelated.name = "Neighbor_high".into();
        unrelated.transform.translation.y = 0.06;
        let material = MaterialSlot {
            bake_fixes: Default::default(),
            material: default(),
            source: serde_json::json!({"material": 0}),
            paint: None,
            authored: None,
        };
        high.materials.push(material.clone());
        unrelated.materials.push(material);
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-high-{}.glb",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, Glb::new_asset(&[high, unrelated]).unwrap()).unwrap();
        let mut settings = ProjectionSettings {
            path: Some(path.clone()),
            ..default()
        };
        let recipe = crate::editor::model_paint_data::custom::Recipe::new(
            crate::editor::model_paint_data::custom::Kind::Point, &[],
        );
        settings.custom = vec![recipe.clone()].into();
        let bake = |settings: &ProjectionSettings| {
            bake_with_projection(vec![low.clone()], 0, 0, 16, settings, &Progress::default())
        };
        let named = bake(&settings);
        let live = live::Scene::prepare(vec![low.clone()], 0, 0, &settings, &Progress::default()).unwrap();
        let preview = live.shade(&recipe, 16, super::super::Quality::Draft, &Progress::default()).unwrap();
        settings.match_names = false;
        let all = bake(&settings);
        settings.front = 1;
        let filled = bake(&settings);
        settings.fill_misses = false;
        let misses = bake(&settings);
        std::fs::remove_file(path).unwrap();
        let named = named.unwrap();
        let all = all.unwrap();
        let misses = misses.unwrap();
        let filled = filled.unwrap();
        let center = (8 * 16 + 8) * 4;
        assert_eq!(&preview[center..center + 4], &named.rgba(BakeMap::Custom(recipe.id))[center..center + 4]);
        assert_eq!(named.hits, named.filled);
        assert_eq!(
            named.rgba(BakeMap::ProjectedNormal)[center..center + 4],
            [128, 128, 255, 255]
        );
        let named_height = named.rgba(BakeMap::ProjectedHeight)[center];
        assert!(named_height > 128);
        assert!(all.rgba(BakeMap::ProjectedHeight)[center] > named_height);
        assert_eq!(misses.hits, 0);
        assert_eq!(filled.hits, 0);
        assert_eq!(filled.filled_misses, filled.filled);
        assert_eq!(
            filled.rgba(BakeMap::ProjectionCoverage),
            misses.rgba(BakeMap::ProjectionCoverage)
        );
        assert_eq!(filled.rgba(BakeMap::ProjectedHeight)[center], named_height);
        assert_eq!(
            misses.rgba(BakeMap::ProjectionCoverage)[center..center + 4],
            [0, 0, 0, 255]
        );
        assert_eq!(misses.rgba(BakeMap::ProjectedHeight)[center], 128);
    }

    #[test]
    fn baked_cage_reaches_high_poly_outside_front_and_rejects_wrong_topology() {
        use crate::editor::model_document::glb::Glb;

        let directory = std::env::temp_dir().join(format!(
            "bevy-bake-cage-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut low = quad(0, 0.);
        low.name = "Plate_low".into();
        let mut high = quad(1, 0.09);
        high.name = "Plate_high".into();
        let mut cage = quad(1, 0.16);
        cage.name = "Plate_cage".into();
        cage.transform.translation.x = 0.02;
        let source = directory.join("high.glb");
        let fitted = directory.join("cage.glb");
        let wrong = directory.join("wrong.obj");
        std::fs::write(&source, Glb::new_asset(&[high]).unwrap()).unwrap();
        std::fs::write(&fitted, Glb::new_asset(&[cage]).unwrap()).unwrap();
        std::fs::write(
            &wrong,
            "v 0 1 0\nv 1 1 0\nv 0 1 -1\nv 2 1 0\nv 3 1 0\nv 2 1 -1\nf 1 2 3\nf 4 5 6\n",
        )
        .unwrap();
        let mut settings = ProjectionSettings {
            path: Some(source),
            front: 1,
            fill_misses: false,
            ..default()
        };
        let bake = |settings: &ProjectionSettings| {
            bake_with_projection(vec![low.clone()], 0, 0, 16, settings, &Progress::default())
        };
        let plain = bake(&settings);
        settings.cage = Some(fitted);
        let projected = bake(&settings);
        settings.cage = Some(wrong);
        let rejected = bake(&settings);
        settings.cage = None;
        settings.auto_fit = true;
        let auto = bake(&settings);
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(plain.unwrap().hits, 0);
        let projected = projected.unwrap();
        assert_eq!(projected.hits, projected.filled);
        let value = projected.rgba(BakeMap::ProjectedHeight)[(8 * 16 + 8) * 4];
        assert!((198..=200).contains(&value), "{value}");
        assert!(matches!(rejected, Err(error) if error.contains("связность")));
        let auto = auto.unwrap();
        assert_eq!(auto.hits, auto.filled);
        assert_eq!(auto.cage_fit.unwrap().raised, 4);
    }

    #[test]
    fn baked_cage_brush_changes_bake_coverage_and_preview() {
        use crate::editor::model_document::glb::Glb;
        use std::sync::Arc;
        let low = quad(0, 0.);
        let mut high = quad(1, 0.09);
        high.name = low.name.clone();
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-brush-{}.glb",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, Glb::new_asset(&[high]).unwrap()).unwrap();
        let mut settings = ProjectionSettings {
            path: Some(path.clone()),
            front: 1,
            fill_misses: false,
            ..default()
        };
        let prepared =
            probe::prepare(vec![low.clone()], 0, &settings, &Progress::default()).unwrap();
        let plain = prepared.sample(0, &settings, &Progress::default()).unwrap();
        settings.manual = Some(super::super::cage_brush::Scales {
            values: Arc::new(vec![8., 1., 1., 1.]),
        });
        let edited = prepared.sample(0, &settings, &Progress::default()).unwrap();
        let maps = bake_with_projection(vec![low], 0, 0, 32, &settings, &Progress::default());
        std::fs::remove_file(path).unwrap();
        let maps = maps.unwrap();
        assert!(maps.hits > 0 && maps.hits < maps.filled);
        assert!(plain.summary().contains("промахи 100%"));
        assert_ne!(plain.summary(), edited.summary());
        assert!(edited.points.iter().any(|point| point.y > 0.1));
    }

    #[test]
    fn baked_cage_probe_draws_low_poly_shell_without_coverage() {
        let settings = ProjectionSettings { front: 10, ..default() };
        let progress = Progress::default();
        let prepared = probe::prepare(vec![quad(0, 0.)], 0, &settings, &progress).unwrap();
        let shell = prepared.sample(0, &settings, &progress).unwrap();
        assert!(!shell.has_source);
        assert_eq!(shell.indices.len(), 6);
        let expected = 2_f32.sqrt() * 0.1;
        assert!(shell.points.iter().all(|point| (point.y - expected).abs() < 1e-5));

        let painted = ProjectionSettings {
            manual: Some(super::super::cage_brush::Scales {
                values: Arc::new(vec![2., 1., 1., 1.]),
            }),
            ..settings
        };
        let shell = prepared.sample(0, &painted, &progress).unwrap();
        let corner = shell.points.iter().find(|point| point.x == 0. && point.z == 0.).unwrap();
        assert!((corner.y - expected * 2.).abs() < 1e-5);
    }

    #[test]
    fn baked_cage_probe_distinguishes_hits_limits_and_misses() {
        use crate::editor::model_document::glb::Glb;
        let low = quad(0, 0.);
        let mut high = quad(1, 0.065);
        high.name = low.name.clone();
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-probe-{}.glb",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, Glb::new_asset(std::slice::from_ref(&high)).unwrap()).unwrap();
        let samples = [1, 5, 10].map(|front| {
            probe::build(
                vec![low.clone()],
                0,
                0,
                &ProjectionSettings {
                    path: Some(path.clone()),
                    front,
                    ..default()
                },
                &Progress::default(),
            )
        });
        let mut other = quad(2, 0.1);
        other.name = high.name.clone();
        std::fs::write(&path, Glb::new_asset(&[high, other]).unwrap()).unwrap();
        let overlap = probe::build(
            vec![low],
            0,
            0,
            &ProjectionSettings {
                path: Some(path.clone()),
                front: 10,
                ..default()
            },
            &Progress::default(),
        );
        std::fs::remove_file(path).unwrap();
        let [miss, tight, hits] = samples.map(Result::unwrap);
        let overlap = overlap.unwrap();
        assert!(
            overlap.summary().contains("чужие части 100%"),
            "{}",
            overlap.summary()
        );
        assert!(
            miss.summary().contains("промахи 100%"),
            "{}",
            miss.summary()
        );
        assert!(
            tight.summary().contains("на пределе 100%"),
            "{}",
            tight.summary()
        );
        assert!(
            hits.summary().contains("Попадания 100%"),
            "{}",
            hits.summary()
        );
        assert!(hits.indices.len() > 12 && hits.edges.len() > 4);
        assert!(
            hits.points
                .iter()
                .all(|p| (p.y - 0.1 * 2_f32.sqrt()).abs() < 1e-6)
        );
    }

    #[test]
    fn baked_relief_distinguishes_ridges_creases_and_flat_diagonals() {
        let progress = Progress::default();
        let floor = quad(0, 0.);
        let mut triangulated = floor.clone();
        let face = triangulated.geometry.faces[0].clone();
        triangulated.geometry.faces = [[0, 1, 2], [0, 2, 3]]
            .into_iter()
            .map(|indices| Face {
                corners: indices
                    .into_iter()
                    .map(|index| face.corners[index].clone())
                    .collect(),
                material: 0,
                smooth: false,
            })
            .collect();
        let flat = bake(vec![triangulated], 0, 0, 32, &progress).unwrap();
        for (map, value) in [
            (BakeMap::Curvature, 128),
            (BakeMap::EdgeDistance, 255),
            (BakeMap::CreaseDistance, 255),
        ] {
            assert!(
                flat.rgba(map)
                    .chunks_exact(4)
                    .all(|p| p == [value, value, value, 255])
            );
        }

        let folded = |height, surface| {
            let mut object = floor.clone();
            object
                .geometry
                .positions
                .extend([Vec3::new(0., height, -1.), Vec3::new(0., height, 0.)]);
            object.geometry.faces.push(Face {
                corners: [0, 3, 4, 5].into_iter().map(Corner::new).collect(),
                material: 1,
                smooth: false,
            });
            let settings = ProjectionSettings { surface, ..default() };
            bake_with_projection(vec![object], 0, 0, 32, &settings, &progress).unwrap()
        };
        let surface = crate::editor::model_bake_settings::SurfaceOptions::default();
        let ridge = folded(-1., surface);
        let crease = folded(1., surface);
        let near = |maps: &Maps, map| maps.rgba(map)[16 * 32 * 4];
        assert!(near(&ridge, BakeMap::Curvature) > 225);
        assert!(near(&ridge, BakeMap::EdgeDistance) < 20);
        assert_eq!(near(&ridge, BakeMap::CreaseDistance), 255);
        assert!(near(&crease, BakeMap::Curvature) < 30);
        assert!(near(&crease, BakeMap::CreaseDistance) < 20);
        assert_eq!(near(&crease, BakeMap::EdgeDistance), 255);
        let wide = folded(-1., crate::editor::model_bake_settings::SurfaceOptions {
            radius: 80, ..surface
        });
        let row = 16 * 32 * 4;
        assert!(wide.rgba(BakeMap::Curvature)[row + 4 * 3] > ridge.rgba(BakeMap::Curvature)[row + 4 * 3]);
        let blunt = folded(-1., crate::editor::model_bake_settings::SurfaceOptions {
            hard_angle: 1000, ..surface
        });
        assert_eq!(near(&blunt, BakeMap::EdgeDistance), 255);
        assert_eq!(near(&blunt, BakeMap::Curvature), 128);
        let auto = folded(-1., crate::editor::model_bake_settings::SurfaceOptions {
            auto_edge_reach: true, ..surface
        });
        assert!((auto.edge_reach - 0.1).abs() < 1e-6);
        assert!(auto.rgba(BakeMap::EdgeDistance)[row + 4 * 3] > ridge.rgba(BakeMap::EdgeDistance)[row + 4 * 3]);
    }

    #[test]
    fn baked_low_poly_finds_smooth_ridges_and_creases_within_reach() {
        let calculate = |sign: f32, edge_reach| {
            let mut object = quad(0, 0.);
            let face = &mut object.geometry.faces[0];
            for (vertex, corner) in face.corners.iter_mut().enumerate() {
                let side = if vertex == 0 || vertex == 3 { -1. } else { 1. };
                corner.source_normal = Some(Vec3::new(side * sign, 0.5, 0.).normalize());
            }
            let settings = ProjectionSettings {
                maps: MapSelection::MINIMUM.toggle(BakeMap::EdgeDistance).toggle(BakeMap::CreaseDistance),
                surface: crate::editor::model_bake_settings::SurfaceOptions {
                    edge_reach,
                    hard_angle: 1200,
                    ..default()
                },
                ..default()
            };
            bake_with_projection(vec![object], 0, 0, 16, &settings, &Progress::default()).unwrap()
        };
        let narrow = calculate(1., 2000);
        let ridge = calculate(1., 10_000);
        let crease = calculate(-1., 10_000);
        let center = 8 * 16 + 8;
        assert_eq!(&narrow.values[center][18..21], &[128, 255, 255]);
        assert_eq!(&ridge.values[center][18..21], &[128, 0, 255]);
        assert_eq!(&crease.values[center][18..21], &[128, 255, 0]);
    }

    #[test]
    fn baked_high_relief_projects_folds_and_interpolated_curvature_onto_flat_uvs() {
        use crate::editor::model_document::glb::Glb;

        let low = quad(0, 0.);
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-relief-{}.glb",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let settings = ProjectionSettings {
            path: Some(path.clone()),
            fill_misses: false,
            ..default()
        };
        let bake_source = |source, settings: &ProjectionSettings| {
            std::fs::write(&path, Glb::new_asset(&[source]).unwrap()).unwrap();
            bake_with_projection(vec![low.clone()], 0, 0, 32, settings, &Progress::default())
                .unwrap()
        };
        let mut flat = quad(1, 0.05);
        flat.name = low.name.clone();
        let raised = bake_source(flat.clone(), &settings);
        let fold = |height| {
            let mut high = flat.clone();
            high.geometry
                .positions
                .extend([Vec3::new(0., height, -1.), Vec3::new(0., height, 0.)]);
            high.geometry.faces.push(Face {
                corners: [0, 3, 4, 5].into_iter().map(Corner::new).collect(),
                material: 1,
                smooth: false,
            });
            bake_source(high, &settings)
        };
        let ridge = fold(-0.6);
        let crease = fold(0.7);
        for (vertex, corner) in flat.geometry.faces[0].corners.iter_mut().enumerate() {
            let side = if vertex == 0 || vertex == 3 { -1. } else { 1. };
            corner.source_normal = Some(Vec3::new(side * 0.9, 0.45, 0.).normalize());
        }
        let smooth = bake_source(flat.clone(), &settings);
        let mut altered = settings.clone();
        altered.surface.sharpness = 50;
        let low_threshold = bake_with_projection(vec![low.clone()], 0, 0, 32, &altered, &Progress::default()).unwrap();
        altered.surface.sharpness = 950;
        let high_threshold = bake_with_projection(vec![low.clone()], 0, 0, 32, &altered, &Progress::default()).unwrap();
        altered.surface.contrast = 100;
        let low_contrast = bake_with_projection(vec![low], 0, 0, 32, &altered, &Progress::default()).unwrap();
        std::fs::remove_file(path).unwrap();
        let near = |maps: &Maps, map| maps.rgba(map)[16 * 32 * 4];
        let center = |maps: &Maps, map| maps.rgba(map)[(16 * 32 + 16) * 4];
        assert_eq!(near(&raised, BakeMap::Curvature), 128);
        assert_eq!(center(&raised, BakeMap::EdgeDistance), 255);
        assert!(near(&ridge, BakeMap::Curvature) > 225);
        assert!(near(&ridge, BakeMap::EdgeDistance) < 20);
        assert_eq!(near(&ridge, BakeMap::CreaseDistance), 255);
        assert!(near(&crease, BakeMap::Curvature) < 30);
        assert!(near(&crease, BakeMap::CreaseDistance) < 20);
        assert_eq!(near(&crease, BakeMap::EdgeDistance), 255);
        assert!(center(&smooth, BakeMap::Curvature) > 150);
        assert!(center(&low_threshold, BakeMap::EdgeDistance) < center(&high_threshold, BakeMap::EdgeDistance));
        assert!(center(&smooth, BakeMap::Curvature) > center(&low_contrast, BakeMap::Curvature));
    }

    #[test]
    fn baked_high_poly_uses_scene_context_for_visibility_and_thickness() {
        use crate::editor::model_document::glb::Glb;

        let mut low = quad(0, 0.);
        low.name = "Floor_low".into();
        let mut surface = quad(1, 0.025);
        surface.name = "Floor_high".into();
        let normal = Vec3::new(0.35, 0.94, 0.).normalize();
        for corner in &mut surface.geometry.faces[0].corners {
            corner.source_normal = Some(normal);
        }
        let mut above = quad(2, 0.035);
        above.name = "Cover_high".into();
        let mut below = quad(3, -0.04);
        below.name = "Base_high".into();
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-context-{}.glb",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        let settings = ProjectionSettings {
            path: Some(path.clone()),
            ..default()
        };
        let bake_high = |objects: &[Object]| {
            std::fs::write(&path, Glb::new_asset(objects).unwrap()).unwrap();
            bake_with_projection(vec![low.clone()], 0, 0, 32, &settings, &Progress::default())
                .unwrap()
        };
        let empty = bake_high(std::slice::from_ref(&surface));
        let surrounded = bake_high(&[surface, above, below]);
        std::fs::remove_file(path).unwrap();
        let center = (16 * 32 + 16) * 4;
        let texel = |maps: &Maps, map| -> [u8; 4] {
            maps.rgba(map)[center..center + 4].try_into().unwrap()
        };
        for map in [
            BakeMap::Occlusion,
            BakeMap::Cavity,
            BakeMap::Sky,
            BakeMap::Thickness,
        ] {
            assert!(texel(&empty, map)[0] > 200, "{map:?}");
            assert!(
                texel(&surrounded, map)[0] < 100,
                "{map:?}: {:?}",
                texel(&surrounded, map)
            );
        }
        assert_eq!(
            texel(&empty, BakeMap::ProjectedHeight),
            texel(&surrounded, BakeMap::ProjectedHeight)
        );
        assert_eq!(texel(&surrounded, BakeMap::ProjectionCoverage), [255; 4]);
        assert!(texel(&surrounded, BakeMap::WorldNormal)[0] > 160);
        assert_eq!(texel(&surrounded, BakeMap::Position)[1], 128);
    }

    #[test]
    fn baked_quality_supersamples_a_step_without_changing_the_center_part() {
        use crate::editor::model_document::glb::Glb;

        let low = quad(0, 0.);
        let base = quad(1, 0.02);
        let mut step = quad(2, 0.06);
        step.geometry.positions[0].x = 0.52;
        step.geometry.positions[3].x = 0.52;
        let path = std::env::temp_dir().join(format!(
            "bevy-bake-supersample-{}.glb",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, Glb::new_asset(&[base, step]).unwrap()).unwrap();
        let calculate = |grid| {
            bake_with_projection(
                vec![low.clone()],
                0,
                0,
                32,
                &ProjectionSettings {
                    path: Some(path.clone()),
                    match_names: false,
                    surface: crate::editor::model_bake_settings::SurfaceOptions {
                        grid,
                        ..default()
                    },
                    ..default()
                },
                &Progress::default(),
            )
        };
        let draft = calculate(None);
        let standard = calculate(Some(2));
        std::fs::remove_file(path).unwrap();
        let draft = draft.unwrap();
        let standard = standard.unwrap();
        let edge = (8 * 32 + 16) * 4;
        let a = draft.rgba(BakeMap::ProjectedHeight);
        let b = standard.rgba(BakeMap::ProjectedHeight);
        assert!(a[edge] + 20 < b[edge] && b[edge] < a[edge + 4]);
        let lo = draft.rgba(BakeMap::PartId);
        let hi = standard.rgba(BakeMap::PartId);
        assert_eq!(lo[edge..edge + 4], hi[edge..edge + 4]);
        assert_ne!(lo[edge..edge + 4], lo[edge + 4..edge + 8]);
    }
}
