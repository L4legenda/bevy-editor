//! Direct placement for directional and point bake recipes.
use super::*;
use crate::editor::{
    model_document::Object,
    ui::model_viewer::editing::input::{Bvh, XrayGizmos, project, ray, rect},
};

struct Scene {
    bounds: mesh::Bounds,
    pick: Option<Bvh>,
}

struct Source {
    model: usize,
    visibility: Option<u64>,
    objects: Vec<Object>,
}

impl Source {
    fn matches(&self, model: usize, visibility: Option<u64>, objects: &[Object]) -> bool {
        self.model == model
            && self.visibility == visibility
            && self.objects.len() == objects.len()
            && self.objects.iter().zip(objects).all(|(a, b)| {
                a.id == b.id
                    && a.hidden == b.hidden
                    && a.transform == b.transform
                    && a.geometry.shares_storage_with(&b.geometry)
            })
    }
}

struct Drag {
    model: usize,
    revision: u64,
    draft: custom::Recipe,
    rear: bool,
    pointer_offset: Vec2,
}

#[derive(Default, Resource)]
pub(in crate::editor::ui::model_viewer) struct Gizmo {
    source: Option<Source>,
    worker: Option<Task<Option<Scene>>>,
    scene: Option<Scene>,
    drag: Option<Drag>,
}

impl Gizmo {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer::paint) fn target(
        &self,
        viewer: &Viewer,
        recipe: &custom::Recipe,
    ) -> Option<Vec3> {
        self.scene
            .as_ref()
            .map(|scene| handle(viewer, scene.bounds, recipe).0)
    }

    pub(in crate::editor::ui::model_viewer::paint::bake) fn draft_for(
        &self, model: usize, recipe: u64, revision: u64,
    ) -> Option<&custom::Recipe> {
        self.drag.as_ref().filter(|drag| {
            drag.model == model && drag.draft.id == recipe && drag.revision == revision
        }).map(|drag| &drag.draft)
    }

    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer::paint) fn draft(&self) -> Option<&custom::Recipe> {
        self.drag.as_ref().map(|drag| &drag.draft)
    }
}

fn on_sphere(origin: Vec3, direction: Vec3, center: Vec3, radius: f32, rear: bool) -> Vec3 {
    let offset = origin - center;
    let middle = -offset.dot(direction);
    let nearest = offset + direction * middle.max(0.);
    let half_chord = (radius * radius - nearest.length_squared()).max(0.).sqrt();
    let distance = (middle + if rear { half_chord } else { -half_chord }).max(0.);
    (offset + direction * distance).normalize_or_zero()
}

fn handle(viewer: &Viewer, bounds: mesh::Bounds, recipe: &custom::Recipe) -> (Vec3, f32) {
    let center = bounds.center();
    let pose = view_pose(viewer);
    let depth = (center - pose.translation).dot(*pose.forward());
    let reach =
        (view_height(viewer, depth) * 0.36).clamp(bounds.radius() * 0.45, bounds.radius() * 1.35);
    let point = if recipe.kind.has_direction() {
        center + recipe.direction() * reach
    } else {
        recipe.point_at(center, bounds.custom_unit())
    };
    (point, reach)
}

