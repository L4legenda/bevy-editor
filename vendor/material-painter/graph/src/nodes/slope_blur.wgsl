// @name Slope Blur
// @category Filters
// @description Smears the input down (or up) the slopes of a gray map: eroded, weathered edges.
// @output color
// @input input: color = 0.5
// @input slope: gray = 0.5
// @param intensity: float(0, 1) = 0.1
// @param uphill: bool = false

fn node(uv: vec2<f32>) -> vec4<f32> {
    let e = px() * 2.0;
    var sum = vec4<f32>(0.0);
    var p = uv;
    let dir = select(-1.0, 1.0, params.uphill);
    for (var i = 0; i < 12; i++) {
        sum += in_input(p);
        let g = vec2<f32>(in_slope(p + vec2<f32>(e, 0.0)) - in_slope(p - vec2<f32>(e, 0.0)), in_slope(p + vec2<f32>(0.0, e)) - in_slope(p - vec2<f32>(0.0, e)));
        p += normalize(g + vec2<f32>(1e-6)) * dir * params.intensity * 0.02;
    }
    return sum / 12.0;
}
