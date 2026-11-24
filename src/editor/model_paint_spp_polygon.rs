//! Replay recorded polygon selections into the imported mask's UV image.
use super::{FillMode, Object, SurfaceTrace};
use bevy::prelude::*;
use std::{collections::BTreeSet, ops::Range};
use subcraft_spp::document::DecalHit;

const NO_UV_TRIANGLE: u32 = u32::MAX;

pub(in crate::editor) struct PolygonReplay {
    pub(super) trace: SurfaceTrace,
    pub(super) center: Vec3,
    parts: Vec<(String, Range<usize>)>,
    uv_offsets: Vec<Vec2>,
    visibility_tolerance: f32,
}

impl PolygonReplay {
    pub fn new(object: &Object, parts: Vec<(String, Range<usize>)>) -> Self {
        let (min, max) = object.geometry.positions.iter().copied().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), point| (min.min(point), max.max(point)),
        );
        Self {
            trace: SurfaceTrace::build(std::slice::from_ref(object), &Default::default(), true),
            center: (min + max) * 0.5,
            parts,
            uv_offsets: (0..object.materials.len())
                .map(|slot| object.uv_offset(slot))
                .collect(),
            // Material Painter measures 0.005 in a mesh normalized to a span of 2.
            visibility_tolerance: (max - min).max_element() * 0.0025,
        }
    }

    pub(super) fn uv_offset(&self, slot: usize) -> Vec2 {
        self.uv_offsets.get(slot).copied().unwrap_or(Vec2::ZERO)
    }

    pub fn triangles(&self, hit: &DecalHit, slot: usize, size: u32) -> Vec<[Vec2; 3]> {
        let mode = match hit.granularity {
            0 => FillMode::Triangle,
            1 => FillMode::Polygon,
            2 => FillMode::Object,
            _ => FillMode::UvIsland,
        };
        let mut selected = BTreeSet::new();
        self.select(hit, slot, mode, Mat4::IDENTITY, &mut selected);
        if let Some(matrix) = hit.duplicate {
            self.select(
                hit,
                slot,
                mode,
                Mat4::from_cols_array(&matrix),
                &mut selected,
            );
        }
        self.uv_triangles(selected, size)
    }

    pub fn excluded_triangles(
        &self,
        slot: usize,
        size: u32,
        default_on: bool,
        exceptions: &[String],
    ) -> Vec<[Vec2; 3]> {
        let exceptions: BTreeSet<_> = exceptions.iter().map(String::as_str).collect();
        let excluded = self
            .parts
            .iter()
            .filter(|(name, _)| exceptions.contains(name.as_str()) == default_on)
            .flat_map(|(_, faces)| faces.clone())
            .filter(|&i| self.trace.attributes[i].slot == slot);
        self.uv_triangles(excluded, size)
    }

    fn uv_triangles(&self, indices: impl IntoIterator<Item = usize>, size: u32) -> Vec<[Vec2; 3]> {
        let topology = self.trace.topology.as_ref().unwrap();
        let attributes = &self.trace.attributes;
        let indices: Vec<_> = indices.into_iter().collect();
        let padding = topology.padding(indices.iter().copied(), attributes, 2. / size as f32);
        indices
            .into_iter()
            .map(|index| attributes[index].uv)
            .chain(
                padding
                    .chunks_exact(3)
                    .map(|points| [points[0], points[1], points[2]]),
            )
            .collect()
    }

    fn select(
        &self,
        hit: &DecalHit,
        slot: usize,
        mode: FillMode,
        duplicate: Mat4,
        selected: &mut BTreeSet<usize>,
    ) {
        let trace = &self.trace;
        let attributes = &trace.attributes;
        let topology = trace.topology.as_ref().unwrap();
        let inverse = duplicate * Mat4::from_cols_array(&hit.proj_view_inv);
        let to_uv = |p: Vec2| {
            let p = duplicate.transform_point3((p * 2. - Vec2::ONE).extend(0.));
            Vec2::new(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5) - self.uv_offset(slot)
        };
        let points: Vec<_> = hit.points.iter().copied().map(Vec2::from_array).collect();
        if points.iter().any(|p| !p.is_finite()) {
            return;
        }
        let points = match points.as_slice() {
            [a, b] if (*a - *b).abs().min_element() < 0.01 => vec![(*a + *b) * 0.5],
            _ => points,
        };
        let matches = |i: usize| attributes[i].slot == slot;
        let mut seeds = Vec::new();
        match points.as_slice() {
            [p] if hit.uv_space => {
                seeds.extend(
                    topology
                        .pick_uv(to_uv(*p), attributes, matches)
                        .map(|(i, _)| i),
                );
            }
            [p] => {
                if let Some((origin, direction)) = ray(inverse, *p) {
                    if hit.through {
                        seeds.extend(trace.bvh.ray_triangles(origin, direction));
                    } else {
                        seeds.extend(
                            trace
                                .bvh
                                .cast_triangle(origin, direction, |_| true)
                                .map(|(i, _)| i),
                        );
                    }
                }
            }
            [a, b] => {
                let [a, b] = if hit.uv_space {
                    [to_uv(*a), to_uv(*b)]
                } else {
                    [*a, *b]
                };
                let bounds = Rect::from_corners(a, b);
                let projection = inverse.inverse();
                for (i, triangle) in attributes.iter().enumerate().filter(|(i, _)| matches(*i)) {
                    if hit.uv_space {
                        let [a, b, c] = triangle.uv;
                        if bounds.contains((a + b + c) / 3.) {
                            seeds.push(i);
                        }
                        continue;
                    }
                    let [a, b, c] = triangle.points;
                    let center = (a + b + c) / 3.;
                    let clip = projection * center.extend(1.);
                    if !clip.is_finite() || clip.w <= 1e-12 || clip.z.abs() > clip.w {
                        continue;
                    }
                    let point = clip.truncate().truncate() / clip.w * 0.5 + Vec2::splat(0.5);
                    if !bounds.contains(point) {
                        continue;
                    }
                    if hit.through
                        || ray(inverse, point).is_some_and(|(origin, direction)| {
                            trace
                                .bvh
                                .cast_triangle(origin, direction, |_| true)
                                .is_some_and(|(index, distance)| {
                                    index == i
                                        || (origin + direction * distance).distance(center)
                                            < self.visibility_tolerance
                                })
                        })
                    {
                        seeds.push(i);
                    }
                }
            }
            _ => return,
        }
        let mut parts = BTreeSet::new();
        for seed in seeds.into_iter().filter(|&i| matches(i)) {
            if mode == FillMode::Object {
                let face = trace.bvh.triangle(seed).face;
                parts.insert(self.parts.partition_point(|(_, part)| part.end <= face));
            } else if !selected.contains(&seed) {
                selected.extend(
                    topology
                        .select(seed, Vec3::ZERO, mode, 1., attributes)
                        .into_iter()
                        .filter(|&i| matches(i)),
                );
            }
        }
        for (_, part) in parts.into_iter().filter_map(|index| self.parts.get(index)) {
            // SPP geometry consists of one triangle per face, in source order.
            selected.extend(part.clone().filter(|&i| matches(i)));
        }
    }

    pub fn uv_owners(&self, slot: usize, size: u32) -> Vec<u32> {
        let mut owners = vec![NO_UV_TRIANGLE; (size * size) as usize];
        // The Painter UV bake has no depth buffer. The last drawn triangle owns the texel.
        for (index, triangle) in self.trace.attributes.iter().enumerate() {
            if triangle.slot == slot {
                Self::rasterize(std::slice::from_ref(&triangle.uv), size, |pixel| {
                    owners[pixel] = index as u32;
                });
            }
        }
        owners
    }

    pub fn padding_sources(owners: &[u32], size: u32, pixels: u32) -> Vec<u32> {
        super::super::export::nearest_sources(
            size,
            pixels,
            owners.iter().map(|&owner| owner != NO_UV_TRIANGLE),
        )
    }

    pub fn rasterize(triangles: &[[Vec2; 3]], size: u32, mut write: impl FnMut(usize)) {
        let extent = Vec2::splat(size as f32);
        for triangle in triangles {
            let uv = triangle.map(|p| p * extent);
            let [a, b, c] = uv;
            let min = a.min(b).min(c).floor().clamp(Vec2::ZERO, extent).as_uvec2();
            let max = a.max(b).max(c).ceil().clamp(Vec2::ZERO, extent).as_uvec2();
            for y in min.y..max.y {
                for x in min.x..max.x {
                    let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if super::super::projection::weights(center, uv)
                        .is_some_and(|weights| weights.min_element() >= -1e-5)
                    {
                        write((y * size + x) as usize);
                    }
                }
            }
        }
    }
}

