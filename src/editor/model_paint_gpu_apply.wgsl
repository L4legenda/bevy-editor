// The stroke-start snapshot caps opacity; erasing changes coverage only.
struct Params {
    vp:mat4x4<f32>,inverse_vp:mat4x4<f32>,eye:vec4<f32>,forward:vec4<f32>,rect:vec4<f32>,
    brush:vec4<f32>,color:vec4<f32>,settings:vec4<u32>,ids:vec4<u32>,
};
@group(0) @binding(0) var<uniform> u:Params;
@group(0) @binding(1) var coverage:texture_2d<f32>;
@group(0) @binding(2) var<storage,read> snapshot:array<u32>;
@group(0) @binding(3) var<storage,read_write> layer:array<u32>;
@group(0) @binding(4) var<storage,read> tiles:array<u32>;
@group(0) @binding(5) var grain:texture_2d<f32>;
fn pack(v:vec4<f32>)->u32 {
    let q=vec4<u32>(clamp(v,vec4<f32>(0.0),vec4<f32>(1.0))*255.0+0.5);
    return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
@compute @workgroup_size(8,8)
fn apply(@builtin(workgroup_id) g:vec3<u32>,@builtin(local_invocation_id) l:vec3<u32>){
    var p=vec3<u32>(g.xy*8u+l.xy,0u);
    if(u.settings.x>1024u){
        let tile=g.y*64u+g.x;if(tile>=tiles[3]){return;}
        let id=tiles[4u+tile];let columns=(u.settings.x+7u)/8u;
        p=vec3<u32>(vec2<u32>(id%columns,id/columns)*8u+l.xy,0u);
    }
    if(any(p.xy>=vec2<u32>(u.settings.x))){return;}
    var m=textureLoad(coverage,vec2<i32>(p.xy),0).r*u.brush.w;
    if ((u.ids.w & 0x10000000u) != 0u) {
        m *= textureLoad(grain,vec2<i32>(p.xy),0).r;
    }
    m=clamp(m,0.0,1.0);
    if(m<=0.0){return;}
    let i=p.y*u.settings.x+p.x;let b=unpack4x8unorm(snapshot[i]);
    if ((u.ids.w & 0x40000000u) != 0u) {
        let add = b.g * (1.0 - m);
        if (u.ids.z != 0u) {
            layer[i] = pack(vec4<f32>(mix(b.r, 1.0, m), add, 0.0, 1.0));
        } else {
            layer[i] = pack(vec4<f32>(b.r * (1.0 - m), add + u.color.r * m, 0.0, 1.0));
        }
        return;
    }
    if(u.ids.z!=0u){layer[i]=pack(vec4<f32>(b.rgb,b.a*(1.0-m)));return;}
    let alpha=m+b.a*(1.0-m);
    let rgb=(u.color.rgb*m+b.rgb*b.a*(1.0-m))/max(alpha,0.000001);
    layer[i]=pack(vec4<f32>(rgb,alpha));
}
