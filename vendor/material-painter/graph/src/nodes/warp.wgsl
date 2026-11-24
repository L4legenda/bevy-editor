// @name Warp
// @category Filters
// @description Pushes the input along the slopes of a gray warp map.
// @output color
// @input input: color = 0.5
// @input warp: gray = 0.5
// @param intensity: float(0, 1) = 0.1

fn node(uv: vec2<f32>) -> vec4<f32> {
    let e = px() * 2.0;
    let gx = in_warp(uv + vec2<f32>(e, 0.0)) - in_warp(uv - vec2<f32>(e, 0.0));
    let gy = in_warp(uv + vec2<f32>(0.0, e)) - in_warp(uv - vec2<f32>(0.0, e));
    return in_input(uv + vec2<f32>(gx, gy) * params.intensity * 8.0);
}
