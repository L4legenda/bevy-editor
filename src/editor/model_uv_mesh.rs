//! UV topology and lossless corner updates; independent of UI and subprocesses.
use super::{
    model_mesh::{Edge, Face, Geometry},
    model_uv_backend::{Output, Request},
};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Input {
    pub request: Request,
    pub corners: Vec<(usize, [usize; 3])>,
}
impl Input {
    pub fn cut_manual_seams(&mut self, seams: &BTreeSet<Edge>) {
        if seams.is_empty() {
            return;
        }
        let request = &mut self.request;
        let mut adjacency = BTreeMap::<Edge, Vec<usize>>::new();
        for (i, &[a, b, c]) in request.triangles.iter().enumerate() {
            for edge in [Edge::new(a, b), Edge::new(b, c), Edge::new(c, a)] {
                adjacency.entry(edge).or_default().push(i);
            }
        }
        let corner = |face: usize, vertex| {
            request.triangles[face].iter().position(|&v| v == vertex).unwrap()
        };
        let mut parents: Vec<_> = (0..request.triangles.len() * 3).collect();
        for (&edge, faces) in &adjacency {
            let [a, b] = faces.as_slice() else { continue };
            if seams.contains(&edge) {
                continue;
            }
            for vertex in [edge.0, edge.1] {
                let a = root(&mut parents, a * 3 + corner(*a, vertex));
                let b = root(&mut parents, b * 3 + corner(*b, vertex));
                parents[a.max(b)] = a.min(b);
            }
        }
        let mut ids = vec![usize::MAX; parents.len()];
        let mut positions = Vec::new();
        let mut triangles = Vec::with_capacity(request.triangles.len());
        for (i, face) in request.triangles.iter().enumerate() {
            triangles.push(std::array::from_fn(|c| {
                let id = &mut ids[root(&mut parents, i * 3 + c)];
                if *id == usize::MAX {
                    *id = positions.len();
                    positions.push(request.positions[face[c]]);
                }
                *id
            }));
        }
        // With isolated marks on a closed vertex fan, both endpoints can still
        // be connected around the other side. Open one corner in that case.
        for &edge in seams {
            let Some([a, b]) = adjacency.get(&edge).map(Vec::as_slice) else {
                continue;
            };
            let ca = [corner(*a, edge.0), corner(*a, edge.1)];
            let cb = [corner(*b, edge.0), corner(*b, edge.1)];
            if triangles[*a][ca[0]] == triangles[*b][cb[0]]
                && triangles[*a][ca[1]] == triangles[*b][cb[1]]
            {
                let point = positions[triangles[*b][cb[0]]];
                triangles[*b][cb[0]] = positions.len();
                positions.push(point);
            }
        }
        request.positions = positions;
        request.triangles = triangles;
    }
}

fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        parents[i] = parents[parents[i]];
        i = parents[i];
    }
    i
}
pub(super) fn input(
    mesh: &Geometry,
    threshold: f32,
    resolution: u32,
    padding: u32,
) -> Result<Input, String> {
    mesh.validate()?;
    let mut corners = Vec::new();
    let mut triangles = Vec::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        for triangle in mesh.triangles(face) {
            let vertices = triangle.map(|c| face.corners[c].vertex);
            let [a, b, c] = vertices.map(|v| mesh.positions[v]);
            if (b - a).cross(c - a).length_squared() <= 1e-24 {
                return Err(
                    "В модели есть вырожденные грани. Исправьте их перед развёрткой.".into(),
                );
            }
            triangles.push(vertices);
            corners.push((index, triangle));
        }
    }
    if triangles.is_empty() {
        return Err("Выберите объект с поверхностью".into());
    }
    if triangles.len() > 500_000 {
        return Err("Для авторазвёртки поддерживается до 500 000 треугольников на объект".into());
    }
    Ok(Input {
        request: Request {
            positions: mesh.positions.iter().map(|p| p.to_array()).collect(),
            triangles,
            threshold,
            resolution,
            padding,
            existing_uv: None,
            options: Default::default(),
        },
        corners,
    })
}
pub(super) fn apply(
    mesh: &Geometry,
    corners: &[(usize, [usize; 3])],
    output: &Output,
) -> Result<Geometry, String> {
    if corners.len() != output.uv.len() {
        return Err("Неполный результат развёртки".into());
    }
    let mut assigned: Vec<Vec<Option<Vec2>>> = mesh
        .faces
        .iter()
        .map(|f| vec![None; f.corners.len()])
        .collect();
    let mut split = BTreeSet::new();
    let mut triangles_by_face = vec![Vec::new(); mesh.faces.len()];
    for (triangle_index, ((fi, corners), values)) in corners.iter().zip(&output.uv).enumerate() {
        triangles_by_face[*fi].push(triangle_index);
        for (ci, uv) in corners.iter().zip(values) {
            let uv = Vec2::from_array(*uv);
            if !uv.is_finite() || uv.min_element() < -0.00001 || uv.max_element() > 1.00001 {
                return Err("Некорректные UV-координаты".into());
            }
            let slot = &mut assigned[*fi][*ci];
            if slot.is_some_and(|previous| !previous.abs_diff_eq(uv, 0.000001)) {
                split.insert(*fi);
            }
            *slot = Some(uv);
        }
    }
    let mut result = Geometry {
        positions: mesh.positions.clone(),
        faces: Vec::new(),
        loose_edges: mesh.loose_edges.clone(),
        uv_seams: mesh.uv_seams.clone(),
    };
    for (fi, face) in mesh.faces.iter().enumerate() {
        if split.contains(&fi) {
            for &triangle in &triangles_by_face[fi] {
                let corners = &corners[triangle].1;
                let values = &output.uv[triangle];
                result.faces.push(Face {
                    corners: corners
                        .iter()
                        .zip(values)
                        .map(|(&ci, &uv)| {
                            let mut c = face.corners[ci].clone();
                            c.uv = Vec2::from_array(uv);
                            c
                        })
                        .collect(),
                    material: face.material,
                    smooth: face.smooth,
                });
            }
        } else {
            let mut face = face.clone();
            for (c, uv) in face.corners.iter_mut().zip(&assigned[fi]) {
                c.uv = uv.ok_or("Движок пропустил угол грани")?;
            }
            result.faces.push(face);
        }
    }
    result.validate()?;
    Ok(result)
}

#[derive(Clone, Default)]
pub(super) struct Atlas {
    pub islands: Vec<usize>,
    pub count: usize,
    pub area: f32,
    pub outside: usize,
    pub seams: Vec<Edge>,
    pub distortion: Vec<Vec<f32>>,
    pub angular_p95: Option<f32>,
}
impl Atlas {
    #[cfg(test)]
    pub fn new(mesh: &Geometry) -> Self {
        Self::with_offsets(mesh, |_| Vec2::ZERO)
    }

