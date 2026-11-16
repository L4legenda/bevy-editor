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

fn selected(uv: vec2<f32>) -> bool {
    if any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) { return false; }
    return textureSampleLevel(selection, selection_sampler, uv, 0.0).r >= 0.5;
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
    if outline {
        let dash = fract((in.uv.x / step_uv.x + in.uv.y / step_uv.y + selection_style.y) / 8.0);
        let value = select(0.02,0.98,dash < 0.5);
        color = vec4<f32>(vec3<f32>(value),1.0);
    }
    return color;
}
