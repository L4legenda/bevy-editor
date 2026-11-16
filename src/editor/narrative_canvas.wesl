import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> view: vec4<f32>;

// Quiet orbital drafting surface: the grid follows the graph, the nebula drifts.
@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let pixel = in.uv * in.size;
    let zoom = max(view.z, 0.01);
    let world = (pixel - view.xy) / zoom;
    let grid = abs(fract(world / 48.0 + vec2<f32>(0.5)) - vec2<f32>(0.5)) * 48.0 * zoom;
    let dot = 1.0 - smoothstep(0.65, 1.35, length(grid));
    let t = globals.time * 0.025;
    let uv = in.uv;
    let gold = exp(-length((uv - vec2<f32>(0.16 + sin(t) * 0.06, 0.25)) * vec2<f32>(2.4, 3.4)) * 3.0);
    let blue = exp(-length((uv - vec2<f32>(0.83, 0.70 + cos(t) * 0.05)) * vec2<f32>(2.1, 2.6)) * 3.0);
    let color = vec3<f32>(0.002, 0.003, 0.006)
        + vec3<f32>(0.040, 0.023, 0.010) * gold
        + vec3<f32>(0.006, 0.012, 0.028) * blue
        + vec3<f32>(0.035, 0.037, 0.050) * dot * 0.35;
    return vec4<f32>(color, 0.90);
}
