//! Signed surface curvature and exact distance to geometric feature segments.
use super::{Bounds, Progress};
use crate::editor::model_bake_settings::SurfaceOptions;
use crate::editor::model_mesh::Geometry;
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(super) struct Segment {
    pub a: Vec3,
    pub b: Vec3,
}
impl Segment {
    fn distance_squared(self, point: Vec3) -> f32 {
        let direction = self.b - self.a;
        let t = ((point - self.a).dot(direction) / direction.length_squared()).clamp(0., 1.);
        point.distance_squared(self.a + direction * t)
    }
}

struct Node {
    min: Vec3,
    max: Vec3,
    children: Option<[usize; 2]>,
    range: std::ops::Range<usize>,
}
impl Node {
    fn distance_squared(&self, point: Vec3) -> f32 {
        (self.min - point)
            .max(point - self.max)
            .max(Vec3::ZERO)
            .length_squared()
    }
}

#[derive(Default)]
struct SegmentTree {
    segments: Vec<Segment>,
    nodes: Vec<Node>,
}
impl SegmentTree {
    fn build(segments: Vec<Segment>) -> Self {
        let mut tree = Self {
            segments,
            nodes: Vec::new(),
        };
        if !tree.segments.is_empty() {
            tree.split(0..tree.segments.len());
        }
        tree
    }

    fn split(&mut self, range: std::ops::Range<usize>) -> usize {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for segment in &self.segments[range.clone()] {
            min = min.min(segment.a).min(segment.b);
            max = max.max(segment.a).max(segment.b);
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            children: None,
            range: range.clone(),
        });
        if range.len() > 8 {
            let extent = max - min;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let split = range.len() / 2;
            self.segments[range.clone()].select_nth_unstable_by(split, |a, b| {
                (a.a[axis] + a.b[axis]).total_cmp(&(b.a[axis] + b.b[axis]))
            });
            let mid = range.start + split;
            let left = self.split(range.start..mid);
            let right = self.split(mid..range.end);
            self.nodes[index].children = Some([left, right]);
        }
        index
    }

    fn distance(&self, point: Vec3, reach: f32) -> f32 {
        let mut best = reach * reach;
        if !self.nodes.is_empty() {
            self.nearest(0, point, &mut best);
        }
        best.sqrt()
    }

    fn nearest(&self, index: usize, point: Vec3, best: &mut f32) {
        let node = &self.nodes[index];
        if node.distance_squared(point) >= *best {
            return;
        }
        if let Some([a, b]) = node.children {
            let (first, second) =
                if self.nodes[a].distance_squared(point) <= self.nodes[b].distance_squared(point) {
                    (a, b)
                } else {
                    (b, a)
                };
            self.nearest(first, point, best);
            self.nearest(second, point, best);
        } else {
            for segment in &self.segments[node.range.clone()] {
                *best = best.min(segment.distance_squared(point));
            }
        }
    }
}

pub(super) struct Features {
    pub curvature: Vec<Vec<f32>>,
    edges: Edges,
}

pub(super) type Segments = HashMap<usize, [Vec<Segment>; 2]>;

pub(super) struct Edges {
    folds: Field,
    curves: Field,
    threshold: f32,
}

