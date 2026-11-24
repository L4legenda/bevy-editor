struct Triangle {
    a: vec2<f32>,
    b: vec2<f32>,
    c: vec2<f32>,
    _padding: vec2<f32>,
}

struct Band {
    size: u32,
    tiles: u32,
    first_row: u32,
    rows: u32,
}

@group(0) @binding(0) var<storage, read> triangles: array<Triangle>;
@group(0) @binding(1) var<storage, read> bins: array<u32>;
@group(0) @binding(2) var<storage, read_write> owners: array<u32>;
@group(0) @binding(3) var<uniform> band: Band;

const EMPTY: u32 = 0xffffffffu;
const CHECK_CPU: u32 = 0xfffffffeu;
const EDGE: f32 = -0.00001;

fn cross(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn owner(pixel: vec2<u32>) -> u32 {
    let tile = (pixel.y / 8u) * band.tiles + pixel.x / 8u;
    let center = vec2<f32>(pixel) + 0.5;
    for (var i = bins[tile]; i < bins[tile + 1]; i += 1u) {
        let id = bins[i];
        let t = triangles[id];
        let low = floor(min(t.a, min(t.b, t.c)));
        let high = ceil(max(t.a, max(t.b, t.c)));
        let xy = vec2<f32>(pixel);
        if any(xy < low) || any(xy >= high) {
            continue;
        }
        let a = t.b - t.a;
        let b = t.c - t.a;
        let q = center - t.a;
        let det = cross(a, b);
        let magnitude = max(abs(a.x * b.y), abs(a.y * b.x));
        if abs(det) <= max(1e-9, magnitude * 1e-5) {
            return CHECK_CPU;
        }
        let u = cross(q, b) / det;
        let v = cross(a, q) / det;
        let edge = min(1.0 - u - v, min(u, v));
        let error = (
            dot(abs(q), abs(b.yx)) + dot(abs(a), abs(q.yx))
            + magnitude * (abs(u) + abs(v))
        ) / abs(det) * 0.000002;
        // Resolve borderline hits with the existing CPU test to preserve ownership
        // and UV seams across GPU float implementations.
        if abs(edge - EDGE) <= max(0.00005, error) {
            return CHECK_CPU;
        }
        if edge >= EDGE {
            return id;
        }
    }
    return EMPTY;
}

@compute @workgroup_size(8, 8)
fn raster_uv(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < band.size && id.y < band.rows {
        owners[id.y * band.size + id.x] = owner(vec2<u32>(id.x, band.first_row + id.y));
    }
}
