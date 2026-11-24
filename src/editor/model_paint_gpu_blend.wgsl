struct Blend {size:u32,opacity:f32,flags:u32,mode:u32};
@group(0) @binding(0) var<uniform> u:Blend;
@group(0) @binding(1) var<storage,read> source:array<u32>;
@group(0) @binding(2) var<storage,read_write> result:array<u32>;
@group(0) @binding(3) var<storage,read> below:array<u32>;
@group(0) @binding(4) var<storage,read> tiles:array<u32>;
@group(0) @binding(5) var<storage,read> mask:array<u32>;
fn pack(v:vec4<f32>)->u32 {
    let q=vec4<u32>(clamp(v,vec4<f32>(0.0),vec4<f32>(1.0))*255.0+0.5);
    return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
// Layer blend modes adapted from Material Painter's wear.wgsl (GPL-3.0-or-later).
fn blend_color(mode: u32, base: vec3<f32>, source: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 1u: { return base * source; }
        case 2u: { return 1.0 - (1.0 - base) * (1.0 - source); }
        case 3u: {
            return select(1.0 - 2.0 * (1.0 - base) * (1.0 - source),
                2.0 * base * source, base < vec3<f32>(0.5));
        }
        case 4u: { return base + source; }
        case 5u: { return base - source; }
        case 6u: { return min(base, source); }
        case 7u: { return max(base, source); }
        case 10u: { return base + source - 128.0 / 255.0; }
        default: { return source; }
    }
}
@compute @workgroup_size(8,8)
fn blend(@builtin(workgroup_id) g:vec3<u32>,@builtin(local_invocation_id) l:vec3<u32>){
    var p=vec3<u32>(g.xy*8u+l.xy,0u);
    if((u.flags&2u)!=0u){
        let tile=g.y*64u+g.x;if(tile>=tiles[3]){return;}
        let id=tiles[4u+tile];let columns=(u.size+7u)/8u;
        p=vec3<u32>(vec2<u32>(id%columns,id/columns)*8u+l.xy,0u);
    }
    if(any(p.xy>=vec2<u32>(u.size))){return;}
    let i=p.y*u.size+p.x;
    if (u.mode == 8u) {
        result[i] = source[i];
        return;
    }
    var b=unpack4x8unorm(result[i]);let s=unpack4x8unorm(source[i]);
    var opacity = u.opacity;
    if ((u.flags & 4u) != 0u) {
        opacity *= f32(mask[i] & 255u) / 255.0;
    }
    if (u.mode == 9u) {
        result[i] = pack(mix(s, b, opacity));
        return;
    }
    if((u.flags&1u)!=0u){b=unpack4x8unorm(below[i]);}
    let a=s.a*opacity;
    result[i]=pack(vec4<f32>(mix(b.rgb,blend_color(u.mode,b.rgb,s.rgb),a),b.a));
}
