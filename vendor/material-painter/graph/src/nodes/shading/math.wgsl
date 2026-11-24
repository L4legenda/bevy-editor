// @name Math
// @category Math
// @description Combines two colors (or numbers) channel by channel.
// @shading
// @output color
// @input a: color = 0, 0, 0
// @input b: color = 1, 1, 1
// @param op: enum(Add, Subtract, Multiply, Divide, Min, Max, Power, Screen) = Multiply

fn node(c: Shade) -> vec4<f32> {
    let a = in_a(c).rgb;
    let b = in_b(c).rgb;
    var r: vec3<f32>;
    switch params.op {
        case 0: { r = a + b; }
        case 1: { r = a - b; }
        case 2: { r = a * b; }
        case 3: { r = a / max(abs(b), vec3<f32>(1e-4)) * sign(b + vec3<f32>(1e-9)); }
        case 4: { r = min(a, b); }
        case 5: { r = max(a, b); }
        case 6: { r = pow(max(a, vec3<f32>(0.0)), b); }
        default: { r = 1.0 - (1.0 - a) * (1.0 - b); }
    }
    return vec4<f32>(r, 1.0);
}
