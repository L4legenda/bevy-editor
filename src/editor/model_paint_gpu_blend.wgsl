struct Blend {size:u32,opacity:f32,reset:u32,partial:u32};
@group(0) @binding(0) var<uniform> u:Blend;
@group(0) @binding(1) var<storage,read> source:array<u32>;
@group(0) @binding(2) var<storage,read_write> result:array<u32>;
@group(0) @binding(3) var<storage,read> below:array<u32>;
@group(0) @binding(4) var<storage,read> tiles:array<u32>;
fn pack(v:vec4<f32>)->u32 {
    let q=vec4<u32>(clamp(v,vec4<f32>(0.0),vec4<f32>(1.0))*255.0+0.5);
    return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
@compute @workgroup_size(8,8)
fn blend(@builtin(workgroup_id) g:vec3<u32>,@builtin(local_invocation_id) l:vec3<u32>){
    var p=vec3<u32>(g.xy*8u+l.xy,0u);
    if(u.partial!=0u){
        let tile=g.y*64u+g.x;if(tile>=tiles[3]){return;}
        let id=tiles[4u+tile];let columns=(u.size+7u)/8u;
        p=vec3<u32>(vec2<u32>(id%columns,id/columns)*8u+l.xy,0u);
    }
    if(any(p.xy>=vec2<u32>(u.size))){return;}
    let i=p.y*u.size+p.x;var b=unpack4x8unorm(result[i]);let s=unpack4x8unorm(source[i]);
    if(u.reset!=0u){b=unpack4x8unorm(below[i]);}
    let a=s.a*u.opacity;
    result[i]=pack(vec4<f32>(b.rgb*(1.0-a)+s.rgb*a,b.a));
}
