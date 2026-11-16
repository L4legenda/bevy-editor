//! Shared view-axis navigator for the scene and model workspaces.
use super::*;

const SIZE: f32 = 116.0;
const ARM: f32 = 37.0;
const RADIUS: f32 = 11.0;
const INSET: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum Workspace {
    Scene(Room),
    Model(usize),
}

pub(in crate::editor) struct Frame {
    pub workspace: Workspace,
    pub view: Transform,
    pub pivot: Vec3,
    pub rect: Rect,
    pub orthographic: bool,
}

struct Gesture {
    start: Vec2,
    previous: Vec2,
    axis: Option<usize>,
    dragged: bool,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct State {
    workspace: Option<Workspace>,
    rotation: Quat,
    gesture: Option<Gesture>,
    hover: Option<usize>,
    blocked: bool,
}

fn direction(index: usize) -> Vec3 {
    viewport::AXES[index % 3] * if index < 3 { 1.0 } else { -1.0 }
}

fn projected(rotation: Quat, index: usize) -> Vec3 {
    let local = rotation.inverse() * direction(index);
    Vec3::new(local.x * ARM, -local.y * ARM, local.z)
}

pub(in crate::editor) fn rect(viewport: Rect) -> Rect {
    let max = Vec2::new(viewport.max.x - INSET, viewport.min.y + INSET + SIZE);
    Rect::from_corners(max - Vec2::splat(SIZE), max)
}

fn hit(rotation: Quat, offset: Vec2) -> Option<usize> {
    (0..6)
        .filter(|&index| offset.distance(projected(rotation, index).truncate()) <= RADIUS + 2.0)
        .max_by(|&a, &b| {
            projected(rotation, a)
                .z
                .total_cmp(&projected(rotation, b).z)
        })
}

fn orient(view: &mut Transform, pivot: Vec3, direction: Vec3) {
    let distance = view.translation.distance(pivot).max(0.001);
    let up = if direction.y.abs() > 0.999 {
        Vec3::NEG_Z * direction.y.signum()
    } else {
        Vec3::Y
    };
    *view = Transform::from_translation(pivot + direction * distance).looking_at(pivot, up);
}

impl State {
    pub fn blocks_input(&self) -> bool {
        self.blocked
    }

    pub fn drive(
        &mut self,
        frame: &mut Frame,
        window: &Window,
        mouse: &ButtonInput<MouseButton>,
        keys: &ButtonInput<KeyCode>,
        enabled: bool,
    ) -> bool {
        if self.workspace != Some(frame.workspace) {
            self.gesture = None;
        }
        self.workspace = Some(frame.workspace);
        self.rotation = frame.view.rotation;
        self.blocked = false;
        self.hover = None;
        if !enabled || !window.focused || keys.just_pressed(KeyCode::Escape) {
            self.gesture = None;
            return false;
        }
        let Some(cursor) = window.cursor_position() else {
            self.gesture = None;
            return false;
        };
        let bounds = rect(frame.rect);
        let inside = bounds.contains(cursor);
        self.blocked = inside || self.gesture.is_some();
        if inside {
            self.hover = hit(frame.view.rotation, cursor - bounds.center());
        }
        if inside && mouse.just_pressed(MouseButton::Left) {
            self.gesture = Some(Gesture {
                start: cursor,
                previous: cursor,
                axis: self.hover,
                dragged: false,
            });
        }
        if let Some(gesture) = self.gesture.as_mut() {
            gesture.dragged |= cursor.distance(gesture.start) > 3.0;
            if mouse.pressed(MouseButton::Left) {
                if gesture.dragged {
                    frame.orthographic = false;
                    let delta = cursor - gesture.previous;
                    let back = *frame.view.back();
                    let yaw = back.x.atan2(back.z) - delta.x * 0.006;
                    let pitch =
                        (back.y.clamp(-1.0, 1.0).asin() + delta.y * 0.006).clamp(-1.53, 1.53);
                    let direction = Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0) * Vec3::Z;
                    orient(&mut frame.view, frame.pivot, direction);
                }
                gesture.previous = cursor;
            } else {
                let gesture = self.gesture.take().unwrap();
                if !gesture.dragged
                    && let Some(axis) = gesture.axis
                {
                    let mut axis = direction(axis);
                    if frame.view.back().dot(axis) > 0.999 {
                        axis = -axis;
                    }
                    orient(&mut frame.view, frame.pivot, axis);
                    frame.orthographic = true;
                }
            }
        }
        self.rotation = frame.view.rotation;
        self.blocked
    }
}

#[derive(Resource)]
struct GizmoUi {
    root: Entity,
    tips: [Entity; 6],
    arms: [Entity; 3],
}

pub(in crate::editor) struct NavigationGizmoPlugin;

impl Plugin for NavigationGizmoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<State>()
            .add_systems(Startup, setup.after(super::setup))
            .add_systems(Update, refresh.after(crate::editor::viewport::navigate));
    }
}

