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
    pub fn build(doc: &Document) -> Self {
        Self::from_objects(&doc.objects)
    }
    pub fn from_objects(objects:&[crate::editor::model_document::Object])->Self {
        let mut this = Self::default();
        for o in objects {
            if o.hidden {
                continue;
            }
            for (i, face) in o.geometry.faces.iter().enumerate() {
                for t in o.geometry.triangles(face) {
                    this.triangles.push(Triangle {
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
        this.order = (0..this.triangles.len()).collect();
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
        self.cast_where(origin, direction, |t| object.is_none_or(|id| id == t.object))
    }

    pub fn cast_where(
        &self,
        origin: Vec3,
        direction: Vec3,
        accepts: impl Fn(&Triangle) -> bool,
    ) -> Option<(usize, usize, f32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut closest = f32::INFINITY;
        let mut hit = None;
        // This balanced BVH is bounded by log2(triangle_count); avoid a heap
        // allocation for every texel visibility query during painting.
        let mut stack = [0;64];
        let mut len=1;
        while len>0 {
            len-=1;let i=stack[len];
            let node = &self.nodes[i];
            if !slab(origin, direction, node.min, node.max, closest) {
                continue;
            }
            if let Some((a, b)) = node.children {
                stack[len]=a;stack[len+1]=b;len+=2;
            } else {
                for &index in &self.order[node.start..node.end] {
                    let t = &self.triangles[index];
                    if !accepts(t) {
                        continue;
                    }
                    if let Some(distance) =
                        intersect(origin, direction, t.points).filter(|&d| d < closest)
                    {
                        closest = distance;
                        hit = Some((t.object, t.face, distance));
                    }
                }
            }
        }
        hit
    }
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
