import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Exposure, flow speed, phase offset, reserved.
@group(1) @binding(0) var<uniform> settings: vec4<f32>;

const HORIZON: f32 = 0.30;
const DISK_INNER: f32 = 0.94;
const DISK_OUTER: f32 = 2.25;

fn hash(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    q = q + vec3<f32>(dot(q, q.yzx + vec3<f32>(33.33)));
    return fract((q.x + q.y) * q.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(
        mix(hash(cell), hash(cell + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(cell + vec2<f32>(0.0, 1.0)), hash(cell + vec2<f32>(1.0)), u.x),
        u.y,
    );
}

fn turbulence(p: vec2<f32>) -> f32 {
    return noise(p) * 0.57 + noise(p * 2.03 + vec2<f32>(17.7)) * 0.29
        + noise(p * 4.09 + vec2<f32>(41.3)) * 0.14;
}

fn accretion(position: vec3<f32>, view: vec3<f32>, time: f32, normal: vec3<f32>) -> vec4<f32> {
    let radius = length(position);
    let envelope = smoothstep(DISK_INNER, DISK_INNER + 0.12, radius)
        * (1.0 - smoothstep(DISK_OUTER - 0.50, DISK_OUTER, radius));
    if envelope < 0.001 { return vec4<f32>(0.0); }

    let disk_y = dot(position, cross(normal, vec3<f32>(1.0, 0.0, 0.0)));
    let angle = atan2(disk_y, position.x);
    // Differential rotation shears the texture into filaments. Cartesian
    // angular coordinates keep the flow continuous across the polar seam.
    let orbit = angle + time * 0.72 * pow(DISK_INNER / radius, 1.5);
    let polar = vec2<f32>(cos(orbit), sin(orbit));
    let eddies = turbulence(polar * (5.5 + radius * 2.8) + vec2<f32>(radius * 9.0));
    let threads = 0.5 + 0.5 * sin(radius * 94.0 + eddies * 6.0 + polar.x * 2.5);
    let waves = 0.5 + 0.5 * sin(radius * 36.0 + eddies * 3.0 - polar.y * 4.0);
    let density = (0.43 + threads * threads * 0.23 + waves * 0.15)
        * (0.48 + eddies * 1.05);
    let heat = pow(clamp((DISK_OUTER - radius) / (DISK_OUTER - DISK_INNER), 0.0, 1.0), 0.65);

    // The approaching side is brighter and slightly hotter than the receding
    // side; this asymmetry makes rotation legible without flashing the halo.
    let tangent = normalize(cross(normal, position));
    let boost = clamp(1.0 + dot(tangent, view) * 0.50 / sqrt(radius), 0.52, 1.65);
    let amber = vec3<f32>(1.0, 0.43, 0.12);
    let ivory = vec3<f32>(1.0, 0.88, 0.61);
    let tint = mix(amber, ivory, clamp(heat * 0.9 + (boost - 1.0) * 0.30, 0.0, 1.0));
    let emission = tint * density * (0.55 + heat * 1.65) * boost * boost;
    return vec4<f32>(emission, envelope * 0.92);
}

// Schwarzschild-inspired inverse-fourth-power light deflection. Conserving
// angular momentum yields the upper/lower lensed images of the same disk.
fn gravity(position: vec3<f32>, strength: f32) -> vec3<f32> {
    let r2 = max(dot(position, position), 0.012);
    return -position * strength / (r2 * r2 * sqrt(r2));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let size = max(in.size, vec2<f32>(1.0));
    // Shrink the illustration rather than crop the disk on narrow panels.
    let unit = max(min(size.y, size.x * 0.55), 1.0);
    let p = (in.uv - vec2<f32>(0.5)) * size / unit * 2.0;
    let time = globals.time * settings.y + settings.z;
    let tilt = -0.095;
    let c = cos(tilt);
    let s = sin(tilt);
    let screen = vec2<f32>(c * p.x - s * p.y, -(s * p.x + c * p.y));
    var position = vec3<f32>(0.0, 0.0, 4.8);
    var velocity = normalize(vec3<f32>(screen * 1.65, -4.8));
    let angular = cross(position, velocity);
    let impact = length(angular);
    let strength = 1.5 * HORIZON * dot(angular, angular);
    let inclination = 0.20;
    let normal = vec3<f32>(0.0, cos(inclination), sin(inclination));

    var light = vec3<f32>(0.0);
    var transmission = 1.0;
    var captured = false;
    // Adaptive steps and a hard bound keep the effect local and inexpensive.
    // Velocity Verlet avoids the unstable, flickering orbit of forward Euler.
    for (var step = 0u; step < 80u; step = step + 1u) {
        let radius = length(position);
        if radius < HORIZON {
            captured = true;
            break;
        }
        if radius > 6.0 && dot(position, velocity) > 0.0 { break; }
        let distance = clamp(radius * 0.18, 0.035, 0.45);
        let acceleration = gravity(position, strength);
        let next = position + velocity * distance + acceleration * (0.5 * distance * distance);
        let a = dot(position, normal);
        let b = dot(next, normal);
        if a * b < 0.0 {
            let crossing = mix(position, next, clamp(a / (a - b), 0.0, 1.0));
            let disk = accretion(crossing, -normalize(velocity), time, normal);
            light = light + transmission * disk.rgb * disk.a;
            transmission = transmission * (1.0 - disk.a);
            if transmission < 0.015 { break; }
        }
        velocity = velocity + (acceleration + gravity(next, strength)) * (0.5 * distance);
        position = next;
    }
    // Rays still orbiting after the budget are inside the unresolved photon rim.
    if length(position) < 1.0 && transmission > 0.98 { captured = true; }

    let critical = 2.5980762 * HORIZON;
    let delta = abs(impact - critical);
    let pixel = 3.3 / unit;
    let photon = 1.0 - smoothstep(pixel * 0.30, pixel * 1.35, delta);
    let corona = exp(-delta * 24.0) * 0.11 + exp(-delta * 8.0) * 0.025;
    let asymmetry = 0.78 + 0.22 * sin(atan2(screen.y, screen.x) - 0.6);
    let pulse = 0.98 + 0.02 * sin(time * 0.65);
    light = light + vec3<f32>(1.0, 0.79, 0.46)
        * (photon * 0.48 + corona) * asymmetry * pulse;

    // A broad, quiet glow carries the disk's light into the surrounding UI.
    let haze_y = screen.y + 0.05;
    let haze = exp(-screen.x * screen.x * 1.5 - haze_y * haze_y * 55.0) * 0.035;
    light = light + vec3<f32>(0.93, 0.58, 0.25) * haze * select(1.0, 0.0, captured);
    let mapped = vec3<f32>(1.0) - exp(-light * settings.x);
    let peak = max(max(mapped.r, mapped.g), mapped.b);
    let coverage = 1.0 - transmission * select(1.0, 0.0, captured);
    let alpha = max(coverage, peak);
    let core = vec3<f32>(0.002, 0.003, 0.005) * select(0.0, transmission, captured);
    let edge = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
    let fade = smoothstep(0.0, 0.065, edge);
    // UiMaterial uses straight alpha; transparent corners reveal the existing
    // starfield and must not leave a dark rectangle around the illustration.
    return vec4<f32>((mapped + core) / max(alpha, 0.0001), alpha * fade);
}