struct Field {
    segments: HashMap<usize, [SegmentTree; 2]>,
    params: Parameters,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor::ui::model_viewer::paint::bake) struct Parameters {
    pub scale: f32,
    pub reach: f32,
    pub contrast: f32,
    pub sharpness: f32,
    pub hard_cosine: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Curvature {
    pub value: f32,
    lines: [u16; 2],
}

impl Curvature {
    pub fn packed(self, value: f32) -> u8 {
        let [ridge, fold] = self.lines.map(|weight| f32::from(weight) / 65535.);
        let signed = value.max(ridge) - (-value).max(fold);
        super::byte(0.5 + signed.clamp(-1., 1.) * 0.5)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub curvature: Curvature,
    pub distance: [f32; 2],
}

impl Default for Sample {
    fn default() -> Self {
        Self { curvature: Curvature::default(), distance: [1.; 2] }
    }
}

impl Parameters {
    pub fn new(bounds: Bounds, scale: f32, options: SurfaceOptions) -> Self {
        let extent = bounds.max - bounds.min;
        let unit = bounds.custom_unit();
        let mut contrast = scale * options.contrast as f32 / 10_000.;
        let reach = if options.auto_edge_reach {
            let reach = (extent.min_element() * 0.1).clamp(unit * 0.005, unit * 0.2);
            contrast *= reach / (unit * 0.2);
            reach
        } else {
            scale * options.edge_reach as f32 / 10_000.
        };
        Self {
            scale,
            reach,
            contrast,
            sharpness: f32::from(options.sharpness) / 1000.,
            hard_cosine: (f32::from(options.hard_angle) / 10.).to_radians().cos(),
            radius: f32::from(options.radius) / 10.,
        }
    }
}

pub(super) fn hard_fold(a: Vec3, b: Vec3, between_centers: Vec3, shallow: f32) -> Option<usize> {
    let folded_back = 170.0_f32.to_radians().cos();
    (a != Vec3::ZERO && b != Vec3::ZERO && (folded_back..=shallow).contains(&a.dot(b)))
        .then(|| usize::from(b.dot(between_centers) >= 0.))
}

impl Features {
    pub fn build(
        mesh: &Geometry,
        positions: &[Vec3],
        normals: &[Vec<Vec3>],
        geometric: &[Vec3],
        parts: &[usize],
        params: Parameters,
        progress: &Progress,
    ) -> Result<Self, String> {
        let mut groups_by_vertex = vec![Vec::<(Vec3, usize)>::new(); positions.len()];
        let mut groups = Vec::with_capacity(mesh.faces.len());
        let mut curvature = Vec::<(f32, u32)>::new();
        let mut centers = Vec::with_capacity(mesh.faces.len());
        for (face, corner_normals) in mesh.faces.iter().zip(normals) {
            progress.check()?;
            let mut group = Vec::with_capacity(face.corners.len());
            let mut center = Vec3::ZERO;
            for (corner, &normal) in face.corners.iter().zip(corner_normals) {
                center += positions[corner.vertex];
                let entries = &mut groups_by_vertex[corner.vertex];
                let group_id = entries
                    .iter()
                    .find(|(other, _)| other.dot(normal) >= 0.99985)
                    .map(|&(_, index)| index)
                    .unwrap_or_else(|| {
                        let index = curvature.len();
                        curvature.push((0., 0));
                        entries.push((normal, index));
                        index
                    });
                group.push(group_id);
            }
            centers.push(center / face.corners.len() as f32);
            groups.push(group);
            for i in 0..face.corners.len() {
                let j = (i + 1) % face.corners.len();
                let delta = positions[face.corners[j].vertex] - positions[face.corners[i].vertex];
                let length_squared = delta.length_squared();
                if length_squared <= 1e-20 {
                    continue;
                }
                let sample = (corner_normals[j] - corner_normals[i]).dot(delta) / length_squared;
                let face_groups = groups.last().unwrap();
                for index in [face_groups[i], face_groups[j]] {
                    let accumulated = &mut curvature[index];
                    accumulated.0 += sample;
                    accumulated.1 += 1;
                }
            }
        }
        let curvature: Vec<Vec<f32>> = groups
            .into_iter()
            .map(|face| {
                face.into_iter()
                    .map(|index| {
                        let (sum, count) = curvature[index];
                        sum / count.max(1) as f32
                    })
                    .collect()
            })
            .collect();

        let mut folds = Segments::new();
        for (edge, adjacent) in mesh.edges() {
            progress.check()?;
            let [a, b] = adjacent.as_slice() else {
                continue;
            };
            let forward = |face: usize| {
                mesh.faces[face]
                    .edges()
                    .any(|(i, j)| i == edge.0 && j == edge.1)
            };
            let na = geometric[*a];
            let nb = if forward(*a) == forward(*b) {
                -geometric[*b]
            } else {
                geometric[*b]
            };
            let Some(kind) = hard_fold(na, nb, centers[*a] - centers[*b], params.hard_cosine) else {
                continue;
            };
            let normal_at = |face: usize, vertex| {
                mesh.faces[face]
                    .corners
                    .iter()
                    .position(|corner| corner.vertex == vertex)
                    .map(|corner| normals[face][corner])
                    .unwrap_or(Vec3::ZERO)
            };
            if [edge.0, edge.1]
                .into_iter()
                .all(|vertex| normal_at(*a, vertex).dot(normal_at(*b, vertex)) >= 0.999)
            {
                continue;
            }
            let segment = Segment {
                a: positions[edge.0],
                b: positions[edge.1],
            };
            if segment.a.distance_squared(segment.b) <= 1e-20 {
                continue;
            }
            folds.entry(parts[*a]).or_default()[kind].push(segment);
        }
        // A smooth bevel is a feature when its curvature corresponds to a
        // radius within the search distance, regardless of the hard-edge angle.
        let threshold = (1.25 * params.reach).recip();
        let mut curves = Segments::new();
        for (face, polygon) in mesh.faces.iter().enumerate() {
            progress.check()?;
            let values = &curvature[face];
            if !values.iter().any(|value| value.abs() >= threshold) {
                continue;
            }
            for triangle in mesh.triangles(polygon) {
                let points = triangle.map(|i| positions[polygon.corners[i].vertex]);
                add_contours(&mut curves, parts[face], points, triangle.map(|i| values[i]), threshold);
            }
        }
        Ok(Self {
            curvature,
            edges: Edges::build(folds, curves, threshold, params, progress)?,
        })
    }

    pub fn sample(&self, part: usize, point: Vec3, curvature: f32, line_width: f32) -> Sample {
        self.edges.sample(part, point, curvature, line_width)
    }
}

pub(super) fn add_contours(
    curves: &mut Segments,
    part: usize,
    points: [Vec3; 3],
    values: [f32; 3],
    threshold: f32,
) {
    for (kind, sign) in [1., -1.].into_iter().enumerate() {
        let values = values.map(|value| value * sign);
        let mut crossings = (0..3).filter_map(|i| {
            let j = (i + 1) % 3;
            let a = values[i];
            let b = values[j];
            ((a < threshold) != (b < threshold))
                .then(|| points[i].lerp(points[j], (threshold - a) / (b - a)))
        });
        if let (Some(a), Some(b)) = (crossings.next(), crossings.next())
            && a.distance_squared(b) > 1e-20
        {
            curves.entry(part).or_default()[kind].push(Segment { a, b });
        }
    }
}

impl Edges {
    pub fn build(
        folds: Segments,
        curves: Segments,
        threshold: f32,
        params: Parameters,
        progress: &Progress,
    ) -> Result<Self, String> {
        Ok(Self {
            folds: Field::build(folds, params, progress)?,
            curves: Field::build(curves, params, progress)?,
            threshold,
        })
    }

    pub fn sample(&self, part: usize, point: Vec3, curvature: f32, width: f32) -> Sample {
        let folds = self.folds.distances(part, point);
        let curves = self.curves.distances(part, point);
        let distances = std::array::from_fn(|i| {
            let sign = if i == 0 { 1. } else { -1. };
            if curvature * sign >= self.threshold {
                0.
            } else {
                folds[i].min(curves[i])
            }
        });
        self.folds.channels(curvature, width, folds, distances)
    }
}

impl Field {
    fn build(
        features: Segments,
        params: Parameters,
        progress: &Progress,
    ) -> Result<Self, String> {
        let mut segments = HashMap::with_capacity(features.len());
        for (part, features) in features {
            progress.check()?;
            segments.insert(part, features.map(SegmentTree::build));
        }
        Ok(Self { segments, params })
    }

    fn distances(&self, part: usize, point: Vec3) -> [f32; 2] {
        let reach = self.params.reach;
        self.segments.get(&part).map_or([reach; 2], |trees| {
            [
                trees[0].distance(point, reach),
                trees[1].distance(point, reach),
            ]
        })
    }

    fn channels(
        &self,
        curvature: f32,
        line_width: f32,
        [ridge, fold]: [f32; 2],
        [edge, crease]: [f32; 2],
    ) -> Sample {
        let reach = self.params.reach;
        let line_width = line_width * (self.params.radius * 0.5);
        let line = |distance: f32| {
            let t = (distance / line_width.max(self.params.scale * 1e-7).min(reach)).clamp(0., 1.);
            1. - t * t * (3. - 2. * t)
        };
        Sample {
            curvature: Curvature {
                value: (curvature * self.params.contrast).clamp(-1., 1.),
                lines: [ridge, fold].map(|distance| (line(distance) * 65535.).round() as u16),
            },
            distance: [edge / reach, crease / reach],
        }
    }
}
