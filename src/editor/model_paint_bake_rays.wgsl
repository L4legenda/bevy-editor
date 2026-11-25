// Mesh visibility pass. UV projection, relief and denoising run outside this pass.
struct Node {
    lo: vec4<f32>,
    hi: vec4<f32>,
    links: vec4<u32>,
}
struct Triangle {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    ids: vec4<u32>,
}
struct Texel {
    p: vec4<f32>, // position, texel width
    n: vec4<f32>,
    geo: vec4<f32>,
    ids: vec4<u32>, // raster index, object, face, connected part
}
struct Config {
    counts: vec4<u32>, // samples per map, number of texels
    settings: vec4<f32>, // scale, low-poly part IDs, visibility maps, number of recipes
    geometry: vec4<u32>, // resolution as f32 bits, triangle stride in vectors, selected maps
    recipes: array<Recipe, 64>,
}
struct Recipe {
    direction_distance: vec4<f32>,
    point_cosine: vec4<f32>,
    kind: vec4<u32>, // kind, fluid strength and wander as f32 bits
}
struct RayHit {
    distance: f32,
    triangle: u32,
}

@group(0) @binding(0) var<storage, read> nodes: array<Node>;
@group(0) @binding(1) var<storage, read> triangles: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> input_texels: array<Texel>;
@group(0) @binding(3) var<storage, read_write> output_texels: array<u32>;
@group(0) @binding(4) var<uniform> config: Config;
@group(0) @binding(5) var<storage, read_write> visibility_atlas: array<vec2<u32>>;

const NONE: u32 = 0xffffffffu;
const OPEN: f32 = 3.402823e38;
const GOLDEN_ANGLE: f32 = 2.3999632;

fn hash_pixel(value: u32) -> u32 {
    var bits = (value ^ (value >> 16u)) * 0x7feb352du;
    bits = (bits ^ (bits >> 15u)) * 0x846ca68bu;
    return bits ^ (bits >> 16u);
}

fn box_entry(index: u32, origin: vec3<f32>, inverse: vec3<f32>, parallel: vec3<bool>, limit: f32) -> f32 {
    let lo = nodes[index].lo.xyz;
    let hi = nodes[index].hi.xyz;
    if (any(parallel & ((origin < lo) | (origin > hi)))) {
        return OPEN;
    }
    let a = (lo - origin) * inverse;
    let b = (hi - origin) * inverse;
    let near = select(min(a, b), vec3(0.0), parallel);
    let far = select(max(a, b), vec3(limit), parallel);
    let entry = max(0.0, max(near.x, max(near.y, near.z)));
    let exit = min(limit, min(far.x, min(far.y, far.z)));
    return select(OPEN, entry, entry <= exit);
}

fn intersect_triangle(origin: vec3<f32>, direction: vec3<f32>, triangle: Triangle) -> f32 {
    let a = triangle.b.xyz - triangle.a.xyz;
    let b = triangle.c.xyz - triangle.a.xyz;
    let q = cross(direction, b);
    let determinant = dot(a, q);
    if (abs(determinant) < 1.0e-12) {
        return -1.0;
    }
    let offset = origin - triangle.a.xyz;
    let u = dot(offset, q) / determinant;
    if (u < 0.0 || u > 1.0) {
        return -1.0;
    }
    let r = cross(offset, a);
    let v = dot(direction, r) / determinant;
    if (v < 0.0 || u + v > 1.0) {
        return -1.0;
    }
    return dot(b, r) / determinant;
}

fn triangle_at(index: u32) -> Triangle {
    let first = index * config.geometry.y;
    return Triangle(bitcast<vec4<f32>>(triangles[first]), bitcast<vec4<f32>>(triangles[first + 1u]),
        bitcast<vec4<f32>>(triangles[first + 2u]), triangles[first + 3u]);
}

