// @name Shape
// @category Generators
// @description A soft-edged shape, repeated in a grid.
// @output gray
// @param shape: enum(Disc, Square, Diamond, Cone, Ring) = Disc
// @param size: float(0, 1) = 0.7
// @param softness: float(0, 1) = 0.05
// @param repeat: int(1, 32) = 1

fn node(uv: vec2<f32>) -> vec4<f32> {
    let q = (fract(uv * f32(params.repeat)) - 0.5) * 2.0;
    let s = max(params.size, 1e-4);
    var d: f32;
    switch params.shape {
        case 1: { d = max(abs(q.x), abs(q.y)); }
        case 2: { d = abs(q.x) + abs(q.y); }
        case 3: { return vec4<f32>(clamp(1.0 - length(q) / s, 0.0, 1.0)); }
        case 4: { d = abs(length(q) - s * 0.75) + s * 0.75; }
        default: { d = length(q); }
    }
    let soft = max(params.softness * s, 1e-4);
    return vec4<f32>(1.0 - smoothstep(s - soft, s, d));
}
