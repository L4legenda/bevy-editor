//! High-poly curvature and feature distances evaluated at the projection hit.
use super::super::{
    Bounds,
    relief::{Field, Segment, hard_fold},
    root,
};
use super::{Hit, Progress, Projection, SourceTriangle, SourceUse, fit::position_key};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

const NO_GROUP: u32 = u32::MAX;
const EDGE_THRESHOLD: f32 = 0.35;

pub(in crate::editor::ui::model_viewer::paint::bake) struct HighRelief {
    parts: Vec<usize>,
    bounds: HashMap<usize, Bounds>,
    details: Option<ReliefDetails>,
}

struct ReliefDetails {
    corners: Vec<[f32; 3]>,
    folds: Field,
    curved: Field,
    threshold: f32,
}

#[derive(Default)]
pub(super) struct Cache {
    parts: Option<Arc<HighRelief>>,
    full: Option<(u32, Arc<HighRelief>)>,
}

impl Cache {
    fn get(&self, scale: u32, details: bool) -> Option<Arc<HighRelief>> {
        if details {
            self.full
                .as_ref()
                .filter(|(size, _)| *size == scale)
                .map(|(_, relief)| relief.clone())
        } else {
            self.full
                .as_ref()
                .map(|(_, relief)| relief.clone())
                .or_else(|| self.parts.clone())
        }
    }
}

struct Vertex {
    index: u32,
    first_face: usize,
    first_group: u32,
}

struct Group {
    normal: Vec3,
    next: u32,
    curvature: f32,
    lengths: f32,
    count: u32,
}

struct EdgeUse {
    from: u32,
    to: u32,
    face: u32,
}

impl EdgeUse {
    fn key(&self) -> (u32, u32) {
        (self.from.min(self.to), self.from.max(self.to))
    }
}

impl Projection {
    pub(in crate::editor::ui::model_viewer::paint::bake) fn build_relief(
        &self,
        scale: f32,
        include_details: bool,
        progress: &Progress,
    ) -> Result<Arc<HighRelief>, String> {
        progress.check()?;
        let key = scale.to_bits();
        if let Some(relief) = self.source.relief.lock().unwrap().get(key, include_details) {
            #[cfg(test)]
            progress
                .high_relief_hit
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(relief);
        }
        let relief = Arc::new(HighRelief::build(
            &self.source.triangles,
            scale,
            include_details,
            progress,
        )?);
        progress.check()?;
        let mut cache = self.source.relief.lock().unwrap();
        if let Some(existing) = cache.get(key, include_details) {
            return Ok(existing);
        }
        if include_details {
            cache.parts = None;
            cache.full = Some((key, relief.clone()));
        } else {
            cache.parts = Some(relief.clone());
        }
        Ok(relief)
    }
}

