mod area_selection;
mod asset_placement;
mod browser;
mod browser_files;
mod dialogue;
mod documents;
mod file_moves;
mod generation;
mod generation_backend;
mod generation_model;
mod hdri_catalog;
mod image_previews;
mod image_texture;
mod lighting;
mod lighting_gizmos;
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub mod mcp;
mod model_document;
mod model_mesh;
mod model_paint_data;
mod model_primitives;
mod model_uv_backend;
mod model_uv_mesh;
mod object_visibility;
mod photo_document;
mod play;
mod render_worker;
mod scene_selection;
pub(crate) mod selection_outline;
pub(crate) mod starfield;
#[cfg(test)]
mod tests;
mod thumbnails;
mod transform_space;
mod ui;
pub(crate) use ui::{icons, orbital};
mod viewport;
mod viewport_background;
mod viewport_grid;
mod wireframe;

use std::{collections::HashMap, path::PathBuf};

use bevy::{
    camera::{ClearColorConfig, primitives::Aabb, visibility::RenderLayers},
    gizmos::AppGizmoBuilder,
    gltf::{Gltf, GltfLoaderSettings},
    input_focus::tab_navigation::TabNavigationPlugin,
    light::PointLightShadowMap,
    prelude::*,
    transform::helper::TransformHelper,
    ui::IsDefaultUiCamera,
    window::{PrimaryWindow, WindowCloseRequested},
    world_serialization::WorldInstanceReady,
};

use crate::{
    data::{BoxBounds, CameraPlacement, LevelEdits as RoomEdits, NodeEdit, PlacedModel, Pose},
    project::{EditorProject, SceneDefinition},
    runtime as level,
};

type Room = usize;
type Shot = usize;
const HISTORY_LIMIT: usize = 64;

