use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum WidthType {
    #[default]
    Offset,
    Width,
    Depth,
    Percent,
    Absolute,
}

impl WidthType {
    pub fn next(self) -> Self {
        match self {
            Self::Offset => Self::Width,
            Self::Width => Self::Depth,
            Self::Depth => Self::Percent,
            Self::Percent => Self::Absolute,
            Self::Absolute => Self::Offset,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Offset => "Отступ",
            Self::Width => "Ширина",
            Self::Depth => "Глубина",
            Self::Percent => "Процент",
            Self::Absolute => "Вдоль рёбер",
        }
    }

    fn fraction(self, amount: f32, length: f32, sine: f32, normals_dot: f32) -> f32 {
        match self {
            Self::Offset => amount / (length * sine),
            Self::Width => amount / (2.0 * ((1.0 + normals_dot) * 0.5).sqrt() * length * sine),
            Self::Depth => amount / (((1.0 - normals_dot) * 0.5).sqrt() * length * sine),
            Self::Percent => amount / 100.0,
            Self::Absolute => amount / length,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Miter {
    #[default]
    Sharp,
    Patch,
    Arc,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum Intersection {
    #[default]
    Grid,
    Cutoff,
}

impl Miter {
    pub fn next_outer(self) -> Self {
        match self {
            Self::Sharp => Self::Patch,
            Self::Patch => Self::Arc,
            Self::Arc => Self::Sharp,
        }
    }

    pub fn next_inner(self) -> Self {
        if self == Self::Sharp {
            Self::Arc
        } else {
            Self::Sharp
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sharp => "острый",
            Self::Patch => "со вставкой",
            Self::Arc => "дугой",
        }
    }
}

impl Intersection {
    pub fn next(self) -> Self {
        match self {
            Self::Grid => Self::Cutoff,
            Self::Cutoff => Self::Grid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) struct BevelOptions {
    pub amount: f32,
    pub width_type: WidthType,
    pub segments: usize,
    pub profile: f32,
    pub custom_profile: Option<[Vec2; 3]>,
    pub clamp_overlap: bool,
    pub loop_slide: bool,
    pub outer_miter: Miter,
    pub inner_miter: Miter,
    pub spread: f32,
    pub intersection: Intersection,
    pub harden_normals: bool,
    pub preserve_uv_seams: bool,
    pub preserve_sharp: bool,
    pub material: Option<usize>,
}

impl Default for BevelOptions {
    fn default() -> Self {
        Self {
            amount: 0.0,
            width_type: WidthType::Offset,
            segments: 1,
            profile: 0.5,
            custom_profile: None,
            clamp_overlap: true,
            loop_slide: true,
            outer_miter: Miter::Sharp,
            inner_miter: Miter::Sharp,
            spread: 0.1,
            intersection: Intersection::Grid,
            harden_normals: false,
            preserve_uv_seams: false,
            preserve_sharp: false,
            material: None,
        }
    }
}

impl BevelOptions {
    fn valid_profile(self) -> bool {
        if !(1..=100).contains(&self.segments) || !(0.0..=1.0).contains(&self.profile) {
            return false;
        }
        self.custom_profile.is_none_or(|points| {
            let points = [Vec2::X, points[0], points[1], points[2], Vec2::Y];
            points
                .windows(2)
                .all(|pair| pair[0].x > pair[1].x && pair[0].y <= pair[1].y)
        })
    }

    fn flat_profile(self) -> bool {
        self.custom_profile.map_or(self.profile == 0.25, |points| {
            points
                .iter()
                .all(|point| (point.x + point.y - 1.0).abs() < 1.0e-6)
        })
    }

    fn profile_weights(self, t: f32) -> Vec2 {
        let Some(points) = self.custom_profile else {
            let (x, y) = profile_weights(self.profile, t);
            return Vec2::new(x, y);
        };
        let x = 1.0 - t;
        let points = [Vec2::X, points[0], points[1], points[2], Vec2::Y];
        let pair = points.windows(2).find(|pair| x >= pair[1].x).unwrap();
        let factor = (pair[0].x - x) / (pair[0].x - pair[1].x);
        Vec2::new(x, pair[0].y.lerp(pair[1].y, factor))
    }
}

#[derive(Clone, Copy)]
struct Offset {
    along_prev: f32,
    along_next: f32,
    miter: Miter,
}

// Leave a narrow strip until we can verify that stitching the contact is safe.
const AVAILABLE_EDGE: f32 = 0.9999;

fn miter_extension(spread: f32, edge_length: f32) -> f32 {
    (spread / edge_length).min(0.25)
}

#[derive(Default)]
struct RailOffset {
    shortest: Option<f32>,
    weighted: f32,
    total_weight: f32,
}

impl RailOffset {
    fn add(&mut self, fraction: f32, sine: f32) {
        self.shortest = Some(self.shortest.map_or(fraction, |old| old.min(fraction)));
        // Moving along the shared rail changes the perpendicular offset by
        // sin(angle) on each face. Minimize the squared offset errors.
        let weight = sine * sine;
        self.weighted += fraction * weight;
        self.total_weight += weight;
    }

    fn fraction(&self, loop_slide: bool) -> f32 {
        if loop_slide {
            self.shortest.unwrap()
        } else {
            self.weighted / self.total_weight
        }
    }
}

impl Geometry {
    pub(in crate::editor) fn bevel_vertices_with(
        &mut self,
        selected: &BTreeSet<usize>,
        options: BevelOptions,
    ) -> Result<BTreeSet<usize>, String> {
        const MANIFOLD: &str = "Фаска вершин требует согласованной сетки вокруг выделения";
        if selected.is_empty() || !options.amount.is_finite() || options.amount <= 0.0 {
            return Err("Выберите вершины и задайте положительную ширину фаски".into());
        }
        if !options.valid_profile() {
            return Err("Недопустимые параметры профиля фаски".into());
        }
        let adjacency = self.edges();
        let mut fractions = BTreeMap::<(usize, usize), RailOffset>::new();
        for face in &self.faces {
            for i in 0..face.corners.len() {
                let (prev, corner, next) = neighbors(face, i);
                let anchor = corner.vertex;
                if !selected.contains(&anchor) {
                    continue;
                }
                let origin = self.positions[anchor];
                let u = self.positions[prev.vertex] - origin;
                let v = self.positions[next.vertex] - origin;
                let cosine = u
                    .normalize_or_zero()
                    .dot(v.normalize_or_zero())
                    .clamp(-1.0, 1.0);
                for (other, length) in [(prev.vertex, u.length()), (next.vertex, v.length())] {
                    let tangent = match options.width_type {
                        WidthType::Width => (2.0 * (1.0 - cosine)).sqrt().recip(),
                        WidthType::Depth => ((1.0 + cosine) * 0.5).sqrt().recip(),
                        _ => 1.0,
                    };
                    let fraction = if options.width_type == WidthType::Percent {
                        options.amount / 100.0
                    } else {
                        options.amount * tangent / length
                    };
                    if !fraction.is_finite() {
                        return Err(
                            "Этот способ измерения ширины неприменим к выбранной вершине".into(),
                        );
                    }
                    fractions
                        .entry((anchor, other))
                        .or_default()
                        .add(fraction, 1.0);
                }
            }
        }
        for edge in &self.loose_edges {
            for (a, b) in [(edge.0, edge.1), (edge.1, edge.0)] {
                if selected.contains(&a) {
                    let fraction = if options.width_type == WidthType::Percent {
                        options.amount / 100.0
                    } else {
                        options.amount / self.positions[a].distance(self.positions[b])
                    };
                    fractions.entry((a, b)).or_default().add(fraction, 1.0);
                }
            }
        }
        if fractions.is_empty() {
            return Err("Выберите вершины, соединённые рёбрами".into());
        }
        let boundary: BTreeSet<_> = adjacency
            .iter()
            .filter(|(_, faces)| faces.len() != 2)
            .flat_map(|(edge, _)| [edge.0, edge.1])
            .collect();
        let mut positions = self.positions.clone();
        let mut anchors: Vec<_> = (0..positions.len()).collect();
        let limit = if options.clamp_overlap {
            fractions
                .values()
                .map(|offset| offset.fraction(true))
                .fold(0.45_f32, f32::max)
                .recip()
                * 0.45
        } else {
            1.0
        };
        let rails: BTreeMap<_, _> = fractions
            .into_iter()
            .map(|((a, b), offset)| {
                let t = offset.fraction(true) * limit;
                let index = positions.len();
                positions.push(self.positions[a].lerp(self.positions[b], t));
                anchors.push(a);
                ((a, b), (index, t))
            })
            .collect();
        let end = |a, b| rails.get(&(a, b)).map_or(a, |point| point.0);
        let mut affected = BTreeSet::new();
        let mut faces = Vec::with_capacity(self.faces.len());
        for (fi, face) in self.faces.iter().enumerate() {
            let mut corners = Vec::new();
            for i in 0..face.corners.len() {
                let (prev, c, next) = neighbors(face, i);
                let anchor = c.vertex;
                if !selected.contains(&anchor) {
                    corners.push(c.clone());
                    continue;
                }
                affected.insert(fi);
                let (a, ta) = rails[&(anchor, prev.vertex)];
                let (b, tb) = rails[&(anchor, next.vertex)];
                let mut left = c.between(prev, a, ta);
                let mut right = c.between(next, b, tb);
                if options.harden_normals || options.preserve_sharp {
                    left.source_normal = c.source_normal;
                    right.source_normal = c.source_normal;
                }
                corners.push(left.clone());
                for step in 1..options.segments {
                    let t = step as f32 / options.segments as f32;
                    let weights = options.profile_weights(t);
                    let origin = self.positions[anchor];
                    let point = origin
                        + (positions[a] - origin) * weights.x
                        + (positions[b] - origin) * weights.y;
                    let vertex = positions.len();
                    positions.push(point);
                    anchors.push(anchor);
                    let mut corner = left.between(&right, vertex, t);
                    corner.source_normal = left.source_normal;
                    corners.push(corner);
                }
                corners.push(right);
            }
            faces.push(Face {
                corners,
                material: face.material,
                smooth: face.smooth,
            });
        }
        let cap_start = faces.len();
        for mut cap in cap_boundaries(&faces, &anchors, &boundary, MANIFOLD)? {
            if let Some(material) = options.material {
                cap.material = material;
            }
            // The cap uses the original corner attributes on its UV border,
            // with fresh normals so its shading follows the new surface.
            for corner in &mut cap.corners {
                corner.source_normal = None;
            }
            if options.segments > 1 && !options.flat_profile() {
                grid_junction(&mut faces, &mut positions, cap, !options.preserve_sharp);
            } else {
                faces.push(cap);
            }
        }
        let mut uv_seams = self.uv_seams.clone();
        if options.preserve_uv_seams {
            uv_seams.extend(
                self.uv_seams
                    .iter()
                    .map(|edge| Edge::new(end(edge.0, edge.1), end(edge.1, edge.0))),
            );
        }
        let loose_edges = self
            .loose_edges
            .iter()
            .map(|edge| Edge::new(end(edge.0, edge.1), end(edge.1, edge.0)))
            .collect();
        let mut result = Geometry {
            positions,
            faces,
            loose_edges,
            uv_seams,
        };
        result.compact();
        result.prune_uv_seams();
        result.validate()?;
        if cap_start < result.faces.len() {
            affected = (cap_start..result.faces.len()).collect();
        }
        *self = result;
        Ok(affected)
    }

    /// Trim the incident faces, bridge both sides of each selected edge,
    /// then close any gaps at its endpoints.
    pub(in crate::editor) fn bevel_edges(
        &mut self,
        selected: &BTreeSet<Edge>,
        width: f32,
    ) -> Result<BTreeSet<usize>, String> {
        self.bevel_edges_with(
            selected,
            BevelOptions {
                amount: width,
                ..default()
            },
        )
    }

    pub(in crate::editor) fn bevel_edges_with(
        &mut self,
        selected: &BTreeSet<Edge>,
        options: BevelOptions,
    ) -> Result<BTreeSet<usize>, String> {
        const MANIFOLD: &str = "Фаска требует согласованной сетки вокруг выбранных рёбер";
        if selected.is_empty() || !options.amount.is_finite() || options.amount <= 0.0 {
            return Err("Выберите рёбра и задайте положительную ширину фаски".into());
        }
        if !options.valid_profile() || !options.spread.is_finite() || options.spread < 0.0 {
            return Err("Недопустимые параметры профиля фаски".into());
        }
        let adjacency = self.edges();
        let endpoints: BTreeSet<_> = selected.iter().flat_map(|e| [e.0, e.1]).collect();
        if selected
            .iter()
            .any(|e| adjacency.get(e).is_none_or(|faces| faces.len() != 2))
            || adjacency.iter().any(|(e, faces)| {
                (endpoints.contains(&e.0) || endpoints.contains(&e.1)) && faces.len() > 2
            })
        {
            return Err(MANIFOLD.into());
        }

        let normals: Vec<_> = self
            .faces
            .iter()
            .map(|f| self.normal(f).normalize_or_zero())
            .collect();
        let dihedral: BTreeMap<_, _> = selected
            .iter()
            .map(|&edge| {
                let [a, b] = adjacency[&edge].as_slice() else {
                    unreachable!()
                };
                (edge, normals[*a].dot(normals[*b]).clamp(-1.0, 1.0))
            })
            .collect();
        let mut offsets = BTreeMap::new();
        let mut rail_fractions = BTreeMap::<(usize, usize), RailOffset>::new();
        for (fi, face) in self.faces.iter().enumerate() {
            let normal = normals[fi];
            for i in 0..face.corners.len() {
                let (prev, c, next) = neighbors(face, i);
                let prev_edge = Edge::new(prev.vertex, c.vertex);
                let next_edge = Edge::new(c.vertex, next.vertex);
                let before = selected.contains(&prev_edge);
                let after = selected.contains(&next_edge);
                if !before && !after {
                    continue;
                }
                let p = self.positions[c.vertex];
                let u = self.positions[prev.vertex] - p;
                let v = self.positions[next.vertex] - p;
                let sine = u.normalize_or_zero().cross(v.normalize_or_zero()).length();
                let sign = v.cross(u).dot(normal).signum();
                if sine < 0.0001 || (sign < 0.0 && before != after) {
                    return Err("Невозможно построить фаску у этого угла".into());
                }
                let fraction = |edge, rail_len| {
                    sign * options.width_type.fraction(
                        options.amount,
                        rail_len,
                        sine,
                        dihedral[&edge],
                    )
                };
                let offset = Offset {
                    along_prev: if after {
                        fraction(next_edge, u.length())
                    } else {
                        0.0
                    },
                    along_next: if before {
                        fraction(prev_edge, v.length())
                    } else {
                        0.0
                    },
                    miter: if !(before && after) {
                        Miter::Sharp
                    } else if sign < 0.0 {
                        options.outer_miter
                    } else {
                        options.inner_miter
                    },
                };
                let largest = offset.along_prev.abs().max(offset.along_next.abs());
                if !largest.is_finite() {
                    return Err("Этот способ измерения ширины неприменим к выбранному углу".into());
                }
                if before != after {
                    let (other, fraction) = if before {
                        (next.vertex, offset.along_next)
                    } else {
                        (prev.vertex, offset.along_prev)
                    };
                    rail_fractions
                        .entry((c.vertex, other))
                        .or_default()
                        .add(fraction, sine);
                }
                offsets.insert((fi, i), offset);
            }
        }

        let rail_fractions: BTreeMap<_, _> = rail_fractions
            .into_iter()
            .map(|(edge, offset)| (edge, offset.fraction(options.loop_slide)))
            .collect();
        let limit = if options.clamp_overlap {
            let mut occupied = AVAILABLE_EDGE;
            for (fi, face) in self.faces.iter().enumerate() {
                for (i, (a, b)) in face.edges().enumerate() {
                    let j = (i + 1) % face.corners.len();
                    let length = self.positions[a].distance(self.positions[b]);
                    let travel = |corner, from, to, forward| {
                        rail_fractions.get(&(from, to)).copied().unwrap_or_else(|| {
                            offsets.get(&(fi, corner)).map_or(0.0, |offset| {
                                let fraction = if forward {
                                    offset.along_next
                                } else {
                                    offset.along_prev
                                };
                                fraction.abs()
                                    + if offset.miter == Miter::Sharp {
                                        0.0
                                    } else {
                                        miter_extension(options.spread, length)
                                    }
                            })
                        })
                    };
                    occupied = occupied.max(travel(i, a, b, true) + travel(j, b, a, false));
                }
            }
            AVAILABLE_EDGE / occupied
        } else {
            1.0
        };

        let mut positions = self.positions.clone();
        let mut anchors: Vec<_> = (0..positions.len()).collect();
        let mut push = |anchor, point| {
            let vertex = positions.len();
            positions.push(point);
            anchors.push(anchor);
            vertex
        };
        let rails: BTreeMap<_, _> = rail_fractions
            .into_iter()
            .map(|((a, b), fraction)| {
                let t = fraction * limit;
                let index = push(a, self.positions[a].lerp(self.positions[b], t));
                ((a, b), (index, t))
            })
            .collect();
        let on_rail = |c: &Corner, toward: &Corner| {
            rails
                .get(&(c.vertex, toward.vertex))
                .map(|&(vertex, t)| c.between(toward, vertex, t))
        };
        let mut faces = Vec::with_capacity(self.faces.len() + selected.len());
        let mut trims = BTreeMap::new();
        for (fi, face) in self.faces.iter().enumerate() {
            let mut corners = Vec::new();
            for i in 0..face.corners.len() {
                let (prev, c, next) = neighbors(face, i);
                let incoming = on_rail(c, prev);
                let outgoing = on_rail(c, next);
                if let Some(offset) = offsets.get(&(fi, i)) {
                    let prev_edge = Edge::new(prev.vertex, c.vertex);
                    let next_edge = Edge::new(c.vertex, next.vertex);
                    match (incoming, outgoing) {
                        (Some(mut corner), _) | (_, Some(mut corner)) => {
                            if options.harden_normals || options.preserve_sharp {
                                corner.source_normal = c.source_normal;
                            }
                            for edge in [prev_edge, next_edge] {
                                if selected.contains(&edge) {
                                    trims.insert((fi, edge, c.vertex), corner.clone());
                                }
                            }
                            corners.push(corner);
                        }
                        (None, None) => {
                            let p = self.positions[c.vertex];
                            let u = self.positions[prev.vertex] - p;
                            let v = self.positions[next.vertex] - p;
                            let ap = offset.along_prev * limit;
                            let an = offset.along_next * limit;
                            let mut make_corner = |ap, an| Corner {
                                vertex: push(c.vertex, p + u * ap + v * an),
                                uv: c.uv + (prev.uv - c.uv) * ap + (next.uv - c.uv) * an,
                                uv1: c.uv1 + (prev.uv1 - c.uv1) * ap + (next.uv1 - c.uv1) * an,
                                color: c.color
                                    + (prev.color - c.color) * ap
                                    + (next.color - c.color) * an,
                                id_color: c.id_color.zip(prev.id_color).zip(next.id_color)
                                    .map(|((c, prev), next)| c + (prev - c) * ap + (next - c) * an),
                                source_normal: if options.harden_normals || options.preserve_sharp {
                                    c.source_normal
                                } else {
                                    None
                                },
                            };
                            let (entry, exit) =
                                if offset.miter == Miter::Sharp || options.spread == 0.0 {
                                    let corner = make_corner(ap, an);
                                    corners.push(corner.clone());
                                    (corner.clone(), corner)
                                } else {
                                    let du = miter_extension(options.spread, u.length()) * limit;
                                    let dv = miter_extension(options.spread, v.length()) * limit;
                                    let entry = make_corner(ap + du, an);
                                    let exit = make_corner(ap, an + dv);
                                    corners.push(entry.clone());
                                    if offset.miter == Miter::Patch {
                                        corners.push(make_corner(ap, an));
                                    } else {
                                        let segments = options.segments.max(2);
                                        for step in 1..segments {
                                            let weights = options
                                                .profile_weights(step as f32 / segments as f32);
                                            corners.push(make_corner(
                                                ap + du * weights.x,
                                                an + dv * weights.y,
                                            ));
                                        }
                                    }
                                    corners.push(exit.clone());
                                    (entry, exit)
                                };
                            trims.insert((fi, prev_edge, c.vertex), entry);
                            trims.insert((fi, next_edge, c.vertex), exit);
                        }
                    }
                } else {
                    let keep_corner = incoming.is_none() || outgoing.is_none();
                    corners.extend(incoming);
                    if keep_corner {
                        corners.push(c.clone());
                    }
                    corners.extend(outgoing);
                }
            }
            faces.push(Face {
                corners,
                material: face.material,
                smooth: face.smooth,
            });
        }
        let flat_profile = options.flat_profile();
        let mut flat_chords = BTreeMap::new();
        let mut profile_vertices = BTreeMap::new();
        let mut profile_edges = BTreeMap::new();
        let mut edge_counts = BTreeMap::<usize, usize>::new();
        let mut uv_seams = self.uv_seams.clone();
        if options.preserve_uv_seams {
            uv_seams.extend(self.uv_seams.iter().map(|edge| {
                let end = |a, b| rails.get(&(a, b)).map_or(a, |&(vertex, _)| vertex);
                Edge::new(end(edge.0, edge.1), end(edge.1, edge.0))
            }));
        }
        for &edge in selected {
            let (left, right) = (adjacency[&edge][0], adjacency[&edge][1]);
            let (a, b) = if self.faces[left]
                .edges()
                .any(|pair| pair == (edge.0, edge.1))
            {
                (edge.0, edge.1)
            } else {
                (edge.1, edge.0)
            };
            if !self.faces[right].edges().any(|pair| pair == (b, a)) {
                return Err(MANIFOLD.into());
            }
            let trim = |face, end| &trims[&(face, edge, end)];
            let mut rows = vec![[trim(left, a).clone(), trim(left, b).clone()]];
            for segment in 1..options.segments {
                let t = segment as f32 / options.segments as f32;
                let weights = options.profile_weights(t);
                let row = [a, b].map(|end| {
                    let left = trim(left, end);
                    let right = trim(right, end);
                    let origin = self.positions[end];
                    let position = origin
                        + (positions[left.vertex] - origin) * weights.x
                        + (positions[right.vertex] - origin) * weights.y;
                    let chord = Edge::new(left.vertex, right.vertex);
                    let sample = if left.vertex == chord.0 {
                        segment
                    } else {
                        options.segments - segment
                    };
                    let vertex = *profile_vertices.entry((chord, sample)).or_insert_with(|| {
                        let vertex = positions.len();
                        positions.push(position);
                        anchors.push(end);
                        vertex
                    });
                    left.between(right, vertex, t)
                });
                rows.push(row);
            }
            rows.push([trim(right, a).clone(), trim(right, b).clone()]);
            for end in [a, b] {
                *edge_counts.entry(end).or_default() += 1;
            }
            let sharp = options.preserve_sharp
                && [a, b].into_iter().any(|vertex| {
                    let normal = |fi: usize| {
                        let face = &self.faces[fi];
                        face.corners
                            .iter()
                            .find(|corner| corner.vertex == vertex)
                            .and_then(|corner| corner.source_normal)
                            .or_else(|| (!face.smooth).then_some(normals[fi]))
                    };
                    normal(left)
                        .zip(normal(right))
                        .is_some_and(|(x, y)| x.dot(y) < 0.999)
                });
            if (options.harden_normals && options.segments > 1) || sharp {
                for (i, row) in rows.iter_mut().enumerate() {
                    let segments = options.segments as f32;
                    let mut step = i as f32;
                    if sharp {
                        step = step.clamp(0.5, segments - 0.5);
                    }
                    let t = step / segments;
                    let normal = normals[left].lerp(normals[right], t).normalize_or_zero();
                    if normal != Vec3::ZERO {
                        for corner in row {
                            corner.source_normal = Some(normal);
                        }
                    }
                }
            }
            if options.preserve_uv_seams && self.uv_seams.contains(&edge) {
                for row in [&rows[0], &rows[options.segments]] {
                    uv_seams.insert(Edge::new(row[0].vertex, row[1].vertex));
                }
            }
            for pair in rows.windows(2) {
                let (left_row, right_row) = (&pair[0], &pair[1]);
                for side in 0..2 {
                    let end = Edge::new(left_row[side].vertex, right_row[side].vertex);
                    profile_edges.insert(end, edge);
                }
                faces.push(Face {
                    corners: [
                        left_row[1].clone(),
                        left_row[0].clone(),
                        right_row[0].clone(),
                        right_row[1].clone(),
                    ]
                    .into(),
                    material: options.material.unwrap_or(self.faces[left].material),
                    smooth: options.segments > 1 && !flat_profile,
                });
            }
            // A flat profile shares its chord with an uncut neighboring
            // face. Insert the samples there to avoid a zero-area cap.
            if flat_profile && options.segments > 1 {
                for side in 0..2 {
                    let a = rows[0][side].vertex;
                    let b = rows[options.segments][side].vertex;
                    let vertices: Vec<_> = rows[1..options.segments]
                        .iter()
                        .map(|row| row[side].vertex)
                        .collect();
                    flat_chords.insert(Edge::new(a, b), (a, vertices));
                }
            }
        }
        if !flat_chords.is_empty() {
            for face in &mut faces[..self.faces.len()] {
                let original = std::mem::take(&mut face.corners);
                for (a, b) in original.iter().zip(original.iter().cycle().skip(1)) {
                    face.corners.push(a.clone());
                    if let Some((start, samples)) = flat_chords.get(&Edge::new(a.vertex, b.vertex))
                    {
                        for (i, &vertex) in samples.iter().enumerate() {
                            let t = (i + 1) as f32 / options.segments as f32;
                            let mut corner = if a.vertex == *start {
                                a.between(b, vertex, t)
                            } else {
                                let vertex = samples[samples.len() - 1 - i];
                                a.between(b, vertex, t)
                            };
                            if options.harden_normals || options.preserve_sharp {
                                corner.source_normal = a
                                    .source_normal
                                    .zip(b.source_normal)
                                    .map(|(a, b)| a.lerp(b, t).normalize_or_zero())
                                    .or(a.source_normal)
                                    .or(b.source_normal);
                            }
                            face.corners.push(corner);
                        }
                    }
                }
            }
        }
        let boundary = adjacency
            .iter()
            .filter(|(_, faces)| faces.len() == 1)
            .flat_map(|(edge, _)| [edge.0, edge.1])
            .collect();
        for cap in cap_boundaries(&faces, &anchors, &boundary, MANIFOLD)? {
            let anchor = anchors[cap.corners[0].vertex];
            let junction = edge_counts[&anchor] >= 3;
            if junction && options.intersection == Intersection::Cutoff {
                cutoff_junction(&mut faces, &mut positions, cap, &profile_edges);
            } else if junction && options.segments > 1 {
                grid_junction(&mut faces, &mut positions, cap, !flat_profile);
            } else {
                faces.push(cap);
            }
        }
        if let Some(material) = options.material {
            for face in &mut faces[self.faces.len()..] {
                face.material = material;
            }
        }
        let chosen = (self.faces.len()..faces.len()).collect();
        let mut result = Geometry {
            positions,
            faces,
            loose_edges: self.loose_edges.clone(),
            uv_seams,
        };
        if options.clamp_overlap
            && let Some((stitched, chosen)) =
                stitch_rail_contacts(&result, &rails, self.faces.len())
        {
            *self = stitched;
            return Ok(chosen);
        }
        result.compact();
        result.validate()?;
        *self = result;
        Ok(chosen)
    }
}

fn stitch_rail_contacts(
    mesh: &Geometry,
    rails: &BTreeMap<(usize, usize), (usize, f32)>,
    first_bevel_face: usize,
) -> Option<(Geometry, BTreeSet<usize>)> {
    let mut remap = BTreeMap::new();
    let mut contact_positions = Vec::new();
    let contact = AVAILABLE_EDGE - 2.0 * f32::EPSILON;
    for (&(a, b), &(vertex, fraction)) in rails {
        if let Some(&(other, reverse)) = rails.get(&(b, a)) {
            if a < b && fraction + reverse >= contact {
                remap.insert(other, vertex);
                let position =
                    mesh.positions[a].lerp(mesh.positions[b], fraction / (fraction + reverse));
                contact_positions.push((vertex, position));
            }
        } else if fraction >= contact {
            remap.insert(vertex, b);
        }
    }
    if remap.is_empty() {
        return None;
    }

    let vertex = |old| remap.get(&old).copied().unwrap_or(old);
    let mut candidate = mesh.clone();
    for (index, position) in contact_positions {
        candidate.positions[index] = position;
    }
    let mut chosen = BTreeSet::new();
    for (i, mut face) in std::mem::take(&mut candidate.faces).into_iter().enumerate() {
        for corner in &mut face.corners {
            corner.vertex = vertex(corner.vertex);
        }
        face.corners.dedup_by_key(|corner| corner.vertex);
        if face.corners.len() > 1
            && face.corners.first().unwrap().vertex == face.corners.last().unwrap().vertex
        {
            face.corners.pop();
        }
        if face.corners.len() >= 3 {
            if i >= first_bevel_face {
                chosen.insert(candidate.faces.len());
            }
            candidate.faces.push(face);
        }
    }
    candidate.uv_seams = mesh
        .uv_seams
        .iter()
        .map(|edge| Edge::new(vertex(edge.0), vertex(edge.1)))
        .filter(|edge| edge.0 != edge.1)
        .collect();
    candidate.compact();
    let boundary_count = |edges: &BTreeMap<Edge, Vec<usize>>| {
        edges.values().filter(|faces| faces.len() == 1).count()
    };
    let edges = candidate.edges();
    if chosen.is_empty()
        || boundary_count(&edges) > boundary_count(&mesh.edges())
        || edges.values().any(|faces| faces.len() > 2)
    {
        return None;
    }
    candidate.validate().ok()?;
    Some((candidate, chosen))
}

fn profile_weights(profile: f32, t: f32) -> (f32, f32) {
    let angle = t * std::f32::consts::FRAC_PI_2;
    let (sin, cos) = angle.sin_cos();
    let exponent = if profile <= 0.5 {
        0.5 / profile.max(0.01)
    } else {
        (2.0 * (1.0 - profile)).max(0.001)
    };
    (1.0 - sin.powf(exponent), 1.0 - cos.powf(exponent))
}

fn neighbors(face: &Face, i: usize) -> (&Corner, &Corner, &Corner) {
    let n = face.corners.len();
    (
        &face.corners[(i + n - 1) % n],
        &face.corners[i],
        &face.corners[(i + 1) % n],
    )
}

fn cap_boundaries(
    faces: &[Face],
    anchors: &[usize],
    open: &BTreeSet<usize>,
    error: &str,
) -> Result<Vec<Face>, String> {
    let mut borders = BTreeMap::<Edge, Vec<_>>::new();
    for face in faces.iter() {
        for (a, b) in face.corners.iter().zip(face.corners.iter().cycle().skip(1)) {
            if anchors[a.vertex] == anchors[b.vertex] && !open.contains(&anchors[a.vertex]) {
                borders
                    .entry(Edge::new(a.vertex, b.vertex))
                    .or_default()
                    .push((b.clone(), a.vertex, face.material));
            }
        }
    }
    let mut outgoing = BTreeMap::new();
    for mut sides in borders.into_values() {
        match sides.len() {
            1 => {
                let (b, a, material) = sides.pop().unwrap();
                if outgoing.insert(b.vertex, (b, a, material)).is_some() {
                    return Err(error.into());
                }
            }
            2 if sides[0].0.vertex == sides[1].1 => {}
            _ => return Err(error.into()),
        }
    }
    let mut caps = Vec::new();
    while let Some((&start, (_, _, material))) = outgoing.first_key_value() {
        let material = *material;
        let mut corners = Vec::new();
        let mut current = start;
        loop {
            let (corner, next, _) = outgoing.remove(&current).ok_or(error)?;
            corners.push(corner);
            if next == start {
                break;
            }
            current = next;
        }
        if corners.len() < 3 {
            return Err(error.into());
        }
        caps.push(Face {
            corners,
            material,
            smooth: false,
        });
    }
    Ok(caps)
}

fn junction_center(positions: &[Vec3], corners: &[Corner]) -> Vec3 {
    corners
        .iter()
        .map(|corner| positions[corner.vertex])
        .sum::<Vec3>()
        / corners.len() as f32
}

fn inset_corner(positions: &mut Vec<Vec3>, source: &Corner, center: Vec3) -> Corner {
    let mut corner = source.clone();
    corner.vertex = positions.len();
    corner.source_normal = None;
    positions.push(positions[source.vertex].lerp(center, 0.5));
    corner
}

fn grid_junction(faces: &mut Vec<Face>, positions: &mut Vec<Vec3>, cap: Face, smooth: bool) {
    let center = junction_center(positions, &cap.corners);
    let ring: Vec<_> = cap
        .corners
        .iter()
        .map(|corner| inset_corner(positions, corner, center))
        .collect();
    for i in 0..ring.len() {
        let j = (i + 1) % ring.len();
        faces.push(Face {
            corners: [
                cap.corners[i].clone(),
                cap.corners[j].clone(),
                ring[j].clone(),
                ring[i].clone(),
            ]
            .into(),
            material: cap.material,
            smooth,
        });
    }
    faces.push(Face {
        corners: ring,
        smooth,
        ..cap
    });
}

fn cutoff_junction(
    faces: &mut Vec<Face>,
    positions: &mut Vec<Vec3>,
    mut cap: Face,
    profile_edges: &BTreeMap<Edge, Edge>,
) {
    let mut profiles: Vec<_> = cap
        .edges()
        .map(|(a, b)| profile_edges.get(&Edge::new(a, b)).copied())
        .collect();
    let start = (0..profiles.len())
        .find(|&i| profiles[i] != profiles[(i + profiles.len() - 1) % profiles.len()]);
    let Some(start) = start else {
        faces.push(cap);
        return;
    };
    profiles.rotate_left(start);
    cap.corners.rotate_left(start);
    let center = junction_center(positions, &cap.corners);
    let ring: Vec<_> = cap
        .corners
        .iter()
        .map(|corner| inset_corner(positions, corner, center))
        .collect();
    let mut inner = Vec::new();
    let mut i = 0;
    while i < profiles.len() {
        let run = profiles[i].map_or(1, |profile| {
            profiles[i..]
                .iter()
                .take_while(|edge| **edge == Some(profile))
                .count()
        });
        let end = (i + run) % profiles.len();
        let a = ring[i].clone();
        let b = ring[end].clone();
        let mut corners = cap.corners[i..i + run].to_vec();
        corners.extend([cap.corners[end].clone(), b, a.clone()]);
        faces.push(Face {
            corners,
            material: cap.material,
            smooth: false,
        });
        inner.push(a);
        i += run;
    }
    faces.push(Face {
        corners: inner,
        ..cap
    });
}
