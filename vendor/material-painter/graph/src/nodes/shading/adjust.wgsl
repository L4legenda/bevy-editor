// @name Adjust Color
// @category Math
// @description Brightness and saturation.
// @shading
// @output color
// @input color: color = 0.5, 0.5, 0.5
// @param brightness: float(0, 4) = 1
// @param saturation: float(0, 2) = 1

fn node(c: Shade) -> vec4<f32> {
    let col = in_color(c).rgb * params.brightness;
    let grey = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(mix(vec3<f32>(grey), col, params.saturation), 1.0);
}