fn load_error<A: Asset>(server: &AssetServer, handle: &Handle<A>) -> Option<String> {
    use bevy::asset::{LoadState, RecursiveDependencyLoadState};
    match server.get_load_states(handle.id()) {
        Some((LoadState::Failed(error), _, _))
        | Some((_, _, RecursiveDependencyLoadState::Failed(error))) => Some(error.to_string()),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    Imported(usize),
    Model(u32),
    Camera(Shot),
    Light(u32),
    Environment,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tool {
    #[default]
    Universal,
    Move,
    Rotate,
    Scale,
}

impl Tool {
    fn operations(self) -> &'static [Self] {
        match self {
            Self::Universal => &[],
            Self::Move => &[Self::Move],
            Self::Rotate => &[Self::Rotate],
            Self::Scale => &[Self::Scale],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
enum Field {
    Position(usize),
    Rotation(usize),
    Scale(usize),
    Lens,
    Light(lighting::LightField),
    Environment(lighting::EnvironmentField),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reset {
    All,
    Position,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Message)]
#[cfg_attr(test, derive(Debug))]
enum Action {
    Open(Room),
    Narrative(ui::NarrativeAction),
    Generation(ui::GenerationAction),
    CloseTab(Room),
    CloseModelTab(usize),
    OpenImageTab(usize),
    CloseImageTab(usize),
    OpenBrowser,
    Select(Selection),
    ToggleVisibility(Selection),
    SceneObject(ui::SceneObjectAction),
    Lighting(lighting::LightingAction),
    HdriLibrary(ui::HdriAction),
    UseDownloadedHdri(usize),
    LightMenu,
    LightAdvanced,
    Tool(Tool),
    LocalSpace,
    ToggleSceneTree,
    Snap,
    Drawer,
    Browse(usize),
    BrowseBack,
    BrowseForward,
    BrowseUp,
    BrowseClear,
    BrowseRefresh,
    BrowseToggle(usize),
    BrowseFile(usize),
    BrowserNewFolder,
    BrowserCreateFolder,
    BrowserNewImage,
    BrowserCreateImage,
    BrowserNewScene,
    BrowserCreateScene,
    BrowserDialogueMenu,
    BrowserPrimitiveMenu,
    BrowserCreatePrimitive(model_primitives::Primitive),
    BrowserConfirmPrimitive,
    BrowserCancelPopup,
    BrowserDeleteFile,
    BrowserOpenEntry,
    BrowserRenameFile,
    BrowserConfirmRename,
    BrowserDuplicateFile,
    BrowserRestoreFile,
    ViewImage(usize),
    ViewModel(usize),
    ModelView(ui::ModelViewAction),
    ImageView(ui::ImageAction),
    ImageEdit(ui::ImageEditAction),
    MoveAsset(browser::Entry, usize),
    DropAsset(browser::Entry, Vec2),
    Undo,
    Redo,
    Save,
    Play,
    Focus,
    Pilot,
    Align,
    #[cfg(test)]
    Add(usize),
    Duplicate,
    Delete,
    Reset(Reset),
    #[cfg(test)]
    Number(Field, f32),
    SaveAndContinue,
    DiscardAndContinue,
    CancelPending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAction {
    CloseNarrative(usize),
    ExitNarrative(usize),
    CloseTab(Room),
    CloseImageTab(usize),
    CloseImageEditor(usize),
    CloseModelTab(usize),
    CloseModelEditor(usize),
    Close,
}

struct Imported {
    entity: Entity,
    name: String,
    original: Pose,
    parent: GlobalTransform,
    local: Transform,
}

#[derive(Clone)]
struct ImageTab {
    id: usize,
    path: PathBuf,
    dirty: bool,
}

#[derive(Clone, Copy)]
enum ImageReturnTab {
    Generation,
    Scene(Room),
    Model(usize),
}

#[derive(Resource)]
struct Editor {
    root: PathBuf,
    project: EditorProject,
    room: Room,
    open_tabs: Vec<Room>,
    documents: HashMap<Room, documents::SceneDocument>,
    restore_selection: Option<String>,
    edits: RoomEdits,
    saved: RoomEdits,
    undo: Vec<RoomEdits>,
    redo: Vec<RoomEdits>,
    camera_navigation: Option<RoomEdits>,
    imported: Vec<Imported>,
    imported_ancestors: HashMap<String, Vec<String>>,
    selection: Option<Selection>,
    selection_group: Option<(Room, Vec<Selection>)>,
    tool: Tool,
    local_space: bool,
    snap: bool,
    drawer: bool,
    /// Stable document tab ID, independent of the browser's current file indices.
    viewing_image: Option<usize>,
    image_tabs: Vec<ImageTab>,
    narrative: dialogue::Workspace,
    generation: generation::Workspace,
    next_image_tab: usize,
    image_return: Option<ImageReturnTab>,
    hdri_library_open: bool,
    viewing_model: Option<usize>,
    model_tabs: Vec<usize>,
    dirty_models: std::collections::BTreeSet<usize>,
    browser_popup: bool,
    scene_object_popup: bool,
    sidebar: ui::SidebarLayout,
    pilot: bool,
    focus: bool,
    pending: Option<PendingAction>,
    exit_requested: bool,
    exit_queue: Vec<Room>,
    status: String,
    play: play::Session,
    ready: bool,
    reload: bool,
    view: Transform,
    pivot: Vec3,
    view_orthographic: bool,
    lighting_defaults: crate::lighting::SceneLighting,
    show_lights: bool,
    hidden_objects: HashMap<Room, std::collections::HashSet<object_visibility::ObjectKey>>,
}

impl Editor {
    fn new(project: EditorProject, edits: RoomEdits) -> Self {
        let view = project.scenes[0].cameras[0].placement.pose.transform();
        let (hidden_objects, status) = match object_visibility::load(&project) {
            Ok(hidden) => (hidden, "Загрузка сцены…".into()),
            Err(error) => (
                HashMap::new(),
                format!("Не удалось загрузить видимость объектов · {error}"),
            ),
        };
        Self {
            root: project.asset_root.clone(),
            project,
            room: 0,
            open_tabs: vec![0],
            documents: HashMap::new(),
            restore_selection: None,
            saved: edits.clone(),
            edits,
            undo: Vec::new(),
            redo: Vec::new(),
            camera_navigation: None,
            imported: Vec::new(),
            imported_ancestors: HashMap::new(),
            selection: None,
            selection_group: None,
            tool: Tool::default(),
            local_space: false,
            snap: false,
            drawer: false,
            viewing_image: None,
            image_tabs: Vec::new(),
            narrative: default(),
            generation: default(),
            next_image_tab: 0,
            image_return: None,
            hdri_library_open: false,
            viewing_model: None,
            model_tabs: Vec::new(),
            dirty_models: default(),
            browser_popup: false,
            scene_object_popup: false,
            sidebar: ui::SidebarLayout::default(),
            pilot: false,
            focus: false,
            pending: None,
            exit_requested: false,
            exit_queue: Vec::new(),
            status,
            play: default(),
            ready: false,
            reload: true,
            view,
            pivot: Vec3::ZERO,
            view_orthographic: false,
            lighting_defaults: crate::lighting::SceneLighting::editor_default(),
            show_lights: true,
            hidden_objects,
        }
    }

    fn dirty(&self) -> bool {
        self.has_scene() && self.edits != self.saved
    }

    fn asset_viewer_open(&self) -> bool {
        self.generation.active
            || self.narrative.active.is_some()
            || self.viewing_image.is_some()
            || self.viewing_model.is_some()
            || self.hdri_library_open
    }

    fn model_tab_action(action: Action) -> bool {
        matches!(
            action,
            Action::Narrative(_)
                | Action::Generation(_)
                | Action::Open(_)
                | Action::CloseTab(_)
                | Action::ViewModel(_)
                | Action::CloseModelTab(_)
                | Action::ModelView(_)
                | Action::ToggleSceneTree
                | Action::Drawer
                | Action::OpenBrowser
                | Action::Browse(_)
                | Action::BrowseBack
                | Action::BrowseForward
                | Action::BrowseUp
                | Action::BrowseClear
                | Action::BrowseRefresh
                | Action::BrowseToggle(_)
                | Action::BrowseFile(_)
                | Action::ViewImage(_)
                | Action::OpenImageTab(_)
                | Action::CloseImageTab(_)
                | Action::ImageView(_)
                | Action::BrowserNewFolder
                | Action::BrowserNewImage
                | Action::BrowserCreateImage
                | Action::BrowserNewScene
                | Action::BrowserCreateScene
                | Action::BrowserDialogueMenu
                | Action::BrowserPrimitiveMenu
                | Action::BrowserCreatePrimitive(_)
                | Action::BrowserConfirmPrimitive
                | Action::ImageEdit(_)
                | Action::BrowserCreateFolder
                | Action::BrowserCancelPopup
                | Action::BrowserDeleteFile
                | Action::BrowserOpenEntry
                | Action::BrowserRenameFile
                | Action::BrowserConfirmRename
                | Action::BrowserDuplicateFile
                | Action::BrowserRestoreFile
        )
    }

    fn open_model_tab(&mut self, index: usize) {
        if index < self.project.catalog.len() {
            self.finish_camera_navigation();
            if !self.model_tabs.contains(&index) {
                self.model_tabs.push(index);
            }
            self.narrative.active = None;
            self.generation.active = false;
            self.viewing_model = Some(index);
            self.viewing_image = None;
        }
    }

    fn image_tab(&self, id: usize) -> Option<&ImageTab> {
        self.image_tabs.iter().find(|tab| tab.id == id)
    }

    fn open_image_tab(&mut self, path: PathBuf) {
        let id = if let Some(tab) = self.image_tabs.iter().find(|tab| tab.path == path) {
            tab.id
        } else {
            let id = self.next_image_tab;
            self.next_image_tab += 1;
            self.image_tabs.push(ImageTab {
                id,
                path,
                dirty: false,
            });
            id
        };
        self.activate_image_tab(id);
    }

    fn activate_image_tab(&mut self, id: usize) {
        if self.image_tab(id).is_none() {
            return;
        }
        if self.viewing_image.is_none() {
            self.image_return = if self.generation.active {
                Some(ImageReturnTab::Generation)
            } else {
                self.viewing_model
                    .map(ImageReturnTab::Model)
                    .or_else(|| self.has_scene().then_some(ImageReturnTab::Scene(self.room)))
            };
        }
        self.finish_camera_navigation();
        self.viewing_model = None;
        self.narrative.active = None;
        self.generation.active = false;
        self.viewing_image = Some(id);
    }

    fn close_image_tab(&mut self, id: usize) {
        let Some(position) = self.image_tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.image_tabs.remove(position);
        if self.viewing_image != Some(id) {
            return;
        }
        self.viewing_image = self
            .image_tabs
            .get(position.min(self.image_tabs.len().saturating_sub(1)))
            .map(|tab| tab.id);
        if self.viewing_image.is_some() {
            return;
        }
        match self.image_return.take() {
            Some(ImageReturnTab::Generation) if self.generation.opened => {
                self.generation.active = true;
            }
            Some(ImageReturnTab::Model(index)) if self.model_tabs.contains(&index) => {
                self.open_model_tab(index)
            }
            Some(ImageReturnTab::Scene(room)) if self.open_tabs.contains(&room) => {
                self.open_document(room);
            }
            _ if !self.has_scene() => {
                self.viewing_model = self.model_tabs.last().copied();
            }
            _ => {}
        }
    }

    fn close_model_tab(&mut self, index: usize) {
        let Some(position) = self.model_tabs.iter().position(|&tab| tab == index) else {
            return;
        };
        self.model_tabs.remove(position);
        if self.viewing_model == Some(index) {
            self.viewing_model = self
                .model_tabs
                .get(position.min(self.model_tabs.len().saturating_sub(1)))
                .copied();
        }
    }

    fn definition(&self) -> &SceneDefinition {
        &self.project.scenes[self.room]
    }

    fn default_camera(&self, shot: Shot) -> CameraPlacement {
        self.definition().cameras[shot].placement
    }

    fn camera(&self, shot: Shot) -> CameraPlacement {
        self.edits
            .cameras
            .get(&self.definition().cameras[shot].id)
            .copied()
            .unwrap_or_else(|| self.default_camera(shot))
    }

    fn pose(&self, selection: Selection) -> Option<Pose> {
        if self.object_deleted(selection) {
            return None;
        }
        match selection {
            Selection::Imported(index) => self.imported.get(index).map(|node| {
                self.edits
                    .nodes
                    .get(&node.name)
                    .map_or(node.original, |edit| edit.pose)
            }),
            Selection::Model(id) => self.model(id).map(|model| model.pose),
            Selection::Camera(shot) => Some(self.camera(shot).pose),
            Selection::Light(id) => self.light(id).map(|light| light.pose),
            Selection::Environment => None,
        }
    }

    fn model(&self, id: u32) -> Option<&PlacedModel> {
        self.edits.models.iter().find(|model| model.id == id)
    }

    fn label(&self, selection: Selection) -> String {
        let custom = match selection {
            Selection::Model(id) => self.edits.model_names.get(&id),
            Selection::Imported(index) => self
                .imported
                .get(index)
                .and_then(|node| self.edits.node_names.get(&node.name)),
            _ => None,
        };
        if let Some(name) = custom {
            return name.clone();
        }
        match selection {
            Selection::Imported(index) => {
                self.imported.get(index).map_or_else(String::new, |node| {
                    self.definition()
                        .node_policies
                        .get(&node.name)
                        .and_then(|policy| policy.label.clone())
                        .unwrap_or_else(|| node.name.clone())
                })
            }
            Selection::Camera(shot) => self.definition().cameras[shot].label.clone(),
            Selection::Light(id) => self
                .light(id)
                .map_or_else(String::new, |light| light.name.clone()),
            Selection::Environment => "Освещение сцены".into(),
            Selection::Model(id) => self.model(id).map_or_else(String::new, |model| {
                let file = model.asset.rsplit('/').next().unwrap_or_default();
                let label = self
                    .project
                    .catalog
                    .iter()
                    .find(|item| item.asset == model.asset)
                    .map_or(file, |item| item.label.as_str());
                format!("{label} · {id}")
            }),
        }
    }

    fn field_allowed(&self, target: Selection, field: Field) -> bool {
        if matches!(field, Field::Light(_) | Field::Environment(_)) {
            return self.lighting_field_allowed(target, field);
        }
        if let Selection::Light(id) = target {
            return self.light(id).is_some_and(|light| match field {
                Field::Position(_) => true,
                Field::Rotation(_) => light.kind != crate::lighting::LightKind::Point,
                _ => false,
            });
        }
        if target == Selection::Environment {
            return false;
        }
        if let Selection::Camera(_) = target {
            return !matches!(field, Field::Scale(_));
        }
        if let Selection::Imported(index) = target
            && let Some(node) = self.imported.get(index)
            && let Some(policy) = self.definition().node_policies.get(&node.name)
        {
            return match field {
                Field::Position(axis) => policy.translation[axis],
                Field::Rotation(axis) => policy.rotation[axis],
                Field::Scale(axis) => policy.scale[axis],
                Field::Lens => false,
                Field::Light(_) | Field::Environment(_) => false,
            };
        }
        !matches!(field, Field::Lens)
    }

    fn set_pose(&mut self, target: Selection, transform: Transform) {
        let pose = Pose::from(transform);
        if self
            .pose(target)
            .is_some_and(|current| same_pose(current, pose))
        {
            return;
        }
        match target {
            Selection::Imported(index) => {
                let node = &self.imported[index];
                if same_pose(pose, node.original) {
                    self.edits.nodes.remove(&node.name);
                } else {
                    self.edits.nodes.insert(
                        node.name.clone(),
                        NodeEdit {
                            original: node.original,
                            pose,
                        },
                    );
                }
            }
            Selection::Model(id) => {
                if let Some(model) = self.edits.models.iter_mut().find(|model| model.id == id) {
                    model.pose = pose;
                }
            }
            Selection::Camera(shot) => {
                let placement = CameraPlacement {
                    pose,
                    ..self.camera(shot)
                };
                self.set_camera(shot, placement);
            }
            Selection::Light(id) => {
                if let Some(light) = self.edits.lights.iter_mut().find(|light| light.id == id) {
                    light.pose = pose;
                }
            }
            Selection::Environment => {}
        }
    }

    fn set_camera(&mut self, shot: Shot, placement: CameraPlacement) {
        let original = self.default_camera(shot);
        if same_pose(placement.pose, original.pose) && (placement.lens - original.lens).abs() < 1e-5
        {
            let key = self.definition().cameras[shot].id.clone();
            self.edits.cameras.remove(&key);
        } else {
            let key = self.definition().cameras[shot].id.clone();
            self.edits.cameras.insert(key, placement);
        }
    }

    fn reset(&mut self, reset: Reset) {
        let Some(target) = self.selection else {
            return;
        };
        if target == Selection::Environment {
            if reset == Reset::All {
                self.edits.lighting = None;
            }
            return;
        }
        let Some(mut pose) = self.pose(target) else {
            return;
        };
        let mut light_defaults = None;
        let original = match target {
            Selection::Imported(index) => self.imported[index].original,
            Selection::Camera(shot) => self.default_camera(shot).pose,
            Selection::Light(id) => {
                let Some(current) = self.light(id) else {
                    return;
                };
                let preset = match current.kind {
                    crate::lighting::LightKind::Point => crate::lighting::LightPreset::Bulb,
                    crate::lighting::LightKind::Spot => crate::lighting::LightPreset::Spot,
                    crate::lighting::LightKind::Directional => crate::lighting::LightPreset::Sun,
                    crate::lighting::LightKind::Rect => crate::lighting::LightPreset::Softbox,
                };
                let mut light = preset.create(id);
                light.name = current.name.clone();
                let original = light.pose;
                light_defaults = Some(light);
                original
            }
            _ => Transform::IDENTITY.into(),
        };
        match reset {
            Reset::All => pose = original,
            Reset::Position => pose.position = original.position,
            Reset::Rotation => pose.rotation = original.rotation,
            Reset::Scale => pose.scale = original.scale,
        }
        self.set_pose(target, pose.transform());
        if reset == Reset::All {
            if let Selection::Camera(shot) = target {
                self.set_camera(shot, self.default_camera(shot));
            }
            if let Some(defaults) = light_defaults
                && let Some(light) = self
                    .edits
                    .lights
                    .iter_mut()
                    .find(|light| light.id == defaults.id)
            {
                *light = defaults;
            }
        }
    }

    fn finish_edit(&mut self, before: RoomEdits) {
        if before != self.edits {
            self.undo.push(before);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn begin_camera_navigation(&mut self) {
        if self.camera_navigation.is_none() {
            self.camera_navigation = Some(self.edits.clone());
        }
    }

    fn finish_camera_navigation(&mut self) {
        if let Some(before) = self.camera_navigation.take() {
            self.finish_edit(before);
        }
    }

    fn select(&mut self, target: Option<Selection>) {
        self.finish_camera_navigation();
        self.selection_group = None;
        self.selection = target;
        if let Some(Selection::Light(id)) = target
            && (self.tool == Tool::Scale
                || (self.tool == Tool::Rotate
                    && self
                        .light(id)
                        .is_some_and(|light| light.kind == crate::lighting::LightKind::Point)))
        {
            self.tool = Tool::Universal;
        }
        if !matches!(target, Some(Selection::Camera(_))) {
            self.pilot = false;
        }
    }

    fn add_model(&mut self, mut model: PlacedModel) {
        let next = self
            .edits
            .models
            .iter()
            .map(|model| model.id)
            .max()
            .unwrap_or(0);
        let Some(id) = next.checked_add(1) else {
            return;
        };
        model.id = id;
        self.edits.models.push(model);
        self.select(Some(Selection::Model(id)));
    }

    fn add_light(&mut self, mut light: crate::lighting::SceneLight) {
        let Some(id) = self
            .edits
            .lights
            .iter()
            .map(|light| light.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return;
        };
        light.id = id;
        self.edits.lights.push(light);
        self.select(Some(Selection::Light(id)));
    }

    fn field_value(&self, target: Selection, field: Field) -> Option<f32> {
        if matches!(field, Field::Light(_) | Field::Environment(_)) {
            return self.lighting_value(target, field);
        }
        let pose = self.inspector_pose(target)?;
        match field {
            Field::Position(axis) => Some(pose.translation[axis]),
            Field::Rotation(axis) => Some(euler_degrees(pose.rotation)[axis]),
            Field::Scale(axis) => Some(pose.scale[axis]),
            Field::Lens => match target {
                Selection::Camera(shot) => Some(self.camera(shot).lens),
                _ => None,
            },
            Field::Light(_) | Field::Environment(_) => None,
        }
    }

    fn set_field(&mut self, field: Field, value: f32) {
        let Some(target) = self.selection else { return };
        if !value.is_finite() || !self.field_allowed(target, field) {
            return;
        }
        if matches!(field, Field::Light(_) | Field::Environment(_)) {
            self.set_lighting_field(field, value);
            return;
        }
        let Some(mut pose) = self.inspector_pose(target) else {
            return;
        };
        match field {
            Field::Position(axis) => pose.translation[axis] = value.clamp(-1000.0, 1000.0),
            Field::Scale(axis) => {
                let sign = if value < 0.0 { -1.0 } else { 1.0 };
                pose.scale[axis] = sign * value.abs().clamp(0.01, 100.0);
            }
            Field::Rotation(axis) => {
                let mut degrees = euler_degrees(pose.rotation);
                degrees[axis] = value.rem_euclid(360.0);
                pose.rotation = Quat::from_euler(
                    EulerRot::YXZ,
                    degrees.y.to_radians(),
                    degrees.x.to_radians(),
                    degrees.z.to_radians(),
                );
            }
            Field::Lens => {
                if let Selection::Camera(shot) = target
                    && self.definition().cameras[shot].projection.valid_lens(value)
                {
                    let camera = CameraPlacement {
                        lens: value,
                        ..self.camera(shot)
                    };
                    self.set_camera(shot, camera);
                } else {
                    self.status = "Значение объектива вне допустимого диапазона".into();
                }
                return;
            }
            Field::Light(_) | Field::Environment(_) => return,
        }
        self.set_inspector_pose(target, pose, field);
    }
}

fn same_pose(left: Pose, right: Pose) -> bool {
    let left = left.transform();
    let right = right.transform();
    left.translation.abs_diff_eq(right.translation, 1e-5)
        && left.scale.abs_diff_eq(right.scale, 1e-5)
        && left.rotation.abs_diff_eq(right.rotation, 1e-5)
}

fn euler_degrees(rotation: Quat) -> Vec3 {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    Vec3::new(pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees())
}

#[derive(Resource, Default)]
struct Scene {
    root: Option<Entity>,
    pending: Option<Handle<Gltf>>,
    models: HashMap<u32, (Entity, String)>,
}

/// Add to an app after DefaultPlugins. The host owns the asset root and window.
pub struct EditorPlugin {
    project: EditorProject,
    edits: RoomEdits,
}

impl EditorPlugin {
    pub fn new(mut project: EditorProject) -> std::io::Result<Self> {
        project.asset_root = project.asset_root.canonicalize()?;
        crate::asset_paths::apply_project_paths(&mut project)?;
        project.validate()?;
        let edits = project.read_scene(0)?;
        Ok(Self { project, edits })
    }
}

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        area_selection::register(app);
        app.add_plugins(ui::LayerDragFxPlugin);
        wireframe::register(app);
        if !app.is_plugin_added::<crate::HdriPlugin>() {
            app.add_plugins(crate::HdriPlugin);
        }
        app.add_plugins((
            TabNavigationPlugin,
            viewport::HandlePlugin,
            starfield::StarfieldPlugin,
            orbital::OrbitalPlugin,
            viewport_background::BackgroundPlugin,
            viewport_grid::GridPlugin,
            ui::ButtonFxPlugin,
            ui::AssetLoadingPlugin,
            ui::SvgIconsPlugin,
            ui::ModelViewerPlugin,
            ui::HdriLibraryPlugin,
            ui::BrowserDeletionPlugin,
            image_texture::PaintTexturePlugin,
            ui::ImageCanvasPlugin,
            crate::window_chrome::WindowChromePlugin,
        ))
        .add_plugins((
            ui::NavigationGizmoPlugin,
            ui::BrowserPrimitivePlugin,
            ui::NarrativePlugin,
            ui::GenerationPlugin,
            ui::SceneObjectsPlugin,
            ui::ImageSelectionPlugin,
        ))
        .insert_resource(PointLightShadowMap { size: 512 })
        .insert_resource(Editor::new(self.project.clone(), self.edits.clone()))
        .init_resource::<Scene>()
        .init_resource::<viewport::PointerState>()
        .init_resource::<ui::NumericInput>()
        .init_resource::<ui::NumberScrub>()
        .init_resource::<ui::DrawerMotion>()
        .init_resource::<ui::DrawerSize>()
        .init_resource::<ui::AssetDrag>()
        .init_resource::<ui::LayerDrag>()
        .init_resource::<asset_placement::PlacementPreview>()
        .init_resource::<image_previews::ImagePreviews>()
        .init_resource::<ui::ImageViewer>()
        .init_resource::<ui::BrowserContext>()
        .init_gizmo_group::<viewport::TransformGizmos>()
        .init_gizmo_group::<selection_outline::SelectionGizmos>()
        .add_message::<Action>()
        .add_systems(
            Startup,
            (setup, selection_outline::setup, ui::setup, viewport::setup).chain(),
        )
        .add_systems(
            Update,
            (
                (
                    ui::browser_context_input,
                    ui::resize_drawer,
                    ui::resize_sidebar,
                    ui::scrub_numbers,
                    ui::drag_assets,
                    ui::drag_photo_layers,
                )
                    .chain(),
                (ui::input, ui::shortcuts, ui::scroll, ui::search_browser),
                (
                    ui::edit_light_name,
                    ui::update_numeric_input,
                    ui::edit_lighting_controls,
                )
                    .chain(),
                (
                    ui::browser_actions,
                    ui::resize_photo_editor,
                    ui::photo_editor_input,
                    ui::photo_color_input,
                    ui::view_images,
                    ui::browser_context_actions,
                )
                    .chain(),
                asset_placement::update,
                (
                    play::poll,
                    handle_actions,
                    ui::image_viewer_cursor,
                    ui::layout_sidebar,
                )
                    .chain(),
                ui::animate_drawer,
                begin_room,
                load_room,
                viewport::navigate,
                viewport::manipulate,
                sync_placements,
                object_visibility::sync,
                ui::update_camera_preview,
                lighting::sync,
                (thumbnails::watch_assets, thumbnails::update).chain(),
                image_previews::update,
                (
                    viewport::draw,
                    asset_placement::draw,
                    viewport_background::sync,
                    viewport_grid::sync,
                    (
                        ui::refresh,
                        ui::refresh_play,
                        ui::refresh_lighting,
                        ui::refresh_lighting_controls,
                    )
                        .chain(),
                    ui::refresh_browser,
                    ui::update_browser_files,
                    ui::refresh_image_viewer
                        .after(ui::refresh)
                        .after(ui::update_browser_files),
                    ui::refresh_photo_editor.after(ui::refresh_image_viewer),
                    ui::refresh_photo_color_picker.after(ui::refresh_photo_editor),
                    ui::refresh_photo_layers.after(ui::refresh_photo_editor),
                    ui::refresh_layer_form.after(ui::refresh_photo_layers),
                    ui::refresh_photo_layer_drag
                        .after(ui::refresh_photo_layers)
                        .after(ui::image_viewer_cursor),
                    ui::photo_layers_input.before(ui::refresh_photo_layers),
                    close_editor,
                ),
                (
                    ui::update_confirmation,
                    ui::update_control_hints,
                    ui::drag_visuals,
                    ui::refresh_browser_context,
                )
                    .chain(),
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            selection_outline::draw
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                .after(bevy::camera::visibility::VisibilitySystems::CalculateBounds),
        );
        app.add_systems(
            Last,
            (ui::cleanup_image_cursor, ui::number_scrub_cursor)
                .chain()
                .before(bevy::window::CursorSystems::Update),
        );
        app.add_systems(Update, ui::lock_numeric_text.after(ui::refresh));
        app.add_systems(
            PostUpdate,
            asset_placement::measure
                .after(bevy::transform::TransformSystems::Propagate)
                .after(bevy::camera::visibility::VisibilitySystems::CalculateBounds),
        );
        app.sub_app_mut(bevy::render::RenderApp)
            .add_systems(bevy::render::ExtractSchedule, thumbnails::track_frame);
    }
}

/// Create an editor app with the default renderer and a project-aware window.
pub fn create_app(project: EditorProject) -> Result<App, Box<dyn std::error::Error>> {
    let plugin = EditorPlugin::new(project)?;
    let mut app = App::new();
    let render_worker = render_worker::enabled();
    if render_worker {
        app.insert_resource(render_worker::RenderWorker)
            .insert_resource(render_worker::update_settings());
    }
    app.add_plugins(crate::AssetRedirectPlugin::new(&plugin.project.asset_root)?);
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: plugin.project.asset_root.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(render_worker::window(&plugin.project.title, render_worker)),
                close_when_requested: false,
                ..default()
            }),
    )
    .add_plugins(plugin);
    Ok(app)
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut editor: ResMut<Editor>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    render_worker: Option<Res<render_worker::RenderWorker>>,
) {
    if render_worker.is_none() {
        window.set_maximized(true);
    }
    commands.insert_resource(browser::Browser::new(&mut editor.project));
    commands.insert_resource(viewport::Surface::new(&mut images));
    commands.insert_resource(thumbnails::Thumbnails::new(
        &mut images,
        editor.project.catalog.len(),
        editor.project.scenes.len(),
    ));
    commands.spawn((
        Camera2d,
        RenderLayers::layer(1),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(0.080, 0.084, 0.102)),
            ..default()
        },
        IsDefaultUiCamera,
    ));
}

fn begin_room(
    mut editor: ResMut<Editor>,
    mut scene: ResMut<Scene>,
    server: Res<AssetServer>,
    mut commands: Commands,
    camera: Query<Entity, With<viewport::EditorCamera>>,
    surface: Res<viewport::Surface>,
) {
    if !std::mem::take(&mut editor.reload) {
        return;
    }
    if let Some(root) = scene.root.take() {
        commands.entity(root).despawn();
    }
    for (_, (entity, _)) in scene.models.drain() {
        commands.entity(entity).despawn();
    }
    for entity in &camera {
        commands.entity(entity).despawn();
    }
    scene.pending = None;
    editor.imported.clear();
    editor.ready = false;
    editor.pilot = false;
    if !editor.has_scene() {
        return;
    }
    let asset = editor.definition().asset.clone();
    viewport::spawn_camera(&mut commands, &editor, editor.view, &surface);
    if editor.definition().is_native() {
        scene.root = Some(
            commands
                .spawn((Transform::IDENTITY, Visibility::Inherited))
                .id(),
        );
        editor.ready = true;
        editor.status = "Сцена готова".into();
        return;
    }
    scene.pending = Some(
        server
            .load_builder()
            .with_settings(|settings: &mut GltfLoaderSettings| {
                settings.load_cameras = false;
                settings.load_lights = true;
                settings.load_animations = false;
            })
            .load(asset),
    );
}

fn load_room(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    mut editor: ResMut<Editor>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
) {
    let Some(handle) = &scene.pending else {
        return;
    };
    if let Some(error) = load_error(&server, handle) {
        editor.status = format!("Ошибка загрузки сцены · {error}");
        scene.pending = None;
        return;
    }
    if !server.is_loaded_with_dependencies(handle.id()) {
        return;
    }
    let Some(gltf) = gltfs.get(handle) else {
        return;
    };
    let world = gltf.default_scene.as_ref().or_else(|| gltf.scenes.first());
    scene.pending = None;
    let Some(world) = world else {
        editor.status = "GLB не содержит сцены".into();
        return;
    };
    let asset = editor.definition().asset.clone();
    let mut root = commands.spawn(WorldAssetRoot(world.clone()));
    if let Some(setup) = editor.project.scene_setup {
        let logical =
            crate::asset_paths::original_asset_path(&editor.root, &asset).unwrap_or(asset);
        setup(&mut root, &server, &logical);
    }
    root.observe(record_room);
    scene.root = Some(root.id());
}

fn record_room(
    ready: On<WorldInstanceReady>,
    scene: Res<Scene>,
    mut editor: ResMut<Editor>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    locals: Query<&Transform>,
    transforms: TransformHelper,
) {
    if !editor.has_scene() || scene.root != Some(ready.entity) {
        return;
    }
    let definition = editor.definition();
    let imported = level::imported_nodes(
        ready.entity,
        &definition.groups,
        &definition.excluded_nodes,
        &children,
        &names,
    )
    .filter_map(|(entity, name)| {
        let world = transforms.compute_global_transform(entity).ok()?;
        let parent = transforms
            .compute_global_transform(parents.get(entity).ok()?.parent())
            .ok()?;
        Some(Imported {
            entity,
            name: name.to_owned(),
            original: world.compute_transform().into(),
            parent,
            local: *locals.get(entity).ok()?,
        })
    })
    .collect();
    editor.imported = imported;
    editor.imported_ancestors = editor
        .imported
        .iter()
        .map(|node| {
            let ancestors = parents
                .iter_ancestors(node.entity)
                .filter_map(|ancestor| {
                    names
                        .get(ancestor)
                        .ok()
                        .map(|name| name.as_str().to_owned())
                })
                .collect();
            (node.name.clone(), ancestors)
        })
        .collect();
    editor.imported.sort_by(|a, b| a.name.cmp(&b.name));
    if editor
        .imported
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        editor.status = "Имена редактируемых объектов должны быть уникальными".into();
        return;
    }
    if let Some(name) = editor.restore_selection.take() {
        let selected = editor.imported.iter().position(|node| node.name == name);
        editor.select(selected.map(Selection::Imported));
    } else if editor.selection.is_none() {
        let selected = editor
            .definition()
            .initial_selection
            .as_ref()
            .and_then(|name| editor.imported.iter().position(|node| node.name == *name));
        editor.select(selected.map(Selection::Imported));
    }
    editor.ready = true;
    editor.status = "Сцена готова".into();
    info!(
        "Scene ready: {} ({} editable objects)",
        editor.definition().label,
        editor.imported.len()
    );
}

