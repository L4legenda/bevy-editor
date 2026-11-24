//! Resolve screen positions to the visible triangle for projected brush stamps.
use super::{CameraMap, FOV, FillMode, Stamp};
use crate::editor::{
    model_document::Object,
    ui::model_viewer::editing::input::{Bvh, PickTriangle},
};
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub(super) struct Attributes {
    pub slot: usize,
    pub points: [Vec3; 3],
    normals: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub vertices: [usize; 3],
}

#[path = "model_paint_fill.rs"]
mod fill;
#[path = "model_paint_spp_polygon.rs"]
mod spp_polygon;
pub(in crate::editor) use spp_polygon::PolygonReplay;
#[path = "model_paint_spp_strokes.rs"]
mod spp_strokes;
pub(in crate::editor) use spp_strokes::{Stencil, StencilCache, StrokeCoverage};

pub(super) struct SurfaceTrace {
    bvh: Bvh,
    attributes: Vec<Attributes>,
    topology: Option<fill::Topology>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct FillTarget {
    pub view: super::gpu::View,
    pub object: usize,
    pub slot: usize,
    pub mode: FillMode,
    pub angle: f32,
}

#[derive(Clone, Copy)]
pub(super) struct OutlineEdge {
    pub world: [Vec3; 2],
    pub uv: [Vec2; 2],
}

#[derive(Clone, Copy, Default)]
pub(super) struct Placement {
    pub center: Vec3,
    pub radius: f32,
    pub normal: Vec3,
    pub tangent: Vec3,
    pub uv: Vec2,
    pub uv_radius: f32,
    pub reflected: bool,
}

impl SurfaceTrace {
    #[cfg(test)]
    pub fn prepare_fill(objects: &[Object]) -> Self {
        Self::build(objects, &super::sets::SetVisibility::default(), true)
    }

    pub fn build(objects: &[Object], visibility: &super::sets::SetVisibility, fill: bool) -> Self {
        let mut triangles = Vec::new();
        let mut attributes = Vec::new();
        for object in objects.iter().filter(|o| !o.hidden) {
            let scale = object.transform.scale;
            let inverse_scale = scale.signum() / scale.abs().max(Vec3::splat(1e-6));
            for (index, face) in object.geometry.faces.iter().enumerate() {
                if !visibility.shown(object.id, face.material) {
                    continue;
                }
                let face_normal = object.geometry.normal(face);
                let uv_offset = object.uv_offset(face.material);
                for indices in object.geometry.triangles(face) {
                    let corners = indices.map(|i| &face.corners[i]);
                    let points = corners.map(|c| {
                        object
                            .transform
                            .transform_point(object.geometry.positions[c.vertex])
                    });
                    let normals = corners.map(|c| {
                        object.transform.rotation
                            * (c.source_normal.unwrap_or(face_normal) * inverse_scale)
                                .normalize_or_zero()
                    });
                    triangles.push(PickTriangle {
                        object: object.id,
                        face: index,
                        points,
                    });
                    attributes.push(Attributes {
                        slot: face.material,
                        points,
                        normals,
                        uv: corners.map(|c| c.uv - uv_offset),
                        vertices: corners.map(|c| c.vertex),
                    });
                }
            }
        }
        let topology =
            fill.then(|| fill::Topology::build(objects, visibility, &triangles, &attributes));
        Self {
            bvh: Bvh::from_triangles(triangles),
            attributes,
            topology,
        }
    }

    pub fn supports_fill(&self) -> bool {
        self.topology.is_some()
    }

    pub fn fill_ranges(
        &self,
        view: super::gpu::View,
        object: usize,
        slot: usize,
        mode: FillMode,
        angle: f32,
        stamps: &[Stamp],
    ) -> Vec<std::ops::Range<u32>> {
        let target = FillTarget {
            view,
            object,
            slot,
            mode,
            angle,
        };
        self.ranges(
            self.selected(
                target,
                stamps
                    .iter()
                    .filter_map(|stamp| self.fill_hit(target, stamp.point)),
            ),
        )
    }

    fn is_target(&self, target: FillTarget, index: usize) -> bool {
        self.bvh.triangle(index).object == target.object
            && self.attributes[index].slot == target.slot
    }

