//! Compare the same triangle/corner UV contract without Python.
use std::{collections::BTreeMap, path::Path};
type V3 = [f64; 3];
type V2 = [f64; 2];
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: V3, b: V3) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn area(a: V2, b: V2, c: V2) -> f64 {
    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() * 0.5
}
fn quantile(mut values: Vec<(f64, f64)>, q: f64) -> f64 {
    values.retain(|(x, w)| x.is_finite() && *w > 0.);
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = values.iter().map(|v| v.1).sum::<f64>() * q;
    let mut sum = 0.;
    for (x, w) in &values {
        sum += w;
        if sum >= target {
            return *x;
        }
    }
    values.last().map_or(0., |v| v.0)
}
fn measure(vertices: &[V3], faces: &[[usize; 3]], path: &Path) -> serde_json::Value {
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let uv: Vec<[[f64; 2]; 3]> = serde_json::from_value(data["uv"].clone()).unwrap();
    assert_eq!(uv.len(), faces.len());
    let mut total_uv = 0.;
    let mut total_3d = 0.;
    let mut collapsed_area = 0.;
    let mut outside = 0;
    let mut angular = Vec::new();
    let mut stretches = Vec::new();
    let mut edges = BTreeMap::<(usize, usize), Vec<(V2, V2)>>::new();
    for (i, f) in faces.iter().enumerate() {
        assert!(uv[i].iter().flatten().all(|x| x.is_finite()));
        if uv[i].iter().flatten().any(|x| *x < -1e-5 || *x > 1.00001) {
            outside += 1;
        }
        for c in 0..3 {
            let d = (c + 1) % 3;
            let (a, b, ua, ub) = if f[c] < f[d] {
                (f[c], f[d], uv[i][c], uv[i][d])
            } else {
                (f[d], f[c], uv[i][d], uv[i][c])
            };
            edges.entry((a, b)).or_default().push((ua, ub));
        }
        let e = sub(vertices[f[1]], vertices[f[0]]);
        let t = sub(vertices[f[2]], vertices[f[0]]);
        let a = len(e);
        let x = dot(t, e) / a;
        let h = (dot(t, t) - x * x).max(0.).sqrt();
        let a3 = a * h * 0.5;
        let a2 = area(uv[i][0], uv[i][1], uv[i][2]);
        total_3d += a3;
        total_uv += a2;
        if a3 <= 1e-24 {
            continue;
        }
        if a2 < 1e-14 {
            collapsed_area += a3;
            continue;
        }
        let u = [uv[i][1][0] - uv[i][0][0], uv[i][1][1] - uv[i][0][1]];
        let v = [uv[i][2][0] - uv[i][0][0], uv[i][2][1] - uv[i][0][1]];
        let j00 = u[0] / a;
        let j10 = u[1] / a;
        let j01 = (v[0] - u[0] * x / a) / h;
        let j11 = (v[1] - u[1] * x / a) / h;
        let g00 = j00 * j00 + j10 * j10;
        let g11 = j01 * j01 + j11 * j11;
        let g01 = j00 * j01 + j10 * j11;
        let trace = g00 + g11;
        let disc = ((g00 - g11).powi(2) + 4. * g01 * g01).sqrt();
        let hi = ((trace + disc) * 0.5).sqrt();
        let lo = ((trace - disc) * 0.5).max(0.).sqrt();
        if lo > 1e-20 {
            angular.push((hi / lo, a3));
            stretches.push((hi, lo, a3));
        }
    }
    let mut seam_edges = 0;
    let mut seam_length = 0.;
    for ((a, b), pairs) in edges {
        let close = |p: V2, q: V2| (p[0] - q[0]).abs() <= 1e-6 && (p[1] - q[1]).abs() <= 1e-6;
        if pairs.len() == 1
            || pairs
                .iter()
                .any(|p| !close(p.0, pairs[0].0) || !close(p.1, pairs[0].1))
        {
            seam_edges += 1;
            seam_length += len(sub(vertices[a], vertices[b]));
        }
    }
    let scale = (total_uv / total_3d).sqrt();
    let stretch: Vec<_> = stretches
        .into_iter()
        .map(|(hi, lo, w)| ((hi / scale).max(scale / lo), w))
        .collect();
    serde_json::json!({"triangles":uv.len(),"charts_reported":data["charts"],"uv_area":total_uv,"surface_area":total_3d,
  "collapsed_surface_fraction":collapsed_area/total_3d,"outside_faces":outside,"seam_edges":seam_edges,"seam_length":seam_length,
  "angular_p50":quantile(angular.clone(),0.5),"angular_p95":quantile(angular.clone(),0.95),"angular_p99":quantile(angular,0.99),
  "normalized_stretch_p95":quantile(stretch,0.95)})
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let req: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let vertices: Vec<V3> = serde_json::from_value(req["positions"].clone()).unwrap();
    let faces: Vec<[usize; 3]> = serde_json::from_value(req["triangles"].clone()).unwrap();
    let a = measure(&vertices, &faces, Path::new(&args[2]));
    let b = measure(&vertices, &faces, Path::new(&args[3]));
    let report = serde_json::json!({"reference":a,"native":b});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if args.len() > 4 {
        std::fs::write(&args[4], serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