fn sync_placements(
    mut commands: Commands,
    editor: Res<Editor>,
    mut scene: ResMut<Scene>,
    server: Res<AssetServer>,
    mut transforms: Query<&mut Transform>,
) {
    if !editor.has_scene() || !editor.ready {
        return;
    }
    for node in &editor.imported {
        if let Ok(mut transform) = transforms.get_mut(node.entity) {
            let local = editor
                .edits
                .nodes
                .get(&node.name)
                .map_or(node.local, |edit| {
                    GlobalTransform::from(edit.pose.transform()).reparented_to(&node.parent)
                });
            transform.set_if_neq(local);
        }
    }
    scene.models.retain(|id, (entity, asset)| {
        if editor.model(*id).is_some_and(|model| model.asset == *asset) {
            return true;
        }
        commands.entity(*entity).despawn();
        false
    });
    for model in &editor.edits.models {
        let (entity, _) = scene.models.entry(model.id).or_insert_with(|| {
            let entity = level::spawn_model(&mut commands, &server, model);
            commands.entity(entity).observe(prepare_placed_model);
            (entity, model.asset.clone())
        });
        if let Ok(mut transform) = transforms.get_mut(*entity) {
            transform.set_if_neq(model.pose.transform());
        }
    }
}

fn prepare_placed_model(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    mut cameras: Query<&mut Camera>,
) {
    // A model may contain exported glTF cameras, but placing it must never
    // activate them or replace the editor's viewport.
    for entity in std::iter::once(ready.entity).chain(children.iter_descendants(ready.entity)) {
        if let Ok(mut camera) = cameras.get_mut(entity) {
            camera.is_active = false;
        }
    }
}

