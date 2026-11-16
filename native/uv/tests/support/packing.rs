//! Independent checks for packing: connected islands, similarity transforms and triangle overlap.
use serde_json::{Value, json};
use std::collections::BTreeMap;
type P = [f64; 2];
type T = [P; 3];
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn area(t: T) -> f64 {
    cross(sub(t[1], t[0]), sub(t[2], t[0])).abs() * 0.5
}
fn root(p: &mut [usize], i: usize) -> usize {
    if p[i] != i {
        p[i] = root(p, p[i]);
    }
    p[i]
}
fn overlap(a: T, b: T) -> bool {
    for t in [a, b] {
        for i in 0..3 {
            let d = sub(t[(i + 1) % 3], t[i]);
            let n = [-d[1], d[0]];
            let length = dot(n, n).sqrt();
            if length < 1e-15 {
                continue;
            }
            let projection = |s: T| {
                let v = s.map(|p| dot(p, n) / length);
                (
                    v.into_iter().fold(f64::INFINITY, f64::min),
                    v.into_iter().fold(f64::NEG_INFINITY, f64::max),
                )
            };
            let (al, ah) = projection(a);
            let (bl, bh) = projection(b);
            if ah.min(bh) - al.max(bl) <= 1e-10 {
                return false;
            }
        }
    }
    true
}

#[test]
fn overlap_check_detects_containment_but_allows_touching_edges() {
    let a = [[0., 0.], [2., 0.], [0., 2.]];
    assert!(overlap(a, [[0.1, 0.1], [0.2, 0.1], [0.1, 0.2]]));
    assert!(!overlap(a, [[2., 0.], [2., 2.], [0., 2.]]));
}
pub fn check(request: &Value, packed: &Value) -> Value {
    let faces: Vec<[usize; 3]> = serde_json::from_value(request["triangles"].clone()).unwrap();
    let before: Vec<T> = serde_json::from_value(request["existing_uv"].clone()).unwrap();
    let after: Vec<T> = serde_json::from_value(packed["uv"].clone()).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(faces.len(), after.len());
    let mut parent: Vec<_> = (0..faces.len()).collect();
    // Match the worker's vertex/UV topology, including exact float32 source keys.
    let mut edges = BTreeMap::new();
    for (i, f) in faces.iter().enumerate() {
        for c in 0..3 {
            let vertex = |k: usize| {
                (
                    f[k],
                    (before[i][k][0] as f32).to_bits(),
                    (before[i][k][1] as f32).to_bits(),
                )
            };
            let a = vertex(c);
            let b = vertex((c + 1) % 3);
            let key = (a.min(b), a.max(b));
            if let Some(&j) = edges.get(&key) {
                let r = root(&mut parent, j);
                let s = root(&mut parent, i);
                parent[r] = s;
            } else {
                edges.insert(key, i);
            }
        }
    }
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for i in 0..faces.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let labels: Vec<_> = (0..faces.len()).map(|i| root(&mut parent, i)).collect();
    let mut error = 0.0_f64;
    let mut scales = Vec::new();
    for ids in groups.values() {
        let seed = *ids
            .iter()
            .max_by(|&&a, &&b| area(before[a]).total_cmp(&area(before[b])))
            .unwrap();
        let a = before[seed];
        let b = after[seed];
        let edge = (0..3)
            .max_by(|&i, &j| {
                dot(sub(a[(i + 1) % 3], a[i]), sub(a[(i + 1) % 3], a[i]))
                    .total_cmp(&dot(sub(a[(j + 1) % 3], a[j]), sub(a[(j + 1) % 3], a[j])))
            })
            .unwrap();
        let origin = a[edge];
        let target = b[edge];
        let u = sub(a[(edge + 1) % 3], origin);
        let v = sub(b[(edge + 1) % 3], target);
        let len = dot(u, u).sqrt();
        if len < 1e-14 {
            continue;
        }
        let scale = (dot(v, v) / dot(u, u)).sqrt();
        scales.push(scale);
        let sign = if cross(sub(a[1], a[0]), sub(a[2], a[0]))
            * cross(sub(b[1], b[0]), sub(b[2], b[0]))
            < 0.
        {
            -1.
        } else {
            1.
        };
        for &id in ids {
            for c in 0..3 {
                let d = sub(before[id][c], origin);
                let x = dot(d, u) / (len * len);
                let y = cross(u, d) / (len * len) * sign;
                let predicted = [
                    target[0] + x * v[0] - y * v[1],
                    target[1] + x * v[1] + y * v[0],
                ];
                error = error
                    .max(dot(sub(predicted, after[id][c]), sub(predicted, after[id][c])).sqrt());
            }
        }
    }
    let bounds: Vec<_> = after
        .iter()
        .map(|t| {
            [
                t.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
                t.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max),
                t.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
                t.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max),
            ]
        })
        .collect();
    let mut sorted: Vec<_> = (0..after.len())
        .filter(|&i| area(after[i]) > 1e-16)
        .collect();
    sorted.sort_by(|&a, &b| bounds[a][0].total_cmp(&bounds[b][0]));
    let mut intersections = 0;
    for (at, &i) in sorted.iter().enumerate() {
        for &j in &sorted[at + 1..] {
            if bounds[j][0] >= bounds[i][1] {
                break;
            }
            if labels[i] == labels[j]
                || bounds[j][2] >= bounds[i][3]
                || bounds[j][3] <= bounds[i][2]
            {
                continue;
            }
            if overlap(after[i], after[j]) {
                intersections += 1;
            }
        }
    }
    let outside = after
        .iter()
        .filter(|t| {
            t.iter()
                .flatten()
                .any(|x| !x.is_finite() || *x < -1e-6 || *x > 1.000001)
        })
        .count();
    let lo = scales.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = scales.iter().copied().fold(0., f64::max);
    let result = json!({"input_islands":groups.len(),"output_islands":packed["charts"],"max_similarity_error_uv":error,"scale_spread":hi/lo-1.,"cross_island_intersections":intersections,"outside_faces":outside,"uv_area_before":before.iter().copied().map(area).sum::<f64>(),"uv_area_after":after.iter().copied().map(area).sum::<f64>()});
    assert!(error < 3e-6, "island shape or seam changed: {result}");
    assert!(
        hi / lo - 1. < 1e-4,
        "relative chart sizes changed: {result}"
    );
    assert_eq!(intersections, 0, "overlapping islands: {result}");
    assert_eq!(outside, 0);
    assert_eq!(
        packed["charts"].as_u64().unwrap() as usize,
        groups.len(),
        "islands were cut/merged"
    );
    result
}
