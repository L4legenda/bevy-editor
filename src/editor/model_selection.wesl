import bevy_pbr::render::forward_io::{Vertex, VertexOutput, decompress_vertex};
import bevy_pbr::render::mesh_view_bindings::view;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: vec4<f32>;

fn overlay_depth(clip: vec4<f32>, perspective_bias: f32, pixels: f32) -> f32 {
    if view.clip_from_view[3].w > 0.5 {
        let world_pixel = 2.0 * params.x
            / (abs(view.clip_from_view[1].y) * max(view.viewport.w, 1.0));
        return min(clip.z + world_pixel * abs(view.clip_from_view[2].z) * pixels, clip.w);
    }
    return min(clip.z * perspective_bias, clip.w);
}

@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let v = decompress_vertex(input, input.instance_index);
    var out: VertexOutput;
    out.world_position = vec4<f32>(v.position, 1.0);
    out.world_normal = v.normal;
    out.uv = v.uv;
    out.color = v.color;
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = input.instance_index;
    var clip = view.clip_from_world * out.world_position;
    if v.color.w > 0.0 && clip.w > 0.0 {
        let viewport = max(view.viewport.zw, vec2<f32>(1.0));
        var offset = v.uv * 4.0;
        if v.color.w > 0.75 {
            let tangent = view.clip_from_world * vec4<f32>(v.normal, 0.0);
            let direction = (tangent.xy * clip.w - clip.xy * tangent.w) * viewport;
            let unit = direction / max(length(direction), 0.000001);
            offset = vec2<f32>(-unit.y, unit.x) * v.uv.x * 2.0;
        }
        clip = vec4<f32>(clip.xy + offset * params.x * 2.0 / viewport * clip.w,
            overlay_depth(clip, 1.012, 2.0), clip.w);
    }
    out.position = clip;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if in.color.w < 0.25 {
        return vec4<f32>(in.color.rgb, params.y);
    }
    let distance = select(length(in.uv) * 4.0, abs(in.uv.x) * 2.0, in.color.w > 0.75);
    let radius = select(2.55, 0.88, in.color.w > 0.75);
    let aa = max(fwidth(distance) * 0.5, 0.3);
    let coverage = 1.0 - smoothstep(radius - aa, radius + aa, distance);
    return vec4<f32>(in.color.rgb, coverage);
}
