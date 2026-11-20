//! Model documents, kept alive across tabs and isolated from scene instances.
use super::*;
use crate::editor::{
    model_document::{self as document, Document, MaterialSlot, Mode, Object, Operation, Pivot, Snapshot},
    model_mesh::{Edge, Element, Geometry, Selection as MeshSelection},
};
use bevy::{
    gltf::{GltfMaterial, GltfMesh},
    mesh::{Indices, PrimitiveTopology},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

#[path = "model_edit_snap.rs"]
mod snapping;
use snapping::SnapTarget;
use input::{BevelControl, LoopCutControl};

#[path = "model_area.rs"]
pub(super) mod area;
#[path = "model_edit_controls.rs"]
pub(in crate::editor) mod controls;
#[path = "model_edit_handles.rs"]
pub(super) mod handles;
#[path = "model_edit_input.rs"]
pub(super) mod input;
#[path = "model_selection.rs"]
pub(super) mod selection;
#[path = "model_uv.rs"]
pub(in crate::editor) mod uv;
#[path = "model_wireframe.rs"]
pub(super) mod wireframe;
#[path = "model_xray.rs"]
pub(super) mod xray;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum EditAction {
    Menu,
    Mode(Mode),
    ToggleMode,
    Element(Element),
    Operate(Operation),
    ExtrudeTool,
    BeginExtrude,
    BeginBevel { vertices: bool },
    BeginLoopCut,
    AdjustLoopCut,
    LoopCut(LoopCutControl),
    AdjustBevel,
    Bevel(BevelControl),
    SelectAll,
    Deselect,
    Invert,
    Linked,
    SelectEdgeLoop,
    CreateMaterial,
    AssignMaterial { object: usize, slot: usize },
    Grow,
    Shrink,
    SelectObject(usize),
    Tool(Tool),
    Begin(Tool),
    Cancel,
    Save,
    Undo,
    Redo,
    Xray,
    Wireframe,
    Local,
    Snap,
    SnapTarget(SnapTarget),
    Pivot(Pivot),
    Proportional,
    Connected,
    AddMenu,
    Add(usize),
    Hide,
    Reveal,
    Focus,
    BoxSelect,
    Bisect,
    BeginKnife { only_selected: bool },
}

/// Shared header controls act on the currently opened model document.
pub(in crate::editor) fn toolbar_action(action: Action) -> Option<EditAction> {
    match action {
        Action::Tool(tool) => Some(EditAction::Tool(tool)),
        Action::LocalSpace => Some(EditAction::Local),
        Action::Snap => Some(EditAction::Snap),
        _ => None,
    }
}

struct Saving {
    index: usize,
    objects: Vec<Object>,
    task: Task<Result<document::glb::Glb, String>>,
}

#[derive(Clone)]
struct RenderedObject {
    entity: Entity,
    geometry: document::MeshData,
    materials: Vec<MaterialSlot>,
    edges: Arc<BTreeMap<Edge, Vec<usize>>>,
}

#[derive(Resource)]
pub(in crate::editor) struct State {
    pub documents: HashMap<usize, Document>,
    pub(super) errors: HashMap<usize, String>,
    rendered: Option<(Entity, u64)>,
    render_objects: HashMap<usize, RenderedObject>,
    default_material: Handle<StandardMaterial>,
    pub gesture: Option<input::Gesture>,
    last_bevel: HashMap<usize, input::LastBevel>,
    last_loop_cut: HashMap<usize, input::LastLoopCut>,
    pub hover: Option<crate::editor::viewport::TransformHandle>,
    bvh: input::Bvh,
    pub(super) bvh_stamp: Option<(usize, u64)>,
    pub(super) last_model: Option<usize>,
    pub tool: Tool,
    pub(super) extrude_tool: bool,
    pub xray: bool,
    pub wireframe: bool,
    pub local: bool,
    pub snap: bool,
    snap_target: SnapTarget,
    pub proportional: bool,
    pub connected: bool,
    pub influence: f32,
    pub menu: bool,
    pub add_menu: bool,
    pub box_start: Option<Vec2>,
    pub box_auto: bool,
    pub box_armed: bool,
    pub bisect_start: Option<Vec2>,
    pub bisect: bool,
    saving: Option<Saving>,
    pub status: String,
    drafts: BTreeSet<usize>,
    input_error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            documents: HashMap::new(),
            errors: HashMap::new(),
            rendered: None,
            render_objects: HashMap::new(),
            default_material: default(),
            gesture: None,
            last_bevel: HashMap::new(),
            last_loop_cut: HashMap::new(),
            hover: None,
            bvh: default(),
            bvh_stamp: None,
            last_model: None,
            tool: Tool::Universal,
            extrude_tool: false,
            xray: false,
            wireframe: false,
            local: false,
            snap: false,
            snap_target: SnapTarget::Step,
            proportional: false,
            connected: false,
            influence: 1.0,
            menu: false,
            add_menu: false,
            box_start: None,
            box_auto: false,
            box_armed: false,
            bisect_start: None,
            bisect: false,
            saving: None,
            status: String::new(),
            drafts: default(),
            input_error: None,
        }
    }
}
impl State {
    fn document_dirty(&self, index: usize, doc: &Document) -> bool {
        self.gesture.as_ref()
            .filter(|gesture| gesture.model == index)
            .and_then(input::Gesture::committed_bevel_state)
            .map_or_else(|| doc.dirty(), |state| doc.paint_pending || state.objects != doc.saved)
    }

