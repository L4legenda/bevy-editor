import bevy_ui_render::ui_vertex_output::UiVertexOutput;

@group(1) @binding(0) var<uniform> ends: vec4<f32>;
@group(1) @binding(1) var<uniform> controls: vec4<f32>;
@group(1) @binding(2) var<uniform> tint: vec4<f32>;
@group(1) @binding(3) var<uniform> metrics: vec4<f32>;
// progress, time, mode (rest / arrival / break / drag), hover intensity
@group(1) @binding(4) var<uniform> motion: vec4<f32>;

fn curve(t: f32) -> vec2<f32> {
    let u = 1.0 - t;
    return ends.xy * u * u * u + controls.xy * (3.0 * u * u * t)
        + controls.zw * (3.0 * u * t * t) + ends.zw * t * t * t;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let p = in.uv * metrics.xy;
    var previous = ends.xy;
    var distance_squared = 1e20;
    var closest_t = 0.0;
    // Distance to the whole curve is shaded once, avoiding overlap seams.
    for (var i = 1u; i <= 48u; i += 1u) {
        let next = curve(f32(i) / 48.0);
        let segment = next - previous;
        let along = clamp(dot(p - previous, segment) / max(dot(segment, segment), 0.00001), 0.0, 1.0);
        let delta = p - previous - segment * along;
        let candidate = dot(delta, delta);
        if candidate < distance_squared {
            distance_squared = candidate;
            closest_t = (f32(i)-1.0+along)/48.0;
        }
        previous = next;
    }
    let distance = sqrt(distance_squared);
    let aa = max(fwidth(distance), 0.35);
    let radius = metrics.z * 0.5;
    let coverage = 1.0 - smoothstep(radius - aa * 0.5, radius + aa * 0.5, distance);
    var reveal = 1.0;
    var pulse = 0.0;
    var glow = motion.w * 0.10;
    var ink = tint.rgb;
    if motion.z > 0.5 && motion.z < 1.5 {
        let tip = 1.0-pow(1.0-motion.x,3.0);
        reveal = 1.0-smoothstep(tip-0.018,tip+0.018,closest_t);
        pulse = exp(-pow((closest_t-tip)/0.06,2.0))*(1.0-motion.x);
        glow = pulse*0.30;
    } else if motion.z > 1.5 && motion.z < 2.5 {
        let gap = motion.x*0.65;
        let edge = abs(closest_t-0.5);
        reveal = smoothstep(gap-0.02,gap+0.02,edge)*pow(1.0-motion.x,1.3);
        pulse = exp(-pow((edge-gap)/0.045,2.0))*(1.0-motion.x);
        glow = pulse*0.20;
        ink = mix(ink,vec3<f32>(0.52,0.36,0.62),motion.x*0.32);
    } else if motion.z > 2.5 {
        let head = fract(motion.y*0.8);
        pulse = exp(-pow((closest_t-head)/0.08,2.0))*0.30;
        glow = pulse*0.12;
    }
    let aura = exp(-distance*distance/22.0)*glow;
    let alpha = tint.a*(coverage*reveal+aura);
    return vec4<f32>(ink*(1.0+pulse*0.3),clamp(alpha,0.0,1.0));
}
