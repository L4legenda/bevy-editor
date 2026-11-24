// @name ID Color
// @category Surface
// @description The baked ID map's color.
// @shading
// @output color

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.s.e.rgb, 1.0);
}
