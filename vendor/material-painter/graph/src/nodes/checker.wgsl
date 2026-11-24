// @name Checker
// @category Generators
// @description Checkerboard.
// @output gray
// @param count: int(1, 64) = 4

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = floor(uv * f32(params.count));
    return vec4<f32>(f32((i32(c.x) + i32(c.y)) & 1));
}
