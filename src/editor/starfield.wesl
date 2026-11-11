import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Opacity, motion speed, field seed, reserved.
@group(1) @binding(0) var<uniform> parameters: vec4<f32>;

fn hash22(point: vec2<f32>) -> vec2<f32> {
    var p = fract(vec3<f32>(point.x, point.y, point.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p = p + vec3<f32>(dot(p, p.yzx + vec3<f32>(33.33)));
    return fract((p.xx + p.yz) * p.zy);
}

fn star_layer(
    pixel: vec2<f32>,
    time: f32,
    cell_size: f32,
    speed: f32,
    depth: f32,
    seed: f32,
) -> vec3<f32> {
    let direction = normalize(vec2<f32>(1.0, 0.36));
    let position = (pixel - direction * time * speed) / cell_size;
    let cell = floor(position);
    let random = hash22(cell + vec2<f32>(seed, seed * 2.731));
    let character = fract(random.x * 17.817 + random.y * 53.931);
    if character < 0.45 {
        return vec3<f32>(0.0);
    }

    // Keep each point and its small trail within its own cell. This avoids a
    // costly neighboring-cell search and prevents clipping as the field moves.
    let delta = (fract(position) - (vec2<f32>(0.16) + random * 0.68)) * cell_size;
    let tail_length = mix(3.0, 12.0, depth);
    if max(abs(delta.x), abs(delta.y)) > tail_length + 8.0 {
        return vec3<f32>(0.0);
    }

    // These are physical pixels: retain a visible soft core on Retina screens.
    let radius = mix(1.15, 2.6, depth) * mix(0.8, 1.2, character);
    let radius_squared = radius * radius;
    let distance_squared = dot(delta, delta);
    let core = exp(-distance_squared * 1.8 / radius_squared);
    let halo = exp(-distance_squared / (radius_squared * 9.0)) * 0.045;
    let along = dot(delta, direction);
    let across = dot(delta, vec2<f32>(-direction.y, direction.x));
    let trail = exp(-across * across / radius_squared)
        * smoothstep(-tail_length, 0.0, along)
        * (1.0 - smoothstep(-0.6, 1.0, along))
        * depth * 0.13;

    // Only a few nearer points receive a delicate diffraction sparkle.
    var sparkle = 0.0;
    if character > 0.94 && depth > 0.8 {
        let horizontal = exp(-abs(delta.x) / 3.5 - delta.y * delta.y / 0.16);
        let vertical = exp(-abs(delta.y) / 3.5 - delta.x * delta.x / 0.16);
        sparkle = (horizontal + vertical) * 0.09;
    }

    let twinkle = 0.82 + 0.18 * sin(time * (0.65 + character * 0.35) + character * 31.0);
    let cool = vec3<f32>(0.62, 0.65, 0.91);
    let warm = vec3<f32>(1.0, 0.88, 0.67);
    let tint = mix(cool, warm, smoothstep(0.55, 0.92, character));
    return tint * (core * 0.58 + halo + trail + sparkle)
        * twinkle * mix(0.52, 1.0, depth);
}

fn shooting_star(
    pixel: vec2<f32>,
    size: vec2<f32>,
    time: f32,
    phase: f32,
    seed: f32,
) -> vec3<f32> {
    // Two staggered tracks produce one short flight roughly every nine
    // seconds, with a new starting point on every pass.
    let period = 18.0;
    let clock = time + phase;
    let cycle = floor(clock / period);
    let age = fract(clock / period) * period;
    let duration = 1.85;
    if age >= duration {
        return vec3<f32>(0.0);
    }

    let random = hash22(vec2<f32>(cycle + seed * 7.31, seed + 91.7));
    let direction = normalize(vec2<f32>(1.0, 0.32 + random.y * 0.2));
    let start = size * vec2<f32>(0.05 + random.x * 0.53, 0.08 + random.y * 0.33);
    let distance = min(size.x * 0.42 + size.y * 0.18, 780.0);
    let head = start + direction * distance * (age / duration);
    let delta = pixel - head;
    let along = dot(delta, direction);
    let across = dot(delta, vec2<f32>(-direction.y, direction.x));
    let tail_length = clamp(size.x * 0.085, 42.0, 140.0);
    if along < -tail_length - 12.0 || along > 12.0 || abs(across) > 12.0 {
        return vec3<f32>(0.0);
    }

    let taper = pow(clamp(1.0 + along / tail_length, 0.0, 1.0), 2.0)
        * (1.0 - smoothstep(0.0, 1.2, along));
    let trail = exp(-across * across / 0.65) * taper;
    let trail_glow = exp(-across * across / 8.0) * taper * 0.055;
    let core = exp(-dot(delta, delta) / 1.5);
    let halo = exp(-dot(delta, delta) / 22.0) * 0.09;
    let fade = smoothstep(0.0, 0.22, age)
        * (1.0 - smoothstep(duration - 0.42, duration, age));
    return vec3<f32>(1.0, 0.83, 0.57)
        * (trail * 0.64 + trail_glow + core * 0.58 + halo) * fade;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let pixel = in.uv * in.size;
    let time = globals.time * max(parameters.y, 0.0);
    let seed = parameters.z;
    var light = star_layer(pixel, time, 128.0, 6.0, 0.25, seed);
    light = light + star_layer(pixel, time, 210.0, 13.0, 0.58, seed + 43.1);
    light = light + star_layer(pixel, time, 340.0, 24.0, 1.0, seed + 91.7);
    light = light + shooting_star(pixel, in.size, time, 0.0, seed + 3.0);
    light = light + shooting_star(pixel, in.size, time, 9.0, seed + 29.0);

    let edge = min(min(pixel.x, in.size.x - pixel.x), min(pixel.y, in.size.y - pixel.y));
    let edge_width = max(min(24.0, min(in.size.x, in.size.y) * 0.18), 1.0);
    let edge_fade = smoothstep(0.0, edge_width, edge);
    let peak = max(max(light.r, light.g), light.b);
    let opacity = clamp(peak * parameters.x * edge_fade, 0.0, 0.86);
    // UiMaterial uses ordinary alpha blending, so RGB must remain straight.
    return vec4<f32>(light / max(peak, 0.00001), opacity);
}
