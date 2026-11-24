// @name Unlit
// @category Looks
// @description A color as is, with no lighting: what the painted color looks like flat.
// @shading
// @output color
// @input color: color = 0.8, 0.8, 0.8

fn node(c: Shade) -> vec4<f32> {
    if (has_color()) {
        return vec4<f32>(in_color(c).rgb, 1.0);
    }
    return vec4<f32>(c.s.albedo, 1.0);
}
