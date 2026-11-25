//! Grow a selection in world space and write its coverage into the active UV tile.
use super::{
    Stack,
    editing::input::{Bvh, PickTriangle},
    sets::SetVisibility,
};
use crate::editor::model_document::Object;
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(
    feature = "mcp",
    derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)
)]
#[cfg_attr(feature = "mcp", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "mcp", schemars(crate = "rmcp::schemars"))]
pub(super) enum MaskMode {
    #[default]
    Replace,
    Add,
    Subtract,
}

#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(
    feature = "mcp",
    derive(serde::Deserialize, serde::Serialize, rmcp::schemars::JsonSchema)
)]
#[cfg_attr(feature = "mcp", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "mcp", schemars(crate = "rmcp::schemars"))]
pub(super) enum StopAt {
    #[default]
    Concave,
    Any,
}

pub(super) enum Source {
    Parts(Vec<usize>),
    Point(Vec3),
    Ray(Vec3, Vec3),
}

pub(super) struct Selection {
    pub source: Source,
    pub crease_angle: f32,
    pub stop_at: StopAt,
    pub mode: MaskMode,
}

pub(super) struct Report {
    pub selected: usize,
    pub total: usize,
    pub active_set: usize,
    pub bounds: Option<[Vec3; 2]>,
}

struct Triangle {
    world: [Vec3; 3],
    uv: Option<[Vec2; 3]>,
}

pub(super) fn apply(
    stack: &mut Stack,
    layer: usize,
    size: u32,
    objects: &[Object],
    target: (usize, usize),
    visibility: &SetVisibility,
    request: Selection,
) -> Result<(Report, bool), String> {
    let mut triangles = Vec::new();
    for object in objects.iter().filter(|object| !object.hidden) {
        for face in &object.geometry.faces {
            if !visibility.shown(object.id, face.material) {
                continue;
            }
            let active = (object.id, face.material) == target;
            for triangle in object.geometry.triangles(face) {
                let corners = triangle.map(|i| &face.corners[i]);
                let world = corners.map(|corner| {
                    object
                        .transform
                        .transform_point(object.geometry.positions[corner.vertex])
                });
                if world.iter().any(|point| !point.is_finite()) {
                    continue;
                }
                let uv =
                    active.then(|| corners.map(|corner| corner.uv - object.uv_offset(target.1)));
                triangles.push(Triangle { world, uv });
            }
        }
    }
    let selected = select(
        &triangles,
        request.source,
        request.crease_angle,
        request.stop_at,
    )?;
    let mut report = Report {
        selected: 0,
        total: triangles.len(),
        active_set: 0,
        bounds: None,
    };
    for (triangle, &selected) in triangles.iter().zip(&selected) {
        if selected {
            report.selected += 1;
            report.active_set += usize::from(triangle.uv.is_some());
            for point in triangle.world {
                let [min, max] = report.bounds.get_or_insert([point; 2]);
                *min = min.min(point);
                *max = max.max(point);
            }
        }
    }
    let coverage = coverage(&triangles, &selected, size);
    let mask = &mut stack.layers[layer].mask;
    let previous = mask.as_deref();
    let black = [0, 0, 0, 255];
    let white = if previous.is_some_and(|mask| mask.affine) {
        [0, 255, 0, 255]
    } else {
        [255; 4]
    };
    let mut pixels = previous.map_or_else(
        || vec![0; coverage.len() * 4],
        |mask| mask.pixels.as_ref().clone(),
    );
    for (pixel, owner) in pixels.chunks_exact_mut(4).zip(coverage) {
        let replacement = match (request.mode, owner == SELECTED) {
            (MaskMode::Replace | MaskMode::Add, true) => white,
            (MaskMode::Replace, false) | (MaskMode::Subtract, true) => black,
            _ if previous.is_none() => black,
            _ => continue,
        };
        pixel.copy_from_slice(&replacement);
    }
    let changed = previous.is_none_or(|old| *old.pixels != pixels);
    if changed {
        let pixels = Arc::new(pixels);
        *mask = Some(Arc::new(match previous {
            Some(old) => old.with_pixels(pixels),
            None => super::Mask::new(pixels, Vec::new()),
        }));
    }
    Ok((report, changed))
}

