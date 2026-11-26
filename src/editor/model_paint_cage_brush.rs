//! Changes the generated bake cage without touching textures or model geometry.
use super::*;
use crate::editor::model_cage_data::Edit;
pub(in crate::editor::ui::model_viewer::paint) use crate::editor::model_cage_data::Scales;
use crate::editor::ui::model_viewer::editing::input::{Bvh, XrayGizmos, ray, rect};

#[derive(Clone, Copy)]
enum Operation {
    Raise,
    Lower,
    Restore,
}

struct Vertices {
    mesh: MeshData,
    transform: Transform,
    positions: Vec<Vec3>,
    cells: HashMap<IVec3, Vec<usize>>,
    width: f32,
}

impl Vertices {
    fn new(object: &crate::editor::model_document::Object) -> Self {
        let positions: Vec<_> = object
            .geometry
            .positions
            .iter()
            .map(|&point| object.transform.transform_point(point))
            .collect();
        let (lo, hi) = positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &point| (lo.min(point), hi.max(point)),
        );
        let width = ((hi - lo).length() / 40.).max(1e-6);
        let mut cells = HashMap::<IVec3, Vec<usize>>::new();
        for (i, &point) in positions.iter().enumerate() {
            let cell = (point / width).floor().as_ivec3();
            cells.entry(cell).or_default().push(i);
        }
        Self {
            mesh: object.geometry.clone(),
            transform: object.transform,
            positions,
            cells,
            width,
        }
    }

    fn work(&self, point: Vec3, radius: f32, mut visit: impl FnMut(usize, f32)) {
        let radius_squared = radius * radius;
        let mut candidate = |i: usize| {
            let distance = self.positions[i].distance_squared(point);
            if distance < radius_squared {
                let x = distance.sqrt() / radius;
                visit(i, 1. - x * x * (3. - 2. * x));
            }
        };
        let lo = ((point - radius) / self.width).floor().as_ivec3();
        let hi = ((point + radius) / self.width).floor().as_ivec3();
        if ((hi.as_vec3() - lo.as_vec3()) + 1.).element_product() >= self.positions.len() as f32 {
            for i in 0..self.positions.len() {
                candidate(i);
            }
        } else {
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        if let Some(indices) = self.cells.get(&IVec3::new(x, y, z)) {
                            for &i in indices {
                                candidate(i);
                            }
                        }
                    }
                }
            }
        }
    }
}

struct Stroke {
    model: usize,
    object: usize,
    mesh: MeshData,
    committed: Arc<crate::editor::model_cage_data::Edits>,
    before: Option<Edit>,
}

#[derive(Resource, Default)]
pub(in crate::editor::ui::model_viewer) struct Brush {
    vertices: Option<Vertices>,
    stroke: Option<Stroke>,
    pick_key: Option<(usize, u64, u64)>,
    pick_task: Option<Task<Bvh>>,
    pick: Option<Bvh>,
}

#[cfg(test)]
impl Brush {
    pub(in crate::editor::ui::model_viewer::paint) fn pick_ready(&self) -> bool {
        self.pick.is_some()
    }
}

