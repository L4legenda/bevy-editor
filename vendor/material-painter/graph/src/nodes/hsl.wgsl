// @name HSL
// @category Adjust
// @description Shifts hue, scales saturation and lightness.
// @output color
// @input input: color = 0.5
// @param hue: float(-0.5, 0.5) = 0
// @param saturation: float(0, 2) = 1
// @param lightness: float(-1, 1) = 0

fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.bg, k.wz), vec4<f32>(c.gb, k.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(p.xyw, c.r), vec4<f32>(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + 1e-10)), d / (q.x + 1e-10), q.x);
}

fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let p = abs(fract(c.xxx + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3<f32>(1.0), clamp(p - 1.0, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = in_input(uv);
    var h = rgb2hsv(c.rgb);
    h.x = fract(h.x + params.hue);
    h.y = clamp(h.y * params.saturation, 0.0, 1.0);
    var rgb = hsv2rgb(h);
    if (params.lightness >= 0.0) {
        rgb = mix(rgb, vec3<f32>(1.0), params.lightness);
    } else {
        rgb = mix(rgb, vec3<f32>(0.0), -params.lightness);
    }
    return vec4<f32>(rgb, c.a);
}
