import bevy_ui_render::ui_vertex_output::UiVertexOutput;

@group(1) @binding(0) var<uniform> state: vec4<f32>;
@group(1) @binding(1) var<uniform> low: vec4<f32>;
@group(1) @binding(2) var<uniform> high: vec4<f32>;

fn spectrum(h: f32) -> vec3<f32> {
    return clamp(abs(fract(h + vec3<f32>(0.0, 0.666667, 0.333333)) * 6.0 - 3.0) - 1.0,
        vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let size = in.size / max(state.w, 0.5);
    let x = in.uv.x;
    let y = abs(in.uv.y - 0.5) * size.y;
    let track = 1.0 - smoothstep(2.0, 2.8, y);
    let gold = vec3<f32>(0.70, 0.46, 0.22);
    let neutral = vec3<f32>(0.072, 0.075, 0.09);
    let below = min(state.x, state.y);
    let above = max(state.x, state.y);
    let px = 0.8 / max(size.x, 1.0);
    let amount = smoothstep(below - px, below + px, x) * (1.0 - smoothstep(above - px, above + px, x));
    var tint = neutral;
    if state.z > 1.5 {
        tint = mix(neutral, spectrum(x - 0.5), 0.42 + amount * 0.16);
    } else if state.z > 0.5 {
        tint = mix(neutral, mix(low.rgb, high.rgb, x), 0.66 + amount * 0.18);
    } else {
        tint = mix(neutral, gold, amount * 0.59);
    }
    var alpha = track;
    if state.y > 0.01 && state.y < 0.99 {
        let mark = (1.0 - smoothstep(0.5, 1.1, abs(x - state.y) * size.x))
            * (1.0 - smoothstep(4.0, 5.0, y));
        tint = mix(tint, vec3<f32>(0.61, 0.60, 0.62), mark);
        alpha = max(alpha, mark * 0.9);
    }
    return vec4<f32>(tint, alpha);
}
