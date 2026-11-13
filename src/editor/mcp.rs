//! MCP access to the live editor. All scene changes run on Bevy's main thread.
mod server;
#[cfg(test)]
mod tests;
mod types;

pub use server::run_stdio;

use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use base64::Engine;
use bevy::{
    camera::primitives::Aabb,
    input_focus::InputFocus,
    prelude::*,
    render::{
        render_resource::TextureUsages,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    winit::{EventLoopProxyWrapper, WinitUserEvent},
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{Editor, Field, Scene, Selection, ui, viewport};
use types::Command;
use types::*;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) enum Payload {
    Json(Value),
    Image {
        png: String,
        width: u32,
        height: u32,
    },
}
type Reply = Result<Payload, String>;

struct Job {
    command: Command,
    reply: oneshot::Sender<Reply>,
    deadline: Instant,
}

#[derive(Clone)]
struct Bridge {
    sender: mpsc::SyncSender<Job>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Bridge {
    async fn request(&self, command: Command) -> Reply {
        let (reply, receiver) = oneshot::channel();
        self.sender
            .try_send(Job {
                command,
                reply,
                deadline: Instant::now() + REQUEST_TIMEOUT,
            })
            .map_err(|_| "The editor is busy or has closed; retry after it is ready".to_string())?;
        (self.wake)();
        tokio::time::timeout(REQUEST_TIMEOUT, receiver)
            .await
            .map_err(|_| "The editor did not respond within 30 seconds".to_string())?
            .map_err(|_| "The editor closed before completing the request".to_string())?
    }
}

#[derive(Resource)]
struct Inbox(Mutex<mpsc::Receiver<Job>>);

/// Add after `EditorPlugin` to expose this editor instance through local MCP.
/// The desktop executable enables it by default; library hosts opt in here.
#[derive(Default)]
pub struct McpPlugin;

impl Plugin for McpPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start.after(super::setup))
            .add_systems(
                Update,
                process
                    .after(super::handle_actions)
                    .before(super::begin_room),
            );
    }
}

