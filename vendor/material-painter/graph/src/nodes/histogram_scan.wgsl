// @name Histogram Scan
// @category Adjust
// @description Turns a gradient into a mask: everything above the position goes white, with an adjustable soft edge.
// @output gray
// @input input: gray = 0.5
// @param position: float(0, 1) = 0.5
// @param contrast: float(0, 1) = 0.8
// @param invert: bool = false

fn node(uv: vec2<f32>) -> vec4<f32> {
    let w = max((1.0 - params.contrast) * 0.5, 1e-4);
    var g = smoothstep(params.position - w, params.position + w, in_input(uv));
    if (params.invert) {
        g = 1.0 - g;
    }
    return vec4<f32>(g);
}
