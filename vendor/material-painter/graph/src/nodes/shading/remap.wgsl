// @name Remap
// @category Math
// @description Maps a range to 0..1 with a smooth step: thresholds and soft masks.
// @shading
// @output gray
// @input value: gray = 0.5
// @param low: float(0, 1) = 0.3
// @param high: float(0, 1) = 0.7
// @param invert: bool = false

fn node(c: Shade) -> vec4<f32> {
    var v = smoothstep(params.low, max(params.high, params.low + 1e-4), in_value(c));
    if (params.invert) {
        v = 1.0 - v;
    }
    return vec4<f32>(v);
}
