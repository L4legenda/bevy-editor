use super::super::super::paint::engine::SurfacePreview;
use super::super::input as model_input;
use super::*;
use crate::editor::{
    image_texture::TextureUploads,
    model_document::udim::{self, Tile},
};
use bevy::input_focus::InputFocus;
use bevy::tasks::AsyncComputeTaskPool;
pub(super) use model_input::Bvh;

fn rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Rect {
    Rect::from_center_size(
        transform.affine().translation * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}
pub(super) fn point(uv: &UvState, canvas: Rect, cursor: Vec2) -> Vec2 {
    let height = 1.18_f32.max(1.18 / (canvas.width() / canvas.height()).max(0.01)) / uv.zoom;
    let delta = (cursor - canvas.center()) * height / canvas.height();
    uv.center + Vec2::new(delta.x, -delta.y)
}

fn tile_delta(drag: &Drag, object: &Object, cursor: Option<Vec2>) -> Option<Vec2> {
    if object.udim_tiles().next().is_none() {
        return None;
    }
    let from = Tile::at_uv(drag.start)?;
    let to = Tile::at_uv(cursor?)?;
    (from != to).then(|| to.offset() - from.offset())
}

pub(super) struct RelocationRequest {
    model: usize,
    source: PathBuf,
    object: usize,
    revision: u64,
    snapshot: Snapshot,
    islands: Vec<usize>,
    selected: BTreeSet<usize>,
    delta: Vec2,
}

type Relocated = (Object, Vec<(usize, SurfacePreview)>);

pub(super) struct Relocation {
    request: RelocationRequest,
    task: Task<Result<Relocated, String>>,
}

impl RelocationRequest {
    #[cfg(test)]
    fn move_islands(&self) -> Result<Object, String> {
        udim::relocate(self.original(), &self.islands, &self.selected, self.delta)
    }

    fn original(&self) -> &Object {
        self.snapshot.objects.iter().find(|object| object.id == self.object).unwrap()
    }

    fn commit(
        self,
        uv: &mut UvState,
        editing: &mut State,
        result: Result<Object, String>,
    ) {
        let Some(doc) = editing.documents.get_mut(&self.model).filter(|doc| {
            doc.source.path == self.source
                && doc.revision == self.revision
                && doc.mode == Mode::Uv
                && doc.active == Some(self.object)
        }) else {
            return;
        };
        match result {
            Ok(object) => {
                if let Some(current) = doc.active_mut() {
                    *current = object;
                    doc.checkpoint(self.snapshot);
                    uv.error.clear();
                    uv.reveal = 1.;
                }
            }
            Err(error) => uv.error = error,
        }
    }
}

pub(super) fn finish_drag(
    uv: &mut UvState,
    editing: &mut State,
    cursor: Option<Vec2>,
) -> Option<RelocationRequest> {
    let drag = uv.drag.take()?;
    if drag.tile_delta.is_some() {
        uv.overlay_revision += 1;
    }
    let doc = editing
        .documents
        .get_mut(&drag.model)
        .filter(|doc| doc.source.path == drag.source)?;
    let original = drag
        .snapshot
        .objects
        .iter()
        .find(|object| Some(object.id) == doc.active)?;
    if let Some(delta) = tile_delta(&drag, original, cursor) {
        let object = original.id;
        restore_preview(doc, &drag);
        return Some(RelocationRequest {
            model: drag.model,
            source: drag.source,
            object,
            revision: doc.revision,
            snapshot: drag.snapshot,
            islands: drag.atlas.islands,
            selected: drag.selected,
            delta,
        });
    }
    if drag.changed {
        doc.checkpoint(drag.snapshot);
    }
    None
}

fn submit(uv: &mut UvState, request: RelocationRequest) {
    let original = request.original().clone();
    let islands = request.islands.clone();
    let selected = request.selected.clone();
    let delta = request.delta;
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let object = udim::relocate(&original, &islands, &selected, delta)?;
        let previews = object.materials.iter().enumerate().filter_map(|(i, slot)| {
            let surface = slot.paint.as_ref()?;
            let already_rendered = original.materials.iter()
                .filter_map(|source| source.paint.as_ref())
                .any(|old| Arc::ptr_eq(old, surface));
            (!already_rendered).then(|| (i, SurfacePreview::prepare(surface)))
        }).collect();
        Ok((object, previews))
    });
    uv.error.clear();
    uv.status = "Перенос UV-островов…".into();
    uv.relocation = Some(Relocation { request, task });
}

