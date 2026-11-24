// Shared helpers, prepended to every shader module.

struct FsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> FsOut {
    var out: FsOut;
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn pcg3(v_in: vec3<u32>) -> vec3<u32> {
    var v = v_in * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn hash33(p: vec3<f32>) -> vec3<f32> {
    let u = pcg3(bitcast<vec3<u32>>(vec3<i32>(floor(p))));
    return vec3<f32>(u) * (1.0 / 4294967295.0);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash33(i).x;
    let b = hash33(i + vec3<f32>(1.0, 0.0, 0.0)).x;
    let c = hash33(i + vec3<f32>(0.0, 1.0, 0.0)).x;
    let d = hash33(i + vec3<f32>(1.0, 1.0, 0.0)).x;
    let e = hash33(i + vec3<f32>(0.0, 0.0, 1.0)).x;
    let f1 = hash33(i + vec3<f32>(1.0, 0.0, 1.0)).x;
    let g = hash33(i + vec3<f32>(0.0, 1.0, 1.0)).x;
    let h = hash33(i + vec3<f32>(1.0, 1.0, 1.0)).x;
    return mix(
        mix(mix(a, b, u.x), mix(c, d, u.x), u.y),
        mix(mix(e, f1, u.x), mix(g, h, u.x), u.y),
        u.z
    );
}

fn fbm(p: vec3<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 5; i++) {
        s += a * vnoise(q);
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 3.1);
        a *= 0.5;
    }
    return s / 0.96875;
}

// Distance to the nearest jittered feature point (Worley / cellular noise).
fn cellular(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    var d = 8.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<f32>(f32(x), f32(y), f32(z));
                let r = o + hash33(i + o) - f;
                d = min(d, dot(r, r));
            }
        }
    }
    return sqrt(d);
}

// Equirectangular environment mapping; matches environment::uv_to_dir.
fn env_uv_to_dir(uv: vec2<f32>) -> vec3<f32> {
    let phi = (uv.x - 0.5) * 6.28318531;
    let theta = uv.y * 3.14159265;
    return vec3<f32>(sin(theta) * sin(phi), cos(theta), -sin(theta) * cos(phi));
}

fn env_dir_to_uv(d: vec3<f32>) -> vec2<f32> {
    let u = 0.5 + atan2(d.x, -d.z) / 6.28318531;
    let v = acos(clamp(d.y, -1.0, 1.0)) / 3.14159265;
    return vec2<f32>(u, v);
}

// Evaluates irradiance from 9 SH coefficients (cosine lobe already convolved).
fn sh_irradiance(sh: array<vec4<f32>, 9>, d: vec3<f32>) -> vec3<f32> {
    var c = sh[0].rgb * 0.282095;
    c += sh[1].rgb * 0.488603 * d.y;
    c += sh[2].rgb * 0.488603 * d.z;
    c += sh[3].rgb * 0.488603 * d.x;
    c += sh[4].rgb * 1.092548 * d.x * d.y;
    c += sh[5].rgb * 1.092548 * d.y * d.z;
    c += sh[6].rgb * 0.315392 * (3.0 * d.z * d.z - 1.0);
    c += sh[7].rgb * 1.092548 * d.x * d.z;
    c += sh[8].rgb * 0.546274 * (d.x * d.x - d.y * d.y);
    return max(c, vec3<f32>(0.0));
}
