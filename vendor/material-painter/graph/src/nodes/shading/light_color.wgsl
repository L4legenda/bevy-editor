// @name Light Color
// @category Lighting
// @description The key light's color times its intensity (black when it's off).
// @shading
// @output color

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.light, 1.0);
}