pub(super) fn poll_relocation(
    mut uv: ResMut<UvState>,
    mut editing: ResMut<State>,
    editor: Res<Editor>,
    keys: Res<ButtonInput<KeyCode>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut uploads: ResMut<TextureUploads>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let Some(mut pending) = uv.relocation.take() else { return };
    let request = &pending.request;
    let available = editor.viewing_model == Some(request.model)
        && editor.viewing_image.is_none()
        && editor.pending.is_none()
        && editing.saving.is_none()
        && editing.documents.get(&request.model).is_some_and(|doc| {
            doc.source.path == request.source
                && doc.revision == request.revision
                && doc.mode == Mode::Uv
                && doc.active == Some(request.object)
        });
    if !available || keys.just_pressed(KeyCode::Escape) {
        uv.status.clear();
        return;
    }
    redraw.write(RequestRedraw);
    let Some(result) = block_on(poll_once(&mut pending.task)) else {
        uv.relocation = Some(pending);
        return;
    };
    uv.status.clear();
    let result = result.map(|(mut object, previews)| {
        for (slot, preview) in previews {
            preview.upload(&mut object.materials[slot], &mut images, &mut materials, &mut uploads);
        }
        object
    });
    pending.request.commit(&mut uv, &mut editing, result);
}

#[cfg(test)]
pub(super) fn finish_drag_for_test(uv: &mut UvState, editing: &mut State, cursor: Option<Vec2>) {
    if let Some(request) = finish_drag(uv, editing, cursor) {
        let result = request.move_islands();
        request.commit(uv, editing, result);
    }
}

fn restore_preview(doc: &mut Document, drag: &Drag) {
    if drag.changed
        && let Some(original) = drag
            .snapshot
            .objects
            .iter()
            .find(|object| Some(object.id) == drag.snapshot.active)
        && let Some(object) = doc
            .objects
            .iter_mut()
            .find(|object| object.id == original.id)
    {
        object.geometry = original.geometry.clone();
        doc.revision += 1;
        doc.ui_revision += 1;
    }
}

