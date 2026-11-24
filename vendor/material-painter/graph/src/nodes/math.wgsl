// @name Math
// @category Adjust
// @description Combines two grays; B is the value while unconnected.
// @output gray
// @input a: gray = 0.5
// @input b: gray = 0.5
// @param op: enum(Add, Subtract, Multiply, Divide, Min, Max, Power, Difference) = Multiply
// @param value: float(0, 4) = 1
// @param clamp_result: bool = true

fn node(uv: vec2<f32>) -> vec4<f32> {
    let a = in_a(uv);
    var b = in_b(uv);
    if ((u32(nu.info.y) & 2u) == 0u) {
        b = params.value;
    }
    var r: f32;
    switch params.op {
        case 0: { r = a + b; }
        case 1: { r = a - b; }
        case 3: { r = a / max(b, 1e-4); }
        case 4: { r = min(a, b); }
        case 5: { r = max(a, b); }
        case 6: { r = pow(max(a, 0.0), b); }
        case 7: { r = abs(a - b); }
        default: { r = a * b; }
    }
    if (params.clamp_result) {
        r = clamp(r, 0.0, 1.0);
    }
    return vec4<f32>(r);
}
