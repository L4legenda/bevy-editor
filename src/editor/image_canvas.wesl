import bevy_ui_render::ui_vertex_output::UiVertexOutput;
import bevy_render::color_operations::linear_rgb_to_srgb;
import bevy_render::color_operations::srgb_to_linear_rgb;

@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
// Source document width/height, full-resolution flag, physical pixels per source pixel.
@group(1) @binding(2) var<uniform> parameters: vec4<f32>;
@group(1) @binding(3) var selection: texture_2d<f32>;
@group(1) @binding(4) var selection_sampler: sampler;
@group(1) @binding(5) var<uniform> selection_style: vec4<f32>;
@group(1) @binding(6) var<uniform> preview_rect: vec4<f32>;
@group(1) @binding(7) var transform_under: texture_2d<f32>;
@group(1) @binding(8) var transform_under_sampler: sampler;
@group(1) @binding(9) var transform_moving: texture_2d<f32>;
@group(1) @binding(10) var transform_moving_sampler: sampler;
@group(1) @binding(11) var transform_above: texture_2d<f32>;
@group(1) @binding(12) var transform_above_sampler: sampler;
@group(1) @binding(13) var<uniform> transform_rows: array<vec4<f32>,3>;
@group(1) @binding(14) var<uniform> transform_source: vec4<f32>;
@group(1) @binding(15) var<uniform> transform_style: vec4<f32>;
@group(1) @binding(16) var<uniform> transform_frame: vec4<f32>;
@group(1) @binding(17) var transform_selection: texture_2d<f32>;
@group(1) @binding(18) var transform_selection_sampler: sampler;
@group(1) @binding(19) var pencil_mask: texture_2d<f32>;
@group(1) @binding(20) var pencil_mask_sampler: sampler;
// Document-space mask origin and size. Zero size disables the read-only cursor.
@group(1) @binding(21) var<uniform> pencil_rect: vec4<f32>;
@group(1) @binding(22) var<uniform> pencil_color: vec4<f32>;

fn pencil_cell(p:vec2<f32>) -> bool {
    let cell=floor(p)-pencil_rect.xy;
    if any(cell<vec2<f32>(0.0)) || any(cell>=pencil_rect.zw) {return false;}
    if selection_style.x>0.5 && textureSampleLevel(selection,selection_sampler,(floor(p)+vec2<f32>(0.5))/parameters.xy,0.0).r<=0.0 {return false;}
    return textureLoad(pencil_mask,vec2<i32>(cell),0).r>0.5;
}

fn source_uv(uv: vec2<f32>) -> vec2<f32> {
    let p=vec3<f32>(uv,1.0);
    let w=dot(transform_rows[2].xyz,p);
    if abs(w)<0.000001 {return vec2<f32>(-10000.0);}
    return vec2<f32>(dot(transform_rows[0].xyz,p),dot(transform_rows[1].xyz,p))/w;
}
fn over(src:vec4<f32>,dst:vec4<f32>) -> vec4<f32> {
    let a=src.a+dst.a*(1.0-src.a);
    return vec4<f32>((src.rgb*src.a+dst.rgb*dst.a*(1.0-src.a))/max(a,0.000001),a);
}
fn moving_texel(p:vec2<i32>,size:vec2<i32>) -> vec4<f32> {
    if any(p<vec2<i32>(0)) || any(p>=size) {return vec4<f32>(0.0);}
    let c=textureLoad(transform_moving,p,0);
    return vec4<f32>(linear_rgb_to_srgb(c.rgb)*c.a,c.a);
}
fn moving_pixel(uv:vec2<f32>) -> vec4<f32> {
    let size=vec2<i32>(textureDimensions(transform_moving));
    let p=uv*vec2<f32>(size)-vec2<f32>(0.5);
    let base=vec2<i32>(floor(p));
    var c=vec4<f32>(0.0);
    if transform_style.y>0.5 {
        c=moving_texel(vec2<i32>(floor(uv*vec2<f32>(size))),size);
    } else {
        let f=fract(p);
        c=mix(mix(moving_texel(base,size),moving_texel(base+vec2<i32>(1,0),size),f.x),
            mix(moving_texel(base+vec2<i32>(0,1),size),moving_texel(base+vec2<i32>(1,1),size),f.x),f.y);
    }
    return vec4<f32>(c.rgb/max(c.a,0.000001),c.a);
}

