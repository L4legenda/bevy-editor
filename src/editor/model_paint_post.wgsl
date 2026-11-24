// Adapted from Material Painter's post.wgsl, bloom.wgsl and common.wgsl.
// GPL-3.0-or-later. See vendor/material-painter/LICENSE.
struct FsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> FsOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return FsOut(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0), uv);
}

// Final viewport pass: HDR scene in, display image out. Lens effects and
// exposure work on scene-linear light; contrast, saturation, vignette and
// grain work on the tone-mapped image.

struct PostU {
    a: vec4<f32>,  // exposure EV, tone mapper, contrast, saturation
    b: vec4<f32>,  // temperature, tint, vignette, bloom strength
    c: vec4<f32>,  // chromatic aberration, grain, sharpen, unused
    d: vec4<f32>,  // texel size xy, aspect, unused
};

@group(0) @binding(0) var<uniform> pu: PostU;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var psamp: sampler;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Narkowicz's fit of the ACES filmic curve.
fn tm_aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Log-encoded sigmoid in the spirit of AgX: highlights desaturate toward
// white instead of clipping to pure hues.
fn tm_agx(c: vec3<f32>) -> vec3<f32> {
    let l = luminance(c);
    // Pull channels slightly together so bright saturated colors roll off.
    let inset = mix(vec3<f32>(l), c, 0.85);
    let lo = -10.0;
    let hi = 6.5;
    let x = clamp((log2(max(inset, vec3<f32>(1e-10))) - lo) / (hi - lo), vec3<f32>(0.0), vec3<f32>(1.0));
    let xp = pow(x, vec3<f32>(1.7));
    let y = xp / (xp + pow(1.0 - x, vec3<f32>(1.7)));
    // Give back some of the saturation the inset took.
    let yl = luminance(y);
    let out = max(mix(vec3<f32>(yl), y, 1.12), vec3<f32>(0.0));
    return pow(out, vec3<f32>(2.2));
}

