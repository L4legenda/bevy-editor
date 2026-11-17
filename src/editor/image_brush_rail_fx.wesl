import bevy_ui_render::ui_vertex_output::UiVertexOutput;

// Panel / handle / fill, eased interaction, pixel density, reveal opacity.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
@group(1) @binding(1) var<uniform> tint: vec4<f32>;

fn round_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let density = max(state.z, 0.5);
    let size = max(in.size / density, vec2<f32>(1.0));
    let p = (in.uv - vec2<f32>(0.5)) * size;
    let emphasis = clamp(state.y, 0.0, 1.0);
    if state.x < 0.5 {
        let d = round_box(p, size * 0.5 - vec2<f32>(0.7), 12.0);
        let mask = 1.0 - smoothstep(-0.6, 0.8, d);
        let rim = exp(-abs(d + 1.3) * 1.45);
        let bevel = exp(-abs(d + 3.6) * 1.2);
        // Only the fine gold frame stands out; the opaque body uses CHROME.
        let gold = vec3<f32>(0.94, 0.66, 0.29);
        return vec4<f32>(tint.rgb + gold * (rim * 0.25 + bevel * 0.008), mask * state.w);
    }
    let radius = min(select(3.5, 3.0, state.x > 1.5), min(size.x, size.y) * 0.5);
    let d = round_box(p, size * 0.5 - vec2<f32>(0.4), max(radius - 0.4, 0.0));
    let mask = 1.0 - smoothstep(-0.5, 0.5, d);
    // A restrained neutral hover response replaces all animated highlights.
    return vec4<f32>(tint.rgb * (1.0 + emphasis * 0.12), mask * state.w);
}