    pub(in crate::editor) fn saving_model(&self, index: usize) -> bool {
        self.saving
            .as_ref()
            .is_some_and(|saving| saving.index == index)
    }
    fn start_save(&mut self, index: usize, editor: &mut Editor) {
        if let Some(error) = &self.input_error {
            editor.status = format!("Не удалось сохранить модель · {error}");
            return;
        }
        if self.saving.is_some() {
            editor.status = "Дождитесь сохранения модели".into();
            return;
        }
        let Some(doc) = self.documents.get_mut(&index) else {
            return;
        };
        if doc.paint_busy {
            editor.status = "Завершаем мазок перед сохранением…".into();
            return;
        }
        if let Some(model) = editor.project.catalog.get(index) {
            match document::source_path(&editor.root, &model.asset) {
                Ok(path) => doc.source.path = path,
                Err(error) => {
                    editor.status = format!("Не удалось сохранить модель · {error}");
                    return;
                }
            }
        }
        if let Err(error) = doc.validate() {
            editor.status = format!("Не удалось сохранить модель · {error}");
            return;
        }
        let mut source = doc.source.clone();
        let objects = doc.objects.clone();
        let original = doc.original.clone();
        let work = objects.clone();
        self.saving = Some(Saving {
            index,
            objects,
            task: AsyncComputeTaskPool::get().spawn(async move {
                source.save(&work, &original)?;
                Ok(source)
            }),
        });
        editor.status = "Сохранение GLB…".into();
    }
    pub fn mode(&self, index: Option<usize>) -> Mode {
        index
            .and_then(|i| self.documents.get(&i))
            .map_or(Mode::Object, |doc| doc.mode)
    }
    pub fn active(&self, action: EditAction, index: Option<usize>) -> bool {
        let doc = index.and_then(|i| self.documents.get(&i));
        match action {
            EditAction::Bevel(control) => self.gesture.as_ref()
                .is_some_and(|gesture| gesture.bevel_control_active(control)),
            EditAction::LoopCut(control) => self.gesture.as_ref()
                .is_some_and(|gesture| gesture.loop_control_active(control)),
            EditAction::Mode(mode) => self.mode(index) == mode,
            EditAction::Element(kind) => doc.is_some_and(|d| d.selection.kind == kind),
            EditAction::Tool(tool) => !self.extrude_tool && self.tool == tool,
            EditAction::ExtrudeTool => self.extrude_tool && self.mode(index) == Mode::Edit,
            EditAction::SelectObject(id) => doc.is_some_and(|d| d.selected.contains(&id)),
            EditAction::Xray => self.xray,
            EditAction::Wireframe => self.wireframe,
            EditAction::Local => self.local,
            EditAction::Snap => self.snap,
            EditAction::SnapTarget(target) => self.snap_target == target,
            EditAction::Pivot(mode) => doc.is_some_and(|d| d.pivot_mode == mode),
            EditAction::Proportional => self.proportional,
            EditAction::Connected => self.connected,
            EditAction::BoxSelect => self.box_armed,
            EditAction::Bisect => self.bisect,
            EditAction::BeginKnife { .. } => self.gesture.as_ref().is_some_and(input::Gesture::is_knifing),
            _ => false,
        }
    }
    pub fn available(&self, action: EditAction, index: Option<usize>) -> bool {
        if let Some(gesture) = self.gesture.as_ref().filter(|g| g.is_beveling()) {
            return match action {
                EditAction::Cancel => true,
                EditAction::Bevel(control) => gesture.bevel_control_available(control),
                _ => false,
            };
        }
        if let Some(gesture) = self.gesture.as_ref().filter(|g| g.is_preview_tool()) {
            return match action {
                EditAction::Cancel => true,
                EditAction::LoopCut(control) => gesture.loop_control_available(control),
                _ => false,
            };
        }
        if action == EditAction::Wireframe {
            // Topology can also be viewed for GLTFs protected from editing.
            return index.is_some();
        }
        if matches!(action, EditAction::Menu | EditAction::Mode(Mode::Object)) {
            return true;
        }
        let doc = index.and_then(|i| self.documents.get(&i));
        let Some(doc) = doc else {
            return false;
        };
        if matches!(doc.mode, Mode::Uv | Mode::Paint)
            && !matches!(
                action,
                EditAction::Menu
                    | EditAction::Mode(_)
                    | EditAction::ToggleMode
                    | EditAction::Save
                    | EditAction::Undo
                    | EditAction::Redo
                    | EditAction::SelectObject(_)
                    | EditAction::Focus
                    | EditAction::Cancel
                    | EditAction::Wireframe
            )
        {
            return false;
        }
        if self.saving.is_some() && matches!(action, EditAction::Save) {
            return false;
        }
        match action {
            EditAction::Bevel(_) | EditAction::LoopCut(_) => false,
            EditAction::BeginLoopCut => doc.mode == Mode::Edit && self.gesture.is_none()
                && doc.active().is_some_and(|object| !object.hidden),
            EditAction::AdjustLoopCut => index.and_then(|index| self.last_loop_cut.get(&index))
                .is_some_and(|last| doc.mode == Mode::Edit && last.revision == doc.revision)
                && self.gesture.is_none(),
            EditAction::BeginBevel { .. } => {
                self.gesture.is_none() && doc.mode == Mode::Edit && doc.selection_center().is_some()
            }
            EditAction::AdjustBevel => index.and_then(|index| self.last_bevel.get(&index))
                .is_some_and(|last| doc.mode == Mode::Edit && last.revision == doc.revision),
            EditAction::Undo => !doc.undo.is_empty() && self.gesture.is_none(),
            EditAction::Redo => !doc.redo.is_empty() && self.gesture.is_none(),
            EditAction::Save => doc.dirty() || doc.paint_busy,
            EditAction::Element(_) => doc.mode == Mode::Edit,
            EditAction::CreateMaterial | EditAction::AssignMaterial { .. } => {
                doc.can_assign_material() && self.gesture.is_none() && !doc.paint_busy
            }
            EditAction::Pivot(_) | EditAction::SnapTarget(_) => {
                matches!(doc.mode, Mode::Object | Mode::Edit) && self.gesture.is_none()
            }
            EditAction::SelectEdgeLoop
            | EditAction::Operate(
                Operation::SlideEdges { .. } | Operation::BevelEdges
                | Operation::MarkUvSeam | Operation::ClearUvSeam,
            ) => {
                doc.mode == Mode::Edit
                    && doc.selection.kind == Element::Edge
                    && !doc.selection.edges.is_empty()
                    && self.gesture.is_none()
            }
            EditAction::Bisect => doc.mode == Mode::Edit,
            EditAction::BeginKnife { .. } => doc.mode == Mode::Edit && self.gesture.is_none()
                && doc.active().is_some_and(|object| !object.hidden),
            EditAction::ExtrudeTool => {
                self.gesture.is_none()
                    && doc.mode == Mode::Edit
                    && (self.extrude_tool || self.available(EditAction::BeginExtrude, index))
            }
            EditAction::BeginExtrude => {
                self.available(EditAction::Operate(Operation::Extrude), index)
            }
            EditAction::Operate(op) => match op {
                Operation::Extrude => {
                    doc.mode == Mode::Edit
                        && self.gesture.is_none()
                        && doc.active().is_some_and(|object| {
                            !object.hidden && match doc.selection.kind {
                                Element::Vertex => !doc.selection.vertices.is_empty(),
                                Element::Edge => !doc.selection.edges.is_empty(),
                                Element::Face => !doc.selection.faces.is_empty(),
                            }
                        })
                }
                Operation::Join | Operation::ApplyTransform | Operation::OriginToGeometry => {
                    doc.mode == Mode::Object
                }
                Operation::DeleteFaces => doc.mode == Mode::Edit,
                _ => true,
            },
            _ => true,
        }
    }
    pub(super) fn clear_gesture(&mut self, _doc_index: Option<usize>, cancel: bool) {
        if let Some(gesture) = self.gesture.take() {
            if gesture.is_loop_cutting() || gesture.is_knifing() {
                self.status.clear();
                return;
            }
            if let Some(doc) = self.documents.get_mut(&gesture.model) {
                if cancel || gesture.is_beveling() {
                    let model = gesture.model;
                    doc.restore(gesture.cancel_snapshot());
                    if let Some(last) = self.last_bevel.get_mut(&model)
                        && last.matches(doc)
                    {
                        last.revision = doc.revision;
                    }
                    self.status.clear();
                } else if doc.validate().is_ok() {
                    doc.checkpoint(gesture.base);
                } else {
                    doc.restore(gesture.base);
                    self.status = "Преобразование отменено: вырожденная геометрия".into();
                }
            }
        }
    }
}

