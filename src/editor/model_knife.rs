use super::*;
use bevy::math::DVec2;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) struct KnifeSegment {
    pub face: usize,
    pub ends: [Vec3; 2],
}

struct Surface<'a> {
    mesh: &'a Geometry,
    face: &'a Face,
    triangles: Vec<[usize; 3]>,
    origin: Vec3,
    axis: usize,
    scale: f32,
}

impl<'a> Surface<'a> {
    fn new(mesh: &'a Geometry, face: &'a Face) -> Self {
        let normal = mesh.normal(face).abs();
        let axis = if normal.x > normal.y && normal.x > normal.z {
            0
        } else if normal.y > normal.z {
            1
        } else {
            2
        };
        let origin = mesh.positions[face.corners[0].vertex];
        let scale = face
            .corners
            .iter()
            .map(|c| mesh.positions[c.vertex].distance(origin))
            .fold(0.0, f32::max)
            .max(1.0e-12);
        Self {
            mesh,
            face,
            triangles: mesh.triangles(face),
            axis,
            origin,
            scale,
        }
    }

    fn project(&self, position: Vec3) -> DVec2 {
        let delta = (position.as_dvec3() - self.origin.as_dvec3()) / f64::from(self.scale);
        match self.axis {
            0 => delta.yz(),
            1 => delta.zx(),
            _ => delta.xy(),
        }
    }

    fn sample(&self, at: DVec2) -> Option<(Vec3, Corner)> {
        let (corners, weights) = self.triangles.iter().find_map(|indices| {
            let corners = indices.map(|i| &self.face.corners[i]);
            let points = corners.map(|c| self.project(self.mesh.positions[c.vertex]));
            let [a, b, c] = points;
            let area = (b - a).perp_dot(c - a);
            if area.abs() < 1.0e-15 {
                return None;
            }
            let v = (at - a).perp_dot(c - a) / area;
            let w = (b - a).perp_dot(at - a) / area;
            let u = 1.0 - v - w;
            [u, v, w]
                .into_iter()
                .all(|weight| weight >= -1.0e-6)
                .then_some((corners, [u as f32, v as f32, w as f32]))
        })?;
        let [a, b, c] = corners;
        let [u, v, w] = weights;
        let source_normal = a
            .source_normal
            .zip(b.source_normal)
            .zip(c.source_normal)
            .map(|((a, b), c)| (a * u + b * v + c * w).normalize_or_zero());
        Some((
            self.mesh.positions[a.vertex] * u
                + self.mesh.positions[b.vertex] * v
                + self.mesh.positions[c.vertex] * w,
            Corner {
                vertex: 0,
                uv: a.uv * u + b.uv * v + c.uv * w,
                uv1: a.uv1 * u + b.uv1 * v + c.uv1 * w,
                color: a.color * u + b.color * v + c.color * w,
                id_color: a.id_color.zip(b.id_color).zip(c.id_color)
                    .map(|((a, b), c)| a * u + b * v + c * w),
                source_part: super::common_source_part([a, b, c].into_iter()).cloned(),
                source_normal,
            },
        ))
    }
}

type EdgeSplits = BTreeMap<Edge, Vec<(f32, usize)>>;

fn register_boundary(
    surface: &Surface,
    position: Vec3,
    splits: &mut EdgeSplits,
    positions: &mut Vec<Vec3>,
) -> Option<usize> {
    let point = surface.project(position);
    for (a, b) in surface.face.edges() {
        let edge = Edge::new(a, b);
        let a = surface.mesh.positions[edge.0];
        let b = surface.mesh.positions[edge.1];
        let start = surface.project(a);
        let delta = surface.project(b) - start;
        let t = ((point - start).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
        if point.distance_squared(start + t * delta) > 1.0e-12 {
            continue;
        }
        if t < 1.0e-6 {
            return Some(edge.0);
        }
        if t > 1.0 - 1.0e-6 {
            return Some(edge.1);
        }
        let splits = splits.entry(edge).or_default();
        let t = t as f32;
        if let Some(&(_, v)) = splits.iter().find(|(at, _)| (at - t).abs() < 1.0e-6) {
            return Some(v);
        }
        let vertex = positions.len();
        positions.push(a.lerp(b, t));
        splits.push((t, vertex));
        return Some(vertex);
    }
    None
}

fn expanded_boundary(face: &Face, splits: &EdgeSplits) -> Vec<Corner> {
    let mut corners = Vec::new();
    for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
        corners.push(a.clone());
        let edge = Edge::new(a.vertex, b.vertex);
        if let Some(points) = splits.get(&edge) {
            let forward = edge.0 == a.vertex;
            for i in 0..points.len() {
                let (t, v) = points[if forward { i } else { points.len() - 1 - i }];
                let t = if forward { t } else { 1.0 - t };
                let mut corner = a.between(b, v, t);
                corner.source_normal = a
                    .source_normal
                    .zip(b.source_normal)
                    .map(|(a, b)| a.lerp(b, t).normalize_or_zero());
                corners.push(corner);
            }
        }
    }
    corners
}

