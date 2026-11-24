// @name Edge Detect
// @category Filters
// @description Bright lines where the gray input changes quickly.
// @output gray
// @input input: gray = 0.5
// @param width: float(0.5, 16) = 1.5
// @param strength: float(0, 16) = 4

fn node(uv: vec2<f32>) -> vec4<f32> {
    let e = px() * params.width;
    let gx = in_input(uv + vec2<f32>(e, 0.0)) - in_input(uv - vec2<f32>(e, 0.0));
    let gy = in_input(uv + vec2<f32>(0.0, e)) - in_input(uv - vec2<f32>(0.0, e));
    return vec4<f32>(clamp(length(vec2<f32>(gx, gy)) * params.strength, 0.0, 1.0));
}
