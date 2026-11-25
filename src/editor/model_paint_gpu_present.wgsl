struct Params {
    vp:mat4x4<f32>,inverse_vp:mat4x4<f32>,eye:vec4<f32>,forward:vec4<f32>,rect:vec4<f32>,
    brush:vec4<f32>,color:vec4<f32>,settings:vec4<u32>,ids:vec4<u32>,
};
@group(0) @binding(0) var<uniform> u:Params;
@group(0) @binding(1) var<storage,read> composite:array<u32>;
@group(0) @binding(2) var<storage,read> auxiliary:array<u32>;
@group(0) @binding(3) var color_map:texture_2d<f32>;
@group(0) @binding(4) var rm_map:texture_2d<f32>;
@group(0) @binding(5) var out0:texture_storage_2d<rgba8unorm,write>;
@group(0) @binding(6) var out1:texture_storage_2d<rgba8unorm,write>;
@group(0) @binding(7) var<storage,read> tiles:array<u32>;
fn h(p:vec2<i32>,from_current:bool)->f32 {
    let q=clamp(p,vec2<i32>(0),vec2<i32>(i32(u.settings.x)-1));let i=u32(q.y)*u.settings.x+u32(q.x);
    if(from_current){return unpack4x8unorm(composite[i]).r;}
    return unpack4x8unorm(auxiliary[i]).r;
}
fn srgb(v:vec3<f32>)->vec3<f32>{return select(1.055*pow(max(v,vec3<f32>(0.0)),vec3<f32>(1.0/2.4))-0.055,v*12.92,v<=vec3<f32>(0.0031308));}
fn display(p:vec3<u32>){
    if(any(p.xy>=vec2<u32>(u.settings.x))){return;}
    let at=vec2<i32>(p.xy);let i=p.y*u.settings.x+p.x;let v=unpack4x8unorm(composite[i]);
    let channel=u.ids.w & 0x0fffffffu;
    var a=v;var b=v;
    if(channel==9u || channel==10u){
        let baked=unpack4x8unorm(auxiliary[i]);
        if(channel==9u){
            let value=mix(baked.r,v.r,v.a);
            a=vec4<f32>(vec3<f32>(value),baked.a);
        }else{
            a=baked;
            if(v.a>0.0){
                let xy=(baked.rg*2.0-1.0)*(1.0-v.a);
                let z=sqrt(max(0.0,1.0-dot(xy,xy)));
                a=vec4<f32>(vec3<f32>(xy,z)*0.5+0.5,baked.a);
            }
        }
        textureStore(out0,at,a);
        return;
    }
    if(channel==0u){a=vec4<f32>(v.rgb,textureLoad(color_map,at,0).a);}
    if(channel==1u){a=vec4<f32>(1.0,v.r,textureLoad(rm_map,at,0).b,1.0);}
    if(channel==2u){a=vec4<f32>(1.0,textureLoad(rm_map,at,0).g,v.r,1.0);}
    if(channel==3u||channel==6u){
        let normal=select(v,unpack4x8unorm(auxiliary[i]),channel==3u);
        if(u.forward.w==0.0){
            a=vec4<f32>(normal.rgb,1.0);
        }else{
            let dx=(h(at+vec2<i32>(1,0),channel==3u)-h(at-vec2<i32>(1,0),channel==3u))*u.forward.w;
            let dy=(h(at+vec2<i32>(0,1),channel==3u)-h(at-vec2<i32>(0,1),channel==3u))*u.forward.w;
            let n=normalize(vec3<f32>(normal.r*2.0-1.0-dx,normal.g*2.0-1.0+dy,max(normal.b*2.0-1.0,0.01)));
            a=vec4<f32>(n*0.5+0.5,1.0);
        }
    }
    if(channel==4u){a=vec4<f32>(srgb(textureLoad(color_map,at,0).rgb),v.r);}
    if(channel==7u){b=vec4<f32>(v.rgb,sqrt(v.r*0.5));}
    textureStore(out0,at,a);
    if(channel==1u||channel==2u||channel==3u||channel==4u||channel==7u){textureStore(out1,at,b);}
}
@compute @workgroup_size(8,8)
fn present(@builtin(workgroup_id) g:vec3<u32>,@builtin(local_invocation_id) l:vec3<u32>){
    if(u.settings.x<=1024u){display(vec3<u32>(g.xy*8u+l.xy,0u));return;}
    let tile=g.y*64u+g.x;if(tile>=tiles[3]){return;}
    let id=tiles[4u+tile];let columns=(u.settings.x+7u)/8u;
    display(vec3<u32>(vec2<u32>(id%columns,id/columns)*8u+l.xy,0u));
}
@compute @workgroup_size(8,8)
fn present_full(@builtin(global_invocation_id) p:vec3<u32>){display(p);}
