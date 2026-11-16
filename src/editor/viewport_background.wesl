import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_pbr::render::forward_io::{Vertex, decompress_vertex};
import bevy_pbr::render::mesh_view_bindings::view;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> top_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> bottom_color: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> violet: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> gold: vec4<f32>;

@vertex
fn vertex(input: Vertex) -> FullscreenVertexOutput {
    let position = decompress_vertex(input, input.instance_index).position;
    let uv = position.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    return FullscreenVertexOutput(vec4<f32>(position.xy, 0.0, 1.0), uv);
}

fn drafting_lines(p: vec2<f32>, footprint: vec2<f32>, spacing: f32) -> f32 {
    let d = abs(fract(p / spacing - vec2<f32>(0.5)) - vec2<f32>(0.5)) * spacing / footprint;
    let lines = vec2<f32>(1.0) - smoothstep(vec2<f32>(0.3), vec2<f32>(1.05), d);
    return max(lines.x, lines.y);
}

fn drafting_background(uv: vec2<f32>) -> vec3<f32> {
    // Parallel rays intersect the dominant world plane, so panning keeps the
    // grid anchored to the model and each axis view gets the correct two axes.
    let near_h = view.world_from_clip * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 1.0, 1.0);
    let origin = near_h.xyz / near_h.w;
    let back = view.world_from_view[2].xyz;
    let direction = abs(back);
    var p: vec2<f32>;
    var horizontal: vec3<f32>;
    var vertical: vec3<f32>;
    let red = vec3<f32>(0.46, 0.14, 0.11);
    let green = vec3<f32>(0.13, 0.38, 0.22);
    let blue = vec3<f32>(0.14, 0.25, 0.53);
    if direction.x >= direction.y && direction.x >= direction.z {
        p = (origin - back * (origin.x / back.x)).zy;
        horizontal = blue;
        vertical = green;
    } else if direction.y >= direction.z {
        p = (origin - back * (origin.y / back.y)).xz;
        horizontal = red;
        vertical = blue;
    } else {
        p = (origin - back * (origin.z / back.z)).xy;
        horizontal = red;
        vertical = green;
    }
    let footprint = max(fwidth(p), vec2<f32>(0.00000001));
    let pixel = max(footprint.x, footprint.y);
    let spacing = pow(10.0, floor(log2(pixel * 40.0) / 3.321928095));
    let fine_fade = smoothstep(4.0, 14.0, spacing / pixel);
    let major = drafting_lines(p, footprint, spacing * 10.0);
    let minor = drafting_lines(p, footprint, spacing) * (1.0 - major);
    var color = mix(vec3<f32>(0.030, 0.039, 0.060), vec3<f32>(0.052, 0.055, 0.079), uv.y);
    color = mix(color, vec3<f32>(0.16, 0.18, 0.24), minor * fine_fade * 0.16);
    color = mix(color, vec3<f32>(0.23, 0.25, 0.32), major * 0.24);
    let h = 1.0 - smoothstep(0.5, 1.35, abs(p.y) / footprint.y);
    let v = 1.0 - smoothstep(0.5, 1.35, abs(p.x) / footprint.x);
    color = mix(color, horizontal, h * 0.62);
    color = mix(color, vertical, v * 0.62);
    return color;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    if view.clip_from_view[3].w > 0.5 {
        return vec4<f32>(drafting_background(uv), 1.0);
    }
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
