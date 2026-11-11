// `constants` is the synthetic WESL module supplied by Bevy's material
// pipeline specialization. It is independent of the bevy_pbr shader library.
@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> mask_color: vec4<f32>;

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return mask_color;
}
