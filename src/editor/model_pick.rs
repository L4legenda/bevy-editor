//! CPU BVH for model/component picking; rebuilt only when document geometry changes.
use crate::editor::model_document::Document;
use bevy::prelude::*;

#[derive(Clone, Copy)]
pub(in crate::editor) struct Triangle {
    pub object: usize,
    pub face: usize,
    pub points: [Vec3; 3],
}
#[derive(Clone)]
struct Node {
    min: Vec3,
    max: Vec3,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}
#[derive(Default, Clone)]
pub(in crate::editor) struct Bvh {
    triangles: Vec<Triangle>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
impl Bvh {
    pub(in crate::editor) fn compute_nodes(&self) -> impl ExactSizeIterator<Item = ([Vec3; 2], [u32; 4])> + '_ {
        self.nodes.iter().map(|node| {
            let links = if let Some((left, right)) = node.children {
                [left as u32, right as u32, 0, 0]
            } else {
                [u32::MAX, u32::MAX, node.start as u32, node.end as u32]
            };
            ([node.min, node.max], links)
        })
    }

    pub(in crate::editor) fn compute_triangles(&self) -> impl ExactSizeIterator<Item = (usize, &Triangle)> + '_ {
        self.order.iter().map(|&index| (index, &self.triangles[index]))
    }

    pub(super) fn triangles(&self) -> &[Triangle] {
        &self.triangles
    }
    pub(in crate::editor) fn triangle(&self, index: usize) -> &Triangle {
        &self.triangles[index]
    }

    pub fn build(doc: &Document) -> Self {
        Self::from_objects(&doc.objects)
    }
    pub fn from_objects(objects: &[crate::editor::model_document::Object]) -> Self {
        Self::from_objects_where(objects, |_, _| true)
    }
    pub fn from_objects_where(
        objects: &[crate::editor::model_document::Object],
        shown: impl Fn(usize, usize) -> bool,
    ) -> Self {
        let mut triangles = Vec::new();
        for o in objects {
            if o.hidden {
                continue;
            }
            for (i, face) in o.geometry.faces.iter().enumerate() {
                if !shown(o.id, face.material) {
                    continue;
                }
                for t in o.geometry.triangles(face) {
                    triangles.push(Triangle {
                        object: o.id,
                        face: i,
                        points: t.map(|j| {
                            o.transform
                                .transform_point(o.geometry.positions[face.corners[j].vertex])
                        }),
                    });
                }
            }
        }
        Self::from_triangles(triangles)
    }
    pub(in crate::editor) fn from_triangles(triangles: Vec<Triangle>) -> Self {
        let order = (0..triangles.len()).collect();
        let mut this = Self {
            triangles,
            order,
            nodes: Vec::new(),
        };
        if !this.order.is_empty() {
            this.branch(0, this.order.len());
        }
        this
    }
    fn branch(&mut self, start: usize, end: usize) -> usize {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for &i in &self.order[start..end] {
            for &p in &self.triangles[i].points {
                min = min.min(p);
                max = max.max(p);
            }
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let span = max - min;
            let axis = if span.x > span.y && span.x > span.z {
                0
            } else if span.y > span.z {
                1
            } else {
                2
            };
            self.order[start..end].sort_by(|&a, &b| {
                let a = self.triangles[a]
                    .points
                    .iter()
                    .map(|p| p[axis])
                    .sum::<f32>();
                let b = self.triangles[b]
                    .points
                    .iter()
                    .map(|p| p[axis])
                    .sum::<f32>();
                a.total_cmp(&b)
            });
            let mid = (start + end) / 2;
            let left = self.branch(start, mid);
            let right = self.branch(mid, end);
            self.nodes[index].children = Some((left, right));
        }
        index
    }
    pub fn cast(
        &self,
        origin: Vec3,
        direction: Vec3,
        object: Option<usize>,
    ) -> Option<(usize, usize, f32)> {
        self.cast_where(origin, direction, |t| {
            object.is_none_or(|id| id == t.object)
        })
    }

    pub fn cast_where(
        &self,
        origin: Vec3,
        direction: Vec3,
        accepts: impl Fn(&Triangle) -> bool,
    ) -> Option<(usize, usize, f32)> {
        let (index, distance) = self.cast_triangle(origin, direction, accepts)?;
        let triangle = &self.triangles[index];
        Some((triangle.object, triangle.face, distance))
    }

    pub(in crate::editor) fn cast_triangle(
        &self,
        origin: Vec3,
        direction: Vec3,
        accepts: impl Fn(&Triangle) -> bool,
    ) -> Option<(usize, f32)> {
        self.cast_triangle_up_to(origin, direction, f32::INFINITY, accepts)
    }

    pub(in crate::editor) fn cast_triangle_up_to(
        &self,
        origin: Vec3,
        direction: Vec3,
        limit: f32,
        accepts: impl Fn(&Triangle) -> bool,
    ) -> Option<(usize, f32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut closest = limit;
        let mut hit = None;
        // This balanced BVH is bounded by log2(triangle_count); avoid a heap
        // allocation for every texel visibility query during painting.
        let mut stack = [0; 64];
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let i = stack[len];
            let node = &self.nodes[i];
            if !slab(origin, direction, node.min, node.max, closest) {
                continue;
            }
            if let Some((a, b)) = node.children {
                stack[len] = a;
                stack[len + 1] = b;
                len += 2;
            } else {
                for &index in &self.order[node.start..node.end] {
                    let t = &self.triangles[index];
                    if !accepts(t) {
                        continue;
                    }
                    if let Some(distance) = intersect(origin, direction, t.points)
                        .filter(|&d| d < closest || (hit.is_none() && d == limit && d.is_finite()))
                    {
                        closest = distance;
                        hit = Some((index, distance));
                    }
                }
            }
        }
        hit
    }

    pub(in crate::editor) fn ray_triangles(&self, origin: Vec3, direction: Vec3) -> Vec<usize> {
        let mut hits = Vec::new();
        let mut pending = Vec::new();
        if !self.nodes.is_empty() {
            pending.push(0);
        }
        while let Some(index) = pending.pop() {
            let node = &self.nodes[index];
            if !slab(origin, direction, node.min, node.max, f32::INFINITY) {
                continue;
            }
            if let Some((left, right)) = node.children {
                pending.extend([left, right]);
            } else {
                hits.extend(self.order[node.start..node.end].iter().copied().filter(|&index| {
                    intersect(origin, direction, self.triangles[index].points).is_some()
                }));
            }
        }
        hits
    }

    pub(in crate::editor) fn triangles_in_box(&self, min: Vec3, max: Vec3, result: &mut Vec<usize>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = [0; 64];
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let node = &self.nodes[stack[len]];
            if node.max.cmplt(min).any() || node.min.cmpgt(max).any() {
                continue;
            }
            if let Some((left, right)) = node.children {
                stack[len] = left;
                stack[len + 1] = right;
                len += 2;
            } else {
                result.extend_from_slice(&self.order[node.start..node.end]);
            }
        }
    }

    pub(in crate::editor) fn nearest_surface_where(
        &self,
        point: Vec3,
        reach: f32,
        accepts: impl Fn(&Triangle) -> bool,
    ) -> Option<(usize, Vec3)> {
        let root = self.nodes.first()?;
        let distance = |node: &Node| point.distance_squared(point.clamp(node.min, node.max));
        let mut closest = reach * reach;
        let mut result = None;
        let mut stack = [(0, distance(root)); 64];
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let (index, lower_bound) = stack[len];
            if lower_bound > closest {
                continue;
            }
            let node = &self.nodes[index];
            if let Some((a, b)) = node.children {
                let mut children = [(a, distance(&self.nodes[a])), (b, distance(&self.nodes[b]))];
                if children[0].1 < children[1].1 {
                    children.swap(0, 1);
                }
                stack[len..len + 2].copy_from_slice(&children);
                len += 2;
            } else {
                for &triangle in &self.order[node.start..node.end] {
                    let source = &self.triangles[triangle];
                    if !accepts(source) {
                        continue;
                    }
                    let nearest = closest_point(point, source.points);
                    let squared = point.distance_squared(nearest);
                    if squared <= closest {
                        closest = squared;
                        result = Some((triangle, nearest));
                    }
                }
            }
        }
        result
    }
}

