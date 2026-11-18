import bevy_ui_render::ui_vertex_output::UiVertexOutput;

// Kind, seconds, reveal/hold progress, destination validity.
@group(1) @binding(0) var<uniform> state: vec4<f32>;
// Linear colour and DPI.
@group(1) @binding(1) var<uniform> tint: vec4<f32>;

fn ring(distance: f32, radius: f32, width: f32) -> f32 {
    return 1.0-smoothstep(width, width+0.85, abs(distance-radius));
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let density = max(tint.w, 0.5);
    let p = (in.uv-vec2<f32>(0.5))*in.size/density;
    let r = length(p);
    let color = tint.rgb;
    let accent = mix(vec3<f32>(0.48,0.28,0.10), vec3<f32>(0.80,0.59,0.30), state.w);
    if state.x < 0.5 {
        // The clockwise ring makes the long-press threshold discoverable.
        let angle = fract((atan2(p.y,p.x)+1.5707963)/6.2831853+1.0);
        let track = ring(r,19.5,0.65)*0.18;
        let progress = ring(r,19.5,1.25)*(1.0-smoothstep(state.z-0.018,state.z+0.012,angle));
        let glow = exp(-pow((r-19.5)/4.0,2.0))*state.z*0.12;
        return vec4<f32>(mix(accent,vec3<f32>(1.0),progress*0.25),track+progress*0.9+glow);
    }
    if state.x > 2.5 {
        // Two spreading waves and eight tiny moving glints dissipate in half a second.
        let t = clamp(state.y/0.52,0.0,1.0);
        let radius = 5.0+70.0*(1.0-pow(1.0-t,2.0));
        let wave = ring(r,radius,1.1)*pow(1.0-t,1.7);
        let echo = ring(r,radius*0.65,0.6)*pow(1.0-t,2.4)*0.42;
        let angle = atan2(p.y,p.x);
        let sparks = pow(max(0.0,cos(angle*8.0+0.3)),28.0)
            *exp(-pow((r-radius*0.83)/2.2,2.0))*pow(1.0-t,2.0)*0.7;
        let flash = exp(-r*r/100.0)*pow(1.0-t,5.0)*0.3;
        let alpha = clamp(wave+echo+sparks+flash,0.0,1.0);
        // A bright outer rim stays visible even when carrying pure black.
        return vec4<f32>(mix(color,vec3<f32>(0.92,0.84,0.69),0.65),alpha);
    }
    if state.x > 1.5 {
        let outline = ring(r,5.0,0.7);
        let shadow = ring(r,6.2,0.6)*0.75;
        let dot = 1.0-smoothstep(0.8,1.6,r);
        let alpha = clamp(outline+shadow+dot*0.8,0.0,1.0);
        return vec4<f32>(mix(vec3<f32>(0.015),vec3<f32>(0.95),
            (outline+dot)/max(alpha,0.001)),alpha);
    }
    // A lit, opaque colour bead with a glass edge, fine dark outline and soft halo.
    let body = 1.0-smoothstep(16.0,16.9,r);
    let rim = ring(r,16.1,0.65);
    let outer = ring(r,17.5,0.8);
    let halo = exp(-pow((r-17.5)/6.5,2.0))*0.12;
    let shade = 0.90+0.10*clamp((-p.y+16.0)/32.0,0.0,1.0);
    let spec = exp(-pow(length((p-vec2<f32>(-5.0,-6.0))/vec2<f32>(7.0,4.0)),2.0))*0.5;
    let small_spec = exp(-pow(length(p-vec2<f32>(-7.0,-8.0))/1.9,2.0))*0.55;
    let body_color = mix(color*shade,vec3<f32>(1.0),spec+small_spec*0.35);
    let weights = body+rim*0.72+outer*0.85+halo;
    let rgb = (body_color*body+vec3<f32>(0.95)*rim*0.72
        +vec3<f32>(0.008)*outer*0.85+mix(color,accent,0.45)*halo)/max(weights,0.001);
    return vec4<f32>(rgb,clamp(weights,0.0,1.0)*state.z);
}
