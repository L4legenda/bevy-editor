// @name Fresnel
// @category Lighting
// @description Bright where the surface turns away from the camera: rims and edges.
// @shading
// @output gray
// @param power: float(0.5, 10) = 4

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(pow(1.0 - clamp(dot(c.n, c.v), 0.0, 1.0), params.power));
}
