import bevy_ui_render::ui_vertex_output::UiVertexOutput;

// Start and end in canvas-local logical pixels.
@group(1) @binding(0) var<uniform> points: vec4<f32>;
// Physical pixel density, eased visibility, seconds, radial flag.
@group(1) @binding(1) var<uniform> style: vec4<f32>;
@group(1) @binding(2) var<uniform> first: vec4<f32>;
@group(1) @binding(3) var<uniform> last: vec4<f32>;

fn edge(distance: f32, width: f32) -> f32 {
    return 1.0 - smoothstep(width, width + 0.85, distance);
}

fn over(front: vec4<f32>, back: vec4<f32>) -> vec4<f32> {
    let a = front.a + back.a * (1.0 - front.a);
    let rgb = (front.rgb * front.a + back.rgb * back.a * (1.0 - front.a)) / max(a, 0.0001);
    return vec4<f32>(rgb, a);
}

fn marker(p: vec2<f32>, paint: vec4<f32>, radius: f32) -> vec4<f32> {
    let d = length(p);
    let tile = (floor(p.x / 3.0) + floor(p.y / 3.0)) * 0.5;
    let checker = select(0.14, 0.39, fract(tile) > 0.1);
    let fill = mix(vec3<f32>(checker), paint.rgb, paint.a);
    let rim = vec3<f32>(1.0, 0.78, 0.46);
    return over(vec4<f32>(fill, edge(d, radius - 1.7)),
        over(vec4<f32>(rim, edge(d, radius)),
             vec4<f32>(0.012, 0.015, 0.022, edge(d, radius + 1.1))));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let pixel = in.uv * in.size / max(style.x, 0.5);
    let p = pixel - points.xy;
    let delta = points.zw - points.xy;
    let length_px = length(delta);
    let span = max(length_px, 0.001);
    let along = clamp(dot(p, delta) / (span * span), 0.0, 1.0);
    let distance = length(p - along * delta);
    let pulse = 0.5 + 0.5 * cos(along * 18.0 - style.z * 3.5);
    var result = vec4<f32>(0.012, 0.015, 0.022, edge(distance, 2.1) * 0.87);
    let gold = vec3<f32>(1.0, 0.73, 0.37);
    result = over(vec4<f32>(gold, edge(distance, 0.65) * (0.82 + pulse * 0.18)), result);
    if style.w > 0.5 && length_px > 7.0 {
        let ring = abs(length(p) - length_px);
        let phase = atan2(p.y, p.x) * ceil(length_px * 0.45) / 6.2831853;
        let dash = 1.0 - smoothstep(0.52, 0.67, fract(phase - style.z * 0.6));
        result = over(vec4<f32>(0.012, 0.015, 0.022, edge(ring, 1.6) * 0.72 * dash), result);
        result = over(vec4<f32>(gold, edge(ring, 0.55) * 0.82 * dash), result);
    }
    let radius = mix(3.0, 6.0, style.y);
    result = over(marker(p, first, radius), result);
    if length_px > 4.0 {
        result = over(marker(pixel - points.zw, last, radius), result);
    }
    return vec4<f32>(result.rgb, result.a * style.y);
}
