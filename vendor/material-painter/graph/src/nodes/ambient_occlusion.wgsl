// @name Ambient Occlusion
// @category Filters
// @description Darkens the low spots of a height map that are surrounded by higher ground.
// @output gray
// @input height: gray = 0.5
// @param radius: float(1, 64) = 12
// @param depth: float(0, 4) = 1
// @param intensity: float(0, 4) = 1.5

fn node(uv: vec2<f32>) -> vec4<f32> {
    let h0 = in_height(uv);
    let n = 32;
    var occ = 0.0;
    for (var i = 0; i < n; i++) {
        let t = (f32(i) + 0.5) / f32(n);
        let r = t * params.radius;
        let a = f32(i) * 2.39996323;
        let o = vec2<f32>(cos(a), sin(a)) * r * px();
        // How steeply the neighbor rises above this point.
        let rise = (in_height(uv + o) - h0) * params.depth * params.radius / max(r, 1.0);
        occ += clamp(rise, 0.0, 1.0);
    }
    return vec4<f32>(clamp(1.0 - occ / f32(n) * params.intensity, 0.0, 1.0));
}
