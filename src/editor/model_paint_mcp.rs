//! MCP access to the material slots, layer edits and history of the paint workspace.
use super::*;
use crate::editor::model_document::Document;
pub(in crate::editor) use bake::{BakeArgs, BakeTarget, McpBake};
pub(in crate::editor) use engine::McpPaintInput;
pub(in crate::editor) use export::{ExportArgs, McpOutput, OutputReply};
pub(in crate::editor) use mcp_preview::{
    CageArgs, CaptureArgs, CaptureReply, McpPreview, ViewArgs as PaintViewArgs, cage, capture,
    set_view,
};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const CHANNEL_KEYS: [&str; 8] = [
    "base_color",
    "roughness",
    "metallic",
    "height",
    "opacity",
    "emission",
    "normal",
    "specular",
];

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct EditArgs {
    /// model_id returned by get_paint_state.
    model_id: usize,
    /// Expected active texture set from get_paint_state.
    set: usize,
    /// Optional workspace revision from get_paint_state to reject stale layer indices.
    revision: Option<u64>,
    edit: Edit,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct SaveArgs {
    model_id: usize,
    /// Save a new GLB in the source model's directory instead of updating the open GLB.
    copy_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    SelectSet {
        index: usize,
    },
    SelectLayer {
        index: usize,
        #[serde(default)]
        mask: bool,
    },
    AddLayer {
        kind: LayerKind,
    },
    AddMaterial {
        preset: String,
    },
    UpdateLayer {
        index: usize,
        name: Option<String>,
        visible: Option<bool>,
        opacity_percent: Option<f32>,
        blend: Option<Blend>,
    },
    DuplicateLayer {
        index: usize,
    },
    RemoveLayer {
        index: usize,
    },
    MoveLayer {
        from: usize,
        to: usize,
        placement: Placement,
    },
    SetMask {
        index: usize,
        mask: MaskBase,
    },
    ApplyGenerator {
        index: usize,
        generator: GeneratorKind,
    },
    AddMaskFilter {
        index: usize,
        filter: FilterKind,
    },
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
enum Placement {
    Above,
    Below,
    Inside,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
enum MaskBase {
    White,
    Black,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct StrokeArgs {
    workspace: PaintWorkspace,
    target: RasterTarget,
    /// Pointer samples in order. Coordinates and pressure are in 0–1.
    samples: Vec<StrokeSample>,
    #[serde(default)]
    brush: BrushValues,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct FillArgs {
    workspace: PaintWorkspace,
    target: RasterTarget,
    /// Normalized position used to pick the starting face.
    position: [f32; 2],
    mode: Option<FillMode>,
    #[serde(default)]
    brush: BrushValues,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
struct PaintWorkspace {
    model_id: usize,
    set: usize,
    revision: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct MeshArgs {
    workspace: PaintWorkspace,
    replacement: MeshOperation,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum MeshOperation {
    Primitive { shape: paint_mesh_replacement::Shape },
    File {
        /// Absolute OBJ or FBX path, or a path relative to the project asset root.
        path: std::path::PathBuf,
    },
    Undo,
    Redo,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(in crate::editor) struct SurfaceArgs {
    workspace: PaintWorkspace,
    layer: usize,
    source: SurfaceSeed,
    #[serde(default)]
    mode: surface_selection::MaskMode,
    #[serde(default)]
    stop_at: surface_selection::StopAt,
    crease_angle: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SurfaceSeed {
    Parts { indices: Vec<usize> },
    Point { position: [f32; 3] },
    Viewport { position: [f32; 2] },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct RasterTarget {
    pub layer: usize,
    #[serde(default)]
    pub mask: bool,
    pub space: PaintSpace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(rename_all = "snake_case")]
pub(super) enum PaintSpace {
    Viewport,
    Uv,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct StrokeSample {
    pub position: [f32; 2],
    pub pressure: Option<f32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub(super) struct BrushValues {
    pub size_px: Option<f32>,
    pub flow_percent: Option<f32>,
    pub hardness_percent: Option<f32>,
    pub opacity_percent: Option<f32>,
    pub spacing_percent: Option<f32>,
    pub erase: bool,
    pub mask_value: Option<u8>,
    pub channels: Option<PaintColors>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(default, deny_unknown_fields)]
pub(super) struct PaintColors {
    base_color: Option<[u8; 3]>,
    roughness: Option<u8>,
    metallic: Option<u8>,
    height: Option<u8>,
    opacity: Option<u8>,
    emission: Option<[u8; 3]>,
    normal: Option<[u8; 3]>,
    specular: Option<u8>,
}

impl PaintColors {
    pub fn colors(&self) -> [Option<[u8; 3]>; 8] {
        [
            self.base_color,
            self.roughness.map(|v| [v; 3]),
            self.metallic.map(|v| [v; 3]),
            self.height.map(|v| [v; 3]),
            self.opacity.map(|v| [v; 3]),
            self.emission,
            self.normal,
            self.specular.map(|v| [v; 3]),
        ]
    }
}

pub(super) enum RasterOperation<'a> {
    Stroke(&'a [StrokeSample]),
    Fill([f32; 2], Option<FillMode>),
}

pub(super) fn model_index(editor: &Editor) -> Result<usize, String> {
    editor
        .viewing_model
        .filter(|_| editor.viewing_image.is_none())
        .ok_or_else(|| "Open a 3D model in the editor first".to_string())
}

fn bake_model_index(editor: &Editor, target: BakeTarget) -> Result<usize, String> {
    let index = model_index(editor)?;
    if target.model_id != index {
        return Err("The active model changed; call get_paint_state again".into());
    }
    Ok(index)
}

pub(in crate::editor) fn bake_status(
    input: &McpBake,
    editor: &Editor,
    editing: &editing::State,
    paint: &PaintState,
    target: BakeTarget,
) -> Result<Value, String> {
    let index = bake_model_index(editor, target)?;
    let doc = editing
        .documents
        .get(&index)
        .ok_or("Wait for the model to load")?;
    let set = target.set.unwrap_or_else(|| {
        paint
            .sessions
            .get(&index)
            .filter(|_| doc.mode == Mode::Paint)
            .map_or_else(|| doc.paint_set_index(), |session| session.set)
    });
    input.status(index, doc, set)
}

pub(in crate::editor) fn start_bake(
    input: &mut McpBake,
    editor: &mut Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: BakeArgs,
) -> Result<Value, String> {
    let index = bake_model_index(editor, args.target)?;
    let doc = editing
        .documents
        .get_mut(&index)
        .ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = paint
        .sessions
        .get_mut(&index)
        .ok_or("Wait for Painting mode to load")?;
    input.start(editor, doc, session, args)
}

pub(in crate::editor) fn cancel_bake(
    input: &mut McpBake,
    editor: &mut Editor,
    editing: &mut editing::State,
    paint: &PaintState,
    target: BakeTarget,
) -> Result<Value, String> {
    let index = bake_model_index(editor, target)?;
    let doc = editing
        .documents
        .get_mut(&index)
        .ok_or("Wait for the model to load")?;
    let set = target.set.unwrap_or_else(|| {
        paint
            .sessions
            .get(&index)
            .map_or_else(|| doc.paint_set_index(), |session| session.set)
    });
    input.cancel(editor, doc, index, set)
}

fn output_document<'a>(
    output: &McpOutput,
    editor: &Editor,
    editing: &'a editing::State,
    paint: &'a PaintState,
    requested_model: usize,
) -> Result<(&'a Document, &'a Session), String> {
    let index = model_index(editor)?;
    if index != requested_model {
        return Err("The active model changed; call get_paint_state again".into());
    }
    let doc = editing
        .documents
        .get(&index)
        .ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = paint
        .sessions
        .get(&index)
        .ok_or("Wait for Painting mode to load")?;
    if doc.paint_busy
        || doc.paint_pending
        || session.busy
        || session.smart_loading
        || output.stroke_active()
    {
        return Err("Wait for the current paint operation to finish before writing files".into());
    }
    Ok((doc, session))
}

pub(in crate::editor) fn save_model(
    output: &mut McpOutput,
    editor: &mut Editor,
    editing: &mut editing::State,
    paint: &PaintState,
    args: SaveArgs,
    reply: OutputReply,
) {
    let (doc, _) = match output_document(output, editor, editing, paint, args.model_id) {
        Ok(document) => document,
        Err(error) => return reply(Err(error)),
    };
    if let Some(name) = args.copy_name.as_deref() {
        let task = save_copy::prepare_copy(doc, &editor.project.asset_root, name)
            .map(|write| move || write().map(|path| json!({"path": path, "copy": true})));
        output.write(task, reply);
    } else {
        editing.start_save_with_reply(
            args.model_id,
            editor,
            Some(Box::new(move |result| {
                reply(result.map(|path| json!({"path": path, "copy": false})))
            })),
        );
    }
}

pub(in crate::editor) fn export_textures(
    output: &mut McpOutput,
    editor: &Editor,
    editing: &editing::State,
    paint: &PaintState,
    args: ExportArgs,
    reply: OutputReply,
) {
    let job = output_document(output, editor, editing, paint, args.model_id).and_then(
        |(doc, session)| {
            let binding = doc
                .paint_target(session.set)
                .map(|(object, slot)| (object.id, slot))
                .filter(|key| Some(*key) == session.binding);
            output.prepare_export(editor, doc, binding, args)
        },
    );
    output.write(job, reply);
}

pub(in crate::editor) fn state(
    editor: &Editor,
    editing: &editing::State,
    paint: &PaintState,
) -> Result<Value, String> {
    let index = model_index(editor)?;
    let doc = editing
        .documents
        .get(&index)
        .ok_or_else(|| editing.errors.get(&index).cloned()
            .unwrap_or_else(|| "Wait for the model to load".into()))?;
    Ok(snapshot(
        index,
        doc,
        paint.sessions.get(&index),
        &paint.tips,
    ))
}

pub(in crate::editor) fn apply(
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: EditArgs,
    now: f64,
) -> Result<Value, String> {
    let workspace = PaintWorkspace {
        model_id: args.model_id,
        set: args.set,
        revision: args.revision,
    };
    let (index, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, workspace)?;
    perform(doc, session, args.edit, now)?;
    Ok(snapshot(index, doc, Some(session), &paint.tips))
}

fn bound_workspace<'a>(
    editor: &Editor,
    editing: &'a mut editing::State,
    sessions: &'a mut HashMap<usize, Session>,
    workspace: PaintWorkspace,
) -> Result<(usize, &'a mut Document, &'a mut Session), String> {
    let index = model_index(editor)?;
    if index != workspace.model_id {
        return Err("The active model changed; call get_paint_state again".into());
    }
    let doc = editing
        .documents
        .get_mut(&index)
        .ok_or("Wait for the model to load")?;
    if doc.mode != Mode::Paint {
        return Err("Switch the model to Painting mode first".into());
    }
    let session = sessions
        .get_mut(&index)
        .ok_or("Wait for Painting mode to load")?;
    if session.set != workspace.set
        || workspace
            .revision
            .is_some_and(|revision| revision != session.revision)
    {
        return Err("The paint workspace changed; call get_paint_state again".into());
    }
    if doc.paint_busy
        || session.busy
        || session.mesh_loading
        || session.resize_request.is_some()
        || !doc
            .paint_target(workspace.set)
            .is_some_and(|(object, slot)| session.binding == Some((object.id, slot)))
    {
        return Err("Wait for the active texture set or paint operation to finish".into());
    }
    Ok((index, doc, session))
}

pub(in crate::editor) fn replace_mesh(
    requests: &mut mesh_tools::Requests,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: MeshArgs,
    reply: Box<dyn mesh_tools::Reply>,
) {
    use mesh_tools::Operation;

    if requests.0.is_some() {
        reply.finish(Err("Wait for the current mesh replacement to finish".into()));
        return;
    }
    let (index, _, session) = match bound_workspace(editor, editing, &mut paint.sessions, args.workspace) {
        Ok(workspace) => workspace,
        Err(error) => {
            reply.finish(Err(error));
            return;
        }
    };
    let operation = match args.replacement {
        MeshOperation::Primitive { shape } => Operation::Primitive(shape),
        MeshOperation::File { path } => Operation::Import(Some(editor.project.asset_root.join(path))),
        MeshOperation::Undo => Operation::Undo,
        MeshOperation::Redo => Operation::Redo,
    };
    requests.0 = Some(mesh_tools::Request {
        model: index,
        set: session.set,
        revision: session.revision,
        operation,
        reply,
    });
}

pub(in crate::editor) fn stroke(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: StrokeArgs,
) -> Result<Value, String> {
    raster(
        input,
        editor,
        editing,
        paint,
        args.workspace,
        args.target,
        args.brush,
        RasterOperation::Stroke(&args.samples),
    )
}

pub(in crate::editor) fn fill(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: FillArgs,
) -> Result<Value, String> {
    raster(
        input,
        editor,
        editing,
        paint,
        args.workspace,
        args.target,
        args.brush,
        RasterOperation::Fill(args.position, args.mode),
    )
}

pub(in crate::editor) fn select_surface(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    args: SurfaceArgs,
) -> Result<Value, String> {
    if editor.browser_popup || editor.hdri_library_open {
        return Err("Close the open editor overlay before selecting a surface".into());
    }
    let crease_angle = args.crease_angle.unwrap_or(30.);
    if !(1. ..=179.).contains(&crease_angle) {
        return Err("The crease angle must be between 1 and 179 degrees".into());
    }
    let (index, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, args.workspace)?;
    let source = input.surface_source(session, args.source)?;
    input.submit_surface(index, doc, session, args.layer, surface_selection::Selection {
        source,
        crease_angle,
        stop_at: args.stop_at,
        mode: args.mode,
    })?;
    let mut result = snapshot(index, doc, Some(session), &paint.tips);
    result["surface_selection"] = json!({"status": "running"});
    Ok(result)
}

fn raster(
    input: &mut McpPaintInput,
    editor: &Editor,
    editing: &mut editing::State,
    paint: &mut PaintState,
    workspace: PaintWorkspace,
    target: RasterTarget,
    brush: BrushValues,
    operation: RasterOperation,
) -> Result<Value, String> {
    if editor.browser_popup || editor.hdri_library_open {
        return Err("Close the open editor overlay before painting".into());
    }
    let (index, doc, session) = bound_workspace(editor, editing, &mut paint.sessions, workspace)?;
    input.submit(index, doc, session, &paint.tips, target, brush, operation)?;
    Ok(snapshot(index, doc, Some(session), &paint.tips))
}

fn perform(doc: &mut Document, session: &mut Session, edit: Edit, now: f64) -> Result<(), String> {
    let before = session.stack.clone();
    match edit {
        Edit::SelectSet { index } => {
            if !session.choose_set(doc, index, now) {
                return Err("Unknown texture set index".into());
            }
        }
        Edit::SelectLayer { index, mask } => {
            check_layer(session, index)?;
            let action = if mask {
                PaintAction::SelectLayerMask(index)
            } else {
                PaintAction::Select(index)
            };
            apply_action(session, action)?;
        }
        Edit::AddLayer { kind } => apply_action(session, PaintAction::Add(kind))?,
        Edit::AddMaterial { preset } => {
            let index = MATERIALS
                .iter()
                .position(|material| material.name.eq_ignore_ascii_case(&preset))
                .ok_or("Unknown material preset; use list_paint_presets")?;
            apply_action(session, PaintAction::AddMaterial(index))?;
        }
        Edit::UpdateLayer {
            index,
            name,
            visible,
            opacity_percent,
            blend,
        } => {
            check_layer(session, index)?;
            let name = name.as_deref().map(Session::valid_layer_name).transpose()?;
            if opacity_percent.is_some_and(|value| !(0. ..=100.).contains(&value)) {
                return Err("Layer opacity must be between 0 and 100 percent".into());
            }
            let layer = &session.stack.layers[index];
            if blend.is_some() && layer.kind == LayerKind::Folder {
                return Err("Folders do not have a layer blend mode".into());
            }
            let mut updated = layer.clone();
            if let Some(name) = name {
                updated.name = name.to_owned();
            }
            if let Some(value) = visible {
                updated.visible = value;
            }
            if let Some(value) = opacity_percent {
                updated.opacity = value;
            }
            if let Some(value) = blend {
                updated.blend = value;
            }
            if updated != *layer {
                session.checkpoint();
                session.stack.layers[index] = updated;
                session.revision += 1;
            }
        }
        Edit::DuplicateLayer { index } => apply_to_layer(session, index, PaintAction::Duplicate)?,
        Edit::RemoveLayer { index } => apply_to_layer(session, index, PaintAction::Remove)?,
        Edit::MoveLayer {
            from,
            to,
            placement,
        } => {
            let target = match placement {
                Placement::Above => ShelfPlacement::Above(to),
                Placement::Below => ShelfPlacement::Below(to),
                Placement::Inside => ShelfPlacement::Inside(to),
            };
            apply_action(session, PaintAction::MoveLayer(from, target))?;
        }
        Edit::SetMask { index, mask } => {
            let action = match mask {
                MaskBase::White => PaintAction::AddMask(true),
                MaskBase::Black => PaintAction::AddMask(false),
                MaskBase::Remove => PaintAction::RemoveMask,
            };
            apply_to_layer(session, index, action)?;
        }
        Edit::ApplyGenerator { index, generator } => {
            check_layer(session, index)?;
            let custom = doc.custom_bakes.first().map(|recipe| recipe.id);
            if generator == GeneratorKind::CustomMap && custom.is_none() {
                return Err("Create a custom bake map before using that generator".into());
            }
            session.apply_generator(generator, Some(index), custom);
            session.stack.reveal_selection();
        }
        Edit::AddMaskFilter { index, filter } => {
            apply_to_layer(session, index, PaintAction::AddFilter(filter))?
        }
        Edit::Undo => apply_action(session, PaintAction::Undo)?,
        Edit::Redo => apply_action(session, PaintAction::Redo)?,
    }
    if !crate::editor::model_paint_data::same_layer_list(&before, &session.stack) {
        doc.paint_pending = true;
    }
    Ok(())
}

fn check_layer(session: &Session, index: usize) -> Result<(), String> {
    if index >= session.stack.layers.len() {
        return Err("Unknown layer index".into());
    }
    Ok(())
}

fn apply_action(session: &mut Session, action: PaintAction) -> Result<(), String> {
    if !session.available(action) {
        return Err("That operation is unavailable for the current paint stack".into());
    }
    session.edit(action);
    Ok(())
}

fn apply_to_layer(session: &mut Session, layer: usize, action: PaintAction) -> Result<(), String> {
    check_layer(session, layer)?;
    let previous = std::mem::replace(&mut session.stack.selected, layer);
    let available = session.available(action);
    session.stack.selected = previous;
    if !available {
        return Err("That operation is unavailable for the requested layer".into());
    }
    if previous != layer {
        session.select_layer(layer);
    }
    session.edit(action);
    Ok(())
}

fn snapshot(
    index: usize,
    doc: &Document,
    session: Option<&Session>,
    tips: &tips::Library,
) -> Value {
    let session = session.filter(|_| doc.mode == Mode::Paint);
    let active = session.map_or_else(|| doc.paint_set_index(), |session| session.set);
    let target = doc.paint_target(active);
    let bound = session.filter(|session| {
        session.binding.is_some()
            && session.binding == target.map(|(object, slot)| (object.id, slot))
    });
    let texture_sets = doc.objects.iter()
        .flat_map(|object| object.materials.iter().enumerate().map(move |(slot, material)| (object, slot, material)))
        .enumerate()
        .map(|(index, (object, slot, material))| {
            let key = (object.id, slot);
            let shown = session.is_none_or(|s| s.set_appearance.visibility.shown(key.0, key.1));
            json!({
                "index": index,
                "object_id": object.id,
                "object": object.name,
                "material_slot": slot,
                "material": material.name(&doc.source.json).unwrap_or("Материал"),
                "udim": material.udim().map(|tile| tile.number()),
                "resolution": bound.filter(|_| index == active).map_or(doc.paint_size(material), |s| s.texture_size),
                "visible": !object.hidden && shown,
                "highlighted": session.is_some_and(|s| s.set_appearance.pinned.contains(&key)),
            })
        })
        .collect::<Vec<_>>();
    let saved_stack = target
        .and_then(|(object, slot)| object.materials[slot].paint.as_deref())
        .map(|surface| &surface.stack);
    let stack = bound.map(|s| &s.stack).or(saved_stack);
    let triangles: usize = doc
        .objects
        .iter()
        .flat_map(|o| o.geometry.faces.iter())
        .map(|face| face.corners.len().saturating_sub(2))
        .sum();
    json!({
        "model_id": index,
        "revision": session.map(|s| s.revision),
        "mesh": {
            "name": doc.source.path.file_name().map(|name| name.to_string_lossy()),
            "triangles": triangles,
            "loading": session.is_some_and(|s| s.mesh_loading),
            "can_undo_replacement": doc.can_restore_paint_mesh(false),
            "can_redo_replacement": doc.can_restore_paint_mesh(true),
        },
        "mode": match doc.mode { Mode::Object => "object", Mode::Edit => "edit", Mode::Uv => "uv", Mode::Paint => "paint" },
        "dirty": doc.dirty(),
        "paint_busy": doc.paint_busy,
        "surface_selection": bound.and_then(|session| {
            let (binding, result) = session.surface_selection.as_ref()?;
            (session.binding == Some(*binding)).then(|| match result {
                Ok(report) => json!({
                    "status": "complete",
                    "triangles_selected": report.selected,
                    "triangles_total": report.total,
                    "triangles_in_active_set": report.active_set,
                    "bounds": report.bounds.map(|bounds| bounds.map(|point| point.to_array())),
                }),
                Err(error) => json!({"status": "failed", "error": error}),
            })
        }),
        "texture_sets": texture_sets,
        "active_set": target.map(|_| active),
        "resolution": texture_sets.get(active).map(|set| &set["resolution"]),
        "layers_bottom_to_top": stack.map_or_else(Vec::new, layer_rows),
        "view": session.map(|s| json!({"mode": s.view, "path_trace": s.path_trace})),
        "brush": session.map(|s| brush(s, tips)),
        "bake": {
            "busy": bound.is_some_and(|s| s.bake_busy),
            "ready": bound.is_some_and(|s| s.bake_ready),
            "quality": doc.bake_settings.quality,
            "id_source": doc.bake_settings.id_source,
            "match_names": doc.bake_settings.match_names,
            "has_high_poly": doc.bake_settings.high_poly.is_some(),
            "has_cage": doc.bake_settings.cage.is_some(),
        },
        "undo_steps": bound.map_or(0, |s| s.undo.len()),
        "redo_steps": bound.map_or(0, |s| s.redo.len()),
    })
}

fn layer_rows(stack: &Stack) -> Vec<Value> {
    stack.layers.iter().enumerate().map(|(index, layer)| {
        let channels = CHANNEL_KEYS.iter().enumerate()
            .filter_map(|(i, key)| layer.channel(i).is_some().then_some(key)).collect::<Vec<_>>();
        let mask = layer.mask.as_ref().map(|mask| json!({
            "generator": mask.generator,
            "filter_count": mask.filters.len(),
            "layer_count": mask.layers.len(),
            "has_graph": mask.graph.is_some(),
        }));
        json!({
            "index": index,
            "name": layer.name,
            "visible": layer.visible,
            "shown": stack.shown(index),
            "opacity_percent": layer.opacity,
            "blend": layer.blend,
            "kind": match layer.kind { LayerKind::Paint => "paint", LayerKind::Fill => "fill", LayerKind::Folder => "folder" },
            "folder": stack.parents(index).next(),
            "layers_inside": (layer.kind == LayerKind::Folder).then(|| stack.block(index).len() - 1),
            "collapsed": layer.collapsed,
            "channels": channels,
            "mask": mask,
            "selected": index == stack.selected,
        })
    }).collect()
}

fn brush(session: &Session, tips: &tips::Library) -> Value {
    let channels = CHANNEL_KEYS
        .iter()
        .enumerate()
        .filter_map(|(index, key)| (session.stroke_channels() & (1 << index) != 0).then_some(key))
        .collect::<Vec<_>>();
    json!({
        "tool": session.tool,
        "size_px": session.size,
        "flow_percent": session.flow,
        "opacity_percent": session.opacity,
        "hardness_percent": session.hardness,
        "spacing_percent": session.spacing,
        "stabilizer": session.stabilizer,
        "color_rgb8": session.brush_color(),
        "pressure_size": session.pressure_size,
        "pressure_flow": session.pressure_flow,
        "projection": session.projection,
        "symmetry": session.symmetry,
        "tip": session.tip.and_then(|i| tips.entries.get(i)).map(|tip| &tip.name),
        "grain": session.grain.and_then(|i| tips.grains.get(i)).map(|grain| &grain.name),
        "target": if session.editing_mask && session.has_mask() { "mask" } else { "material" },
        "channels": channels,
        "fill_mode": session.fill_mode,
    })
}

pub(in crate::editor) fn catalog(paint: &PaintState) -> Value {
    json!({
        "channels": CHANNEL_KEYS,
        "materials": MATERIALS.iter().map(|preset| preset.name).collect::<Vec<_>>(),
        "smart_materials": SMART_MATERIALS.iter().map(|preset| preset.name).collect::<Vec<_>>(),
        "local_presets": paint.library.entries().map(|(index, entry)| json!({"index": index, "name": entry.name})).collect::<Vec<_>>(),
        "generators": GeneratorKind::ALL.map(|kind| json!({"id": kind, "label": kind.label()})),
        "generator_inputs": GeneratorInput::ALL.map(|input| json!({"id": input as usize, "label": input.label()})),
        "blend_modes": Blend::ALL.map(|blend| json!({"id": blend, "label": blend.label()})),
        "brush_tips": paint.tips.names(),
        "fill_modes": FillMode::OPTIONS.map(|(label, mode)| json!({"id": mode, "label": label})),
        "bake_qualities": crate::editor::model_bake_settings::Quality::ALL,
        "bake_maps": bake::BakeMap::ALL.map(|map| json!({"id": map, "label": map.label(), "requires_high_poly": map.requires_high_poly()})),
        "export_presets": export::Template::ALL.map(|preset| json!({"id": preset, "label": preset.label()})),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        model_document::{MaterialSlot, Object, glb::Glb},
        model_mesh::Geometry,
    };

    fn fixture() -> (Document, Session) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/assets/models/block.glb");
        let source = Glb::load(&path).unwrap();
        let material = |name, tile, paint| MaterialSlot {
            material: Handle::default(),
            source: json!({"name": name, "extras": {"bevy_editor_udim": tile}}),
            authored: None,
            paint,
            bake_fixes: Default::default(),
        };
        let pixels = Arc::new(vec![0; 4]);
        let saved = Arc::new(Surface {
            size: 512,
            base: std::array::from_fn(|_| pixels.clone()),
            stack: Stack {
                layers: vec![Layer::new("Saved paint".into(), LayerKind::Paint)],
                selected: 0,
            },
            material: json!({}),
            baked: None,
        });
        let object = Object {
            id: 7,
            name: "Cube".into(),
            source_node: None,
            geometry: Geometry::default().into(),
            transform: Transform::IDENTITY,
            materials: vec![
                material("Stone", 1001, Some(saved)),
                material("Stone", 1013, None),
            ],
            hidden: false,
        };
        let mut document = Document::new(source, vec![object]);
        document.mode = Mode::Paint;
        let mut child = Layer::new("Live paint".into(), LayerKind::Paint);
        child.depth = 1;
        child.channels[0] = Some(pixels);
        let mut group = Layer::new("Group".into(), LayerKind::Folder);
        group.visible = false;
        let mut session = Session {
            set: 1,
            binding: Some((7, 1)),
            texture_size: 2048,
            stack: Stack {
                layers: vec![child, group],
                selected: 0,
            },
            size: 64.,
            projection: BrushProjection::Surface,
            ..default()
        };
        session.set_appearance.toggle_visibility((7, 0));
        session.set_appearance.toggle_pin((7, 1));
        (document, session)
    }

    #[test]
    fn snapshot_tracks_live_set_and_falls_back_to_saved_layers_until_rebound() {
        let (mut document, mut session) = fixture();
        let paint = PaintState::default();
        let live = snapshot(0, &document, Some(&session), &paint.tips);
        assert_eq!(live["active_set"], 1);
        assert_eq!(live["resolution"], 2048);
        assert_eq!(live["texture_sets"][0]["visible"], false);
        assert_eq!(live["texture_sets"][1]["udim"], 1013);
        assert_eq!(live["texture_sets"][1]["highlighted"], true);
        assert_eq!(live["layers_bottom_to_top"][0]["name"], "Live paint");
        assert_eq!(live["layers_bottom_to_top"][0]["folder"], 1);
        assert_eq!(live["layers_bottom_to_top"][0]["shown"], false);
        assert_eq!(
            live["layers_bottom_to_top"][0]["channels"],
            json!(["base_color"])
        );
        assert_eq!(live["layers_bottom_to_top"][1]["layers_inside"], 1);
        assert_eq!(live["brush"]["size_px"].as_f64(), Some(64.));
        assert_eq!(live["brush"]["projection"], "surface");

        session.set = 0;
        let loading = snapshot(0, &document, Some(&session), &paint.tips);
        assert_eq!(loading["resolution"], 512);
        assert_eq!(loading["layers_bottom_to_top"][0]["name"], "Saved paint");
        document.mode = Mode::Object;
        let object_view = snapshot(0, &document, Some(&session), &paint.tips);
        assert_eq!(object_view["mode"], "object");
        assert!(object_view["brush"].is_null());
        assert_eq!(
            object_view["layers_bottom_to_top"][0]["name"],
            "Saved paint"
        );
    }

    #[test]
    fn paint_edit_validates_workspace_and_keeps_atomic_shared_history() {
        let (document, mut session) = fixture();
        session.texture_size = 16;
        let project_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json");
        let project = crate::EditorProject::load(project_path).unwrap();
        let scene = project.read_scene(0).unwrap();
        let mut editor = Editor::new(project, scene);
        editor.viewing_model = Some(0);
        let mut editing = editing::State::default();
        editing.documents.insert(0, document);
        let mut paint = PaintState::default();
        paint.sessions.insert(0, session);
        let request = |operation| EditArgs {
            model_id: 0,
            set: 1,
            revision: None,
            edit: operation,
        };
        let edited = |name, opacity| Edit::UpdateLayer {
            index: 0,
            name: Some(name),
            visible: Some(false),
            opacity_percent: Some(opacity),
            blend: Some(Blend::Multiply),
        };
        let invalid = request(edited("Ignored".into(), 120.));
        assert!(apply(&editor, &mut editing, &mut paint, invalid, 0.).is_err());
        assert!(paint.sessions[&0].undo.is_empty());
        assert_eq!(paint.sessions[&0].stack.layers[0].name, "Live paint");

        let patch = request(edited("Polished".into(), 55.));
        let updated = apply(&editor, &mut editing, &mut paint, patch, 0.).unwrap();
        let row = &updated["layers_bottom_to_top"][0];
        assert_eq!(row["name"], "Polished");
        assert_eq!(row["blend"], "Multiply");
        assert_eq!(row["visible"], false);
        assert_eq!(row["opacity_percent"].as_f64(), Some(55.));
        assert_eq!(updated["undo_steps"], 1);
        assert_eq!(updated["dirty"], true);
        let stale = EditArgs {
            revision: Some(updated["revision"].as_u64().unwrap() - 1),
            ..request(Edit::RemoveLayer { index: 0 })
        };
        assert!(apply(&editor, &mut editing, &mut paint, stale, 0.).is_err());
        assert_eq!(paint.sessions[&0].stack.layers.len(), 2);

        apply(&editor, &mut editing, &mut paint, request(Edit::Undo), 0.).unwrap();
        assert_eq!(paint.sessions[&0].stack.layers[0].name, "Live paint");
        apply(&editor, &mut editing, &mut paint, request(Edit::Redo), 0.).unwrap();
        assert_eq!(paint.sessions[&0].stack.layers[0].name, "Polished");
        for edit in [
            Edit::SetMask {
                index: 0,
                mask: MaskBase::Black,
            },
            Edit::ApplyGenerator {
                index: 0,
                generator: GeneratorKind::Clouds,
            },
            Edit::AddMaskFilter {
                index: 0,
                filter: FilterKind::Levels,
            },
            Edit::MoveLayer {
                from: 0,
                to: 1,
                placement: Placement::Above,
            },
        ] {
            apply(&editor, &mut editing, &mut paint, request(edit), 0.).unwrap();
        }
        let session = &paint.sessions[&0];
        assert!(session.stack.valid_hierarchy());
        assert_eq!(session.stack.layers[1].depth, 0);
        let mask = session.stack.layers[1].mask.as_ref().unwrap();
        assert_eq!(mask.generator.unwrap().kind, GeneratorKind::Clouds);
        assert_eq!(mask.filters.len(), 1);
        let changed = apply(
            &editor,
            &mut editing,
            &mut paint,
            request(Edit::SelectSet { index: 0 }),
            0.,
        )
        .unwrap();
        assert_eq!(changed["active_set"], 0);
        assert_eq!(editing.documents[&0].paint_set, Some(0));
        assert_eq!(changed["layers_bottom_to_top"][0]["name"], "Saved paint");
        assert!(apply(&editor, &mut editing, &mut paint, request(Edit::Undo), 0.).is_err());
    }
}
