// @name Mesh Map
// @category Inputs
// @description A baked mesh map (ambient occlusion, curvature, edges and more) in the mesh UV space. Use it for masks that follow the model.
// @output gray
// @param map: meshmap

fn node(uv: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(map(uv));
}
