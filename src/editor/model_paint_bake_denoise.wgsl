// À-trous filter for the four visibility channels and the bent normal.
struct Guide {
    x: f32,
    y: f32,
    z: f32,
    texel: f32,
    part: u32,
    island: u32,
    normal: u32,
}

struct Settings {
    width: u32,
    rows: u32,
    step: i32,
    radius: f32,
}

@compute @workgroup_size(8, 8)
fn smooth_curvature(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= settings.width || id.y >= settings.rows) {
        return;
    }
    let index = id.y * settings.width + id.x;
    let center = guides[index];
    let offset = index * 2u;
    if (center.texel <= 0.0) {
        after[offset] = before[offset];
        return;
    }
    let position = vec3(center.x, center.y, center.z);
    let normal = normal_or(decode_normal(center.normal), vec3(0.0));
    let size = vec2<i32>(i32(settings.width), i32(settings.rows));
    let scale = center.texel * center.texel;
    let variance = 2.0 * settings.radius * settings.radius * scale;
    var curvature = 0.0;
    var total = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let shift = vec2(x, y) * settings.step;
            let sample = vec2<i32>(id.xy) + shift;
            if (any(sample < vec2(0)) || any(sample >= size)) {
                continue;
            }
            let neighbor = u32(sample.y) * settings.width + u32(sample.x);
            let other = guides[neighbor];
            if (other.texel <= 0.0 || other.part != center.part || other.island != center.island) {
                continue;
            }
            let delta = vec3(other.x, other.y, other.z) - position;
            let distance = dot(delta, delta);
            let reach = length(vec2<f32>(shift)) * 2.0 + 2.0;
            if (distance > reach * reach * scale) {
                continue;
            }
            let facing = smoothstep(0.6, 0.85, dot(normal,
                normal_or(decode_normal(other.normal), vec3(0.0))));
            let weight = KERNEL[x + 2] * KERNEL[y + 2] * facing * exp(-distance / variance);
            curvature += bitcast<f32>(before[neighbor * 2u]) * weight;
            total += weight;
        }
    }
    after[offset] = before[offset];
    if (total > 0.0) {
        after[offset] = bitcast<u32>(curvature / total);
    }
}

@group(0) @binding(0) var<storage, read> guides: array<Guide>;
@group(0) @binding(1) var<storage, read> before: array<u32>;
@group(0) @binding(2) var<storage, read_write> after: array<u32>;
@group(0) @binding(3) var<uniform> settings: Settings;

const KERNEL = array(0.0625, 0.25, 0.375, 0.25, 0.0625);

fn unpack(word: u32) -> vec4<f32> {
    return vec4<f32>((vec4(word) >> vec4(0u, 8u, 16u, 24u)) & vec4(255u));
}

fn pack(bytes: vec4<f32>) -> u32 {
    let b = vec4<u32>(floor(clamp(bytes, vec4(0.0), vec4(255.0)) + vec4(0.5)));
    return b.x | (b.y << 8u) | (b.z << 16u) | (b.w << 24u);
}

fn decode_normal(word: u32) -> vec3<f32> {
    return unpack(word).xyz * (2.0 / 255.0) - vec3(1.0);
}

fn normal_or(vector: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let inverse_length = inverseSqrt(dot(vector, vector));
    if (inverse_length > 0.0 && inverse_length <= 3.402823e38) {
        return vector * inverse_length;
    }
    return fallback;
}

@compute @workgroup_size(8, 8)
fn denoise(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= settings.width || id.y >= settings.rows) {
        return;
    }
    let index = id.y * settings.width + id.x;
    let center = guides[index];
    let offset = index * 2u;
    if (center.texel <= 0.0) {
        after[offset] = before[offset];
        after[offset + 1u] = before[offset + 1u];
        return;
    }
    let position = vec3(center.x, center.y, center.z);
    let normal = normal_or(decode_normal(center.normal), vec3(0.0));
    let size = vec2<i32>(i32(settings.width), i32(settings.rows));
    var channels = vec4(0.0);
    var bent = vec3(0.0);
    var total = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let shift = vec2(x, y) * settings.step;
            let sample = vec2<i32>(id.xy) + shift;
            if (any(sample < vec2(0)) || any(sample >= size)) {
                continue;
            }
            let neighbor = u32(sample.y) * settings.width + u32(sample.x);
            let other = guides[neighbor];
            if (other.texel <= 0.0 || other.part != center.part || other.island != center.island) {
                continue;
            }
            let delta = vec3(other.x, other.y, other.z) - position;
            let radius = (length(vec2<f32>(shift)) * 2.0 + 2.0);
            if (dot(delta, delta) > radius * radius * (center.texel * center.texel)) {
                continue;
            }
            var facing = 1.0;
            if (center.normal != other.normal) {
                facing = clamp(dot(normal, normal_or(decode_normal(other.normal), vec3(0.0))), 0.0, 1.0);
                for (var power = 0; power < 6; power++) {
                    facing = facing * facing;
                }
            }
            let plane = dot(delta, normal) * (1.0 / (center.texel * f32(settings.step)));
            let weight = KERNEL[x + 2] * KERNEL[y + 2] * facing * exp(-4.0 * plane * plane);
            channels += unpack(before[neighbor * 2u]) * weight;
            bent += decode_normal(before[neighbor * 2u + 1u]) * weight;
            total += weight;
        }
    }
    if (total < 1e-6) {
        after[offset] = before[offset];
        after[offset + 1u] = before[offset + 1u];
    } else {
        after[offset] = pack(channels / total);
        after[offset + 1u] = pack(vec4(normal_or(bent, normal) * 127.5 + vec3(127.5), 0.0));
    }
}