pub(super) fn ray(inverse: Mat4, point: Vec2) -> Option<(Vec3, Vec3)> {
    let ndc = point * 2. - Vec2::ONE;
    let near = inverse * ndc.extend(-1.).extend(1.);
    let far = inverse * ndc.extend(1.).extend(1.);
    let origin = near.truncate() / near.w;
    // A perspective matrix can place the far end at infinity.
    let direction = if far.w.abs() < 1e-12 {
        far.truncate()
    } else {
        far.truncate() / far.w - origin
    };
    Some((origin, direction.try_normalize()?)).filter(|(origin, _)| origin.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face, Geometry};

    #[test]
    fn spp_import_polygon_mask_respects_occlusion_and_materials() {
        let mut geometry = Geometry::default();
        let uv = [Vec2::ZERO, Vec2::X, Vec2::Y];
        for (slot, depth) in [0.5, -0.5].into_iter().enumerate() {
            let start = geometry.positions.len();
            geometry.positions.extend(uv.map(|p| p.extend(depth)));
            geometry.faces.push(Face {
                corners: uv
                    .into_iter()
                    .enumerate()
                    .map(|(i, uv)| {
                        let mut corner = Corner::new(start + i);
                        corner.uv = uv;
                        corner
                    })
                    .collect(),
                material: slot,
                smooth: false,
            });
        }
        let object = Object {
            id: 0,
            source_node: None,
            name: "Occlusion".into(),
            geometry: geometry.into(),
            transform: Transform::IDENTITY,
            materials: Vec::new(),
            hidden: false,
        };
        let replay =
            PolygonReplay::new(&object, vec![("Rear".into(), 0..1), ("Front".into(), 1..2)]);
        let mut hit = DecalHit {
            value: 1.,
            granularity: 2,
            points: vec![[0.6, 0.6]],
            proj_view_inv: Mat4::IDENTITY.to_cols_array(),
            through: false,
            uv_space: false,
            duplicate: None,
        };
        assert!(replay.triangles(&hit, 0, 128).is_empty());
        assert!(!replay.triangles(&hit, 1, 128).is_empty());
        hit.through = true;
        assert!(!replay.triangles(&hit, 0, 128).is_empty());
        assert!(replay.triangles(&hit, 2, 128).is_empty());
    }
}