fn start(
    mut commands: Commands,
    editor: Res<Editor>,
    event_loop: Option<Res<EventLoopProxyWrapper>>,
    surface: Option<Res<viewport::Surface>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (sender, receiver) = mpsc::sync_channel(64);
    let proxy = event_loop
        .as_deref()
        .map(|proxy| std::ops::Deref::deref(proxy).clone());
    let bridge = Bridge {
        sender,
        wake: Arc::new(move || {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(WinitUserEvent::WakeUp);
            }
        }),
    };
    match server::start(&editor.project, bridge) {
        Ok(server) => {
            if let Some(surface) = surface
                && let Some(mut image) = images.get_mut(&surface.0)
            {
                image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            }
            commands.insert_resource(Inbox(Mutex::new(receiver)));
            commands.insert_resource(server);
        }
        Err(error) => warn!("Could not start editor MCP: {error}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn process(
    mut commands: Commands,
    inbox: Option<Res<Inbox>>,
    mut editor: ResMut<Editor>,
    scene: Res<Scene>,
    mut pointer: ResMut<viewport::PointerState>,
    numeric: Option<Res<ui::NumericInput>>,
    lighting: Option<Res<ui::LightingUi>>,
    drag: Option<Res<ui::AssetDrag>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    surface: Option<Res<viewport::Surface>>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    transforms: Query<&GlobalTransform>,
    mut focus: Option<ResMut<InputFocus>>,
) {
    let Some(inbox) = inbox else { return };
    for _ in 0..16 {
        let Some(job) = inbox
            .0
            .lock()
            .ok()
            .and_then(|receiver| receiver.try_recv().ok())
        else {
            break;
        };
        // Dropping a timed-out HTTP request must not apply its queued mutation later.
        if job.reply.is_closed() || Instant::now() >= job.deadline {
            continue;
        }
        let mutates = job.command.mutates();
        let busy = editor.pending.is_some()
            || editor.camera_navigation.is_some()
            || pointer.dragging()
            || numeric.as_ref().is_some_and(|numeric| {
                numeric.has_changes(&editor) || numeric.error_message().is_some()
            })
            || lighting.as_ref().is_some_and(|lighting| {
                lighting.name_has_changes(&editor) || lighting.is_control_active()
            })
            || drag.as_ref().is_some_and(|drag| drag.active())
            || mouse.as_ref().is_some_and(|mouse| {
                mouse.any_pressed([MouseButton::Left, MouseButton::Middle, MouseButton::Right])
            });
        if mutates && busy {
            let _ = job.reply.send(Err(
                "Finish the active edit, drag, camera navigation or confirmation dialog first"
                    .into(),
            ));
            continue;
        }
        if matches!(job.command, Command::Capture) {
            if !editor.ready || !editor.has_scene() {
                let _ = job
                    .reply
                    .send(Err("Wait for an open scene to finish loading".into()));
                continue;
            }
            let Some(surface) = surface.as_ref() else {
                let _ = job
                    .reply
                    .send(Err("This editor has no rendered viewport".into()));
                continue;
            };
            let reply = Mutex::new(Some(job.reply));
            commands
                .spawn(Screenshot::image(surface.0.clone()))
                .observe(move |captured: On<ScreenshotCaptured>| {
                    let Some(reply) = reply.lock().ok().and_then(|mut reply| reply.take()) else {
                        return;
                    };
                    if reply.is_closed() {
                        return;
                    }
                    let image = captured
                        .image
                        .clone()
                        .try_into_dynamic()
                        .map_err(|error| error.to_string());
                    let result = image.and_then(|image| {
                        let image = image.thumbnail(1600, 1000);
                        let mut output = std::io::Cursor::new(Vec::new());
                        image
                            .write_to(&mut output, image::ImageFormat::Png)
                            .map_err(|error| error.to_string())?;
                        Ok(Payload::Image {
                            png: base64::engine::general_purpose::STANDARD
                                .encode(output.into_inner()),
                            width: image.width(),
                            height: image.height(),
                        })
                    });
                    let _ = reply.send(result);
                });
            continue;
        }
        let result = if matches!(job.command, Command::Save) {
            if !editor.ready || !editor.has_scene() {
                Err("Wait for an open scene to finish loading".into())
            } else if super::save_editor(&mut editor, &scene, &children, &meshes, &transforms) {
                Ok(state(&editor))
            } else {
                Err(editor.status.clone())
            }
        } else {
            execute(&mut editor, job.command)
        };
        if mutates && result.is_ok() {
            pointer.stop_navigation();
            if let Some(focus) = focus.as_mut() {
                focus.clear();
            }
        }
        let _ = job.reply.send(result.map(Payload::Json));
    }
}

fn object_id(editor: &Editor, target: Selection) -> String {
    let suffix = match target {
        Selection::Imported(index) => format!("imported:{}", editor.imported[index].name),
        Selection::Model(id) => format!("model:{id}"),
        Selection::Camera(index) => format!("camera:{}", editor.definition().cameras[index].id),
        Selection::Light(id) => format!("light:{id}"),
        Selection::Environment => "environment".into(),
    };
    format!("{}::{suffix}", editor.definition().file)
}

fn resolve(editor: &Editor, id: &str) -> Result<Selection, String> {
    let suffix = id
        .strip_prefix(&format!("{}::", editor.definition().file))
        .ok_or("Object ID belongs to another scene; use list_objects for the active scene")?;
    let selection = if let Some(name) = suffix.strip_prefix("imported:") {
        editor
            .imported
            .iter()
            .position(|node| node.name == name)
            .map(Selection::Imported)
    } else if let Some(id) = suffix
        .strip_prefix("model:")
        .and_then(|id| id.parse::<u32>().ok())
    {
        editor.model(id).map(|_| Selection::Model(id))
    } else if let Some(id) = suffix
        .strip_prefix("light:")
        .and_then(|id| id.parse::<u32>().ok())
    {
        editor.light(id).map(|_| Selection::Light(id))
    } else if let Some(id) = suffix.strip_prefix("camera:") {
        editor
            .definition()
            .cameras
            .iter()
            .position(|camera| camera.id == id)
            .map(Selection::Camera)
    } else {
        (suffix == "environment").then_some(Selection::Environment)
    };
    selection.ok_or_else(|| format!("Unknown object ID: {id}"))
}

fn transform_json(pose: crate::Pose) -> Value {
    json!({ "position": pose.position, "rotation_degrees": super::euler_degrees(pose.transform().rotation).to_array(),
        "rotation_quaternion": pose.rotation, "scale": pose.scale })
}

fn object(editor: &Editor, target: Selection) -> Value {
    let kind = match target {
        Selection::Imported(_) => "imported",
        Selection::Model(_) => "model",
        Selection::Camera(_) => "camera",
        Selection::Light(_) => "light",
        Selection::Environment => "environment",
    };
    let mut value = json!({ "id": object_id(editor, target), "name": editor.label(target), "kind": kind,
        "visible": !editor.object_hidden(target), "selected": editor.selection == Some(target),
        "transform": editor.pose(target).map(transform_json),
        "editable": { "position": (0..3).map(|axis| editor.field_allowed(target, Field::Position(axis))).collect::<Vec<_>>(),
            "rotation": (0..3).map(|axis| editor.field_allowed(target, Field::Rotation(axis))).collect::<Vec<_>>(),
            "scale": (0..3).map(|axis| editor.field_allowed(target, Field::Scale(axis))).collect::<Vec<_>>() } });
    match target {
        Selection::Light(id) => value["light"] = json!(editor.light(id)),
        Selection::Camera(shot) => {
            value["lens"] = json!(editor.camera(shot).lens);
        }
        Selection::Model(id) => value["asset"] = json!(editor.model(id).map(|model| &model.asset)),
        Selection::Environment => value["lighting"] = json!(editor.environment()),
        _ => {}
    }
    value
}

fn state(editor: &Editor) -> Value {
    json!({ "project": editor.project.title, "asset_root": editor.root, "ready": editor.ready,
        "active_scene": editor.has_scene().then(|| editor.definition().file.clone()),
        "dirty": editor.dirty(), "status": editor.status,
        "selection": editor.selection.map(|target| object(editor, target)),
        "undo_count": editor.undo.len(), "redo_count": editor.redo.len(),
        "confirmation_pending": editor.pending.is_some(), "view": transform_json(editor.view.into()) })
}

fn page(items: impl IntoIterator<Item = Value>, page: Page) -> Result<Value, String> {
    let limit = page.limit.unwrap_or(100);
    if !(1..=250).contains(&limit) {
        return Err("limit must be between 1 and 250".into());
    }
    let items: Vec<_> = items.into_iter().collect();
    let total = items.len();
    let next = page.offset.saturating_add(limit);
    Ok(
        json!({ "total": total, "offset": page.offset, "next_offset": (next < total).then_some(next),
        "items": items.into_iter().skip(page.offset).take(limit).collect::<Vec<_>>() }),
    )
}

fn patch_pose(
    editor: &Editor,
    target: Option<Selection>,
    original: crate::Pose,
    patch: &TransformPatch,
) -> Result<crate::Pose, String> {
    let mut pose = original.transform();
    for (values, current, field, minimum, maximum) in [
        (
            patch.position,
            pose.translation,
            Field::Position as fn(usize) -> Field,
            -1000.0,
            1000.0,
        ),
        (
            patch.scale,
            pose.scale,
            Field::Scale as fn(usize) -> Field,
            -100.0,
            100.0,
        ),
    ] {
        if let Some(values) = values {
            for axis in 0..3 {
                let value = values[axis];
                if !value.is_finite()
                    || !(minimum..=maximum).contains(&value)
                    || (matches!(field(axis), Field::Scale(_)) && value.abs() < 0.01)
                {
                    return Err("Invalid transform: position must be within ±1000 and absolute scale within 0.01–100".into());
                }
                if (value - current[axis]).abs() > 1e-5
                    && target.is_some_and(|target| !editor.field_allowed(target, field(axis)))
                {
                    return Err(format!("Transform axis {axis} is locked for this object"));
                }
            }
        }
    }
    if let Some(values) = patch.position {
        pose.translation = values.into();
    }
    if let Some(values) = patch.scale {
        pose.scale = values.into();
    }
    if let Some(values) = patch.rotation_degrees {
        let current = super::euler_degrees(pose.rotation);
        for axis in 0..3 {
            if !values[axis].is_finite() {
                return Err("Rotation must contain finite angles".into());
            }
            let delta = (values[axis] - current[axis] + 180.0).rem_euclid(360.0) - 180.0;
            if delta.abs() > 1e-4
                && target.is_some_and(|target| !editor.field_allowed(target, Field::Rotation(axis)))
            {
                return Err(format!("Rotation axis {axis} is locked for this object"));
            }
        }
        pose.rotation = Quat::from_euler(
            EulerRot::YXZ,
            values[1].rem_euclid(360.0).to_radians(),
            values[0].rem_euclid(360.0).to_radians(),
            values[2].rem_euclid(360.0).to_radians(),
        );
    }
    let pose: crate::Pose = pose.into();
    if !pose.valid() {
        return Err("Invalid transform".into());
    }
    Ok(pose)
}

fn execute(editor: &mut Editor, command: Command) -> Result<Value, String> {
    if command.mutates() && editor.pending.is_some() {
        return Err("Resolve the editor confirmation dialog first".into());
    }
    if !matches!(
        command,
        Command::State | Command::Scenes | Command::Assets(_) | Command::Open(_)
    ) && (!editor.ready || !editor.has_scene())
    {
        return Err("Wait for an open scene to finish loading".into());
    }
    match command {
        Command::State => Ok(state(editor)),
        Command::Scenes => Ok(json!({ "scenes": editor.project.scenes.iter().enumerate().map(|(index, scene)| json!({
            "id": scene.file, "name": scene.label, "asset": scene.asset, "open": editor.open_tabs.contains(&index),
            "active": editor.has_scene() && editor.room == index, "dirty": editor.document_dirty(index)
        })).collect::<Vec<_>>() })),
        Command::Objects(args) => page((0..editor.imported.len()).map(Selection::Imported)
            .chain(editor.edits.models.iter().map(|model| Selection::Model(model.id)))
            .chain((0..editor.definition().cameras.len()).map(Selection::Camera))
            .chain(editor.edits.lights.iter().map(|light| Selection::Light(light.id)))
            .chain([Selection::Environment]).map(|target| object(editor, target)), args),
        Command::Assets(args) => page(editor.project.catalog.iter().map(|item| json!({
            "asset": item.asset, "name": item.label, "category": item.category, "solid": item.solid
        })), args),
        Command::Open(args) => {
            let room = editor.project.scenes.iter().position(|scene| scene.file == args.scene_id).ok_or("Unknown scene ID")?;
            if !editor.open_document(room) { return Err(editor.status.clone()); }
            Ok(state(editor))
        }
        Command::Select(args) => {
            let target = resolve(editor, &args.object_id)?;
            editor.select(Some(target));
            Ok(object(editor, target))
        }
        Command::Transform(args) => {
            let target = resolve(editor, &args.object_id)?;
            let original = editor.pose(target).ok_or("This object has no transform")?;
            let pose = patch_pose(editor, Some(target), original, &args.transform)?;
            let before = editor.edits.clone();
            editor.set_pose(target, pose.transform());
            editor.finish_edit(before);
            Ok(object(editor, target))
        }
        Command::AddModel(args) => {
            if !editor.project.catalog.iter().any(|item| item.asset == args.asset) { return Err("Choose a project model from list_assets".into()); }
            if editor.edits.models.iter().any(|model| model.id == u32::MAX) { return Err("Model IDs are exhausted".into()); }
            let pose = patch_pose(editor, None, Transform::IDENTITY.into(), &args.transform.unwrap_or_default())?;
            let before = editor.edits.clone();
            editor.add_model(crate::PlacedModel { id: 0, asset: args.asset, pose, bounds: None });
            editor.finish_edit(before);
            Ok(object(editor, editor.selection.ok_or("Model could not be created")?))
        }
        Command::AddLight(args) => {
            let preset = serde_json::from_value::<crate::LightPreset>(json!(args.preset.unwrap_or_else(|| "bulb".into())))
                .map_err(|_| "Unknown light preset; use bulb, candle, softbox, neon, spot, sun or moon")?;
            if editor.edits.lights.iter().any(|light| light.id == u32::MAX) { return Err("Light IDs are exhausted".into()); }
            if let Some(position) = args.position {
                patch_pose(editor, None, Transform::IDENTITY.into(), &TransformPatch { position: Some(position), ..default() })?;
            }
            let before = editor.edits.clone();
            super::lighting::handle(editor, super::lighting::LightingAction::Add(preset));
            let target = editor.selection.ok_or("Light could not be created")?;
            if let Some(position) = args.position {
                let mut pose = editor.pose(target).ok_or("Light has no transform")?.transform();
                pose.translation = position.into(); editor.set_pose(target, pose);
            }
            editor.finish_edit(before);
            Ok(object(editor, target))
        }
        Command::Light(args) => {
            let Selection::Light(id) = resolve(editor, &args.object_id)? else { return Err("Object is not a light".into()); };
            let index = editor.edits.lights.iter().position(|light| light.id == id).ok_or("Unknown light")?;
            let mut light = editor.edits.lights[index].clone();
            if let Some(name) = args.name { light.name = name; }
            if let Some(enabled) = args.enabled { light.enabled = enabled; }
            if let Some(color) = args.color { light.color = color; }
            if let Some(intensity) = args.intensity { light.intensity = intensity; }
            if let Some(range) = args.range { light.range = range; }
            if let Some(shadows) = args.shadows { light.shadows = shadows; }
            if let Some(kelvin) = args.temperature_kelvin { light.temperature = Some(kelvin); }
            if let Some(enabled) = args.use_temperature { light.temperature = enabled.then_some(light.temperature.unwrap_or(6500.0)); }
            light.validate().map_err(|error| error.to_string())?;
            let before = editor.edits.clone();
            editor.edits.lights[index] = light;
            editor.initialize_light_environment(id);
            editor.finish_edit(before);
            Ok(object(editor, Selection::Light(id)))
        }
        Command::Remove(args) => {
            let target = resolve(editor, &args.object_id)?;
            if !matches!(target, Selection::Model(_) | Selection::Light(_)) { return Err("Only added models and lights can be removed; hide imported objects instead".into()); }
            let before = editor.edits.clone();
            match target {
                Selection::Model(id) => editor.edits.models.retain(|model| model.id != id),
                Selection::Light(id) => editor.edits.lights.retain(|light| light.id != id),
                _ => unreachable!(),
            }
            if editor.selection == Some(target) { editor.select(None); }
            editor.finish_edit(before);
            Ok(state(editor))
        }
        Command::Visibility(args) => {
            let target = resolve(editor, &args.object_id)?;
            if target == Selection::Environment { return Err("The scene environment cannot be hidden".into()); }
            if editor.object_hidden(target) == args.visible { editor.toggle_visibility(target); }
            if editor.object_hidden(target) == args.visible { return Err(editor.status.clone()); }
            Ok(object(editor, target))
        }
        Command::View(args) => {
            let position = Vec3::from(args.position); let target = Vec3::from(args.target);
            if !position.is_finite() || !target.is_finite() || position.abs().max_element() > 1000.0
                || target.abs().max_element() > 1000.0 || position.distance_squared(target) < 1e-6 {
                return Err("View position and target must be distinct finite points within ±1000".into());
            }
            let direction = (target - position).normalize();
            let up = if direction.dot(Vec3::Y).abs() > 0.999 { Vec3::Z } else { Vec3::Y };
            editor.pilot = false; editor.focus = false; editor.pivot = target;
            editor.view = Transform::from_translation(position).looking_at(target, up);
            Ok(state(editor))
        }
        Command::Undo | Command::Redo => {
            let redo = matches!(command, Command::Redo);
            let history = if redo { &mut editor.redo } else { &mut editor.undo };
            let edits = history.pop().ok_or(if redo { "Nothing to redo" } else { "Nothing to undo" })?;
            let previous = std::mem::replace(&mut editor.edits, edits);
            if redo { editor.undo.push(previous); } else { editor.redo.push(previous); }
            if editor.selection.is_some_and(|target| editor.pose(target).is_none() && target != Selection::Environment) { editor.select(None); }
            Ok(state(editor))
        }
        Command::Save | Command::Capture => Err("This operation requires the live renderer".into()),
    }
}