fn trace_hit(origin: vec3<f32>, direction: vec3<f32>, limit: f32, ids: vec4<u32>, part_filter: u32, exit_at: f32) -> RayHit {
    let parallel = abs(direction) < vec3(1.0e-12);
    let inverse = vec3(1.0) / select(direction, vec3(1.0), parallel);
    let root_entry = box_entry(0u, origin, inverse, parallel, limit);
    if (root_entry == OPEN) {
        return RayHit(OPEN, NONE);
    }
    var stack: array<vec2<u32>, 64>;
    // Keep the entry distance so a deferred child can be culled without another slab test.
    stack[0] = vec2(0u, bitcast<u32>(root_entry));
    var len = 1u;
    var closest = limit;
    var hit = NONE;
    var hit_leaf = 0u;
    while (len != 0u) {
        len -= 1u;
        let pending = stack[len];
        if (bitcast<f32>(pending.y) > closest) {
            continue;
        }
        let node = nodes[pending.x];
        if (node.links.x != NONE) {
            let left = node.links.x;
            let right = node.links.y;
            let left_entry = box_entry(left, origin, inverse, parallel, closest);
            let right_entry = box_entry(right, origin, inverse, parallel, closest);
            if (left_entry < OPEN && right_entry < OPEN) {
                let left_nearer = left_entry < right_entry;
                let left_item = vec2(left, bitcast<u32>(left_entry));
                let right_item = vec2(right, bitcast<u32>(right_entry));
                stack[len] = select(left_item, right_item, left_nearer);
                stack[len + 1u] = select(right_item, left_item, left_nearer);
                len += 2u;
            } else if (left_entry < OPEN) {
                stack[len] = vec2(left, bitcast<u32>(left_entry));
                len += 1u;
            } else if (right_entry < OPEN) {
                stack[len] = vec2(right, bitcast<u32>(right_entry));
                len += 1u;
            }
        } else {
            for (var i = node.links.z; i < node.links.w; i++) {
                let triangle = triangle_at(i);
                let same_face = triangle.ids.x == ids.y && triangle.ids.y == ids.z;
                let same_part = triangle.ids.z == ids.w && (config.settings.y == 0.0 || triangle.ids.x == ids.y);
                if ((part_filter < 3u && same_face) || (part_filter == 1u && !same_part) || (part_filter == 2u && same_part)) {
                    continue;
                }
                if (part_filter >= 4u && (triangle.ids.w == 0u || (part_filter == 4u
                    && dot(cross(triangle.b.xyz - triangle.a.xyz, triangle.c.xyz - triangle.a.xyz), direction) >= 0.0))) {
                    continue;
                }
                let distance = intersect_triangle(origin, direction, triangle);
                // Node IDs grow left-to-right. Use the original right-first tie break.
                let wins_tie = distance == closest && (hit == NONE || pending.x > hit_leaf);
                if (distance >= 0.0 && (distance < closest || wins_tie)) {
                    if (distance <= exit_at) {
                        return RayHit(distance, i);
                    }
                    closest = distance;
                    hit = i;
                    hit_leaf = pending.x;
                }
            }
        }
    }
    return RayHit(select(OPEN, closest, hit != NONE), hit);
}

fn trace(origin: vec3<f32>, direction: vec3<f32>, limit: f32, ids: vec4<u32>, part_filter: u32) -> f32 {
    return trace_hit(origin, direction, limit, ids, part_filter, -1.0).distance;
}

fn blocked(origin: vec3<f32>, direction: vec3<f32>, limit: f32, ids: vec4<u32>, part_filter: u32) -> bool {
    return trace_hit(origin, direction, limit, ids, part_filter, limit).triangle != NONE;
}

fn tangent_to(normal: vec3<f32>) -> vec3<f32> {
    let sign_z = select(1.0, -1.0, (bitcast<u32>(normal.z) & 0x80000000u) != 0u);
    let a = -1.0 / (sign_z + normal.z);
    return vec3<f32>(normal.x * normal.y * a, sign_z + normal.y * normal.y * a, -normal.y);
}

