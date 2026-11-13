import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_ui_render::ui::sd_inset_rounded_box;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Hover, press, selection, availability.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
// Cursor and the most recent activation origin, in local UVs.
@group(1) @binding(1) var<uniform> cursor: vec4<f32>;
// Ripple age, primary action, keyboard focus, project card.
@group(1) @binding(2) var<uniform> details: vec4<f32>;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    if state.w < 0.01 { return vec4<f32>(0.0); }
    let size = max(in.size, vec2<f32>(1.0));
    let point = (in.uv - vec2<f32>(0.5)) * size;
    let distance = sd_inset_rounded_box(point, size, in.border_radius_x,
        in.border_radius_y, max(in.border_widths, vec4<f32>(0.0)) + vec4<f32>(0.4));
    let clip = 1.0 - smoothstep(-0.8, 0.3, distance);
    if clip <= 0.0 { return vec4<f32>(0.0); }
    let time = globals.time;
    let scale = max(min(size.x, size.y), 1.0);
    let delta = (in.uv - cursor.xy) * size / scale;
    let spotlight = exp(-dot(delta, delta) * 2.8);
    let gold = vec3<f32>(1.0, 0.78, 0.43);
    let violet = vec3<f32>(0.51, 0.43, 0.86);
    let ivory = vec3<f32>(1.0, 0.94, 0.80);
    let edge = exp(-max(-distance, 0.0) / 1.2);
    let phase = atan2(point.y, point.x) - time * 0.7;
    let runner = pow(0.5 + 0.5 * cos(phase), 7.0);
    let edge_alpha = edge * (state.z * (0.10 + 0.21 * runner)
        + state.x * (0.06 + 0.18 * spotlight) + details.z * 0.12);
    let glow_alpha = state.x * (0.018 + 0.11 * spotlight) * (1.0 - details.y * 0.5);
    let sweep_position = fract(time * 0.13) * 1.8 - 0.4;
    let sweep_delta = in.uv.x + in.uv.y * 0.22 - sweep_position;
    let sweep = exp(-sweep_delta * sweep_delta * 85.0);
    let sheen_alpha = details.y * (0.014 + sweep * 0.07) * (0.6 + state.x * 0.4);
    let age = clamp(details.x, 0.0, 0.85);
    let progress = age / 0.85;
    let radius = length(size) * (1.0 - pow(1.0 - progress, 2.0));
    let ripple_distance = length((in.uv - cursor.zw) * size);
    let wave = (ripple_distance - radius) / max(scale * 0.035, 1.8);
    let envelope = smoothstep(0.0, 0.035, age) * (1.0 - smoothstep(0.12, 0.75, age));
    let ripple_alpha = exp(-wave * wave) * envelope * 0.30;
    let press_alpha = state.y * 0.055;
    let alpha = glow_alpha + edge_alpha + sheen_alpha + ripple_alpha + press_alpha;
    let light = mix(violet, gold, spotlight * 0.65) * glow_alpha
        + mix(gold, ivory, runner) * edge_alpha
        + ivory * (sheen_alpha + ripple_alpha)
        + vec3<f32>(0.19, 0.10, 0.20) * press_alpha;
    return vec4<f32>(light / max(alpha, 0.00001), min(alpha, 0.46) * clip);
}
