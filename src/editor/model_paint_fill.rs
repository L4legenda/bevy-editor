//! Geometry selection for polygon fill. Triangle indices follow the paint vertex buffer.
use super::{Attributes, FillMode, Object, PickTriangle};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

type FaceId = (usize, usize);
type Edge = (usize, usize, usize);
const UV_GRID: usize = 32;

struct Face {
    triangles: Vec<usize>,
    edges: Vec<(Edge, [Vec3; 2])>,
}

pub(super) struct Topology {
    faces: HashMap<FaceId, Face>,
    borders: HashMap<Edge, Vec<FaceId>>,
    owners: Vec<FaceId>,
    partners: Vec<Option<usize>>,
    loop_owner: HashMap<FaceId, FaceId>,
    loop_quads: HashMap<FaceId, Face>,
    objects: HashMap<usize, Vec<usize>>,
    sets: HashMap<(usize, usize), Vec<usize>>,
    adjacent: Vec<Vec<usize>>,
    normals: Vec<Vec3>,
    parts: Groups,
    islands: Groups,
    uv_border: Vec<[bool; 3]>,
    uv_bins: Vec<Vec<usize>>,
    wide_uv: Vec<usize>,
}

struct Groups {
    ids: Vec<usize>,
    members: Vec<Vec<usize>>,
}

fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

impl Groups {
    fn build(count: usize, edges: impl IntoIterator<Item = Vec<usize>>) -> Self {
        let mut parents: Vec<_> = (0..count).collect();
        for users in edges {
            let first = root(&mut parents, users[0]);
            for &other in &users[1..] {
                let other = root(&mut parents, other);
                parents[other] = first;
            }
        }
        let ids: Vec<_> = (0..count).map(|index| root(&mut parents, index)).collect();
        let mut members = vec![Vec::new(); count];
        for (index, &id) in ids.iter().enumerate() {
            members[id].push(index);
        }
        Self { ids, members }
    }

    fn of(&self, index: usize) -> &[usize] {
        &self.members[self.ids[index]]
    }
}

fn edge(object: usize, a: usize, b: usize) -> Edge {
    (object, a.min(b), a.max(b))
}

fn uv_cell(uv: Vec2) -> UVec2 {
    (uv * UV_GRID as f32)
        .as_uvec2()
        .min(UVec2::splat(UV_GRID as u32 - 1))
}