    fn fill_hit(&self, target: FillTarget, point: Vec2) -> Option<(usize, Vec3)> {
        let view = target.view;
        if !view.camera.rect.contains(point) {
            return None;
        }
        if view.uv {
            let uv = (point - view.camera.rect.min) / view.camera.rect.size();
            self.topology
                .as_ref()?
                .pick_uv(uv, &self.attributes, |i| self.is_target(target, i))
        } else {
            self.pick_surface(view.camera, point, target.object, target.slot)
        }
    }

    pub fn pick_surface(
        &self,
        camera: CameraMap,
        point: Vec2,
        object: usize,
        slot: usize,
    ) -> Option<(usize, Vec3)> {
        let (origin, direction) = camera.ray(point);
        self.bvh
            .cast_triangle(origin, direction, |_| true)
            .filter(|&(i, _)| {
                self.bvh.triangle(i).object == object && self.attributes[i].slot == slot
            })
            .map(|(i, distance)| (i, origin + direction * distance))
    }

    fn selected(
        &self,
        target: FillTarget,
        hits: impl IntoIterator<Item = (usize, Vec3)>,
    ) -> std::collections::BTreeSet<usize> {
        let Some(topology) = &self.topology else {
            return Default::default();
        };
        let mut selected = std::collections::BTreeSet::new();
        let min_dot = target.angle.to_radians().cos();
        for (index, point) in hits {
            if target.mode == FillMode::Loop || !selected.contains(&index) {
                selected.extend(topology.select(
                    index,
                    point,
                    target.mode,
                    min_dot,
                    &self.attributes,
                ));
            }
        }
        selected.retain(|&i| self.is_target(target, i));
        selected
    }

    fn ranges(&self, indices: impl IntoIterator<Item = usize>) -> Vec<std::ops::Range<u32>> {
        let mut ranges: Vec<std::ops::Range<u32>> = Vec::new();
        for index in indices {
            let start = index as u32 * 3;
            match ranges.last_mut() {
                Some(last) if last.end == start => last.end += 3,
                _ => ranges.push(start..start + 3),
            }
        }
        ranges
    }

    pub fn fill_padding(
        &self,
        ranges: &[std::ops::Range<u32>],
        pixels: f32,
        size: u32,
    ) -> Vec<Vec2> {
        self.topology.as_ref().map_or_else(Vec::new, |topology| {
            topology.padding(
                ranges
                    .iter()
                    .flat_map(|r| (r.start / 3..r.end / 3).map(|i| i as usize)),
                &self.attributes,
                pixels / size as f32,
            )
        })
    }

    pub fn fill_region(
        &self,
        target: FillTarget,
        region: &super::fill_drag::Region,
    ) -> Vec<std::ops::Range<u32>> {
        let view = target.view;
        let camera = view.camera;
        let hits = self
            .attributes
            .iter()
            .enumerate()
            .filter_map(|(index, triangle)| {
                if !self.is_target(target, index) {
                    return None;
                }
                let [a, b, c] = triangle.points;
                let center = (a + b + c) / 3.;
                let position = if view.uv {
                    let [a, b, c] = triangle.uv;
                    camera.rect.min + (a + b + c) / 3. * camera.rect.size()
                } else {
                    camera.project(center)?
                };
                if !region.contains(position) {
                    return None;
                }
                if !view.uv && !region.through {
                    let (origin, direction) = camera.ray(position);
                    let depth = (center - origin).dot(direction);
                    let tolerance = camera.epsilon.max(depth.abs() * 0.001);
                    if self
                        .bvh
                        .cast_triangle(origin, direction, |_| true)
                        .is_some_and(|(hit, near)| hit != index && near < depth - tolerance)
                    {
                        return None;
                    }
                }
                Some((index, center))
            });
        self.ranges(self.selected(target, hits))
    }

