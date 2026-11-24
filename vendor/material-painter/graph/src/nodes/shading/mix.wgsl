// @name Mix
// @category Math
// @description Blends from A to B by a mask.
// @shading
// @output color
// @input a: color = 0, 0, 0
// @input b: color = 1, 1, 1
// @input mask: gray = 0.5

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(mix(in_a(c).rgb, in_b(c).rgb, clamp(in_mask(c), 0.0, 1.0)), 1.0);
}