fn hemisphere(normal: vec3<f32>, sample_id: u32, count: u32, phase: f32) -> vec3<f32> {
    let radius = sqrt((f32(sample_id) + 0.5) / f32(count));
    let angle = phase + f32(sample_id) * GOLDEN_ANGLE;
    let tangent = tangent_to(normal);
    let bitangent = cross(normal, tangent);
    return normal * sqrt(1.0 - radius * radius)
        + radius * (tangent * cos(angle) + bitangent * sin(angle));
}

fn byte(value: f32) -> u32 {
    return u32(floor(clamp(value, 0.0, 1.0) * 255.0 + 0.5));
}

fn visibility_maps(texel: Texel) -> vec2<u32> {
    let geo = texel.geo.xyz;
    let normal = texel.n.xyz;
    let scale = config.settings.x;
    let phase = f32(hash_pixel(texel.ids.x) >> 8u) * (6.28318530718 / 16777216.0);
    let bias = geo * (scale * 0.00002);
    let outside = texel.p.xyz + bias;
    let inside = texel.p.xyz - bias;
    let cavity_reach = scale * 0.04;
    let maps = config.geometry.z;
    let ao_enabled = (maps & 1u) != 0u;
    let cavity_enabled = (maps & 2u) != 0u;
    let thickness_enabled = (maps & 4u) != 0u;
    let sky_enabled = (maps & 8u) != 0u;
    let ao_reach = select(cavity_reach, scale * 0.6, ao_enabled);
    let ao_exit = select(ao_reach, cavity_reach, cavity_enabled);
    let thickness_reach = scale * 0.5;
    var ao = 0u;
    var cavity = 0u;
    var bent = vec3<f32>(0.0);
    let ao_count = select(0u, config.counts.x, ao_enabled || cavity_enabled);
    for (var i = 0u; i < ao_count; i++) {
        var direction = hemisphere(normal, i, config.counts.x, phase);
        let incidence = dot(direction, geo);
        if (incidence < 0.0) {
            direction = normalize(direction - 2.0 * incidence * geo);
        }
        // A hit inside the smaller reach marks both visibility maps as blocked.
        let hit = trace_hit(outside, direction, ao_reach, texel.ids, 0u, ao_exit).distance;
        if (hit > cavity_reach) {
            cavity += 1u;
        }
        if (ao_enabled && hit > ao_reach) {
            ao += 1u;
            bent += direction;
        }
    }
    var thickness = 0.0;
    let thickness_count = select(0u, config.counts.y, thickness_enabled);
    for (var i = 0u; i < thickness_count; i++) {
        var direction = hemisphere(-normal, i, config.counts.y, phase + 1.0);
        let incidence = dot(direction, geo);
        if (incidence > 0.0) {
            direction = normalize(direction - 2.0 * incidence * geo);
        }
        thickness += min(thickness_reach,
            trace(inside, direction, thickness_reach, texel.ids, select(0u, 1u, config.settings.y != 0.0)));
    }
    var sky = 0u;
    let cone_cos = cos(40.0 * (3.14159265359 / 180.0));
    let sky_count = select(0u, config.counts.z, sky_enabled);
    for (var i = 0u; i < sky_count; i++) {
        let y = 1.0 - (1.0 - cone_cos) * ((f32(i) + 0.5) / f32(config.counts.z));
        let radius = sqrt(1.0 - y * y);
        let angle = phase + f32(i) * GOLDEN_ANGLE;
        let direction = vec3<f32>(radius * cos(angle), y, radius * sin(angle));
        if (dot(normal, direction) > 0.0 && dot(geo, direction) > 0.0
            && !blocked(outside, direction, OPEN, texel.ids, 0u)) {
            sky += 1u;
        }
    }
    var bent_direction = normal;
    if (dot(bent, bent) > 0.0) {
        bent_direction = normalize(bent);
    }
    let coded = bent_direction * 0.5 + 0.5;
    let visibility = select(255u, byte(f32(ao) / f32(config.counts.x)), ao_enabled)
        | (select(255u, byte(f32(cavity) / f32(config.counts.x)), cavity_enabled) << 8u)
        | (select(255u, byte(thickness / (thickness_reach * f32(config.counts.y))), thickness_enabled) << 16u)
        | (select(255u, byte(f32(sky) / f32(config.counts.z)), sky_enabled) << 24u);
    let bent_normal = byte(coded.x) | (byte(coded.y) << 8u) | (byte(coded.z) << 16u);
    return vec2<u32>(visibility, bent_normal);
}

