// @name Color
// @category Math
// @description A fixed color.
// @shading
// @output color
// @param color: color = 1, 1, 1
// @param strength: float(0, 10) = 1

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(params.color * params.strength, 1.0);
}
