import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Glass / plasma / stream, eased interaction, setting, seed.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
@group(1) @binding(1) var<uniform> metrics: vec4<f32>;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    q += vec3<f32>(dot(q, q.yzx + vec3<f32>(33.33)));
    return fract((q.x + q.y) * q.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(mix(hash21(cell), hash21(cell + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash21(cell + vec2<f32>(0.0, 1.0)), hash21(cell + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn mist(p: vec2<f32>) -> f32 {
    return noise(p) * 0.57 + noise(p * 2.13 + vec2<f32>(8.3)) * 0.28
        + noise(p * 4.27 + vec2<f32>(17.1)) * 0.15;
}

fn round_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let density = max(metrics.x, 0.5);
    let size = max(in.size / density, vec2<f32>(1.0));
    let p = (in.uv - vec2<f32>(0.5)) * size;
    let time = globals.time;
    let seed = state.w;
    let engagement = clamp(state.y, 0.0, 1.0);
    let gold = vec3<f32>(0.94, 0.66, 0.29);
    let violet = vec3<f32>(0.30, 0.025, 0.74);
    let cyan = vec3<f32>(0.08, 0.65, 1.0);

    if state.x < 0.5 {
        let d = round_box(p, size * 0.5 - vec2<f32>(0.7), 12.0);
        let mask = 1.0 - smoothstep(-0.6, 0.8, d);
        let inner = max(-d, 0.0);
        let rim = exp(-abs(d + 1.3) * 1.45);
        let bevel = exp(-abs(d + 3.6) * 1.2);
        // Warped reflections and caustics make the dark glass feel fluid.
        let fold = sin(p.y * 0.024 - time * 0.28 + sin(p.x * 0.06 + time * 0.19));
        let reflection = exp(-pow((p.x - fold * 12.0 + size.x * 0.19) / 8.0, 2.0));
        let caustic = pow(0.5 + 0.5 * sin(p.y * 0.036 + fold * 2.0 - time * 0.21), 9.0);
        let fresnel = exp(-inner / 8.0);
        let travelling = 0.5 + 0.5 * sin(p.y * 0.019 - time * 0.42);
        let cool = mix(vec3<f32>(0.10, 0.13, 0.24), vec3<f32>(0.26, 0.12, 0.37), travelling);
        var rgb = mix(vec3<f32>(0.014, 0.016, 0.027), vec3<f32>(0.028, 0.022, 0.044), in.uv.y);
        rgb += cool * (reflection * 0.12 + fresnel * 0.10);
        rgb += gold * (rim * (0.19 + travelling * 0.35) + bevel * 0.025 + caustic * fresnel * 0.045);
        let cell = floor((p + vec2<f32>(seed)) / vec2<f32>(17.0, 31.0));
        let point = fract((p + vec2<f32>(seed)) / vec2<f32>(17.0, 31.0)) - vec2<f32>(0.5);
        let star = exp(-dot(point * vec2<f32>(17.0, 31.0), point * vec2<f32>(17.0, 31.0)) * 2.0)
            * step(0.86, hash21(cell)) * (0.65 + 0.35 * sin(time * 0.8 + hash21(cell) * 6.28));
        rgb += vec3<f32>(0.45, 0.38, 0.65) * star * 0.20;
        return vec4<f32>(rgb, (0.94 + rim * 0.05) * mask);
    }

    if state.x > 1.5 {
        let d = round_box(p, size * 0.5, min(3.0, size.x * 0.5));
        let mask = 1.0 - smoothstep(-0.5, 0.4, d);
        let pulse = pow(0.5 + 0.5 * sin(p.y * 0.18 + time * 1.5 + seed), 8.0);
        let beam = exp(-p.x * p.x * 0.44);
        let rgb = mix(violet * 0.35, gold * 0.55, beam) + cyan * pulse * 0.18;
        return vec4<f32>(rgb, mask * (0.65 + pulse * 0.2));
    }

    // The luminous envelope can flow; its rectangular input target never moves.
    let half_core = max(size * 0.5 - metrics.yz - vec2<f32>(2.0, 3.0), vec2<f32>(1.0));
    let q = p / half_core;
    let radius = length(q);
    let angle = atan2(q.y, q.x);
    let t = time * 0.65 + seed * 0.37;
    let boundary = 0.91 + 0.055 * sin(angle * 5.0 + t) + 0.035 * sin(angle * 3.0 - t * 1.3);
    let distance = radius - boundary;
    let body = 1.0 - smoothstep(-0.05, 0.035, distance);
    let corona = exp(-pow(max(distance, 0.0) * 4.6, 2.0));
    let rim = exp(-abs(distance) * 47.0);
    let clouds = mist(q * 2.2 + vec2<f32>(t * 0.23, -t * 0.19));
    let spiral = sin(angle * 3.0 + radius * 9.0 - t * 2.5 + clouds * 4.0);
    let veins = pow(1.0 - abs(sin(clouds * 17.0 + radius * 3.0 - t)), 7.0);
    let plasma = mix(violet, vec3<f32>(0.62, 0.055, 0.49), 0.5 + spiral * 0.5);
    var rgb = plasma * (0.22 + clouds * 0.75);
    rgb += cyan * veins * (0.38 + engagement * 0.25);
    rgb += mix(cyan, gold, 0.5 + 0.5 * sin(angle * 2.0 + t * 0.5)) * rim * 0.85;
    // A warm star-like nucleus ties the nebula colors to the application's gold accent.
    let twist = time * 0.10 + seed;
    let rotated = vec2<f32>(q.x * cos(twist) - q.y * sin(twist), q.x * sin(twist) + q.y * cos(twist));
    let star_rays = exp(-abs(rotated.x) * 22.0) / (1.0 + abs(rotated.y) * 9.0)
        + exp(-abs(rotated.y) * 30.0) / (1.0 + abs(rotated.x) * 8.0);
    let nucleus = exp(-dot(q, q) * 24.0);
    rgb += gold * (star_rays * 0.35 + nucleus * 0.65) * (0.85 + engagement * 0.45);
    rgb += vec3<f32>(0.6, 0.72, 1.0) * nucleus * 0.35;
    let halo_rgb = mix(violet * 0.5, cyan * 0.5, 0.5 + 0.5 * sin(angle + t));
    rgb = mix(halo_rgb, rgb, body);
    let edge_clip = clamp(min(size.x * 0.5 - abs(p.x), size.y * 0.5 - abs(p.y)) / 2.0, 0.0, 1.0);
    let alpha = max(body * 0.98, corona * (0.30 + engagement * 0.28)) * edge_clip;
    return vec4<f32>(rgb * (1.0 + engagement * 0.20), alpha);
}
