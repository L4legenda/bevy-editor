import bevy_ui_render::ui_vertex_output::UiVertexOutput;

@group(1) @binding(0) var<uniform> viewport: vec4<f32>;
@group(1) @binding(1) var<uniform> pivot: vec4<f32>;
@group(1) @binding(2) var<uniform> tips: array<vec4<f32>, 3>;
@group(1) @binding(3) var<uniform> colors: array<vec4<f32>, 3>;
@group(1) @binding(4) var<uniform> faces: array<vec4<f32>, 36>;
@group(1) @binding(5) var<uniform> planes: array<vec4<f32>, 12>;
@group(1) @binding(6) var<uniform> plane_colors: array<vec4<f32>, 3>;
@group(1) @binding(7) var<uniform> outer_ring: vec4<f32>;

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let edge = b - a;
    let t = clamp(dot(p - a, edge) / max(dot(edge, edge), 0.0001), 0.0, 1.0);
    return length(p - a - edge * t);
}

fn quad_distance(p: vec2<f32>, corners: array<vec2<f32>, 4>) -> f32 {
    var distance = 1e6;
    var lower = 1e6;
    var upper = -1e6;
    var area = 0.0;
    for (var corner = 0u; corner < 4u; corner += 1u) {
        let a = corners[corner];
        let b = corners[(corner + 1u) % 4u];
        let edge = b - a;
        let side = edge.x * (p.y - a.y) - edge.y * (p.x - a.x);
        area += a.x * b.y - a.y * b.x;
        lower = min(lower, side);
        upper = max(upper, side);
        distance = min(distance, segment_distance(p, a, b));
    }
    // Edge-on faces have no interior and must not cover the entire viewport.
    if abs(area) < 0.01 {
        return 1e6;
    }
    return select(distance, -distance, lower >= 0.0 || upper <= 0.0);
}

fn coverage(distance: f32) -> f32 {
    let aa = max(fwidth(distance), 0.3);
    return 1.0 - smoothstep(-aa * 0.5, aa * 0.5, distance);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    if viewport.z < 0.5 {
        return vec4<f32>(0.0);
    }
    let point = in.uv * viewport.xy;
    var color = vec3<f32>(0.0);
    var alpha = 0.0;
    if outer_ring.z > 0.5 {
        alpha = coverage(abs(length(point - pivot.xy) - outer_ring.x) - 0.55)
            * mix(0.58, 0.9, outer_ring.y);
        color = mix(vec3<f32>(0.32), vec3<f32>(1.0, 0.62, 0.17), outer_ring.y) * alpha;
    }
    for (var axis = 0u; axis < 3u; axis += 1u) {
        if tips[axis].z < 0.5 {
            continue;
        }
        let scaling = viewport.w == 1.0;
        let end = tips[axis].xy;
        let delta = end - pivot.xy;
        let extent = max(length(delta), 0.001);
        let direction = delta / extent;
        let head_length = min(13.0, extent * 0.4);
        let half_width = 4.8 * min(1.0, extent / 24.0);
        let offset = point - end;
        let x = dot(offset, direction);
        let y = dot(offset, vec2<f32>(-direction.y, direction.x));
        // Filled triangle: two sloped sides and the flat rear edge.
        let triangle = max(-x - head_length,
            (x * half_width + abs(y) * head_length)
                / length(vec2<f32>(head_length, half_width)));
        let shaft_end = select(end - direction * head_length, end, scaling);
        let shaft = shaft_end - pivot.xy;
        let t = clamp(dot(point - pivot.xy, shaft) / max(dot(shaft, shaft), 0.001), 0.0, 1.0);
        let line = length(point - pivot.xy - shaft * t) - 1.0;
        let shape = select(min(triangle, line), line, scaling);
        let opacity = coverage(shape) * tips[axis].z;
        color = mix(color, colors[axis].rgb, opacity);
        alpha = opacity + alpha * (1.0 - opacity);
        if scaling {
            for (var face = 0u; face < 3u; face += 1u) {
                let index = (axis * 3u + face) * 4u;
                let corners = array<vec2<f32>, 4>(faces[index].xy, faces[index + 1u].xy,
                    faces[index + 2u].xy, faces[index + 3u].xy);
                let fill = coverage(quad_distance(point, corners)) * tips[axis].z;
                // Different face values make the solid cube readable.
                let shade = select(select(0.78, 1.0, face == 1u), 0.60, face == 2u);
                color = mix(color, colors[axis].rgb * shade, fill);
                alpha = fill + alpha * (1.0 - fill);
            }
        }
    }
    for (var plane = 0u; plane < 3u; plane += 1u) {
        let index = plane * 4u;
        if planes[index].z < 0.5 {
            continue;
        }
        let corners = array<vec2<f32>, 4>(planes[index].xy, planes[index + 1u].xy,
            planes[index + 2u].xy, planes[index + 3u].xy);
        let fill = coverage(quad_distance(point, corners));
        color = mix(color, plane_colors[plane].rgb, fill);
        alpha = fill + alpha * (1.0 - fill);
    }
    // The white pivot overlays the crossing shafts, with a dark one-pixel rim.
    if viewport.w != 3.0 {
        let radius = length(point - pivot.xy);
        let rim = coverage(radius - 4.2);
        let fill = coverage(radius - 3.1);
        color = mix(color, vec3<f32>(0.055), rim);
        color = mix(color, vec3<f32>(1.0), fill);
        alpha = rim + alpha * (1.0 - rim);
    }
    return vec4<f32>(color / max(alpha, 0.0001), alpha);
}
