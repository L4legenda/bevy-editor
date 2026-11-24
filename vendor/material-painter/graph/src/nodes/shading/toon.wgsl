// @name Toon
// @category Looks
// @description Cel shading: the light falls into flat bands, with a soft edge between them. Unconnected inputs use the painted color and the key light.
// @shading
// @output color
// @input base: color = 0.8, 0.8, 0.8
// @input light: gray = 1
// @param bands: int(1, 8) = 2
// @param softness: float(0, 0.5) = 0.04
// @param shadow_tint: color = 0.35, 0.38, 0.55
// @param ambient: float(0, 1) = 0.5

fn node(c: Shade) -> vec4<f32> {
    var base = c.s.albedo;
    if (has_base()) {
        base = in_base(c).rgb;
    }
    var lit = clamp(dot(c.n, c.l), 0.0, 1.0) * c.shadow;
    if (has_light()) {
        lit = in_light(c);
    }
    let n = f32(params.bands);
    let x = lit * n;
    // Flat bands, each stepping up softly just before the next boundary.
    let w = clamp(params.softness * n, 1e-3, 1.0);
    let level = clamp((floor(x) + smoothstep(1.0 - w, 1.0, fract(x))) / n, 0.0, 1.0);
    let dark = base * params.shadow_tint * params.ambient;
    // The key light's tint (white when the key light is off).
    let light = max(c.light, vec3<f32>(0.0));
    let lum = max(max(light.r, light.g), light.b);
    var tint = vec3<f32>(1.0);
    if (lum > 1e-3) {
        tint = light / lum;
    }
    return vec4<f32>(mix(dark, base * tint, level), 1.0);
}
