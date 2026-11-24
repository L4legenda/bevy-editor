// Pack live paint textures from their native resolutions on the GPU.
struct Pack {
    factor: vec4<f32>,
    options: vec4<f32>, // mode, opaque/flip, alpha cutoff, use texture
};
@group(0) @binding(0) var<uniform> pack: Pack;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var layer: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn pack_map(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(layer);
    if (any(id.xy >= size)) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    var c = textureSampleLevel(source, linear_sampler, uv, 0.0) * pack.factor;
    switch u32(pack.options.x) {
        case 0u: {
            if (pack.options.y > 0.5) {
                c.a = 1.0;
            } else if (pack.options.z >= 0.0) {
                c.a = select(0.0, 1.0, c.a >= pack.options.z);
            }
        }
        case 2u: {
            if (pack.options.w < 0.5) {
                c = vec4<f32>(0.5, 0.5, 1.0, 1.0);
            } else if (pack.options.y > 0.5) {
                c.g = 1.0 - c.g;
            }
        }
        case 5u: {
            c = vec4<f32>(pack.factor.r * pow(textureSampleLevel(source, linear_sampler, uv, 0.0).a, 2.0));
        }
        default: {}
    }
    textureStore(layer, vec2<i32>(id.xy), c);
}
