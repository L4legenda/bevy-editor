import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_ui_render::ui::sd_inset_rounded_box;

// Local seconds, appearance, pointer speed, physical pixels per logical pixel.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
// Linear accent RGB and surface kind: vacancy / card / group / edge / landing.
@group(1) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let density = max(state.w, 0.5);
    let size = max(in.size, vec2<f32>(1.0));
    let point = (in.uv - vec2<f32>(0.5)) * size;
    let distance = sd_inset_rounded_box(point, size, in.border_radius_x,
        in.border_radius_y, vec4<f32>(0.6 * density));
    let clip = 1.0 - smoothstep(-0.7 * density, 0.5 * density, distance);
    let edge = exp(-abs(distance + 1.0 * density) / (0.9 * density));
    let inner = max(-distance, 0.0) / density;
    let t = state.x;
    let accent = tint.rgb;
    // A broad glint travels along the rim, leaving the centre quiet and readable.
    let travel = fract(t * 0.16 + 0.12);
    let glint = exp(-pow((in.uv.x + in.uv.y * 0.18 - travel * 1.35) / 0.17, 2.0));
    let breathe = 0.94 + 0.06 * sin(t * 2.8);
    let rim = edge * (0.48 + glint * 0.38) * breathe;
    let recess = exp(-inner / 8.0);

    if tint.w > 3.5 {
        // A short inward-settling halo confirms a completed drop.
        let age = clamp(t / 0.32, 0.0, 1.0);
        let wave = exp(-pow((inner - (1.0-age)*9.0) / 2.8, 2.0));
        let fade = (1.0-age)*(1.0-age);
        return vec4<f32>(mix(accent, vec3<f32>(1.0), wave*0.16),
            (edge*0.6 + wave*0.18) * fade * clip);
    }
    if tint.w > 2.5 {
        return vec4<f32>(mix(accent, vec3<f32>(1.0), glint*0.3),
            (0.68 + glint*0.28)*state.y*clip);
    }
    if tint.w > 0.5 && tint.w < 1.5 {
        let sheen = exp(-pow((in.uv.x*0.85 + in.uv.y*0.3 - t*0.65 + 0.15)/0.18, 2.0));
        let body = mix(vec3<f32>(0.008,0.010,0.017), vec3<f32>(0.020,0.017,0.026), 1.0-in.uv.y);
        let light = accent * (rim*0.24 + sheen*0.025 + state.z*0.009);
        let alpha = 0.98 * clip * state.y;
        return vec4<f32>(body + light, alpha);
    }
    // The gap is mostly transparent: a cool recess, fine rim and warm moving light.
    let fill = 0.014 + recess*0.012;
    let alpha = (fill + rim*0.67) * clip * state.y;
    let rgb = mix(accent*0.32, mix(accent, vec3<f32>(1.0), glint*0.16),
        clamp(rim / max(fill+rim, 0.001), 0.0, 1.0));
    return vec4<f32>(rgb, alpha);
}
