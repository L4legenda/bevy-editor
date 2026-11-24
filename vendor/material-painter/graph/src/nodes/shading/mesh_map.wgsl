// @name Mesh Map
// @category Surface
// @description A baked mesh map: occlusion, curvature, cavity, thickness, sky, edges.
// @shading
// @output gray
// @param map: enum(Ambient Occlusion, Curvature, Cavity, Thickness, Sky Visibility, Edge Distance, Crease Distance, Part Random) = Ambient Occlusion

fn node(c: Shade) -> vec4<f32> {
    switch params.map {
        case 0: { return vec4<f32>(c.s.a.r); }
        case 1: { return vec4<f32>(c.s.a.g); }
        case 2: { return vec4<f32>(c.s.a.b); }
        case 3: { return vec4<f32>(c.s.a.a); }
        case 4: { return vec4<f32>(c.s.b.r); }
        case 5: { return vec4<f32>(c.s.b.g); }
        case 6: { return vec4<f32>(c.s.b.b); }
        default: { return vec4<f32>(c.s.b.a); }
    }
}
