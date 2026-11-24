// @name Cells
// @category Generators
// @description Tileable Worley cells: distance to the cell center, the edges between cells, or a random value per cell.
// @output gray
// @param mode: enum(Distance, Edges, Random) = Distance
// @param scale: int(1, 128) = 8
// @param jitter: float(0, 1) = 1
// @param seed: int(0, 1000) = 0

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = cells(uv, f32(params.scale), params.jitter, f32(params.seed));
    switch params.mode {
        case 1: { return vec4<f32>(clamp((c.y - c.x) * 2.0, 0.0, 1.0)); }
        case 2: { return vec4<f32>(c.z); }
        default: { return vec4<f32>(clamp(c.x, 0.0, 1.0)); }
    }
}
