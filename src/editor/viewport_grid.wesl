import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> parameters: vec4<f32>;

fn grid_lines(coordinates: vec2<f32>, footprint: vec2<f32>, spacing: f32) -> f32 {
    let position = coordinates / spacing;
    let derivative = max(footprint / spacing, vec2<f32>(0.00001));
    let distance = abs(fract(position - vec2<f32>(0.5)) - vec2<f32>(0.5)) / derivative;
    let coverage = vec2<f32>(1.0) - smoothstep(vec2<f32>(0.25), vec2<f32>(1.05), distance);
    // Fade each direction separately as its divisions become smaller than a
    // few screen pixels, rather than letting dense horizon lines shimmer.
    let resolution_fade = vec2<f32>(1.0)
        - smoothstep(vec2<f32>(0.15), vec2<f32>(0.65), derivative);
    let lines = coverage * resolution_fade;
    return max(lines.x, lines.y);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let position = in.world_position.xz;
    let footprint = max(fwidth(position), vec2<f32>(0.00001));
    let minor = grid_lines(position, footprint, parameters.x);
    let major = grid_lines(position, footprint, parameters.y);
    let x_axis = 1.0 - smoothstep(0.35, 1.25, abs(position.y) / footprint.y);
    let z_axis = 1.0 - smoothstep(0.35, 1.25, abs(position.x) / footprint.x);

    let minor_alpha = minor * (1.0 - major) * 0.16;
    let major_alpha = major * 0.25;
    let x_alpha = x_axis * 0.42;
    let z_alpha = z_axis * 0.42;
    let total = minor_alpha + major_alpha + x_alpha + z_alpha;
    let light = vec3<f32>(0.16, 0.21, 0.29) * minor_alpha
        + vec3<f32>(0.31, 0.33, 0.37) * major_alpha
        + vec3<f32>(0.46, 0.16, 0.115) * x_alpha
        + vec3<f32>(0.12, 0.245, 0.46) * z_alpha;

    let distance = length(position - view.world_position.xz);
    let distance_fade = 1.0 - smoothstep(parameters.z, parameters.w, distance);
    return vec4<f32>(light / max(total, 0.00001), min(total, 0.72) * distance_fade);
}
