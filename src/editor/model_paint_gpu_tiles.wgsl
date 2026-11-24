// Compact touched UV tiles entirely on the GPU. No bounds readback or CPU wait.
struct Grid {columns:u32, rows:u32, expand:u32, pad:u32};
@group(0) @binding(0) var<uniform> u:Grid;
@group(0) @binding(1) var<storage,read> dirty:array<u32>;
// Header is also a dispatch-indirect command: x, y, z, tile count; IDs follow.
@group(0) @binding(2) var<storage,read_write> work:array<atomic<u32>>;
@compute @workgroup_size(64)
fn compact(@builtin(global_invocation_id) p:vec3<u32>){
    let i=p.x;
    if(i==0u){atomicStore(&work[0],64u);atomicStore(&work[2],1u);}
    if(i>=u.columns*u.rows){return;}
    var touched=dirty[i]!=0u;
    if(u.expand!=0u){
        let at=vec2<i32>(vec2<u32>(i%u.columns,i/u.columns));
        for(var y=-1;y<=1;y++){
            for(var x=-1;x<=1;x++){
                let q=at+vec2<i32>(x,y);
                if(all(q>=vec2<i32>(0))&&all(q<vec2<i32>(i32(u.columns),i32(u.rows)))){
                    touched=touched||dirty[u32(q.y)*u.columns+u32(q.x)]!=0u;
                }
            }
        }
    }
    if(!touched){return;}
    let slot=atomicAdd(&work[3],1u);
    atomicStore(&work[4u+slot],i);
    atomicMax(&work[1],slot/64u+1u);
}

// Each tile extends pixel bounds by the full 8x8 tile. Store minima
// as distance from the far edge so the whole buffer can be cleared to zero.
@compute @workgroup_size(64)
fn bounds(@builtin(global_invocation_id) p: vec3<u32>) {
    let i = p.x;
    if (i >= u.columns * u.rows || dirty[i] == 0u) { return; }
    let xy = vec2<u32>(i % u.columns, i / u.columns);
    atomicMax(&work[0], (xy.x + 1u) * 8u);
    atomicMax(&work[1], (xy.y + 1u) * 8u);
    atomicMax(&work[2], (u.columns - xy.x) * 8u);
    atomicMax(&work[3], (u.rows - xy.y) * 8u);
}