pub(in crate::editor::ui::model_viewer) fn input(
    editor: Res<Editor>,
    mut editing: ResMut<editing::State>,
    mut paint: ResMut<PaintState>,
    mut gizmo: ResMut<Gizmo>,
    viewer: Res<Viewer>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    text_inputs: Query<(), With<bevy::ui_widgets::TextInput>>,
    drawer: Res<DrawerMotion>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<Canvas>>,
    controls: engine::VisibleControls,
    navigation: Option<Res<crate::editor::ui::navigation_gizmo::State>>,
    mut overlay: Gizmos<XrayGizmos>,
) {
    let active = (|| {
        let model = editor.viewing_model?;
        let session = paint.sessions.get(&model)?;
        let doc = editing.documents.get(&model)?;
        let id = session.bake_custom_gizmo?;
        let recipe = doc.custom_bakes.iter().find(|recipe| recipe.id == id)?;
        if !session.custom_gizmo_active()
            || doc.mode != Mode::Paint
            || editor.pending.is_some()
            || editor.viewing_image.is_some()
            || editor.browser_popup
            || editor.hdri_library_open
            || session.bake_busy
            || session.bake_picking
            || !window.focused
            || drawer.resizing
        {
            return None;
        }
        Some((model, doc, session, recipe))
    })();
    let Some((model, doc, session, recipe)) = active else {
        *gizmo = Gizmo::default();
        return;
    };
    if keys.just_pressed(KeyCode::Escape) && focus.get().is_none_or(|e| !text_inputs.contains(e)) {
        if gizmo.drag.take().is_none() {
            let session = paint.sessions.get_mut(&model).unwrap();
            session.bake_custom_gizmo = None;
            session.revision += 1;
        }
        return;
    }
    let point = recipe.kind == custom::Kind::Point;
    let visibility = point.then_some(session.set_appearance.visibility.revision);
    if gizmo
        .source
        .as_ref()
        .is_none_or(|source| !source.matches(model, visibility, &doc.objects))
    {
        let objects = doc.objects.clone();
        let shown = session.set_appearance.visibility.clone();
        gizmo.worker = Some(AsyncComputeTaskPool::get().spawn({
            let objects = objects.clone();
            async move {
                let bounds = mesh::Bounds::of_scene(&objects)?;
                let pick = point
                    .then(|| Bvh::from_objects_where(&objects, |id, slot| shown.shown(id, slot)));
                Some(Scene { bounds, pick })
            }
        }));
        gizmo.source = Some(Source {
            model,
            visibility,
            objects,
        });
        gizmo.scene = None;
        gizmo.drag = None;
    }
    if let Some(scene) = gizmo
        .worker
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        gizmo.scene = scene;
        gizmo.worker = None;
    }
    let Some(viewport) = rect(&canvas) else {
        return;
    };
    let Gizmo { scene, drag, .. } = &mut *gizmo;
    let Some(scene) = scene.as_ref() else { return };
    if drag.as_ref().is_some_and(|drag| {
        drag.model != model || drag.draft.id != recipe.id || drag.revision != doc.revision
    }) {
        *drag = None;
    }
    let revision = doc.revision;
    let (at, reach) = handle(&viewer, scene.bounds, recipe);
    let placement = window.cursor_position().filter(|&cursor| {
        viewport.contains(cursor)
            && !drawer.covers(cursor, &window)
            && !navigation
                .as_ref()
                .is_some_and(|gizmo| gizmo.blocks_input())
            && !keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
            && !mouse.any_pressed([MouseButton::Right, MouseButton::Middle])
            && !controls.contains(cursor)
    });
    if mouse.just_pressed(MouseButton::Left)
        && let Some(cursor) = placement
    {
        let screen = project(&viewer, viewport, at).filter(|at| at.distance(cursor) <= 15.);
        if point || screen.is_some() {
            let (origin, direction) = ray(&viewer, viewport, screen.unwrap_or(cursor));
            *drag = Some(Drag {
                model,
                revision,
                draft: recipe.clone(),
                rear: (at - origin).dot(direction)
                    > (scene.bounds.center() - origin).dot(direction),
                pointer_offset: screen.map_or(Vec2::ZERO, |screen| screen - cursor),
            });
        }
    }
    if let Some(gesture) = drag.as_mut()
        && let Some(cursor) = placement
    {
        let cursor = cursor + gesture.pointer_offset;
        let (origin, direction) = ray(&viewer, viewport, cursor);
        let center = scene.bounds.center();
        if point {
            if let Some((_, _, distance)) = scene
                .pick
                .as_ref()
                .and_then(|pick| pick.cast(origin, direction, None))
            {
                gesture.draft.set_point(
                    origin + direction * distance,
                    center,
                    scene.bounds.custom_unit(),
                );
            }
        } else {
            gesture
                .draft
                .set_direction(on_sphere(origin, direction, center, reach, gesture.rear));
        }
    }
    if !mouse.pressed(MouseButton::Left)
        && let Some(gesture) = drag.take()
        && gesture.draft != *recipe
    {
        let doc = editing.documents.get_mut(&model).unwrap();
        let before = doc.snapshot();
        let recipe = Arc::make_mut(&mut doc.custom_bakes)
            .iter_mut()
            .find(|recipe| recipe.id == gesture.draft.id)
            .unwrap();
        *recipe = gesture.draft;
        doc.checkpoint(before);
        return;
    }
    let recipe = drag.as_ref().map_or(recipe, |drag| &drag.draft);
    let (at, reach) = handle(&viewer, scene.bounds, recipe);
    let pose = view_pose(&viewer);
    let size =
        view_height(&viewer, (at - pose.translation).dot(*pose.forward())) / viewport.height();
    let depth = *pose.forward();
    let color = Color::srgb(1., 0.76, 0.36);
    overlay.circle(
        Isometry3d::new(at, Quat::from_rotation_arc(Vec3::Z, depth)),
        size * 9.,
        color,
    );
    for axis in [*pose.up(), *pose.right()] {
        overlay.line(at - axis * size * 5., at + axis * size * 5., color);
    }
    if point {
        let radius = scene.bounds.custom_unit() * f32::from(recipe.distance) / 100.;
        overlay
            .sphere(
                Isometry3d::from_translation(at),
                radius,
                color.with_alpha(0.35),
            )
            .resolution(48);
    } else {
        let center = scene.bounds.center();
        let direction = recipe.direction();
        let tip = center - direction * reach * 0.25;
        overlay.arrow(at, tip, color).with_tip_length(reach * 0.14);
        let (right, up) = direction.any_orthonormal_pair();
        let angle = f32::from(recipe.spread).to_radians();
        let (sin, cos) = angle.sin_cos();
        for axis in [right, -right, up, -up] {
            overlay.line(
                center,
                center + (direction * cos + axis * sin) * reach,
                color.with_alpha(0.35),
            );
        }
    }
}
