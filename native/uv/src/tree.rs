//! Structured Ward clustering of normals and normalized triangle centroids.
//! Matches the Geometric mode's six features, adjacency and disconnected joins.
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};
type Edge = (usize, usize);
pub fn edges(faces: &[[usize; 3]]) -> BTreeMap<Edge, Vec<usize>> {
    let mut out = BTreeMap::<Edge, Vec<usize>>::new();
    for (i, f) in faces.iter().enumerate() {
        for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
            out.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    out
}
pub struct Union {
    parent: Vec<usize>,
}
impl Union {
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    pub fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }
    pub fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        self.parent[b] = a;
    }
}
pub fn prepare(vertices: &[[f64; 3]], faces: &[[usize; 3]]) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let all = edges(faces);
    if all.values().all(|v| v.len() <= 2) {
        return (vertices.to_vec(), faces.to_vec());
    }
    let mut union = Union::new(faces.len() * 3);
    for ((a, b), adjacent) in all {
        if adjacent.len() != 2 {
            continue;
        }
        let (i, j) = (adjacent[0], adjacent[1]);
        for v in [a, b] {
            let ci = faces[i].iter().position(|&x| x == v).unwrap();
            let cj = faces[j].iter().position(|&x| x == v).unwrap();
            union.join(i * 3 + ci, j * 3 + cj);
        }
    }
    let mut ids = BTreeMap::new();
    let mut v = Vec::new();
    let mut f = Vec::new();
    for (i, face) in faces.iter().enumerate() {
        let mut triangle = [0; 3];
        for c in 0..3 {
            let root = union.root(i * 3 + c);
            triangle[c] = *ids.entry(root).or_insert_with(|| {
                v.push(vertices[face[c]]);
                v.len() - 1
            });
        }
        f.push(triangle);
    }
    (v, f)
}
#[derive(Clone, Copy, Debug)]
struct Candidate(f64, usize, usize);
impl PartialEq for Candidate {
    fn eq(&self, b: &Self) -> bool {
        self.cmp(b) == Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        Some(self.cmp(b))
    }
}
impl Ord for Candidate {
    fn cmp(&self, b: &Self) -> Ordering {
        b.0.total_cmp(&self.0)
            .then_with(|| b.1.cmp(&self.1))
            .then_with(|| b.2.cmp(&self.2))
    }
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn build(vertices: &[[f64; 3]], faces: &[[usize; 3]]) -> Vec<[usize; 3]> {
    let n = faces.len();
    if n < 2 {
        return Vec::new();
    }
    let count = 2 * n - 1;
    let mut adjacency = vec![BTreeSet::new(); count];
    let mut union = Union::new(n);
    for adjacent in edges(faces).into_values().filter(|a| a.len() == 2) {
        let (a, b) = (adjacent[0], adjacent[1]);
        adjacency[a].insert(b);
        adjacency[b].insert(a);
        union.join(a, b);
    }
    let mut representatives = BTreeMap::new();
    for i in 0..n {
        representatives.entry(union.root(i)).or_insert(i);
    }
    let mut reps: Vec<_> = representatives.into_values().collect();
    reps.sort_unstable();
    for pair in reps.windows(2) {
        adjacency[pair[0]].insert(pair[1]);
        adjacency[pair[1]].insert(pair[0]);
    }
    let centroids: Vec<[f64; 3]> = faces
        .iter()
        .map(|f| {
            std::array::from_fn(|k| {
                (vertices[f[0]][k] + vertices[f[1]][k] + vertices[f[2]][k]) / 3.
            })
        })
        .collect();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for c in &centroids {
        for k in 0..3 {
            low[k] = low[k].min(c[k]);
            high[k] = high[k].max(c[k]);
        }
    }
    let extent = (0..3).map(|k| high[k] - low[k]).fold(1e-12, f64::max);
    let mut moments = vec![[0.; 6]; count];
    let mut sizes = vec![0.; count];
    let mut active = vec![true; count];
    for (i, f) in faces.iter().enumerate() {
        let normal = cross(
            sub(vertices[f[1]], vertices[f[0]]),
            sub(vertices[f[2]], vertices[f[0]]),
        );
        let length = normal.iter().map(|x| x * x).sum::<f64>().sqrt();
        for k in 0..3 {
            moments[i][k] = if length > 1e-13 {
                normal[k] / length
            } else {
                0.
            };
            moments[i][k + 3] = 0.3 * ((centroids[i][k] - low[k]) / extent);
        }
        sizes[i] = 1.;
    }
    fn distance(a: usize, b: usize, sizes: &[f64], m: &[[f64; 6]]) -> f64 {
        let variance = (0..6)
            .map(|k| {
                let d = m[a][k] / sizes[a] - m[b][k] / sizes[b];
                d * d
            })
            .sum::<f64>();
        sizes[a] * sizes[b] / (sizes[a] + sizes[b]) * variance
    }
    let mut heap = BinaryHeap::new();
    for a in 0..n {
        for &b in &adjacency[a] {
            if b < a {
                heap.push(Candidate(distance(a, b, &sizes, &moments), a, b));
            }
        }
    }
    let mut tree = Vec::with_capacity(n - 1);
    for next in n..count {
        let Candidate(_, a, b) = loop {
            let c = heap.pop().expect("Connected face graph");
            if active[c.1] && active[c.2] {
                break c;
            }
        };
        active[a] = false;
        active[b] = false;
        sizes[next] = sizes[a] + sizes[b];
        for k in 0..6 {
            moments[next][k] = moments[a][k] + moments[b][k];
        }
        let neighbors: BTreeSet<_> = adjacency[a]
            .union(&adjacency[b])
            .copied()
            .filter(|&k| active[k] && k < next)
            .collect();
        for &other in &neighbors {
            adjacency[other].remove(&a);
            adjacency[other].remove(&b);
            adjacency[other].insert(next);
            heap.push(Candidate(
                distance(next, other, &sizes, &moments),
                next,
                other,
            ));
        }
        adjacency[next] = neighbors;
        adjacency[a].clear();
        adjacency[b].clear();
        tree.push([next, b, a]);
    }
    tree
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_faces_form_one_complete_tree() {
        let v = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [4., 0., 0.],
            [5., 0., 0.],
            [4., 1., 0.],
        ];
        assert_eq!(build(&v, &[[0, 1, 2], [3, 4, 5]]), vec![[2, 0, 1]]);
    }
    #[test]
    fn nonmanifold_fans_split_without_moving_any_corner() {
        let v = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., 1.],
        ];
        let f = vec![[0, 1, 2], [1, 0, 3], [0, 1, 4]];
        let (a, b) = prepare(&v, &f);
        assert!(edges(&b).values().all(|v| v.len() <= 2));
        for i in 0..f.len() {
            for j in 0..3 {
                assert_eq!(v[f[i][j]], a[b[i][j]]);
            }
        }
    }
}
