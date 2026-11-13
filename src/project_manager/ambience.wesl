import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::globals::Globals;

@group(0) @binding(1) var<uniform> globals: Globals;
// Smoothed pointer position, pointer proximity, modal dimming.
@group(1) @binding(0) var<uniform> cursor: vec4<f32>;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let size = max(in.size, vec2<f32>(1.0));
    let aspect = size.x / size.y;
    let parallax = (cursor.xy - vec2<f32>(0.5)) * vec2<f32>(0.07, 0.04);
    let p = (in.uv - vec2<f32>(0.5) + parallax) * vec2<f32>(aspect, 1.0);
    let time = globals.time * 0.075;
    let cool_center = vec2<f32>(0.37 * aspect + sin(time * 0.6) * 0.12, -0.24 + cos(time * 0.7) * 0.06);
    let warm_center = vec2<f32>(-0.40 * aspect + cos(time * 0.5) * 0.14, 0.29 + sin(time * 0.4) * 0.08);
    let cool_delta = p - cool_center;
    let warm_delta = p - warm_center;
    let cool_cloud = exp(-dot(cool_delta, cool_delta) * 3.8);
    let warm_cloud = exp(-dot(warm_delta, warm_delta) * 4.4);
    let flow = sin(p.x * 3.0 + time) * 0.11 + sin(p.x * 5.5 - time * 0.7) * 0.035;
    let ribbon_delta = p.y - flow;
    let ribbon = exp(-ribbon_delta * ribbon_delta * 28.0)
        * (0.55 + 0.45 * sin(p.x * 2.4 - time * 0.65));
    let pointer_delta = (in.uv - cursor.xy) * vec2<f32>(aspect, 1.0);
    let pointer_glow = exp(-dot(pointer_delta, pointer_delta) * 18.0) * cursor.z;
    let violet = vec3<f32>(0.42, 0.31, 0.70);
    let gold = vec3<f32>(0.74, 0.46, 0.20);
    let light = violet * (cool_cloud * 0.055 + ribbon * 0.010)
        + gold * warm_cloud * 0.045
        + mix(violet, gold, in.uv.x) * pointer_glow * 0.025;
    let pixel = in.uv * size;
    let edge = min(min(pixel.x, size.x - pixel.x), min(pixel.y, size.y - pixel.y));
    let vignette = smoothstep(0.0, 22.0, edge);
    let peak = max(max(light.r, light.g), light.b);
    // Spatial dithering keeps the broad, dark clouds smooth on 8-bit displays.
    let noise = fract(52.9829189 * fract(dot(floor(pixel), vec2<f32>(0.06711056, 0.00583715)))) - 0.5;
    let alpha = clamp(peak + noise * (0.75 / 255.0), 0.0, 0.20)
        * vignette * (1.0 - cursor.w * 0.35);
    return vec4<f32>(light / max(peak, 0.00001), alpha);
}
