//! UV workspace: transactions, background jobs and view-only comparison.
use super::*;
use crate::editor::{model_uv_backend as backend, model_uv_mesh as topology};
use crate::editor::model_import_mesh::AutoUnwrap;
use crate::editor::model_bake_settings;
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use bevy::window::RequestRedraw;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[path = "model_uv_jobs.rs"]
mod jobs;

#[path = "model_uv_input.rs"]
mod input;
#[path = "model_uv_ui.rs"]
mod ui;
#[path = "model_uv_view.rs"]
mod view;

pub(in crate::editor) const UDIM_IMPORT_HINT: &str = "При следующем открытии внешней модели выделяет отдельный набор текстур для каждой плитки UDIM. Отключите для моделей с повторяющимися UV. Сохранённые плитки восстанавливаются всегда.";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum UvAction {
    Unwrap,
    Preview,
    Accept,
    Dismiss,
    Brush,
    ClearMask,
    Suggest,
    Heatmap,
    Pack,
    Cancel,
    Fit,
    Checker,
    Seams,
    Compare,
    SelectAll,
    Deselect,
    Rotate,
    ScaleUp,
    ScaleDown,
    Resolution,
    Padding,
    Quality(usize),
    DetectUdim,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct ImportPreferences {
    udim_tiles: bool,
    auto_unwrap: AutoUnwrap,
    texture_size: u32,
    bake_on_load: bool,
    bake_defaults: model_bake_settings::Defaults,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Default for ImportPreferences {
    fn default() -> Self {
        Self {
            udim_tiles: true,
            auto_unwrap: AutoUnwrap::default(),
            texture_size: crate::editor::model_paint_data::DEFAULT_SIZE,
            bake_on_load: false,
            bake_defaults: model_bake_settings::Defaults::default(),
            path: None,
        }
    }
}

impl ImportPreferences {
    fn open(path: Option<PathBuf>) -> Self {
        let mut prefs = Self::default();
        if let Some(file) = path.as_deref() {
            let load = || -> std::io::Result<Self> {
                let bytes = bevy_scene_format::read_limited(file, 8192)?;
                Ok(serde_json::from_slice(&bytes)?)
            };
            match load() {
                Ok(saved) => prefs = saved,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => warn!("Could not read model import preferences: {error}"),
            }
        }
        prefs.path = path;
        if !crate::editor::model_paint_data::RESOLUTIONS.contains(&prefs.texture_size) {
            prefs.texture_size = crate::editor::model_paint_data::DEFAULT_SIZE;
        }
        prefs
    }

    fn update(&mut self, change: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        let mut next = self.clone();
        change(&mut next);
        if let Some(file) = self.path.as_deref() {
            bevy_scene_format::atomic_write(
                file,
                &serde_json::to_vec(&next)?,
                bevy_scene_format::Revision::read(file)?,
            )?;
        }
        *self = next;
        Ok(())
    }

    fn toggle_udim(&mut self) -> std::io::Result<()> {
        self.update(|prefs| prefs.udim_tiles = !prefs.udim_tiles)
    }
}
struct Job {
    source: PathBuf,
    action: UvAction,
    index: usize,
    object: usize,
    original: document::MeshData,
    corners: Vec<(usize, [usize; 3])>,
    settings: (u32, u32),
    progress: Arc<backend::Progress>,
    task: Task<Result<backend::Output, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.progress.cancel();
    }
}
struct Comparison {
    index: usize,
    object: usize,
    before: document::MeshData,
    after: document::MeshData,
}
type FaceMask = BTreeSet<[usize; 3]>;
fn face_key(mut vertices: [usize; 3]) -> [usize; 3] {
    vertices.sort_unstable();
    vertices
}
enum Proposed {
    Layout(document::MeshData),
    Island(FaceMask),
}
struct Proposal {
    source: PathBuf,
    index: usize,
    object: usize,
    original: document::MeshData,
    result: Proposed,
    settings: (u32, u32),
}
#[derive(Clone, Copy, Default, PartialEq)]
enum Tool {
    #[default]
    Select,
    Brush,
    Wand,
}
struct Drag {
    model: usize,
    source: PathBuf,
    start: Vec2,
    snapshot: Snapshot,
    atlas: topology::Atlas,
    selected: BTreeSet<usize>,
    changed: bool,
    tile_delta: Option<Vec2>,
}

#[derive(Resource)]
pub(in crate::editor) struct UvState {
    import: ImportPreferences,
    job: Option<Job>,
    proposal: Option<Proposal>,
    layout_settings: Option<(u32, u32)>,
    masks: BTreeMap<(PathBuf, usize), FaceMask>,
    tool: Tool,
    overlay_revision: u64,
    brush_at: Option<Vec2>,
    heatmap: bool,
    before: Option<Comparison>,
    comparing: bool,
    selected: BTreeSet<usize>,
    atlas: topology::Atlas,
    stamp: Option<(usize, usize, u64, bool)>,
    view_object: Option<(usize, usize)>,
    center: Vec2,
    zoom: f32,
    pan: Option<Vec2>,
    drag: Option<Drag>,
    relocation: Option<input::Relocation>,
    checker: bool,
    seams: bool,
    quality: usize,
    resolution: u32,
    padding: u32,
    error: String,
    status: String,
    elapsed: f32,
    progress: f32,
    reveal: f32,
    split: f32,
    bvh: input::Bvh,
    bvh_stamp: Option<(usize, u64)>,
}
impl Default for UvState {
    fn default() -> Self {
        Self {
            import: ImportPreferences::open(
                super::super::paint::presets::directory()
                    .and_then(|folder| folder.parent().map(|root| root.join("model-import.json"))),
            ),
            job: None,
            proposal: None,
            layout_settings: None,
            masks: default(),
            tool: Tool::Select,
            overlay_revision: 0,
            brush_at: None,
            heatmap: false,
            before: None,
            comparing: false,
            selected: default(),
            atlas: default(),
            stamp: None,
            view_object: None,
            center: Vec2::splat(0.5),
            zoom: 1.,
            pan: None,
            drag: None,
            relocation: None,
            checker: true,
            seams: true,
            quality: 1,
            resolution: 2048,
            padding: 8,
            error: String::new(),
            status: String::new(),
            elapsed: 0.,
            progress: 0.,
            reveal: 0.,
            split: 0.,
            bvh: default(),
            bvh_stamp: None,
        }
    }
}
impl UvState {
    pub(in crate::editor) fn detect_udim_on_import(&self) -> bool {
        self.import.udim_tiles
    }

    pub(in crate::editor) fn auto_unwrap(&self) -> AutoUnwrap {
        self.import.auto_unwrap
    }

    pub(in crate::editor) fn toggle_udim_import(&mut self) -> std::io::Result<()> {
        self.import.toggle_udim()
    }

    pub(in crate::editor) fn cycle_auto_unwrap(&mut self) -> std::io::Result<()> {
        self.import.update(|prefs| prefs.auto_unwrap = prefs.auto_unwrap.next())
    }

    pub(in crate::editor) fn texture_size_on_import(&self) -> u32 {
        self.import.texture_size
    }

    pub(in crate::editor) fn cycle_texture_size(&mut self) -> std::io::Result<()> {
        let sizes = crate::editor::model_paint_data::RESOLUTIONS;
        self.import.update(|prefs| {
            let index = sizes.iter().position(|size| *size == prefs.texture_size).unwrap_or(0);
            prefs.texture_size = sizes[(index + 1) % sizes.len()];
        })
    }

    pub(in crate::editor) fn bake_on_load(&self) -> bool {
        self.import.bake_on_load
    }

    pub(in crate::editor) fn toggle_bake_on_load(&mut self) -> std::io::Result<()> {
        self.import.update(|prefs| prefs.bake_on_load = !prefs.bake_on_load)
    }

    pub(in crate::editor) fn bake_defaults(&self) -> model_bake_settings::Defaults {
        self.import.bake_defaults
    }

    pub(in crate::editor) fn cycle_bake_quality(&mut self) -> std::io::Result<()> {
        self.import.update(|prefs| prefs.bake_defaults.quality = prefs.bake_defaults.quality.next())
    }

    pub(in crate::editor) fn toggle_bake_match_names(&mut self) -> std::io::Result<()> {
        self.import.update(|prefs| prefs.bake_defaults.match_names = !prefs.bake_defaults.match_names)
    }

    pub(super) fn display_geometry(&self, index: usize, object: &Object) -> document::MeshData {
        if let Some(p) = &self.proposal
            && p.index == index
            && p.object == object.id
            && p.original == object.geometry
            && let Proposed::Layout(layout) = &p.result
        {
            return if self.comparing {
                p.original.clone()
            } else {
                layout.clone()
            };
        }
        self.before
            .as_ref()
            .filter(|b| {
                self.comparing
                    && b.index == index
                    && b.object == object.id
                    && b.after == object.geometry
            })
            .map_or_else(|| object.geometry.clone(), |b| b.before.clone())
    }
    fn comparison_available(&self, index: Option<usize>, state: &State) -> bool {
        if let Some(p) = &self.proposal {
            return matches!(p.result, Proposed::Layout(_))
                && Some(p.index) == index
                && state
                    .documents
                    .get(&p.index)
                    .and_then(Document::active)
                    .is_some_and(|o| o.id == p.object && o.geometry == p.original);
        }
        self.before.as_ref().is_some_and(|b| {
            Some(b.index) == index
                && state
                    .documents
                    .get(&b.index)
                    .and_then(|d| d.active())
                    .is_some_and(|o| o.id == b.object && o.geometry == b.after)
        })
    }
    fn available(&self, action: UvAction, index: Option<usize>, state: &State) -> bool {
        let doc = index
            .and_then(|i| state.documents.get(&i))
            .filter(|d| d.mode == Mode::Uv);
        let surface = doc
            .and_then(|d| d.active())
            .is_some_and(|o| !o.hidden && !o.geometry.faces.is_empty());
        let free = surface
            && self.job.is_none()
            && self.relocation.is_none()
            && self.proposal.is_none()
            && state.saving.is_none();
        match action {
            UvAction::DetectUdim => doc.is_some(),
            UvAction::Accept | UvAction::Dismiss => self.proposal.is_some() && self.job.is_none(),
            UvAction::Brush | UvAction::Suggest => free && !self.comparing,
            UvAction::ClearMask => {
                free && doc
                    .and_then(|d| {
                        d.active()
                            .and_then(|o| self.masks.get(&(d.source.path.clone(), o.id)))
                    })
                    .is_some_and(|m| !m.is_empty())
            }
            UvAction::Cancel => {
                self.job.is_some()
                    || self.relocation.is_some()
                    || self.proposal.is_some()
                    || self.tool != Tool::Select
            }
            UvAction::Unwrap | UvAction::Preview => free,
            UvAction::Pack => free && !self.comparing && self.atlas.area > 1e-9,
            UvAction::Compare => self.job.is_none() && self.comparison_available(index, state),
            UvAction::Resolution | UvAction::Padding | UvAction::Quality(_) => {
                surface && self.job.is_none()
            }
            UvAction::Rotate | UvAction::ScaleUp | UvAction::ScaleDown => {
                free && !self.selected.is_empty() && !self.comparing
            }
            _ => surface,
        }
    }
    fn active(&self, action: UvAction) -> bool {
        match action {
            UvAction::Brush => self.tool == Tool::Brush,
            UvAction::Suggest => self.tool == Tool::Wand,
            UvAction::Heatmap => self.heatmap,
            UvAction::Checker => self.checker,
            UvAction::Seams => self.seams,
            UvAction::Compare => self.comparing,
            UvAction::Quality(q) => q == self.quality,
            _ => false,
        }
    }
    fn rebuild(&mut self, index: usize, doc: &Document) {
        let Some(object) = doc.active() else {
            self.atlas = default();
            self.stamp = None;
            return;
        };
        let stamp = (index, object.id, doc.revision, self.comparing);
        if self.stamp == Some(stamp) {
            return;
        }
        if self.view_object != Some((index, object.id)) {
            self.view_object = Some((index, object.id));
            self.brush_at = None;
            self.selected.clear();
            self.fit(Some(object));
        }
        let geometry = self.display_geometry(index, object);
        self.atlas = topology::Atlas::with_offsets(&geometry, |slot| object.uv_offset(slot));
        if self.heatmap {
            self.atlas.measure_stretch(&geometry);
        }
        self.selected.retain(|i| *i < self.atlas.count);
        self.stamp = Some(stamp);
    }
    fn fit(&mut self, object: Option<&Object>) {
        let mut tiles = object.into_iter().flat_map(Object::udim_tiles);
        let first = tiles.next().map_or(Vec2::ZERO, |tile| tile.offset());
        let (mut min, mut max) = tiles.map(|tile| tile.offset())
            .fold((first, first + Vec2::ONE), |(min, max), offset| {
                (min.min(offset), max.max(offset + Vec2::ONE))
            });
        if object.is_some_and(|object| object.materials.iter().any(|slot| slot.udim().is_none())) {
            min = min.min(Vec2::ZERO);
            max = max.max(Vec2::ONE);
        }
        self.center = (min + max) * 0.5;
        self.zoom = (max - min).max_element().recip();
    }

    fn transform(&mut self, index: usize, state: &mut State, angle: f32, scale: f32) {
        if self.comparing || self.job.is_some() || self.proposal.is_some() {
            return;
        }
        let Some(doc) = state.documents.get_mut(&index) else {
            return;
        };
        let before = doc.snapshot();
        if let Some(object) = doc.active_mut() {
            let mut geometry = object.geometry.clone();
            self.atlas.transform(
                &mut geometry,
                &self.selected,
                Vec2::ZERO,
                angle,
                scale,
            );
            if !object.preserves_udim_assignment(&geometry) {
                self.error = "Остров выходит за границы своего тайла UDIM".into();
                return;
            }
            object.geometry = geometry;
        }
        self.error.clear();
        doc.checkpoint(before);
        self.reveal = 1.;
    }
}

pub(in crate::editor) struct UvPlugin;
impl Plugin for UvPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UvState>()
            .init_resource::<view::AssetsState>()
            .init_gizmo_group::<view::UvSeams>()
            .add_systems(
                Startup,
                ui::setup
                    .after(super::super::setup)
                    .after(controls::setup)
                    .before(super::super::environment::setup),
            )
            .add_systems(Startup, view::setup_gizmos)
            .add_systems(
                Update,
                (actions, poll, input::poll_relocation, input::handle)
                    .chain()
                    .after(super::super::actions)
                    .before(super::render),
            )
            .add_systems(Update, view::materials.after(xray::sync))
            .add_systems(
                Update,
                view::overlay.after(super::render).after(input::handle),
            )
            .add_systems(
                Update,
                (ui::refresh, view::refresh)
                    .chain()
                    .after(controls::refresh)
                    .before(crate::editor::ui::refresh_inspector),
            )
            .add_systems(
                PostUpdate,
                view::seams.after(bevy::transform::TransformSystems::Propagate),
            );
    }
}
fn actions(
    mut incoming: MessageReader<Action>,
    editor: Res<Editor>,
    mut state: ResMut<State>,
    mut uv: ResMut<UvState>,
) {
    for action in incoming.read() {
        let Action::ModelView(ModelViewAction::Uv(action)) = *action else {
            continue;
        };
        let Some(index) = editor
            .viewing_model
            .filter(|_| editor.pending.is_none() && editor.viewing_image.is_none())
        else {
            continue;
        };
        if !uv.available(action, Some(index), &state) {
            continue;
        }
        uv.error.clear();
        match action {
            UvAction::Unwrap | UvAction::Preview | UvAction::Pack => {
                state.clear_gesture(Some(index), false);
                if let Some(doc) = state.documents.get(&index) {
                    jobs::start(&mut uv, index, doc, action, None);
                }
                state.rendered = None;
            }
            UvAction::Accept => jobs::accept(&mut uv, &mut state, index),
            UvAction::Dismiss | UvAction::Cancel => {
                uv.job.take();
                uv.relocation.take();
                uv.proposal = None;
                uv.tool = Tool::Select;
                uv.brush_at = None;
                uv.comparing = false;
                uv.stamp = None;
                uv.overlay_revision += 1;
                state.rendered = None;
                uv.status.clear();
                uv.progress = 0.;
            }
            UvAction::Suggest | UvAction::Brush => {
                let tool = if action == UvAction::Suggest {
                    Tool::Wand
                } else {
                    Tool::Brush
                };
                uv.tool = if uv.tool == tool { Tool::Select } else { tool };
                uv.overlay_revision += 1;
                uv.brush_at = None;
            }
            UvAction::ClearMask => {
                if let Some(doc) = state.documents.get(&index)
                    && let Some(id) = doc.active
                {
                    uv.masks.remove(&(doc.source.path.clone(), id));
                    uv.overlay_revision += 1;
                }
            }
            UvAction::Heatmap => {
                uv.heatmap = !uv.heatmap;
                uv.overlay_revision += 1;
                uv.stamp = None;
            }
            UvAction::Fit => {
                uv.fit(state.documents.get(&index).and_then(Document::active));
            }
            UvAction::Checker => uv.checker = !uv.checker,
            UvAction::Seams => uv.seams = !uv.seams,
            UvAction::Compare => {
                uv.comparing = !uv.comparing;
                uv.stamp = None;
                uv.selected.clear();
                state.rendered = None;
            }
            UvAction::SelectAll => uv.selected = (0..uv.atlas.count).collect(),
            UvAction::Deselect => uv.selected.clear(),
            UvAction::Rotate => uv.transform(index, &mut state, std::f32::consts::FRAC_PI_2, 1.),
            UvAction::ScaleUp => uv.transform(index, &mut state, 0., 1.1),
            UvAction::ScaleDown => uv.transform(index, &mut state, 0., 1. / 1.1),
            UvAction::Resolution => {
                uv.resolution = match uv.resolution {
                    1024 => 2048,
                    2048 => 4096,
                    _ => 1024,
                }
            }
            UvAction::Padding => {
                uv.padding = match uv.padding {
                    4 => 8,
                    8 => 16,
                    _ => 4,
                }
            }
            UvAction::Quality(q) => uv.quality = q.min(2),
            UvAction::DetectUdim => {
                if let Err(error) = uv.import.toggle_udim() {
                    uv.error = format!("Не удалось сохранить настройку импорта · {error}");
                }
            }
        }
    }
}

