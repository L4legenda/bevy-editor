import bevy_ui_render::ui_vertex_output::UiVertexOutput;
@group(1) @binding(0) var<uniform> metrics: vec4<f32>;
@group(1) @binding(1) var<uniform> motion: vec4<f32>;
@group(1) @binding(2) var<uniform> tint: vec4<f32>;

fn rounded(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(r);
    return length(max(q,vec2<f32>(0.0))) + min(max(q.x,q.y),0.0) - r;
}
@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let size = max(metrics.xy,vec2<f32>(1.0));
    let p = (in.uv - 0.5) * size;
    let radius = select(7.0,12.0,motion.z < 0.5);
    let d = rounded(p,max(size*0.5-vec2<f32>(0.8),vec2<f32>(1.0)),min(radius,min(size.x,size.y)*0.4));
    let inside = 1.0-smoothstep(-0.7,0.7,d);
    let rim = exp(-abs(d+0.7)*1.35);
    let phase = clamp(motion.x,0.0,1.0);
    if motion.z > 1.5 {
        let pulse = pow(sin(phase*3.14159265),2.0);
        let sweep = exp(-pow((in.uv.x-phase)/0.24,2.0));
        return vec4<f32>(tint.rgb,(rim*0.65+sweep*inside*0.045)*pulse);
    }
    if motion.z < 0.5 {
        let haze = exp(-length((in.uv-vec2<f32>(0.12,0.02))*vec2<f32>(2.5,4.2)));
        let blue = exp(-length((in.uv-vec2<f32>(0.95,0.95))*vec2<f32>(3.5,3.0)));
        let color = mix(tint.rgb,vec3<f32>(0.18,0.36,0.63),blue);
        let sweep = exp(-pow((in.uv.x+in.uv.y*0.3-phase*1.6+0.3)/0.2,2.0))*(1.0-phase);
        let pixel = floor(in.uv*size);
        let noise = (fract(sin(dot(pixel,vec2<f32>(12.9898,78.233)))*43758.5453)-0.5)*0.0015;
        let veil = max(haze*0.006+blue*0.006+noise,0.0);
        return vec4<f32>(color,(inside*veil+rim*(0.06+sweep*0.18))*metrics.z);
    }
    let selected = motion.y;
    let hover = metrics.w;
    let sweep = exp(-pow((in.uv.x-phase*1.4+0.2)/0.14,2.0))*sin(phase*3.14159265)*hover;
    return vec4<f32>(tint.rgb,(inside*(selected*0.018+hover*0.030+sweep*0.020)+rim*(selected*0.36+hover*0.22))*metrics.z);
}