fn cone(axis: vec3<f32>, cosine: f32, index: u32, count: u32, shift: vec2<f32>) -> vec3<f32> {
    let u = fract(f32(index) / f32(count) + shift.x);
    let v = fract(f32(reverseBits(index)) * (1.0 / 4294967296.0) + shift.y);
    let axial = 1.0 - u * (1.0 - cosine);
    let radius = sqrt(max(0.0, 1.0 - axial * axial));
    let angle = v * 6.28318530718;
    let tangent = tangent_to(axis);
    return axis * axial + tangent * (radius * cos(angle)) + cross(axis, tangent) * (radius * sin(angle));
}

fn lattice_hash(point: vec3<f32>) -> u32 {
    let cell = bitcast<vec3<u32>>(vec3<i32>(floor(point)));
    return hash_pixel(cell.x * 73856093u ^ cell.y * 19349663u ^ cell.z * 83492791u);
}

fn fraction(bits: u32) -> f32 {
    return f32(bits & 0xffffffu) / 16777216.0;
}

fn surface_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(fraction(lattice_hash(i)), fraction(lattice_hash(i + vec3<f32>(1.0, 0.0, 0.0))), w.x);
    let b = mix(fraction(lattice_hash(i + vec3<f32>(0.0, 1.0, 0.0))),
        fraction(lattice_hash(i + vec3<f32>(1.0, 1.0, 0.0))), w.x);
    let c = mix(fraction(lattice_hash(i + vec3<f32>(0.0, 0.0, 1.0))),
        fraction(lattice_hash(i + vec3<f32>(1.0, 0.0, 1.0))), w.x);
    let d = mix(fraction(lattice_hash(i + vec3<f32>(0.0, 1.0, 1.0))),
        fraction(lattice_hash(i + vec3<f32>(1.0, 1.0, 1.0))), w.x);
    return mix(mix(a, b, w.y), mix(c, d, w.y), w.z);
}

struct Landing {
    valid: bool,
    p: vec3<f32>,
    n: vec3<f32>,
}

fn land(origin: vec3<f32>, direction: vec3<f32>, limit: f32) -> Landing {
    let hit = trace_hit(origin, direction, limit, vec4<u32>(0u), 3u, -1.0);
    if (hit.triangle == NONE) {
        return Landing(false, vec3<f32>(0.0), vec3<f32>(0.0));
    }
    let point = origin + direction * hit.distance;
    let triangle = triangle_at(hit.triangle);
    let ab = triangle.b.xyz - triangle.a.xyz;
    let ac = triangle.c.xyz - triangle.a.xyz;
    let area = cross(ab, ac);
    let area_squared = dot(area, area);
    var normal = -direction;
    if (area_squared > 0.0) {
        normal = normalize(area);
        let local = point - triangle.a.xyz;
        let v = dot(cross(local, ac), area) / area_squared;
        let w = dot(cross(ab, local), area) / area_squared;
        let first = hit.triangle * config.geometry.y + 4u;
        let a = bitcast<vec4<f32>>(triangles[first]).xyz;
        let b = bitcast<vec4<f32>>(triangles[first + 1u]).xyz;
        let c = bitcast<vec4<f32>>(triangles[first + 2u]).xyz;
        let interpolated = a * (1.0 - v - w) + b * v + c * w;
        if (dot(interpolated, interpolated) > 0.0) {
            normal = normalize(interpolated);
        }
    }
    return Landing(true, point, select(normal, -normal, dot(normal, direction) > 0.0));
}

