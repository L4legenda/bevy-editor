// Keep the same source texel for every map at an island boundary.
struct Band {
    width: u32,
    words_per_row: u32,
    source_rows: u32,
    first_output_row: u32,
    output_rows: u32,
}

@group(0) @binding(0) var<storage, read> coverage: array<u32>;
@group(0) @binding(1) var<storage, read_write> offsets: array<u32>;
@group(0) @binding(2) var<uniform> band: Band;

const NEIGHBORS = array(vec2(-1, 0), vec2(0, -1), vec2(1, 0), vec2(0, 1));

fn covered(point: vec2<i32>) -> bool {
    if (any(point < vec2(0)) || point.x >= i32(band.width) || point.y >= i32(band.source_rows)) {
        return false;
    }
    let x = u32(point.x);
    let word = coverage[u32(point.y) * band.words_per_row + x / 4u];
    return ((word >> ((x % 4u) * 8u)) & 255u) != 0u;
}

fn source_offset(point: vec2<i32>) -> u32 {
    if (point.x >= i32(band.width) || covered(point)) {
        return 0u;
    }
    // Search the first dilation pass before looking through its newly filled neighbors.
    for (var i = 0u; i < 4u; i++) {
        let offset = NEIGHBORS[i];
        if (covered(point + offset)) {
            return u32((offset.y + 2) * 5 + offset.x + 3);
        }
    }
    let bounds = vec2<i32>(i32(band.width), i32(band.source_rows));
    for (var i = 0u; i < 4u; i++) {
        let next = point + NEIGHBORS[i];
        if (any(next < vec2(0)) || any(next >= bounds)) {
            continue;
        }
        for (var j = 0u; j < 4u; j++) {
            if (covered(next + NEIGHBORS[j])) {
                let offset = NEIGHBORS[i] + NEIGHBORS[j];
                return u32((offset.y + 2) * 5 + offset.x + 3);
            }
        }
    }
    return 0u;
}

@compute @workgroup_size(8, 8)
fn pad_uv(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= band.words_per_row || id.y >= band.output_rows) {
        return;
    }
    let point = vec2<i32>(i32(id.x * 4u), i32(id.y + band.first_output_row));
    var word = 0u;
    for (var i = 0u; i < 4u; i++) {
        word |= source_offset(point + vec2<i32>(i32(i), 0)) << (i * 8u);
    }
    offsets[id.y * band.words_per_row + id.x] = word;
}
