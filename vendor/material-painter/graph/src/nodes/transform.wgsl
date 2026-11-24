// @name Transform
// @category Filters
// @description Repeats, turns, scales and moves the input. It wraps, so whole repeats stay tileable.
// @output color
// @input input: color = 0.5
// @param repeat: int(1, 32) = 1
// @param rotation: float(-180, 180) = 0
// @param scale: float(0.1, 8) = 1
// @param offset_x: float(-1, 1) = 0
// @param offset_y: float(-1, 1) = 0

fn node(uv: vec2<f32>) -> vec4<f32> {
    let q = rotate2(uv - 0.5, radians(params.rotation)) / params.scale + 0.5;
    return in_input(q * f32(params.repeat) - vec2<f32>(params.offset_x, params.offset_y));
}
