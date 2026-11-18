//! Polygon topology for model documents, independent of rendering and ECS entities.
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct Edge(pub usize, pub usize);
impl Edge {
    pub fn new(a: usize, b: usize) -> Self {
        Self(a.min(b), a.max(b))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Corner {
    pub vertex: usize,
    pub uv: Vec2,
    pub uv1: Vec2,
    pub color: Vec4,
    pub source_normal: Option<Vec3>,
}
impl Corner {
    pub fn new(vertex: usize) -> Self {
        Self {
            vertex,
            uv: Vec2::ZERO,
            uv1: Vec2::ZERO,
            color: Vec4::ONE,
            source_normal: None,
        }
    }
    fn between(&self, other: &Self, vertex: usize, t: f32) -> Self {
        Self {
            vertex,
            uv: self.uv.lerp(other.uv, t),
            uv1: self.uv1.lerp(other.uv1, t),
            color: self.color.lerp(other.color, t),
            source_normal: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Face {
    pub corners: Vec<Corner>,
    pub material: usize,
    pub smooth: bool,
}
impl Face {
    pub fn edges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.corners
            .iter()
            .zip(self.corners.iter().cycle().skip(1))
            .map(|(a, b)| (a.vertex, b.vertex))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Geometry {
    pub positions: Vec<Vec3>,
    pub faces: Vec<Face>,
    pub loose_edges: BTreeSet<Edge>,
}

#[derive(Default, Clone)]
pub(super) struct Buffers {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub uv1: Vec<[f32; 2]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub polygons: Vec<Vec<u32>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Element {
    #[default]
    Vertex,
    Edge,
    Face,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Selection {
    pub kind: Element,
    pub vertices: BTreeSet<usize>,
    pub edges: BTreeSet<Edge>,
    pub faces: BTreeSet<usize>,
}
impl Selection {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.edges.clear();
        self.faces.clear();
    }
    pub fn all(&mut self, mesh: &Geometry) {
        self.clear();
        match self.kind {
            Element::Vertex => self.vertices.extend(0..mesh.positions.len()),
            Element::Edge => self.edges.extend(mesh.edges().keys().copied()),
            Element::Face => self.faces.extend(0..mesh.faces.len()),
        }
    }
    pub fn selected_vertices(&self, mesh: &Geometry) -> BTreeSet<usize> {
        match self.kind {
            Element::Vertex => self
                .vertices
                .iter()
                .copied()
                .filter(|&v| v < mesh.positions.len())
                .collect(),
            Element::Edge => self
                .edges
                .iter()
                .flat_map(|e| [e.0, e.1])
                .filter(|&v| v < mesh.positions.len())
                .collect(),
            Element::Face => self
                .faces
                .iter()
                .filter_map(|&f| mesh.faces.get(f))
                .flat_map(|f| f.corners.iter().map(|c| c.vertex))
                .collect(),
        }
    }
    pub fn selected_faces(&self, mesh: &Geometry) -> BTreeSet<usize> {
        mesh.faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let yes = match self.kind {
                    Element::Face => self.faces.contains(&i),
                    Element::Vertex => f.corners.iter().all(|c| self.vertices.contains(&c.vertex)),
                    Element::Edge => f
                        .edges()
                        .all(|(a, b)| self.edges.contains(&Edge::new(a, b))),
                };
                yes.then_some(i)
            })
            .collect()
    }
    pub fn convert(&mut self, kind: Element, mesh: &Geometry) {
        let vertices = self.selected_vertices(mesh);
        self.clear();
        self.kind = kind;
        match kind {
            Element::Vertex => self.vertices = vertices,
            Element::Edge => {
                self.edges = mesh
                    .edges()
                    .into_keys()
                    .filter(|e| vertices.contains(&e.0) && vertices.contains(&e.1))
                    .collect()
            }
            Element::Face => {
                self.faces = mesh
                    .faces
                    .iter()
                    .enumerate()
                    .filter_map(|(i, f)| {
                        f.corners
                            .iter()
                            .all(|c| vertices.contains(&c.vertex))
                            .then_some(i)
                    })
                    .collect()
            }
        }
    }
    pub fn invert(&mut self, mesh: &Geometry) {
        let old = self.clone();
        self.all(mesh);
        self.vertices.retain(|v| !old.vertices.contains(v));
        self.edges.retain(|v| !old.edges.contains(v));
        self.faces.retain(|v| !old.faces.contains(v));
    }
    pub fn connected(&mut self, mesh: &Geometry, grow: bool, shrink: bool) {
        let mut set = self.selected_vertices(mesh);
        let edges = mesh.edges();
        if shrink {
            let boundary: BTreeSet<_> = edges
                .keys()
                .filter(|e| set.contains(&e.0) != set.contains(&e.1))
                .flat_map(|e| [e.0, e.1])
                .collect();
            set.retain(|v| !boundary.contains(v));
        } else {
            loop {
                let before = set.len();
                let add: Vec<_> = edges
                    .keys()
                    .filter(|e| set.contains(&e.0) || set.contains(&e.1))
                    .flat_map(|e| [e.0, e.1])
                    .collect();
                set.extend(add);
                if grow || set.len() == before {
                    break;
                }
            }
        }
        let kind = self.kind;
        self.clear();
        self.kind = Element::Vertex;
        self.vertices = set;
        self.convert(kind, mesh);
    }
}

impl Geometry {
    pub fn validate(&self) -> Result<(), String> {
        if self.positions.len() > 1_000_000 || self.faces.len() > 1_000_000 {
            return Err("Лимит: 1 000 000 вершин или граней на объект".into());
        }
        if self
            .positions
            .iter()
            .any(|p| !p.is_finite() || p.abs().max_element() > 1.0e12)
        {
            return Err("Недопустимые координаты вершин".into());
        }
        if self
            .loose_edges
            .iter()
            .any(|e| e.0 >= e.1 || e.1 >= self.positions.len())
        {
            return Err("Повреждённое свободное ребро".into());
        }
        for face in &self.faces {
            if face.corners.len() < 3 || face.corners.len() > 4096 {
                return Err("Грань должна содержать от 3 до 4096 вершин".into());
            }
            let mut unique = HashSet::new();
            if face.corners.iter().any(|c| {
                c.vertex >= self.positions.len()
                    || !unique.insert(c.vertex)
                    || !c.uv.is_finite()
                    || !c.uv1.is_finite()
                    || !c.color.is_finite()
                    || c.source_normal.is_some_and(|n| !n.is_finite())
            }) {
                return Err("Повреждённая топология или UV".into());
            }
            let edge_scale = face
                .edges()
                .map(|(a, b)| self.positions[a].distance_squared(self.positions[b]))
                .fold(0.0_f32, f32::max);
            if edge_scale == 0.0
                || self.normal(face).length_squared() <= edge_scale * edge_scale * 1.0e-14
            {
                return Err("Вырожденная грань: операция отменена".into());
            }
            if self.triangles(face).len() != face.corners.len() - 2 {
                return Err("Грань пересекает себя: операция отменена".into());
            }
        }
        Ok(())
    }
    pub fn normal(&self, face: &Face) -> Vec3 {
        face.corners
            .iter()
            .zip(face.corners.iter().cycle().skip(1))
            .fold(Vec3::ZERO, |n, (a, b)| {
                let p = self.positions[a.vertex];
                let q = self.positions[b.vertex];
                n + Vec3::new(
                    (p.y - q.y) * (p.z + q.z),
                    (p.z - q.z) * (p.x + q.x),
                    (p.x - q.x) * (p.y + q.y),
                )
            })
    }
    pub fn discard_normals(&mut self) {
        for face in &mut self.faces {
            for corner in &mut face.corners {
                corner.source_normal = None;
            }
        }
    }
    pub fn center(&self, face: &Face) -> Vec3 {
        face.corners
            .iter()
            .map(|c| self.positions[c.vertex])
            .sum::<Vec3>()
            / face.corners.len() as f32
    }
    pub fn radius(&self) -> f32 {
        let min = self
            .positions
            .iter()
            .copied()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let max = self
            .positions
            .iter()
            .copied()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        if min.is_finite() {
            (max - min).length().max(0.0001)
        } else {
            1.0
        }
    }
    pub fn edges(&self) -> BTreeMap<Edge, Vec<usize>> {
        let mut result: BTreeMap<Edge, Vec<usize>> = self
            .loose_edges
            .iter()
            .map(|&edge| (edge, Vec::new()))
            .collect();
        for (i, face) in self.faces.iter().enumerate() {
            for (a, b) in face.edges() {
                result.entry(Edge::new(a, b)).or_default().push(i);
            }
        }
        result
    }
    pub fn line_positions(&self) -> Vec<[f32; 3]> {
        self.loose_edges
            .iter()
            .flat_map(|e| [e.0, e.1])
            .map(|v| self.positions[v].to_array())
            .collect()
    }
    pub fn triangles(&self, face: &Face) -> Vec<[usize; 3]> {
        let n = self.normal(face).abs();
        let axis = if n.x > n.y && n.x > n.z {
            0
        } else if n.y > n.z {
            1
        } else {
            2
        };
        let project = |v: Vec3| match axis {
            0 => v.yz(),
            1 => v.zx(),
            _ => v.xy(),
        };
        let points: Vec<_> = face
            .corners
            .iter()
            .map(|c| project(self.positions[c.vertex]))
            .collect();
        let area: f32 = points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(a, b)| a.perp_dot(*b))
            .sum();
        let sign = area.signum();
        let mut polygon: Vec<_> = (0..points.len()).collect();
        let mut result = Vec::new();
        let min = points
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let max = points
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
        let epsilon = (max - min).length_squared() * 1.0e-10;
        while polygon.len() > 3 {
            let ear = (0..polygon.len()).find(|&i| {
                let a = polygon[(i + polygon.len() - 1) % polygon.len()];
                let b = polygon[i];
                let c = polygon[(i + 1) % polygon.len()];
                if (points[b] - points[a]).perp_dot(points[c] - points[b]) * sign <= epsilon {
                    return false;
                }
                !polygon
                    .iter()
                    .copied()
                    .filter(|v| ![a, b, c].contains(v))
                    .any(|v| {
                        [(a, b), (b, c), (c, a)].into_iter().all(|(x, y)| {
                            (points[y] - points[x]).perp_dot(points[v] - points[x]) * sign
                                >= -epsilon
                        })
                    })
            });
            let Some(i) = ear else {
                break;
            };
            result.push([
                polygon[(i + polygon.len() - 1) % polygon.len()],
                polygon[i],
                polygon[(i + 1) % polygon.len()],
            ]);
            polygon.remove(i);
        }
        if polygon.len() == 3 {
            result.push([polygon[0], polygon[1], polygon[2]]);
        }
        result
    }
    fn select_faces(&self, selection: &mut Selection, faces: impl IntoIterator<Item = usize>) {
        let kind = selection.kind;
        selection.clear();
        selection.kind = Element::Face;
        selection.faces.extend(faces);
        selection.convert(kind, self);
    }
    pub fn transform(
        &mut self,
        selection: &Selection,
        matrix: Mat4,
        proportional: Option<(f32, bool)>,
    ) {
        let selected = selection.selected_vertices(self);
        let originals = self.positions.clone();
        let linked = proportional.filter(|(_, connected)| *connected).map(|_| {
            let mut s = selection.clone();
            s.connected(self, false, false);
            s.selected_vertices(self)
        });
        for (i, p) in self.positions.iter_mut().enumerate() {
            let weight = if selected.contains(&i) {
                1.0
            } else if let Some((radius, _)) = proportional {
                if linked.as_ref().is_some_and(|linked| !linked.contains(&i)) {
                    0.0
                } else {
                    let distance = selected
                        .iter()
                        .map(|&v| originals[v].distance(originals[i]))
                        .fold(f32::INFINITY, f32::min);
                    let t = (1.0 - distance / radius.max(0.000001)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                }
            } else {
                0.0
            };
            if weight > 0.0 {
                *p = p.lerp(matrix.transform_point3(*p), weight);
            }
        }
    }
    pub fn duplicate(&mut self, selection: &mut Selection) -> Result<(), String> {
        let faces = selection.selected_faces(self);
        if faces.is_empty() {
            return Err("Выберите грани для дублирования".into());
        }
        let mut vertices = BTreeMap::new();
        let start = self.faces.len();
        for i in faces {
            let mut face = self.faces[i].clone();
            for c in &mut face.corners {
                c.vertex = *vertices.entry(c.vertex).or_insert_with(|| {
                    self.positions.push(self.positions[c.vertex]);
                    self.positions.len() - 1
                });
            }
            self.faces.push(face);
        }
        self.select_faces(selection, start..self.faces.len());
        Ok(())
    }
    pub fn extrude(&mut self, selection: &mut Selection, distance: f32) -> Result<(), String> {
        for (vertex, origin, normal) in self.prepare_extrude(selection)? {
            self.positions[vertex] = origin + normal * distance;
        }
        Ok(())
    }
    pub fn prepare_extrude(
        &mut self,
        selection: &mut Selection,
    ) -> Result<Vec<(usize, Vec3, Vec3)>, String> {
        if selection.kind != Element::Face {
            return self.extrude_elements(selection);
        }
        let selected = selection.selected_faces(self);
        if selected.is_empty() {
            return Err("Для выдавливания выделите грани или замкнутую область".into());
        }
        let mut normals: BTreeMap<usize, Vec3> = BTreeMap::new();
        for &i in &selected {
            let n = self.normal(&self.faces[i]);
            for c in &self.faces[i].corners {
                *normals.entry(c.vertex).or_default() += n;
            }
        }
        let mut remap = BTreeMap::new();
        let mut motion = Vec::with_capacity(normals.len());
        for (v, n) in normals {
            let p = self.positions[v];
            motion.push((self.positions.len(), p, n.normalize_or_zero()));
            remap.insert(v, self.positions.len());
            self.positions.push(p);
        }
        let adjacency = self.edges();
        let mut walls = Vec::new();
        for &i in &selected {
            let face = &self.faces[i];
            for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
                if adjacency[&Edge::new(a.vertex, b.vertex)]
                    .iter()
                    .filter(|f| selected.contains(f))
                    .count()
                    != 1
                {
                    continue;
                }
                let mut na = a.clone();
                na.vertex = remap[&a.vertex];
                let mut nb = b.clone();
                nb.vertex = remap[&b.vertex];
                walls.push(Face {
                    corners: vec![a.clone(), b.clone(), nb, na],
                    material: face.material,
                    smooth: face.smooth,
                });
            }
        }
        for &i in &selected {
            for c in &mut self.faces[i].corners {
                c.vertex = remap[&c.vertex];
            }
        }
        self.faces.extend(walls);
        let compacted = self.compact_vertices();
        for (vertex, _, _) in &mut motion {
            *vertex = compacted[vertex];
        }
        self.select_faces(selection, selected);
        Ok(motion)
    }
    pub fn extrusion_direction(&self, selection: &Selection) -> Option<Vec3> {
        let vertices = selection.selected_vertices(self);
        if vertices.is_empty() {
            return None;
        }
        let mut normals = self
            .faces
            .iter()
            .enumerate()
            .filter(|(i, face)| match selection.kind {
                Element::Face => selection.faces.contains(i),
                Element::Edge => face
                    .edges()
                    .any(|(a, b)| selection.edges.contains(&Edge::new(a, b))),
                Element::Vertex => face.corners.iter().any(|c| vertices.contains(&c.vertex)),
            })
            .map(|(_, face)| self.normal(face));
        let first = normals.next().unwrap_or(Vec3::ZERO);
        let normal = (first + normals.sum::<Vec3>())
            .try_normalize()
            .or_else(|| first.try_normalize())
            .unwrap_or(Vec3::Y);
        let tangent = if selection.kind == Element::Edge {
            selection.edges.first().map(|e| [e.0, e.1])
        } else if selection.kind == Element::Vertex {
            self.loose_edges
                .iter()
                .rev()
                .find(|e| vertices.contains(&e.0) || vertices.contains(&e.1))
                .map(|e| [e.0, e.1])
        } else {
            None
        };
        let Some([a, b]) =
            tangent.filter(|&[a, b]| a < self.positions.len() && b < self.positions.len())
        else {
            return Some(normal);
        };
        let tangent = (self.positions[b] - self.positions[a]).normalize_or_zero();
        if selection.kind == Element::Vertex && first == Vec3::ZERO && tangent != Vec3::ZERO {
            return Some(tangent);
        }
        if normal.cross(tangent).length_squared() < 1.0e-4 && tangent != Vec3::ZERO {
            Some(tangent.any_orthonormal_vector())
        } else {
            Some(normal)
        }
    }
    fn extrude_elements(
        &mut self,
        selection: &mut Selection,
    ) -> Result<Vec<(usize, Vec3, Vec3)>, String> {
        let adjacency = if selection.kind == Element::Edge {
            self.edges()
        } else {
            BTreeMap::new()
        };
        let edges: Vec<_> = selection
            .edges
            .iter()
            .copied()
            .filter(|e| adjacency.contains_key(e))
            .collect();
        let vertices = if selection.kind == Element::Vertex {
            selection.selected_vertices(self)
        } else {
            edges.iter().flat_map(|e| [e.0, e.1]).collect()
        };
        let direction = self
            .extrusion_direction(selection)
            .filter(|_| !vertices.is_empty())
            .ok_or("Выберите вершины или рёбра для экструдирования")?;
        let mut remap = BTreeMap::new();
        let mut motion = Vec::with_capacity(vertices.len());
        for v in vertices {
            let origin = self.positions[v];
            let new = self.positions.len();
            remap.insert(v, new);
            motion.push((new, origin, direction));
            self.positions.push(origin);
        }
        if selection.kind == Element::Vertex {
            self.loose_edges
                .extend(remap.iter().map(|(&a, &b)| Edge::new(a, b)));
            selection.vertices = remap.into_values().collect();
        } else {
            selection.edges.clear();
            for edge in edges {
                let (a, b, material, smooth) = adjacency[&edge]
                    .first()
                    .and_then(|&i| {
                        let face = &self.faces[i];
                        face.corners
                            .iter()
                            .zip(face.corners.iter().cycle().skip(1))
                            .find(|(a, b)| Edge::new(a.vertex, b.vertex) == edge)
                            .map(|(a, b)| (b.clone(), a.clone(), face.material, face.smooth))
                    })
                    .unwrap_or_else(|| (Corner::new(edge.0), Corner::new(edge.1), 0, false));
                let mut na = a.clone();
                let mut nb = b.clone();
                na.vertex = remap[&a.vertex];
                nb.vertex = remap[&b.vertex];
                selection.edges.insert(Edge::new(na.vertex, nb.vertex));
                self.loose_edges.remove(&edge);
                self.faces.push(Face {
                    corners: vec![a, b, nb, na],
                    material,
                    smooth,
                });
            }
        }
        Ok(motion)
    }
    pub fn inset(&mut self, selection: &mut Selection, distance: f32) -> Result<(), String> {
        let selected = selection.selected_faces(self);
        if selected.is_empty() {
            return Err("Выберите грани для inset".into());
        }
        let adjacency = self.edges();
        let mut shifts: BTreeMap<usize, (Vec3, usize)> = BTreeMap::new();
        for &i in &selected {
            let f = &self.faces[i];
            let n = self.normal(f).normalize_or_zero();
            for (a, b) in f.edges() {
                if adjacency[&Edge::new(a, b)]
                    .iter()
                    .filter(|f| selected.contains(f))
                    .count()
                    != 1
                {
                    continue;
                }
                let inward = n.cross((self.positions[b] - self.positions[a]).normalize_or_zero());
                for v in [a, b] {
                    let entry = shifts.entry(v).or_default();
                    entry.0 += inward;
                    entry.1 += 1;
                }
            }
        }
        if shifts.is_empty() {
            return Err("Inset требует область с граничными рёбрами".into());
        }
        let mut remap = BTreeMap::new();
        let vertices: BTreeSet<_> = selected
            .iter()
            .flat_map(|&i| self.faces[i].corners.iter().map(|c| c.vertex))
            .collect();
        for v in vertices {
            let shift = shifts.get(&v).map_or(Vec3::ZERO, |(n, count)| {
                n.normalize_or_zero() * distance * (*count as f32 / n.length().max(0.25))
            });
            remap.insert(v, self.positions.len());
            self.positions.push(self.positions[v] + shift);
        }
        let mut walls = Vec::new();
        for &i in &selected {
            let f = &self.faces[i];
            for (a, b) in f.corners.iter().zip(f.corners.iter().cycle().skip(1)) {
                if adjacency[&Edge::new(a.vertex, b.vertex)]
                    .iter()
                    .filter(|f| selected.contains(f))
                    .count()
                    != 1
                {
                    continue;
                }
                let mut na = a.clone();
                na.vertex = remap[&a.vertex];
                let mut nb = b.clone();
                nb.vertex = remap[&b.vertex];
                walls.push(Face {
                    corners: vec![a.clone(), b.clone(), nb, na],
                    material: f.material,
                    smooth: f.smooth,
                });
            }
        }
        for &i in &selected {
            for c in &mut self.faces[i].corners {
                c.vertex = remap[&c.vertex];
            }
        }
        self.faces.extend(walls);
        self.compact();
        self.select_faces(selection, selected);
        Ok(())
    }
    pub fn subdivide(&mut self, selection: &mut Selection) -> Result<(), String> {
        let selected = selection.selected_faces(self);
        if selected.is_empty() {
            return Err("Выберите грани для подразделения".into());
        }
        let original = self.faces.clone();
        let mut mid = BTreeMap::new();
        for &i in &selected {
            for (a, b) in original[i].edges() {
                mid.entry(Edge::new(a, b)).or_insert_with(|| {
                    self.positions
                        .push((self.positions[a] + self.positions[b]) * 0.5);
                    self.positions.len() - 1
                });
            }
        }
        let mut result = Vec::new();
        let mut chosen = Vec::new();
        for (i, face) in original.iter().enumerate() {
            if !selected.contains(&i) {
                let mut f = face.clone();
                f.corners.clear();
                for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
                    f.corners.push(a.clone());
                    if let Some(&v) = mid.get(&Edge::new(a.vertex, b.vertex)) {
                        f.corners.push(a.between(b, v, 0.5));
                    }
                }
                result.push(f);
                continue;
            }
            let corners = &face.corners;
            let edges: Vec<_> = corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .map(|(a, b)| a.between(b, mid[&Edge::new(a.vertex, b.vertex)], 0.5))
                .collect();
            let mut polygons = Vec::new();
            if corners.len() == 3 {
                for j in 0..3 {
                    polygons.push(vec![
                        corners[j].clone(),
                        edges[j].clone(),
                        edges[(j + 2) % 3].clone(),
                    ]);
                }
                polygons.push(edges);
            } else {
                let v = self.positions.len();
                self.positions.push(self.center(face));
                let count = corners.len() as f32;
                let center = Corner {
                    source_normal: None,
                    vertex: v,
                    uv: corners.iter().map(|c| c.uv).sum::<Vec2>() / count,
                    uv1: corners.iter().map(|c| c.uv1).sum::<Vec2>() / count,
                    color: corners.iter().map(|c| c.color).sum::<Vec4>() / count,
                };
                for j in 0..corners.len() {
                    polygons.push(vec![
                        corners[j].clone(),
                        edges[j].clone(),
                        center.clone(),
                        edges[(j + corners.len() - 1) % corners.len()].clone(),
                    ]);
                }
            }
            for corners in polygons {
                chosen.push(result.len());
                result.push(Face {
                    corners,
                    material: face.material,
                    smooth: face.smooth,
                });
            }
        }
        self.faces = result;
        self.select_faces(selection, chosen);
        Ok(())
    }
    pub fn triangulate(&mut self, selection: &mut Selection) {
        let selected = selection.selected_faces(self);
        let mut result = Vec::new();
        let mut chosen = Vec::new();
        for (i, f) in self.faces.iter().enumerate() {
            if selected.contains(&i) {
                for t in self.triangles(f) {
                    chosen.push(result.len());
                    result.push(Face {
                        corners: t.map(|j| f.corners[j].clone()).to_vec(),
                        material: f.material,
                        smooth: f.smooth,
                    });
                }
            } else {
                result.push(f.clone());
            }
        }
        self.faces = result;
        self.select_faces(selection, chosen);
    }
    pub fn dissolve(&mut self, selection: &mut Selection, quads_only: bool) -> Result<(), String> {
        let selected = selection.selected_faces(self);
        let adjacency = self.edges();
        let mut parent: Vec<_> = (0..self.faces.len()).collect();
        fn root(parent: &mut [usize], index: usize) -> usize {
            if parent[index] != index {
                parent[index] = root(parent, parent[index]);
            }
            parent[index]
        }
        let mut edges: Vec<_> = if selection.kind == Element::Edge {
            selection.edges.iter().copied().collect()
        } else {
            adjacency
                .iter()
                .filter(|(_, f)| f.len() == 2 && f.iter().all(|i| selected.contains(i)))
                .map(|(&e, _)| e)
                .collect()
        };
        let mut changed = false;
        while let Some(edge) = edges.pop() {
            let Some(faces) = adjacency.get(&edge).filter(|v| v.len() == 2) else {
                continue;
            };
            let (a, b) = (root(&mut parent, faces[0]), root(&mut parent, faces[1]));
            if a == b {
                continue;
            }
            let fa = &self.faces[a];
            let fb = &self.faces[b];
            let normal_a = self.normal(fa).normalize_or_zero();
            let normal_b = self.normal(fb).normalize_or_zero();
            // Flat GLB triangles on a bent quad have different face normals.
            // These are a consequence of triangulation, not an authored seam.
            let flat_normals =
                [(fa, normal_a), (fb, normal_b)]
                    .into_iter()
                    .all(|(face, normal)| {
                        !face.smooth
                            && face.corners.iter().all(|corner| {
                                corner
                                    .source_normal
                                    .is_none_or(|n| n.abs_diff_eq(normal, 1e-4))
                            })
                    });
            if fa.material != fb.material
                || [edge.0, edge.1].into_iter().any(|v| {
                    let a = fa.corners.iter().find(|c| c.vertex == v).unwrap();
                    let b = fb.corners.iter().find(|c| c.vertex == v).unwrap();
                    a.uv != b.uv || a.uv1 != b.uv1 || a.color != b.color
                })
                || (quads_only
                    && (fa.corners.len() != 3
                        || fb.corners.len() != 3
                        || fa.smooth != fb.smooth
                        || [edge.0, edge.1].into_iter().any(|v| {
                            let a = fa.corners.iter().find(|c| c.vertex == v).unwrap();
                            let b = fb.corners.iter().find(|c| c.vertex == v).unwrap();
                            match (a.source_normal, b.source_normal) {
                                (Some(a), Some(b)) => !flat_normals && !a.abs_diff_eq(b, 1e-5),
                                (None, None) => false,
                                _ => true,
                            }
                        })
                        || normal_a.dot(normal_b) < 40.0_f32.to_radians().cos()))
            {
                continue;
            }
            let border: Vec<_> = fa
                .corners
                .iter()
                .zip(fa.corners.iter().cycle().skip(1))
                .chain(fb.corners.iter().zip(fb.corners.iter().cycle().skip(1)))
                .filter(|(x, y)| Edge::new(x.vertex, y.vertex) != edge)
                .map(|(x, y)| (x.clone(), y.vertex))
                .collect();
            let mut polygon = Vec::new();
            let mut current = border[0].0.vertex;
            for _ in 0..border.len() {
                let Some((corner, next)) = border.iter().find(|(c, _)| c.vertex == current) else {
                    break;
                };
                polygon.push(corner.clone());
                current = *next;
            }
            if polygon.len() != border.len() || current != polygon[0].vertex {
                continue;
            }
            if quads_only {
                if polygon.len() != 4 {
                    continue;
                }
                let points: Vec<_> = polygon.iter().map(|c| self.positions[c.vertex]).collect();
                let normal = (normal_a + normal_b).normalize_or_zero();
                let scale = (0..4)
                    .map(|i| points[i].distance_squared(points[(i + 1) % 4]))
                    .fold(0.0_f32, f32::max);
                // Quads can be non-planar after editing their vertices. Keep
                // their projected boundary convex and retain sharp creases.
                if (0..4).any(|i| {
                    (points[(i + 1) % 4] - points[i])
                        .cross(points[(i + 2) % 4] - points[(i + 1) % 4])
                        .dot(normal)
                        <= scale * 1e-7
                }) {
                    continue;
                }
                // Ear clipping starts at corner zero. Keep the original hidden
                // diagonal so UVs, colors and normals interpolate identically.
                if Edge::new(polygon[1].vertex, polygon[3].vertex) != edge {
                    polygon.rotate_left(1);
                }
                if flat_normals && !normal_a.abs_diff_eq(normal_b, 1e-5) {
                    for corner in &mut polygon {
                        corner.source_normal = None;
                    }
                }
            }
            self.faces[a] = Face {
                corners: polygon,
                material: fa.material,
                smooth: fa.smooth || fb.smooth,
            };
            parent[b] = a;
            changed = true;
        }
        if !changed {
            return Err("Не найдены соседние совместимые грани".into());
        }
        self.faces = std::mem::take(&mut self.faces)
            .into_iter()
            .enumerate()
            .filter(|(i, _)| parent[*i] == *i)
            .map(|(_, f)| f)
            .collect();
        selection.all(self);
        Ok(())
    }
    pub fn delete(&mut self, selection: &mut Selection, faces_only: bool) {
        let vertices = selection.selected_vertices(self);
        let faces = selection.selected_faces(self);
        if !faces_only {
            match selection.kind {
                Element::Vertex => self
                    .loose_edges
                    .retain(|e| !vertices.contains(&e.0) && !vertices.contains(&e.1)),
                Element::Edge => self.loose_edges.retain(|e| !selection.edges.contains(e)),
                Element::Face => {}
            }
        }
        self.faces = self
            .faces
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                if faces_only || selection.kind == Element::Face {
                    !faces.contains(i)
                } else if selection.kind == Element::Edge {
                    !f.edges()
                        .any(|(a, b)| selection.edges.contains(&Edge::new(a, b)))
                } else {
                    !f.corners.iter().any(|c| vertices.contains(&c.vertex))
                }
            })
            .map(|(_, f)| f.clone())
            .collect();
        selection.clear();
        self.compact();
    }
    pub fn compact(&mut self) {
        self.compact_vertices();
    }
    fn compact_vertices(&mut self) -> HashMap<usize, usize> {
        let used: BTreeSet<_> = self
            .faces
            .iter()
            .flat_map(|f| f.corners.iter().map(|c| c.vertex))
            .chain(self.loose_edges.iter().flat_map(|e| [e.0, e.1]))
            .collect();
        let remap: HashMap<_, _> = used
            .iter()
            .enumerate()
            .map(|(new, &old)| (old, new))
            .collect();
        self.positions = used.iter().map(|&v| self.positions[v]).collect();
        self.loose_edges = self
            .loose_edges
            .iter()
            .map(|e| Edge::new(remap[&e.0], remap[&e.1]))
            .collect();
        for f in &mut self.faces {
            for c in &mut f.corners {
                c.vertex = remap[&c.vertex];
            }
        }
        remap
    }
    pub fn merge(
        &mut self,
        selection: &mut Selection,
        distance: Option<f32>,
    ) -> Result<(), String> {
        let selected = selection.selected_vertices(self);
        if selected.len() < 2 {
            return Err("Выберите минимум две вершины".into());
        }
        let mut remap: HashMap<usize, usize> = HashMap::new();
        if let Some(distance) = distance {
            let distance = distance.max(self.radius() * 1.0e-8);
            let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
            for &v in &selected {
                let p = self.positions[v];
                let q = (p / distance).floor().as_i64vec3();
                let mut found = None;
                'search: for x in -1..=1 {
                    for y in -1..=1 {
                        for z in -1..=1 {
                            if let Some(candidates) = cells.get(&(q.x + x, q.y + y, q.z + z)) {
                                for &other in candidates {
                                    if p.distance_squared(self.positions[other])
                                        <= distance * distance
                                    {
                                        found = Some(other);
                                        break 'search;
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(other) = found {
                    remap.insert(v, other);
                } else {
                    cells.entry((q.x, q.y, q.z)).or_default().push(v);
                }
            }
        } else {
            let center =
                selected.iter().map(|&v| self.positions[v]).sum::<Vec3>() / selected.len() as f32;
            let first = *selected.first().unwrap();
            self.positions[first] = center;
            for &v in &selected {
                remap.insert(v, first);
            }
        }
        if remap.is_empty() {
            return Err("Нет вершин в пределах указанного расстояния".into());
        }
        self.loose_edges = self
            .loose_edges
            .iter()
            .map(|e| {
                Edge::new(
                    *remap.get(&e.0).unwrap_or(&e.0),
                    *remap.get(&e.1).unwrap_or(&e.1),
                )
            })
            .filter(|e| e.0 != e.1)
            .collect();
        for f in &mut self.faces {
            for c in &mut f.corners {
                if let Some(&v) = remap.get(&c.vertex) {
                    c.vertex = v;
                }
            }
            let mut used = HashSet::new();
            f.corners.retain(|c| used.insert(c.vertex));
        }
        self.faces = self
            .faces
            .iter()
            .filter(|f| f.corners.len() >= 3 && self.normal(f).length_squared() > 1.0e-16)
            .cloned()
            .collect();
        self.compact();
        selection.all(self);
        Ok(())
    }
    pub fn fill(&mut self, selection: &mut Selection) -> Result<(), String> {
        let chosen = selection.selected_vertices(self);
        let adjacency = self.edges();
        let boundary: Vec<_> = self
            .faces
            .iter()
            .flat_map(Face::edges)
            .filter(|&(a, b)| {
                adjacency[&Edge::new(a, b)].len() == 1
                    && (chosen.is_empty() || (chosen.contains(&a) && chosen.contains(&b)))
            })
            .collect();
        let mut remaining: BTreeSet<_> = boundary.into_iter().map(|(a, b)| (b, a)).collect();
        let mut added = Vec::new();
        while let Some(&(start, next)) = remaining.first() {
            remaining.remove(&(start, next));
            let mut polygon = vec![start];
            let mut current = next;
            while current != start && polygon.len() < 4096 {
                polygon.push(current);
                let Some(&(_, next)) = remaining.iter().find(|(a, _)| *a == current) else {
                    break;
                };
                remaining.remove(&(current, next));
                current = next;
            }
            if current == start && polygon.len() >= 3 {
                added.push(self.faces.len());
                self.faces.push(Face {
                    corners: polygon.into_iter().map(Corner::new).collect(),
                    material: 0,
                    smooth: false,
                });
            }
        }
        if added.is_empty() && chosen.len() >= 3 {
            let center =
                chosen.iter().map(|&v| self.positions[v]).sum::<Vec3>() / chosen.len() as f32;
            let mut normal = Vec3::ZERO;
            for f in &self.faces {
                if f.corners.iter().any(|c| chosen.contains(&c.vertex)) {
                    normal += self.normal(f);
                }
            }
            if normal.length_squared() < 1.0e-12 {
                let p: Vec<_> = chosen.iter().map(|&v| self.positions[v]).collect();
                normal = (p[1] - p[0]).cross(p[2] - p[0]);
            }
            let normal = normal.normalize_or_zero();
            if normal == Vec3::ZERO {
                return Err("Вершины лежат на одной прямой".into());
            }
            let x = normal.any_orthonormal_vector();
            let y = normal.cross(x);
            let mut vertices: Vec<_> = chosen.into_iter().collect();
            vertices.sort_by(|&a, &b| {
                let a = self.positions[a] - center;
                let b = self.positions[b] - center;
                a.dot(y)
                    .atan2(a.dot(x))
                    .total_cmp(&b.dot(y).atan2(b.dot(x)))
            });
            added.push(self.faces.len());
            self.faces.push(Face {
                corners: vertices.into_iter().map(Corner::new).collect(),
                material: 0,
                smooth: false,
            });
        }
        if added.is_empty() {
            return Err("Выберите замкнутую границу или минимум три вершины".into());
        }
        self.select_faces(selection, added);
        Ok(())
    }
    pub fn recalculate_normals(&mut self, selected: &Selection, flip: bool) {
        self.discard_normals();
        let chosen = selected.selected_faces(self);
        if flip {
            for &i in &chosen {
                self.faces[i].corners.reverse();
            }
            return;
        }
        let edges = self.edges();
        let mut visited = HashSet::new();
        for start in 0..self.faces.len() {
            if !visited.insert(start) {
                continue;
            }
            let mut queue = vec![start];
            let mut component = vec![start];
            while let Some(i) = queue.pop() {
                let own: Vec<_> = self.faces[i].edges().collect();
                for (a, b) in own {
                    for &j in &edges[&Edge::new(a, b)] {
                        if !visited.insert(j) {
                            continue;
                        }
                        if self.faces[j].edges().any(|(x, y)| x == a && y == b) {
                            self.faces[j].corners.reverse();
                        }
                        queue.push(j);
                        component.push(j);
                    }
                }
            }
            let mut volume = 0.0;
            for &i in &component {
                let f = &self.faces[i];
                for t in self.triangles(f) {
                    let a = self.positions[f.corners[t[0]].vertex];
                    let b = self.positions[f.corners[t[1]].vertex];
                    let c = self.positions[f.corners[t[2]].vertex];
                    volume += a.dot(b.cross(c));
                }
            }
            if volume < 0.0 {
                for i in component {
                    self.faces[i].corners.reverse();
                }
            }
        }
    }
    pub fn smooth_vertices(&mut self, selection: &Selection, strength: f32) {
        let chosen = selection.selected_vertices(self);
        let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); self.positions.len()];
        for e in self.edges().keys() {
            neighbors[e.0].push(e.1);
            neighbors[e.1].push(e.0);
        }
        let old = self.positions.clone();
        for v in chosen {
            if !neighbors[v].is_empty() {
                let center =
                    neighbors[v].iter().map(|&i| old[i]).sum::<Vec3>() / neighbors[v].len() as f32;
                self.positions[v] = old[v].lerp(center, strength.clamp(-1.0, 1.0));
            }
        }
    }
    pub fn project_uv(&mut self, selection: &Selection, axis: usize, spherical: bool) {
        let chosen = selection.selected_faces(self);
        let radius = self.radius();
        for i in chosen {
            let normal = self.normal(&self.faces[i]).abs();
            let axis = if axis > 2 {
                if normal.x > normal.y && normal.x > normal.z {
                    0
                } else if normal.y > normal.z {
                    1
                } else {
                    2
                }
            } else {
                axis
            };
            for c in &mut self.faces[i].corners {
                let p = self.positions[c.vertex];
                c.uv = if spherical {
                    let p = p.normalize_or_zero();
                    Vec2::new(
                        p.z.atan2(p.x) / std::f32::consts::TAU + 0.5,
                        p.y.clamp(-1.0, 1.0).asin() / std::f32::consts::PI + 0.5,
                    )
                } else {
                    (match axis {
                        0 => p.zy(),
                        1 => p.xz(),
                        _ => p.xy(),
                    }) / radius
                        + Vec2::splat(0.5)
                };
            }
        }
    }
    pub fn bisect(&mut self, normal: Vec3, offset: f32) -> Result<(), String> {
        let normal = normal.normalize_or_zero();
        if normal == Vec3::ZERO {
            return Err("Недопустимая плоскость разреза".into());
        }
        let epsilon = self.radius() * 1.0e-6;
        let original = self.faces.clone();
        let mut intersections = BTreeMap::new();
        let mut result = Vec::new();
        let mut cuts = 0;
        for face in original {
            let distances: Vec<_> = face
                .corners
                .iter()
                .map(|c| normal.dot(self.positions[c.vertex]) - offset)
                .collect();
            if !distances.iter().any(|d| *d > epsilon) || !distances.iter().any(|d| *d < -epsilon) {
                result.push(face);
                continue;
            }
            cuts += 1;
            for positive in [true, false] {
                let mut corners = Vec::new();
                for i in 0..face.corners.len() {
                    let a = &face.corners[i];
                    let b = &face.corners[(i + 1) % face.corners.len()];
                    let da = distances[i];
                    let db = distances[(i + 1) % distances.len()];
                    if if positive {
                        da >= -epsilon
                    } else {
                        da <= epsilon
                    } {
                        corners.push(a.clone());
                    }
                    if (da > epsilon && db < -epsilon) || (da < -epsilon && db > epsilon) {
                        let t = da / (da - db);
                        let v = *intersections
                            .entry(Edge::new(a.vertex, b.vertex))
                            .or_insert_with(|| {
                                let v = self.positions.len();
                                self.positions.push(
                                    self.positions[a.vertex].lerp(self.positions[b.vertex], t),
                                );
                                v
                            });
                        corners.push(a.between(b, v, t));
                    }
                }
                if corners.len() >= 3 {
                    result.push(Face {
                        corners,
                        material: face.material,
                        smooth: face.smooth,
                    });
                }
            }
        }
        if cuts == 0 {
            return Err("Плоскость не пересекает геометрию".into());
        }
        self.faces = result;
        self.discard_normals();
        Ok(())
    }
    pub fn loop_cut(&mut self, edge: Edge) -> Result<(), String> {
        let adjacency = self.edges();
        let mut queue = vec![edge];
        let mut split = BTreeMap::new();
        let mut cut_edges = BTreeSet::new();
        while let Some(edge) = queue.pop() {
            cut_edges.insert(edge);
            if let Some(faces) = adjacency.get(&edge) {
                for &i in faces {
                    let f = &self.faces[i];
                    if f.corners.len() != 4 || split.contains_key(&i) {
                        continue;
                    }
                    let j = f
                        .edges()
                        .position(|(a, b)| Edge::new(a, b) == edge)
                        .unwrap();
                    let opposite =
                        Edge::new(f.corners[(j + 2) % 4].vertex, f.corners[(j + 3) % 4].vertex);
                    split.insert(i, j);
                    queue.push(opposite);
                }
            }
        }
        if split.is_empty() {
            return Err(
                "Разрез петли проходит по четырёхугольным граням; Alt+J объединяет треугольники"
                    .into(),
            );
        }
        let mut points = BTreeMap::new();
        for e in cut_edges {
            let v = self.positions.len();
            self.positions
                .push((self.positions[e.0] + self.positions[e.1]) * 0.5);
            points.insert(e, v);
        }
        let mut result = Vec::new();
        for (i, f) in self.faces.iter().enumerate() {
            if let Some(&j) = split.get(&i) {
                let c: Vec<_> = (0..4).map(|k| f.corners[(j + k) % 4].clone()).collect();
                let a = c[0].between(&c[1], points[&Edge::new(c[0].vertex, c[1].vertex)], 0.5);
                let b = c[2].between(&c[3], points[&Edge::new(c[2].vertex, c[3].vertex)], 0.5);
                for corners in [
                    vec![c[0].clone(), a.clone(), b.clone(), c[3].clone()],
                    vec![a, c[1].clone(), c[2].clone(), b],
                ] {
                    result.push(Face {
                        corners,
                        material: f.material,
                        smooth: f.smooth,
                    });
                }
            } else {
                let mut corners = Vec::new();
                for (a, b) in f.corners.iter().zip(f.corners.iter().cycle().skip(1)) {
                    corners.push(a.clone());
                    if let Some(&v) = points.get(&Edge::new(a.vertex, b.vertex)) {
                        corners.push(a.between(b, v, 0.5));
                    }
                }
                result.push(Face {
                    corners,
                    material: f.material,
                    smooth: f.smooth,
                });
            }
        }
        self.faces = result;
        Ok(())
    }
    pub fn bevel_vertices(&mut self, selection: &mut Selection, width: f32) -> Result<(), String> {
        let chosen = selection.selected_vertices(self);
        if chosen.is_empty() {
            return Err("Выберите вершины для фаски".into());
        }
        let edges = self.edges();
        let mut points = BTreeMap::new();
        for e in edges.keys() {
            for (a, b) in [(e.0, e.1), (e.1, e.0)] {
                if chosen.contains(&a) {
                    let delta = self.positions[b] - self.positions[a];
                    let amount = width.max(0.0).min(delta.length() * 0.45);
                    let v = self.positions.len();
                    self.positions
                        .push(self.positions[a] + delta.normalize_or_zero() * amount);
                    points.insert((a, b), v);
                }
            }
        }
        let original = self.faces.clone();
        let mut caps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut result = Vec::new();
        for f in original {
            let mut corners = Vec::new();
            for i in 0..f.corners.len() {
                let c = &f.corners[i];
                if chosen.contains(&c.vertex) {
                    let p = &f.corners[(i + f.corners.len() - 1) % f.corners.len()];
                    let n = &f.corners[(i + 1) % f.corners.len()];
                    let a = points[&(c.vertex, p.vertex)];
                    let b = points[&(c.vertex, n.vertex)];
                    let ta = (self.positions[a] - self.positions[c.vertex]).length()
                        / self.positions[p.vertex]
                            .distance(self.positions[c.vertex])
                            .max(1.0e-12);
                    let tb = (self.positions[b] - self.positions[c.vertex]).length()
                        / self.positions[n.vertex]
                            .distance(self.positions[c.vertex])
                            .max(1.0e-12);
                    corners.push(c.between(p, a, ta));
                    corners.push(c.between(n, b, tb));
                    caps.entry(c.vertex).or_default().extend([a, b]);
                } else {
                    corners.push(c.clone());
                }
            }
            result.push(Face {
                corners,
                material: f.material,
                smooth: f.smooth,
            });
        }
        for (v, vertices) in caps {
            let mut vertices: BTreeSet<_> = vertices.into_iter().collect();
            if vertices.len() < 3 {
                continue;
            }
            let normal = self
                .faces
                .iter()
                .filter(|f| f.corners.iter().any(|c| c.vertex == v))
                .map(|f| self.normal(f))
                .sum::<Vec3>()
                .normalize_or_zero();
            if normal == Vec3::ZERO {
                return Err("Фаска требует согласованных нормалей вокруг вершины".into());
            }
            let x = normal.any_orthonormal_vector();
            let y = normal.cross(x);
            let center = self.positions[v];
            let mut sorted: Vec<_> = std::mem::take(&mut vertices).into_iter().collect();
            sorted.sort_by(|&a, &b| {
                let a = self.positions[a] - center;
                let b = self.positions[b] - center;
                a.dot(y)
                    .atan2(a.dot(x))
                    .total_cmp(&b.dot(y).atan2(b.dot(x)))
            });
            result.push(Face {
                corners: sorted.into_iter().map(Corner::new).collect(),
                material: 0,
                smooth: false,
            });
        }
        self.faces = result;
        self.compact();
        selection.all(self);
        Ok(())
    }
    pub fn buffers(&self) -> BTreeMap<usize, Buffers> {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for f in &self.faces {
            if f.smooth {
                let n = self.normal(f);
                for c in &f.corners {
                    normals[c.vertex] += n;
                }
            }
        }
        let mut output: BTreeMap<usize, Buffers> = BTreeMap::new();
        let mut maps: BTreeMap<usize, HashMap<[u32; 12], u32>> = BTreeMap::new();
        for f in &self.faces {
            let normal = self.normal(f).normalize_or_zero();
            let mut polygon = vec![0; f.corners.len()];
            for triangle in self.triangles(f) {
                for i in triangle {
                    let c = &f.corners[i];
                    let n = c.source_normal.unwrap_or_else(|| {
                        if f.smooth {
                            normals[c.vertex].normalize_or_zero()
                        } else {
                            normal
                        }
                    });
                    let key = [
                        c.vertex as u32,
                        n.x.to_bits(),
                        n.y.to_bits(),
                        n.z.to_bits(),
                        c.uv.x.to_bits(),
                        c.uv.y.to_bits(),
                        c.uv1.x.to_bits(),
                        c.uv1.y.to_bits(),
                        c.color.x.to_bits(),
                        c.color.y.to_bits(),
                        c.color.z.to_bits(),
                        c.color.w.to_bits(),
                    ];
                    let buffers = output.entry(f.material).or_default();
                    let index = *maps
                        .entry(f.material)
                        .or_default()
                        .entry(key)
                        .or_insert_with(|| {
                            let index = buffers.positions.len() as u32;
                            buffers.positions.push(self.positions[c.vertex].to_array());
                            buffers.normals.push(n.to_array());
                            buffers.uv.push(c.uv.to_array());
                            buffers.uv1.push(c.uv1.to_array());
                            buffers.colors.push(c.color.to_array());
                            index
                        });
                    buffers.indices.push(index);
                    polygon[i] = index;
                }
            }
            output.entry(f.material).or_default().polygons.push(polygon);
        }
        output
    }
}

#[cfg(test)]
#[path = "model_mesh_tests.rs"]
mod tests;
