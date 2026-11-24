// @name Metallic
// @category Surface
// @description The painted metalness.
// @shading
// @output gray

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.s.metal);
}