fn setup(mut commands: Commands, interface: Res<Interface>) {
    let root = node(
        &mut commands,
        interface.viewport,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            right: px(INSET),
            top: px(INSET),
            width: px(SIZE),
            height: px(SIZE),
            border_radius: BorderRadius::MAX,
            ..default()
        },
    );
    commands.entity(root).insert((
        ZIndex(40),
        BackgroundColor(Color::NONE),
        AccessibleLabel("Навигатор осей · нажмите ось или перетащите для вращения вида".into()),
    ));
    let arms = std::array::from_fn(|_| {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    height: px(2),
                    ..default()
                },
                UiTransform::default(),
                BackgroundColor(Color::NONE),
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .id()
    });
    let tips = std::array::from_fn(|index| {
        let tip = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: px(RADIUS * 2.0),
                    height: px(RADIUS * 2.0),
                    border: UiRect::all(px(1.5)),
                    border_radius: BorderRadius::MAX,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                BorderColor::all(Color::NONE),
                AccessibleLabel(format!(
                    "Вид по оси {}{}",
                    if index < 3 { "+" } else { "−" },
                    ["X", "Y", "Z"][index % 3]
                )),
                ChildOf(root),
            ))
            .id();
        if index < 3 {
            text(
                &mut commands,
                tip,
                &interface.font,
                ["X", "Y", "Z"][index],
                12.0,
                Color::srgb(0.06, 0.07, 0.09),
            );
        }
        tip
    });
    commands.insert_resource(GizmoUi { root, tips, arms });
}

fn refresh(
    mut commands: Commands,
    state: Res<State>,
    ui: Res<GizmoUi>,
    editor: Res<Editor>,
    interface: Res<Interface>,
    bounds: Query<(&ComputedNode, &UiGlobalTransform)>,
    model_canvas: Query<Entity, With<crate::editor::ui::model_viewer::Canvas>>,
    mut nodes: Query<&mut Node>,
    mut backgrounds: Query<&mut BackgroundColor>,
    mut borders: Query<&mut BorderColor>,
    mut transforms: Query<&mut UiTransform>,
) {
    let workspace = editor
        .viewing_model
        .map(Workspace::Model)
        .or_else(|| editor.has_scene().then_some(Workspace::Scene(editor.room)));
    let shown = workspace.is_some()
        && workspace == state.workspace
        && editor.viewing_image.is_none()
        && editor.pending.is_none()
        && !editor.hdri_library_open
        && !editor.browser_popup
        && !editor.scene_object_popup
        && (editor.viewing_model.is_some() || !editor.pilot);
    let shown = shown
        && (editor.viewing_model.is_none()
            || model_canvas
                .single()
                .ok()
                .and_then(|canvas| bounds.get(canvas).ok())
                .is_some_and(|(node, _)| node.size().min_element() > 0.));
    if let Ok(mut node) = nodes.get_mut(ui.root) {
        node.display = if shown { Display::Flex } else { Display::None };
        node.right = px(INSET);
        node.top = px(INSET);
        if editor.viewing_model.is_some()
            && let Ok(canvas) = model_canvas.single()
            && let (Ok((parent, p)), Ok((child, c))) =
                (bounds.get(interface.viewport), bounds.get(canvas))
        {
            let parent_right = p.affine().translation.x + parent.size().x * 0.5;
            let child_right = c.affine().translation.x + child.size().x * 0.5;
            node.right = px(INSET + (parent_right - child_right) * parent.inverse_scale_factor());
            let parent_top = p.affine().translation.y - parent.size().y * 0.5;
            let child_top = c.affine().translation.y - child.size().y * 0.5;
            node.top = px(INSET + (child_top - parent_top) * parent.inverse_scale_factor());
        }
    }
    if !shown {
        return;
    }
    if let Ok(mut background) = backgrounds.get_mut(ui.root) {
        background.set_if_neq(BackgroundColor(if state.blocked {
            Color::srgba(0.16, 0.17, 0.20, 0.38)
        } else {
            Color::NONE
        }));
    }
    for index in 0..6 {
        let p = projected(state.rotation, index);
        let color = viewport::AXIS_COLORS[index % 3];
        let hovered = state.hover == Some(index);
        let tip = ui.tips[index];
        if let Ok(mut node) = nodes.get_mut(tip) {
            node.left = px(SIZE * 0.5 + p.x - RADIUS);
            node.top = px(SIZE * 0.5 + p.y - RADIUS);
        }
        commands
            .entity(tip)
            .insert(ZIndex(10 + ((p.z + 1.0) * 10.0) as i32));
        if let Ok(mut background) = backgrounds.get_mut(tip) {
            background.set_if_neq(BackgroundColor(if hovered {
                Color::WHITE
            } else if index < 3 {
                color
            } else {
                color.with_alpha(0.18)
            }));
        }
        if let Ok(mut border) = borders.get_mut(tip) {
            border.set_if_neq(BorderColor::all(if hovered { Color::WHITE } else { color }));
        }
        if index < 3 {
            let arm = ui.arms[index];
            let offset = p.truncate();
            if let Ok(mut node) = nodes.get_mut(arm) {
                node.left = px(SIZE * 0.5 + p.x * 0.5 - offset.length() * 0.5);
                node.top = px(SIZE * 0.5 + p.y * 0.5 - 1.0);
                node.width = px(offset.length());
            }
            if let Ok(mut transform) = transforms.get_mut(arm) {
                transform.set_if_neq(UiTransform::from_rotation(Rot2::radians(p.y.atan2(p.x))));
            }
            if let Ok(mut background) = backgrounds.get_mut(arm) {
                background.set_if_neq(BackgroundColor(color.with_alpha(0.9)));
            }
        }
    }
}

#[cfg(test)]
#[path = "navigation_gizmo_tests.rs"]
mod tests;
