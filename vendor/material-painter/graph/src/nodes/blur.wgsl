// @name Blur
// @category Filters
// @description Soft Gaussian-like blur, radius in texels.
// @output color
// @input input: color = 0.5
// @param radius: float(0, 64) = 4

fn node(uv: vec2<f32>) -> vec4<f32> {
    if (params.radius < 0.5) {
        return in_input(uv);
    }
    // Vogel disc: evenly spread taps, weighted toward the center.
    let n = 48;
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0; i < n; i++) {
        let r = sqrt((f32(i) + 0.5) / f32(n));
        let a = f32(i) * 2.39996323;
        let o = vec2<f32>(cos(a), sin(a)) * r * params.radius * px();
        let w = exp(-r * r * 2.0);
        sum += in_input(uv + o) * w;
        wsum += w;
    }
    return sum / wsum;
}
