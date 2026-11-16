// The stroke-start snapshot caps opacity; erasing changes coverage only.
struct Params {
    vp:mat4x4<f32>,inverse_vp:mat4x4<f32>,eye:vec4<f32>,forward:vec4<f32>,rect:vec4<f32>,
    brush:vec4<f32>,color:vec4<f32>,settings:vec4<u32>,ids:vec4<u32>,
};
@group(0) @binding(0) var<uniform> u:Params;
@group(0) @binding(1) var coverage:texture_2d<f32>;
@group(0) @binding(2) var<storage,read> snapshot:array<u32>;
@group(0) @binding(3) var<storage,read_write> layer:array<u32>;
fn pack(v:vec4<f32>)->u32 {
    let q=vec4<u32>(clamp(v,vec4<f32>(0.0),vec4<f32>(1.0))*255.0+0.5);
    return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
@compute @workgroup_size(8,8)
fn apply(@builtin(global_invocation_id) p:vec3<u32>){
    if(any(p.xy>=vec2<u32>(u.settings.x))){return;}
    let m=clamp(textureLoad(coverage,vec2<i32>(p.xy),0).r*u.brush.w,0.0,1.0);
    if(m<=0.0){return;}
    let i=p.y*u.settings.x+p.x;let b=unpack4x8unorm(snapshot[i]);
    if(u.ids.z!=0u){layer[i]=pack(vec4<f32>(b.rgb,b.a*(1.0-m)));return;}
    let alpha=m+b.a*(1.0-m);
    let rgb=(u.color.rgb*m+b.rgb*b.a*(1.0-m))/max(alpha,0.000001);
    layer[i]=pack(vec4<f32>(rgb,alpha));
}
