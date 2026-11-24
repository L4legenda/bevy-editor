// @name Tiles
// @category Generators
// @description Bricks or tiles: rows offset by a fraction, mortar gaps, bevelled edges and a random brightness per tile.
// @output gray
// @param columns: int(1, 64) = 4
// @param rows: int(1, 64) = 8
// @param offset: float(0, 1) = 0.5
// @param gap: float(0, 0.4) = 0.04
// @param bevel: float(0, 0.5) = 0.08
// @param variation: float(0, 1) = 0.3
// @param seed: int(0, 1000) = 0

fn node(uv: vec2<f32>) -> vec4<f32> {
    let n = vec2<f32>(f32(params.columns), f32(params.rows));
    var p = uv * n;
    let row = floor(p.y);
    p.x += params.offset * row;
    let col = floor(p.x) - n.x * floor(floor(p.x) / n.x);
    let f = fract(p);
    // Distance to the tile's edge, in tile heights (tiles are n.y/n.x wide).
    let aspect = n.y / n.x;
    let edge = min(min(f.x, 1.0 - f.x) * aspect, min(f.y, 1.0 - f.y));
    let h = smoothstep(params.gap * 0.5, params.gap * 0.5 + max(params.bevel, 1e-4), edge);
    let lum = 1.0 - params.variation * hash1(vec2<f32>(col, row), f32(params.seed));
    return vec4<f32>(h * lum);
}