fn cancel_drag(uv: &mut UvState, editing: &mut State) {
    if let Some(drag) = uv.drag.take() {
        if drag.tile_delta.is_some() {
            uv.overlay_revision += 1;
        }
        if let Some(doc) = editing
            .documents
            .get_mut(&drag.model)
            .filter(|doc| doc.source.path == drag.source)
        {
            restore_preview(doc, &drag);
        }
        uv.error.clear();
    }
}
pub(super) fn handle(
    mut uv: ResMut<UvState>,
    mut editing: ResMut<State>,
    editor: Res<Editor>,
    viewer: Res<Viewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    focus: Res<InputFocus>,
    inputs: Query<(), With<TextInput>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ui::UvCanvas>>,
    model_canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    controls: Query<(&ComputedNode, &UiGlobalTransform), With<Control>>,
    drawer: Res<DrawerMotion>,
) {
    let Some(index) = editor.viewing_model else {
        cancel_drag(&mut uv, &mut editing);
        return;
    };
    if editor.pending.is_some()
        || editor.viewing_image.is_some()
        || editing.mode(Some(index)) != Mode::Uv
    {
        cancel_drag(&mut uv, &mut editing);
        return;
    }
    if uv.relocation.is_some() {
        return;
    }
    if uv.drag.as_ref().is_some_and(|drag| {
        drag.model != index
            || editing.documents.get(&index).is_none_or(|doc| {
                doc.source.path != drag.source || doc.active != drag.snapshot.active
            })
    }) {
        cancel_drag(&mut uv, &mut editing);
    }
    if !window.focused || keys.just_pressed(KeyCode::Escape) {
        cancel_drag(&mut uv, &mut editing);
        uv.pan = None;
        uv.brush_at = None;
        return;
    }
    if focus.get().is_some_and(|e| inputs.contains(e)) {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        if mouse.just_released(MouseButton::Left) {
            let _ = finish_drag(&mut uv, &mut editing, None);
        }
        uv.brush_at = None;
        return;
    };
    let Ok((computed, transform)) = canvas.single() else {
        return;
    };
    let bounds = rect(computed, transform);
    if bounds.size().min_element() < 2. {
        return;
    }
    let over = bounds.contains(cursor)
        && !drawer.covers(cursor, &window)
        && !drawer.resizing
        && !editor.browser_popup
        && !editor.hdri_library_open;
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let control = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !mouse.pressed(MouseButton::Left) {
        uv.brush_at = None;
    }
    if mouse.just_released(MouseButton::Left) {
        let destination = over.then(|| point(&uv, bounds, cursor));
        if let Some(request) = finish_drag(&mut uv, &mut editing, destination) {
            submit(&mut uv, request);
            return;
        }
    }
    if !mouse.pressed(MouseButton::Middle)
        && !(mouse.pressed(MouseButton::Left) && keys.pressed(KeyCode::Space))
    {
        uv.pan = None;
    }
    if over
        && (mouse.just_pressed(MouseButton::Middle)
            || (mouse.just_pressed(MouseButton::Left) && keys.pressed(KeyCode::Space)))
    {
        uv.pan = Some(cursor);
        cancel_drag(&mut uv, &mut editing);
    }
    if let Some(previous) = uv.pan {
        let a = point(&uv, bounds, previous);
        let b = point(&uv, bounds, cursor);
        uv.center += a - b;
        uv.pan = Some(cursor);
        return;
    }
    if over && !control && !alt && scroll.delta.y.abs() > 0.001 {
        let before = point(&uv, bounds, cursor);
        uv.zoom = (uv.zoom * (scroll.delta.y * 0.12).exp()).clamp(0.01, 40.);
        let after = point(&uv, bounds, cursor);
        uv.center += before - after;
    }
    if over && !control {
        if keys.just_pressed(KeyCode::KeyF) {
            uv.fit(editing.documents.get(&index).and_then(Document::active));
        }
        if keys.just_pressed(KeyCode::KeyA) {
            if alt {
                uv.selected.clear();
            } else {
                uv.selected = (0..uv.atlas.count).collect();
            }
        }
        if keys.just_pressed(KeyCode::KeyR) {
            uv.transform(index, &mut editing, std::f32::consts::FRAC_PI_2, 1.);
        }
    }
    let Some(doc) = editing.documents.get_mut(&index) else {
        return;
    };
    if over && mouse.just_pressed(MouseButton::Left) && !alt {
        let at = point(&uv, bounds, cursor);
        let object = doc.active();
        let hit = object.and_then(|o| uv.atlas.hit(&uv.display_geometry(index, o), at));
        if let Some(id) = hit {
            if shift {
                if !uv.selected.insert(id) {
                    uv.selected.remove(&id);
                }
            } else if !uv.selected.contains(&id) {
                uv.selected = BTreeSet::from([id]);
            }
            if !uv.comparing
                && uv.job.is_none()
                && uv.proposal.is_none()
                && !doc.paint_busy
                && !doc.paint_pending
                && uv.selected.contains(&id)
            {
                uv.drag = Some(Drag {
                    model: index,
                    source: doc.source.path.clone(),
                    start: at,
                    snapshot: doc.snapshot(),
                    atlas: uv.atlas.clone(),
                    selected: uv.selected.clone(),
                    changed: false,
                    tile_delta: None,
                });
            }
        } else if !shift {
            uv.selected.clear();
        }
    }
    if mouse.pressed(MouseButton::Left) {
        if let Some(mut drag) = uv.drag.take() {
            let at = point(&uv, bounds, cursor);
            let destination = doc
                .active()
                .and_then(|object| tile_delta(&drag, object, over.then_some(at)));
            if drag.tile_delta != destination {
                drag.tile_delta = destination;
                uv.overlay_revision += 1;
            }
            if destination.is_some() {
                uv.error.clear();
                uv.drag = Some(drag);
                return;
            }
            let delta = at - drag.start;
            if drag.changed || delta.length() > 0.003 / uv.zoom {
                if let Some(object) = doc.active_mut()
                    && let Some(original) = drag.snapshot.objects.iter().find(|o| o.id == object.id)
                {
                    let mut geometry = original.geometry.clone();
                    drag.atlas
                        .transform(&mut geometry, &drag.selected, delta, 0., 1.);
                    if object.preserves_udim_assignment(&geometry) {
                        object.geometry = geometry;
                        doc.revision += 1;
                        drag.changed = true;
                        uv.error.clear();
                    } else {
                        uv.error = "Остров выходит за границы своего тайла UDIM".into();
                    }
                }
            }
            uv.drag = Some(drag);
            return;
        }
    }
    let pressed = mouse.just_pressed(MouseButton::Left);
    if !(pressed || (uv.brush_at.is_some() && mouse.pressed(MouseButton::Left))) {
        return;
    }
    if !over
        && !editor.browser_popup
        && !editor.hdri_library_open
        && !drawer.resizing
        && !alt
        && !keys.pressed(KeyCode::Space)
        && !drawer.covers(cursor, &window)
        && !controls
            .iter()
            .any(|(n, t)| !n.is_empty() && rect(n, t).contains(cursor))
    {
        if let Ok((n, t)) = model_canvas.single() {
            let bounds = rect(n, t);
            if bounds.contains(cursor) {
                if uv.bvh_stamp != Some((index, doc.revision)) {
                    uv.bvh = Bvh::build(doc);
                    uv.bvh_stamp = Some((index, doc.revision));
                }
                if uv.tool == Tool::Brush && uv.job.is_none() && uv.proposal.is_none() {
                    if let Some(object) = doc.active() {
                        paint(
                            &mut uv, doc, object, &viewer, bounds, cursor, shift, pressed,
                        );
                    }
                    return;
                }
                let (origin, direction) = model_input::ray(&viewer, bounds, cursor);
                if let Some((id, face, _)) = uv.bvh.cast(origin, direction, None) {
                    if uv.tool == Tool::Wand && uv.job.is_none() && uv.proposal.is_none() {
                        if doc.active != Some(id) {
                            doc.select_object(id, false);
                        }
                        jobs::start(&mut uv, index, doc, UvAction::Suggest, Some(face));
                    } else if doc.active != Some(id) {
                        doc.select_object(id, false);
                        uv.stamp = None;
                        uv.selected.clear();
                    } else if let Some(&island) = uv.atlas.islands.get(face) {
                        if !shift {
                            uv.selected.clear();
                        }
                        uv.selected.insert(island);
                    }
                }
            }
        }
    }
    uv.brush_at = None;
}