pub(in crate::editor::ui::model_viewer) fn input(
    editor: Res<Editor>,
    mut editing: ResMut<editing::State>,
    mut paint: ResMut<PaintState>,
    mut baked: ResMut<State>,
    mut brush: ResMut<Brush>,
    viewer: Res<Viewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    drawer: Res<DrawerMotion>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    controls: engine::VisibleControls,
    mut gizmos: Gizmos<XrayGizmos>,
) {
    let cancelled = keys.just_pressed(KeyCode::Escape);
    if brush.stroke.as_ref().is_some_and(|stroke| {
        cancelled
            || !mouse.pressed(MouseButton::Left)
            || editor.viewing_model != Some(stroke.model)
            || editor.pending.is_some()
            || !window.focused
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
            || drawer.resizing
            || editing.documents.get(&stroke.model).is_none_or(|doc| {
                doc.mode != Mode::Paint || !Arc::ptr_eq(&stroke.committed, &doc.cage_edits)
            })
            || paint.sessions.get(&stroke.model).is_none_or(|session| {
                !session.cage_brush_active()
                    || session.binding.is_none_or(|(object, _)| object != stroke.object)
            })
    }) && let Some(stroke) = brush.stroke.take() {
        finish(stroke, &mut editing, &mut paint, &mut baked, cancelled);
        if cancelled {
            return;
        }
    }
    let active = (|| {
        let model = editor.viewing_model?;
        let doc = editing.documents.get(&model)?;
        let session = paint.sessions.get(&model)?;
        let (id, _) = session.binding?;
        let object = doc
            .objects
            .iter()
            .find(|object| object.id == id && !object.hidden)?;
        if doc.mode != Mode::Paint
            || !session.cage_brush_active()
            || session.bake_busy
            || session.bake_picking
            || !window.focused
            || editor.pending.is_some()
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
            || drawer.resizing
        {
            return None;
        }
        Some((model, doc, object))
    })();
    let Some((model, doc, object)) = active else {
        if let Some(stroke) = brush.stroke.take() {
            finish(stroke, &mut editing, &mut paint, &mut baked, false);
        }
        *brush = Brush::default();
        return;
    };
    let session = paint.sessions.get_mut(&model).unwrap();
    let visibility = &session.set_appearance.visibility;
    let pick_key = (model, doc.revision, visibility.revision);
    if brush.pick_key != Some(pick_key) {
        brush.pick_key = Some(pick_key);
        brush.pick = None;
        let objects = doc.objects.clone();
        let visibility = visibility.clone();
        brush.pick_task = Some(AsyncComputeTaskPool::get().spawn(async move {
            Bvh::from_objects_where(&objects, |object, slot| visibility.shown(object, slot))
        }));
    }
    if let Some(pick) = brush
        .pick_task
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        brush.pick = Some(pick);
        brush.pick_task = None;
    }
    let Some(pick) = &brush.pick else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Some(viewport) = rect(&canvas).filter(|rect| rect.contains(cursor)) else {
        return;
    };
    if drawer.covers(cursor, &window)
        || keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
        || mouse.any_pressed([MouseButton::Right, MouseButton::Middle])
        || controls.contains(cursor)
    {
        return;
    }
    let (origin, direction) = ray(&viewer, viewport, cursor);
    let Some((id, _, distance)) = pick.cast(origin, direction, None) else {
        return;
    };
    if id != object.id {
        return;
    }
    let hit = origin + direction * distance;
    let pose = view_pose(&viewer);
    let width = view_height(&viewer, (hit - pose.translation).dot(*pose.forward()));
    let radius = width * session.cage_radius / viewport.height();
    if !radius.is_finite() || radius <= 0. {
        return;
    }
    let operation = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        Operation::Restore
    } else if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
        Operation::Lower
    } else {
        Operation::Raise
    };
    let color = match operation {
        Operation::Raise => Color::srgb(1., 0.78, 0.36),
        Operation::Lower => Color::srgb(0.43, 0.72, 1.),
        Operation::Restore => Color::WHITE,
    };
    gizmos.linestrip(
        (0..=48).map(|i| {
            let (sin, cos) = (i as f32 * std::f32::consts::TAU / 48.).sin_cos();
            hit + (*pose.right() * cos + *pose.up() * sin) * radius
        }),
        color,
    );

    if mouse.just_pressed(MouseButton::Left) {
        let before = session
            .cage_edits
            .get(&object.id)
            .filter(|edit| edit.matches(&object.geometry))
            .cloned();
        brush.stroke = Some(Stroke {
            model,
            object: object.id,
            mesh: object.geometry.clone(),
            committed: doc.cage_edits.clone(),
            before,
        });
    }
    if !brush
        .stroke
        .as_ref()
        .is_some_and(|stroke| stroke.model == model && stroke.object == object.id)
    {
        return;
    }
    if !brush.vertices.as_ref().is_some_and(|cache| {
        cache.mesh.shares_storage_with(&object.geometry) && cache.transform == object.transform
    }) {
        brush.vertices = Some(Vertices::new(object));
    }
    let vertices = brush.vertices.as_ref().unwrap();
    let dt = time.delta_secs().clamp(1. / 240., 0.1);
    let speed = session.cage_strength * 2.5;
    let mut changes = Vec::new();
    let existing = session
        .cage_edits
        .get(&id)
        .filter(|edit| edit.matches(&object.geometry));
    vertices.work(hit, radius, |i, weight| {
        let scale = existing.map_or(1., |edit| edit.scales.values[i]);
        let next = match operation {
            Operation::Raise => scale * (speed * dt * weight).exp(),
            Operation::Lower => scale * (-speed * dt * weight).exp(),
            Operation::Restore => scale.lerp(1., (dt * 8. * weight).min(1.)),
        }
        .clamp(0.05, 8.);
        let next = if (next - 1.).abs() < 0.0001 { 1. } else { next };
        if scale != next {
            changes.push((i, next));
        }
    });
    if changes.is_empty() {
        return;
    }
    let edit = session.cage_edits.entry(id).or_insert_with(|| Edit {
        mesh: object.geometry.clone(),
        scales: Scales {
            values: Arc::new(vec![1.; vertices.positions.len()]),
        },
    });
    if !edit.matches(&object.geometry) {
        edit.mesh = object.geometry.clone();
        edit.scales.values = Arc::new(vec![1.; vertices.positions.len()]);
    }
    let scales = Arc::make_mut(&mut edit.scales.values);
    for (i, next) in changes {
        scales[i] = next;
    }
    touch(session, &mut baked, model, object.id);
}