fn selected(uv: vec2<f32>) -> bool {
    var sample_uv=uv;
    if transform_style.x>0.5 {sample_uv=transform_frame.xy+source_uv(uv)*transform_frame.zw;}
    if any(sample_uv < vec2<f32>(0.0)) || any(sample_uv >= vec2<f32>(1.0)) { return false; }
    if transform_style.x>0.5 {
        return textureSampleLevel(transform_selection,transform_selection_sampler,sample_uv,0.0).r>=0.5;
    }
    return textureSampleLevel(selection, selection_sampler, sample_uv, 0.0).r >= 0.5;
}
fn draft_selected(p: vec2<f32>) -> bool {
    if selection_style.z > 1.5 {
        let center = (preview_rect.xy + preview_rect.zw) * 0.5;
        let radius = max((preview_rect.zw - preview_rect.xy) * 0.5, vec2<f32>(0.001));
        let q = (p - center) / radius;
        return dot(q,q) <= 1.0;
    }
    return all(p >= preview_rect.xy) && all(p < preview_rect.zw);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let texels = vec2<f32>(textureDimensions(image));
    let footprint = max(fwidth(in.uv * texels), vec2<f32>(0.000001));
    let per_texel = vec2<f32>(1.0) / footprint;
    let scale = min(per_texel.x, per_texel.y);
    let full_resolution = parameters.z > 0.5;
    let sharp = full_resolution && (scale >= 2.0 || (scale >= 0.999 && abs(scale - round(scale)) < 0.001));
    var uv = in.uv;
    if sharp {
        // Texel interiors stay flat. Only a one-device-pixel boundary is interpolated,
        // so fractional zoom and pan do not shimmer and integer zoom stays pixel-exact.
        let position = uv * texels - vec2<f32>(0.5);
        let base = floor(position);
        let edge = clamp((fract(position) - vec2<f32>(0.5)) * per_texel + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(1.0));
        uv = (base + vec2<f32>(0.5) + edge) / texels;
    }
    var color = textureSampleLevel(image, image_sampler, uv, 0.0);
    if transform_style.x>0.5 {
        let under=textureSampleLevel(transform_under,transform_under_sampler,uv,0.0);
        let above=textureSampleLevel(transform_above,transform_above_sampler,uv,0.0);
        var moving=vec4<f32>(0.0);
        let src=source_uv(in.uv);
        if all(src>=vec2<f32>(0.0)) && all(src<vec2<f32>(1.0)) {
            moving=moving_pixel(transform_source.xy+src*transform_source.zw);
            moving.a *= transform_style.z;
        }
        let c=over(vec4<f32>(linear_rgb_to_srgb(above.rgb),above.a),
            over(moving,vec4<f32>(linear_rgb_to_srgb(under.rgb),under.a)));
        color=vec4<f32>(srgb_to_linear_rgb(c.rgb),c.a);
    }
    let document = in.uv * parameters.xy;
    let per_pixel = vec2<f32>(1.0) / max(fwidth(document), vec2<f32>(0.000001));
    // Use the view's uniform zoom for the threshold: per-fragment derivatives can
    // straddle 5.0 through floating-point roundoff and make the grid flicker at 500%.
    if full_resolution && parameters.w > 5.0 && color.a > 0.0 {
        let edge = fract(document) * per_pixel;
        if edge.x < 1.0 || edge.y < 1.0 {
            // White on dark pixels, weaker black on light pixels. Source alpha keeps
            // the transparency checker clear; UI blending applies that alpha once.
            var srgb = linear_rgb_to_srgb(color.rgb);
            let dark_share = smoothstep(0.35, 0.75, dot(srgb, vec3<f32>(0.299, 0.587, 0.114)));
            let white = 0.25 * (1.0 - dark_share);
            let black = 0.16 * dark_share;
            srgb = srgb * (1.0 - white - black) + vec3<f32>(white);
            color = vec4<f32>(srgb_to_linear_rgb(srgb), color.a);
        }
    }
    let step_uv = max(fwidth(in.uv), vec2<f32>(0.0000001));
    var outline = false;
    if selection_style.x > 0.5 {
        let inside = selected(in.uv);
        outline = selected(in.uv + vec2<f32>(step_uv.x,0.0)) != inside
            || selected(in.uv - vec2<f32>(step_uv.x,0.0)) != inside
            || selected(in.uv + vec2<f32>(0.0,step_uv.y)) != inside
            || selected(in.uv - vec2<f32>(0.0,step_uv.y)) != inside;
    }
    if selection_style.z > 0.5 {
        let step_doc = step_uv * parameters.xy;
        let inside = draft_selected(document);
        outline = outline || draft_selected(document + vec2<f32>(step_doc.x,0.0)) != inside
            || draft_selected(document - vec2<f32>(step_doc.x,0.0)) != inside
            || draft_selected(document + vec2<f32>(0.0,step_doc.y)) != inside
            || draft_selected(document - vec2<f32>(0.0,step_doc.y)) != inside;
    }
    if pencil_rect.z>0.0 && pencil_cell(document) {
        // Each preview texel is one document pixel. The thin contrasting edge
        // follows those exact cells, including the staircase of a round tip.
        let step=max(step_uv*parameters.xy,vec2<f32>(0.00001));
        let border=!pencil_cell(document+vec2<f32>(step.x,0.0))
            || !pencil_cell(document-vec2<f32>(step.x,0.0))
            || !pencil_cell(document+vec2<f32>(0.0,step.y))
            || !pencil_cell(document-vec2<f32>(0.0,step.y));
        color=over(pencil_color,color);
        if border {
            let value=select(0.98,0.02,dot(color.rgb,vec3<f32>(0.299,0.587,0.114))>0.4);
            color=vec4<f32>(vec3<f32>(value),1.0);
        }
    }
    if outline {
        let dash = fract((in.uv.x / step_uv.x + in.uv.y / step_uv.y + selection_style.y) / 8.0);
        let value = select(0.02,0.98,dash < 0.5);
        color = vec4<f32>(vec3<f32>(value),1.0);
    }
    return color;
}
