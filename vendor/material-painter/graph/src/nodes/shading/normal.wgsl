// @name Normal
// @category Lighting
// @description The shading normal (with baked normal and painted height), or the plain mesh normal, as a color.
// @shading
// @output color
// @param mesh_only: bool = false
// @param as_color: bool = true

fn node(c: Shade) -> vec4<f32> {
    var n = c.n;
    if (params.mesh_only) {
        n = c.gn;
    }
    if (params.as_color) {
        return vec4<f32>(n * 0.5 + 0.5, 1.0);
    }
    return vec4<f32>(n, 1.0);
}