    pub fn fill_outline(&self, target: FillTarget, position: Vec2) -> Vec<OutlineEdge> {
        let mut edges = std::collections::BTreeMap::new();
        for index in self.selected(target, self.fill_hit(target, position)) {
            let triangle = &self.attributes[index];
            let [a, b, c] = triangle.points;
            let camera = target.view.camera;
            let toward_eye = if camera.ortho {
                -*camera.pose.forward()
            } else {
                camera.pose.translation - (a + b + c) / 3.
            };
            let front = (b - a).cross(c - a).dot(toward_eye) > 0.;
            for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                let key = |k: usize| {
                    let uv = if target.view.uv {
                        (triangle.uv[k] * 100_000.).round().as_ivec2().to_array()
                    } else {
                        [0; 2]
                    };
                    (triangle.vertices[k], uv)
                };
                let (a, b) = (key(i), key(j));
                let edge = OutlineEdge {
                    world: [triangle.points[i], triangle.points[j]],
                    uv: [triangle.uv[i], triangle.uv[j]],
                };
                edges
                    .entry((a.min(b), a.max(b)))
                    .and_modify(|(_, count, facing): &mut (OutlineEdge, usize, [bool; 2])| {
                        *count += 1;
                        facing[usize::from(front)] = true;
                    })
                    .or_insert((edge, 1, [!front, front]));
            }
        }
        edges
            .into_values()
            .filter_map(|(edge, count, facing)| {
                let silhouette = !target.view.uv && facing[0] && facing[1];
                (count == 1 || silhouette).then_some(edge)
            })
            .collect()
    }

    fn hit(
        &self,
        origin: Vec3,
        direction: Vec3,
        object: usize,
        slot: usize,
        reflected: bool,
    ) -> Option<(Vec3, Vec3, Vec2, f32)> {
        let (index, distance) = self
            .bvh
            .cast_triangle(origin, direction, |t| !reflected || t.object == object)?;
        let triangle = &self.attributes[index];
        if self.bvh.triangle(index).object != object || triangle.slot != slot {
            return None;
        }
        let center = origin + direction * distance;
        let a = triangle.points[1] - triangle.points[0];
        let b = triangle.points[2] - triangle.points[0];
        let offset = center - triangle.points[0];
        let normal = a.cross(b);
        let numerator = normal.length_squared();
        if numerator < 1e-20 {
            return None;
        }
        let u = offset.cross(b).dot(normal) / numerator;
        let v = a.cross(offset).dot(normal) / numerator;
        let w = 1. - u - v;
        let uv = triangle.uv[0] * w + triangle.uv[1] * u + triangle.uv[2] * v;
        let n = (triangle.normals[0] * w + triangle.normals[1] * u + triangle.normals[2] * v)
            .try_normalize()
            .unwrap_or_else(|| normal.normalize());
        let uv_area = (triangle.uv[1] - triangle.uv[0])
            .perp_dot(triangle.uv[2] - triangle.uv[0])
            .abs();
        Some((center, n, uv, (uv_area / numerator.sqrt()).sqrt()))
    }

    pub fn resolve(
        &self,
        camera: CameraMap,
        object: usize,
        slot: usize,
        stamp: Stamp,
        brush: super::Brush,
    ) -> [Option<Placement>; 2] {
        let (origin, direction) = camera.ray(stamp.point);
        let Some((center, normal, uv, density)) = self.hit(origin, direction, object, slot, false)
        else {
            return [None, None];
        };
        let height = if camera.ortho {
            camera.height
        } else {
            let depth = (center - camera.pose.translation).dot(*camera.pose.forward());
            2. * depth.max(0.0001) * (FOV * 0.5).tan()
        };
        let radius = (brush.radius * stamp.radius).max(0.5) * height / camera.rect.height();
        let right = *camera.pose.right();
        let tangent = (right - normal * normal.dot(right))
            .try_normalize()
            .unwrap_or_else(|| normal.any_orthonormal_vector());
        let first = Placement {
            center,
            radius,
            normal,
            tangent,
            uv,
            uv_radius: radius * density,
            reflected: false,
        };
        let plane = Vec3::from_array(brush.mirror[..3].try_into().unwrap());
        if plane == Vec3::ZERO {
            return [Some(first), None];
        }
        let reflect = |p: Vec3| p - 2. * plane * plane.dot(p);
        let reflected_center = reflect(center) + 2. * plane * brush.mirror[3];
        let reflected_normal = reflect(normal);
        let mut reflected = Placement {
            center: reflected_center,
            normal: reflected_normal,
            tangent: reflect(tangent),
            reflected: true,
            ..first
        };
        if brush.projection == super::BrushProjection::Uv {
            let hit = self
                .hit(
                    reflected_center + reflected_normal * radius,
                    -reflected_normal,
                    object,
                    slot,
                    true,
                )
                .filter(|(p, _, _, _)| p.distance(reflected_center) < 2. * radius);
            let Some((_, _, uv, density)) = hit else {
                return [Some(first), None];
            };
            reflected.uv = uv;
            reflected.uv_radius = radius * density;
        }
        [Some(first), Some(reflected)]
    }
}
