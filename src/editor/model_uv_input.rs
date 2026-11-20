use super::super::input as model_input;
use super::*;
use bevy::input_focus::InputFocus;
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
        return;
    };
    if editor.pending.is_some()
        || editor.viewing_image.is_some()
        || editing.mode(Some(index)) != Mode::Uv
    {
        return;
    }
    if !window.focused || keys.just_pressed(KeyCode::Escape) {
        if let Some(drag) = uv.drag.take() {
            if let Some(doc) = editing.documents.get_mut(&index) {
                doc.restore(drag.snapshot);
            }
        }
        uv.pan = None;
        uv.brush_at = None;
        return;
    }
    if focus.get().is_some_and(|e| inputs.contains(e)) {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
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
        if let Some(drag) = uv.drag.take() {
            if drag.changed {
                if let Some(doc) = editing.documents.get_mut(&index) {
                    doc.checkpoint(drag.snapshot);
                }
            }
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
        uv.drag = None;
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
        uv.zoom = (uv.zoom * (scroll.delta.y * 0.12).exp()).clamp(0.15, 40.);
        let after = point(&uv, bounds, cursor);
        uv.center += before - after;
    }
    if over && !control {
        if keys.just_pressed(KeyCode::KeyF) {
            uv.center = Vec2::splat(0.5);
            uv.zoom = 1.;
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
                && uv.selected.contains(&id)
            {
                uv.drag = Some(Drag {
                    start: at,
                    snapshot: doc.snapshot(),
                    atlas: uv.atlas.clone(),
                    selected: uv.selected.clone(),
                    changed: false,
                });
            }
        } else if !shift {
            uv.selected.clear();
        }
    }
    if mouse.pressed(MouseButton::Left) {
        if let Some(mut drag) = uv.drag.take() {
            let delta = point(&uv, bounds, cursor) - drag.start;
            if drag.changed || delta.length() > 0.003 / uv.zoom {
                doc.objects = drag.snapshot.objects.clone();
                if let Some(object) = doc.active_mut() {
                    drag.atlas
                        .transform(&mut object.geometry, &drag.selected, delta, 0., 1.);
                }
                doc.revision += 1;
                drag.changed = true;
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
