// @name Tile Sampler
// @category Generators
// @description Scatters a shape (or the pattern input) over a grid, with random position, size, rotation, brightness and gaps. Tileable.
// @output gray
// @input pattern: gray = 1
// @param shape: enum(Disc, Square, Diamond, Cone, Pattern Input) = Disc
// @param columns: int(1, 64) = 8
// @param rows: int(1, 64) = 8
// @param size: float(0.05, 2) = 0.8
// @param size_random: float(0, 1) = 0.3
// @param position_random: float(0, 1) = 0.5
// @param rotation_random: float(0, 1) = 0.5
// @param brightness_random: float(0, 1) = 0.4
// @param gaps: float(0, 1) = 0
// @param blend: enum(Max, Add) = Max
// @param seed: int(0, 1000) = 0

fn shape_at(q: vec2<f32>) -> f32 {
    // q spans -0.5..0.5 across one stamp.
    if (abs(q.x) > 0.5 || abs(q.y) > 0.5) {
        return 0.0;
    }
    let p = q * 2.0;
    switch params.shape {
        case 1: { return 1.0; }
        case 2: { return 1.0 - smoothstep(0.96, 1.0, abs(p.x) + abs(p.y)); }
        case 3: { return clamp(1.0 - length(p), 0.0, 1.0); }
        case 4: { return in_pattern(q + 0.5); }
        default: { return 1.0 - smoothstep(0.92, 1.0, length(p)); }
    }
}

fn node(uv: vec2<f32>) -> vec4<f32> {
    let n = vec2<f32>(f32(params.columns), f32(params.rows));
    let base = floor(uv * n);
    var v = 0.0;
    // Stamps can reach into the neighboring cells.
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let c = base + vec2<f32>(f32(x), f32(y));
            let cell = c - n * floor(c / n);
            let h = hash2(cell, f32(params.seed));
            let h2 = hash2(cell, f32(params.seed) + 11.0);
            let h3 = hash2(cell, f32(params.seed) + 23.0);
            if (h3.x < params.gaps) {
                continue;
            }
            let center = (c + 0.5 + (h - 0.5) * params.position_random) / n;
            let scale = params.size * (1.0 - params.size_random * h2.x) / n;
            var d = uv - center;
            d -= round(d);
            let q = rotate2(d, -(h2.y - 0.5) * TAU * params.rotation_random) / scale;
            let s = shape_at(q) * (1.0 - params.brightness_random * h3.y);
            if (params.blend == 1) {
                v += s;
            } else {
                v = max(v, s);
            }
        }
    }
    return vec4<f32>(clamp(v, 0.0, 1.0));
}