fn select(
    triangles: &[Triangle],
    source: Source,
    crease_angle: f32,
    stop_at: StopAt,
) -> Result<Vec<bool>, String> {
    if triangles.is_empty() {
        return Err("There is no visible geometry to select".into());
    }
    let mut vertices = HashMap::new();
    let corners = triangles
        .iter()
        .map(|triangle| {
            triangle.world.map(|p| {
                let key = (p * 1.0e4).round().as_ivec3();
                let next = vertices.len();
                *vertices.entry(key).or_insert(next)
            })
        })
        .collect::<Vec<_>>();
    if let Source::Parts(parts) = source {
        let mut parents: Vec<_> = (0..vertices.len()).collect();
        for &[a, b, c] in &corners {
            let first = root(&mut parents, a);
            for v in [b, c] {
                let other = root(&mut parents, v);
                parents[other] = first;
            }
        }
        let mut indices = HashMap::new();
        let parts = parts.into_iter().collect::<std::collections::HashSet<_>>();
        let result = corners
            .iter()
            .map(|t| {
                let next = indices.len();
                parts.contains(indices.entry(root(&mut parents, t[0])).or_insert(next))
            })
            .collect();
        if parts.iter().any(|part| *part >= indices.len()) {
            return Err(format!("Select part indices below {}", indices.len()));
        }
        return Ok(result);
    }
    let bvh = Bvh::from_triangles(
        triangles
            .iter()
            .enumerate()
            .map(|(i, triangle)| PickTriangle {
                object: 0,
                face: i,
                points: triangle.world,
            })
            .collect(),
    );
    let seed = match source {
        Source::Point(point) => bvh
            .nearest_surface_where(point, f32::INFINITY, |_| true)
            .map(|hit| hit.0),
        Source::Ray(origin, direction) => bvh
            .cast_triangle(origin, direction, |_| true)
            .map(|hit| hit.0),
        Source::Parts(_) => unreachable!(),
    }
    .ok_or("The selection seed does not hit the visible mesh")?;
    let mut edges = HashMap::<_, Vec<_>>::new();
    let edge_keys =
        |[a, b, c]: [usize; 3]| [[a, b], [b, c], [c, a]].map(|[a, b]| (a.min(b), a.max(b)));
    for (i, &corners) in corners.iter().enumerate() {
        for edge in edge_keys(corners) {
            if edge.0 != edge.1 {
                edges.entry(edge).or_default().push(i);
            }
        }
    }
    let planes = triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.world;
            ((b - a).cross(c - a).normalize_or_zero(), (a + b + c) / 3.)
        })
        .collect::<Vec<_>>();
    let limit = crease_angle.to_radians().cos();
    let mut selected = vec![false; triangles.len()];
    let mut pending = vec![seed];
    selected[seed] = true;
    while let Some(i) = pending.pop() {
        let (normal, center) = planes[i];
        for j in edge_keys(corners[i])
            .iter()
            .filter_map(|edge| edges.get(edge))
            .flatten()
            .copied()
        {
            let (other_normal, other_center) = planes[j];
            let sharp = normal.dot(other_normal) < limit;
            let concave = (other_center - center).dot(normal) > 0.;
            if !selected[j] && !(sharp && (concave || matches!(stop_at, StopAt::Any))) {
                selected[j] = true;
                pending.push(j);
            }
        }
    }
    Ok(selected)
}

fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

const SELECTED: u8 = 1;
const UNSELECTED: u8 = 2;

fn coverage(triangles: &[Triangle], selected: &[bool], size: u32) -> Vec<u8> {
    let mut owner = vec![0; size as usize * size as usize];
    for (triangle, &selected) in triangles.iter().zip(selected) {
        if !selected && let Some(uv) = triangle.uv {
            raster(uv, size, 0., |pixel| owner[pixel] = UNSELECTED);
        }
    }
    for (triangle, &selected) in triangles.iter().zip(selected) {
        if selected && let Some(uv) = triangle.uv {
            raster(uv, size, 0., |pixel| owner[pixel] = SELECTED);
            raster(uv, size, 3., |pixel| {
                if owner[pixel] != UNSELECTED {
                    owner[pixel] = SELECTED;
                }
            });
        }
    }
    owner
}

fn raster(uv: [Vec2; 3], size: u32, pad: f32, mut write: impl FnMut(usize)) {
    let [a, b, c] = uv.map(|p| p * size as f32);
    let area = (b - a).perp_dot(c - a);
    if !area.is_finite() || area.abs() < 1.0e-8 {
        return;
    }
    let edges = [(a, b), (b, c), (c, a)].map(|(start, end)| {
        let edge = (end - start).normalize_or_zero() * area.signum();
        let inward = Vec2::new(-edge.y, edge.x);
        (inward, inward.dot(start))
    });
    let min = (a.min(b).min(c) - Vec2::splat(pad)).floor().max(Vec2::ZERO);
    let max = (a.max(b).max(c) + Vec2::splat(pad))
        .ceil()
        .min(Vec2::splat(size as f32));
    for y in min.y as usize..max.y as usize {
        for x in min.x as usize..max.x as usize {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            if edges
                .iter()
                .all(|(normal, depth)| normal.dot(p) >= depth - pad)
            {
                write(y * size as usize + x);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_surface_selection_crosses_convex_folds_and_stops_at_concave_folds() {
        for (third, expected) in [(Vec3::Z, false), (Vec3::NEG_Z, true)] {
            let triangles = [
                Triangle {
                    world: [Vec3::ZERO, Vec3::X, Vec3::Y],
                    uv: None,
                },
                Triangle {
                    world: [Vec3::X, Vec3::ZERO, third],
                    uv: None,
                },
            ];
            let seed = || Source::Ray(Vec3::new(0.25, 0.25, 2.), Vec3::NEG_Z);
            assert_eq!(
                select(&triangles, seed(), 30., StopAt::Concave).unwrap(),
                [true, expected]
            );
            assert_eq!(
                select(&triangles, seed(), 30., StopAt::Any).unwrap(),
                [true, false]
            );
            assert_eq!(
                select(&triangles, Source::Parts(vec![0]), 30., StopAt::Any).unwrap(),
                [true, true]
            );
        }
    }

    #[test]
    fn mesh_surface_selection_padding_respects_the_unselected_chart() {
        let triangles = [
            Triangle {
                world: [Vec3::ZERO, Vec3::X, Vec3::Y],
                uv: Some([Vec2::ZERO, Vec2::X, Vec2::Y]),
            },
            Triangle {
                world: [Vec3::X, Vec3::ONE, Vec3::Y],
                uv: Some([Vec2::X, Vec2::ONE, Vec2::Y]),
            },
        ];
        let owner = coverage(&triangles, &[true, false], 16);
        assert_eq!(owner[15], SELECTED);
        assert_eq!(owner[16 + 15], UNSELECTED);
    }
}
