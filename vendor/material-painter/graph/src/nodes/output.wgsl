// @name Output
// @category Outputs
// @description Sends its input to a channel of the layer that uses this graph.
// @output color
// @input input: color = 0.5
// @param usage: enum(Base Color, Roughness, Metallic, Height, Mask, Opacity, Emission, Normal, Specular) = Base Color

fn node(uv: vec2<f32>) -> vec4<f32> {
    return in_input(uv);
}