impl HighRelief {
    fn build(
        triangles: &[SourceTriangle],
        scale: f32,
        include_details: bool,
        progress: &Progress,
    ) -> Result<Self, String> {
        let mut parents: Vec<_> = (0..triangles.len()).collect();
        let mut vertices = HashMap::new();
        let mut positions = Vec::new();
        let mut groups = Vec::<Group>::new();
        let mut corner_groups = Vec::new();
        let mut edges = Vec::new();
        if include_details {
            corner_groups.reserve_exact(triangles.len());
            edges.reserve_exact(
                triangles
                    .iter()
                    .filter(|triangle| triangle.usage == SourceUse::Project)
                    .count()
                    * 3,
            );
        }
        let minimum_length_squared = (scale * 1e-8).powi(2);
        for (face, triangle) in triangles.iter().enumerate() {
            if face % 4096 == 0 {
                progress.check()?;
            }
            if triangle.usage != SourceUse::Project {
                if include_details {
                    corner_groups.push([NO_GROUP; 3]);
                }
                continue;
            }
            let mut packed = [0; 3];
            let mut ids = [0; 3];
            for (i, (&point, &normal)) in triangle.points.iter().zip(&triangle.normals).enumerate()
            {
                let vertex = vertices
                    .entry((triangle.part, position_key(point)))
                    .or_insert_with(|| {
                        let index = positions.len() as u32;
                        if include_details {
                            positions.push(point);
                        }
                        Vertex {
                            index,
                            first_face: face,
                            first_group: NO_GROUP,
                        }
                    });
                let a = root(&mut parents, face);
                let b = root(&mut parents, vertex.first_face);
                parents[a.max(b)] = a.min(b);
                if !include_details {
                    continue;
                }
                packed[i] = vertex.index;

                let mut group = vertex.first_group;
                while group != NO_GROUP && groups[group as usize].normal.dot(normal) < 0.99985 {
                    group = groups[group as usize].next;
                }
                if group == NO_GROUP {
                    group = groups.len() as u32;
                    groups.push(Group {
                        normal,
                        next: vertex.first_group,
                        curvature: 0.,
                        lengths: 0.,
                        count: 0,
                    });
                    vertex.first_group = group;
                }
                ids[i] = group;
            }
            if !include_details {
                continue;
            }
            corner_groups.push(ids);
            for i in 0..3 {
                let j = (i + 1) % 3;
                edges.push(EdgeUse {
                    from: packed[i],
                    to: packed[j],
                    face: face as u32,
                });
                let delta = triangle.points[j] - triangle.points[i];
                let length_squared = delta.length_squared();
                if length_squared > minimum_length_squared {
                    let sample =
                        (triangle.normals[j] - triangle.normals[i]).dot(delta) / length_squared;
                    let length = length_squared.sqrt();
                    for group in [ids[i], ids[j]] {
                        let group = &mut groups[group as usize];
                        group.curvature += sample;
                        group.lengths += length;
                        group.count += 1;
                    }
                }
            }
        }
        drop(vertices);
        let parts: Vec<_> = (0..parents.len()).map(|i| root(&mut parents, i)).collect();
        drop(parents);

        let mut bounds = HashMap::<usize, Bounds>::new();
        for (face, triangle) in triangles.iter().enumerate() {
            if face % 4096 == 0 {
                progress.check()?;
            }
            if triangle.usage == SourceUse::Project {
                let part_bounds = bounds.entry(parts[face]).or_default();
                for &point in &triangle.points {
                    part_bounds.add(point);
                }
            }
        }
        if !include_details {
            return Ok(Self {
                parts,
                bounds,
                details: None,
            });
        }
        edges.sort_unstable_by_key(EdgeUse::key);
        let mut folds = HashMap::<usize, [Vec<Segment>; 2]>::new();
        let normal = |[a, b, c]: [Vec3; 3]| (b - a).cross(c - a).normalize_or_zero();
        for (index, neighbors) in edges.chunk_by(|a, b| a.key() == b.key()).enumerate() {
            if index % 4096 == 0 {
                progress.check()?;
            }
            let [a, b] = neighbors else { continue };
            let (first, second) = (a.face as usize, b.face as usize);
            let p = triangles[first].points;
            let q = triangles[second].points;
            let nb = if a.from == b.from {
                -normal(q)
            } else {
                normal(q)
            };
            let offset = (p[0] - q[0] + p[1] - q[1] + p[2] - q[2]) / 3.;
            if let Some(kind) = hard_fold(normal(p), nb, offset) {
                folds.entry(parts[first]).or_default()[kind].push(Segment {
                    a: positions[a.from as usize],
                    b: positions[a.to as usize],
                });
            }
        }
        drop(edges);
        drop(positions);

        let values = smooth(triangles, &corner_groups, &groups, scale, progress)?;
        drop(groups);
        let corners: Vec<_> = corner_groups
            .into_iter()
            .map(|ids| {
                if ids[0] == NO_GROUP {
                    [0.; 3]
                } else {
                    ids.map(|i| values[i as usize])
                }
            })
            .collect();
        let threshold = EDGE_THRESHOLD / (0.25 * scale);
        let mut curved = HashMap::<usize, [Vec<Segment>; 2]>::new();
        for (face, (triangle, values)) in triangles.iter().zip(&corners).enumerate() {
            if face % 4096 == 0 {
                progress.check()?;
            }
            if triangle.usage != SourceUse::Project {
                continue;
            }
            for (kind, sign) in [1., -1.].into_iter().enumerate() {
                if let Some(line) =
                    contour(triangle.points, values.map(|value| value * sign), threshold)
                {
                    curved.entry(parts[face]).or_default()[kind].push(line);
                }
            }
        }
        Ok(Self {
            parts,
            bounds,
            details: Some(ReliefDetails {
                corners,
                folds: Field::build(folds, scale, progress)?,
                curved: Field::build(curved, scale, progress)?,
                threshold,
            }),
        })
    }