pub(super) fn initialize(
    mut state: ResMut<State>,
    viewer: Res<Viewer>,
    editor: Res<Editor>,
    gltfs: Res<Assets<Gltf>>,
    server: Res<AssetServer>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    gltf_materials: Res<Assets<GltfMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    prepared: Query<(), With<Prepared>>,
) {
    let Some(index) = editor.viewing_model.filter(|_| viewer.fitted) else {
        return;
    };
    let Some(_) = viewer.root.filter(|root| prepared.contains(*root)) else {
        return;
    };
    if state.documents.contains_key(&index) || state.errors.contains_key(&index) {
        return;
    }
    let Some(source) = viewer.source.as_ref() else {
        return;
    };
    let ModelSource::Glb(handle) = source else {
        state.errors.insert(
            index,
            "FBX · просмотр модели. Правка сетки и UV доступна для GLB.".into(),
        );
        return;
    };
    let Some(asset) = gltfs.get(handle) else {
        return;
    };
    // Handle::default() is Bevy's pink diagnostic material. Reuse the GLTF
    // loader's fallback so unassigned primitives match their scene instances.
    state.default_material = server.load(
        bevy::asset::AssetPath::from(editor.project.catalog[index].asset.clone()).with_label(
            format!("{}/std", bevy::gltf::GltfAssetLabel::DefaultMaterial),
        ),
    );
    let path = match document::source_path(&editor.root, &editor.project.catalog[index].asset) {
        Ok(path) => path,
        Err(error) => {
            state.errors.insert(index, error);
            return;
        }
    };
    let result = (|| {
        let source = document::glb::Glb::load(&path)?;
        let parsed =
            bevy::gltf::gltf::Gltf::from_slice(source.data()).map_err(|e| e.to_string())?;
        let blob = parsed.blob.as_deref().unwrap_or_default();
        let mut objects = Vec::new();
        let mut pool = HashMap::new();
        struct Import<'a> {
            asset: &'a Gltf,
            gltf_meshes: &'a Assets<GltfMesh>,
            gltf_materials: &'a Assets<GltfMaterial>,
            materials: &'a mut Assets<StandardMaterial>,
            pool: &'a mut HashMap<bevy::asset::AssetId<GltfMaterial>, Handle<StandardMaterial>>,
            default: Handle<StandardMaterial>,
            blob: &'a [u8],
            json: &'a serde_json::Value,
            source: &'a document::glb::Glb,
        }
        fn visit(
            node: bevy::gltf::gltf::Node,
            parent: Mat4,
            objects: &mut Vec<Object>,
            ctx: &mut Import,
            depth: usize,
        ) -> Result<(), String> {
            if depth > 256 {
                return Err("Слишком глубокая иерархия модели".into());
            }
            let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
            if let Some(mesh) = node.mesh() {
                if node.skin().is_some() || mesh.primitives().any(|p| p.morph_targets().len() > 0) {
                    return Err("Сетки со скелетом или morph targets доступны для просмотра; топология защищена от повреждения".into());
                }
                if !world.is_finite() || world.determinant().abs() < 1.0e-20 {
                    return Err("Матрица объекта содержит нулевой масштаб; примените трансформации в исходном файле".into());
                }
                let transform = Transform::from_matrix(world);
                if !world.is_finite()
                    || transform.scale.abs().min_element() < 1.0e-7
                    || transform
                        .to_matrix()
                        .to_cols_array()
                        .iter()
                        .zip(world.to_cols_array())
                        .any(|(a, b)| (*a - b).abs() > 0.001)
                {
                    return Err("Матрица объекта содержит сдвиг или нулевой масштаб; примените трансформации в исходном файле".into());
                }
                let loaded = ctx
                    .asset
                    .meshes
                    .get(mesh.index())
                    .and_then(|h| ctx.gltf_meshes.get(h))
                    .ok_or("Геометрия ещё не загружена")?;
                let mut geometry = Geometry::default();
                let mut slots = Vec::new();
                for primitive in mesh.primitives() {
                    let i = primitive.index();
                    let reader =
                        primitive.reader(|buffer| (buffer.index() == 0).then_some(ctx.blob));
                    let positions = reader
                        .read_positions()
                        .ok_or("Координаты должны находиться во внутреннем буфере GLB")?
                        .collect::<Vec<_>>();
                    let topology = match primitive.mode() {
                        bevy::gltf::gltf::mesh::Mode::Triangles => PrimitiveTopology::TriangleList,
                        bevy::gltf::gltf::mesh::Mode::TriangleStrip => {
                            PrimitiveTopology::TriangleStrip
                        }
                        bevy::gltf::gltf::mesh::Mode::Lines => PrimitiveTopology::LineList,
                        _ => return Err("Правка поддерживает треугольные поверхности и рёбра".into()),
                    };
                    let mut data = Mesh::new(topology, bevy::asset::RenderAssetUsages::MAIN_WORLD);
                    data.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                    if let Some(v) = reader.read_normals() {
                        data.insert_attribute(Mesh::ATTRIBUTE_NORMAL, v.collect::<Vec<_>>());
                    }
                    for (channel, attribute) in
                        [(0, Mesh::ATTRIBUTE_UV_0), (1, Mesh::ATTRIBUTE_UV_1)]
                    {
                        if let Some(v) = reader.read_tex_coords(channel) {
                            data.insert_attribute(attribute, v.into_f32().collect::<Vec<_>>());
                        }
                    }
                    if let Some(v) = reader.read_colors(0) {
                        data.insert_attribute(
                            Mesh::ATTRIBUTE_COLOR,
                            v.into_rgba_f32().collect::<Vec<_>>(),
                        );
                    }
                    if let Some(v) = reader.read_indices() {
                        data.insert_indices(Indices::U32(v.into_u32().collect()));
                    }
                    let topology =
                        &ctx.json["meshes"][mesh.index()]["extras"]["bevy_editor_topology"];
                    document::from_mesh_with_topology(
                        &data,
                        &mut geometry,
                        i,
                        (topology["version"].as_u64() == Some(1))
                            .then(|| &topology["primitives"][i]),
                    )?;
                    let material = if let Some(handle) =
                        loaded.primitives.get(i).and_then(|p| p.material.as_ref())
                    {
                        if let Some(cached) = ctx.pool.get(&handle.id()) {
                            cached.clone()
                        } else {
                            let value = ctx
                                .gltf_materials
                                .get(handle)
                                .ok_or("Материал ещё не загружен")?;
                            let handle_material = ctx
                                .materials
                                .add(bevy::pbr::gltf::standard_material_from_gltf_material(value));
                            ctx.pool.insert(handle.id(), handle_material.clone());
                            handle_material
                        }
                    } else {
                        ctx.default.clone()
                    };
                    slots.push(MaterialSlot {
                        authored: None,
                        paint: ctx.source.paint_surface(&ctx.json["meshes"][mesh.index()]["primitives"][i])?,
                        material,
                        source: ctx.json["meshes"][mesh.index()]["primitives"][i].clone(),
                    });
                }
                let topology = &ctx.json["meshes"][mesh.index()]["extras"]["bevy_editor_topology"];
                if topology["version"].as_u64() == Some(1) {
                    geometry.restore_uv_seams(&topology["uv_seams"])?;
                }
                geometry.validate()?;
                objects.push(Object {
                    id: node.index(),
                    source_node: Some(node.index()),
                    name: node
                        .name()
                        .or(mesh.name())
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Объект {}", node.index() + 1)),
                    geometry: geometry.into(),
                    transform,
                    materials: slots,
                    hidden: false,
                });
            }
            for child in node.children() {
                visit(child, world, objects, ctx, depth + 1)?;
            }
            Ok(())
        }
        {
            let mut ctx = Import {
                asset,
                gltf_meshes: &gltf_meshes,
                gltf_materials: &gltf_materials,
                materials: &mut materials,
                pool: &mut pool,
                default: state.default_material.clone(),
                blob,
                json: &source.json,
                source: &source,
            };
            let scene = parsed
                .default_scene()
                .or_else(|| parsed.scenes().next())
                .ok_or("В модели нет сцены")?;
            for root in scene.nodes() {
                visit(root, Mat4::IDENTITY, &mut objects, &mut ctx, 0)?;
            }
        }
        let doc = Document::new(source, objects);
        doc.validate()?;
        Ok::<_, String>(doc)
    })();
    match result {
        Ok(doc) => {
            state.documents.insert(index, doc);
            state.rendered = None;
            state.status.clear();
        }
        Err(error) => {
            if error.contains("ещё")
                && viewer
                    .started
                    .is_some_and(|start| start.elapsed().as_secs() < 30)
            {
                state.status = error;
            } else {
                state.errors.insert(index, error);
            }
        }
    }
}