// Follow a concave wall, stay on the face, or turn around a convex edge.
fn surface_step(current: Landing, tangent: vec3<f32>, step: f32) -> Landing {
    let lift = step * 0.25;
    let start = current.p + current.n * lift;
    let forward = start + tangent * step;
    let down = lift + step * 2.0;
    var next = land(start, tangent, step);
    if (next.valid) {
        return next;
    }
    next = land(forward, -current.n, down);
    if (next.valid) {
        return next;
    }
    return land(forward - current.n * down, -tangent, step * 2.0);
}

const FLOW_STEPS: u32 = 32u;

fn flow_map(texel: Texel, recipe: Recipe, bias: f32) -> u32 {
    let distance_along = recipe.direction_distance.w;
    let source = recipe.direction_distance.xyz;
    let cosine = recipe.point_cosine.w;
    let strength = bitcast<f32>(recipe.kind.y);
    let wander = bitcast<f32>(recipe.kind.z);
    let walks = clamp(config.counts.z / 8u, 2u, 8u);
    let samples = walks * FLOW_STEPS;
    let step = distance_along / f32(FLOW_STEPS);
    let frequency = 6.0 / max(distance_along, 1e-6);
    let world_texel = config.settings.x * 2.0 / bitcast<f32>(config.geometry.x);
    let seed = hash_pixel(lattice_hash(texel.p.xyz / world_texel) ^ 0x3c6ef372u);
    let shift = vec2<f32>(fract(fraction(seed) + 0.5 / f32(samples)), fraction(hash_pixel(seed ^ 0x9e3779b9u)));
    var total = 0.0;
    for (var walk = 0u; walk < walks; walk++) {
        var current = Landing(true, texel.p.xyz, texel.n.xyz);
        var heading = vec3<f32>(0.0);
        var gathered = 0.0;
        let noise_offset = vec3<f32>(17.13, 5.71, 11.37) * f32(walk);
        for (var index = 0u; index < FLOW_STEPS; index++) {
            let direction = cone(source, cosine, walk * FLOW_STEPS + index, samples, shift);
            let facing = dot(current.n, direction);
            var wet = 0.0;
            if (facing > 0.0 && !blocked(current.p + current.n * bias, direction, OPEN, texel.ids, 3u)) {
                wet = facing;
            }
            gathered += wet;
            var upstream = source - current.n * dot(current.n, source);
            if (dot(upstream, upstream) < 0.04) {
                upstream = heading - current.n * dot(current.n, heading);
            }
            if (dot(upstream, upstream) < 1e-8) {
                gathered += wet * f32(FLOW_STEPS - 1u - index);
                break;
            }
            upstream = normalize(upstream);
            if (wander > 0.0) {
                let angle = (surface_noise(current.p * frequency + noise_offset) - 0.5) * 2.0 * wander;
                upstream = upstream * cos(angle) + cross(current.n, upstream) * sin(angle);
            }
            heading = upstream;
            let next = surface_step(current, upstream, step);
            if (!next.valid) {
                break;
            }
            current = next;
        }
        total += 1.0 - exp(-strength * gathered / f32(FLOW_STEPS));
    }
    return byte(total / f32(walks));
}

