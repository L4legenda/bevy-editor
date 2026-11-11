import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_ui_render::ui::sd_inset_rounded_box;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Eased hover, eased press, seconds since last press, effect strength.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
// Current cursor UV, last press origin UV.
@group(1) @binding(1) var<uniform> pointer: vec4<f32>;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let hover = clamp(state.x, 0.0, 1.0);
    let press = clamp(state.y, 0.0, 1.0);
    let strength = clamp(state.w, 0.0, 1.0);
    let ripple_active = select(0.0, 1.0, state.z >= 0.0 && state.z < 1.2);
    let age = max(state.z, 0.0);
    let ripple_envelope = ripple_active * smoothstep(0.0, 0.055, age)
        * (1.0 - smoothstep(0.16, 0.86, age));
    if strength <= 0.0 || max(max(hover, press), ripple_envelope) <= 0.0001 {
        return vec4<f32>(0.0);
    }

    let size = max(in.size, vec2<f32>(1.0));
    let point = (in.uv - vec2<f32>(0.5)) * size;
    // The material pipeline supplies physical pixel borders in L/T/R/B
    // order. Native elliptical clipping keeps the effect inside that border.
    let inset = max(in.border_widths, vec4<f32>(0.0)) + vec4<f32>(0.6);
    let distance_to_edge = sd_inset_rounded_box(
        point,
        size,
        in.border_radius_x,
        in.border_radius_y,
        inset,
    );
    let clip = 1.0 - smoothstep(-0.8, 0.25, distance_to_edge);
    if clip <= 0.0 {
        return vec4<f32>(0.0);
    }

    let scale = max(min(size.x, size.y), 1.0);
    let position = point / scale;
    let cursor_uv = clamp(pointer.xy, vec2<f32>(0.0), vec2<f32>(1.0));
    let cursor = (cursor_uv - vec2<f32>(0.5)) * size / scale;
    let cursor_delta = position - cursor;
    let cursor_glow = exp(-dot(cursor_delta, cursor_delta) / 0.44);
    let time = globals.time;

    // Slow, broad folds feel like moving silk rather than a sweeping flash.
    let flow = position.x * 2.3 + position.y * 3.8 - time * 0.43
        + sin(position.y * 4.2 + time * 0.24) * 0.38;
    let fold = sin(flow);
    let cross_fold = cos(position.x * 1.7 - position.y * 4.1 + time * 0.31);
    let pearl = 0.5 + 0.5 * sin(fold * 2.1 + cross_fold * 0.65 + time * 0.16);
    let amethyst = vec3<f32>(0.37, 0.32, 0.67);
    let rose = vec3<f32>(0.72, 0.36, 0.46);
    let champagne = vec3<f32>(1.0, 0.83, 0.58);
    let ivory = vec3<f32>(1.0, 0.93, 0.82);
    let silk = mix(mix(amethyst, rose, pearl), champagne, cursor_glow * 0.5);
    let sheen = exp(-(fold - 0.18) * (fold - 0.18) * 22.0);
    let filament = exp(-(fold + 0.48) * (fold + 0.48) * 230.0);
    let breathing = 0.91 + 0.09 * sin(time * 1.15 + position.x * 0.4);

    let inner_edge = exp(-max(-distance_to_edge, 0.0) / 1.35);
    let edge_lustre = inner_edge * (0.3 + 0.7 * cursor_glow);
    let hover_alpha = hover * (1.0 - press * 0.32) * breathing;
    let silk_alpha = hover_alpha * (0.025 + cursor_glow * 0.064);
    let sheen_alpha = hover_alpha * (sheen * 0.026 + filament * 0.016)
        * (0.35 + cursor_glow * 0.65);
    let edge_alpha = (hover * 0.14 + press * 0.11) * edge_lustre;

    // A press settles gently into plum while a thin pearly wave travels out
    // from the actual touch point. All distances remain aspect-correct.
    let press_origin = clamp(pointer.zw, vec2<f32>(0.0), vec2<f32>(1.0));
    let ripple_distance = length((in.uv - press_origin) * size);
    let ripple_progress = clamp(age / 0.86, 0.0, 1.0);
    let ripple_radius = length(size)
        * (1.0 - (1.0 - ripple_progress) * (1.0 - ripple_progress));
    let ripple_width = max(scale * 0.025, 1.6);
    let ripple_delta = (ripple_distance - ripple_radius) / ripple_width;
    let ripple_ring = exp(-ripple_delta * ripple_delta * 1.4);
    let ripple_alpha = ripple_ring * ripple_envelope * 0.17;
    let sink_alpha = press * (0.048 + cursor_glow * 0.026);

    let total_alpha = silk_alpha + sheen_alpha + edge_alpha + ripple_alpha + sink_alpha;
    let light = silk * silk_alpha
        + ivory * sheen_alpha
        + mix(champagne, ivory, 0.55) * edge_alpha
        + ivory * ripple_alpha
        + vec3<f32>(0.17, 0.075, 0.13) * sink_alpha;
    // The UI pipeline uses straight alpha; labels render above this material.
    return vec4<f32>(
        light / max(total_alpha, 0.00001),
        min(total_alpha, 0.34) * strength * clip,
    );
}
