//! Effect property conversion adapted from PhotoCraft ui-egui layer_style.rs.
//! Copyright PhotoCraft contributors; MIT OR Apache-2.0 (see vendor/photocraft).
use photocraft_doc::{Effect, FxPaint, GlowSource, GlowTechnique, StrokePosition};
use serde_json::{Value, json};
fn hex(color: &photocraft_doc::Color) -> String {
    let [r, g, b, _] = color.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}
pub(super) fn values_of(e: &Effect, light: f32) -> Value {
    let mut v =
        photocraft_engine::layer_style::effect_defaults(photocraft_engine::layer_style::kind_of(e));
    let set = |v: &mut Value, k: &str, x: Value| {
        v[k] = x;
    };
    let drop_keys = |v: &mut Value, keys: &[&str]| {
        if let Some(o) = v.as_object_mut() {
            for k in keys {
                o.remove(*k);
            }
        }
    };
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(
                &mut v,
                "angle",
                json!(if s.use_global_light { light } else { s.angle }),
            );
            set(&mut v, "useGlobalLight", json!(s.use_global_light));
            set(&mut v, "distance", json!(s.distance));
            set(
                &mut v,
                if matches!(e, Effect::DropShadow(_)) {
                    "spread"
                } else {
                    "choke"
                },
                json!((s.spread * 100.0).round()),
            );
            set(&mut v, "size", json!(s.size));
            set(
                &mut v,
                "contour",
                photocraft_engine::layer_style::contour_param(&s.contour),
            );
            set(&mut v, "antiAlias", json!(s.anti_alias));
            set(&mut v, "noise", json!((s.noise * 100.0).round()));
            if matches!(e, Effect::DropShadow(_)) {
                set(&mut v, "knocksOut", json!(s.knocks_out));
            }
        }
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => {
            set(&mut v, "blend", json!(g.common.blend.label()));
            set(&mut v, "opacity", json!((g.common.opacity * 100.0).round()));
            if let FxPaint::Color(c) = &g.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                drop_keys(&mut v, &["color"]);
            }
            set(
                &mut v,
                "technique",
                json!(if g.technique == GlowTechnique::Precise {
                    "precise"
                } else {
                    "softer"
                }),
            );
            set(
                &mut v,
                if matches!(e, Effect::OuterGlow(_)) {
                    "spread"
                } else {
                    "choke"
                },
                json!((g.spread * 100.0).round()),
            );
            set(&mut v, "size", json!(g.size));
            set(&mut v, "range", json!((g.range * 100.0).round()));
            set(
                &mut v,
                "contour",
                photocraft_engine::layer_style::contour_param(&g.contour),
            );
            set(&mut v, "antiAlias", json!(g.anti_alias));
            set(&mut v, "noise", json!((g.noise * 100.0).round()));
            if matches!(e, Effect::InnerGlow(_)) {
                set(
                    &mut v,
                    "source",
                    json!(if g.source == GlowSource::Center {
                        "center"
                    } else {
                        "edge"
                    }),
                );
            }
        }
        Effect::Stroke(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(
                &mut v,
                "position",
                json!(match s.position {
                    StrokePosition::Inside => "inside",
                    StrokePosition::Center => "center",
                    _ => "outside",
                }),
            );
            set(&mut v, "overprint", json!(s.overprint));
            let gradient = if let FxPaint::Gradient(gradient) = &s.paint {
                Some(gradient)
            } else {
                None
            };
            set(
                &mut v,
                "strokePaint",
                json!(if gradient.is_some() {
                    "gradient"
                } else {
                    "solid"
                }),
            );
            set(
                &mut v,
                "from",
                json!(
                    gradient
                        .and_then(|g| g.stops.first())
                        .map_or("#000000".into(), |(_, c)| hex(c))
                ),
            );
            set(
                &mut v,
                "to",
                json!(
                    gradient
                        .and_then(|g| g.stops.last())
                        .map_or("#ffffff".into(), |(_, c)| hex(c))
                ),
            );
            set(&mut v, "angle", json!(gradient.map_or(90.0, |g| g.angle)));
            use photocraft_doc::effects::GradientStyle;
            set(
                &mut v,
                "style",
                json!(match gradient.map(|g| g.style) {
                    Some(GradientStyle::Radial) => "radial",
                    Some(GradientStyle::Angle) => "angle",
                    Some(GradientStyle::Reflected) => "reflected",
                    Some(GradientStyle::Diamond) => "diamond",
                    _ => "linear",
                }),
            );
            if let FxPaint::Color(c) = &s.paint {
                set(&mut v, "color", json!(hex(c)));
            }
        }
        Effect::ColorOverlay { common, color } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(color)));
        }
        Effect::GradientOverlay {
            common,
            gradient: g,
            interpolation,
            ..
        } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            let stops = interpolation
                .as_ref()
                .map_or(&g.stops, |source| &source.stops);
            if let Some(source) = interpolation {
                set(&mut v, "method", json!(source.method));
                set(&mut v, "smoothness", json!(source.smoothness));
            }
            set(
                &mut v,
                "stops",
                Value::Array(stops.iter().map(|(t, c)| json!([t, hex(c)])).collect()),
            );
            set(
                &mut v,
                "opacityStops",
                Value::Array(
                    g.opacity_stops
                        .iter()
                        .map(|(t, a)| json!([t, a * 100.0]))
                        .collect(),
                ),
            );
            set(&mut v, "reverse", json!(g.reverse));
            set(&mut v, "offset", json!([g.offset.0, g.offset.1]));
            set(
                &mut v,
                "style",
                json!(match g.style {
                    photocraft_doc::effects::GradientStyle::Radial => "radial",
                    photocraft_doc::effects::GradientStyle::Angle => "angle",
                    photocraft_doc::effects::GradientStyle::Reflected => "reflected",
                    photocraft_doc::effects::GradientStyle::Diamond => "diamond",
                    photocraft_doc::effects::GradientStyle::Linear => "linear",
                }),
            );
            set(&mut v, "angle", json!(g.angle));
            set(&mut v, "scale", json!((g.scale * 100.0).round()));
        }
        Effect::PatternOverlay {
            common,
            name,
            id,
            scale,
            angle,
            link,
            phase,
        } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(
                &mut v,
                "pattern",
                json!(if id.is_empty() { name } else { id }),
            );
            set(&mut v, "scale", json!((scale * 100.0).round()));
            set(&mut v, "angle", json!(angle));
            set(&mut v, "link", json!(link));
            set(&mut v, "phaseX", json!(phase.0));
            set(&mut v, "phaseY", json!(phase.1));
        }
        Effect::Satin(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(s.angle));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "invert", json!(s.invert));
            set(
                &mut v,
                "contour",
                photocraft_engine::layer_style::contour_param(&s.contour),
            );
            set(&mut v, "antiAlias", json!(s.anti_alias));
        }
        Effect::BevelEmboss(b) => {
            set(
                &mut v,
                "style",
                json!(match b.style {
                    photocraft_doc::effects::BevelStyle::OuterBevel => "outer",
                    photocraft_doc::effects::BevelStyle::Emboss => "emboss",
                    photocraft_doc::effects::BevelStyle::PillowEmboss => "pillow",
                    photocraft_doc::effects::BevelStyle::StrokeEmboss => "stroke",
                    photocraft_doc::effects::BevelStyle::InnerBevel => "inner",
                }),
            );
            set(
                &mut v,
                "technique",
                json!(match b.technique {
                    photocraft_doc::effects::BevelTechnique::ChiselHard => "chiselHard",
                    photocraft_doc::effects::BevelTechnique::ChiselSoft => "chiselSoft",
                    photocraft_doc::effects::BevelTechnique::Smooth => "smooth",
                }),
            );
            set(&mut v, "depth", json!((b.depth * 100.0).round()));
            set(&mut v, "size", json!(b.size));
            set(&mut v, "soften", json!(b.soften));
            set(
                &mut v,
                "angle",
                json!(if b.use_global_light { light } else { b.angle }),
            );
            set(&mut v, "useGlobalLight", json!(b.use_global_light));
            set(&mut v, "altitude", json!(b.altitude));
            set(&mut v, "direction", json!(if b.up { "up" } else { "down" }));
            set(
                &mut v,
                "glossContour",
                photocraft_engine::layer_style::contour_param(&b.gloss_contour),
            );
            set(&mut v, "glossAntiAlias", json!(b.gloss_anti_alias));
            set(&mut v, "highlightBlend", json!(b.highlight.blend.label()));
            set(&mut v, "highlightColor", json!(hex(&b.highlight_color)));
            set(
                &mut v,
                "highlightOpacity",
                json!((b.highlight.opacity * 100.0).round()),
            );
            set(&mut v, "shadowBlend", json!(b.shadow.blend.label()));
            set(&mut v, "shadowColor", json!(hex(&b.shadow_color)));
            set(
                &mut v,
                "shadowOpacity",
                json!((b.shadow.opacity * 100.0).round()),
            );
            set(&mut v, "contour", json!(b.contour.is_some()));
            if let Some(c) = &b.contour {
                set(
                    &mut v,
                    "contourName",
                    photocraft_engine::layer_style::contour_param(&c.contour),
                );
                set(&mut v, "contourRange", json!((c.range * 100.0).round()));
                set(&mut v, "contourAntiAlias", json!(c.anti_alias));
            }
        }
    }
    v
}
