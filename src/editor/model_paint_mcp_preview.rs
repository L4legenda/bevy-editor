//! MCP controls the current model viewport and reads its composed GPU image.
use super::*;
use crate::editor::model_document::{Document, ModelCamera, PaintPreview, PaintUv};
use bevy::{
    ecs::system::SystemParam,
    render::{
        gpu_readback::{ReadbackComplete, ReadbackOnce},
        render_resource::{TextureFormat, TextureUsages},
        renderer::RenderDevice,
    },
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[path = "model_paint_mcp_render.rs"]
pub(in crate::editor::ui::model_viewer) mod offscreen;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct ViewArgs {
    model_id: usize,
    center: Option<[f32; 3]>,
    distance: Option<f32>,
    yaw_degrees: Option<f32>,
    pitch_degrees: Option<f32>,
    fov_degrees: Option<f32>,
    orthographic: Option<bool>,
    layout: Option<View>,
    preview: Option<MapPreview>,
    path_trace: Option<bool>,
    /// Sampling and ground settings for the live path-traced viewport.
    trace_settings: Option<TracePatch>,
    /// Show the model's 3D wireframe in the raster viewport.
    wireframe: Option<bool>,
    uv_overlay: Option<UvOverlayPatch>,
    material: Option<MaterialPreviewPatch>,
    /// Edit the same shared model preview environment as the editor's lighting panel.
    environment: Option<environment::PreviewPatch>,
    exposure_stops: Option<f32>,
    post: Option<PostPatch>,
    /// Remove the model's local grading override and start from the environment settings.
    #[serde(default)]
    reset_post: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct TracePatch {
    /// Maximum samples per pixel, 16–4096.
    samples: Option<u32>,
    /// Maximum light bounces, 1–16.
    bounces: Option<u32>,
    /// Show the ground plane and its shadows.
    ground: Option<bool>,
}

impl TracePatch {
    fn resolve(&self, mut settings: pathtrace::Settings) -> Result<pathtrace::Settings, String> {
        for (name, new, current, range) in [
            ("samples", self.samples, &mut settings.samples, pathtrace::Settings::SAMPLE_RANGE),
            ("bounces", self.bounces, &mut settings.bounces, pathtrace::Settings::BOUNCE_RANGE),
        ] {
            if let Some(value) = new {
                if !range.contains(&value) {
                    return Err(format!("{name} must be between {} and {}", range.start(), range.end()));
                }
                *current = value;
            }
        }
        settings.ground = self.ground.unwrap_or(settings.ground);
        Ok(settings)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct UvOverlayPatch {
    show_wire: Option<bool>,
    /// UV wire opacity as a fraction from 0.05 to 1.
    wire_opacity: Option<f32>,
    show_seams_2d: Option<bool>,
    show_seams_3d: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct MaterialPreviewPatch {
    /// Set an explicit relief strength from 0 to 8.
    height_strength: Option<f32>,
    /// Use resolution-dependent relief strength instead of an explicit value.
    #[serde(default)]
    automatic_height: bool,
    emission_strength: Option<f32>,
    use_baked_normal: Option<bool>,
    /// Ambient occlusion strength as a fraction from 0 to 1.
    ao_strength: Option<f32>,
}

fn preview_value(key: &str, parameter: PreviewParameter, value: f32, scale: f32) -> Result<f32, String> {
    let (min, max) = (parameter.min() / scale, parameter.max() / scale);
    if !(min..=max).contains(&value) {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(value)
}

impl ViewArgs {
    fn overlay_patch(&self, mut uv: PaintUv) -> Result<PaintUv, String> {
        uv.show_wire_3d = self.wireframe.unwrap_or(uv.show_wire_3d);
        if let Some(patch) = &self.uv_overlay {
            uv.show_wire = patch.show_wire.unwrap_or(uv.show_wire);
            uv.show_seams_2d = patch.show_seams_2d.unwrap_or(uv.show_seams_2d);
            uv.show_seams_3d = patch.show_seams_3d.unwrap_or(uv.show_seams_3d);
            if let Some(value) = patch.wire_opacity {
                uv.wire_opacity = preview_value("wire_opacity", PreviewParameter::UvWireOpacity, value, 100.)?;
            }
        }
        Ok(uv)
    }

    fn material_patch(&self, mut material: PaintPreview) -> Result<PaintPreview, String> {
        let Some(patch) = &self.material else { return Ok(material) };
        if patch.automatic_height && patch.height_strength.is_some() {
            return Err("Choose either height_strength or automatic_height".into());
        }
        if patch.automatic_height {
            material.height_strength = None;
        } else if let Some(value) = patch.height_strength {
            material.height_strength = Some(preview_value("height_strength", PreviewParameter::Height, value, 1.)?);
        }
        if let Some(value) = patch.emission_strength {
            material.emission_strength = preview_value("emission_strength", PreviewParameter::Emission, value, 1.)?;
        }
        if let Some(value) = patch.ao_strength {
            material.ao_strength = preview_value("ao_strength", PreviewParameter::Occlusion, value, 100.)?;
        }
        material.use_baked_normal = patch.use_baked_normal.unwrap_or(material.use_baked_normal);
        Ok(material)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct PostPatch {
    tone_map: Option<crate::ToneMap>,
    /// Use the environment tone mapper while retaining the other model grading settings.
    #[serde(default)]
    inherit_tone_map: bool,
    contrast: Option<f32>,
    saturation: Option<f32>,
    temperature: Option<f32>,
    tint: Option<f32>,
    bloom: Option<f32>,
    bloom_threshold: Option<f32>,
    vignette: Option<f32>,
    #[serde(alias = "chromatic")]
    chromatic_aberration: Option<f32>,
    sharpen: Option<f32>,
    grain: Option<f32>,
}

impl PostPatch {
    fn apply(&self, effects: &mut post::Effects) -> Result<(), String> {
        use post::Field;
        if self.inherit_tone_map && self.tone_map.is_some() {
            return Err("Choose either tone_map or inherit_tone_map".into());
        }
        if self.inherit_tone_map {
            effects.curve = None;
        } else if let Some(curve) = self.tone_map {
            effects.curve = Some(curve);
        }
        for (key, field, value) in [
            ("contrast", Field::Contrast, self.contrast),
            ("saturation", Field::Saturation, self.saturation),
            ("temperature", Field::Temperature, self.temperature),
            ("tint", Field::Tint, self.tint),
            ("bloom", Field::Bloom, self.bloom),
            ("bloom_threshold", Field::BloomThreshold, self.bloom_threshold),
            ("vignette", Field::Vignette, self.vignette),
            ("chromatic_aberration", Field::Chromatic, self.chromatic_aberration),
            ("sharpen", Field::Sharpen, self.sharpen),
            ("grain", Field::Grain, self.grain),
        ] {
            if let Some(value) = value {
                let (min, max) = field.range();
                if !(min..=max).contains(&value) {
                    return Err(format!("{key} must be between {min} and {max}"));
                }
                field.set(effects, value);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum MapPreview {
    Material,
    Channel { channel: String },
    Baked { map: bake::BakeMap },
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct CageArgs {
    model_id: usize,
    /// Defaults to showing the preview when changing projection settings.
    show: Option<bool>,
    /// Update projection settings without starting a texture bake.
    projection: Option<bake::ProjectionPatch>,
}

impl CageArgs {
    pub(in crate::editor) fn mutates(&self) -> bool {
        self.show.is_some() || self.projection.is_some()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct CaptureArgs {
    model_id: usize,
}

#[derive(SystemParam)]
pub(in crate::editor) struct McpPreview<'w> {
    viewer: Option<ResMut<'w, Viewer>>,
    cage: Option<Res<'w, bake::cage_preview::State>>,
    bake: Option<Res<'w, bake::State>>,
    env: Option<ResMut<'w, environment::Settings>>,
    runtime: Option<Res<'w, engine::Runtime>>,
    trace: Option<Res<'w, pathtrace::viewport::Main>>,
}

impl McpPreview<'_> {
    pub(super) fn trace_progress(&self, index: usize, doc: &Document, session: &Session) -> Value {
        self.trace.as_deref().map_or_else(
            || json!({"status": "unavailable", "reason": null, "samples": null,
                "target_samples": session.trace_settings.samples}),
            |trace| trace.progress(index, doc, session),
        )
    }
}

fn active_model(editor: &Editor, model: usize) -> Result<usize, String> {
    let index = mcp_state::model_index(editor)?;
    if index != model {
        return Err("The active model changed; call get_paint_state again".into());
    }
    if editor.pending.is_some() || editor.browser_popup || editor.hdri_library_open {
        return Err("Close the current editor overlay to use the model viewport".into());
    }
    Ok(index)
}

pub(in crate::editor) fn set_view(
    input: &mut McpPreview,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: ViewArgs,
) -> Result<Value, String> {
    let model = active_model(editor, args.model_id)?;
    let doc = editing
        .documents
        .get_mut(&model)
        .filter(|doc| doc.mode == Mode::Paint)
        .ok_or("Switch the model to Painting mode first")?;
    let session = paint
        .sessions
        .get_mut(&model)
        .ok_or("Wait for Painting mode to load")?;
    let viewer = input
        .viewer
        .as_deref_mut()
        .filter(|viewer| {
            viewer.model == Some(model) && viewer.fitted && !viewer.pending_document_camera
        })
        .ok_or("Wait for the 3D viewport to load")?;
    if viewer.gesture.is_some()
        || doc.paint_busy
        || input
            .runtime
            .as_deref()
            .is_some_and(engine::Runtime::stroke_active)
    {
        return Err("Finish the current model gesture or paint operation first".into());
    }
    let previous = viewer.camera();
    let camera = camera_patch(previous, &args)?;
    let uv = args.overlay_patch(doc.paint_uv)?;
    let material = args.material_patch(doc.paint_preview)?;
    let trace = args.trace_settings.as_ref().map(|patch| patch.resolve(session.trace_settings)).transpose()?;
    if (uv, material) != (doc.paint_uv, doc.paint_preview) && !doc.can_store_paint_preview() {
        return Err("This model cannot store the requested preview settings".into());
    }
    let preview = args.preview.as_ref().map(preview_mode).transpose()?;
    if preview.is_some_and(|mode| !session.available(PaintAction::Preview3d(mode))) {
        return Err("The requested map is not available for the active texture set".into());
    }
    if preview.is_some_and(|mode| mode != Preview3d::Material)
        && (args.path_trace == Some(true) || args.layout == Some(View::Two))
    {
        return Err("Material and baked-map previews require the raster 3D viewport".into());
    }
    let env = input
        .env
        .as_deref()
        .ok_or("Model environment settings are unavailable")?;
    let next_env = args.environment.as_ref().map(|patch| patch.resolve(env)).transpose()?;
    let effective_env = next_env.as_ref().unwrap_or(env);
    let inherited = args.reset_post.then(|| effective_env.paint_post(None));
    let mut post = inherited.unwrap_or_else(|| effective_env.paint_post(Some(doc)));
    let effects = args.post.as_ref().filter(|patch| **patch != PostPatch::default());
    if let Some(effects) = effects {
        effects.apply(&mut post.effects)?;
    }
    if let Some(exposure) = args.exposure_stops {
        post.exposure = exposure;
        let (min, max) = DocumentPost::EXPOSURE_RANGE;
        if !(min..=max).contains(&exposure) {
            return Err(format!("Exposure must be between {min} and {max} stops"));
        }
    }
    let next_post = if args.exposure_stops.is_some() || effects.is_some() {
        Some(post)
    } else if args.reset_post {
        None
    } else {
        doc.paint_post
    };
    if next_post != doc.paint_post && (!doc.can_store_paint_preview() || !post.valid()) {
        return Err("This model cannot store the requested preview grading".into());
    }
    let environment = effective_env.mcp_preview();
    let tone_map = post.effects.curve.unwrap_or_else(|| effective_env.trace_light(None).2);
    if let Some(next_env) = next_env.filter(|next| next != env) {
        next_env.save(&editor.project.asset_root)
            .map_err(|error| format!("Could not save the model preview environment: {error}"))?;
        *input.env.as_deref_mut().unwrap() = next_env;
    }
    if previous != camera {
        viewer.restore_camera(camera);
        doc.model_camera = Some(camera);
    }
    if (doc.paint_post, doc.paint_uv, doc.paint_preview) != (next_post, uv, material) || previous != camera {
        doc.paint_post = next_post;
        doc.paint_uv = uv;
        doc.paint_preview = material;
        doc.ui_revision += 1;
    }
    if let Some(layout) = args.layout.filter(|&layout| layout != session.view) {
        session.edit(PaintAction::View(layout));
    }
    if let Some(trace) = trace
        && (trace.samples, trace.bounces, trace.ground)
            != (session.trace_settings.samples, session.trace_settings.bounces, session.trace_settings.ground)
    {
        session.trace_settings = trace;
        session.revision += 1;
    }
    if let Some(path_trace) = args
        .path_trace
        .filter(|&enable| enable != session.path_trace)
    {
        session.edit(PaintAction::PathTrace);
        debug_assert_eq!(session.path_trace, path_trace);
    }
    if let Some(mode) = preview {
        session.edit(PaintAction::Preview3d(mode));
    }
    let trace = session.trace_settings;
    Ok(
        json!({"model_id": model, "layout": session.view, "path_trace": session.path_trace,
        "trace_settings": {"samples": trace.samples, "bounces": trace.bounces, "ground": trace.ground},
        "exposure_stops": post.exposure, "camera": {"center": camera.center, "distance": camera.distance,
        "yaw_degrees": camera.yaw.to_degrees(), "pitch_degrees": camera.pitch.to_degrees(),
        "fov_degrees": camera.fov.to_degrees(), "orthographic": camera.orthographic},
        "channel": session.diagnostic_channel().and_then(|index| mcp_state::CHANNEL_KEYS.get(index)),
        "bake_map": session.diagnostic_bake(),
        "wireframe": uv.show_wire_3d, "uv_overlay": {"show_wire": uv.show_wire,
            "wire_opacity": uv.wire_opacity, "show_seams_2d": uv.show_seams_2d, "show_seams_3d": uv.show_seams_3d},
        "material": material,
        "environment": environment,
        "post": {"overridden": doc.paint_post.is_some(), "effects": post.effects,
            "tone_map": tone_map}}),
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct CameraPatch {
    center: Option<[f32; 3]>,
    distance: Option<f32>,
    yaw_degrees: Option<f32>,
    pitch_degrees: Option<f32>,
    fov_degrees: Option<f32>,
    orthographic: Option<bool>,
}

fn camera_patch(previous: ModelCamera, args: &ViewArgs) -> Result<ModelCamera, String> {
    CameraPatch {
        center: args.center,
        distance: args.distance,
        yaw_degrees: args.yaw_degrees,
        pitch_degrees: args.pitch_degrees,
        fov_degrees: args.fov_degrees,
        orthographic: args.orthographic,
    }.apply(previous)
}

impl CameraPatch {
    fn apply(&self, previous: ModelCamera) -> Result<ModelCamera, String> {
        let camera = ModelCamera {
            center: self.center.unwrap_or(previous.center),
            distance: self.distance.unwrap_or(previous.distance),
            yaw: self.yaw_degrees
                .map(|yaw| yaw.rem_euclid(360.).to_radians())
                .unwrap_or(previous.yaw),
            pitch: self.pitch_degrees.map(f32::to_radians).unwrap_or(previous.pitch),
            fov: self.fov_degrees.map(f32::to_radians).unwrap_or(previous.fov),
            orthographic: self.orthographic.unwrap_or(previous.orthographic),
        };
        if !camera.valid() {
            return Err("Invalid model camera; use finite coordinates, positive distance, pitch within ±90° and FOV from 1° to 175°".into());
        }
        Ok(camera)
    }
}

fn preview_mode(preview: &MapPreview) -> Result<Preview3d, String> {
    Ok(match preview {
        MapPreview::Material => Preview3d::Material,
        MapPreview::Channel { channel } => Preview3d::Channel(
            mcp_state::CHANNEL_KEYS
                .iter()
                .position(|key| key == channel)
                .ok_or("Unknown paint channel; use list_paint_presets")?,
        ),
        MapPreview::Baked { map } => Preview3d::Baked(*map),
    })
}

pub(in crate::editor) fn cage(
    input: &McpPreview,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: CageArgs,
) -> Result<Value, String> {
    let model = active_model(editor, args.model_id)?;
    let doc = editing
        .documents
        .get_mut(&model)
        .filter(|doc| doc.mode == Mode::Paint)
        .ok_or("Switch the model to Painting mode first")?;
    let session = paint
        .sessions
        .get_mut(&model)
        .ok_or("Wait for Painting mode to load")?;
    let state = input
        .cage
        .as_deref()
        .ok_or("The cage preview is unavailable")?;
    let binding = session
        .binding
        .ok_or("Wait for the active texture set to load")?;
    if args.mutates()
        && (doc.paint_busy || doc.paint_pending || session.busy || session.resize_request.is_some()
            || input
                .runtime
                .as_deref()
                .is_some_and(engine::Runtime::stroke_active))
    {
        return Err("Finish the current paint operation first".into());
    }
    let settings = if let Some(patch) = &args.projection {
        if input.bake.as_deref().is_some_and(bake::State::configuration_busy) {
            return Err("Wait for the current bake operation to finish".into());
        }
        Some(patch.resolve(&editor.project.asset_root, &doc.bake_settings)?)
    } else {
        None
    };
    let show = args.show.or_else(|| settings.as_ref().map(|_| true));
    let set_visible = session.set_appearance.visibility.shown(binding.0, binding.1)
        && doc.objects.iter().any(|object| object.id == binding.0 && !object.hidden);
    if show == Some(true) && !set_visible {
        return Err("Cage preview needs a visible texture set".into());
    }
    if let Some(settings) = settings {
        bake::apply_settings(doc, session, settings);
    }
    if show == Some(true) {
        if session.panel != Panel::Maps {
            session.edit(PaintAction::Panel(Panel::Maps));
        }
        if !session.bake_inspect_cage || session.view == View::Two {
            session.set_cage_inspection(true);
        }
    } else if show == Some(false) && session.bake_inspect_cage {
        session.set_cage_inspection(false);
    }
    let visible = session.bake_inspect_cage
        && session.panel == Panel::Maps
        && session.view != View::Two
        && set_visible;
    let report = if visible {
        state.report(model, doc, binding, session.cage_edit_version)
    } else {
        json!({"status": "hidden"})
    };
    Ok(json!({
        "model_id": model, "set": session.set, "visible": visible, "report": report,
        "settings": doc.bake_settings,
    }))
}

pub(in crate::editor) type CaptureReply =
    Box<dyn FnOnce(Result<image::DynamicImage, String>) + Send + Sync>;

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct CaptureRequest {
    model: usize,
    reply: Option<CaptureReply>,
}

#[derive(Component)]
pub(in crate::editor::ui::model_viewer) struct CaptureDeadline(Instant);

pub(in crate::editor) fn capture(
    commands: &mut Commands,
    editor: &Editor,
    args: CaptureArgs,
    reply: CaptureReply,
) {
    match active_model(editor, args.model_id) {
        Ok(model) => {
            commands.spawn(CaptureRequest {
                model,
                reply: Some(reply),
            });
        }
        Err(error) => reply(Err(error)),
    }
}

pub(in crate::editor::ui::model_viewer) fn capture_requests(
    mut commands: Commands,
    editor: Res<Editor>,
    viewer: Res<Viewer>,
    paint: Res<PaintState>,
    editing: Res<editing::State>,
    device: Option<Res<RenderDevice>>,
    image_node: Query<(&ImageNode, &ComputedNode), With<pathtrace::viewport::CanvasImage>>,
    images: Res<Assets<Image>>,
    mut requests: Query<(Entity, &mut CaptureRequest)>,
    expired: Query<(Entity, &CaptureDeadline)>,
) {
    for (entity, deadline) in &expired {
        if Instant::now() >= deadline.0 {
            commands.entity(entity).try_despawn();
        }
    }
    for (entity, mut request) in &mut requests {
        let Some(reply) = request.reply.take() else {
            continue;
        };
        let target = (|| {
            active_model(&editor, request.model)?;
            if !viewer.fitted
                || viewer.model != Some(request.model)
                || device.is_none()
                || editing.mode(Some(request.model)) != Mode::Paint
                || paint.sessions.get(&request.model).is_none_or(|session| {
                    session.view == View::Two || session.panel == Panel::Shader
                })
            {
                return Err("Open the 3D paint viewport before capturing it".into());
            }
            let (node, computed) = image_node.single().map_err(|_| "No model viewport image")?;
            let image = images
                .get(&node.image)
                .filter(|image| {
                    computed.size().min_element() >= 2.
                        && image
                            .texture_descriptor
                            .usage
                            .contains(TextureUsages::COPY_SRC)
                })
                .ok_or("Wait for the model viewport image to be ready")?;
            if image.texture_descriptor.format != TextureFormat::Rgba8UnormSrgb {
                return Err("Unsupported viewport image format".into());
            }
            Ok((node.image.clone(), image.width(), image.height()))
        })();
        match target {
            Ok((image, width, height)) => {
                let reply = std::sync::Mutex::new(Some(reply));
                commands
                    .entity(entity)
                    .remove::<CaptureRequest>()
                    .insert((
                        ReadbackOnce::texture(image),
                        CaptureDeadline(Instant::now() + Duration::from_secs(30)),
                    ))
                    .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                        if let Some(reply) = reply.lock().ok().and_then(|mut reply| reply.take()) {
                            reply(decode_pixels(width, height, &event.data));
                        }
                        commands.entity(event.entity).try_despawn();
                    });
            }
            Err(error) => {
                reply(Err(error));
                commands.entity(entity).try_despawn();
            }
        }
    }
}

fn decode_pixels(width: u32, height: u32, bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let row = width as usize * 4;
    let stride = RenderDevice::align_copy_bytes_per_row(row);
    if row == 0 || height == 0 || bytes.len() != stride * height as usize {
        return Err("Viewport resized during capture; retry with the new size".into());
    }
    let packed = bytes
        .chunks_exact(stride)
        .flat_map(|data| data[..row].iter().copied())
        .collect();
    image::RgbaImage::from_raw(width, height, packed)
        .map(image::DynamicImage::ImageRgba8)
        .ok_or_else(|| "Could not read the viewport pixels".into())
}
