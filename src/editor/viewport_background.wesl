import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> top_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> bottom_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> violet: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> gold: vec4<f32>;

@vertex
fn vertex(@location(0) position: vec3<f32>) -> FullscreenVertexOutput {
    let uv = position.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    return FullscreenVertexOutput(vec4<f32>(position.xy, 0.0, 1.0), uv);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var color = mix(top_color.rgb, bottom_color.rgb, smoothstep(0.0, 1.0, uv.y));
    let cool_offset = (uv - vec2<f32>(0.69, 0.40)) * vec2<f32>(1.1, 2.0);
    let warm_offset = (uv - vec2<f32>(0.19, 0.74)) * vec2<f32>(1.5, 2.8);
    color = color + violet.rgb * exp(-dot(cool_offset, cool_offset) * 3.0) * 0.42;
    color = color + gold.rgb * exp(-dot(warm_offset, warm_offset) * 3.0) * 0.25;
    let edge = length((uv - vec2<f32>(0.5)) * vec2<f32>(1.0, 0.75));
    color = color * (1.0 - smoothstep(0.20, 0.72, edge) * 0.22);
    // A barely visible, static grain prevents banding in the dark gradients.
    let noise = fract(sin(dot(in.position.xy, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    return vec4<f32>(max(color + (noise - 0.5) * 0.00012, vec3<f32>(0.0)), 1.0);
}
