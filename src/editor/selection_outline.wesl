import bevy_ui_render::ui_vertex_output::UiVertexOutput;

@group(1) @binding(0) var mask: texture_2d<f32>;
@group(1) @binding(1) var mask_sampler: sampler;
@group(1) @binding(2) var<uniform> color: vec4<f32>;
@group(1) @binding(3) var<uniform> parameters: vec4<f32>;

const DIRECTIONS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.70710678, 0.70710678),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(-0.70710678, 0.70710678),
    vec2<f32>(-1.0, 0.0),
    vec2<f32>(-0.70710678, -0.70710678),
    vec2<f32>(0.0, -1.0),
    vec2<f32>(0.70710678, -0.70710678),
);

fn coverage(uv: vec2<f32>) -> f32 {
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return 0.0;
    }
    return clamp(textureSampleLevel(mask, mask_sampler, uv, 0.0).r, 0.0, 1.0);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let width = parameters.x;
    if width <= 0.0 {
        return vec4<f32>(0.0);
    }

    let texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(mask));
    let center = coverage(in.uv);
    let inner_radius = max(width - 0.5, 0.0);
    let outer_radius = width + 0.5;
    var inner = 0.0;
    var outer = 0.0;
    // Two half-pixel-separated rings provide a soft one-pixel fringe even for
    // a binary MSAA-off mask. The center excludes the object's filled area.
    for (var index = 0u; index < 8u; index += 1u) {
        let direction = DIRECTIONS[index] * texel;
        inner = max(inner, coverage(in.uv + direction * inner_radius));
        outer = max(outer, coverage(in.uv + direction * outer_radius));
    }
    let alpha = clamp(0.5 * (inner + outer) * (1.0 - center), 0.0, 1.0);
    return vec4<f32>(color.rgb, color.a * alpha);
}
