// @name Levels
// @category Adjust
// @description Remaps the input range: input black and white points, gamma (above 1 brightens), output range.
// @output color
// @input input: color = 0.5
// @param in_low: float(0, 1) = 0
// @param in_high: float(0, 1) = 1
// @param gamma: float(0.1, 4) = 1
// @param out_low: float(0, 1) = 0
// @param out_high: float(0, 1) = 1

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = in_input(uv);
    let t = clamp((c.rgb - params.in_low) / max(params.in_high - params.in_low, 1e-4), vec3<f32>(0.0), vec3<f32>(1.0));
    let g = pow(t, vec3<f32>(1.0 / max(params.gamma, 1e-3)));
    return vec4<f32>(mix(vec3<f32>(params.out_low), vec3<f32>(params.out_high), g), c.a);
}
