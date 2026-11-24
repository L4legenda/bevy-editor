// @name Specular Highlight
// @category Lighting
// @description A Blinn-Phong highlight from the key light. Tightness comes from the painted roughness unless set by hand.
// @shading
// @output gray
// @param use_roughness: bool = true
// @param tightness: float(1, 512) = 64

fn node(c: Shade) -> vec4<f32> {
    let h = normalize(c.l + c.v);
    var power = params.tightness;
    if (params.use_roughness) {
        let r = clamp(c.s.rough, 0.03, 1.0);
        power = 2.0 / (r * r * r * r) - 2.0;
    }
    let spec = pow(max(dot(c.n, h), 0.0), max(power, 1.0)) * step(0.0, dot(c.n, c.l));
    return vec4<f32>(spec * c.shadow);
}