    pub fn with_offsets(mesh: &Geometry, offset: impl Fn(usize) -> Vec2) -> Self {
        let mut parent: Vec<_> = (0..mesh.faces.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut edges: BTreeMap<Edge, Vec<(usize, Vec2, Vec2)>> = BTreeMap::new();
        let mut atlas = Self::default();
        for (fi, f) in mesh.faces.iter().enumerate() {
            for (a, b) in f.corners.iter().zip(f.corners.iter().cycle().skip(1)) {
                let (a, b) = if a.vertex < b.vertex { (a, b) } else { (b, a) };
                edges
                    .entry(Edge::new(a.vertex, b.vertex))
                    .or_default()
                    .push((fi, a.uv, b.uv));
            }
            let origin = offset(f.material);
            if f.corners
                .iter()
                .any(|c| (c.uv - origin).min_element() < -0.00001 || (c.uv - origin).max_element() > 1.00001)
            {
                atlas.outside += 1;
            }
            for [a, b, c] in mesh.triangles(f) {
                atlas.area += (f.corners[b].uv - f.corners[a].uv)
                    .perp_dot(f.corners[c].uv - f.corners[a].uv)
                    .abs()
                    * 0.5;
            }
        }
        for (edge, adjacent) in &edges {
            let mut seam = adjacent.len() == 1;
            for (i, &(fi, a, b)) in adjacent.iter().enumerate() {
                for &(fj, c, d) in &adjacent[..i] {
                    if a.abs_diff_eq(c, 0.000001) && b.abs_diff_eq(d, 0.000001) {
                        let x = root(&mut parent, fi);
                        let y = root(&mut parent, fj);
                        parent[y] = x;
                    } else {
                        seam = true;
                    }
                }
            }
            if seam {
                atlas.seams.push(*edge);
            }
        }
        let mut ids = BTreeMap::new();
        for i in 0..mesh.faces.len() {
            let r = root(&mut parent, i);
            let n = ids.len();
            let id = *ids.entry(r).or_insert(n);
            atlas.islands.push(id);
        }
        atlas.count = ids.len();
        atlas
    }
    pub fn measure_stretch(&mut self, mesh: &Geometry) {
        let mut weighted = Vec::new();
        self.distortion = mesh
            .faces
            .iter()
            .map(|f| {
                mesh.triangles(f)
                    .into_iter()
                    .map(|tri| {
                        let (ratio, area) = angular_stretch(
                            tri.map(|i| mesh.positions[f.corners[i].vertex]),
                            tri.map(|i| f.corners[i].uv),
                        );
                        if area > 0. {
                            weighted.push((ratio, area));
                        }
                        ratio
                    })
                    .collect()
            })
            .collect();
        weighted.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let target = 0.95 * weighted.iter().map(|p| p.1).sum::<f64>();
        let mut sum = 0.;
        self.angular_p95 = weighted.into_iter().find_map(|(ratio, area)| {
            sum += area;
            (sum >= target).then_some(ratio)
        });
    }
    pub fn hit(&self, mesh: &Geometry, p: Vec2) -> Option<usize> {
        for (fi, f) in mesh.faces.iter().enumerate().rev() {
            for t in mesh.triangles(f) {
                let [a, b, c] = t.map(|ci| f.corners[ci].uv);
                let cross = [
                    (b - a).perp_dot(p - a),
                    (c - b).perp_dot(p - b),
                    (a - c).perp_dot(p - c),
                ];
                if (cross.iter().all(|&x| x >= -1e-9) || cross.iter().all(|&x| x <= 1e-9))
                    && (b - a).perp_dot(c - a).abs() > 1e-12
                {
                    return self.islands.get(fi).copied();
                }
            }
        }
        None
    }
    pub fn transform(
        &self,
        mesh: &mut Geometry,
        selected: &BTreeSet<usize>,
        translation: Vec2,
        angle: f32,
        scale: f32,
    ) {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for (fi, f) in mesh.faces.iter().enumerate() {
            if selected.contains(&self.islands[fi]) {
                for c in &f.corners {
                    min = min.min(c.uv);
                    max = max.max(c.uv);
                }
            }
        }
        if !min.is_finite() {
            return;
        }
        let pivot = (min + max) * 0.5;
        let (s, c) = angle.sin_cos();
        for (fi, f) in mesh.faces.iter_mut().enumerate() {
            if selected.contains(&self.islands[fi]) {
                for corner in &mut f.corners {
                    let p = (corner.uv - pivot) * scale;
                    corner.uv =
                        pivot + Vec2::new(c * p.x - s * p.y, s * p.x + c * p.y) + translation;
                }
            }
        }
    }
}

// Ratio of singular values of the per-triangle surface-to-UV Jacobian.
fn angular_stretch(p: [Vec3; 3], uv: [Vec2; 3]) -> (f32, f64) {
    let [a, b, c] = p.map(|v| v.as_dvec3());
    let u = b - a;
    let v = c - a;
    let twice_area = u.cross(v).length();
    if twice_area <= 0. {
        return (f32::INFINITY, 0.);
    }
    let [a, b, c] = uv.map(|v| v.as_dvec2());
    let len = u.length();
    let j0 = (b - a) / len;
    let j1 = ((c - a) - j0 * (v.dot(u) / len)) * (len / twice_area);
    let det = j0.perp_dot(j1).abs();
    if det <= 0. {
        return (f32::INFINITY, twice_area * 0.5);
    }
    let trace = j0.length_squared() + j1.length_squared();
    let high = (trace + ((trace - 2. * det) * (trace + 2. * det)).max(0.).sqrt()) * 0.5;
    ((high / det).max(1.) as f32, twice_area * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::Corner;
    fn quad() -> Geometry {
        Geometry {
            loose_edges: default(),
            uv_seams: default(),
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Y, Vec3::Y],
            faces: vec![Face {
                corners: (0..4)
                    .map(|i| {
                        let mut c = Corner::new(i);
                        c.uv1 = Vec2::splat(0.25);
                        c.color = Vec4::splat(0.7);
                        c.source_normal = Some(Vec3::Z);
                        c
                    })
                    .collect(),
                material: 3,
                smooth: true,
            }],
        }
    }
    #[test]
    fn uv_angular_heatmap_uses_singular_ratio_and_surface_area_percentiles() {
        let triangle = [Vec3::ZERO, Vec3::X, Vec3::Y];
        for tex in [
            [Vec2::ZERO, Vec2::Y * 4., -Vec2::X * 4.],
            [Vec2::ZERO, -Vec2::X, Vec2::Y],
        ] {
            assert_eq!(angular_stretch(triangle, tex), (1., 0.5));
        }
        assert_eq!(
            angular_stretch(triangle, [Vec2::ZERO, Vec2::X * 2., Vec2::Y]),
            (2., 0.5)
        );
        assert!(angular_stretch(triangle, [Vec2::ZERO; 3]).0.is_infinite());
        let mut mesh = quad();
        for c in &mut mesh.faces[0].corners {
            c.uv = mesh.positions[c.vertex].truncate();
        }
        mesh.positions
            .extend([Vec3::ZERO, Vec3::X * 0.05, Vec3::Y * 0.05]);
        mesh.faces.push(Face {
            corners: (4..7).map(Corner::new).collect(),
            material: 0,
            smooth: false,
        });
        let mut atlas = Atlas::new(&mesh);
        atlas.measure_stretch(&mesh);
        assert_eq!(atlas.angular_p95, Some(1.));
        assert!(atlas.distortion[1][0].is_infinite());
        mesh.positions[5] = Vec3::X;
        mesh.positions[6] = Vec3::Y;
        atlas.measure_stretch(&mesh);
        assert!(atlas.angular_p95.unwrap().is_infinite());
    }
    #[test]
    fn uv_mapping_keeps_geometry_materials_normals_and_quads_when_continuous() {
        let mesh = quad();
        let input = input(&mesh, 1.25, 2048, 8).unwrap();
        let uv = input
            .corners
            .iter()
            .map(|(_, t)| t.map(|i| mesh.positions[i].truncate().to_array()))
            .collect();
        let out = apply(
            &mesh,
            &input.corners,
            &Output {
                suggestion: vec![],
                uv,
                charts: 1,
                utilization: 0.8,
            },
        )
        .unwrap();
        assert_eq!(out.positions, mesh.positions);
        assert_eq!(out.faces.len(), 1);
        assert_eq!(out.faces[0].material, 3);
        for c in &out.faces[0].corners {
            assert_eq!(c.source_normal, Some(Vec3::Z));
            assert_eq!(c.uv1, Vec2::splat(0.25));
            assert_eq!(c.color, Vec4::splat(0.7));
        }
    }
    #[test]
    fn uv_cut_through_quad_splits_only_that_face_without_moving_vertices() {
        let mesh = quad();
        let input = input(&mesh, 1.25, 2048, 8).unwrap();
        let mut uv: Vec<_> = input
            .corners
            .iter()
            .map(|(_, t)| t.map(|i| mesh.positions[i].truncate().to_array()))
            .collect();
        uv[1] = [[0.1, 0.1], [0.4, 0.1], [0.2, 0.4]];
        let out = apply(
            &mesh,
            &input.corners,
            &Output {
                suggestion: vec![],
                uv,
                charts: 2,
                utilization: 0.5,
            },
        )
        .unwrap();
        assert_eq!(out.positions, mesh.positions);
        assert_eq!(out.faces.len(), 2);
        assert!(
            out.faces
                .iter()
                .all(|f| f.material == 3 && f.corners.len() == 3)
        );
        assert_eq!(Atlas::new(&out).count, 2);
    }
    #[test]
    fn uv_rejects_partial_nonfinite_output_and_keeps_secondary_channel_on_transforms() {
        let mesh = quad();
        let input = input(&mesh, 1.25, 2048, 8).unwrap();
        assert!(
            apply(
                &mesh,
                &input.corners,
                &Output {
                    suggestion: vec![],
                    uv: vec![],
                    charts: 0,
                    utilization: 0.
                }
            )
            .is_err()
        );
        let mut out = mesh.clone();
        for c in &mut out.faces[0].corners {
            c.uv = out.positions[c.vertex].truncate();
        }
        let atlas = Atlas::new(&out);
        assert_eq!(atlas.hit(&out, Vec2::splat(0.5)), Some(0));
        atlas.transform(&mut out, &BTreeSet::from([0]), Vec2::X * 0.1, 0., 0.9);
        assert_eq!(out.positions, mesh.positions);
        assert!(
            out.faces[0]
                .corners
                .iter()
                .all(|c| c.uv1 == Vec2::splat(0.25))
        );
    }
}
