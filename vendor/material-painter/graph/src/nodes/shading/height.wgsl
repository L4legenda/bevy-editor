// @name Height
// @category Surface
// @description The painted height (0 is the surface).
// @shading
// @output gray

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.s.height);
}
