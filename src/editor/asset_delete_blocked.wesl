import bevy_ui_render::ui_vertex_output::UiVertexOutput;

// Seconds since rejection, display scale, eased opacity, reserved.
@group(1) @binding(0) var<uniform> state: vec4<f32>;

fn rounded_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t = state.x;
    let opacity = state.z;
    if opacity <= 0.0001 { return vec4<f32>(0.0); }
    let size = max(in.size / max(state.y, 0.5), vec2<f32>(1.0));
    let p = (in.uv - vec2<f32>(0.5)) * size;
    let edge = rounded_box(p, size * 0.5 - vec2<f32>(1.5), 8.0);
    let aa = max(fwidth(edge), 0.5);
    let inside = 1.0 - smoothstep(-aa, aa, edge);

    let gold = vec3<f32>(1.0, 0.77, 0.39);
    let coral = vec3<f32>(1.0, 0.30, 0.22);
    let pearl = vec3<f32>(1.0, 0.92, 0.76);
    let warm = mix(coral, gold, 0.5 + 0.5 * sin(p.y * 0.014 + t * 1.4));
    let impact = exp(-pow((t - 0.16) / 0.14, 2.0))
        + 0.45 * exp(-pow((t - 0.46) / 0.18, 2.0));
    let rim = exp(-abs(edge) * 1.1);
    let glow = exp(-abs(edge) * 0.13);
    let sweep = exp(-pow((p.y + p.x * 0.35 + size.y * 0.65 - t * 210.0) / 12.0, 2.0));
    let radial = length(p - vec2<f32>(0.0, -28.0));
    let wave = exp(-pow((radial - t * 210.0) / 4.0, 2.0)) * exp(-t * 3.8);

    let caption = smoothstep(-2.0, 24.0, p.y);
    var alpha = inside * mix(0.82 + 0.05 * impact, 0.99, caption);
    var light = vec3<f32>(0.012, 0.010, 0.017) * alpha;
    let field = (rim * (0.36 + 0.55 * impact) + glow * (0.045 + 0.16 * impact)
        + sweep * 0.09 + wave * 0.45) * inside;
    light += warm * field;
    alpha += field * (1.0 - alpha);

    // A small recoil settles into a protective, slowly lit seal.
    let recoil = sin(t * 55.0) * exp(-t * 12.0) * 3.0;
    let scale = 1.0 - 0.13 * exp(-t * 11.0);
    let q = (p - vec2<f32>(recoil, -28.0)) / scale;
    let r = length(q);
    let ring_edge = abs(r - 26.0);
    let ring = exp(-ring_edge * 1.5);
    let arc = pow(0.5 + 0.5 * cos(atan2(q.y, q.x) - t * 2.0), 7.0);
    let halo = exp(-dot(q, q) / 700.0);
    let seal = (ring * (0.12 + arc * 0.58) + halo * 0.07) * inside;
    light += gold * seal;
    alpha += seal * (1.0 - alpha);

    // The lock is drawn as signed distances, so it stays crisp at any DPI.
    let body = rounded_box(q - vec2<f32>(0.0, 5.0), vec2<f32>(9.0, 7.5), 2.0);
    let latch = max(abs(length(q - vec2<f32>(0.0, -4.0)) - 6.5) - 0.9, q.y + 3.0);
    let stroke = min(abs(body) - 0.85, latch);
    let ink = 1.0 - smoothstep(-0.4, 0.55, stroke);
    let keyhole = min(length(q - vec2<f32>(0.0, 3.0)) - 1.3,
        rounded_box(q - vec2<f32>(0.0, 6.0), vec2<f32>(0.8, 2.0), 0.6));
    let key = 1.0 - smoothstep(-0.3, 0.5, keyhole);
    let symbol = max(ink, key) * inside;
    light += mix(gold, pearl, 0.55) * symbol;
    alpha += symbol * (1.0 - alpha);
    return vec4<f32>(light / max(alpha, 0.0001), min(alpha, 1.0) * opacity);
}
