struct Blend {size:u32,opacity:f32,pad:vec2<u32>};
@group(0) @binding(0) var<uniform> u:Blend;
@group(0) @binding(1) var<storage,read> source:array<u32>;
@group(0) @binding(2) var<storage,read_write> result:array<u32>;
fn pack(v:vec4<f32>)->u32 {
    let q=vec4<u32>(clamp(v,vec4<f32>(0.0),vec4<f32>(1.0))*255.0+0.5);
    return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
@compute @workgroup_size(8,8)
fn blend(@builtin(global_invocation_id) p:vec3<u32>){
    if(any(p.xy>=vec2<u32>(u.size))){return;}
    let i=p.y*u.size+p.x;let b=unpack4x8unorm(result[i]);let s=unpack4x8unorm(source[i]);
    let a=s.a*u.opacity;
    result[i]=pack(vec4<f32>(b.rgb*(1.0-a)+s.rgb*a,b.a));
}
