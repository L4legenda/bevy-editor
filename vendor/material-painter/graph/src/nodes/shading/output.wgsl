// @name Shader Output
// @category Output
// @description What the 3D view shows (before exposure and tone mapping).
// @shading
// @output color
// @input color: color = 1, 0, 1

fn node(c: Shade) -> vec4<f32> {
    return in_color(c);
}
