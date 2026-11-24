// @name Smart Mask
// @category Material
// @description Where wear, dirt and dust land, from the model's shape: up to four inputs (edges, cavities, sky exposure, noise...) mixed by weight, broken up by noise, then cut by a level and contrast. The masks smart materials use.
// @surface
// @output gray
// @param input_1: enum(None, Edges, Convexity, Concavity, Occlusion, Cavity, Sky exposure, Facing up, Thin areas, Part gradient, World height, Part random, Creases, Streaks, Noise, ID match, Overhangs, Wood grain, Cells, Scratches, Front distance) = Edges
// @param weight_1: float(-4, 4) = 1
// @param input_2: enum(None, Edges, Convexity, Concavity, Occlusion, Cavity, Sky exposure, Facing up, Thin areas, Part gradient, World height, Part random, Creases, Streaks, Noise, ID match, Overhangs, Wood grain, Cells, Scratches, Front distance) = None
// @param weight_2: float(-4, 4) = 1
// @param input_3: enum(None, Edges, Convexity, Concavity, Occlusion, Cavity, Sky exposure, Facing up, Thin areas, Part gradient, World height, Part random, Creases, Streaks, Noise, ID match, Overhangs, Wood grain, Cells, Scratches, Front distance) = None
// @param weight_3: float(-4, 4) = 1
// @param input_4: enum(None, Edges, Convexity, Concavity, Occlusion, Cavity, Sky exposure, Facing up, Thin areas, Part gradient, World height, Part random, Creases, Streaks, Noise, ID match, Overhangs, Wood grain, Cells, Scratches, Front distance) = None
// @param weight_4: float(-4, 4) = 1
// @param level: float(-1, 4) = 0.5
// @param contrast: float(0, 1) = 0.1
// @param scale: float(0.1, 80) = 4
// @param noise: float(0, 2) = 0.5
// @param edge_width: float(0.001, 0.2) = 0.02
// @param seed: float(0, 100) = 0
// @param invert: bool = false

// One weighted input (index 0: none).
fn pick(i: i32, w: f32, s: MaskSurface) -> f32 {
    if (i <= 0 || w == 0.0) {
        return 0.0;
    }
    return w * mask_input(i - 1, s);
}

fn node(uv: vec2<f32>) -> vec4<f32> {
    var s = surface(uv);
    s.scale = params.scale;
    s.seed = params.seed;
    s.width = params.edge_width;
    let v = pick(params.input_1, params.weight_1, s) + pick(params.input_2, params.weight_2, s)
        + pick(params.input_3, params.weight_3, s) + pick(params.input_4, params.weight_4, s);
    return vec4<f32>(mask_finish(v, s, params.noise, params.level, params.contrast, params.invert));
}
