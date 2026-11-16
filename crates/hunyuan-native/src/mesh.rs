//! Native surface extraction, topology-preserving QEM reduction and self-contained GLB/OBJ.
use crate::tensor::check_cancel;
use anyhow::{Result, ensure};
use fast_surface_nets::{SurfaceNetsBuffer, surface_nets};
use ndshape::RuntimeShape;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashSet},
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::AtomicBool,
};

#[derive(Clone)]
pub struct Mesh {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
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
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn normal(p: [[f64; 3]; 3]) -> [f64; 3] {
    cross(sub(p[1], p[0]), sub(p[2], p[0]))
}
impl Mesh {
    pub fn extract(field: &[f32], res: u32) -> Result<Self> {
        let n = res + 1;
        ensure!(field.len() == (n as usize).pow(3), "Invalid occupancy grid");
        let mut buffer = SurfaceNetsBuffer::default();
        surface_nets(
            field,
            &RuntimeShape::<u32, 3>::new([n, n, n]),
            [0, 0, 0],
            [res, res, res],
            &mut buffer,
        );
        let vertices = buffer
            .positions
            .into_iter()
            .map(|p| p.map(|v| v / res as f32 * 2.02 - 1.01))
            .collect();
        let faces = buffer
            .indices
            .chunks_exact(3)
            .map(|f| [f[0], f[1], f[2]])
            .collect();
        let mut mesh = Self { vertices, faces };
        ensure!(
            !mesh.faces.is_empty(),
            "Модель не создала поверхность. Попробуйте другой seed или изображение."
        );
        let volume: f64 = mesh
            .faces
            .iter()
            .map(|f| {
                let p = f.map(|i| mesh.vertices[i as usize].map(f64::from));
                dot(p[0], cross(p[1], p[2]))
            })
            .sum();
        if volume < 0. {
            for f in &mut mesh.faces {
                f.swap(1, 2);
            }
        }
        Ok(mesh)
    }
    fn normals(&self) -> Vec<[f32; 3]> {
        let mut normals = vec![[0f64; 3]; self.vertices.len()];
        for f in &self.faces {
            let n = normal(f.map(|i| self.vertices[i as usize].map(f64::from)));
            for &i in f {
                for a in 0..3 {
                    normals[i as usize][a] += n[a];
                }
            }
        }
        normals
            .into_iter()
            .map(|n| {
                let len = dot(n, n).sqrt();
                if len > 1e-30 {
                    n.map(|v| (v / len) as f32)
                } else {
                    [0., 1., 0.]
                }
            })
            .collect()
    }
    pub fn glb(&self, path: &Path) -> Result<()> {
        let normals = self.normals();
        let mut data = Vec::with_capacity(self.vertices.len() * 24 + self.faces.len() * 12);
        for p in &self.vertices {
            for v in p {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
        for p in &normals {
            for v in p {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
        for f in &self.faces {
            for v in f {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
        let count = self.vertices.len();
        let pos_bytes = count * 12;
        let bounds = |min: bool| {
            std::array::from_fn::<_, 3, _>(|a| {
                self.vertices.iter().map(|p| p[a]).fold(
                    if min {
                        f32::INFINITY
                    } else {
                        f32::NEG_INFINITY
                    },
                    |x, y| if min { x.min(y) } else { x.max(y) },
                )
            })
        };
        let json = serde_json::json!({"asset":{"version":"2.0","generator":"Bevy Editor / Hunyuan3D-2mini Candle"},
            "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2,"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.65,0.67,0.7,1.0],"metallicFactor":0.0,"roughnessFactor":0.75},"doubleSided":true}],
            "buffers":[{"byteLength":data.len()}],"bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":pos_bytes,"target":34962},
                {"buffer":0,"byteOffset":pos_bytes,"byteLength":pos_bytes,"target":34962},
                {"buffer":0,"byteOffset":pos_bytes*2,"byteLength":self.faces.len()*12,"target":34963}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":count,"type":"VEC3","min":bounds(true),"max":bounds(false)},
                {"bufferView":1,"componentType":5126,"count":count,"type":"VEC3"},
                {"bufferView":2,"componentType":5125,"count":self.faces.len()*3,"type":"SCALAR"}]});
        let mut json = serde_json::to_vec(&json)?;
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let mut file = BufWriter::new(std::fs::File::create(path)?);
        file.write_all(b"glTF")?;
        file.write_all(&2u32.to_le_bytes())?;
        file.write_all(&((12 + 8 + json.len() + 8 + data.len()) as u32).to_le_bytes())?;
        file.write_all(&(json.len() as u32).to_le_bytes())?;
        file.write_all(b"JSON")?;
        file.write_all(&json)?;
        file.write_all(&(data.len() as u32).to_le_bytes())?;
        file.write_all(b"BIN\0")?;
        file.write_all(&data)?;
        file.flush()?;
        Ok(())
    }
    pub fn obj(&self, path: &Path) -> Result<()> {
        let mut out = BufWriter::new(std::fs::File::create(path)?);
        for p in &self.vertices {
            writeln!(out, "v {} {} {}", p[0], p[1], p[2])?;
        }
        for p in self.normals() {
            writeln!(out, "vn {} {} {}", p[0], p[1], p[2])?;
        }
        for f in &self.faces {
            writeln!(
                out,
                "f {0}//{0} {1}//{1} {2}//{2}",
                f[0] + 1,
                f[1] + 1,
                f[2] + 1
            )?;
        }
        out.flush()?;
        Ok(())
    }
}

type Quadric = [f64; 10];
fn add(a: Quadric, b: Quadric) -> Quadric {
    std::array::from_fn(|i| a[i] + b[i])
}
fn evaluate(q: Quadric, p: [f64; 3]) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x
        + 2. * q[1] * x * y
        + 2. * q[2] * x * z
        + 2. * q[3] * x
        + q[4] * y * y
        + 2. * q[5] * y * z
        + 2. * q[6] * y
        + q[7] * z * z
        + 2. * q[8] * z
        + q[9]
}
fn optimum(q: Quadric, a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    let r0 = [q[0], q[1], q[2]];
    let r1 = [q[1], q[4], q[5]];
    let r2 = [q[2], q[5], q[7]];
    let det = dot(r0, cross(r1, r2));
    let mid = std::array::from_fn(|i| (a[i] + b[i]) * 0.5);
    let mut candidates = vec![a, b, mid];
    if det.abs() > 1e-14 {
        let rhs = [-q[3], -q[6], -q[8]];
        let p = [
            dot(rhs, cross(r1, r2)) / det,
            dot(r0, cross(rhs, r2)) / det,
            dot(r0, cross(r1, rhs)) / det,
        ];
        // Avoid spikes far outside this edge's neighborhood on nearly singular patches.
        if dot(sub(p, mid), sub(p, mid)) < dot(sub(a, b), sub(a, b)) * 4. {
            candidates.push(p);
        }
    }
    candidates
        .into_iter()
        .min_by(|a, b| evaluate(q, *a).total_cmp(&evaluate(q, *b)))
        .unwrap()
}
#[derive(Clone)]
struct Edge {
    cost: f64,
    a: usize,
    b: usize,
    va: u32,
    vb: u32,
    p: [f64; 3],
}
impl PartialEq for Edge {
    fn eq(&self, o: &Self) -> bool {
        self.cost == o.cost
    }
}
impl Eq for Edge {}
impl PartialOrd for Edge {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Edge {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

pub fn simplify(
    mesh: &mut Mesh,
    target: usize,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f32, &str),
) -> Result<()> {
    if mesh.faces.len() <= target {
        return Ok(());
    }
    let mut p: Vec<_> = mesh.vertices.iter().map(|v| v.map(f64::from)).collect();
    let mut q = vec![[0.; 10]; p.len()];
    let mut version = vec![0u32; p.len()];
    let mut adjacent = vec![Vec::<usize>::new(); p.len()];
    let mut alive = vec![true; mesh.faces.len()];
    for (i, f) in mesh.faces.iter().enumerate() {
        let points = f.map(|v| p[v as usize]);
        let mut n = normal(points);
        let area = dot(n, n).sqrt();
        if area < 1e-25 {
            alive[i] = false;
            continue;
        }
        n = n.map(|v| v / area);
        let d = -dot(n, points[0]);
        let [x, y, z] = n;
        let plane = [
            x * x,
            x * y,
            x * z,
            x * d,
            y * y,
            y * z,
            y * d,
            z * z,
            z * d,
            d * d,
        ]
        .map(|v| v * area);
        for &v in f {
            q[v as usize] = add(q[v as usize], plane);
            adjacent[v as usize].push(i);
        }
    }
    let edge = |a: usize, b: usize, q: &[Quadric], p: &[[f64; 3]], version: &[u32]| {
        let quad = add(q[a], q[b]);
        let pos = optimum(quad, p[a], p[b]);
        Edge {
            cost: evaluate(quad, pos).max(0.),
            a,
            b,
            va: version[a],
            vb: version[b],
            p: pos,
        }
    };
    let mut queue = BinaryHeap::new();
    let mut edges = HashSet::new();
    for f in &mesh.faces {
        for k in 0..3 {
            let a = f[k].min(f[(k + 1) % 3]) as usize;
            let b = f[k].max(f[(k + 1) % 3]) as usize;
            if edges.insert((a, b)) {
                queue.push(edge(a, b, &q, &p, &version));
            }
        }
    }
    drop(edges);
    let original = mesh.faces.len();
    let mut remaining = alive.iter().filter(|&&v| v).count();
    let mut collapsed = 0;
    while remaining > target {
        if collapsed % 256 == 0 {
            check_cancel(cancelled)?;
            progress(
                (original - remaining) as f32 / (original - target) as f32,
                "Оптимизация сетки",
            );
        }
        let Some(e) = queue.pop() else { break };
        let (a, b) = (e.a, e.b);
        if version[a] != e.va
            || version[b] != e.vb
            || adjacent[a].is_empty()
            || adjacent[b].is_empty()
        {
            continue;
        }
        let mut faces = adjacent[a].clone();
        faces.extend_from_slice(&adjacent[b]);
        faces.sort_unstable();
        faces.dedup();
        faces.retain(|&i| alive[i]);
        let mut na = HashSet::new();
        let mut nb = HashSet::new();
        let mut common_faces = 0;
        for &i in &faces {
            let f = mesh.faces[i];
            if f.contains(&(a as u32)) {
                na.extend(f.into_iter().map(|v| v as usize));
            }
            if f.contains(&(b as u32)) {
                nb.extend(f.into_iter().map(|v| v as usize));
            }
            if f.contains(&(a as u32)) && f.contains(&(b as u32)) {
                common_faces += 1;
            }
        }
        na.remove(&a);
        na.remove(&b);
        nb.remove(&a);
        nb.remove(&b);
        // The link condition prevents merging separate sheets and creating nonmanifold edges.
        if common_faces != 2 || na.intersection(&nb).count() != 2 {
            continue;
        }
        let valid = faces.iter().all(|&i| {
            let f = mesh.faces[i];
            if f.contains(&(a as u32)) && f.contains(&(b as u32)) {
                return true;
            }
            let old = normal(f.map(|v| p[v as usize]));
            let new = normal(f.map(|v| {
                if v as usize == a || v as usize == b {
                    e.p
                } else {
                    p[v as usize]
                }
            }));
            dot(new, new) > 1e-30 && dot(old, new) > 0.2 * (dot(old, old) * dot(new, new)).sqrt()
        });
        if !valid {
            continue;
        }
        p[a] = e.p;
        q[a] = add(q[a], q[b]);
        version[a] += 1;
        version[b] += 1;
        adjacent[a].clear();
        adjacent[b].clear();
        for i in faces {
            let f = &mut mesh.faces[i];
            for v in f.iter_mut() {
                if *v == b as u32 {
                    *v = a as u32;
                }
            }
            if f[0] == f[1] || f[0] == f[2] || f[1] == f[2] {
                alive[i] = false;
                remaining -= 1;
                continue;
            }
            adjacent[a].push(i);
            for &v in f.iter() {
                let v = v as usize;
                if v != a && !adjacent[v].contains(&i) {
                    adjacent[v].push(i);
                }
            }
        }
        let mut neighbors = na;
        neighbors.extend(nb);
        for v in neighbors {
            queue.push(edge(a.min(v), a.max(v), &q, &p, &version));
        }
        collapsed += 1;
        // Stale entries refer to previous vertex versions and can be discarded in bulk.
        if queue.len() > remaining * 16 {
            queue.retain(|e| version[e.a] == e.va && version[e.b] == e.vb);
        }
    }
    let mut remap = vec![u32::MAX; p.len()];
    let mut vertices = Vec::new();
    let mut faces = Vec::with_capacity(remaining);
    for (i, f) in mesh.faces.iter().enumerate() {
        if !alive[i] {
            continue;
        }
        let f = f.map(|v| {
            let r = &mut remap[v as usize];
            if *r == u32::MAX {
                *r = vertices.len() as u32;
                vertices.push(p[v as usize].map(|x| x as f32));
            }
            *r
        });
        faces.push(f);
    }
    mesh.vertices = vertices;
    mesh.faces = faces;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sphere() -> Mesh {
        let n = 25;
        let mut v = Vec::new();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    v.push(
                        ([x, y, z]
                            .map(|v| v as f32 / 24. * 2.02 - 1.01)
                            .into_iter()
                            .map(|v| v * v)
                            .sum::<f32>())
                        .sqrt()
                            - 0.7,
                    );
                }
            }
        }
        Mesh::extract(&v, 24).unwrap()
    }
    #[test]
    fn surface_and_decimation_remain_closed_and_outward() {
        let mut m = sphere();
        let old = m.faces.len();
        simplify(&mut m, 400, &AtomicBool::new(false), &|_, _| {}).unwrap();
        assert!(m.faces.len() <= 402 && m.faces.len() < old);
        let mut edges = std::collections::HashMap::new();
        for f in &m.faces {
            for i in 0..3 {
                let a = f[i];
                let b = f[(i + 1) % 3];
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&n| n == 2));
        assert!(
            m.normals()
                .iter()
                .zip(&m.vertices)
                .all(|(n, p)| dot(n.map(f64::from), p.map(f64::from)) > 0.)
        );
    }
    #[test]
    fn glb_has_valid_chunks_and_embedded_normals() {
        let m = sphere();
        let path =
            std::env::temp_dir().join(format!("hunyuan-native-mesh-{}.glb", std::process::id()));
        m.glb(&path).unwrap();
        let b = std::fs::read(&path).unwrap();
        assert_eq!(&b[0..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize,
            b.len()
        );
        let n = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let j: serde_json::Value = serde_json::from_slice(&b[20..20 + n]).unwrap();
        assert_eq!(j["meshes"][0]["primitives"][0]["attributes"]["NORMAL"], 1);
        std::fs::remove_file(path).unwrap();
    }
}
