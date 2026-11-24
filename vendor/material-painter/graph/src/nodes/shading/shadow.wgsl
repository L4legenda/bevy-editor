// @name Shadow
// @category Lighting
// @description 1 in the key light, 0 in its shadow.
// @shading
// @output gray

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.shadow);
}
