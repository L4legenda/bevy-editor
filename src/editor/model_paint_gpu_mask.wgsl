// Material Painter's screen/UV stamp coverage, adapted to Bevy's render device.
struct Params {
    vp: mat4x4<f32>, inverse_vp: mat4x4<f32>,
    eye: vec4<f32>, forward: vec4<f32>, rect: vec4<f32>,
    brush: vec4<f32>, color: vec4<f32>, settings: vec4<u32>, ids: vec4<u32>,
};
@group(0) @binding(0) var<uniform> u: Params;
@group(0) @binding(1) var<storage, read> stamps: array<vec4<f32>>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var scene_ids: texture_2d<u32>;
@group(0) @binding(4) var uv_ids: texture_2d<u32>;
struct Vertex {
    @location(0) position:vec3<f32>, @location(1) uv:vec2<f32>,
    @location(2) face:u32, @location(3) object:u32, @location(4) slot:u32,
};
struct Varying {
    @builtin(position) position:vec4<f32>, @location(0) world:vec3<f32>,
    @location(1) uv:vec2<f32>, @location(2) @interpolate(flat) face:u32,
    @location(3) @interpolate(flat) object:u32, @location(4) @interpolate(flat) slot:u32,
};
fn output(v:Vertex,p:vec4<f32>)->Varying {
    return Varying(p,v.position,v.uv,v.face,v.object,v.slot);
}
@vertex fn vs_scene(v:Vertex)->Varying {return output(v,u.vp*vec4<f32>(v.position,1.0));}
@vertex fn vs_uv(v:Vertex)->Varying {return output(v,vec4<f32>(v.uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.0,1.0));}
@fragment fn fs_scene(v:Varying)->@location(0) vec2<u32>{return vec2<u32>(v.object,v.face);}
@fragment fn fs_uv(v:Varying)->@location(0) u32 {
    if(v.object!=u.ids.x||v.slot!=u.ids.y){discard;}
    return v.face;
}
@fragment fn fs_mask(v:Varying)->@location(0) vec4<f32> {
    if(v.object!=u.ids.x||v.slot!=u.ids.y){discard;}
    var pixel=u.rect.xy+v.uv*u.rect.zw;
    let dims=vec2<f32>(textureDimensions(scene_depth));
    if(u.settings.z==0u){
        let clip=u.vp*vec4<f32>(v.world,1.0);
        if(clip.w<=0.0){discard;}
        let ndc=clip.xyz/clip.w;
        if(any(abs(ndc.xy)>vec2<f32>(1.0))||ndc.z<0.0||ndc.z>1.0){discard;}
        let at=clamp(vec2<i32>((ndc.xy*vec2<f32>(0.5,-0.5)+0.5)*dims),vec2<i32>(0),vec2<i32>(dims)-1);
        let z=textureLoad(scene_depth,at,0);
        if(z<=0.0){discard;}
        let sp=(vec2<f32>(at)+0.5)/dims*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
        let back=u.inverse_vp*vec4<f32>(sp,z,1.0);
        let scene_t=dot(back.xyz/back.w-u.eye.xyz,u.forward.xyz);
        let t=dot(v.world-u.eye.xyz,u.forward.xyz);
        if(t>scene_t+max(u.eye.w,abs(scene_t)*0.005)){discard;}
        pixel=u.rect.xy+(ndc.xy*vec2<f32>(0.5,-0.5)+0.5)*u.rect.zw;
    }
    var keep=1.0;
    for(var i=0u;i<u.settings.y;i++){
        let stamp=stamps[i].xy;
        var face=0u;
        if(u.settings.z==0u){
            let at=clamp(vec2<i32>((stamp-u.rect.xy)/u.rect.zw*dims),vec2<i32>(0),vec2<i32>(dims)-1);
            let id=textureLoad(scene_ids,at,0).xy;
            if(id.x!=u.ids.x){continue;}
            face=id.y;
        }else if(u.settings.w!=0u){
            let at=clamp(vec2<i32>((stamp-u.rect.xy)/u.rect.zw*f32(u.settings.x)),vec2<i32>(0),vec2<i32>(i32(u.settings.x)-1));
            face=textureLoad(uv_ids,at,0).x;
        }
        var fall=0.0;
        if(u.settings.w!=0u){
            fall=select(0.0,1.0,face==v.face&&face!=0u);
        }else{
            let d=distance(pixel,stamp)/max(u.brush.x,0.5);
            let t=clamp((d-u.brush.y)/max(1.0-u.brush.y,0.00001),0.0,1.0);
            fall=select(1.0-t*t*(3.0-2.0*t),0.0,d>=1.0);
        }
        keep*=1.0-fall*select(u.brush.z,1.0,u.settings.w!=0u);
    }
    let coverage=1.0-keep;
    if(coverage<=0.0){discard;}
    return vec4<f32>(coverage,0.0,0.0,coverage);
}