fn bounds_for(
    entity: Entity,
    children: &Query<&Children>,
    meshes: &Query<(&Aabb, &GlobalTransform)>,
    transforms: &Query<&GlobalTransform>,
) -> Option<BoxBounds> {
    let inverse = transforms.get(entity).ok()?.affine().inverse();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for mesh in children.iter_descendants(entity) {
        let Ok((bounds, world)) = meshes.get(mesh) else {
            continue;
        };
        let local = inverse * world.affine();
        let center = local.transform_point3(bounds.center.into());
        let half = Vec3::from(local.matrix3.abs() * bounds.half_extents);
        min = min.min(center - half);
        max = max.max(center + half);
    }
    (min.is_finite() && max.is_finite()).then(|| BoxBounds {
        center: ((max + min) * 0.5).to_array(),
        half_size: ((max - min) * 0.5).max(Vec3::splat(0.002)).to_array(),
    })
}

fn save_editor(
    editor: &mut Editor,
    scene: &Scene,
    children: &Query<&Children>,
    meshes: &Query<(&Aabb, &GlobalTransform)>,
    transforms: &Query<&GlobalTransform>,
) -> bool {
    let mut saved = editor.edits.clone();
    let mut waiting = false;
    for model in &mut saved.models {
        if model.bounds.is_none()
            && editor
                .project
                .catalog
                .iter()
                .any(|item| item.asset == model.asset && item.solid)
        {
            model.bounds = scene
                .models
                .get(&model.id)
                .and_then(|(entity, _)| bounds_for(*entity, children, meshes, transforms));
            waiting |= model.bounds.is_none();
        }
    }
    if waiting {
        editor.status = "Дождитесь загрузки добавленных моделей перед сохранением".into();
        return false;
    }
    match editor
        .definition()
        .validate_edits(&saved)
        .and_then(|()| saved.write(&editor.root, &editor.definition().file))
    {
        Ok(()) => {
            editor.edits = saved.clone();
            editor.saved = saved;
            editor.status = format!("Сохранено · {}", editor.definition().label);
            true
        }
        Err(error) => {
            editor.status = format!("Не удалось сохранить · {error}");
            false
        }
    }
}

