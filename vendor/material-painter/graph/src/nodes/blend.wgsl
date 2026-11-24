// @name Blend
// @category Blend
// @description Puts the foreground over the background through an optional mask.
// @output color
// @input foreground: color = 1
// @input background: color = 0
// @input mask: gray = 1
// @param mode: enum(Normal, Multiply, Screen, Overlay, Add, Subtract, Lighten, Darken, Soft Light) = Normal
// @param opacity: float(0, 1) = 1

fn overlay(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return select(1.0 - 2.0 * (1.0 - a) * (1.0 - b), 2.0 * a * b, a < vec3<f32>(0.5));
}

fn node(uv: vec2<f32>) -> vec4<f32> {
    let f = in_foreground(uv).rgb;
    let b = in_background(uv);
    var r: vec3<f32>;
    switch params.mode {
        case 1: { r = f * b.rgb; }
        case 2: { r = 1.0 - (1.0 - f) * (1.0 - b.rgb); }
        case 3: { r = overlay(b.rgb, f); }
        case 4: { r = b.rgb + f; }
        case 5: { r = b.rgb - f; }
        case 6: { r = max(b.rgb, f); }
        case 7: { r = min(b.rgb, f); }
        case 8: { r = (1.0 - 2.0 * f) * b.rgb * b.rgb + 2.0 * f * b.rgb; }
        default: { r = f; }
    }
    let m = clamp(in_mask(uv) * params.opacity, 0.0, 1.0);
    return vec4<f32>(mix(b.rgb, clamp(r, vec3<f32>(0.0), vec3<f32>(1.0)), m), b.a);
}
