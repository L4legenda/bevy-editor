// @name Invert
// @category Adjust
// @description One minus the input.
// @output color
// @input input: color = 0

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = in_input(uv);
    return vec4<f32>(1.0 - c.rgb, c.a);
}
