// @name Diffuse Light
// @category Lighting
// @description How much the key light faces the surface, with its shadow. Wrap softens the terminator (0.5 is half-Lambert).
// @shading
// @output gray
// @param wrap: float(0, 1) = 0
// @param shadows: bool = true

fn node(c: Shade) -> vec4<f32> {
    let nl = (dot(c.n, c.l) + params.wrap) / (1.0 + params.wrap);
    var lit = clamp(nl, 0.0, 1.0);
    if (params.shadows) {
        lit = lit * c.shadow;
    }
    return vec4<f32>(lit);
}
