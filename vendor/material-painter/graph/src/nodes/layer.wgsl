// @name Layer
// @category Material
// @description One channel of a material layer laid over the stack below it: a value (or a tri-planar image) with fractal and per-part variation, blended in through a mask. Chain them per channel; smart materials are made of these.
// @surface
// @output color
// @input below: color = 0
// @input mask: gray = 1
// @param channel: enum(Base Color, Roughness, Metallic, Height, Emission) = Base Color
// @param color: color = 0.8, 0.8, 0.8
// @param value: float(-1, 1) = 0.5
// @param variation: float(0, 1) = 0
// @param variation_scale: float(0.1, 100) = 4
// @param part_variation: float(0, 1) = 0
// @param seed: float(0, 100) = 0
// @param blend: enum(Normal, Multiply, Screen, Overlay, Add, Subtract, Darken, Lighten) = Normal
// @param opacity: float(0, 1) = 1
// @param image: image
// @param tiling: float(0.05, 32) = 1
// @param sharpness: float(1, 16) = 4

fn node(uv: vec2<f32>) -> vec4<f32> {
    let ch = params.channel;
    // Unconnected, the stack starts where a texture set does.
    var below = in_below(uv).rgb;
    if ((u32(nu.info.y) & 1u) == 0u) {
        below = vec3<f32>(select(0.0, select(0.5, 0.75, ch == 0), ch <= 1));
    }
    let s = surface(uv);
    // Color channels take the color; the others the value.
    var v = select(vec3<f32>(params.value), params.color, ch == 0 || ch == 4);
    // An image (a scan) replaces it: a 1x1 slot is the empty one.
    if (textureDimensions(t_img0).x > 1u) {
        let t = triplanar(t_img0, s, params.tiling, params.sharpness);
        if (ch == 0 || ch == 4) {
            v = t.rgb;
        } else if (ch == 3) {
            // Mid grey is the surface; the value scales it.
            v = vec3<f32>((t.r - 0.5) * 2.0 * params.value);
        } else {
            v = t.rrr;
        }
    }
    // Variation as fill layers do it: color multiplied, roughness and height added.
    let kind = select(select(-1, 3, ch == 3), ch, ch <= 1);
    v = fill_variation(v, kind, s.p, params.variation, params.variation_scale, params.seed, params.part_variation, s.b.a);
    let m = clamp(in_mask(uv), 0.0, 1.0) * params.opacity;
    var out = mix(below, blend3(params.blend, below, v), m);
    if (ch != 3) {
        out = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(out, 1.0);
}
