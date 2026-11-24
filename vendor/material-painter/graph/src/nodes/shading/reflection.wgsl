// @name Reflection
// @category Lighting
// @description The environment reflected in the surface, blurred by roughness.
// @shading
// @output color
// @param use_roughness: bool = true
// @param roughness: float(0, 1) = 0.2

fn node(c: Shade) -> vec4<f32> {
    var r = params.roughness;
    if (params.use_roughness) {
        r = c.s.rough;
    }
    return vec4<f32>(env(reflect(-c.v, c.n), clamp(r, 0.03, 1.0)), 1.0);
}
