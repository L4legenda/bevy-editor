// @name Noise
// @category Generators
// @description Tileable fractal noise: smooth clouds, soft value noise, billowy turbulence or sharp ridges.
// @output gray
// @param style: enum(Perlin, Value, Turbulence, Ridged) = Perlin
// @param scale: int(1, 128) = 6
// @param octaves: int(1, 10) = 5
// @param roughness: float(0, 1) = 0.5
// @param seed: int(0, 1000) = 0

fn octave(uv: vec2<f32>, s: f32, seed: f32) -> f32 {
    switch params.style {
        case 1: { return value_noise(uv, s, seed); }
        case 2: { return abs(perlin(uv, s, seed) * 2.0 - 1.0); }
        case 3: { let r = 1.0 - abs(perlin(uv, s, seed) * 2.0 - 1.0); return r * r; }
        default: { return perlin(uv, s, seed); }
    }
}

fn node(uv: vec2<f32>) -> vec4<f32> {
    var sum = 0.0;
    var amp = 1.0;
    var total = 0.0;
    var s = f32(params.scale);
    for (var o = 0; o < params.octaves; o++) {
        sum += octave(uv, s, f32(params.seed) + f32(o) * 31.0) * amp;
        total += amp;
        amp *= params.roughness;
        s *= 2.0;
    }
    return vec4<f32>(sum / max(total, 1e-4));
}
