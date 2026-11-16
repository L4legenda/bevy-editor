import bevy_pbr::render::forward_io::{Vertex, decompress_vertex};
import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_clip};
import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh::morph_vertex;
import bevy_pbr::render::skinning::skin_model;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> tint: vec4<f32>;

@vertex
fn vertex(input: Vertex) -> @builtin(position) vec4<f32> {
    var vertex = decompress_vertex(input, input.instance_index);
    @if(MORPH_TARGETS)
    vertex = morph_vertex(vertex, input.instance_index);
    @if(SKINNED)
    let world_from_local = skin_model(vertex.joint_indices, vertex.joint_weights, input.instance_index);
    @else
    let world_from_local = get_world_from_local(input.instance_index);
    var clip = mesh_position_local_to_clip(
        world_from_local, vec4<f32>(vertex.position, 1.0));
    // Reversed-Z lift avoids coplanar flicker. Raster depth bias is forbidden
    // for LineList; moving clip depth leaves the source geometry untouched.
    if view.clip_from_view[3].w > 0.5 {
        // Lift by less than a screen pixel in the two-sided orthographic slab.
        let world_pixel = 2.0 / (abs(view.clip_from_view[1].y) * max(view.viewport.w, 1.0));
        clip.z = min(clip.z + world_pixel * abs(view.clip_from_view[2].z) * 0.75, clip.w);
    } else {
        clip.z = min(clip.z * 1.0008, clip.w);
    }
    return clip;
}

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return tint;
}
