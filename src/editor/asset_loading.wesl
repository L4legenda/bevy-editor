import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_ui_render::ui::sd_inset_rounded_box;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Opacity, per-card animation phase, reserved.
@group(1) @binding(0) var<uniform> state: vec4<f32>;

const TAU: f32 = 6.28318530718;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let size = max(in.size, vec2<f32>(1.0));
    let pixel = (in.uv - vec2<f32>(0.5)) * size;
    let edge = sd_inset_rounded_box(pixel, size, in.border_radius_x, in.border_radius_y, vec4<f32>(0.0));
    let clip = 1.0 - smoothstep(-0.8, 0.2, edge);
    if clip * state.x <= 0.0001 { return vec4<f32>(0.0); }

    let p = pixel / min(size.x, size.y);
    let time = globals.time;
    let phase = state.y;
    let clock = time * 1.9 + phase;
    let gold = vec3<f32>(0.82, 0.55, 0.25);
    let pearl = vec3<f32>(1.0, 0.87, 0.65);
    let violet = vec3<f32>(0.36, 0.23, 0.64);

    // A soft, opaque surface masks blank render targets until they are ready.
    var color = mix(vec3<f32>(0.007, 0.008, 0.013), vec3<f32>(0.014, 0.011, 0.022), in.uv.x);
    let warm = p - vec2<f32>(-0.43, 0.37);
    let cool = p - vec2<f32>(0.51, -0.38);
    color += gold * exp(-dot(warm, warm) * 4.0) * 0.025;
    color += violet * exp(-dot(cool, cool) * 3.0) * 0.045;

    // A broad diagonal sheen passes slowly, with no flashing or fake progress.
    let sweep = (fract(time / 3.6 + phase / TAU) * 2.7 - 1.35);
    let stripe = p.x + p.y * 0.42 - sweep;
    color += mix(violet, gold, in.uv.x) * exp(-stripe * stripe * 45.0) * 0.045;

    // Aspect-correct orbital arcs, a warm comet and a smaller cool satellite.
    let radius = 0.185;
    let radial = length(p);
    let distance = abs(radial - radius);
    let aa = max(fwidth(radial), 0.001);
    let ring = 1.0 - smoothstep(0.005, 0.005 + aa, distance);
    let angle = atan2(p.y, p.x);
    let trail = pow(1.0 - fract((clock - angle) / TAU), 3.5);
    color += gold * ring * (0.055 + 0.68 * trail);
    color += gold * exp(-distance * 90.0) * trail * 0.08;
    let head = p - vec2<f32>(cos(clock), sin(clock)) * radius;
    color += pearl * exp(-dot(head, head) / 0.00012) * 0.95;
    color += gold * exp(-dot(head, head) / 0.0018) * 0.15;

    let ellipse = vec2<f32>(p.x * 0.70 + p.y * 0.30, (p.y * 0.70 - p.x * 0.30) * 1.9);
    let orbit = abs(length(ellipse) - 0.215);
    color += violet * exp(-orbit * 220.0) * 0.12;
    let satellite_angle = -clock * 0.67;
    let orbit_point = vec2<f32>(cos(satellite_angle), sin(satellite_angle) / 1.9) * 0.215;
    let satellite = p - vec2<f32>(orbit_point.x * 0.70 - orbit_point.y * 0.30, orbit_point.x * 0.30 + orbit_point.y * 0.70) / 0.58;
    color += violet * exp(-dot(satellite, satellite) / 0.00013) * 0.7;
    color += violet * exp(-dot(satellite, satellite) / 0.0015) * 0.12;

    // A small luminous diamond anchors the motion; breathing stays subtle.
    let diamond = abs(p.x) + abs(p.y);
    let core = 1.0 - smoothstep(0.026, 0.026 + aa, diamond);
    color += pearl * core * (0.28 + 0.04 * sin(time * 2.1 + phase));
    color += gold * exp(-dot(p, p) / 0.002) * 0.045;
    return vec4<f32>(color, clip * state.x);
}