fn apply_asset_drop(
    editor: &mut Editor,
    browser: &browser::Browser,
    entry: browser::Entry,
    point: Vec3,
    bounds: Option<BoxBounds>,
) -> bool {
    if !editor.has_scene()
        || !editor.ready
        || editor.pending.is_some()
        || !point.is_finite()
        || point.abs().max_element() > 1000.0
        || bounds.is_some_and(|bounds| !bounds.valid())
    {
        return false;
    }
    let Some(asset) = browser.model_asset(&editor.project.catalog, entry) else {
        return false;
    };
    // Bounds used to place a preview must not make decorative assets solid.
    let bounds = bounds.filter(|_| {
        editor
            .project
            .catalog
            .iter()
            .any(|item| item.asset == asset && item.solid)
    });
    editor.finish_camera_navigation();
    let before = editor.edits.clone();
    let label = asset.rsplit('/').next().unwrap_or(&asset).to_owned();
    editor.add_model(PlacedModel {
        id: 0,
        asset,
        pose: Transform::from_translation(point).into(),
        bounds,
    });
    editor.finish_edit(before);
    editor.status = format!("Добавлена модель · {label}");
    true
}

fn handle_actions(
    mut actions: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    numeric: Option<Res<ui::NumericInput>>,
    hdri_library: Option<Res<ui::HdriLibrary>>,
    scene: Res<Scene>,
    mut pointer: ResMut<viewport::PointerState>,
    children: Query<&Children>,
    meshes: Query<(&Aabb, &GlobalTransform)>,
    transforms: Query<&GlobalTransform>,
) {
    for action in actions.read().copied() {
        if matches!(action, Action::Play) && editor.play.running() {
            if let Err(error) = editor.play.stop() {
                editor.play.error = Some(format!("Не удалось остановить игру · {error}"));
            }
            continue;
        }
        if matches!(
            action,
            Action::Narrative(_)
                | Action::MoveAsset(_, _)
                | Action::HdriLibrary(_)
                | Action::DropAsset(_, _)
                | Action::Browse(_)
                | Action::BrowseBack
                | Action::BrowseForward
                | Action::BrowseUp
                | Action::BrowseClear
                | Action::BrowseRefresh
                | Action::BrowseToggle(_)
                | Action::BrowseFile(_)
                | Action::BrowserNewFolder
                | Action::BrowserNewImage
                | Action::BrowserCreateImage
                | Action::BrowserNewScene
                | Action::BrowserCreateScene
                | Action::BrowserDialogueMenu
                | Action::BrowserPrimitiveMenu
                | Action::BrowserCreatePrimitive(_)
                | Action::BrowserConfirmPrimitive
                | Action::BrowserCreateFolder
                | Action::BrowserCancelPopup
                | Action::ViewImage(_)
                | Action::BrowserDeleteFile
                | Action::BrowserOpenEntry
                | Action::BrowserRenameFile
                | Action::BrowserConfirmRename
                | Action::BrowserDuplicateFile
                | Action::BrowserRestoreFile
                | Action::OpenImageTab(_)
                | Action::CloseImageTab(_)
                | Action::ViewModel(_)
                | Action::CloseModelTab(_)
                | Action::ModelView(_)
                | Action::ImageView(_)
                | Action::ImageEdit(_)
                | Action::LightMenu
                | Action::LightAdvanced
        ) {
            continue;
        }
        let confirmation = matches!(
            action,
            Action::SaveAndContinue | Action::DiscardAndContinue | Action::CancelPending
        );
        // Asset documents finish their asynchronous save before closing.
        if matches!(
            editor.pending,
            Some(
                PendingAction::CloseNarrative(_)
                    | PendingAction::ExitNarrative(_)
                    | PendingAction::CloseImageTab(_)
                    | PendingAction::CloseImageEditor(_)
                    | PendingAction::CloseModelTab(_)
                    | PendingAction::CloseModelEditor(_)
            )
        ) {
            continue;
        }
        if editor.pending.is_some() != confirmation {
            continue;
        }
        if pointer.dragging()
            || (((editor.generation.active && !Editor::model_tab_action(action))
                || (editor.narrative.active.is_some() && !Editor::model_tab_action(action))
                || (editor.viewing_image.is_some() && !Editor::model_tab_action(action))
                || (editor.hdri_library_open && !matches!(action, Action::UseDownloadedHdri(_)))
                || editor.browser_popup
                || editor.scene_object_popup
                || (editor.viewing_model.is_some() && !Editor::model_tab_action(action)))
                && !confirmation)
        {
            continue;
        }
        if matches!(
            action,
            Action::Save
                | Action::Generation(ui::GenerationAction::Open)
                | Action::Generation(ui::GenerationAction::Toggle)
                | Action::Open(_)
                | Action::CloseTab(_)
                | Action::Play
                | Action::Lighting(_)
                | Action::UseDownloadedHdri(_)
                | Action::LocalSpace
        ) && let Some(error) = numeric.as_ref().and_then(|input| input.error_message())
        {
            editor.status = error.into();
            if matches!(action, Action::Play) {
                editor.play.error = Some(error.into());
            }
            continue;
        }
        editor.finish_camera_navigation();
        pointer.stop_navigation();
        match action {
            Action::Generation(ui::GenerationAction::Open) => editor.open_generation(),
            Action::Generation(ui::GenerationAction::Toggle) => {
                if editor.generation.active {
                    editor.close_generation();
                } else {
                    editor.open_generation();
                }
            }
            Action::Generation(ui::GenerationAction::Close) => editor.close_generation(),
            Action::Generation(_) => {}
            Action::Open(room) => {
                editor.open_document(room);
            }
            Action::CloseTab(room) => {
                if editor.document_dirty(room) {
                    editor.open_document(room);
                    editor.pending = Some(PendingAction::CloseTab(room));
                    editor.status = "Есть несохранённые изменения".into();
                } else {
                    editor.close_document(room);
                }
            }
            Action::CancelPending => {
                editor.pending = None;
                editor.exit_queue.clear();
                editor.status = "Продолжайте редактирование".into();
            }
            Action::DiscardAndContinue => continue_pending(&mut editor),
            Action::Select(target)
                if editor.ready && editor.has_scene() && editor.tool == Tool::Universal =>
            {
                editor.select(Some(target))
            }
            Action::ToggleVisibility(target) if editor.ready && editor.has_scene() => {
                editor.toggle_visibility(target);
            }
            Action::OpenBrowser => editor.drawer = true,
            Action::Tool(tool) => {
                let supported = tool == Tool::Universal
                    || (!matches!(editor.selection, Some(Selection::Environment))
                        && (!matches!(editor.selection, Some(Selection::Light(_)))
                            || editor.selection.is_some_and(|selection| {
                                (0..3).any(|axis| viewport::allowed(&editor, selection, tool, axis))
                            })));
                if supported {
                    editor.tool = tool;
                }
            }
            Action::ToggleSceneTree => editor.sidebar.toggle_tree(),
            Action::LocalSpace => {
                editor.local_space = !editor.local_space;
                editor.status = if editor.local_space {
                    "Используется локальное пространство"
                } else {
                    "Используется глобальное пространство"
                }
                .into();
            }
            Action::Snap => editor.snap = !editor.snap,
            Action::Drawer => editor.drawer = !editor.drawer,
            Action::Focus => {
                editor.pilot = false;
                editor.focus = true;
            }
            Action::Pilot => {
                if editor.ready && matches!(editor.selection, Some(Selection::Camera(_))) {
                    editor.pilot = !editor.pilot;
                }
            }
            Action::Undo if editor.has_scene() => {
                if let Some(previous) = editor.undo.pop() {
                    let current = std::mem::replace(&mut editor.edits, previous);
                    editor.redo.push(current);
                    editor.status = "Действие отменено".into();
                }
            }
            Action::Redo if editor.has_scene() => {
                if let Some(next) = editor.redo.pop() {
                    let current = std::mem::replace(&mut editor.edits, next);
                    editor.undo.push(current);
                    editor.status = "Действие повторено".into();
                }
            }
            Action::Save | Action::SaveAndContinue if editor.has_scene() && editor.ready => {
                if save_editor(&mut editor, &scene, &children, &meshes, &transforms)
                    && matches!(action, Action::SaveAndContinue)
                {
                    continue_pending(&mut editor);
                }
            }
            Action::Save | Action::SaveAndContinue => {}
            Action::Play if editor.has_scene() && editor.ready => {
                editor.finish_camera_navigation();
                if editor.definition().play.is_some()
                    && editor.dirty()
                    && !save_editor(&mut editor, &scene, &children, &meshes, &transforms)
                {
                    editor.play.error = Some(editor.status.clone());
                    continue;
                }
                if let Err(error) = play::start(&mut editor) {
                    editor.play.error = Some(format!("Не удалось запустить игру · {error}"));
                }
            }
            action if editor.ready => {
                let before = editor.edits.clone();
                match action {
                    #[cfg(test)]
                    Action::Add(index) => {
                        let asset = editor.project.catalog[index].asset.clone();
                        let point = editor.pivot.with_y(0.0);
                        editor.add_model(PlacedModel {
                            id: 0,
                            asset,
                            pose: Transform::from_translation(point).into(),
                            bounds: None,
                        });
                    }
                    Action::Duplicate => editor.duplicate_targets(),
                    Action::Delete => editor.delete_targets(),
                    Action::Reset(reset) => editor.reset(reset),
                    Action::Align => {
                        if let Some(target @ Selection::Camera(_)) = editor.selection
                            && !editor.pilot
                        {
                            let view = editor.view;
                            editor.set_pose(target, view);
                        }
                    }
                    #[cfg(test)]
                    Action::Number(field, value) => editor.set_field(field, value),
                    Action::Lighting(action) => {
                        lighting::handle(&mut editor, action);
                    }
                    Action::UseDownloadedHdri(index) => {
                        if let Some(asset) = hdri_library
                            .as_ref()
                            .and_then(|library| library.asset(index))
                            .cloned()
                        {
                            let mut settings = editor.environment().clone();
                            settings.hdri = None;
                            settings.hdri_asset = Some(asset.clone());
                            if settings.validate().is_ok() {
                                editor.edits.lighting = Some(settings);
                                editor.hdri_library_open = false;
                                editor.select(Some(Selection::Environment));
                                editor.status = format!("Подготовка HDRI · {}", asset.name);
                            }
                        }
                    }
                    _ => {}
                }
                editor.finish_edit(before);
            }
            _ => {}
        }
        if editor
            .selection
            .is_some_and(|target| target != Selection::Environment && editor.pose(target).is_none())
        {
            editor.select(None);
        }
    }
}

