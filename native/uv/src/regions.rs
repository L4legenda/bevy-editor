//! Partition at material boundaries, hard folds and the outline of protected paint.
use crate::tree::{Union, edges};
use serde::Serialize;
use std::collections::BTreeMap;

pub fn groups(
    vertices: &[[f64; 3]],
    faces: &[[usize; 3]],
    materials: &[usize],
    paint: &[usize],
) -> Vec<usize> {
    let mut protected = vec![false; faces.len()];
    for &i in paint {
        protected[i] = true;
    }
    let normals: Vec<_> = faces
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| vertices[i]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = n.iter().map(|x| x * x).sum::<f64>().sqrt();
            n.map(|x| if length > 0. { x / length } else { 0. })
        })
        .collect();
    let mut union = Union::new(faces.len());
    for pair in edges(faces).into_values() {
        let [a, b] = pair.as_slice() else { continue };
        let dot = (0..3).map(|i| normals[*a][i] * normals[*b][i]).sum::<f64>();
        if protected[*a] == protected[*b]
            && (protected[*a] || (materials.get(*a) == materials.get(*b) && dot > 0.5))
        {
            union.join(*a, *b);
        }
    }
    (0..faces.len()).map(|i| union.root(i)).collect()
}
#[derive(Serialize)]
pub struct Part {
    ids: Vec<usize>,
    positions: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
    tree: Vec<[usize; 3]>,
}
pub fn parts(vertices: &[[f64; 3]], faces: &[[usize; 3]], groups: &[usize]) -> Vec<Part> {
    let mut regions = BTreeMap::<usize, Vec<usize>>::new();
    for (i, group) in groups.iter().enumerate() {
        regions.entry(*group).or_default().push(i);
    }
    regions
        .into_values()
        .map(|ids| {
            let mut local = BTreeMap::new();
            let mut positions = Vec::new();
            let triangles: Vec<_> = ids
                .iter()
                .map(|&i| {
                    faces[i].map(|v| {
                        *local.entry(v).or_insert_with(|| {
                            positions.push(vertices[v]);
                            positions.len() - 1
                        })
                    })
                })
                .collect();
            let (positions, triangles) = crate::tree::prepare(&positions, &triangles);
            let tree = crate::tree::build(&positions, &triangles);
            Part {
                ids,
                positions,
                triangles,
                tree,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hard_fold_stays_joined_only_if_both_faces_are_protected() {
        let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let faces = [[0, 1, 2], [1, 0, 3]];
        for mask in [vec![], vec![0]] {
            let parts = groups(&vertices, &faces, &[], &mask);
            assert_ne!(parts[0], parts[1]);
        }
        let parts = groups(&vertices, &faces, &[0, 1], &[0, 1]);
        assert_eq!(parts[0], parts[1]);
    }
}