#[derive(Component)]
pub(super) struct OwnedModel;

pub(super) fn render(
    mut state: ResMut<State>,
    mut viewer: ResMut<Viewer>,
    editor: Res<Editor>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    owned: Query<(), With<OwnedModel>>,
    uv: Res<uv::UvState>,
) {
    let Some(index) = editor.viewing_model else {
        state.rendered = None;
        state.render_objects.clear();
        return;
    };
    let Some(doc) = state.documents.get(&index) else {
        return;
    };
    let Some(mut root) = viewer.root else {
        return;
    };
    if state.rendered == Some((root, doc.revision)) {
        return;
    }
    let objects = doc.objects.clone();
    let revision = doc.revision;
    let uv_mode = doc.mode == Mode::Uv;
    if !owned.contains(root) {
        commands.entity(root).try_despawn();
        let Some(stage) = viewer.stage else {
            return;
        };
        root = commands
            .spawn((
                OwnedModel,
                Prepared,
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(stage),
            ))
            .id();
        viewer.root = Some(root);
        state.render_objects.clear();
    }
    let mut previous = std::mem::take(&mut state.render_objects);
    for mut object in objects {
        if uv_mode {
            object.geometry = uv.display_geometry(index, &object);
        }
        let old = previous.remove(&object.id);
        if let Some(old) = old
            .as_ref()
            .filter(|old| old.geometry == object.geometry && old.materials == object.materials)
        {
            commands.entity(old.entity).try_insert((
                Name::new(object.name),
                object.transform,
                if object.hidden {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                },
            ));
            state.render_objects.insert(object.id, old.clone());
            continue;
        }
        let edges = old
            .as_ref()
            .filter(|old| {
                old.geometry == object.geometry
                    || (old.geometry.faces == object.geometry.faces
                        && old.geometry.loose_edges == object.geometry.loose_edges)
            })
            .map_or_else(
                || Arc::new(object.geometry.edges()),
                |old| old.edges.clone(),
            );
        if let Some(old) = old {
            commands.entity(old.entity).try_despawn();
        }
        let entity = commands
            .spawn((
                Name::new(object.name),
                object.transform,
                if object.hidden {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                },
                RenderLayers::layer(LAYER),
                ChildOf(root),
            ))
            .id();
        for (slot, buffers) in object.geometry.buffers() {
            if buffers.indices.is_empty() {
                continue;
            }
            let handle = meshes.add(document::render_mesh(buffers));
            let material = object
                .materials
                .get(slot)
                .map_or_else(|| state.default_material.clone(), |m| m.material.clone());
            commands.spawn((
                Mesh3d(handle.clone()),
                MeshMaterial3d(material.clone()),
                xray::SourceMaterial(material),
                RenderLayers::layer(LAYER),
                ChildOf(entity),
            ));
        }
        if !object.geometry.loose_edges.is_empty() {
            let positions = object.geometry.line_positions();
            let mut mesh = Mesh::new(PrimitiveTopology::LineList, bevy::asset::RenderAssetUsages::default());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; positions.len()]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(state.default_material.clone()),
                RenderLayers::layer(LAYER),
                ChildOf(entity),
            ));
        }
        state.render_objects.insert(
            object.id,
            RenderedObject {
                entity,
                geometry: object.geometry,
                materials: object.materials,
                edges,
            },
        );
    }
    for (_, old) in previous {
        commands.entity(old.entity).try_despawn();
    }
    state.rendered = Some((root, revision));
}