fn apply_job(
    state: &mut State,
    index: usize,
    id: usize,
    original: &document::MeshData,
    corners: &[(usize, [usize; 3])],
    output: &backend::Output,
) -> Result<(document::MeshData, document::MeshData), String> {
    let object = state.documents.get(&index)
        .and_then(|doc| doc.objects.iter().find(|object| object.id == id))
        .ok_or("Объект удалён")?;
    if &object.geometry != original {
        return Err("Модель изменилась во время расчёта. Запустите развёртку снова.".into());
    }
    let after = apply_layout(object, corners, output)?;
    commit_layout(state, index, id, original, after)
}
fn apply_layout(
    object: &Object,
    corners: &[(usize, [usize; 3])],
    output: &backend::Output,
) -> Result<document::MeshData, String> {
    let mut mesh = topology::apply(&object.geometry, corners, output)?;
    for face in &mut mesh.faces {
        let offset = object.uv_offset(face.material);
        if offset != Vec2::ZERO {
            for corner in &mut face.corners {
                corner.uv += offset;
            }
        }
    }
    Ok(mesh.into())
}
fn commit_layout(
    state: &mut State,
    index: usize,
    id: usize,
    original: &document::MeshData,
    after: document::MeshData,
) -> Result<(document::MeshData, document::MeshData), String> {
    let doc = state
        .documents
        .get_mut(&index)
        .ok_or("Вкладка модели закрыта")?;
    let object = doc
        .objects
        .iter()
        .find(|o| o.id == id)
        .ok_or("Объект удалён")?;
    if &object.geometry != original {
        return Err("Модель изменилась во время расчёта. Запустите развёртку снова.".into());
    }
    let snapshot = doc.snapshot();
    doc.objects
        .iter_mut()
        .find(|o| o.id == id)
        .unwrap()
        .geometry = after.clone();
    doc.selection.clear();
    doc.checkpoint(snapshot);
    Ok((original.clone(), after))
}
fn poll(
    editor: Res<Editor>,
    mut state: ResMut<State>,
    mut uv: ResMut<UvState>,
    time: Res<Time>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    jobs::poll_job(&mut uv, &mut state, &editor);
    let showing = editor
        .viewing_model
        .and_then(|i| state.documents.get(&i))
        .is_some_and(|d| d.mode == Mode::Uv)
        && editor.pending.is_none()
        && editor.viewing_image.is_none();
    if uv.comparing && (!showing || !uv.comparison_available(editor.viewing_model, &state)) {
        uv.comparing = false;
        uv.stamp = None;
        state.rendered = None;
    }
    if !showing {
        uv.tool = Tool::Select;
        uv.brush_at = None;
    }
    let dt = time.delta_secs().min(0.1);
    let target = if showing { 1. } else { 0. };
    let previous = uv.split;
    uv.split += (target - uv.split) * (1. - (-dt * 15.).exp());
    if (target - uv.split).abs() < 0.001 {
        uv.split = target;
    }
    if let Some(job) = &uv.job {
        let status = job.progress.snapshot();
        let fraction = status.fraction;
        let phase = status.phase;
        uv.elapsed += dt;
        uv.progress += (fraction - uv.progress) * (1. - (-dt * 9.).exp());
        if !phase.is_empty() {
            uv.status = phase;
        }
        redraw.write(RequestRedraw);
    }
    if (uv.split - previous).abs() > 0.00001 || uv.reveal > 0.001 {
        redraw.write(RequestRedraw);
    }
    uv.reveal = (uv.reveal - dt * 0.8).max(0.);
    if showing
        && let Some((index, doc)) = editor
            .viewing_model
            .and_then(|i| state.documents.get(&i).map(|d| (i, d)))
    {
        uv.rebuild(index, doc);
    }
}

#[cfg(test)]
#[path = "model_uv_tests.rs"]
mod tests;
