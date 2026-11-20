use super::*;

#[derive(Clone, Debug)]
pub(in crate::editor) struct LoopCut {
    // Orient each crossed edge toward the same side of the strip.
    rails: BTreeMap<Edge, (usize, usize)>,
    faces: BTreeMap<usize, usize>,
    seed: Edge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) struct LoopCutOptions {
    pub count: usize,
    pub position: f32,
    pub even: bool,
    pub flipped: bool,
    pub clamp: bool,
    pub correct_uv: bool,
    pub smoothness: f32,
    pub smooth_falloff: bool,
}

impl Default for LoopCutOptions {
    fn default() -> Self {
        Self {
            count: 1,
            position: 0.5,
            even: false,
            flipped: false,
            clamp: true,
            correct_uv: true,
            smoothness: 0.0,
            smooth_falloff: true,
        }
    }
}

impl LoopCut {
    pub fn find(mesh: &Geometry, seed: Edge) -> Result<Self, String> {
        let adjacency = mesh.edges();
        Self::find_in(mesh, &adjacency, seed)
    }

    pub fn find_in(
        mesh: &Geometry,
        adjacency: &BTreeMap<Edge, Vec<usize>>,
        seed: Edge,
    ) -> Result<Self, String> {
        let mut rails = BTreeMap::new();
        let mut faces = BTreeMap::new();
        let mut pending = vec![(seed.0, seed.1)];
        while let Some((a, b)) = pending.pop() {
            let edge = Edge::new(a, b);
            if let Some(&direction) = rails.get(&edge) {
                if direction != (a, b) {
                    return Err("Полоса четырёхугольников перекручена".into());
                }
                continue;
            }
            let incident = adjacency.get(&edge).ok_or("Выберите ребро для разреза")?;
            if incident.len() > 2 {
                return Err("У ребра полосы больше двух смежных граней".into());
            }
            rails.insert(edge, (a, b));
            for &face_index in incident {
                let face = &mesh.faces[face_index];
                if face.corners.len() != 4 {
                    continue;
                }
                let start = face
                    .edges()
                    .position(|(u, v)| Edge::new(u, v) == edge)
                    .ok_or("Повреждённая полоса граней")?;
                if let Some(&existing) = faces.get(&face_index) {
                    if (start + 4 - existing) % 2 != 0 {
                        return Err("Полоса пересекает саму себя".into());
                    }
                    continue;
                }
                faces.insert(face_index, start);
                let opposite = (
                    face.corners[(start + 3) % 4].vertex,
                    face.corners[(start + 2) % 4].vertex,
                );
                pending.push(if face.corners[start].vertex == a {
                    opposite
                } else {
                    (opposite.1, opposite.0)
                });
            }
        }
        if faces.is_empty() {
            return Err("Разрез петли проходит по четырёхугольникам".into());
        }
        Ok(Self { rails, faces, seed })
    }

    pub fn seed(&self) -> Edge {
        self.seed
    }

    fn fraction(&self, mesh: &Geometry, edge: Edge, options: LoopCutOptions, cut: usize) -> f32 {
        if options.count > 1 {
            return (cut + 1) as f32 / (options.count + 1) as f32;
        }
        let factor = if options.even {
            let length = |e: Edge| mesh.positions[e.0].distance(mesh.positions[e.1]);
            let ratio = length(self.seed) / length(edge).max(f32::EPSILON);
            if options.flipped {
                1.0 - (1.0 - options.position) * ratio
            } else {
                options.position * ratio
            }
        } else {
            options.position
        };
        if options.clamp {
            factor.clamp(0.001, 0.999)
        } else {
            factor
        }
    }

    fn points(&self, mesh: &Geometry, options: LoopCutOptions) -> BTreeMap<Edge, Vec<(f32, Vec3)>> {
        // Average incident normals for optional curvature-preserving offset.
        let mut normals = vec![Vec3::ZERO; mesh.positions.len()];
        if options.smoothness != 0.0 {
            for face in &mesh.faces {
                let normal = mesh.normal(face);
                for corner in &face.corners {
                    normals[corner.vertex] += normal;
                }
            }
            for normal in &mut normals {
                *normal = normal.normalize_or_zero();
            }
        }
        self.rails
            .iter()
            .map(|(&edge, &(a, b))| {
                let start = mesh.positions[a];
                let end = mesh.positions[b];
                let direction = (end - start).normalize_or_zero();
                let bend = (normals[b] - normals[a]).dot(direction) * start.distance(end) * 0.25;
                let points = (0..options.count)
                    .map(|i| {
                        let t = self.fraction(mesh, edge, options, i);
                        let falloff = if options.smooth_falloff {
                            (4.0 * t * (1.0 - t)).max(0.0)
                        } else {
                            1.0
                        };
                        let position = start.lerp(end, t)
                            + normals[a].lerp(normals[b], t).normalize_or_zero()
                                * (bend * falloff * options.smoothness);
                        (t, position)
                    })
                    .collect();
                (edge, points)
            })
            .collect()
    }