// Khronos PBR Neutral: keeps base colors faithful, compresses only highlights.
fn tm_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    var c = c_in;
    let x = min(c.r, min(c.g, c.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    c -= offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c *= new_peak / peak;
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

// Hable's filmic curve.
fn hable(x: vec3<f32>) -> vec3<f32> {
    let a = 0.15;
    let b = 0.50;
    let c = 0.10;
    let d = 0.20;
    let e = 0.02;
    let f = 0.30;
    return ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
}

fn tm_filmic(c: vec3<f32>) -> vec3<f32> {
    let white = hable(vec3<f32>(11.2));
    return clamp(hable(c * 2.0) / white, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    switch u32(pu.a.y) {
        case 0u: { return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
        case 1u: { return c / (1.0 + c); }
        case 2u: { return clamp(c / (1.0 + luminance(c)), vec3<f32>(0.0), vec3<f32>(1.0)); }
        case 3u: { return tm_agx(c); }
        case 4u: { return clamp(tm_neutral(c), vec3<f32>(0.0), vec3<f32>(1.0)); }
        case 6u: { return tm_filmic(c); }
        default: { return tm_aces(c); }
    }
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

fn hash2(p: vec2<f32>) -> f32 {
    let u = pcg3(bitcast<vec3<u32>>(vec3<i32>(vec3<f32>(p, 17.0))));
    return f32(u.x) * (1.0 / 4294967295.0);
}

@fragment
fn grade(in: FsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let base = textureSampleLevel(hdr, psamp, uv, 0.0);
    let t = pu.d.xy;
    // Work on straight color; edges against a transparent background arrive premultiplied.
    let inv_a = select(0.0, 1.0 / base.a, base.a > 1e-4);
    var c = base.rgb * inv_a;

    // Chromatic aberration: red and blue pushed apart toward the edges.
    let ca = pu.c.x;
    if (ca > 0.0) {
        let off = (uv - 0.5) * ca * 0.012;
        c.r = textureSampleLevel(hdr, psamp, uv + off, 0.0).r * inv_a;
        c.b = textureSampleLevel(hdr, psamp, uv - off, 0.0).b * inv_a;
    }
    // Unsharp mask against the four neighbors.
    let sharpen = pu.c.z;
    if (sharpen > 0.0) {
        let n = textureSampleLevel(hdr, psamp, uv + vec2<f32>(t.x, 0.0), 0.0).rgb
            + textureSampleLevel(hdr, psamp, uv - vec2<f32>(t.x, 0.0), 0.0).rgb
            + textureSampleLevel(hdr, psamp, uv + vec2<f32>(0.0, t.y), 0.0).rgb
            + textureSampleLevel(hdr, psamp, uv - vec2<f32>(0.0, t.y), 0.0).rgb;
        c = max(c + (c - n * 0.25 * inv_a) * sharpen, vec3<f32>(0.0));
    }
    if (pu.b.w > 0.0) {
        c += textureSampleLevel(bloom_tex, psamp, uv, 0.0).rgb * pu.b.w;
    }
    c *= exp2(pu.a.x);

    // White balance: temperature along blue-amber, tint along green-magenta.
    let temp = pu.b.x;
    let tint = pu.b.y;
    c *= vec3<f32>(1.0 + 0.25 * temp, 1.0 - 0.2 * tint, 1.0 - 0.25 * temp);

    c = tonemap(max(c, vec3<f32>(0.0)));

    // Grading in a perceptual space.
    var p = pow(c, vec3<f32>(1.0 / 2.2));
    p = (p - 0.5) * pu.a.z + 0.5;
    let l = luminance(p);
    p = mix(vec3<f32>(l), p, pu.a.w);
    let v = (uv - 0.5) * vec2<f32>(pu.d.z, 1.0);
    p *= 1.0 - pu.b.z * smoothstep(0.25, 0.95, length(v) * 1.3);
    let grain = pu.c.y;
    if (grain > 0.0) {
        p += (hash2(in.pos.xy) - 0.5) * grain * 0.12;
    }
    c = pow(clamp(p, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    // Premultiplied out, like the scene came in.
    return vec4<f32>(c * base.a, base.a);
}

// Bloom: a downsample chain (soft threshold and firefly-safe averaging on the
// first step) followed by tent-filtered upsamples added back up the chain.

struct BloomU {
    p: vec4<f32>,  // source texel size xy, threshold, first pass (1) or not (0)
};

@group(0) @binding(4) var<uniform> bu: BloomU;
@group(0) @binding(5) var bsrc: texture_2d<f32>;
@group(0) @binding(6) var bsamp: sampler;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Keeps what is above the threshold, with a soft knee below it. Brightness
// is the strongest channel, not luminance: pure red or blue light (luminance
// 0.21 and 0.07 of white's) blooms as soon as white of the same peak does.
fn bright(c: vec3<f32>) -> vec3<f32> {
    let thr = bu.p.z;
    let knee = thr * 0.5 + 1e-4;
    let l = max(c.r, max(c.g, c.b));
    var soft = clamp(l - thr + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    return c * max(soft, l - thr) / max(l, 1e-4);
}

@fragment
fn down(in: FsOut) -> @location(0) vec4<f32> {
    let t = bu.p.xy;
    // Four bilinear taps cover a 4x4 texel footprint.
    var taps = array<vec3<f32>, 4>(
        textureSampleLevel(bsrc, bsamp, in.uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb,
        textureSampleLevel(bsrc, bsamp, in.uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb,
        textureSampleLevel(bsrc, bsamp, in.uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb,
        textureSampleLevel(bsrc, bsamp, in.uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb,
    );
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0; i < 4; i++) {
        var c = taps[i];
        var w = 1.0;
        if (bu.p.w > 0.5) {
            c = bright(c);
            // Weighting by inverse brightness stops single hot pixels from flickering.
            w = 1.0 / (1.0 + luma(c));
        }
        sum += c * w;
        wsum += w;
    }
    return vec4<f32>(sum / wsum, 1.0);
}

@fragment
fn up(in: FsOut) -> @location(0) vec4<f32> {
    let t = bu.p.xy;
    var c = textureSampleLevel(bsrc, bsamp, in.uv, 0.0).rgb * 4.0;
    c += textureSampleLevel(bsrc, bsamp, in.uv + vec2<f32>(t.x, 0.0), 0.0).rgb * 2.0;
    c += textureSampleLevel(bsrc, bsamp, in.uv - vec2<f32>(t.x, 0.0), 0.0).rgb * 2.0;
    c += textureSampleLevel(bsrc, bsamp, in.uv + vec2<f32>(0.0, t.y), 0.0).rgb * 2.0;
    c += textureSampleLevel(bsrc, bsamp, in.uv - vec2<f32>(0.0, t.y), 0.0).rgb * 2.0;
    c += textureSampleLevel(bsrc, bsamp, in.uv + t, 0.0).rgb;
    c += textureSampleLevel(bsrc, bsamp, in.uv - t, 0.0).rgb;
    c += textureSampleLevel(bsrc, bsamp, in.uv + vec2<f32>(t.x, -t.y), 0.0).rgb;
    c += textureSampleLevel(bsrc, bsamp, in.uv + vec2<f32>(-t.x, t.y), 0.0).rgb;
    return vec4<f32>(c / 16.0, 1.0);
}
