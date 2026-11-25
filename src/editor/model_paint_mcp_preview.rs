//! MCP controls the current model viewport and reads its composed GPU image.
use super::*;
use crate::editor::model_document::ModelCamera;
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
    exposure_stops: Option<f32>,
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
    /// Omit to get the current report without changing the preview.
    pub(in crate::editor) show: Option<bool>,
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
    env: Option<Res<'w, environment::Settings>>,
    runtime: Option<Res<'w, engine::Runtime>>,
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
    let preview = args.preview.as_ref().map(preview_mode).transpose()?;
    if preview.is_some_and(|mode| !session.available(PaintAction::Preview3d(mode))) {
        return Err("The requested map is not available for the active texture set".into());
    }
    if preview.is_some_and(|mode| mode != Preview3d::Material)
        && (args.path_trace == Some(true) || args.layout == Some(View::Two))
    {
        return Err("Material and baked-map previews require the raster 3D viewport".into());
    }
    let mut post = input
        .env
        .as_deref()
        .ok_or("Model environment settings are unavailable")?
        .paint_post(Some(doc));
    if let Some(exposure) = args.exposure_stops {
        post.exposure = exposure;
        if !post.valid() || !doc.can_store_paint_preview() {
            return Err(
                "Exposure must be between -5 and 3 stops on a writable model preview".into(),
            );
        }
    }
    if previous != camera {
        viewer.restore_camera(camera);
        doc.model_camera = Some(camera);
        doc.ui_revision += 1;
    }
    if args.exposure_stops.is_some() && doc.paint_post != Some(post) {
        doc.paint_post = Some(post);
        doc.ui_revision += 1;
    }
    if let Some(layout) = args.layout.filter(|&layout| layout != session.view) {
        session.edit(PaintAction::View(layout));
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
    Ok(
        json!({"model_id": model, "layout": session.view, "path_trace": session.path_trace,
        "exposure_stops": post.exposure, "camera": {"center": camera.center, "distance": camera.distance,
        "yaw_degrees": camera.yaw.to_degrees(), "pitch_degrees": camera.pitch.to_degrees(),
        "fov_degrees": camera.fov.to_degrees(), "orthographic": camera.orthographic},
        "channel": session.diagnostic_channel().and_then(|index| mcp_state::CHANNEL_KEYS.get(index)),
        "bake_map": session.diagnostic_bake()}),
    )
}

fn camera_patch(previous: ModelCamera, args: &ViewArgs) -> Result<ModelCamera, String> {
    let camera = ModelCamera {
        center: args.center.unwrap_or(previous.center),
        distance: args.distance.unwrap_or(previous.distance),
        yaw: args
            .yaw_degrees
            .map(|yaw| yaw.rem_euclid(360.).to_radians())
            .unwrap_or(previous.yaw),
        pitch: args
            .pitch_degrees
            .map(f32::to_radians)
            .unwrap_or(previous.pitch),
        fov: args
            .fov_degrees
            .map(f32::to_radians)
            .unwrap_or(previous.fov),
        orthographic: args.orthographic.unwrap_or(previous.orthographic),
    };
    if !camera.valid() {
        return Err("Invalid model camera; use finite coordinates, positive distance, pitch within ±90° and FOV from 1° to 175°".into());
    }
    Ok(camera)
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
    editing: &editing::State,
    paint: &mut PaintState,
    args: CageArgs,
) -> Result<Value, String> {
    let model = active_model(editor, args.model_id)?;
    let doc = editing
        .documents
        .get(&model)
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
    if args.show.is_some()
        && (doc.paint_busy
            || input
                .runtime
                .as_deref()
                .is_some_and(engine::Runtime::stroke_active))
    {
        return Err("Finish the current paint operation first".into());
    }
    if args.show == Some(true) {
        if !session.available(PaintAction::BakeInspectCage)
            || !doc
                .objects
                .iter()
                .any(|object| object.id == binding.0 && !object.hidden)
            || !session
                .set_appearance
                .visibility
                .shown(binding.0, binding.1)
        {
            return Err("Cage preview needs a high-poly source and a visible texture set".into());
        }
        if session.panel != Panel::Maps {
            session.edit(PaintAction::Panel(Panel::Maps));
        }
        if !session.bake_inspect_cage || session.view == View::Two {
            session.set_cage_inspection(true);
        }
    } else if args.show == Some(false) && session.bake_inspect_cage {
        session.set_cage_inspection(false);
    }
    let visible = session.bake_inspect_cage
        && session.panel == Panel::Maps
        && session.view != View::Two
        && session.bake_projection.path.is_some()
        && session
            .set_appearance
            .visibility
            .shown(binding.0, binding.1);
    let report = if visible {
        state.report(model, doc, binding, session.cage_edit_version)
    } else {
        json!({"status": "hidden"})
    };
    Ok(json!({"model_id": model, "set": session.set, "visible": visible, "report": report}))
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
