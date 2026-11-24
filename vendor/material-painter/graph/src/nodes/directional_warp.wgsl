// @name Directional Warp
// @category Filters
// @description Shifts the input one way by the brightness of a warp map: streaks, drips, smears.
// @output color
// @input input: color = 0.5
// @input warp: gray = 0.5
// @param intensity: float(0, 1) = 0.1
// @param angle: float(-180, 180) = 90

fn node(uv: vec2<f32>) -> vec4<f32> {
    let a = radians(params.angle);
    return in_input(uv + vec2<f32>(cos(a), sin(a)) * in_warp(uv) * params.intensity);
}
