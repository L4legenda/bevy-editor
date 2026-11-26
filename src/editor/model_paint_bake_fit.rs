//! Fits a generated cage to signed high-poly crossings along the low-poly vertex normals.
use super::super::root;
use super::{Cage, Progress, Projection};
use bevy::prelude::*;
use std::collections::HashMap;

const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 8.;
const MARGIN: f32 = 1.15;

#[derive(Clone, Default)]
pub(in crate::editor::ui::model_viewer::paint) struct FitReport {
    pub raised: usize,
    pub lowered: usize,
    pub unresolved: usize,
}

impl Projection {
    pub(in crate::editor::ui::model_viewer::paint) fn fit_cage(
        &self,
        positions: &[Vec3],
        normals: &[Vec3],
        groups: &[Vec<u32>],
        scene_span: f32,
        progress: &Progress,
    ) -> Result<(Cage, FitReport), String> {
        let parts = self.connected_parts(progress)?;
        let max_scale = (scene_span / self.front.max(self.rear)).min(MAX_SCALE);
        let reach = self.front.max(self.rear) * max_scale;
        let bias = scene_span * 1e-6;

        // Split vertices at UV seams or hard edges must move together.
        let mut welded = HashMap::<[u32; 3], (usize, Vec3)>::new();
        let mut welded_groups = HashMap::<usize, Vec<u32>>::new();
        for (i, (&position, &normal)) in positions.iter().zip(normals).enumerate() {
            if i % 4096 == 0 {
                progress.check()?;
            }
            let entry = welded.entry(position_key(position)).or_insert((i, Vec3::ZERO));
            entry.1 += normal;
            if let Some(groups) = groups.get(i) {
                let entry = welded_groups.entry(entry.0).or_default();
                for &group in groups {
                    if !entry.contains(&group) {
                        entry.push(group);
                    }
                }
            }
        }
        let mut scales = vec![1.; positions.len()];
        let mut directions = vec![Vec3::ZERO; positions.len()];
        let mut report = FitReport::default();
        for (i, &position) in positions.iter().enumerate() {
            if i % 512 == 0 {
                progress.check()?;
            }
            let (first, normal) = welded[&position_key(position)];
            if first != i {
                scales[i] = scales[first];
                directions[i] = directions[first];
                continue;
            }
            let Some(direction) = normal.try_normalize() else {
                report.unresolved += 1;
                continue;
            };
            directions[i] = direction;
            let groups = welded_groups.get(&i).map(Vec::as_slice).filter(|groups| !groups.is_empty()).unwrap_or(&[0]);
            let mut fitted = false;
            let mut missing = false;
            let mut required = MIN_SCALE;
            let mut allowed = max_scale;
            for &group in groups {
                let Some((triangle, offset)) = self.closest_crossing(position, direction, reach, bias, group) else {
                    missing = true;
                    continue;
                };
                fitted = true;
                required = required.max(offset * MARGIN / self.front).max(-offset * MARGIN / self.rear);
                // Start inside the gap when another connected part would be hit first.
                let start = offset + bias;
                if let Some((_, distance)) = self.source.bvh.cast_triangle_up_to(
                    position + direction * start, direction, (reach - start).max(0.),
                    |other| self.can_project(other, group) && parts[other.face] != parts[triangle],
                ) {
                    let bound = ((offset + (bias + distance) * 0.5) / self.front).clamp(MIN_SCALE, max_scale);
                    allowed = allowed.min(bound);
                }
            }
            report.unresolved += usize::from(missing || required > allowed);
            if !fitted {
                continue;
            }
            let scale = if required <= allowed {
                1.0_f32.clamp(required, allowed)
            } else {
                allowed
            };
            report.raised += usize::from(scale > 1.001);
            report.lowered += usize::from(scale < 0.999);
            scales[i] = scale;
        }
        let offsets = directions
            .into_iter()
            .zip(&scales)
            .map(|(direction, &scale)| direction * (self.front * scale))
            .collect();
        Ok((Cage::fitted(offsets, scales), report))
    }

    pub(in crate::editor::ui::model_viewer::paint) fn connected_parts(
        &self,
        progress: &Progress,
    ) -> Result<Vec<usize>, String> {
        let mut parents: Vec<_> = (0..self.source.triangles.len()).collect();
        let mut vertices = HashMap::new();
        for (i, triangle) in self.source.triangles.iter().enumerate() {
            if i % 4096 == 0 {
                progress.check()?;
            }
            for point in triangle.points {
                let other = *vertices
                    .entry((triangle.part, position_key(point)))
                    .or_insert(i);
                let a = root(&mut parents, i);
                let b = root(&mut parents, other);
                parents[a] = b;
            }
        }
        Ok((0..parents.len()).map(|i| root(&mut parents, i)).collect())
    }
}

pub(super) fn position_key(point: Vec3) -> [u32; 3] {
    point
        .to_array()
        .map(|value| if value == 0. { 0 } else { value.to_bits() })
}

#[cfg(test)]
mod tests {
    use super::super::{ProjectionSettings, SourceTriangle};
    use super::*;

    fn panel(z: f32, part: usize) -> SourceTriangle {
        SourceTriangle::new(
            [
                Vec3::new(-1., -1., z),
                Vec3::new(1., -1., z),
                Vec3::new(0., 1., z),
            ],
            [Vec3::Z; 3],
            part,
        )
        .unwrap()
    }

    #[test]
    fn baked_fit_reaches_behind_and_keeps_split_vertices_together() {
        let settings = ProjectionSettings {
            fill_misses: false,
            ..default()
        };
        let projection = Projection::new(vec![panel(-0.16, 0)], 1., &settings);
        let positions = [Vec3::ZERO; 2];
        let normals = [Vec3::new(0.2, 0., 1.), Vec3::new(-0.2, 0., 1.)];
        let (cage, report) = projection
            .fit_cage(&positions, &normals, &[], 1., &Progress::default())
            .unwrap();
        assert_eq!(report.raised, 1);
        assert_eq!(report.unresolved, 0);
        assert!(
            projection
                .sample(Vec3::ZERO, Vec3::Z, Vec3::Z, None, 0)
                .is_none()
        );
        let triangle = cage.triangle([0, 1, 0]);
        let first = triangle.ray(Vec3::X);
        let split = triangle.ray(Vec3::Y);
        assert_eq!(first.offset, split.offset);
        assert_eq!(first.scale, split.scale);
        let hit = projection
            .sample(Vec3::ZERO, Vec3::Z, Vec3::Z, Some(first), 0)
            .unwrap();
        assert!(hit.ray_hit && hit.height < -0.8);
    }

    #[test]
    fn baked_fit_starts_between_separate_parts_to_avoid_the_outer_surface() {
        let settings = ProjectionSettings {
            fill_misses: false,
            ..default()
        };
        // Disconnected surfaces in a single source object still count as separate parts.
        let projection = Projection::new(vec![panel(0.012, 0), panel(0.035, 0)], 1., &settings);
        let (cage, report) = projection
            .fit_cage(&[Vec3::ZERO], &[Vec3::Z], &[], 1., &Progress::default())
            .unwrap();
        assert_eq!(report.lowered, 1);
        assert_eq!(report.unresolved, 0);
        let ray = cage.triangle([0; 3]).ray(Vec3::X);
        assert!((0.012..0.035).contains(&ray.offset.z));
        let hit = projection
            .sample(Vec3::ZERO, Vec3::Z, Vec3::Z, Some(ray), 0)
            .unwrap();
        assert!(hit.ray_hit && (hit.height - 0.012 / ray.offset.z).abs() < 1e-5);
    }
}
