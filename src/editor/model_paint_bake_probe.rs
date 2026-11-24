//! Samples projection coverage on the same shell and rays used by the baker.
use super::{
    Bounds, Object, Progress, ProjectionSettings, project, projection_cage, projection_directions,
    world_normal,
};
use bevy::{color::ColorToComponents, prelude::*};

const TRIANGLE_BUDGET: usize = 8192;
const SAMPLE_BUDGET: usize = 32000;

#[derive(Default)]
pub(in crate::editor::ui::model_viewer::paint) struct Diagnostic {
    pub points: Vec<Vec3>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub edges: Vec<(usize, usize)>,
    counts: [usize; 4],
    pub sampled: bool,
}

impl Diagnostic {
    pub fn summary(&self) -> String {
        let percent =
            |index: usize| self.counts[index] as f32 * 100. / self.points.len().max(1) as f32;
        format!(
            "Попадания {:.0}% · на пределе {:.0}% · промахи {:.0}% · чужие части {:.0}%{}",
            percent(0),
            percent(1),
            percent(2),
            percent(3),
            if self.sampled {
                " · выборка"
            } else {
                ""
            }
        )
    }
}

#[cfg(test)]
pub(super) fn build(
    objects: Vec<Object>,
    target: usize,
    slot: usize,
    settings: &ProjectionSettings,
    progress: &Progress,
) -> Result<Diagnostic, String> {
    prepare(objects, target, settings, progress)?.sample(slot, settings, progress)
}

pub(in crate::editor::ui::model_viewer::paint::bake) struct Prepared {
    object: Object,
    span: f32,
    projector: project::Projection,
    parts: Vec<usize>,
    positions: Vec<Vec3>,
    directions: Vec<Vec3>,
    cage: Option<project::Cage>,
}

pub(in crate::editor::ui::model_viewer::paint::bake) fn prepare(
    objects: Vec<Object>,
    target: usize,
    settings: &ProjectionSettings,
    progress: &Progress,
) -> Result<Prepared, String> {
    let object = objects
        .iter()
        .find(|object| object.id == target && !object.hidden)
        .ok_or("Объект скрыт")?;
    let path = settings.path.as_deref().ok_or("Выберите high-poly")?;
    let mut bounds = Bounds::default();
    for object in objects.iter().filter(|object| !object.hidden) {
        for face in &object.geometry.faces {
            progress.check()?;
            for corner in &face.corners {
                bounds.add(
                    object
                        .transform
                        .transform_point(object.geometry.positions[corner.vertex]),
                );
            }
        }
    }
    let span = (bounds.max - bounds.min).length().max(1e-5);
    let projector = project::Projection::load(path, &object.name, span, settings, progress)?;
    let parts = projector.connected_parts(progress)?;
    let mesh = &object.geometry;
    let positions: Vec<_> = mesh
        .positions
        .iter()
        .map(|&point| object.transform.transform_point(point))
        .collect();
    let directions = projection_directions(object, &positions, settings.cage.is_none(), progress)?;
    let mut baseline = settings.clone();
    baseline.manual = None;
    let (cage, _) = projection_cage(
        &projector,
        object,
        &positions,
        &directions,
        span,
        &baseline,
        progress,
    )?;
    Ok(Prepared {
        object: object.clone(),
        span,
        projector,
        parts,
        positions,
        directions,
        cage,
    })
}

