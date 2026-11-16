import bevy_ui_render::ui_vertex_output::UiVertexOutput;

@group(1) @binding(0) var<uniform> motion: vec4<f32>;

const GOLD: vec3<f32> = vec3<f32>(1.0, 0.72, 0.36);
const MINT: vec3<f32> = vec3<f32>(0.37, 0.86, 0.74);
const CAMERA_DISTANCE: f32 = 3.6;
const ORBIT_RADIUS: f32 = 1.38;
const ORBIT_TILT_COS: f32 = 0.3478261;
const ORBIT_TILT_SIN: f32 = 0.937558;

fn rotate(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn local(p: vec3<f32>, time: f32) -> vec3<f32> {
    let xz = rotate(p.xz, time * 0.22 + 0.55);
    let yz = rotate(vec2<f32>(p.y, xz.y), -0.38 + sin(time * 0.23) * 0.12);
    return vec3<f32>(xz.x, yz.x, yz.y);
}

fn shape(p: vec3<f32>, time: f32) -> f32 {
    let q = local(p, time);
    let box = abs(q) - vec3<f32>(0.52);
    let rounded = length(max(box, vec3<f32>(0.0)))
        + min(max(box.x, max(box.y, box.z)), 0.0) - 0.20;
    let sphere = length(q) - 0.83;
    let morph = 0.34 + 0.28 * sin(time * 0.38);
    let wave = sin(q.x * 5.0 + time * 0.6) * sin(q.y * 5.0 - time * 0.4)
        * sin(q.z * 5.0 + time * 0.3) * 0.025;
    return mix(rounded, sphere, morph) + wave;
}

fn normal(p: vec3<f32>, time: f32) -> vec3<f32> {
    let e = vec2<f32>(0.0015, -0.0015);
    return normalize(e.xyy * shape(p + e.xyy, time)
        + e.yyx * shape(p + e.yyx, time)
        + e.yxy * shape(p + e.yxy, time)
        + e.xxx * shape(p + e.xxx, time));
}

fn grid(q: vec3<f32>, n: vec3<f32>, pixel: f32) -> f32 {
    let cell = abs(fract(q * 3.0 + vec3<f32>(0.5)) - vec3<f32>(0.5)) / 3.0;
    let line = vec3<f32>(1.0) - smoothstep(vec3<f32>(0.005), vec3<f32>(0.005 + pixel), cell);
    let weights = pow(abs(n), vec3<f32>(6.0));
    return dot(vec3<f32>(max(line.y, line.z), max(line.x, line.z), max(line.x, line.y)),
        weights / max(dot(weights, vec3<f32>(1.0)), 0.0001));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let size = max(in.size, vec2<f32>(1.0));
    let unit = min(size.x, size.y);
    let p = (in.uv - vec2<f32>(0.5)) * size / unit * 3.1;
    let time = motion.x;
    let pixel = 3.1 / unit;
    let gold = mix(GOLD, vec3<f32>(1.0, 0.82, 0.52), motion.y * 0.3);

    // Transparent, feathered light keeps the artwork part of the existing panel.
    var light = gold * exp(-dot(p, p) * 2.0) * 0.045;
    light += MINT * exp(-dot(p - vec2<f32>(0.4, -0.3), p - vec2<f32>(0.4, -0.3)) * 3.5) * 0.035;
    let halo = abs(length(p) - 1.20);
    light += gold * exp(-halo * 90.0) * 0.13;

    let origin = vec3<f32>(0.0, 0.0, CAMERA_DISTANCE);
    let ray = normalize(vec3<f32>(p, -CAMERA_DISTANCE));
    // Intersect the same tilted 3D circle used by the travelling star.
    // Its near and far arcs then share depth with the ray-marched cube.
    let orbit_normal = vec3<f32>(rotate(vec2<f32>(0.0, -ORBIT_TILT_SIN), 0.3), ORBIT_TILT_COS);
    let orbit_depth = -dot(origin, orbit_normal) / min(dot(ray, orbit_normal), -0.00001);
    let orbit_position = origin + ray * orbit_depth;
    let orbit_radius = length(orbit_position);
    let ring = abs(orbit_radius - ORBIT_RADIUS);
    let footprint = clamp(fwidth(orbit_radius), 0.001, 0.04);
    let ring_core = 1.0 - smoothstep(footprint * 0.15, footprint * 0.85, ring);
    let ring_glow = exp(-ring / max(footprint * 1.8, 0.008));
    let ring_tint = mix(MINT, gold, smoothstep(-1.0, 1.0, rotate(p, -0.3).x));
    let orbit_light = ring_tint * (ring_core * 0.27 + ring_glow * 0.07);

    let angle = time * 0.5;
    let star_xy = rotate(vec2<f32>(cos(angle), sin(angle) * ORBIT_TILT_COS) * ORBIT_RADIUS, 0.3);
    let star = vec3<f32>(star_xy, sin(angle) * ORBIT_RADIUS * ORBIT_TILT_SIN);
    let point = star.xy * CAMERA_DISTANCE / (CAMERA_DISTANCE - star.z);
    let point_distance = length(p - point);
    let star_light = gold * (exp(-point_distance * point_distance * 1700.0) * 0.8
        + exp(-point_distance * 32.0) * 0.18);
    let star_depth = dot(star - origin, ray);
    let b = dot(origin, ray);
    let discriminant = b * b - dot(origin, origin) + 1.1 * 1.1;
    var coverage = 0.0;
    var object_depth = 1e10;
    if discriminant > 0.0 {
        var distance = -b - sqrt(discriminant);
        let end = -b + sqrt(discriminant);
        var hit = false;
        // Restrict tracing to the small object's bounding sphere, with a fixed budget.
        for (var step = 0u; step < 48u; step += 1u) {
            let d = shape(origin + ray * distance, time);
            if d < 0.0015 { hit = true; break; }
            distance += max(d * 0.82, 0.001);
            if distance > end { break; }
        }
        if hit {
            let position = origin + ray * distance;
            let n = normal(position, time);
            let q = local(position, time);
            let local_normal = local(n, time);
            let key = normalize(vec3<f32>(-0.6, 0.8, 1.5));
            let diffuse = max(dot(n, key), 0.0);
            let rim = pow(1.0 - max(dot(n, -ray), 0.0), 3.0);
            let specular = pow(max(dot(n, normalize(key - ray)), 0.0), 65.0);
            let lattice = grid(q, local_normal, pixel * 0.85);
            let scan_height = sin(time * 0.72) * 0.85;
            let scan = exp(-abs(position.y - scan_height) * 20.0);
            let tint = mix(gold, MINT, smoothstep(0.1, 0.85, q.y) * 0.65);
            let base = vec3<f32>(0.055, 0.065, 0.075) * (0.55 + diffuse * 1.6);
            let mesh = tint * lattice * (0.24 + diffuse * 0.38 + scan * (0.65 + motion.y * 0.4));
            let surface = base + mesh + tint * rim * 0.6
                + vec3<f32>(1.0, 0.91, 0.72) * specular * 0.6 + MINT * scan * 0.055;
            light = surface;
            coverage = 1.0;
            object_depth = distance;
        }
    }

    // Compose the foreground arc after the surface; the back arc and star are
    // hidden only where the cube is genuinely closer to the camera.
    if orbit_depth < object_depth { light += orbit_light; }
    if star_depth < object_depth { light += star_light; }

    let peak = clamp(max(light.r, max(light.g, light.b)), 0.0, 1.0);
    let alpha = max(coverage, peak);
    let edge = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
    return vec4<f32>(light / max(alpha, 0.0001), alpha * smoothstep(0.0, 0.07, edge));
}