fn finish(
    stroke: Stroke,
    editing: &mut editing::State,
    paint: &mut PaintState,
    baked: &mut State,
    cancelled: bool,
) {
    let Some(doc) = editing.documents.get_mut(&stroke.model) else { return };
    let Some(session) = paint.sessions.get_mut(&stroke.model) else { return };
    if !Arc::ptr_eq(&stroke.committed, &doc.cage_edits) {
        sync(doc, session);
        return;
    }
    if cancelled || doc.objects.iter().all(|object| {
        object.id != stroke.object || !stroke.mesh.shares_storage_with(&object.geometry)
    }) {
        if let Some(edit) = stroke.before {
            session.cage_edits.insert(stroke.object, edit);
        } else {
            session.cage_edits.remove(&stroke.object);
        }
        touch(session, baked, stroke.model, stroke.object);
        return;
    }
    let edit = session.cage_edits.get(&stroke.object)
        .filter(|edit| edit.scales.values.iter().any(|&value| value != 1.))
        .cloned();
    if edit.is_none() {
        session.cage_edits.remove(&stroke.object);
    }
    if doc.cage_edits.get(&stroke.object) == edit.as_ref() {
        return;
    }
    let before = doc.snapshot();
    if let Some(edit) = edit {
        Arc::make_mut(&mut doc.cage_edits).insert(stroke.object, edit);
    } else {
        Arc::make_mut(&mut doc.cage_edits).remove(&stroke.object);
    }
    doc.checkpoint(before);
    session.cage_snapshot = doc.cage_edits.clone();
}

pub(super) fn sync(doc: &Document, session: &mut Session) {
    if Arc::ptr_eq(&doc.cage_edits, &session.cage_snapshot) {
        return;
    }
    session.cage_edits = doc.cage_edits.iter().map(|(&id, edit)| (id, edit.clone())).collect();
    session.cage_snapshot = doc.cage_edits.clone();
    session.cage_edit_version = session.cage_edit_version.wrapping_add(1);
    session.revision += 1;
}

fn touch(session: &mut Session, baked: &mut State, model: usize, object: usize) {
    baked.invalidate_cage(model, object);
    session.cage_edit_version = session.cage_edit_version.wrapping_add(1);
    if session.bake_ready {
        session.bake_ready = false;
        session.revision += 1;
    }
}
