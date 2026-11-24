// @name Base Color
// @category Surface
// @description The painted base color.
// @shading
// @output color

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.s.albedo, 1.0);
}
