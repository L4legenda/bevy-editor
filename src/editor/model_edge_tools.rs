use super::*;

impl Geometry {
    pub(in crate::editor) fn edge_loops(&self, seeds: &BTreeSet<Edge>) -> BTreeSet<Edge> {
        let faces = self.edges();
        let mut incident = vec![Vec::new(); self.positions.len()];
        for &edge in faces.keys() {
            incident[edge.0].push(edge);
            incident[edge.1].push(edge);
        }
        let mut selected = BTreeSet::new();
        let mut pending: Vec<_> = seeds.iter().copied().collect();
        while let Some(edge) = pending.pop() {
            let Some(side) = faces.get(&edge) else {
                continue;
            };
            if !selected.insert(edge) {
                continue;
            }
            for vertex in [edge.0, edge.1] {
                let fan = &incident[vertex];
                let continuation =
                    match side.len() {
                        1 => unique(
                            fan.iter()
                                .copied()
                                .filter(|e| *e != edge && faces[e].len() == 1),
                        ),
                        2 if fan.len() == 4
                            && fan.iter().all(|e| {
                                faces[e].len() == 2
                                    && faces[e].iter().all(|&f| self.faces[f].corners.len() == 4)
                            }) =>
                        {
                            unique(fan.iter().copied().filter(|e| {
                                *e != edge && faces[e].iter().all(|f| !side.contains(f))
                            }))
                        }
                        _ => None,
                    };
                pending.extend(continuation);
            }
        }
        selected
    }

    pub(in crate::editor) fn slide_edges(
        &mut self,
        selected: &BTreeSet<Edge>,
        factor: f32,
    ) -> Result<(), String> {
        const INVALID: &str = "Для скольжения выберите непрерывную цепочку рёбер с двумя сторонами";
        let adjacency = self.edges();
        let mut rails = BTreeMap::<(usize, usize), Vec<(usize, usize)>>::new();
        let mut valence = BTreeMap::<usize, usize>::new();
        for &edge in selected {
            let sides = adjacency
                .get(&edge)
                .filter(|sides| sides.len() == 2)
                .ok_or(INVALID)?;
            for vertex in [edge.0, edge.1] {
                let count = valence.entry(vertex).or_default();
                *count += 1;
                if *count > 2 {
                    return Err(INVALID.into());
                }
            }
            for &side in sides {
                let corners = &self.faces[side].corners;
                let rail = |vertex, opposite| -> Result<(usize, usize), String> {
                    let i = corners
                        .iter()
                        .position(|c| c.vertex == vertex)
                        .ok_or(INVALID)?;
                    let prev = corners[(i + corners.len() - 1) % corners.len()].vertex;
                    let next = corners[(i + 1) % corners.len()].vertex;
                    let neighbor = if prev == opposite { next } else { prev };
                    if selected.contains(&Edge::new(vertex, neighbor)) {
                        return Err(INVALID.into());
                    }
                    Ok((vertex, neighbor))
                };
                let a = rail(edge.0, edge.1)?;
                let b = rail(edge.1, edge.0)?;
                rails.entry(a).or_default().push(b);
                rails.entry(b).or_default().push(a);
            }
        }
        let mut choices = BTreeMap::new();
        for &vertex in valence.keys() {
            if choices.contains_key(&vertex) {
                continue;
            }
            let mut sides = rails
                .range((vertex, 0)..=(vertex, usize::MAX))
                .map(|(&key, _)| key);
            let first = sides.next().ok_or(INVALID)?;
            let second = sides.next().ok_or(INVALID)?;
            if sides.next().is_some() {
                return Err(INVALID.into());
            }
            let mut pending = vec![if factor < 0.0 { second } else { first }];
            while let Some((vertex, rail)) = pending.pop() {
                if let Some(previous) = choices.insert(vertex, rail) {
                    if previous != rail {
                        return Err(INVALID.into());
                    }
                } else {
                    pending.extend(rails[&(vertex, rail)].iter().copied());
                }
            }
        }
        if choices.is_empty() || choices.values().any(|v| choices.contains_key(v)) {
            return Err(INVALID.into());
        }
        if selected.iter().any(|edge| {
            let a = (edge.0, choices[&edge.0]);
            let b = (edge.1, choices[&edge.1]);
            !rails[&a].contains(&b)
        }) {
            return Err(INVALID.into());
        }
        for (vertex, rail) in choices {
            self.positions[vertex] =
                self.positions[vertex].lerp(self.positions[rail], factor.abs().min(0.99));
        }
        Ok(())
    }
}

fn unique(mut edges: impl Iterator<Item = Edge>) -> Option<Edge> {
    let edge = edges.next()?;
    edges.next().is_none().then_some(edge)
}
