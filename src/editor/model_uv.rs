//! UV workspace: transactions, background jobs and view-only comparison.
use super::*;
use crate::editor::{model_uv_backend as backend, model_uv_mesh as topology};
use bevy::tasks::{IoTaskPool, Task, block_on, poll_once};
use bevy::window::RequestRedraw;
use std::collections::BTreeSet;

#[path = "model_uv_input.rs"]
mod input;
#[path = "model_uv_ui.rs"]
mod ui;
#[path = "model_uv_view.rs"]
mod view;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) enum UvAction {
    Unwrap,
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
}
struct Job {
    index: usize,
    object: usize,
    original: document::MeshData,
    input: topology::Input,
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
struct Drag {
    start: Vec2,
    snapshot: Snapshot,
    atlas: topology::Atlas,
    selected: BTreeSet<usize>,
    changed: bool,
}

#[derive(Resource)]
pub(in crate::editor) struct UvState {
    job: Option<Job>,
    before: Option<Comparison>,
    comparing: bool,
    selected: BTreeSet<usize>,
    atlas: topology::Atlas,
    stamp: Option<(usize, usize, u64, bool)>,
    center: Vec2,
    zoom: f32,
    pan: Option<Vec2>,
    drag: Option<Drag>,
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
            job: None,
            before: None,
            comparing: false,
            selected: default(),
            atlas: default(),
            stamp: None,
            center: Vec2::splat(0.5),
            zoom: 1.,
            pan: None,
            drag: None,
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
    pub(super) fn display_geometry(&self, index: usize, object: &Object) -> document::MeshData {
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
        let doc = index.and_then(|i| state.documents.get(&i));
        let surface = doc
            .and_then(|d| d.active())
            .is_some_and(|o| !o.hidden && !o.geometry.faces.is_empty());
        match action {
            UvAction::Cancel => self.job.is_some(),
            UvAction::Unwrap => surface && self.job.is_none() && state.saving.is_none(),
            UvAction::Pack => {
                surface && self.atlas.area > 1e-9 && self.job.is_none() && state.saving.is_none()
            }
            UvAction::Compare => self.job.is_none() && self.comparison_available(index, state),
            UvAction::Resolution | UvAction::Padding | UvAction::Quality(_) => {
                surface && self.job.is_none()
            }
            UvAction::Rotate | UvAction::ScaleUp | UvAction::ScaleDown => {
                surface && !self.selected.is_empty() && !self.comparing && self.job.is_none()
            }
            _ => surface,
        }
    }
    fn active(&self, action: UvAction) -> bool {
        match action {
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
        if self
            .stamp
            .is_none_or(|old| old.0 != index || old.1 != object.id)
        {
            self.selected.clear();
            self.center = Vec2::splat(0.5);
            self.zoom = 1.;
        }
        self.atlas = topology::Atlas::new(&self.display_geometry(index, object));
        self.selected.retain(|i| *i < self.atlas.count);
        self.stamp = Some(stamp);
    }
    fn transform(&mut self, index: usize, state: &mut State, angle: f32, scale: f32) {
        if self.comparing || self.job.is_some() {
            return;
        }
        let Some(doc) = state.documents.get_mut(&index) else {
            return;
        };
        let before = doc.snapshot();
        if let Some(object) = doc.active_mut() {
            self.atlas.transform(
                &mut object.geometry,
                &self.selected,
                Vec2::ZERO,
                angle,
                scale,
            );
        }
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
                (actions, poll, input::handle)
                    .chain()
                    .after(super::super::actions)
                    .before(super::render),
            )
            .add_systems(Update, view::materials.after(xray::sync))
            .add_systems(
                Update,
                (ui::refresh, view::refresh)
                    .chain()
                    .after(controls::refresh),
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
            UvAction::Unwrap | UvAction::Pack => {
                state.clear_gesture(Some(index), false);
                let Some(object) = state.documents.get(&index).and_then(|d| d.active()) else {
                    continue;
                };
                let request = topology::input(
                    &object.geometry,
                    [1.1, 1.25, 1.5][uv.quality],
                    uv.resolution,
                    uv.padding,
                );
                match request {
                    Ok(mut input) => {
                        if action == UvAction::Pack {
                            input.request.existing_uv = Some(
                                input
                                    .corners
                                    .iter()
                                    .map(|(fi, corners)| {
                                        corners.map(|ci| {
                                            object.geometry.faces[*fi].corners[ci].uv.to_array()
                                        })
                                    })
                                    .collect(),
                            );
                        }
                        let progress = Arc::new(backend::Progress::default());
                        let worker_progress = progress.clone();
                        let request = input.request.clone();
                        uv.job = Some(Job {
                            index,
                            object: object.id,
                            original: object.geometry.clone(),
                            input,
                            progress,
                            task: IoTaskPool::get()
                                .spawn(async move { backend::run(request, worker_progress) }),
                        });
                        uv.elapsed = 0.;
                        uv.progress = 0.;
                        uv.status = "Подготовка PartUV Geometric…".into();
                        uv.comparing = false;
                        uv.drag = None;
                        state.rendered = None;
                    }
                    Err(error) => uv.error = error,
                }
            }
            UvAction::Cancel => {
                uv.job.take();
                uv.status = "Развёртка отменена · модель не изменена".into();
                uv.progress = 0.;
            }
            UvAction::Fit => {
                uv.center = Vec2::splat(0.5);
                uv.zoom = 1.;
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
        }
    }
}

fn apply_job(
    state: &mut State,
    index: usize,
    id: usize,
    original: &document::MeshData,
    input: &topology::Input,
    output: &backend::Output,
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
    let geometry = topology::apply(original, input, output)?;
    let snapshot = doc.snapshot();
    let after: document::MeshData = geometry.into();
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
    if uv
        .job
        .as_ref()
        .is_some_and(|job| !state.documents.contains_key(&job.index))
    {
        uv.job.take();
    }
    let ready = uv
        .job
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)));
    if let Some(result) = ready {
        let job = uv.job.take().unwrap();
        match result.and_then(|output| {
            apply_job(
                &mut state,
                job.index,
                job.object,
                &job.original,
                &job.input,
                &output,
            )
        }) {
            Ok((before, after)) => {
                uv.before = Some(Comparison {
                    index: job.index,
                    object: job.object,
                    before,
                    after,
                });
                uv.comparing = false;
                uv.stamp = None;
                uv.status = "Развёртка готова · Ctrl+Z — отменить · Ctrl+S — сохранить".into();
                uv.progress = 1.;
                uv.reveal = 1.;
                uv.error.clear();
            }
            Err(error) => {
                uv.error = error;
                uv.status.clear();
                uv.progress = 0.;
            }
        }
    }
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
    if let Some((index, doc)) = editor
        .viewing_model
        .and_then(|i| state.documents.get(&i).map(|d| (i, d)))
    {
        uv.rebuild(index, doc);
    }
}

#[cfg(test)]
#[path = "model_uv_tests.rs"]
mod tests;