fn close_editor(
    mut requests: MessageReader<WindowCloseRequested>,
    mut editor: ResMut<Editor>,
    numeric: Option<Res<ui::NumericInput>>,
    text_editor: Option<Res<ui::narrative::text_editor::State>>,
    mut exit: MessageWriter<AppExit>,
) {
    if text_editor.as_ref().is_some_and(|state| state.is_open()) {
        requests.read().for_each(drop);
        return;
    }
    if std::mem::take(&mut editor.exit_requested) {
        exit.write(AppExit::Success);
        return;
    }
    if requests.read().last().is_none() {
        return;
    }
    if let Some(error) = numeric.as_ref().and_then(|input| input.error_message()) {
        editor.status = error.into();
        return;
    }
    if editor.pending.is_none() {
        editor.request_exit();
        if std::mem::take(&mut editor.exit_requested) {
            exit.write(AppExit::Success);
        }
    }
}

fn continue_pending(editor: &mut Editor) {
    match editor.pending.take() {
        Some(PendingAction::CloseNarrative(id)) => editor.narrative.close(id),
        Some(PendingAction::ExitNarrative(_)) => editor.continue_exit(),
        Some(PendingAction::CloseTab(room)) => editor.close_document(room),
        Some(PendingAction::Close) => editor.continue_exit(),
        Some(PendingAction::CloseImageTab(id)) => editor.close_image_tab(id),
        Some(PendingAction::CloseImageEditor(_)) => editor.continue_exit(),
        Some(PendingAction::CloseModelTab(id)) => editor.close_model_tab(id),
        Some(PendingAction::CloseModelEditor(_)) => editor.continue_exit(),
        None => {}
    }
}
