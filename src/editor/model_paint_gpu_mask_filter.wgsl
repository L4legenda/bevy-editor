// The spatial passes and noise functions are adapted from Material Painter.
struct Filter {
    size: u32,
    kind: u32,
    region: u32,
    vertical: u32,
    params: vec4<f32>,
    margin: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: Filter;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read> table: array<u32>;
@group(0) @binding(3) var<storage, read_write> effective: array<u32>;
@group(0) @binding(4) var<storage, read> tiles: array<u32>;

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


fn raw_at(c: vec2<i32>) -> u32 {
    let size = vec2<i32>(i32(u.size));
    var p = vec2<u32>(clamp(c, vec2<i32>(0), size - 1));
    if (u.kind == 3u && u.params.y != 0.0) {
        p = vec2<u32>(((c % size) + size) % size);
    }
    return source[p.y * u.size + p.x] & 255u;
}

fn at(c: vec2<i32>) -> f32 {
    return f32(raw_at(c)) / 255.0;
}

fn bilinear(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let t = fract(p);
    return mix(
        mix(at(i), at(i + vec2<i32>(1, 0)), t.x),
        mix(at(i + vec2<i32>(0, 1)), at(i + vec2<i32>(1, 1)), t.x), t.y
    );
}

@compute @workgroup_size(8, 8)
fn filter_mask(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    var p = group.xy * 8u + local.xy;
    if (u.region == 1u) {
        let tile = group.y * 64u + group.x;
        if (tile >= tiles[3]) { return; }
        let id = tiles[4u + tile];
        let columns = (u.size + 7u) / 8u;
        p = vec2<u32>(id % columns, id / columns) * 8u + local.xy;
    }
    if (u.region == 2u) {
        if (tiles[0] == 0u) { return; }
        let gutter = vec2<u32>(u.margin.x);
        let min = vec2<u32>(((u.size + 7u) / 8u) * 8u) - vec2<u32>(tiles[2], tiles[3]);
        let max = vec2<u32>(tiles[0], tiles[1]);
        if (any(p + gutter < min) || any(p >= max + gutter)) { return; }
    }
    if (any(p >= vec2<u32>(u.size))) { return; }
    let index = p.y * u.size + p.x;
    let raw = source[index] & 255u;
    let s = f32(raw) / 255.0;
    let a = u.params.x;
    let b = u.params.y;
    let q = (vec2<f32>(p) + 0.5) * (b / f32(u.size));
    let c = vec2<i32>(p);
    var v = s;
    switch u.kind {
        case 3u: {
            let n = i32(a);
            let dir = select(vec2<i32>(1, 0), vec2<i32>(0, 1), u.vertical != 0u);
            v = 0.0;
            for (var k = -n; k <= n; k++) {
                v += at(c + k * dir) * bitcast<f32>(table[u32(k + n)]);
            }
        }
        case 4u: {
            let x = vec2<i32>(i32(b), 0);
            let y = vec2<i32>(0, i32(b));
            let ring = (at(c + x) + at(c - x) + at(c + y) + at(c - y)) * 0.25;
            v = s + a * (s - ring);
        }
        case 5u: {
            let seed = u.params.z;
            let o = vec2<f32>(
                fbm(vec3<f32>(q, 1.3 + seed)),
                fbm(vec3<f32>(q + vec2<f32>(5.2, 1.7), 7.1 + seed)),
            );
            v = bilinear(vec2<f32>(p) + (o - 0.5) * (a * f32(u.size)));
        }
        case 6u: {
            let n = smoothstep(0.35, 0.65, fbm(vec3<f32>(q, u.params.z)));
            v = s * (1.0 - a * (1.0 - n));
        }
        case 7u: {
            let l = f32(table[index]) / 255.0;
            var mixed = l;
            switch u32(a) {
                case 1u: { mixed = s * l; }
                case 2u: { mixed = s + l; }
                case 3u: { mixed = s - l; }
                case 4u: { mixed = max(s, l); }
                case 5u: { mixed = min(s, l); }
                default: {}
            }
            v = mix(s, mixed, b);
        }
        case 8u: {
            var multiplier = 255u;
            if (a != 0.0) { multiplier = table[index]; }
            let value = min(255u, (raw * multiplier + 127u) / 255u + ((source[index] >> 8u) & 255u));
            effective[index] = value * 0x00010101u | 0xff000000u;
            return;
        }
        case 9u: {
            let vertical = u.vertical != 0u;
            let dir = select(vec2<i32>(1, 0), vec2<i32>(0, 1), vertical);
            let threshold = u32(b);
            let radius = i32(a);
            let stage = u32(u.params.w);
            var neighborhood = 0u;
            for (var k = -1; k <= 1; k++) {
                let sample = raw_at(c + k * radius * dir);
                neighborhood |= select(select(2u, 1u, sample >= threshold), sample & 3u, stage != 0u);
                if (neighborhood == 3u) { break; }
            }
            let inside = select(raw >= threshold, (raw & 4u) != 0u, stage != 0u);
            var value = neighborhood | select(0u, 4u, inside);
            if (stage == 2u) {
                let side = u32(u.params.z);
                let on_outline = neighborhood == 3u &&
                    (side == 3u || (side == 1u && !inside) || (side == 2u && inside));
                value = select(0u, 255u, on_outline);
            }
            effective[index] = value * 0x00010101u | 0xff000000u;
            return;
        }
        default: {
            effective[index] = table[raw] * 0x00010101u | 0xff000000u;
            return;
        }
    }
    let value = u32(floor(clamp(v, 0.0, 1.0) * 255.0 + 0.5));
    effective[index] = value * 0x00010101u | 0xff000000u;
}