// Sample a screen-space brush through the same BVH used for picking. Rays stop at
// the first visible face, so a stroke on a cheek does not paint the back of the head.
fn paint(
    uv: &mut UvState,
    doc: &Document,
    object: &Object,
    viewer: &Viewer,
    bounds: Rect,
    cursor: Vec2,
    erase: bool,
    pressed: bool,
) {
    let previous = if pressed {
        cursor
    } else {
        uv.brush_at.unwrap_or(cursor)
    };
    if !pressed && uv.brush_at == Some(cursor) {
        return;
    }
    uv.brush_at = Some(cursor);
    let steps = ((cursor - previous).length() / 6.).ceil().clamp(1., 64.) as usize;
    let mut hit = BTreeSet::new();
    for step in 1..=steps {
        let center = previous.lerp(cursor, step as f32 / steps as f32);
        for y in -3i32..=3 {
            for x in -3i32..=3 {
                if x * x + y * y > 9 {
                    continue;
                }
                let point = center + Vec2::new(x as f32, y as f32) * 4.;
                if !bounds.contains(point) {
                    continue;
                }
                let (origin, dir) = model_input::ray(viewer, bounds, point);
                if let Some((id, fi, _)) = uv.bvh.cast(origin, dir, None)
                    && id == object.id
                {
                    hit.insert(fi);
                }
            }
        }
    }
    let mask = uv
        .masks
        .entry((doc.source.path.clone(), object.id))
        .or_default();
    let mut changed = false;
    for fi in hit {
        let f = &object.geometry.faces[fi];
        for tri in object.geometry.triangles(f) {
            let key = face_key(tri.map(|i| f.corners[i].vertex));
            changed |= if erase {
                mask.remove(&key)
            } else {
                mask.insert(key)
            };
        }
    }
    if changed {
        uv.overlay_revision += 1;
    }
}
