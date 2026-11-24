// @name Value
// @category Inputs
// @description One number everywhere. Wire it into inputs and show it with ◇ as a material parameter, so one slider steers the graph.
// @output gray
// @param value: float(0, 1) = 0.5

fn node(uv: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(params.value);
}
