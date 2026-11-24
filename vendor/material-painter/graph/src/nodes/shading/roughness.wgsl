// @name Roughness
// @category Surface
// @description The painted roughness.
// @shading
// @output gray

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.s.rough);
}
