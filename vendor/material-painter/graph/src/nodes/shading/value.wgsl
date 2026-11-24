// @name Value
// @category Math
// @description A fixed number.
// @shading
// @output gray
// @param value: float(-4, 4) = 1

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(params.value);
}