impl Topology {
    pub fn build(
        objects: &[Object],
        visibility: &super::super::sets::SetVisibility,
        triangles: &[PickTriangle],
        attributes: &[Attributes],
    ) -> Self {
        let mut faces = HashMap::new();
        let mut borders: HashMap<_, Vec<_>> = HashMap::new();
        for object in objects.iter().filter(|object| !object.hidden) {
            let point = |v: usize| {
                object
                    .transform
                    .transform_point(object.geometry.positions[v])
            };
            for (face, polygon) in object.geometry.faces.iter().enumerate() {
                if !visibility.shown(object.id, polygon.material) {
                    continue;
                }
                let id = (object.id, face);
                let edges = polygon
                    .edges()
                    .map(|(a, b)| {
                        let edge = edge(object.id, a, b);
                        borders.entry(edge).or_default().push(id);
                        (edge, [point(a), point(b)])
                    })
                    .collect();
                faces.insert(
                    id,
                    Face {
                        triangles: Vec::new(),
                        edges,
                    },
                );
            }
        }
        let mut surface_edges: HashMap<_, Vec<_>> = HashMap::new();
        let mut uv_edges: HashMap<_, Vec<_>> = HashMap::new();
        let mut by_object: HashMap<_, Vec<_>> = HashMap::new();
        let mut sets: HashMap<_, Vec<_>> = HashMap::new();
        let mut uv_bins = vec![Vec::new(); UV_GRID * UV_GRID];
        let mut wide_uv = Vec::new();
        let mut normals = Vec::with_capacity(triangles.len());
        let quantize = |uv: Vec2| (uv * 100_000.).round().as_ivec2().to_array();
        for (index, (triangle, data)) in triangles.iter().zip(attributes).enumerate() {
            let object = triangle.object;
            faces
                .get_mut(&(object, triangle.face))
                .unwrap()
                .triangles
                .push(index);
            by_object.entry(object).or_default().push(index);
            sets.entry((object, data.slot)).or_default().push(index);
            let [a, b, c] = data.points;
            normals.push((b - a).cross(c - a).normalize_or_zero());
            for i in 0..3 {
                let j = (i + 1) % 3;
                let (a, b) = (data.vertices[i], data.vertices[j]);
                let key = edge(object, a, b);
                surface_edges.entry(key).or_default().push(index);
                let (u, v) = (quantize(data.uv[i]), quantize(data.uv[j]));
                let uv_pair = if a <= b { (u, v) } else { (v, u) };
                uv_edges
                    .entry((key, uv_pair))
                    .or_default()
                    .push(index * 3 + i);
            }
            let [a, b, c] = data.uv;
            let min = uv_cell(a.min(b).min(c));
            let max = uv_cell(a.max(b).max(c));
            if ((max - min + UVec2::ONE).element_product()) > 16 {
                wide_uv.push(index);
            } else {
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        uv_bins[y as usize * UV_GRID + x as usize].push(index);
                    }
                }
            }
        }
        let mut adjacent = vec![Vec::new(); triangles.len()];
        for users in surface_edges.values() {
            if let [a, b] = users.as_slice() {
                adjacent[*a].push(*b);
                adjacent[*b].push(*a);
            }
        }
        let owners: Vec<_> = triangles.iter().map(|t| (t.object, t.face)).collect();
        let longest: Vec<_> = attributes
            .iter()
            .zip(triangles)
            .map(|(data, t)| {
                let i = (0..3)
                    .max_by(|&a, &b| {
                        let length =
                            |k: usize| data.points[k].distance_squared(data.points[(k + 1) % 3]);
                        length(a).total_cmp(&length(b))
                    })
                    .unwrap();
                edge(t.object, data.vertices[i], data.vertices[(i + 1) % 3])
            })
            .collect();
        let mut partners = vec![None; triangles.len()];
        let mut loop_owner = HashMap::new();
        let mut loop_quads = HashMap::new();
        for (i, &diagonal) in longest.iter().enumerate() {
            let [a, b] = surface_edges[&diagonal].as_slice() else {
                continue;
            };
            let j = if *a == i { *b } else { *a };
            let (left, right) = (&faces[&owners[i]], &faces[&owners[j]]);
            if i >= j
                || longest[j] != diagonal
                || left.edges.len() != 3
                || right.edges.len() != 3
                || normals[i].dot(normals[j]) <= 0.5
            {
                continue;
            }
            let k = left
                .edges
                .iter()
                .position(|&(key, _)| key == diagonal)
                .unwrap();
            let m = right
                .edges
                .iter()
                .position(|&(key, _)| key == diagonal)
                .unwrap();
            let mut edges = vec![left.edges[(k + 1) % 3], left.edges[(k + 2) % 3]];
            let mut other = [right.edges[(m + 1) % 3], right.edges[(m + 2) % 3]];
            if left.edges[k].1[1] != right.edges[m].1[0] {
                other.reverse();
            }
            edges.extend(other);
            partners[i] = Some(j);
            partners[j] = Some(i);
            loop_owner.insert(owners[j], owners[i]);
            loop_quads.insert(
                owners[i],
                Face {
                    triangles: vec![i, j],
                    edges,
                },
            );
        }
        let mut uv_border = vec![[false; 3]; triangles.len()];
        for sides in uv_edges.values() {
            if let [side] = sides.as_slice() {
                uv_border[side / 3][side % 3] = true;
            }
        }
        Self {
            faces,
            borders,
            owners,
            partners,
            loop_owner,
            loop_quads,
            objects: by_object,
            sets,
            adjacent,
            normals,
            parts: Groups::build(triangles.len(), surface_edges.into_values()),
            islands: Groups::build(
                triangles.len(),
                uv_edges
                    .into_values()
                    .map(|sides| sides.into_iter().map(|side| side / 3).collect()),
            ),
            uv_border,
            uv_bins,
            wide_uv,
        }
    }

    pub fn pick_uv(
        &self,
        uv: Vec2,
        data: &[Attributes],
        target: impl Fn(usize) -> bool,
    ) -> Option<(usize, Vec3)> {
        // The last drawn triangle wins in overlapping charts, as in the UV ID pass.
        self.uv_hits(uv, data, target)
            .max_by_key(|&(index, _)| index)
    }

    pub fn pick_first_uv(
        &self,
        uv: Vec2,
        data: &[Attributes],
        target: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        self.uv_hits(uv, data, target).map(|(index, _)| index).min()
    }

    fn uv_hits<'a>(
        &'a self,
        uv: Vec2,
        data: &'a [Attributes],
        target: impl Fn(usize) -> bool + 'a,
    ) -> impl Iterator<Item = (usize, Vec3)> + 'a {
        let cell = uv_cell(uv);
        let local = &self.uv_bins[cell.y as usize * UV_GRID + cell.x as usize];
        local
            .iter()
            .chain(&self.wide_uv)
            .copied()
            .filter(move |&index| target(index))
            .filter_map(move |index| {
                let [a, b, c] = data[index].uv;
                let d = (b - a).perp_dot(c - a);
                if d.abs() <= 1e-12 {
                    return None;
                }
                let u = (uv - a).perp_dot(c - a) / d;
                let v = (b - a).perp_dot(uv - a) / d;
                let w = 1. - u - v;
                let [a, b, c] = data[index].points;
                (u.min(v).min(w) >= -1e-6).then_some((index, a * w + b * u + c * v))
            })
    }

    pub fn select(
        &self,
        index: usize,
        point: Vec3,
        mode: FillMode,
        min_dot: f32,
        data: &[Attributes],
    ) -> Vec<usize> {
        let id = self.owners[index];
        match mode {
            FillMode::Triangle => vec![index],
            FillMode::Polygon => match self.partners[index] {
                Some(other) if self.normals[index].dot(self.normals[other]) > 0.999 => {
                    vec![index, other]
                }
                _ => self.faces[&id].triangles.clone(),
            },
            FillMode::Loop => self.face_loop(id, point),
            FillMode::Flat => {
                let mut seen = HashSet::from([index]);
                let mut pending = vec![index];
                while let Some(current) = pending.pop() {
                    for &next in &self.adjacent[current] {
                        if self.normals[current].dot(self.normals[next]) >= min_dot - 1e-6
                            && seen.insert(next)
                        {
                            pending.push(next);
                        }
                    }
                }
                seen.into_iter().collect()
            }
            FillMode::Part => self.parts.of(index).to_vec(),
            FillMode::Object => self.objects[&id.0].clone(),
            FillMode::UvIsland => self.islands.of(index).to_vec(),
            FillMode::Set => self.sets[&(id.0, data[index].slot)].clone(),
        }
    }

    fn face_loop(&self, id: FaceId, point: Vec3) -> Vec<usize> {
        let canonical = |id| self.loop_owner.get(&id).copied().unwrap_or(id);
        let outline = |id| self.loop_quads.get(&id).unwrap_or(&self.faces[&id]);
        let id = canonical(id);
        let face = outline(id);
        let mut selected = face.triangles.clone();
        if face.edges.len() != 4 {
            return selected;
        }
        let distance = |[a, b]: [Vec3; 2]| {
            let axis = b - a;
            let t = ((point - a).dot(axis) / axis.length_squared().max(1e-12)).clamp(0., 1.);
            point.distance_squared(a + t * axis)
        };
        let exit = (0..4)
            .min_by(|&a, &b| distance(face.edges[a].1).total_cmp(&distance(face.edges[b].1)))
            .unwrap();
        let mut seen = HashSet::from([id]);
        for direction in [exit, (exit + 2) % 4] {
            let mut current = id;
            let mut edge = face.edges[direction].0;
            while let [a, b] = self.borders[&edge].as_slice() {
                let (a, b) = (canonical(*a), canonical(*b));
                let next = if a == current { b } else { a };
                let face = outline(next);
                if face.edges.len() != 4 || !seen.insert(next) {
                    break;
                }
                selected.extend(&face.triangles);
                let entry = face.edges.iter().position(|&(key, _)| key == edge).unwrap();
                edge = face.edges[(entry + 2) % 4].0;
                current = next;
            }
        }
        selected
    }

    pub fn padding(
        &self,
        indices: impl IntoIterator<Item = usize>,
        data: &[Attributes],
        pad: f32,
    ) -> Vec<Vec2> {
        let mut vertices = Vec::new();
        for index in indices {
            let uv = data[index].uv;
            if (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs() <= 1e-12 {
                continue;
            }
            for (i, &border) in self.uv_border[index].iter().enumerate() {
                if !border {
                    continue;
                }
                let [a, b, c] = [uv[i], uv[(i + 1) % 3], uv[(i + 2) % 3]];
                let along = (b - a).normalize() * pad;
                let mut outward = along.perp();
                if outward.dot(c - a) > 0. {
                    outward = -outward;
                }
                let (a, b) = (a - along, b + along);
                vertices.extend([a, b, b + outward, a, b + outward, a + outward]);
            }
        }
        vertices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_mesh::{Corner, Face as Polygon, Geometry};

    #[test]
    fn polygon_fill_preview_outlines_a_closed_cube() {
        use super::super::{CameraMap, FillTarget, SurfaceTrace};
        use crate::editor::model_primitives::Primitive;
        let cube = Primitive::Cube;
        let geometry = cube.geometry(&cube.defaults()).unwrap();
        assert!(geometry.edges().values().all(|users| users.len() == 2));
        let object = Object {
            id: 0,
            source_node: None,
            name: String::new(),
            geometry: geometry.into(),
            transform: Transform::IDENTITY,
            materials: Vec::new(),
            hidden: false,
        };
        let rect = Rect::from_corners(Vec2::ZERO, Vec2::splat(512.));
        let pose = Transform::from_xyz(0., 0., 5.);
        let target = FillTarget {
            view: super::super::super::gpu::View {
                camera: CameraMap {
                    pose,
                    inverse: pose.to_matrix().inverse(),
                    rect,
                    clip: rect,
                    ortho: false,
                    fov: crate::editor::viewport::EDITOR_FOV,
                    height: 4.,
                    depth: 10.,
                    epsilon: 0.001,
                },
                uv: false,
            },
            object: 0,
            slot: 0,
            mode: FillMode::Object,
            angle: 30.,
        };
        let outline = SurfaceTrace::prepare_fill(&[object]).fill_outline(target, rect.center());
        assert_eq!(outline.len(), 4);
        assert!(
            outline
                .iter()
                .all(|edge| edge.world.iter().all(|p| p.z > 0.))
        );
    }

    #[test]
    fn polygon_fill_modes_follow_geometry_uv_seams_and_materials() {
        let positions = vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(1., 0., 0.),
            Vec3::new(1., 1., 0.),
            Vec3::new(0., 1., 0.),
            Vec3::new(2., 0., 0.),
            Vec3::new(2., 1., 0.),
            Vec3::new(2., 0., 1.),
            Vec3::new(2., 1., 1.),
            Vec3::new(4., 0., 0.),
            Vec3::new(5., 0., 0.),
            Vec3::new(5., 1., 0.),
            Vec3::new(4., 1., 0.),
        ];
        let faces = [[0, 1, 2, 3], [1, 4, 5, 2], [4, 6, 7, 5], [8, 9, 10, 11]]
            .into_iter()
            .enumerate()
            .map(|(face, vertices)| Polygon {
                corners: vertices
                    .into_iter()
                    .zip([Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y])
                    .map(|(v, uv)| {
                        let mut corner = Corner::new(v);
                        corner.uv = uv * 0.2 + Vec2::X * face as f32 * 0.25;
                        corner
                    })
                    .collect(),
                material: usize::from(face == 2),
                smooth: false,
            })
            .collect();
        let object = Object {
            id: 0,
            source_node: None,
            name: String::new(),
            geometry: Geometry {
                positions,
                faces,
                ..default()
            }
            .into(),
            transform: Transform::IDENTITY,
            materials: Vec::new(),
            hidden: false,
        };
        let mut triangulated = object.clone();
        triangulated.geometry.faces = object
            .geometry
            .faces
            .iter()
            .flat_map(|face| {
                [[0, 1, 2], [0, 2, 3]].map(|indices| Polygon {
                    corners: indices.map(|i| face.corners[i].clone()).to_vec(),
                    material: face.material,
                    smooth: face.smooth,
                })
            })
            .collect();
        let split = super::super::SurfaceTrace::prepare_fill(&[triangulated]);
        let trace = super::super::SurfaceTrace::prepare_fill(&[object]);
        let topology = trace.topology.as_ref().unwrap();
        let point = Vec3::new(0.99, 0.5, 0.);
        let select = |mode, angle: f32| {
            let mut result =
                topology.select(0, point, mode, angle.to_radians().cos(), &trace.attributes);
            result.sort_unstable();
            result
        };
        assert_eq!(select(FillMode::Triangle, 30.), [0]);
        assert_eq!(select(FillMode::Polygon, 30.), [0, 1]);
        assert_eq!(select(FillMode::Loop, 30.), [0, 1, 2, 3, 4, 5]);
        assert_eq!(select(FillMode::Flat, 30.), [0, 1, 2, 3]);
        assert_eq!(select(FillMode::Flat, 100.), [0, 1, 2, 3, 4, 5]);
        assert_eq!(select(FillMode::Part, 30.), [0, 1, 2, 3, 4, 5]);
        assert_eq!(select(FillMode::Object, 30.), [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(select(FillMode::UvIsland, 30.), [0, 1]);
        assert_eq!(select(FillMode::Set, 30.), [0, 1, 2, 3, 6, 7]);
        let inferred = split.topology.as_ref().unwrap();
        let recovered = |mode| {
            let mut faces = inferred.select(0, point, mode, 0.9, &split.attributes);
            faces.sort_unstable();
            faces
        };
        assert_eq!(recovered(FillMode::Polygon), [0, 1]);
        assert_eq!(recovered(FillMode::Loop), [0, 1, 2, 3, 4, 5]);
        assert_eq!(inferred.padding([0, 1], &split.attributes, 0.01).len(), 24);
        let (index, world) = topology
            .pick_uv(Vec2::new(0.08, 0.11), &trace.attributes, |_| true)
            .unwrap();
        assert!(index < 2);
        assert!(world.abs_diff_eq(Vec3::new(0.4, 0.55, 0.), 1e-5));
    }
}
