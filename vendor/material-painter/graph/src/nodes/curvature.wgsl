// @name Curvature
// @category Filters
// @description Ridges of a height map go bright, creases go dark, flat stays mid gray.
// @output gray
// @input height: gray = 0.5
// @param width: float(0.5, 16) = 2
// @param intensity: float(0, 32) = 6

fn node(uv: vec2<f32>) -> vec4<f32> {
    let e = px() * params.width;
    let h = in_height(uv);
    let sum = in_height(uv + vec2<f32>(e, 0.0)) + in_height(uv - vec2<f32>(e, 0.0)) + in_height(uv + vec2<f32>(0.0, e)) + in_height(uv - vec2<f32>(0.0, e));
    // A peak sits above its neighbors: convex reads bright.
    let k = h * 4.0 - sum;
    return vec4<f32>(clamp(0.5 + k * params.intensity, 0.0, 1.0));
}
