// @name Gradient Map
// @category Adjust
// @description Colors a grayscale input with a three-color ramp.
// @output color
// @input input: gray = 0.5
// @param dark: color = 0.08, 0.06, 0.05
// @param mid: color = 0.45, 0.32, 0.22
// @param light: color = 0.9, 0.85, 0.75
// @param mid_position: float(0.01, 0.99) = 0.5

fn node(uv: vec2<f32>) -> vec4<f32> {
    let g = clamp(in_input(uv), 0.0, 1.0);
    let m = params.mid_position;
    let lo = mix(params.dark, params.mid, g / m);
    let hi = mix(params.mid, params.light, (g - m) / (1.0 - m));
    return vec4<f32>(select(hi, lo, g < m), 1.0);
}
