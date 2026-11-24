// @name Directional Noise
// @category Generators
// @description Noise stretched along one axis: brushed metal, fibers, wood grain, rain streaks. Tileable.
// @output gray
// @param direction: enum(Horizontal, Vertical) = Horizontal
// @param scale: int(1, 64) = 4
// @param stretch: int(1, 64) = 16
// @param octaves: int(1, 8) = 4
// @param roughness: float(0, 1) = 0.55
// @param seed: int(0, 1000) = 0

// Gradient noise with its own period along each axis.
fn aniso(p_in: vec2<f32>, period: vec2<f32>, seed: f32) -> f32 {
    let p = p_in * period;
    let i = floor(p);
    let f = fract(p);
    let w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let c00 = i - period * floor(i / period);
    let c10 = (i + vec2<f32>(1.0, 0.0)) - period * floor((i + vec2<f32>(1.0, 0.0)) / period);
    let c01 = (i + vec2<f32>(0.0, 1.0)) - period * floor((i + vec2<f32>(0.0, 1.0)) / period);
    let c11 = (i + vec2<f32>(1.0, 1.0)) - period * floor((i + vec2<f32>(1.0, 1.0)) / period);
    let a = grad_dot(c00, f, seed);
    let b = grad_dot(c10, f - vec2<f32>(1.0, 0.0), seed);
    let c = grad_dot(c01, f - vec2<f32>(0.0, 1.0), seed);
    let d = grad_dot(c11, f - vec2<f32>(1.0, 1.0), seed);
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y) * 0.7071 + 0.5;
}

fn node(uv: vec2<f32>) -> vec4<f32> {
    // Few cells along the streaks, many across them.
    var period = vec2<f32>(f32(params.scale), f32(params.scale * params.stretch));
    if (params.direction == 1) {
        period = period.yx;
    }
    var sum = 0.0;
    var amp = 1.0;
    var total = 0.0;
    for (var o = 0; o < params.octaves; o++) {
        sum += aniso(uv, period, f32(params.seed) + f32(o) * 31.0) * amp;
        total += amp;
        amp *= params.roughness;
        period *= 2.0;
    }
    return vec4<f32>(sum / max(total, 1e-4));
}
