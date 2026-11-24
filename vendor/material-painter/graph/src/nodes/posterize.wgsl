// @name Posterize
// @category Adjust
// @description Snaps each channel to a few even steps, for a stylized, banded look.
// @output color
// @input input: color = 0.5
// @param steps: int(2, 16) = 4

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = in_input(uv);
    let n = f32(params.steps) - 1.0;
    return vec4<f32>(floor(c.rgb * n + 0.5) / n, c.a);
}
