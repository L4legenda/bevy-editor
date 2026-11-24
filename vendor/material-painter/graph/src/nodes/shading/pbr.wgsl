// @name PBR
// @category Looks
// @description The physically based look of the 3D view. Unconnected inputs use the painted surface.
// @shading
// @output color
// @input base: color = 0.5, 0.5, 0.5
// @input roughness: gray = 0.5
// @input metallic: gray = 0

fn node(c: Shade) -> vec4<f32> {
    var s = c.s;
    if (has_base()) {
        s.albedo = in_base(c).rgb;
    }
    if (has_roughness()) {
        s.rough = in_roughness(c);
    }
    if (has_metallic()) {
        s.metal = in_metallic(c);
    }
    return vec4<f32>(shade_pbr(s, c.n, c.v, c.shadow), 1.0);
}