fn closest_point(point: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
    let normal = (b - a).cross(c - a).normalize_or_zero();
    let projected = point - normal * (point - a).dot(normal);
    let edges = [(a, b), (b, c), (c, a)];
    if normal != Vec3::ZERO
        && edges
            .iter()
            .all(|&(s, e)| (e - s).cross(projected - s).dot(normal) >= 0.)
    {
        return projected;
    }
    edges
        .into_iter()
        .map(|(s, e)| {
            let edge = e - s;
            if edge == Vec3::ZERO {
                s
            } else {
                s + edge * ((point - s).dot(edge) / edge.length_squared()).clamp(0., 1.)
            }
        })
        .min_by(|a, b| {
            point
                .distance_squared(*a)
                .total_cmp(&point.distance_squared(*b))
        })
        .unwrap()
}
fn slab(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3, limit: f32) -> bool {
    let mut near = 0.0_f32;
    let mut far = limit;
    for i in 0..3 {
        if direction[i].abs() < 1.0e-12 {
            if origin[i] < min[i] || origin[i] > max[i] {
                return false;
            }
        } else {
            let a = (min[i] - origin[i]) / direction[i];
            let b = (max[i] - origin[i]) / direction[i];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if far < near {
                return false;
            }
        }
    }
    true
}
fn intersect(origin: Vec3, direction: Vec3, p: [Vec3; 3]) -> Option<f32> {
    let a = p[1] - p[0];
    let b = p[2] - p[0];
    let q = direction.cross(b);
    let det = a.dot(q);
    if det.abs() < 1.0e-12 {
        return None;
    }
    let d = origin - p[0];
    let u = d.dot(q) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let r = d.cross(a);
    let v = direction.dot(r) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = b.dot(r) / det;
    (t >= 0.0).then_some(t)
}
