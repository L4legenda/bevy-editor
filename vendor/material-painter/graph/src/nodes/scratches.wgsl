// @name Scratches
// @category Generators
// @description Thin random scratches, tileable.
// @output gray
// @param count: int(1, 400) = 80
// @param length: float(0.01, 1) = 0.2
// @param width: float(0.2, 8) = 1
// @param angle: float(-180, 180) = 30
// @param spread: float(0, 1) = 0.5
// @param seed: int(0, 1000) = 0

fn node(uv: vec2<f32>) -> vec4<f32> {
    var v = 0.0;
    let w = params.width * px();
    for (var i = 0; i < params.count; i++) {
        let h = hash2(vec2<f32>(f32(i), 3.0), f32(params.seed));
        let h2 = hash2(vec2<f32>(f32(i), 9.0), f32(params.seed));
        let a = radians(params.angle) + (h2.x - 0.5) * PI * params.spread;
        let dir = vec2<f32>(cos(a), sin(a));
        let len = params.length * (0.4 + 0.6 * h2.y);
        // Nearest wrapped copy of the segment's center.
        var d = uv - h;
        d -= round(d);
        let t = clamp(dot(d, dir), -len * 0.5, len * 0.5);
        let dist = length(d - dir * t);
        let fade = 1.0 - abs(t) / (len * 0.5);
        v = max(v, (1.0 - smoothstep(0.0, w, dist)) * sqrt(max(fade, 0.0)) * (0.5 + 0.5 * h2.x));
    }
    return vec4<f32>(v);
}
