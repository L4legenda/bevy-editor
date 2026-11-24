// @name Gradient
// @category Generators
// @description Linear ramp from black to white, turned by an angle.
// @output gray
// @param angle: float(-180, 180) = 0
// @param mirror: bool = false
// @param repeat: int(1, 32) = 1

fn node(uv: vec2<f32>) -> vec4<f32> {
    let d = rotate2(uv - 0.5, radians(params.angle)).x + 0.5;
    var g = fract(d * f32(params.repeat));
    if (params.mirror) {
        g = 1.0 - abs(g * 2.0 - 1.0);
    }
    return vec4<f32>(g);
}