fn custom_map(texel: Texel, recipe: Recipe) -> u32 {
    let p = texel.p.xyz;
    let normal = texel.n.xyz;
    let kind = recipe.kind.x;
    let reach = recipe.direction_distance.w;
    if (kind == 5u) {
        return byte(1.0 - smoothstep(0.0, 1.0, distance(p, recipe.point_cosine.xyz) / reach));
    }
    let ground = smoothstep(0.0, 1.0, (normal.y - 0.25) / 0.5);
    if (kind == 3u && ground == 0.0) {
        return 0u;
    }
    let scale = config.settings.x;
    let bias = clamp(texel.p.w * 0.125 / scale, 0.00001, 0.004) * scale;
    if (kind == 1u) {
        return flow_map(texel, recipe, bias);
    }
    let origin = p + normal * bias;
    let cell = p / scale * 4096.0;
    let quantized = bitcast<vec3<u32>>(vec3<i32>(sign(cell) * floor(abs(cell) + 0.5)));
    let seed = hash_pixel(quantized.x ^ hash_pixel(quantized.y) ^ hash_pixel(quantized.z));
    let shift = vec2<f32>(f32(seed >> 8u), f32(hash_pixel(seed) >> 8u)) / 16777216.0;
    let count = max(config.counts.z, 8u);
    let axis = select(recipe.direction_distance.xyz, normal, kind == 4u);
    let cosine = select(recipe.point_cosine.w, 0.0, kind == 4u);
    var result = 0.0;
    var nearest = reach;
    for (var index = 0u; index < count; index++) {
        var direction = cone(axis, cosine, index, count, shift);
        let facing = dot(direction, normal);
        if (kind == 2u) {
            if (facing > 0.0 && !blocked(origin, direction, reach, texel.ids, 0u)) {
                result += facing;
            }
        } else if (kind == 3u) {
            if (facing < 0.0) {
                direction -= 2.0 * facing * normal;
            }
            if (blocked(origin, direction, reach, texel.ids, 0u)) {
                result += ground;
            }
        } else if (kind == 4u) {
            nearest = min(nearest, trace(origin, direction, nearest, texel.ids, 2u));
        }
    }
    if (kind == 4u) {
        let proximity = 1.0 - nearest / reach;
        return byte(proximity * proximity);
    }
    return byte(result / f32(count));
}

@compute @workgroup_size(64)
fn trace_maps(@builtin(global_invocation_id) global: vec3<u32>) {
    let index = global.x;
    if (index >= config.counts.w) {
        return;
    }
    let texel = input_texels[index];
    let recipe_count = u32(config.settings.w);
    let offset = index * (2u + recipe_count);
    if (config.settings.z != 0.0) {
        let levels = visibility_maps(texel);
        output_texels[offset] = levels.x;
        output_texels[offset + 1u] = levels.y;
    }
    for (var recipe = 0u; recipe < recipe_count; recipe++) {
        output_texels[offset + 2u + recipe] = custom_map(texel, config.recipes[recipe]);
    }
}

@compute @workgroup_size(64)
fn trace_resident_maps(@builtin(global_invocation_id) global: vec3<u32>) {
    let index = global.x;
    if (index >= config.counts.w) {
        return;
    }
    let texel = input_texels[index];
    let levels = visibility_maps(texel);
    // Alpha marks pixels written by the ray pass, including those without a filter guide.
    visibility_atlas[texel.ids.x] = vec2(levels.x, levels.y | 0xff000000u);
    let recipe_count = u32(config.settings.w);
    let offset = index * recipe_count;
    for (var recipe = 0u; recipe < recipe_count; recipe++) {
        output_texels[offset + recipe] = custom_map(texel, config.recipes[recipe]);
    }
}

fn nearest_edge(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let edge = b - a;
    let length_squared = dot(edge, edge);
    if (length_squared == 0.0) {
        return a;
    }
    return a + edge * clamp(dot(p - a, edge) / length_squared, 0.0, 1.0);
}

