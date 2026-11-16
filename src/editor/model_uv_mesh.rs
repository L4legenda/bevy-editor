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
        },
        corners,
    })
}
pub(super) fn apply(mesh: &Geometry, input: &Input, output: &Output) -> Result<Geometry, String> {
    if input.corners.len() != output.uv.len() {
        return Err("Неполный результат развёртки".into());
    }
    let mut assigned: Vec<Vec<Option<Vec2>>> = mesh
        .faces
        .iter()
        .map(|f| vec![None; f.corners.len()])
        .collect();
    let mut split = BTreeSet::new();
    let mut triangles_by_face = vec![Vec::new(); mesh.faces.len()];
    for (triangle_index, ((fi, corners), values)) in
        input.corners.iter().zip(&output.uv).enumerate()
    {
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
    };
    for (fi, face) in mesh.faces.iter().enumerate() {
        if split.contains(&fi) {
            for &triangle in &triangles_by_face[fi] {
                let corners = &input.corners[triangle].1;
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
}
impl Atlas {
    pub fn new(mesh: &Geometry) -> Self {
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
            if f.corners
                .iter()
                .any(|c| c.uv.min_element() < -0.00001 || c.uv.max_element() > 1.00001)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::Corner;
    fn quad() -> Geometry {
        Geometry {
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
            &input,
            &Output {
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
            &input,
            &Output {
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
                &input,
                &Output {
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
