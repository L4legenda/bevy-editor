// Helpers every graph node can call. Noises take an integer `scale` (cells
// across the texture) and wrap at it, so every output tiles seamlessly.

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;

// One texel in uv.
fn px() -> f32 {
    return 1.0 / res();
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn rotate2(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

fn hash2(p: vec2<f32>, seed: f32) -> vec2<f32> {
    let h = pcg3(bitcast<vec3<u32>>(vec3<i32>(vec3<f32>(floor(p), floor(seed)))));
    return vec2<f32>(h.xy) * (1.0 / 4294967295.0);
}

fn hash1(p: vec2<f32>, seed: f32) -> f32 {
    return hash2(p, seed).x;
}

// Lattice cell `c` wrapped to a period of `n` cells.
fn wrap_cell(c: vec2<f32>, n: f32) -> vec2<f32> {
    return c - n * floor(c / n);
}

// Tileable value noise in 0..1.
fn value_noise(uv: vec2<f32>, scale: f32, seed: f32) -> f32 {
    let n = max(round(scale), 1.0);
    let p = uv * n;
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = hash1(wrap_cell(i, n), seed);
    let b = hash1(wrap_cell(i + vec2<f32>(1.0, 0.0), n), seed);
    let c = hash1(wrap_cell(i + vec2<f32>(0.0, 1.0), n), seed);
    let d = hash1(wrap_cell(i + vec2<f32>(1.0, 1.0), n), seed);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

fn grad_dot(c: vec2<f32>, f: vec2<f32>, seed: f32) -> f32 {
    let a = hash1(c, seed + 17.0) * TAU;
    return dot(vec2<f32>(cos(a), sin(a)), f);
}

// Tileable gradient (Perlin) noise in about 0..1.
fn perlin(uv: vec2<f32>, scale: f32, seed: f32) -> f32 {
    let n = max(round(scale), 1.0);
    let p = uv * n;
    let i = floor(p);
    let f = fract(p);
    let w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = grad_dot(wrap_cell(i, n), f, seed);
    let b = grad_dot(wrap_cell(i + vec2<f32>(1.0, 0.0), n), f - vec2<f32>(1.0, 0.0), seed);
    let c = grad_dot(wrap_cell(i + vec2<f32>(0.0, 1.0), n), f - vec2<f32>(0.0, 1.0), seed);
    let d = grad_dot(wrap_cell(i + vec2<f32>(1.0, 1.0), n), f - vec2<f32>(1.0, 1.0), seed);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y) * 0.7071 + 0.5;
}

// Fractal sum of tileable Perlin noise: each octave doubles the scale and
// scales the amplitude by `roughness`. Normalized to 0..1.
fn fbm2(uv: vec2<f32>, scale: f32, octaves: i32, roughness: f32, seed: f32) -> f32 {
    var sum = 0.0;
    var amp = 1.0;
    var total = 0.0;
    var s = max(round(scale), 1.0);
    for (var o = 0; o < octaves; o++) {
        sum += perlin(uv, s, seed + f32(o) * 31.0) * amp;
        total += amp;
        amp *= roughness;
        s *= 2.0;
    }
    return sum / max(total, 1e-4);
}

// Tileable Worley cells: x = distance to the nearest feature point (in
// cells), y = distance to the second nearest, z = a random value per cell.
fn cells(uv: vec2<f32>, scale: f32, jitter: f32, seed: f32) -> vec3<f32> {
    let n = max(round(scale), 1.0);
    let p = uv * n;
    let i = floor(p);
    let f = fract(p);
    var d1 = 8.0;
    var d2 = 8.0;
    var id = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2<f32>(f32(x), f32(y));
            let c = wrap_cell(i + o, n);
            let h = hash2(c, seed);
            let pt = o + 0.5 + (h - 0.5) * jitter;
            let d = distance(f, pt);
            if (d < d1) {
                d2 = d1;
                d1 = d;
                id = hash1(c, seed + 7.0);
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    return vec3<f32>(d1, d2, id);
}
