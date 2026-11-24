// @name Ambient Light
// @category Lighting
// @description Diffuse light from the environment (HDRI) around the surface.
// @shading
// @output color

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(irradiance(c.n) / PI, 1.0);
}
