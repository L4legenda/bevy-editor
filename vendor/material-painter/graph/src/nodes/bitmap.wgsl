// @name Bitmap
// @category Inputs
// @description An image file, optionally repeated.
// @output color
// @param image: image
// @param repeat: int(1, 32) = 1

fn node(uv: vec2<f32>) -> vec4<f32> {
    return img_image(uv * f32(params.repeat));
}