pub(super) fn request_close(editor: &mut Editor, index: usize) {
    if editor.dirty_models.contains(&index) {
        editor.open_model_tab(index);
        editor.pending = Some(crate::editor::PendingAction::CloseModelTab(index));
        editor.status = "Есть несохранённые изменения модели".into();
    } else {
        editor.close_model_tab(index);
    }
}

pub(super) fn process_actions(
    actions: &[Action],
    editor: &mut Editor,
    viewer: &mut Viewer,
    state: &mut State,
    server: &AssetServer,
    previews: &mut crate::editor::thumbnails::Thumbnails,
    materials: &mut Assets<StandardMaterial>,
    shift: bool,
) {
    if let Some((index, objects, result)) = state.saving.as_mut().and_then(|job| {
        block_on(poll_once(&mut job.task)).map(|result| (job.index, job.objects.clone(), result))
    }) {
        state.saving = None;
        match result {
            Ok(source) => {
                if let Some(doc) = state.documents.get_mut(&index) {
                    doc.source = source;
                    doc.saved = objects;
                    doc.ui_revision += 1;
                }
                editor.status = "Модель сохранена в GLB".into();
                // Scene instances retain Scene0 after the viewer releases its
                // GLTF root, including when Save also closes the model tab.
                server.reload(format!("{}#Scene0", editor.project.catalog[index].asset));
                previews.invalidate_asset(&editor.project.catalog[index].asset);
                if let Some(
                    crate::editor::PendingAction::CloseModelTab(id)
                    | crate::editor::PendingAction::CloseModelEditor(id),
                ) = editor.pending
                {
                    if id == index {
                        let exiting = matches!(
                            editor.pending,
                            Some(crate::editor::PendingAction::CloseModelEditor(_))
                        );
                        editor.pending = None;
                        editor.close_model_tab(index);
                        state.documents.remove(&index);
                        editor.dirty_models.remove(&index);
                        if exiting {
                            editor.continue_exit();
                        }
                    }
                }
            }
            Err(error) => {
                editor.status = format!("Не удалось сохранить модель · {error}");
            }
        }
    }
    for &action in actions {
        if state.gesture.as_ref().is_some_and(|gesture| {
            gesture.is_preview_tool() && match action {
                Action::CloseModelTab(index) => gesture.model == index,
                Action::ViewModel(index) => gesture.model != index,
                Action::ModelView(ModelViewAction::Close | ModelViewAction::Retry) => true,
                _ => false,
            }
        }) {
            state.clear_gesture(None, true);
        }
        if let Some(
            crate::editor::PendingAction::CloseModelTab(index)
            | crate::editor::PendingAction::CloseModelEditor(index),
        ) = editor.pending
        {
            match action {
                Action::SaveAndContinue => state.start_save(index, editor),
                Action::DiscardAndContinue => {
                    if state.saving.is_none() {
                        let exiting = matches!(
                            editor.pending,
                            Some(crate::editor::PendingAction::CloseModelEditor(_))
                        );
                        state.documents.remove(&index);
                        editor.dirty_models.remove(&index);
                        editor.pending = None;
                        editor.close_model_tab(index);
                        if exiting {
                            editor.continue_exit();
                        }
                    }
                }
                Action::CancelPending => {
                    editor.pending = None;
                    editor.exit_queue.clear();
                }
                _ => {}
            }
            continue;
        }
        if editor.pending.is_some()
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
        {
            continue;
        }
        let action = match action {
            Action::ModelView(ModelViewAction::Edit(action)) => action,
            action => match toolbar_action(action) {
                Some(action) => action,
                None => continue,
            },
        };
        let Some(index) = editor.viewing_model else {
            continue;
        };
        if state.gesture.as_ref().is_some_and(input::Gesture::is_preview_tool)
            && !matches!(action, EditAction::Cancel | EditAction::Bevel(_) | EditAction::LoopCut(_))
        {
            continue;
        }
        if state.mode(Some(index)) == Mode::Uv && !state.available(action, Some(index)) {
            continue;
        }
        if state.input_error.is_some()
            && !matches!(action, EditAction::Cancel | EditAction::Wireframe)
        {
            state.status = state.input_error.clone().unwrap();
            continue;
        }
        match action {
            EditAction::LoopCut(control) => {
                if let Some(gesture) = state.gesture.as_mut().filter(|g| g.model == index) {
                    gesture.set_loop_control(control);
                }
            }
            EditAction::Bevel(control) => {
                if let Some(gesture) = state.gesture.as_mut().filter(|g| g.model == index) {
                    let slots = state.documents[&index].active()
                        .map_or(0, |o| o.materials.len());
                    gesture.set_bevel_control(control, slots);
                }
            }
            EditAction::Menu => {
                state.clear_gesture(Some(index), false);
                state.menu = !state.menu;
                state.add_menu = false;
            }
            EditAction::AddMenu => {
                state.clear_gesture(Some(index), false);
                state.add_menu = !state.add_menu;
                state.menu = false;
            }
            EditAction::Save => {
                state.clear_gesture(Some(index), false);
                state.start_save(index, editor);
            }
            EditAction::Cancel => {
                state.clear_gesture(Some(index), true);
                state.menu = false;
                state.add_menu = false;
                state.box_start = None;
                state.box_armed = false;
                state.bisect = false;
                state.bisect_start = None;
                state.extrude_tool = false;
            }
            EditAction::ExtrudeTool => {
                if state.available(action, Some(index)) {
                    state.extrude_tool = !state.extrude_tool;
                    state.box_start = None;
                    state.box_armed = false;
                    state.bisect = false;
                    state.bisect_start = None;
                }
            }
            EditAction::BeginExtrude => {
                if state.available(action, Some(index)) {
                    let doc = &state.documents[&index];
                    state.gesture = input::Gesture::extrude(doc, index);
                    state.box_start = None;
                    viewer.gesture = None;
                } else {
                    state.status = "Выделите вершины, рёбра или грани для экструдирования".into();
                }
            }
            EditAction::BeginBevel { .. } | EditAction::AdjustBevel => {
                if !state.available(action, Some(index)) {
                    state.status = "Выделите геометрию или завершите предыдущую операцию".into();
                    continue;
                }
                let doc = &state.documents[&index];
                state.gesture = match action {
                    EditAction::BeginBevel { vertices } => input::Gesture::bevel(doc, index, vertices),
                    _ => input::Gesture::adjust_bevel(doc, index, &state.last_bevel[&index]),
                };
                if state.gesture.is_some() {
                    state.box_start = None;
                    state.box_armed = false;
                    state.bisect = false;
                    state.bisect_start = None;
                    state.menu = false;
                    state.add_menu = false;
                    state.extrude_tool = false;
                    viewer.gesture = None;
                }
            }
            EditAction::BeginLoopCut | EditAction::AdjustLoopCut => {
                if !state.available(action, Some(index)) {
                    continue;
                }
                let doc = &state.documents[&index];
                state.gesture = if action == EditAction::BeginLoopCut {
                    input::Gesture::loop_cut(doc, index)
                } else {
                    input::Gesture::adjust_loop_cut(doc, index, &state.last_loop_cut[&index])
                };
                state.box_start = None;
                state.box_armed = false;
                state.bisect = false;
                state.bisect_start = None;
                state.menu = false;
                state.add_menu = false;
                state.extrude_tool = false;
                viewer.gesture = None;
            }
            EditAction::BeginKnife { only_selected } => {
                if !state.available(action, Some(index)) {
                    continue;
                }
                let doc = &state.documents[&index];
                state.gesture = input::Gesture::knife(doc, index, only_selected);
                state.box_start = None;
                state.box_armed = false;
                state.bisect = false;
                state.bisect_start = None;
                state.menu = false;
                state.add_menu = false;
                state.extrude_tool = false;
                viewer.gesture = None;
            }
            EditAction::Begin(tool) => {
                state.box_start = None;
                state.box_auto = false;
                if let Some(doc) = state.documents.get(&index) {
                    if let Some(pivot) = doc.pivot() {
                        state.gesture = Some(input::Gesture::new(tool, doc.snapshot(), index, pivot));
                        viewer.gesture = None;
                    }
                }
            }
            EditAction::Tool(tool) => {
                state.box_start = None;
                state.box_auto = false;
                state.tool = tool;
                state.extrude_tool = false;
                state.clear_gesture(Some(index), true);
            }
            EditAction::Xray => state.xray = !state.xray,
            EditAction::Wireframe => state.wireframe = !state.wireframe,
            EditAction::Local => state.local = !state.local,
            EditAction::Snap => state.snap = !state.snap,
            EditAction::SnapTarget(target) if state.available(action, Some(index)) => {
                state.snap_target = target;
                state.snap = true;
            }
            EditAction::Pivot(mode) if state.available(action, Some(index)) => {
                let doc = state.documents.get_mut(&index).unwrap();
                doc.pivot_mode = mode;
                doc.ui_revision += 1;
            }
            EditAction::Proportional => state.proportional = !state.proportional,
            EditAction::Connected => state.connected = !state.connected,
            EditAction::BoxSelect => {
                state.box_armed = true;
                state.bisect = false;
            }
            EditAction::Bisect => {
                state.extrude_tool = false;
                state.bisect = state.mode(Some(index)) == Mode::Edit;
                state.bisect_start = None;
                state.box_armed = false;
            }
            EditAction::Focus => {
                if let Some(doc) = state.documents.get(&index) {
                    if let Some(center) = doc.pivot() {
                        let radius = doc.active().map_or(viewer.radius, |o| {
                            o.geometry.radius() * o.transform.scale.abs().max_element() * 0.5
                        });
                        viewer.center = center;
                        viewer.distance = fit_distance(radius.max(0.001), 1.0);
                        viewer.fitted = true;
                    }
                }
            }
            _ => {
                state.clear_gesture(Some(index), false);
                let Some(doc) = state.documents.get_mut(&index) else {
                    state.status = state
                        .errors
                        .get(&index)
                        .cloned()
                        .unwrap_or("Модель ещё загружается".into());
                    continue;
                };
                let result: Result<(), String> = (|| {
                    match action {
                        EditAction::Mode(mode) => {
                            state.extrude_tool = false;
                            state.box_start = None;
                            doc.set_mode(mode);
                            state.menu = false;
                            state.bisect = false;
                            state.bisect_start = None;
                            viewer.gesture = None;
                        }
                        EditAction::ToggleMode => {
                            state.extrude_tool = false;
                            state.box_start = None;
                            doc.set_mode(if doc.mode == Mode::Object {
                                Mode::Edit
                            } else {
                                Mode::Object
                            });
                            state.bisect = false;
                            state.bisect_start = None;
                            viewer.gesture = None;
                        }
                        EditAction::Element(kind) => {
                            if let Some(mesh) = doc.active().map(|o| o.geometry.clone()) {
                                doc.selection.convert(kind, &mesh);
                                doc.ui_revision += 1;
                            }
                        }
                        EditAction::Operate(operation) => {
                            doc.operate(operation)?;
                        }
                        EditAction::Undo => doc.undo(),
                        EditAction::Redo => doc.redo(),
                        EditAction::SelectObject(id) => doc.select_object(id, shift),
                        EditAction::SelectEdgeLoop => {
                            let mesh = &doc.active().ok_or("Выберите объект")?.geometry;
                            doc.selection.edges = mesh.edge_loops(&doc.selection.edges);
                            doc.ui_revision += 1;
                        }
                        EditAction::CreateMaterial => {
                            if !doc.can_assign_material() || doc.paint_busy {
                                return Err("Выделите грани для назначения материала".into());
                            }
                            let material = doc.material_draft.slot(materials);
                            doc.assign_material(material, &state.default_material)?;
                        }
                        EditAction::AssignMaterial { object, slot } => {
                            let material = doc.objects.iter()
                                .find(|o| o.id == object)
                                .and_then(|o| o.materials.get(slot))
                                .cloned()
                                .ok_or("Материал не найден")?;
                            doc.assign_material(material, &state.default_material)?;
                        }
                        EditAction::SelectAll
                        | EditAction::Deselect
                        | EditAction::Invert
                        | EditAction::Linked
                        | EditAction::Grow
                        | EditAction::Shrink => {
                            if doc.mode == Mode::Object {
                                match action {
                                    EditAction::Deselect => doc.selected.clear(),
                                    EditAction::Invert => {
                                        doc.selected = doc
                                            .objects
                                            .iter()
                                            .filter(|o| !doc.selected.contains(&o.id))
                                            .map(|o| o.id)
                                            .collect()
                                    }
                                    _ => {
                                        doc.selected = doc
                                            .objects
                                            .iter()
                                            .filter(|o| !o.hidden)
                                            .map(|o| o.id)
                                            .collect()
                                    }
                                };
                            } else if let Some(mesh) = doc.active().map(|o| o.geometry.clone()) {
                                match action {
                                    EditAction::SelectAll => doc.selection.all(&mesh),
                                    EditAction::Deselect => doc.selection.clear(),
                                    EditAction::Invert => doc.selection.invert(&mesh),
                                    _ => doc.selection.connected(
                                        &mesh,
                                        matches!(action, EditAction::Grow),
                                        matches!(action, EditAction::Shrink),
                                    ),
                                }
                            }
                            doc.ui_revision += 1;
                        }
                        EditAction::Hide => {
                            for o in &mut doc.objects {
                                if doc.selected.contains(&o.id) {
                                    o.hidden = true;
                                }
                            }
                            doc.revision += 1;
                            doc.ui_revision += 1;
                        }
                        EditAction::Reveal => {
                            for o in &mut doc.objects {
                                o.hidden = false;
                            }
                            doc.revision += 1;
                            doc.ui_revision += 1;
                        }
                        EditAction::Add(primitive) => {
                            let mesh = match primitive {
                                0 => Mesh::from(Cuboid::from_size(Vec3::ONE)),
                                1 => Mesh::from(Sphere::new(0.5)),
                                2 => Mesh::from(Plane3d::default().mesh().size(1.0, 1.0)),
                                _ => Mesh::from(Cylinder::new(0.5, 1.0)),
                            };
                            let mut geometry = Geometry::default();
                            document::from_mesh(&mesh, &mut geometry, 0)?;
                            let before = doc.snapshot();
                            let size = (viewer.radius * 0.6).clamp(0.01, 1000.0);
                            for p in &mut geometry.positions {
                                *p *= size;
                            }
                            let id = doc.objects.iter().map(|o| o.id).max().unwrap_or(0) + 1;
                            let materials = doc.active().map_or(Vec::new(), |o| {
                                o.materials.first().cloned().into_iter().collect()
                            });
                            let name =
                                ["Куб", "Сфера", "Плоскость", "Цилиндр"][primitive.min(3)].into();
                            if doc.mode == Mode::Edit {
                                let o = doc.active_mut().ok_or("Нет активного объекта")?;
                                let offset = o.geometry.positions.len();
                                let m = o.transform.to_matrix().inverse()
                                    * Mat4::from_translation(viewer.center);
                                o.geometry.positions.extend(
                                    geometry
                                        .positions
                                        .into_iter()
                                        .map(|p| m.transform_point3(p)),
                                );
                                let first = o.geometry.faces.len();
                                o.geometry
                                    .faces
                                    .extend(geometry.faces.into_iter().map(|mut f| {
                                        for c in &mut f.corners {
                                            c.vertex += offset;
                                        }
                                        f
                                    }));
                                doc.selection.kind = Element::Face;
                                doc.selection.faces =
                                    (first..doc.active().unwrap().geometry.faces.len()).collect();
                            } else {
                                doc.objects.push(Object {
                                    id,
                                    source_node: None,
                                    name,
                                    geometry: geometry.into(),
                                    transform: Transform::from_translation(viewer.center),
                                    materials,
                                    hidden: false,
                                });
                                doc.select_object(id, false);
                            }
                            doc.checkpoint(before);
                            state.add_menu = false;
                        }
                        _ => {}
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    state.status = error;
                } else {
                    state.status.clear();
                }
            }
        }
    }
    state
        .documents
        .retain(|index, _| editor.model_tabs.contains(index));
    state
        .errors
        .retain(|index, _| editor.model_tabs.contains(index));
    state
        .drafts
        .retain(|index| editor.model_tabs.contains(index));
    state.last_bevel.retain(|index, _| editor.model_tabs.contains(index));
    state.last_loop_cut.retain(|index, _| editor.model_tabs.contains(index));
    editor.dirty_models = state
        .documents
        .iter()
        .filter(|&(&index, doc)| state.document_dirty(index, doc))
        .map(|(&i, _)| i)
        .chain(state.drafts.iter().copied())
        .collect();
}

pub(in crate::editor) fn shortcuts(
    keys: &ButtonInput<KeyCode>,
    state: Option<&State>,
    index: Option<usize>,
    actions: &mut MessageWriter<Action>,
) -> bool {
    // Modal transforms own number, axis, confirmation and cancellation keys.
    if state.is_some_and(|state| state.gesture.is_some()) {
        return true;
    }
    let mode = state.map_or(Mode::Object, |state| state.mode(index));
    if matches!(mode, Mode::Uv | Mode::Paint) {
        let control = keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]);
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let edit = if control && keys.just_pressed(KeyCode::KeyS) {
            Some(EditAction::Save)
        } else if control && keys.just_pressed(KeyCode::KeyZ) {
            Some(if shift {
                EditAction::Redo
            } else {
                EditAction::Undo
            })
        } else if control && keys.just_pressed(KeyCode::KeyY) {
            Some(EditAction::Redo)
        } else if keys.just_pressed(KeyCode::Tab) {
            Some(EditAction::Mode(Mode::Object))
        } else {
            None
        };
        if let Some(action) = edit {
            if mode == Mode::Paint && matches!(action, EditAction::Undo | EditAction::Redo) {
                actions.write(Action::ModelView(ModelViewAction::Paint(
                    if action == EditAction::Undo {
                        super::paint::PaintAction::Undo
                    } else {
                        super::paint::PaintAction::Redo
                    },
                )));
            } else {
                actions.write(Action::ModelView(ModelViewAction::Edit(action)));
            }
        }
        if mode == Mode::Paint {
            let tool = if keys.just_pressed(KeyCode::KeyB) {
                Some(super::paint::PaintTool::Brush)
            } else if keys.just_pressed(KeyCode::KeyE) {
                Some(super::paint::PaintTool::Eraser)
            } else if keys.just_pressed(KeyCode::KeyI) {
                Some(super::paint::PaintTool::Picker)
            } else {
                None
            };
            if let Some(tool) = tool {
                actions.write(Action::ModelView(ModelViewAction::Paint(
                    super::paint::PaintAction::Tool(tool),
                )));
            }
            if keys.just_pressed(KeyCode::Home) {
                actions.write(Action::ModelView(ModelViewAction::Fit));
            }
        }
        if mode == Mode::Uv && keys.just_pressed(KeyCode::Escape) {
            actions.write(Action::ModelView(ModelViewAction::Uv(uv::UvAction::Cancel)));
        }
        return true;
    }
    if keys.just_pressed(KeyCode::Home) {
        actions.write(Action::ModelView(ModelViewAction::Fit));
        return true;
    }
    let control = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let action = if control {
        if keys.just_pressed(KeyCode::KeyS) {
            Some(EditAction::Save)
        } else if keys.just_pressed(KeyCode::KeyZ) {
            Some(if shift {
                EditAction::Redo
            } else {
                EditAction::Undo
            })
        } else if keys.just_pressed(KeyCode::KeyY) {
            Some(EditAction::Redo)
        } else if keys.just_pressed(KeyCode::KeyI) {
            Some(EditAction::Invert)
        } else if keys.just_pressed(KeyCode::KeyL) {
            Some(EditAction::Linked)
        } else if keys.just_pressed(KeyCode::KeyR) {
            Some(EditAction::BeginLoopCut)
        } else if keys.just_pressed(KeyCode::KeyT) {
            Some(EditAction::Operate(Operation::Triangulate))
        } else if keys.just_pressed(KeyCode::KeyB) && mode == Mode::Edit {
            Some(EditAction::BeginBevel { vertices: shift })
        } else if keys.just_pressed(KeyCode::KeyJ) {
            Some(EditAction::Operate(Operation::Join))
        } else {
            None
        }
    } else if keys.just_pressed(KeyCode::F9) && mode == Mode::Edit {
        let loop_cut = state.zip(index)
            .is_some_and(|(state, index)| state.available(EditAction::AdjustLoopCut, Some(index)));
        Some(if loop_cut { EditAction::AdjustLoopCut } else { EditAction::AdjustBevel })
    } else if keys.just_pressed(KeyCode::Tab) {
        Some(if shift {
            EditAction::Snap
        } else {
            EditAction::ToggleMode
        })
    } else if keys.just_pressed(KeyCode::Digit1) {
        Some(EditAction::Element(Element::Vertex))
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(EditAction::Element(Element::Edge))
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(EditAction::Element(Element::Face))
    } else if keys.just_pressed(KeyCode::KeyG) {
        Some(EditAction::Begin(Tool::Move))
    } else if keys.just_pressed(KeyCode::KeyR) {
        Some(EditAction::Begin(Tool::Rotate))
    } else if keys.just_pressed(KeyCode::KeyS) {
        Some(EditAction::Begin(Tool::Scale))
    } else if keys.just_pressed(KeyCode::KeyE) {
        Some(EditAction::BeginExtrude)
    } else if keys.just_pressed(KeyCode::KeyI) {
        Some(EditAction::Operate(Operation::Inset))
    } else if keys.just_pressed(KeyCode::KeyD) && shift {
        Some(EditAction::Operate(Operation::Duplicate))
    } else if keys.just_pressed(KeyCode::KeyA) {
        Some(if shift {
            EditAction::AddMenu
        } else if alt {
            EditAction::Deselect
        } else {
            EditAction::SelectAll
        })
    } else if keys.just_pressed(KeyCode::KeyJ) && alt {
        Some(EditAction::Operate(Operation::Quads))
    } else if keys.just_pressed(KeyCode::KeyF) {
        Some(if mode == Mode::Edit {
            EditAction::Operate(Operation::Fill)
        } else {
            EditAction::Focus
        })
    } else if keys.just_pressed(KeyCode::NumpadDecimal) {
        Some(EditAction::Focus)
    } else if keys.just_pressed(KeyCode::KeyQ) {
        Some(EditAction::Tool(Tool::Universal))
    } else if keys.just_pressed(KeyCode::KeyW) {
        Some(EditAction::Tool(Tool::Move))
    } else if keys.just_pressed(KeyCode::KeyM) {
        Some(EditAction::Operate(if alt {
            Operation::MergeDistance
        } else {
            Operation::MergeCenter
        }))
    } else if keys.just_pressed(KeyCode::KeyB) {
        Some(EditAction::BoxSelect)
    } else if keys.just_pressed(KeyCode::KeyK) {
        Some(EditAction::BeginKnife { only_selected: shift })
    } else if keys.just_pressed(KeyCode::KeyO) {
        Some(if alt {
            EditAction::Connected
        } else {
            EditAction::Proportional
        })
    } else if keys.just_pressed(KeyCode::KeyZ) && alt {
        Some(EditAction::Xray)
    } else if keys.just_pressed(KeyCode::KeyH) {
        Some(if alt {
            EditAction::Reveal
        } else {
            EditAction::Hide
        })
    } else if keys.just_pressed(KeyCode::Delete) {
        Some(EditAction::Operate(Operation::Delete))
    } else if keys.just_pressed(KeyCode::Escape) {
        Some(EditAction::Cancel)
    } else {
        None
    };
    if let Some(action) = action {
        actions.write(Action::ModelView(ModelViewAction::Edit(action)));
        true
    } else {
        false
    }
}

#[cfg(test)]
#[path = "model_edit_tests.rs"]
mod tests;
