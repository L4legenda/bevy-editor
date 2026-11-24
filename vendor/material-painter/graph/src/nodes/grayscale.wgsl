// @name Grayscale
// @category Adjust
// @description Color to gray: luminance, or one channel.
// @output gray
// @input input: color = 0.5
// @param channel: enum(Luminance, Red, Green, Blue, Alpha) = Luminance

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = in_input(uv);
    switch params.channel {
        case 1: { return vec4<f32>(c.r); }
        case 2: { return vec4<f32>(c.g); }
        case 3: { return vec4<f32>(c.b); }
        case 4: { return vec4<f32>(c.a); }
        default: { return vec4<f32>(luma(c.rgb)); }
    }
}