    pub fn preview(&self, mesh: &Geometry, options: LoopCutOptions) -> Vec<[Vec3; 2]> {
        if !self.fits_vertex_limit(mesh, options) {
            return Vec::new();
        }
        let points = self.points(mesh, options);
        self.faces
            .iter()
            .flat_map(|(&fi, &start)| {
                let face = &mesh.faces[fi];
                let rail = |i: usize| {
                    let a: &Corner = &face.corners[i];
                    let b = &face.corners[(i + 1) % 4];
                    &points[&Edge::new(a.vertex, b.vertex)]
                };
                rail(start)
                    .iter()
                    .zip(rail((start + 2) % 4))
                    .map(|(a, b)| [a.1, b.1])
            })
            .collect()
    }

    pub fn apply(
        &self,
        mesh: &mut Geometry,
        options: LoopCutOptions,
    ) -> Result<BTreeSet<Edge>, String> {
        if !(1..=100).contains(&options.count)
            || !options.position.is_finite()
            || !options.smoothness.is_finite()
        {
            return Err("Недопустимые настройки разреза петли".into());
        }
        if !self.fits_vertex_limit(mesh, options) {
            return Err("Разрез превысит лимит модели в 250 000 вершин".into());
        }
        let points = self.points(mesh, options);
        let mut result = mesh.clone();
        result.faces.clear();
        let mut split_points = BTreeMap::<Edge, Vec<(usize, f32)>>::new();
        for (edge, points) in points {
            let mut vertices = Vec::with_capacity(points.len());
            for (t, position) in points {
                vertices.push((result.positions.len(), t));
                result.positions.push(position);
            }
            if result.uv_seams.remove(&edge) {
                let (a, b) = self.rails[&edge];
                let mut from = a;
                for to in vertices.iter().map(|&(v, _)| v).chain([b]) {
                    result.uv_seams.insert(Edge::new(from, to));
                    from = to;
                }
            }
            split_points.insert(edge, vertices);
        }
        let corners_on = |a: &Corner, b: &Corner| {
            let edge = Edge::new(a.vertex, b.vertex);
            let Some(points) = split_points.get(&edge) else {
                return Vec::new();
            };
            let reversed = self.rails[&edge].0 != a.vertex;
            let len = points.len();
            (0..len)
                .map(|i| {
                    let (vertex, t) = points[if reversed { len - 1 - i } else { i }];
                    let fraction = if reversed { 1.0 - t } else { t };
                    let mut corner = a.between(b, vertex, fraction);
                    if !options.correct_uv {
                        let t = (i + 1) as f32 / (len + 1) as f32;
                        corner.uv = a.uv.lerp(b.uv, t);
                        corner.uv1 = a.uv1.lerp(b.uv1, t);
                    }
                    corner.source_normal = a
                        .source_normal
                        .zip(b.source_normal)
                        .map(|(a, b)| a.lerp(b, fraction).normalize_or_zero());
                    corner
                })
                .collect::<Vec<_>>()
        };
        let mut new_edges = BTreeSet::new();
        for (fi, face) in mesh.faces.iter().enumerate() {
            let mut push = |corners| {
                result.faces.push(Face {
                    corners,
                    material: face.material,
                    smooth: face.smooth,
                })
            };
            if let Some(&i) = self.faces.get(&fi) {
                let [a, b, c, d] = std::array::from_fn(|j| &face.corners[(i + j) % 4]);
                let left = corners_on(a, b);
                let right = corners_on(d, c);
                let mut previous = (a.clone(), d.clone());
                for (a, b) in left.into_iter().zip(right) {
                    new_edges.insert(Edge::new(a.vertex, b.vertex));
                    push(vec![previous.0, a.clone(), b.clone(), previous.1]);
                    previous = (a, b);
                }
                push(vec![previous.0, b.clone(), c.clone(), previous.1]);
            } else {
                let mut corners = Vec::new();
                for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
                    corners.push(a.clone());
                    corners.extend(corners_on(a, b));
                }
                push(corners);
            }
        }
        result.validate()?;
        *mesh = result;
        Ok(new_edges)
    }

    pub fn fits_vertex_limit(&self, mesh: &Geometry, options: LoopCutOptions) -> bool {
        mesh.positions
            .len()
            .saturating_add(self.rails.len().saturating_mul(options.count))
            <= 250_000
    }
}
