//! Binds an authored cage to the corresponding low-poly vertices.
use super::{Progress, SourceTriangle, base_name, read_triangles};
use crate::editor::model_document::Object;
use bevy::prelude::*;
use std::{collections::HashMap, path::Path};

pub(in crate::editor::ui::model_viewer::paint) struct Cage {
    offsets: Vec<Vec3>,
    reach: f32,
    scales: Option<Vec<f32>>,
    max_scale: f32,
}

#[derive(Clone, Copy)]
pub(in crate::editor::ui::model_viewer::paint) struct CageRay {
    pub offset: Vec3,
    pub reach: f32,
    pub scale: f32,
    pub max_scale: f32,
}

pub(in crate::editor::ui::model_viewer::paint) struct CageTriangle {
    offsets: [Vec3; 3],
    scales: [f32; 3],
    reach: f32,
    max_scale: f32,
}

impl CageTriangle {
    pub fn ray(&self, weights: Vec3) -> CageRay {
        let [a, b, c] = self.offsets;
        CageRay {
            offset: a * weights.x + b * weights.y + c * weights.z,
            reach: self.reach,
            scale: Vec3::from_array(self.scales).dot(weights),
            max_scale: self.max_scale,
        }
    }
}

impl Cage {
    pub fn load(
        path: &Path,
        object: &Object,
        positions: &[Vec3],
        progress: &Progress,
    ) -> Result<Self, String> {
        let expected = base_name(&object.name);
        let mut triangles = read_triangles(
            path,
            |name| {
                let name = base_name(name);
                let name = ["_cage", "-cage", " cage", ".cage"]
                    .into_iter()
                    .find_map(|suffix| name.strip_suffix(suffix))
                    .unwrap_or(&name);
                (base_name(name) == expected).into()
            },
            progress,
        )?;
        if triangles.is_empty() {
            // A cage for a single object can be exported with any object name.
            triangles = read_triangles(path, |_| true.into(), progress)?;
            if let Some(first) = triangles.first()
                && triangles.iter().any(|triangle| triangle.part != first.part)
            {
                return Err(format!(
                    "В оболочке нет детали для «{}» · проверьте имена объектов",
                    object.name
                ));
            }
        }

        let mesh = &object.geometry;
        let mirrored =
            object.transform.scale.x * object.transform.scale.y * object.transform.scale.z < 0.;
        let mut faces = Vec::new();
        for face in &mesh.faces {
            progress.check()?;
            for triangle in mesh.triangles(face) {
                let mut vertices = triangle.map(|i| face.corners[i].vertex);
                if mirrored {
                    vertices.swap(1, 2);
                }
                faces.push((face.material, vertices));
            }
        }
        if triangles.len() != faces.len() {
            return Err(format!(
                "У оболочки {} треугольников, у объекта {}. Используйте копию объекта с той же сеткой.",
                triangles.len(),
                faces.len()
            ));
        }

        let ordered = bind(
            faces.iter().map(|(_, vertices)| *vertices),
            &triangles,
            positions,
            progress,
        )?;
        let offsets = if faces.is_sorted_by_key(|(material, _)| *material) {
            ordered
        } else {
            // GLB export writes a primitive per material. Try that order too.
            faces.sort_by_key(|(material, _)| *material);
            let grouped = bind(
                faces.into_iter().map(|(_, vertices)| vertices),
                &triangles,
                positions,
                progress,
            )?;
            match (ordered, grouped) {
                (Some(a), Some(b)) => {
                    let cost = |offsets: &[Vec3]| {
                        offsets
                            .iter()
                            .map(|offset| offset.length_squared() as f64)
                            .sum::<f64>()
                    };
                    Some(if cost(&a) <= cost(&b) { a } else { b })
                }
                (a, b) => a.or(b),
            }
        }
        .ok_or("У оболочки другой порядок треугольников или другая связность вершин")?;
        Ok(Self::from_offsets(offsets, None))
    }

    pub(super) fn fitted(offsets: Vec<Vec3>, scales: Vec<f32>) -> Self {
        Self::from_offsets(offsets, Some(scales))
    }

    pub(in crate::editor::ui::model_viewer::paint::bake::mesh) fn apply_paint(
        base: Option<&Self>,
        directions: &[Vec3],
        overrides: &[f32],
        [front, rear]: [f32; 2],
        span: f32,
        progress: &Progress,
    ) -> Result<Option<Self>, String> {
        if directions.len() != overrides.len() || overrides.iter().all(|&value| value == 1.) {
            return Ok(None);
        }
        let limit = (span / front.max(rear)).min(8.);
        Self::painted(base, directions, overrides, front, limit, progress).map(Some)
    }

    fn painted(
        base: Option<&Self>,
        directions: &[Vec3],
        overrides: &[f32],
        front: f32,
        max_scale: f32,
        progress: &Progress,
    ) -> Result<Self, String> {
        let mut offsets = Vec::with_capacity(directions.len());
        let mut scales = Vec::with_capacity(directions.len());
        for (i, &direction) in directions.iter().enumerate() {
            if i % 4096 == 0 {
                progress.check()?;
            }
            let baseline = base
                .and_then(|cage| cage.scales.as_ref())
                .map_or(1., |scales| scales[i]);
            let direction = base
                .and_then(|cage| cage.offsets[i].try_normalize())
                .unwrap_or_else(|| direction.normalize_or_zero());
            let scale = (baseline * overrides[i]).clamp(0.05, max_scale);
            offsets.push(direction * (front * scale));
            scales.push(scale);
        }
        Ok(Self::fitted(offsets, scales))
    }

    fn from_offsets(offsets: Vec<Vec3>, scales: Option<Vec<f32>>) -> Self {
        let reach = offsets
            .iter()
            .map(|offset| offset.length())
            .fold(0., f32::max);
        let max_scale = scales
            .as_ref()
            .map_or(1., |scales| scales.iter().copied().fold(0., f32::max));
        Self {
            offsets,
            reach,
            scales,
            max_scale,
        }
    }

    pub fn triangle(&self, vertices: [usize; 3]) -> CageTriangle {
        CageTriangle {
            offsets: vertices.map(|vertex| self.offsets[vertex]),
            scales: vertices.map(|vertex| self.scales.as_ref().map_or(1., |scales| scales[vertex])),
            reach: self.reach,
            max_scale: self.max_scale,
        }
    }
}

fn bind(
    faces: impl Iterator<Item = [usize; 3]>,
    cage: &[SourceTriangle],
    positions: &[Vec3],
    progress: &Progress,
) -> Result<Option<Vec<Vec3>>, String> {
    let mut by_vertex = vec![None; positions.len()];
    let mut by_cage = HashMap::new();
    for (i, (face, source)) in faces.zip(cage).enumerate() {
        if i % 4096 == 0 {
            progress.check()?;
        }
        for (vertex, point) in face.into_iter().zip(source.points) {
            // Weld positions for comparison so normal/UV splits do not look like open seams.
            let key = point
                .to_array()
                .map(|value| if value == 0. { 0 } else { value.to_bits() });
            if by_vertex[vertex].is_some_and(|old| old != point)
                || *by_cage.entry(key).or_insert(vertex) != vertex
            {
                return Ok(None);
            }
            by_vertex[vertex] = Some(point);
        }
    }
    let offsets = by_vertex
        .into_iter()
        .zip(positions)
        .map(|(point, &low)| point.map_or(Vec3::ZERO, |point| point - low))
        .collect();
    Ok(Some(offsets))
}
