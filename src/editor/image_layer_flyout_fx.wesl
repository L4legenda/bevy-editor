import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_ui_render::ui::sd_inset_rounded_box;

@group(1) @binding(0) var<uniform> motion: vec4<f32>;
@group(1) @binding(1) var<uniform> panel: vec4<f32>;
@group(1) @binding(2) var<uniform> accent: vec4<f32>;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let density = max(motion.z, 0.5);
    let point = (in.uv - vec2<f32>(0.5)) * in.size;
    let distance = sd_inset_rounded_box(point, in.size, in.border_radius_x,
        in.border_radius_y, vec4<f32>(0.55 * density)) / density;
    let coverage = 1.0 - smoothstep(-0.6, 0.55, distance);
    let edge = exp(-abs(distance) / 0.9);
    let travel = mix(-0.25, 1.45, motion.x);
    let glint = exp(-pow((in.uv.y + (1.0 - in.uv.x) * 0.2 - travel) / 0.16, 2.0));
    let sweep = glint * sin(motion.x * 3.14159);
    let top_light = (1.0 - in.uv.y) * 0.005;
    let body = panel.rgb + vec3<f32>(top_light) + accent.rgb * sweep * 0.035;
    let rim = mix(vec3<f32>(0.070, 0.064, 0.074), accent.rgb, sweep * 0.67);
    return vec4<f32>(mix(body, rim, edge), coverage * motion.y);
}
