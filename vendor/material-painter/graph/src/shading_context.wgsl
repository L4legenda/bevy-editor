const PI: f32 = 3.141592653589793;

struct Surface {
    albedo: vec3<f32>,
    rough: f32,
    metal: f32,
    height: f32,
    a: vec4<f32>,
    b: vec4<f32>,
    e: vec4<f32>,
};

struct Shade {
    s: Surface,
    n: vec3<f32>,
    gn: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    light: vec3<f32>,
    shadow: f32,
    uv: vec2<f32>,
    wpos: vec3<f32>,
};