fn nearest_on_triangle(p: vec3<f32>, triangle: Triangle) -> vec3<f32> {
    let a = triangle.a.xyz;
    let b = triangle.b.xyz;
    let c = triangle.c.xyz;
    let n = normalize(cross(b - a, c - a));
    let on_plane = p - n * dot(p - a, n);
    if (dot(cross(b - a, on_plane - a), n) >= 0.0
        && dot(cross(c - b, on_plane - b), n) >= 0.0
        && dot(cross(a - c, on_plane - c), n) >= 0.0) {
        return on_plane;
    }
    let ab = nearest_edge(p, a, b);
    let bc = nearest_edge(p, b, c);
    let ca = nearest_edge(p, c, a);
    let first = select(ab, bc, dot(bc - p, bc - p) < dot(ab - p, ab - p));
    return select(first, ca, dot(ca - p, ca - p) < dot(first - p, first - p));
}

fn box_distance(index: u32, p: vec3<f32>) -> f32 {
    let node = nodes[index];
    let delta = clamp(p, node.lo.xyz, node.hi.xyz) - p;
    return dot(delta, delta);
}

struct Closest {
    triangle: u32,
    point: vec3<f32>,
}

fn closest_surface(p: vec3<f32>, reach: f32) -> Closest {
    var result = Closest(NONE, vec3<f32>(0.0));
    var best = reach * reach;
    var stack: array<u32, 64>;
    stack[0] = 0u;
    var len = 1u;
    while (len != 0u) {
        len -= 1u;
        let index = stack[len];
        if (box_distance(index, p) > best) {
            continue;
        }
        let node = nodes[index];
        if (node.links.x != NONE) {
            let left = node.links.x;
            let right = node.links.y;
            let left_first = box_distance(left, p) < box_distance(right, p);
            stack[len] = select(left, right, left_first);
            stack[len + 1u] = select(right, left, left_first);
            len += 2u;
        } else {
            for (var i = node.links.z; i < node.links.w; i++) {
                let triangle = triangle_at(i);
                if (triangle.ids.w == 0u) {
                    continue;
                }
                let point = nearest_on_triangle(p, triangle);
                let delta = point - p;
                let squared = dot(delta, delta);
                if (squared <= best) {
                    best = squared;
                    result = Closest(i, point);
                }
            }
        }
    }
    return result;
}

@compute @workgroup_size(64)
fn project_queries(@builtin(global_invocation_id) global: vec3<u32>) {
    if (global.x >= config.counts.w) {
        return;
    }
    let probe = input_texels[global.x];
    let low = probe.p.xyz;
    let range = probe.p.w;
    let direction = -probe.n.xyz;
    let front = probe.n.w;
    let origin = probe.geo.xyz;
    let limit = front + probe.geo.w;
    let prefer_front_faces = (probe.ids.w & 2u) != 0u;
    var hit = trace_hit(origin, direction, limit, vec4<u32>(0u), select(5u, 4u, prefer_front_faces), -1.0);
    if (prefer_front_faces && hit.triangle == NONE) {
        hit = trace_hit(origin, direction, limit, vec4<u32>(0u), 5u, -1.0);
    }
    var face = NONE;
    var position = vec3<f32>(0.0);
    var height = 0.0;
    let direct = hit.triangle != NONE;
    if (direct) {
        face = triangle_at(hit.triangle).ids.y;
        position = origin + direction * hit.distance;
        height = (front - hit.distance) / range;
    } else if ((probe.ids.w & 1u) != 0u) {
        let closest = closest_surface(low, range * 4.0);
        if (closest.triangle != NONE) {
            face = triangle_at(closest.triangle).ids.y;
            position = closest.point;
            height = dot(position - low, bitcast<vec3<f32>>(probe.ids.xyz)) / range;
        }
    }
    let offset = global.x * 6u;
    output_texels[offset] = face;
    output_texels[offset + 1u] = bitcast<u32>(position.x);
    output_texels[offset + 2u] = bitcast<u32>(position.y);
    output_texels[offset + 3u] = bitcast<u32>(position.z);
    output_texels[offset + 4u] = bitcast<u32>(height);
    output_texels[offset + 5u] = u32(direct);
}
