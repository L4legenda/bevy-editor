//! Shared drag threshold, modifiers and overlay for scene/model selection.
use super::*;
use bevy::{input::ButtonInput, ui::UiSystems};

pub(super) const DRAG_THRESHOLD: f32 = 4.0;
pub(super) fn dragged(start: Vec2, end: Vec2) -> bool {
    start.distance(end) >= DRAG_THRESHOLD
}
pub(super) fn overlaps(a: Rect, b: Rect) -> bool {
    a.min.cmple(b.max).all() && b.min.cmple(a.max).all()
}
pub(super) fn projected_bounds(
    points: impl IntoIterator<Item = Vec3>,
    project: impl Fn(Vec3) -> Option<Vec2>,
) -> Option<Rect> {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for p in points {
        if let Some(p) = project(p).filter(|p| p.is_finite()) {
            min = min.min(p);
            max = max.max(p);
        }
    }
    min.is_finite().then_some(Rect::from_corners(min, max))
}
pub(super) fn segment_hits(area: Rect, a: Vec2, b: Vec2) -> bool {
    clip_segment(area, a, b).is_some()
}
pub(super) fn clip_segment(area: Rect, a: Vec2, b: Vec2) -> Option<[Vec2; 2]> {
    let d = b - a;
    let (mut near, mut far) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        if d[axis].abs() < 0.00001 {
            if a[axis] < area.min[axis] || a[axis] > area.max[axis] {
                return None;
            }
        } else {
            let p = (area.min[axis] - a[axis]) / d[axis];
            let q = (area.max[axis] - a[axis]) / d[axis];
            near = near.max(p.min(q));
            far = far.min(p.max(q));
            if near > far {
                return None;
            }
        }
    }
    Some([a + d * near, a + d * far])
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Replace,
    Add,
    Subtract,
}
impl Mode {
    pub(super) fn keys(keys: &ButtonInput<KeyCode>) -> Self {
        if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
            Self::Subtract
        } else if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            Self::Add
        } else {
            Self::Replace
        }
    }
    pub(super) fn apply<T: Ord>(
        &self,
        selected: &mut std::collections::BTreeSet<T>,
        hits: std::collections::BTreeSet<T>,
    ) {
        match self {
            Self::Replace => *selected = hits,
            Self::Add => selected.extend(hits),
            Self::Subtract => selected.retain(|v| !hits.contains(v)),
        }
    }
}

#[derive(Resource)]
pub(super) struct Overlay(Entity);
pub(super) fn setup(mut commands: Commands, interface: Res<ui::Interface>) {
    let entity = commands
        .spawn((
            AccessibleLabel("Рамка выделения".into()),
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                border: UiRect::all(px(1.5)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.72, 1.0, 0.10)),
            BorderColor::all(Color::srgb(0.18, 0.83, 1.0)),
            ZIndex(40),
            Pickable::IGNORE,
            ChildOf(interface.viewport),
        ))
        .id();
    commands.insert_resource(Overlay(entity));
}
pub(super) fn draw(
    editor: Res<Editor>,
    pointer: Res<viewport::PointerState>,
    model: Res<ui::ModelEditingState>,
    window: Single<&Window, With<PrimaryWindow>>,
    drawer: Res<ui::DrawerMotion>,
    overlay: Res<Overlay>,
    mut nodes: Query<&mut Node>,
) {
    let rect = if editor.pending.is_some() || !window.focused {
        None
    } else if editor.viewing_model.is_some() {
        model
            .box_start
            .zip(window.cursor_position())
            .filter(|(a, b)| !model.box_auto || dragged(*a, *b))
            .map(|(a, b)| Rect::from_corners(a, b))
    } else if editor.viewing_image.is_none() {
        pointer.selection_rect()
    } else {
        None
    };
    let Ok(mut node) = nodes.get_mut(overlay.0) else {
        return;
    };
    node.display = Display::None;
    if let Some(rect) = rect {
        let allowed = ui::scene_input_rect(&window, &drawer, &editor.sidebar);
        let min = rect.min.clamp(allowed.min, allowed.max);
        let max = rect.max.clamp(allowed.min, allowed.max);
        node.display = Display::Flex;
        node.left = px(min.x);
        node.top = px(min.y - ui::TOP);
        node.width = px((max.x - min.x).max(0.0));
        node.height = px((max.y - min.y).max(0.0));
    }
}
pub(super) fn cancel(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut pointer: ResMut<viewport::PointerState>,
    mut model: ResMut<ui::ModelEditingState>,
) {
    if keys.just_pressed(KeyCode::Escape) && (pointer.selecting() || model.box_start.is_some()) {
        pointer.cancel_selection();
        model.box_start = None;
        model.box_auto = false;
        model.box_armed = false;
        keys.clear_just_pressed(KeyCode::Escape);
    }
}
pub(super) fn register(app: &mut App) {
    app.add_systems(Startup, setup.after(ui::setup))
        .add_systems(Update, cancel.before(ui::shortcuts))
        .add_systems(PostUpdate, draw.before(UiSystems::Layout));
}
