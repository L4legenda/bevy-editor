// @name Uniform Color
// @category Generators
// @description A single flat color.
// @output color
// @param color: color = 0.5, 0.5, 0.5

fn node(uv: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(params.color, 1.0);
}
