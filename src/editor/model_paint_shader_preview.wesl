import bevy_pbr::render::{
    clustered_forward,
    forward_io::{VertexOutput, FragmentOutput},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::PbrInput,
    shadows::fetch_directional_shadow,
};
@if(ENVIRONMENT_MAP)
import bevy_pbr::{
    light_probe::environment_map::compute_radiances,
    render::pbr_lighting::LayerLightingInput,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> cp: array<vec4<f32>, 128>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var maps_a: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var maps_b: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var maps_e: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var height_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var maps_sampler: sampler;

var<private> scene_input: PbrInput;

fn shade_pbr(s: Surface, n: vec3<f32>, v: vec3<f32>, shadow: f32) -> vec3<f32> {
    var input = scene_input;
    input.N = n;
    input.V = v;
    input.material.base_color = vec4<f32>(s.albedo, input.material.base_color.a);
    input.material.perceptual_roughness = s.rough;
    input.material.metallic = s.metal;
    return apply_pbr_lighting(input).rgb;
}

fn view_depth(position: vec4<f32>) -> f32 {
    return (view_bindings::view.view_from_world * position).z;
}

@if(ENVIRONMENT_MAP)
fn scene_radiances(n: vec3<f32>, direction: vec3<f32>, roughness: f32) -> mat2x3<f32> {
    let index = clustered_forward::view_fragment_cluster_index(
        scene_input.frag_coord.xy, view_depth(scene_input.world_position), scene_input.is_orthographic,
    );
    var ranges = clustered_forward::unpack_clusterable_object_index_ranges(index);
    let layer = LayerLightingInput(n, direction, max(dot(n, scene_input.V), 0.001), roughness, roughness * roughness);
    let radiances = compute_radiances(layer, &ranges, scene_input.world_position.xyz, false);
    return mat2x3<f32>(radiances.irradiance, radiances.radiance);
}
@else
fn scene_radiances(n: vec3<f32>, direction: vec3<f32>, roughness: f32) -> mat2x3<f32> {
    return mat2x3<f32>(vec3<f32>(0.0), vec3<f32>(0.0));
}

fn irradiance(n: vec3<f32>) -> vec3<f32> {
    return scene_radiances(n, n, 1.0)[0] + PI * view_bindings::lights.ambient_color.rgb;
}

fn env(direction: vec3<f32>, roughness: f32) -> vec3<f32> {
    return scene_radiances(scene_input.N, direction, roughness)[1];
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var input = pbr_input_from_standard_material(in, front);
    input.material.base_color = alpha_discard(
        input.material.flags, input.material.alpha_cutoff, input.material.base_color,
    );
    scene_input = input;
    var uv = vec2<f32>(0.0);
    @if(VERTEX_UVS_A)
    uv = in.uv;

    var c: Shade;
    c.s = Surface(
        input.material.base_color.rgb,
        input.material.perceptual_roughness,
        input.material.metallic,
        textureSampleLevel(height_map, maps_sampler, uv, 0.0).r * 2.0 - 1.0,
        textureSampleLevel(maps_a, maps_sampler, uv, 0.0),
        textureSampleLevel(maps_b, maps_sampler, uv, 0.0),
        textureSampleLevel(maps_e, maps_sampler, uv, 0.0),
    );
    c.n = input.N;
    c.gn = input.world_normal;
    c.v = input.V;
    c.uv = uv;
    c.wpos = input.world_position.xyz;
    c.l = vec3<f32>(0.0, 1.0, 0.0);
    c.light = vec3<f32>(0.0);
    c.shadow = 1.0;
    if (view_bindings::lights.n_directional_lights > 0u) {
        let light = view_bindings::lights.directional_lights[0];
        c.l = light.direction_to_light;
        c.light = light.color.rgb;
        if ((input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && (light.flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            c.shadow = fetch_directional_shadow(
                0u, input.world_position, input.world_normal, view_depth(input.world_position), input.frag_coord.xy,
            );
        }
    }
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(
        input, vec4<f32>(custom_shade(c), input.material.base_color.a),
    );
    return out;
}
