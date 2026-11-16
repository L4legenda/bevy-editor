import bevy_ui_render::ui_vertex_output::UiVertexOutput;
@group(1) @binding(0) var<uniform> metrics: vec4<f32>;
@group(1) @binding(1) var<uniform> tint: vec4<f32>;

fn hash(p: vec2<f32>) -> f32 { return fract(sin(dot(p,vec2<f32>(127.1,311.7)))*43758.5453); }
fn rounded(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q=abs(p)-half_size+vec2<f32>(radius);
    return length(max(q,vec2<f32>(0.0)))+min(max(q.x,q.y),0.0)-radius;
}
@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t=clamp(metrics.z,0.0,1.0);
    let p=in.uv*metrics.xy-metrics.xy*0.5;
    let half_size=max(metrics.xy*0.5-vec2<f32>(20.0),vec2<f32>(1.0));
    let radius=min(12.0*tint.w,min(half_size.x,half_size.y));
    if metrics.w<0.5 {
        let d=rounded(p,half_size,radius);
        let ring=exp(-pow((d-2.0-t*13.0)/2.1,2.0));
        let rim=exp(-abs(d)/3.0);
        let sweep=0.6+0.4*cos(atan2(p.y,p.x)-t*4.0);
        return vec4<f32>(tint.rgb,(ring*0.22+rim*0.28)*pow(1.0-t,1.6)*sweep);
    }
    let d=rounded(p+vec2<f32>(0.0,t*8.0),half_size*(1.0-t*0.045),radius);
    let grain=hash(floor((p+vec2<f32>(0.0,t*16.0))/3.5));
    let dissolve=smoothstep(t*1.15-0.15,t*1.15+0.12,grain);
    let rim=exp(-abs(d)/1.4);
    let fill=1.0-smoothstep(-1.0,1.0,d);
    let dust=step(0.982,grain)*exp(-abs(d-3.0-t*10.0)/5.0)*0.18;
    return vec4<f32>(mix(tint.rgb,vec3<f32>(0.48,0.36,0.66),t*0.35),
        (rim*0.55*dissolve+fill*0.005+dust)*pow(1.0-t,1.25));
}