    pub fn part(&self, triangle: usize) -> usize {
        self.parts[triangle]
    }

    pub fn component(&self, triangle: usize) -> (usize, Bounds) {
        let part = self.part(triangle);
        (part, self.bounds[&part])
    }

    pub fn sample(&self, hit: &Hit, width: f32) -> [f32; 3] {
        let Some(details) = &self.details else {
            return [0.5, 1., 1.];
        };
        let part = self.parts[hit.triangle];
        let curvature = Vec3::from_array(details.corners[hit.triangle]).dot(hit.weights);
        let folds = details.folds.distances(part, hit.point);
        let curves = details.curved.distances(part, hit.point);
        let edge = if curvature >= details.threshold {
            0.
        } else {
            folds[0].min(curves[0])
        };
        let crease = if curvature <= -details.threshold {
            0.
        } else {
            folds[1].min(curves[1])
        };
        details
            .folds
            .channels(curvature, width, folds, [edge, crease])
    }
}

fn smooth(
    triangles: &[SourceTriangle],
    corner_groups: &[[u32; 3]],
    groups: &[Group],
    scale: f32,
    progress: &Progress,
) -> Result<Vec<f32>, String> {
    let limit = 4. / scale;
    let mut values: Vec<_> = groups
        .iter()
        .map(|g| (g.curvature / g.count.max(1) as f32).clamp(-limit, limit))
        .collect();
    let inverse_spacing: Vec<_> = groups
        .iter()
        .map(|g| {
            (g.lengths / g.count.max(1) as f32)
                .max(scale * 1e-8)
                .recip()
        })
        .collect();
    for _ in 0..2 {
        let mut sum = values.clone();
        let mut weights = vec![1.; values.len()];
        for (face, (triangle, ids)) in triangles.iter().zip(corner_groups).enumerate() {
            if face % 4096 == 0 {
                progress.check()?;
            }
            if ids[0] == NO_GROUP {
                continue;
            }
            for i in 0..3 {
                let j = (i + 1) % 3;
                let length_squared = triangle.points[i].distance_squared(triangle.points[j]);
                for (a, b) in [
                    (ids[i] as usize, ids[j] as usize),
                    (ids[j] as usize, ids[i] as usize),
                ] {
                    let weight = (-0.5 * length_squared * inverse_spacing[a].powi(2)).exp();
                    sum[a] += weight * values[b];
                    weights[a] += weight;
                }
            }
        }
        for (value, (sum, weight)) in values.iter_mut().zip(sum.into_iter().zip(weights)) {
            *value = sum / weight;
        }
    }
    Ok(values)
}

fn contour(points: [Vec3; 3], values: [f32; 3], threshold: f32) -> Option<Segment> {
    let mut crossings = (0..3).filter_map(|i| {
        let j = (i + 1) % 3;
        let a = values[i];
        let b = values[j];
        ((a < threshold) != (b < threshold))
            .then(|| points[i].lerp(points[j], (threshold - a) / (b - a)))
    });
    let a = crossings.next()?;
    let b = crossings.next()?;
    (a.distance_squared(b) > 1e-20).then_some(Segment { a, b })
}