impl Prepared {
    pub(in crate::editor::ui::model_viewer::paint::bake) fn sample(
        &self,
        slot: usize,
        settings: &ProjectionSettings,
        progress: &Progress,
    ) -> Result<Diagnostic, String> {
        let Self {
            object,
            span,
            projector,
            parts,
            positions,
            directions,
            cage,
        } = self;
        let span = *span;
        let painted = settings
            .manual
            .as_ref()
            .filter(|_| settings.cage.is_none())
            .map(|manual| {
                projector.paint_cage(cage.as_ref(), directions, &manual.values, span, progress)
            })
            .transpose()?
            .flatten();
        let cage = painted.as_ref().or(cage.as_ref());
        let mesh = &object.geometry;
        let to_world = world_normal(object.transform);
        let triangle_estimate = mesh
            .faces
            .iter()
            .filter(|face| face.material == slot)
            .map(|face| face.corners.len().saturating_sub(2))
            .sum::<usize>();
        let stride = triangle_estimate.div_ceil(TRIANGLE_BUDGET).max(1);
        let samples_per_triangle = SAMPLE_BUDGET / triangle_estimate.div_ceil(stride).max(1);
        let grid_limit = (1..=10)
            .rev()
            .find(|&grid| (grid + 1) * (grid + 2) / 2 <= samples_per_triangle)
            .unwrap_or(1);
        let colors = [
            Color::srgb(0.30, 0.87, 0.50),
            Color::srgb(1., 0.73, 0.22),
            Color::srgb(1., 0.26, 0.31),
            Color::srgb(0.89, 0.38, 0.91),
        ]
        .map(|color| color.to_linear().to_f32_array());
        let mut output = Diagnostic {
            sampled: stride > 1,
            ..default()
        };
        let mut ordinal = 0;
        for face in mesh.faces.iter().filter(|face| face.material == slot) {
            progress.check()?;
            let geo = to_world(mesh.normal(face));
            for triangle in mesh.triangles(face) {
                let include = ordinal % stride == 0;
                ordinal += 1;
                if !include {
                    continue;
                }
                let vertices = triangle.map(|i| face.corners[i].vertex);
                let points = vertices.map(|i| positions[i]);
                let normals = vertices.map(|i| directions[i].try_normalize().unwrap_or(geo));
                let longest = points[0]
                    .distance(points[1])
                    .max(points[1].distance(points[2]))
                    .max(points[2].distance(points[0]));
                let grid = ((longest / span * 36.).ceil() as usize).clamp(1, grid_limit);
                let shell = cage.map(|cage| cage.triangle(vertices));
                let start = output.points.len();
                let vertex =
                    |i: usize, j: usize| start + i * (grid + 1) - i * i.saturating_sub(1) / 2 + j;
                let segment_at = |w: Vec3| {
                    let blend = |[a, b, c]: [Vec3; 3]| a * w.x + b * w.y + c * w.z;
                    let point = blend(points);
                    let direction = blend(normals).try_normalize().unwrap_or(geo);
                    let cage = shell.as_ref().map(|shell| shell.ray(w));
                    (point, projector.segment(point, direction, cage))
                };
                for i in 0..=grid {
                    for j in 0..=grid - i {
                        let y = i as f32 / grid as f32;
                        let z = j as f32 / grid as f32;
                        let w = Vec3::new(1. - y - z, y, z);
                        let (point, segment, shell_point) = if i == 0 || j == 0 || i + j == grid {
                            // UV texels are sampled in the interior. Avoid numerical misses when
                            // a diagnostic vertex falls exactly on the edge of both meshes.
                            let (_, edge) = segment_at(w);
                            let (point, segment) = segment_at(w.lerp(Vec3::splat(1. / 3.), 1e-5));
                            (point, segment, edge.origin)
                        } else {
                            let (point, segment) = segment_at(w);
                            let origin = segment.origin;
                            (point, segment, origin)
                        };
                        let status = match projector.cast(&segment) {
                            None => 2,
                            Some((hit, distance)) => {
                                let nearest = projector.closest_crossing(
                                    point,
                                    segment.outward,
                                    span.max(segment.front + segment.rear),
                                    span * 1e-6,
                                );
                                let height = segment.front - distance;
                                if nearest.is_some_and(|(own, _)| parts[own] != parts[hit]) {
                                    3
                                } else if height > 0.85 * segment.front
                                    || height < -0.85 * segment.rear
                                {
                                    1
                                } else {
                                    0
                                }
                            }
                        };
                        output.counts[status] += 1;
                        output.points.push(shell_point);
                        output.colors.push(colors[status]);
                    }
                }
                for i in 0..grid {
                    for j in 0..grid - i {
                        let a = vertex(i, j);
                        let b = vertex(i + 1, j);
                        let c = vertex(i, j + 1);
                        output.indices.extend([a as u32, b as u32, c as u32]);
                        if i + j + 1 < grid {
                            output.indices.extend([
                                b as u32,
                                vertex(i + 1, j + 1) as u32,
                                c as u32,
                            ]);
                        } else {
                            output.edges.push((b, c));
                        }
                        if j == 0 {
                            output.edges.push((a, b));
                        }
                        if i == 0 {
                            output.edges.push((a, c));
                        }
                    }
                }
            }
        }
        if output.points.is_empty() {
            return Err("В наборе нет граней".into());
        }
        Ok(output)
    }
}
