// @name UV
// @category Surface
// @description Texture coordinates as red and green.
// @shading
// @output color

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(c.uv, 0.0, 1.0);
}
