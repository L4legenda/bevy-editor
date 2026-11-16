import bevy_pbr::render::forward_io::{Vertex, VertexOutput, decompress_vertex};
import bevy_pbr::render::mesh_view_bindings::{view, globals};

struct Selection { color: vec4<f32>, }
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> selection: Selection;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> style: vec4<f32>;

fn overlay_depth(clip: vec4<f32>, perspective_bias: f32, pixels: f32) -> f32 {
    if view.clip_from_view[3].w > 0.5 {
        // A pixel-sized lift keeps depth-tested feedback on the surface even
        // with a two-sided orthographic clip range; it must not become X-Ray.
        let world_pixel = 2.0 * style.x
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
    // Normalized tags: face 0, vertex 1/3, active vertex 2/3, edge 1.
    if v.color.w > 0.85 && clip.w > 0.0 {
        let viewport = max(view.viewport.zw, vec2<f32>(1.0));
        let tangent = view.clip_from_world * vec4<f32>(v.normal, 0.0);
        let direction = (tangent.xy * clip.w - clip.xy * tangent.w) * viewport;
        let unit = direction / max(length(direction), 0.000001);
        let offset = vec2<f32>(-unit.y, unit.x) * v.uv.x * 8.0;
        clip = vec4<f32>(clip.xy + offset * style.x * 2.0 / viewport * clip.w,
            overlay_depth(clip, 1.03, 8.0), clip.w);
    } else if v.color.w > 0.15 && clip.w > 0.0 {
        let radius = select(9.5, 12.0, v.color.w > 0.5);
        clip = vec4<f32>(clip.xy + v.uv * radius * style.x * 2.0 / max(view.viewport.zw, vec2<f32>(1.0)) * clip.w,
            overlay_depth(clip, 1.04, radius), clip.w);
    }
    out.position = clip;
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += vec3<f32>(dot(q, q.yzx + vec3<f32>(33.33)));
    return fract((q.x + q.y) * q.z);
}
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec3<f32>(3.0) - 2.0 * f);
    return mix(
        mix(mix(hash3(i), hash3(i + vec3<f32>(1,0,0)), u.x),
            mix(hash3(i + vec3<f32>(0,1,0)), hash3(i + vec3<f32>(1,1,0)), u.x), u.y),
        mix(mix(hash3(i + vec3<f32>(0,0,1)), hash3(i + vec3<f32>(1,0,1)), u.x),
            mix(hash3(i + vec3<f32>(0,1,1)), hash3(i + vec3<f32>(1,1,1)), u.x), u.y), u.z);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let time = globals.time;
    let uv_width = fwidth(in.uv);
    let gold = vec3<f32>(2.8, 2.2, 1.3);
    let pearl = vec3<f32>(3.3, 2.8, 1.9);
    let ink = vec3<f32>(0.030, 0.008, 0.18);
    let violet = vec3<f32>(0.40, 0.10, 0.85);
    if in.color.w > 0.85 {
        // A solid plum ribbon gives the champagne core contrast on bright models.
        // Distances are logical pixels, so the core and rim stay stable on zoom.
        let d = abs(in.uv.x) * 8.0;
        let aa = max(uv_width.x * 8.0, 0.35);
        let core = 1.0 - smoothstep(1.15 - aa * 0.5, 1.15 + aa * 0.5, d);
        let body = 1.0 - smoothstep(4.25 - aa * 0.5, 4.25 + aa * 0.5, d);
        let halo = 1.0 - smoothstep(4.25, 8.0, d);
        let phase = dot(in.color.xyz, vec3<f32>(2.1, 3.2, 1.7)) - time * 0.12;
        let glint = pow(0.5 + 0.5 * cos(phase * 6.2831853), 10.0);
        let base = mix(violet, ink, body);
        return vec4<f32>(mix(base, mix(gold, pearl, glint * 0.35), core),
            max(body, halo * 0.38));
    }
    if in.color.w > 0.15 {
        let is_single = in.color.w > 0.5;
        let extent = select(9.5, 12.0, is_single);
        let p = in.uv * extent;
        let r = length(p);
        let aa = max(length(uv_width) * extent * 0.5, 0.35);
        let core = 1.0 - smoothstep(2.5 - aa, 2.5 + aa, r);
        let body = 1.0 - smoothstep(4.6 - aa, 4.6 + aa, r);
        // The outer orbit remains visible when the manipulator covers the center.
        let orbit_radius = select(6.0, 8.5, is_single);
        let orbit_body = 1.0 - smoothstep(1.55 - aa * 0.5, 1.55 + aa * 0.5, abs(r - orbit_radius));
        let orbit = 1.0 - smoothstep(0.55, 0.55 + aa, abs(r - orbit_radius));
        let ray = (1.0 - smoothstep(0.25, 0.8 + aa * 0.3, min(abs(p.x), abs(p.y))))
            * (1.0 - smoothstep(4.0, extent - 0.5, max(abs(p.x), abs(p.y))));
        let breath = 0.91 + 0.06 * sin(time * 1.1 + dot(in.color.xyz, vec3<f32>(2.0, 3.0, 4.0)));
        let halo = exp(-r * r / (extent * extent * 0.30)) * breath;
        var color = mix(violet, ink, max(body, orbit_body));
        color = mix(color, violet * 1.3, ray);
        color = mix(color, gold, orbit);
        color = mix(color, pearl, core);
        let alpha = max(max(body, orbit_body), max(ray * 0.65, halo * 0.26));
        return vec4<f32>(color, alpha);
    }

    // Object-local clouds stay coherent across faces during camera/object motion.
    let p = (in.color.xyz - vec3<f32>(0.5)) * 4.0;
    let drift = vec3<f32>(time * 0.035, -time * 0.022, time * 0.018);
    let cloud = noise(p * 1.7 + drift) * 0.68 + noise(p * 3.6 - drift * 0.7) * 0.32;
    let fold = 0.5 + 0.5 * sin(dot(p, vec3<f32>(2.6, 3.2, 1.8)) + cloud * 5.0 - time * 0.13);
    let deep = vec3<f32>(0.045, 0.014, 0.095);
    var color = mix(deep, selection.color.rgb, smoothstep(0.16, 0.82, cloud));
    color += vec3<f32>(0.10, 0.035, 0.16) * pow(fold, 4.0);
    color += vec3<f32>(0.16, 0.075, 0.025) * pow(fold, 18.0) * 0.22;

    // Decorative face stars are deliberately smaller than editable vertices.
    let field = in.uv * 8.0;
    let cell = floor(field);
    let seed = hash3(vec3<f32>(cell, 7.1));
    let center = vec2<f32>(hash3(vec3<f32>(cell, 1.3)), hash3(vec3<f32>(cell, 5.7))) * 0.7 + 0.15;
    let distance = length(fract(field) - center);
    let aa = max(length(uv_width * 8.0), 0.002);
    let star = (1.0 - smoothstep(0.018, 0.018 + aa, distance)) * step(0.93, seed);
    let twinkle = 0.82 + 0.12 * sin(time * 0.7 + seed * 18.0);
    color += vec3<f32>(0.75, 0.53, 0.34) * star * twinkle;
    var eye = normalize(view.world_position - in.world_position.xyz);
    if view.clip_from_view[3].w > 0.5 {
        eye = view.world_from_view[2].xyz;
    }
    let normal = in.world_normal / max(length(in.world_normal), 0.000001);
    let fresnel = pow(1.0 - abs(dot(normal, eye)), 2.0);
    color += vec3<f32>(0.06, 0.025, 0.10) * fresnel;
    return vec4<f32>(color, selection.color.a);
}