impl Geometry {
    pub fn apply_knife(&mut self, segments: &[KnifeSegment]) -> Result<BTreeSet<Edge>, String> {
        const INVALID: &str = "Не удалось построить разрез по заданному пути";
        let mut grouped = BTreeMap::<usize, Vec<_>>::new();
        let mut result = self.clone();
        result.faces.clear();
        let mut splits = EdgeSplits::new();
        for segment in segments {
            let face = self.faces.get(segment.face).ok_or(INVALID)?;
            if segment.ends.iter().any(|pos| !pos.is_finite()) {
                return Err(INVALID.into());
            }
            let surface = Surface::new(self, face);
            if surface
                .project(segment.ends[0])
                .distance_squared(surface.project(segment.ends[1]))
                <= 1.0e-12
            {
                continue;
            }
            let ends = segment.ends.map(|pos| {
                let boundary = register_boundary(&surface, pos, &mut splits, &mut result.positions);
                (pos, boundary)
            });
            grouped.entry(segment.face).or_default().push(ends);
        }
        for points in splits.values_mut() {
            points.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        let mut cuts = BTreeSet::new();
        let mut disposable = BTreeSet::new();
        for (fi, face) in self.faces.iter().enumerate() {
            let corners = expanded_boundary(face, &splits);
            let Some(paths) = grouped.get(&fi) else {
                result.faces.push(Face {
                    corners,
                    ..face.clone()
                });
                continue;
            };
            let surface = Surface::new(self, face);
            let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
            let mut attributes = HashMap::new();
            let mut boundary = Vec::new();
            let mut boundary_handles = HashMap::new();
            let as_point = |v: DVec2| Point2::new(v.x, v.y);
            for corner in corners {
                let at = surface.project(result.positions[corner.vertex]);
                let handle = cdt.insert(as_point(at)).map_err(|_| INVALID)?;
                boundary.push((handle, at));
                boundary_handles.insert(corner.vertex, handle);
                attributes.insert(handle, corner);
            }
            for ((a, _), (b, _)) in boundary.iter().zip(boundary.iter().cycle().skip(1)) {
                if cdt.try_add_constraint(*a, *b).is_empty() {
                    return Err(INVALID.into());
                }
            }
            let mut interior: Vec<(_, DVec2)> = Vec::new();
            for path in paths {
                let mut endpoint = |(pos, vertex): (Vec3, Option<usize>)| {
                    if let Some(v) = vertex {
                        return Ok(boundary_handles[&v]);
                    }
                    let at = surface.project(pos);
                    if let Some((handle, _)) = interior
                        .iter()
                        .find(|(_, old)| at.distance_squared(*old) < 1.0e-12)
                    {
                        return Ok(*handle);
                    }
                    let handle = cdt.insert(as_point(at)).map_err(|_| INVALID)?;
                    interior.push((handle, at));
                    Ok::<_, &str>(handle)
                };
                let a = endpoint(path[0])?;
                let b = endpoint(path[1])?;
                cdt.add_constraint_and_split(a, b, |point| point);
            }
            for v in cdt.vertices() {
                if attributes.contains_key(&v.fix()) {
                    continue;
                }
                let point = v.position();
                let Some((position, mut corner)) = surface.sample(DVec2::new(point.x, point.y))
                else {
                    return Err(INVALID.into());
                };
                corner.vertex = result.positions.len();
                result.positions.push(position);
                attributes.insert(v.fix(), corner);
            }
            let signed_area: f64 = boundary
                .iter()
                .zip(boundary.iter().cycle().skip(1))
                .map(|((_, a), (_, b))| a.perp_dot(*b))
                .sum();
            let perimeter: BTreeSet<_> = boundary
                .iter()
                .zip(boundary.iter().cycle().skip(1))
                .map(|((a, _), (b, _))| Edge::new(attributes[a].vertex, attributes[b].vertex))
                .collect();
            let first = result.faces.len();
            for triangle in cdt.inner_faces() {
                let center = triangle
                    .vertices()
                    .into_iter()
                    .map(|v| {
                        let p = v.position();
                        DVec2::new(p.x, p.y)
                    })
                    .sum::<DVec2>()
                    / 3.0;
                if surface.sample(center).is_none() {
                    continue;
                }
                let mut corners = triangle.vertices().map(|v| attributes[&v.fix()].clone());
                if signed_area < 0.0 {
                    corners.swap(1, 2);
                }
                result.faces.push(Face {
                    corners: corners.into(),
                    ..face.clone()
                });
            }
            let surface_edges: BTreeSet<_> = result.faces[first..]
                .iter()
                .flat_map(|f| f.edges())
                .map(|(a, b)| Edge::new(a, b))
                .collect();
            for edge in cdt.undirected_edges() {
                let [a, b] = edge.vertices().map(|v| attributes[&v.fix()].vertex);
                let key = Edge::new(a, b);
                if perimeter.contains(&key) || !surface_edges.contains(&key) {
                    continue;
                }
                if edge.is_constraint_edge() {
                    cuts.insert(key);
                } else if !self.uv_seams.contains(&key) {
                    disposable.insert(key);
                }
            }
        }
        if cuts.is_empty() {
            return Err("Путь не создаёт новых рёбер".into());
        }
        for (edge, points) in splits {
            if !result.uv_seams.remove(&edge) {
                continue;
            }
            let mut from = edge.0;
            for to in points.into_iter().map(|(_, v)| v).chain([edge.1]) {
                result.uv_seams.insert(Edge::new(from, to));
                from = to;
            }
        }
        if result.positions.len() > 250_000 {
            return Err("Разрез превысит лимит модели в 250 000 вершин".into());
        }
        result.validate()?;
        let mut merged = result.clone();
        let mut selection = Selection {
            kind: Element::Edge,
            edges: disposable,
            ..default()
        };
        if merged.dissolve(&mut selection, false).is_ok()
            && merged.validate().is_ok()
            && cuts.iter().all(|edge| selection.edges.contains(edge))
        {
            result = merged;
        }
        *self = result;
        Ok(cuts)
    }
}
