// Material Painter's screen/UV stamp coverage, adapted to Bevy's render device.
struct Params {
    vp: mat4x4<f32>, inverse_vp: mat4x4<f32>,
    eye: vec4<f32>, forward: vec4<f32>, rect: vec4<f32>,
    brush: vec4<f32>, color: vec4<f32>, settings: vec4<u32>, ids: vec4<u32>,
    mirror: vec4<f32>, grain: vec4<f32>, grain_flags: vec4<u32>,
    projection: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Params;
struct Stamp {
    position_rotation: vec4<f32>,
    pressure: vec4<f32>,
    center: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    uv: vec4<f32>,
};
@group(0) @binding(1) var<storage, read> stamps: array<Stamp>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var scene_ids: texture_2d<u32>;
@group(0) @binding(4) var uv_occupied: texture_2d<u32>;
@group(0) @binding(5) var<storage,read_write> dirty:array<atomic<u32>>;
@group(0) @binding(6) var tip_map: texture_2d<f32>;
@group(0) @binding(7) var tip_sampler: sampler;
@group(0) @binding(8) var grain_pattern: texture_2d<f32>;
@group(0) @binding(9) var grain_sampler: sampler;
struct Vertex {
    @location(0) position:vec3<f32>, @location(1) uv:vec2<f32>,
    @location(2) face:u32, @location(3) object:u32, @location(4) slot:u32,
    @location(5) normal:vec3<f32>,
};
struct Varying {
    @builtin(position) position:vec4<f32>, @location(0) world:vec3<f32>,
    @location(1) uv:vec2<f32>, @location(2) @interpolate(flat) face:u32,
    @location(3) @interpolate(flat) object:u32, @location(4) @interpolate(flat) slot:u32,
    @location(5) normal:vec3<f32>,
};
fn output(v:Vertex,p:vec4<f32>)->Varying {
    return Varying(p,v.position,v.uv,v.face,v.object,v.slot,v.normal);
}
@vertex fn vs_scene(v:Vertex)->Varying {return output(v,u.vp*vec4<f32>(v.position,1.0));}
@vertex fn vs_uv(v:Vertex)->Varying {return output(v,vec4<f32>(v.uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.0,1.0));}
@fragment fn fs_scene(v:Varying)->@location(0) vec2<u32>{return vec2<u32>(v.object,v.face);}
@fragment fn fs_occupied(v: Varying) -> @location(0) u32 {
    if (v.object != u.ids.x || v.slot != u.ids.y) { discard; }
    return 1u;
}
fn tip_fall(off: vec2<f32>, rotation: vec2<f32>) -> f32 {
    let d = length(off);
    if (d >= 1.0) { return 0.0; }
    if ((u.ids.w & 0x20000000u) != 0u) {
        let q = vec2<f32>(dot(off, rotation), dot(off, vec2<f32>(-rotation.y, rotation.x)));
        return textureSampleLevel(tip_map, tip_sampler, q * 0.5 + 0.5, 0.0).r;
    }
    let t = clamp((d-u.brush.y) / max(1.0-u.brush.y, 0.00001), 0.0, 1.0);
    return 1.0-t*t*(3.0-2.0*t);
}

fn facing(normal: vec3<f32>, direction: vec3<f32>) -> f32 {
    let len = length(normal);
    if (len <= 1e-6) { return 1.0; }
    let lim = u.projection.y;
    let advance = max(min(lim + 0.15, 1.0), lim + 1e-4);
    return smoothstep(lim, advance, dot(normal / len, direction));
}

fn coverage_at(world: vec3<f32>, uv: vec2<f32>, normal: vec3<f32>, mirrored: bool) -> f32 {
    var pixel=u.rect.xy+uv*u.rect.zw;
    let dims=vec2<f32>(textureDimensions(scene_depth));
    var ndc=vec3<f32>(0.0);
    if(u.settings.z==0u){
        let clip=u.vp*vec4<f32>(world,1.0);
        if(clip.w<=0.0){return 0.0;}
        ndc=clip.xyz/clip.w;
        if(any(abs(ndc.xy)>vec2<f32>(1.0))||ndc.z<0.0||ndc.z>1.0){return 0.0;}
        pixel=u.rect.xy+(ndc.xy*vec2<f32>(0.5,-0.5)+0.5)*u.rect.zw;
    }
    var keep=1.0;
    for(var i=0u;i<u.settings.y;i++){
        let stamp=stamps[i].position_rotation;
        let pressure=stamps[i].pressure.xy;
        let off=(pixel-stamp.xy)/max(u.brush.x*pressure.x,0.5);
        let fall=tip_fall(off,stamp.zw);
        if(fall<=0.0){continue;}
        if(u.settings.z==0u){
            let at=clamp(vec2<i32>((stamp.xy-u.rect.xy)/u.rect.zw*dims),vec2<i32>(0),vec2<i32>(dims)-1);
            if(textureLoad(scene_ids,at,0).x!=u.ids.x){continue;}
        }
        keep*=1.0-fall*u.brush.z*pressure.y;
    }
    let coverage=1.0-keep;
    if(coverage<=0.0){return 0.0;}
    // Most UV texels miss the brush. Test depth only after the cheap disc test.
    if(u.settings.z==0u && u.projection.z==0.0){
        let at=clamp(vec2<i32>((ndc.xy*vec2<f32>(0.5,-0.5)+0.5)*dims),vec2<i32>(0),vec2<i32>(dims)-1);
        let z=textureLoad(scene_depth,at,0);
        if(z<=0.0){return 0.0;}
        let sp=(vec2<f32>(at)+0.5)/dims*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
        let back=u.inverse_vp*vec4<f32>(sp,z,1.0);
        let scene_t=dot(back.xyz/back.w-u.eye.xyz,u.forward.xyz);
        let t=dot(world-u.eye.xyz,u.forward.xyz);
        let epsilon = max(u.eye.w,abs(scene_t)*0.005);
        if(t>scene_t+epsilon){return 0.0;}
        if(mirrored && (abs(t-scene_t)>epsilon || textureLoad(scene_ids,at,0).x!=u.ids.x)){
            return 0.0;
        }
    }
    if (u.settings.z == 0u && u.projection.y > 1e-4) {
        let eye = select(normalize(u.eye.xyz-world), -u.forward.xyz, u.projection.w != 0.0);
        return coverage * facing(normal, eye);
    }
    return coverage;
}

fn coverage_projected(v: Varying) -> f32 {
    var keep = vec2<f32>(1.0);
    for (var i = 0u; i < u.settings.y; i++) {
        let stamp = stamps[i];
        let flip = stamp.pressure.z;
        let radius = stamp.center.w;
        var value = 0.0;
        if (u.projection.x == 2.0) {
            if (stamp.normal.w <= 0.0) { continue; }
            let off = (v.uv - stamp.uv.xy) / stamp.normal.w * vec2<f32>(flip, 1.0);
            value = tip_fall(off, stamp.position_rotation.zw);
        } else {
            let delta = v.world - stamp.center.xyz;
            let n = stamp.normal.xyz;
            if (radius <= 0.0 || abs(dot(delta,n)) > radius) { continue; }
            let x = stamp.tangent.xyz;
            let y = cross(x, n) * flip;
            let off = vec2<f32>(dot(delta,x), dot(delta,y)) / radius;
            value = tip_fall(off, stamp.position_rotation.zw) * facing(v.normal,n);
        }
        let amount = value * u.brush.z * stamp.pressure.y;
        if (flip < 0.0) {
            keep.y *= 1.0 - amount;
        } else {
            keep.x *= 1.0 - amount;
        }
    }
    return 1.0 - min(keep.x, keep.y);
}

fn mark_dirty(position: vec2<f32>) {
    if(u.settings.x>1024u || (u.ids.w & 0x80000000u) != 0u){
        let tile=vec2<u32>(position)/8u;
        atomicStore(&dirty[tile.y*((u.settings.x+7u)/8u)+tile.x],1u);
    }
}

@vertex fn vs_padding(@location(0) uv: vec2<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}
@fragment fn fs_padding(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    if (textureLoad(uv_occupied, vec2<i32>(position.xy), 0).x != 0u) { discard; }
    mark_dirty(position.xy);
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}

@fragment fn fs_mask(v:Varying)->@location(0) vec4<f32> {
    if(v.object!=u.ids.x||v.slot!=u.ids.y){discard;}
    var coverage = 0.0;
    if (u.settings.w != 0u) {
        coverage = 1.0;
    } else if (u.projection.x == 0.0) {
        coverage = coverage_at(v.world, v.uv, v.normal, false);
        if (any(u.mirror.xyz != vec3<f32>(0.0))) {
            let reflected = v.world - 2.0 * (dot(u.mirror.xyz, v.world) - u.mirror.w) * u.mirror.xyz;
            let normal = v.normal - 2.0 * dot(u.mirror.xyz, v.normal) * u.mirror.xyz;
            coverage = max(coverage, coverage_at(reflected, v.uv, normal, true));
        }
    } else {
        coverage = coverage_projected(v);
    }
    if(coverage<=0.0){discard;}
    mark_dirty(v.position.xy);
    return vec4<f32>(coverage,0.0,0.0,coverage);
}

fn grain_sample(uv: vec2<f32>) -> f32 {
    let q = uv * u.grain.y;
    let rotated = vec2<f32>(dot(q, vec2<f32>(u.grain.w, u.grain.z)),
                            dot(q, vec2<f32>(-u.grain.z, u.grain.w)));
    return textureSampleLevel(grain_pattern, grain_sampler, rotated, 0.0).r;
}

@fragment fn fs_grain(v: Varying) -> @location(0) f32 {
    if (v.object != u.ids.x || v.slot != u.ids.y) { discard; }
    var value: f32;
    if (u.grain_flags.x != 0u) {
        value = grain_sample(v.uv);
    } else {
        let p = v.world;
        let a = abs(v.normal);
        let w = a * a * a * a;
        let weights = w / max(dot(w, vec3<f32>(1.0)), 1e-4);
        value = grain_sample(vec2<f32>(p.z, -p.y)) * weights.x
              + grain_sample(p.xz) * weights.y
              + grain_sample(vec2<f32>(p.x, -p.y)) * weights.z;
    }
    return mix(1.0, select(value, 1.0 - value, u.grain_flags.y != 0u), u.grain.x);
}
