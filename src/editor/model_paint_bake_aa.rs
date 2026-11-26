//! Supersamples projected normals and height inside the same UV triangle.
use super::project::{CageRay, CageTriangle, Hit, Projection, Query, tangent_normal};
use crate::editor::model_mesh::UvBarycentric;
use bevy::prelude::*;

pub(super) struct ProjectTriangle<'a> {
    pub barycentric: UvBarycentric,
    pub points: [Vec3; 3],
    pub normals: [Vec3; 3],
    pub directions: [Vec3; 3],
    pub geometric: Vec3,
    pub axes: (Vec3, Vec3),
    pub cage: Option<&'a CageTriangle>,
}

pub(super) struct Texel {
    pub center: Hit,
    pub normal: Vec3,
    pub height: f32,
}

pub(super) struct Probe {
    pub query: Query,
    distance: f32,
}

pub(super) struct Group {
    start: usize,
    end: usize,
    axes: (Vec3, Vec3),
    include_center: bool,
}

impl Group {
    pub fn resolve(&self, probes: &[Probe], hits: &[Option<Hit>]) -> Option<Texel> {
        let mut representative = hits[self.start].filter(|hit| hit.ray_hit);
        let mut nearest = representative.map_or(f32::INFINITY, |_| 0.);
        let mut normal = Vec3::ZERO;
        let mut height = 0.;
        let mut count = 0;
        for i in self.start..self.end {
            let Some(hit) = hits[i].filter(|hit| hit.ray_hit) else {
                continue;
            };
            if i == self.start && !self.include_center {
                continue;
            }
            let probe = &probes[i];
            normal += tangent_normal(hit.normal, probe.query.normal, self.axes);
            height += hit.height;
            count += 1;
            if probe.distance < nearest {
                nearest = probe.distance;
                representative = Some(hit);
            }
        }
        if count == 0 {
            return hits[self.start].map(|center| Texel {
                center,
                normal: tangent_normal(center.normal, probes[self.start].query.normal, self.axes),
                height: center.height,
            });
        }
        let center = representative.unwrap();
        Some(Texel {
            center,
            normal: normal.try_normalize().unwrap_or_else(|| {
                tangent_normal(center.normal, probes[self.start].query.normal, self.axes)
            }),
            height: height / count as f32,
        })
    }
}

#[derive(Clone, Copy)]
struct Subpixel {
    offset: Vec2,
    distance: f32,
}

pub(super) struct SamplePattern {
    grid: u32,
    samples: Vec<Subpixel>,
}

impl SamplePattern {
    pub fn new(grid: u32) -> Self {
        let samples = (0..grid).flat_map(|y| {
            (0..grid).map(move |x| {
                let offset = (Vec2::new(x as f32, y as f32) + 0.5) / grid as f32 - 0.5;
                Subpixel { offset, distance: offset.length_squared() }
            })
        }).collect();
        Self { grid, samples }
    }

    pub fn max_probes(&self) -> usize {
        self.samples.len() + usize::from(self.grid % 2 == 0)
    }
}

impl ProjectTriangle<'_> {
    fn ray(&self, w: Vec3) -> (Vec3, Vec3, Vec3, Option<CageRay>) {
        let mix = |[a, b, c]: [Vec3; 3]| a * w.x + b * w.y + c * w.z;
        let point = mix(self.points);
        let normal = mix(self.normals).try_normalize().unwrap_or(self.geometric);
        let direction = mix(self.directions)
            .try_normalize()
            .unwrap_or(self.geometric);
        (
            point,
            direction,
            normal,
            self.cage.map(|triangle| triangle.ray(w)),
        )
    }

    pub fn center_probe(&self, projector: &Projection, weights: Vec3) -> Probe {
        let (point, direction, normal, cage) = self.ray(weights);
        Probe {
            query: projector.query(point, direction, normal, cage, true),
            distance: 0.,
        }
    }

    pub fn enqueue(
        &self,
        projector: &Projection,
        pixel: Vec2,
        weights: Vec3,
        pattern: &SamplePattern,
        probes: &mut Vec<Probe>,
    ) -> Group {
        let start = probes.len();
        probes.push(self.center_probe(projector, weights));
        for sample in &pattern.samples {
            if sample.offset == Vec2::ZERO {
                continue;
            }
            let weights = self.barycentric.at(pixel + sample.offset);
            if !(weights.min_element() >= -1e-5) {
                continue;
            }
            let (point, direction, normal, cage) = self.ray(weights);
            probes.push(Probe {
                query: projector.query(point, direction, normal, cage, false),
                distance: sample.distance,
            });
        }
        Group {
            start,
            end: probes.len(),
            axes: self.axes,
            include_center: pattern.grid % 2 == 1,
        }
    }

    pub fn sample(
        &self,
        projector: &Projection,
        pixel: Vec2,
        weights: Vec3,
        pattern: &SamplePattern,
    ) -> Option<Texel> {
        let (point, direction, normal, cage) = self.ray(weights);
        let single = |hit: Hit| Texel {
            normal: tangent_normal(hit.normal, normal, self.axes),
            height: hit.height,
            center: hit,
        };
        if pattern.grid == 1 {
            return projector.sample(point, direction, normal, cage).map(single);
        }
        let center = projector.direct(point, direction, cage);
        let mut representative = center;
        let mut nearest = center.map_or(f32::INFINITY, |_| 0.);
        let mut sum = Vec3::ZERO;
        let mut height = 0.;
        let mut count = 0;
        for sample in &pattern.samples {
            let weights = self.barycentric.at(pixel + sample.offset);
            if !(weights.min_element() >= -1e-5) {
                continue;
            }
            let (sample_point, sample_direction, sample_normal, sample_cage) = self.ray(weights);
            let result = if sample.offset == Vec2::ZERO {
                center
            } else {
                projector.direct(sample_point, sample_direction, sample_cage)
            };
            if let Some(hit) = result {
                sum += tangent_normal(hit.normal, sample_normal, self.axes);
                height += hit.height;
                count += 1;
                let distance = sample.distance;
                if distance < nearest {
                    nearest = distance;
                    representative = Some(hit);
                }
            }
        }
        if count == 0 {
            return center
                .or_else(|| projector.fill(point, direction, normal, cage))
                .map(single);
        }
        let center = representative.unwrap();
        Some(Texel {
            center,
            normal: sum
                .try_normalize()
                .unwrap_or_else(|| tangent_normal(center.normal, normal, self.axes)),
            height: height / count as f32,
        })
    }
}
