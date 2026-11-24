// @name Posterize
// @category Math
// @description Snaps a color to a few steps.
// @shading
// @output color
// @input color: color = 0.5, 0.5, 0.5
// @param steps: int(2, 16) = 4

fn node(c: Shade) -> vec4<f32> {
    let n = f32(params.steps) - 1.0;
    return vec4<f32>(floor(in_color(c).rgb * n + 0.5) / n, 1.0);
}
