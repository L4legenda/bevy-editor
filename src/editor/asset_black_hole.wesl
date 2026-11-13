import bevy_ui_render::ui_vertex_output::UiVertexOutput;

// Progress, source width/height in physical pixels, deterministic seed.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
@group(1) @binding(1) var snapshot: texture_2d<f32>;
@group(1) @binding(2) var snapshot_sampler: sampler;

fn rotate(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t = clamp(state.x, 0.0, 1.0);
    if t >= 0.9999 { return vec4<f32>(0.0); }
    let source_size = max(state.yz, vec2<f32>(1.0));
    let unit = min(source_size.x, source_size.y);
    let p = (in.uv - vec2<f32>(0.5)) * in.size / unit;
    let r = length(p);
    let angle = atan2(p.y, p.x);
    let wake = smoothstep(0.0, 0.16, t);
    let closing = 1.0 - smoothstep(0.70, 1.0, t);
    let horizon = 0.095 * wake * closing;

    // Inverse-map the captured card: its edges contract and its centre shears
    // into increasingly tight spiral arms before crossing the event horizon.
    let collapse = pow(max(1.0 - smoothstep(0.04, 0.85, t), 0.001), 1.35);
    let pull = smoothstep(0.0, 0.75, t);
    let twist = pull * (1.0 + 0.36 / max(r + 0.12, 0.12));
    let lens = 1.0 + horizon * horizon / max(r * r, 0.002);
    let q = rotate(p * lens / collapse, -twist);
    let source_uv = q * unit / source_size + vec2<f32>(0.5);
    let edge = min(source_uv, vec2<f32>(1.0) - source_uv);
    let card_clip = smoothstep(0.0, 0.012, min(edge.x, edge.y));
    let sample = textureSampleLevel(snapshot, snapshot_sampler, clamp(source_uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    let outside = smoothstep(horizon * 0.82, horizon * 1.12 + 0.001, r);
    let card_alpha = sample.a * card_clip * outside * (1.0 - smoothstep(0.69, 0.86, t));

    // A tilted accretion disk and a narrow photon ring surround a black core.
    let tilted = rotate(p, -0.30);
    let disk_r = length(vec2<f32>(tilted.x, tilted.y * 2.9));
    let disk_radius = 0.205 * wake * closing;
    let disk_width = max(0.023 * closing, 0.002);
    let disk = exp(-pow((disk_r - disk_radius) / disk_width, 2.0));
    let flow = 0.65 + 0.35 * sin(angle * 3.0 + t * 24.0 + r * 35.0 + state.w);
    let ring = exp(-pow((r - horizon * 1.15) / max(0.009 * closing, 0.001), 2.0));
    let halo = exp(-r * r / max(0.052 * closing * closing, 0.0001));
    let arms = pow(0.5 + 0.5 * sin(angle * 3.0 - r * 32.0 - t * 18.0), 12.0)
        * exp(-r * 7.0) * smoothstep(0.06, 0.16, r);
    let envelope = wake * closing;
    let light_alpha = clamp((disk * flow * 0.85 + ring * 0.94 + halo * 0.16 + arms * 0.24)
        * envelope * outside, 0.0, 0.98);
    let hot = vec3<f32>(1.0, 0.76, 0.36);
    let amber = vec3<f32>(0.96, 0.31, 0.08);
    let violet = vec3<f32>(0.40, 0.20, 0.66);
    let disk_color = mix(amber, hot, clamp(ring + flow * 0.3, 0.0, 1.0));
    let light_color = mix(violet, disk_color, clamp(disk + ring, 0.0, 1.0));
    let core_alpha = (1.0 - smoothstep(horizon * 0.88, horizon + 0.003, r)) * envelope;
    let alpha = card_alpha + light_alpha * (1.0 - card_alpha);
    let rgb = (sample.rgb * card_alpha + light_color * light_alpha * (1.0 - card_alpha)) / max(alpha, 0.0001);
    // Straight alpha, matching Bevy's UI material pipeline.
    return vec4<f32>(mix(rgb, vec3<f32>(0.001, 0.001, 0.003), core_alpha),
        alpha + core_alpha * (1.0 - alpha));
}
