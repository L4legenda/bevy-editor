//! Shared sidebar width and the split between the tree and inspector.
use super::*;
use bevy::{
    input_focus::FocusCause,
    ui::CalculatedClip,
    window::{CursorIcon, SystemCursorIcon},
};

pub(super) const GRIP: f32 = 6.0;
const MIN_WIDTH: f32 = 280.0;
const MIN_VIEW: f32 = 320.0;
const TREE_HEADER: f32 = 37.0;
const MIN_TREE: f32 = 100.0;
const MIN_INSPECTOR: f32 = 160.0;

#[derive(Clone, Copy, PartialEq, Eq, Component)]
pub(in crate::editor) enum Axis {
    Width,
    Height,
}

#[derive(Component)]
pub(in crate::editor) struct Sidebar;
#[derive(Component)]
pub(in crate::editor) struct Inspector;

struct Drag {
    axis: Axis,
    offset: f32,
    width: Option<f32>,
    height: Option<f32>,
}

#[derive(Default)]
pub(in crate::editor) struct SidebarLayout {
    preferred_width: Option<f32>,
    preferred_height: Option<f32>,
    drag: Option<Drag>,
    suppressed: bool,
    cursor_owned: bool,
    previous_cursor: Option<CursorIcon>,
}

impl SidebarLayout {
    fn width_bounds(window_width: f32) -> (f32, f32) {
        let max = (window_width - MIN_VIEW).max(0.0);
        (MIN_WIDTH.min(max), max)
    }

    fn height_bounds(window_height: f32) -> (f32, f32) {
        let max = (window_height - TOP - FOOT - TREE_HEADER - MIN_TREE - GRIP).max(0.0);
        (MIN_INSPECTOR.min(max), max)
    }

    pub fn width(&self, window_width: f32) -> f32 {
        let (min, max) = Self::width_bounds(window_width);
        self.preferred_width
            .filter(|value| value.is_finite())
            .unwrap_or(SIDE)
            .clamp(min, max)
    }

    pub fn inspector_height(&self, window_height: f32, lighting: bool) -> f32 {
        let (min, max) = Self::height_bounds(window_height);
        let default = if lighting {
            ((window_height - TOP - FOOT) * 0.65).clamp(280.0, 520.0)
        } else {
            300.0
        };
        self.preferred_height
            .filter(|value| value.is_finite())
            .unwrap_or(default)
            .clamp(min, max)
    }

    pub fn resizing(&self) -> bool {
        self.drag.is_some() || self.suppressed
    }
}

fn grip(commands: &mut Commands, parent: Entity, axis: Axis) {
    let horizontal = axis == Axis::Height;
    let label = if horizontal {
        "Изменить высоту инспектора"
    } else {
        "Изменить ширину правой панели"
    };
    let grip = commands
        .spawn((
            axis,
            Button,
            Hovered::default(),
            TabIndex(0),
            AccessibleLabel(label.into()),
            BackgroundColor(Color::srgba(0.19, 0.19, 0.22, 0.35)),
            Node {
                position_type: if horizontal {
                    PositionType::Relative
                } else {
                    PositionType::Absolute
                },
                left: px(0),
                top: px(0),
                width: if horizontal { percent(100) } else { px(GRIP) },
                height: if horizontal { px(GRIP) } else { percent(100) },
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            ZIndex(20),
            ChildOf(parent),
        ))
        .id();
    commands.spawn((
        Node {
            width: px(if horizontal { 32 } else { 2 }),
            height: px(if horizontal { 2 } else { 32 }),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(MUTED),
        Pickable::IGNORE,
        ChildOf(grip),
    ));
}

pub(super) fn width_grip(commands: &mut Commands, sidebar: Entity) {
    commands
        .entity(sidebar)
        .insert((Sidebar, AccessibleLabel("Правая панель".into())));
    grip(commands, sidebar, Axis::Width);
}

pub(super) fn height_grip(commands: &mut Commands, sidebar: Entity) {
    grip(commands, sidebar, Axis::Height);
}

pub(super) fn inspector(commands: &mut Commands, inspector: Entity) {
    commands
        .entity(inspector)
        .insert((Inspector, AccessibleLabel("Инспектор свойств".into())));
}

pub(in crate::editor) fn resize(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    drawer: Res<DrawerMotion>,
    mut drawer_size: ResMut<DrawerSize>,
    mut focus: ResMut<InputFocus>,
    lighting: Option<Res<LightingUi>>,
    assets: Option<Res<AssetDrag>>,
    mut grips: Query<(
        Entity,
        &Axis,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
        &mut BackgroundColor,
    )>,
    mut panes: Query<(&mut Node, Has<Sidebar>), Or<(With<Sidebar>, With<Inspector>)>>,
) {
    let (window_entity, window, cursor_icon) = *window;
    let light = matches!(
        editor.selection,
        Some(Selection::Light(_) | Selection::Environment)
    );
    let allowed = editor.pending.is_none()
        && window.focused
        && !drawer.resizing
        && !lighting.as_ref().is_some_and(|ui| ui.blocks_scene_input())
        && !assets.as_ref().is_some_and(|drag| drag.active());
    let layout = &mut editor.sidebar;
    if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
        layout.suppressed = false;
    }
    if !allowed || !mouse.pressed(MouseButton::Left) {
        layout.drag = None;
    }
    if keys.just_pressed(KeyCode::Escape)
        && let Some(drag) = layout.drag.take()
    {
        layout.preferred_width = drag.width;
        layout.preferred_height = drag.height;
        layout.suppressed = mouse.pressed(MouseButton::Left);
    }
    let hover = if allowed && !layout.suppressed {
        window.cursor_position().and_then(|point| {
            // The asset drawer overlays both the sidebar and its splitters.
            if point.y >= window.height() - FOOT - drawer.visible_height {
                return None;
            }
            let point = point * window.scale_factor();
            grips
                .iter()
                .find(|(_, _, node, transform, clip, _)| {
                    !node.is_empty()
                        && node.contains_point(**transform, point)
                        && clip.is_none_or(|clip| clip.contains_point(point))
                })
                .map(|(entity, axis, ..)| (entity, *axis))
        })
    } else {
        None
    };
    if let Some((entity, axis)) = hover
        && mouse.just_pressed(MouseButton::Left)
        && let Some(point) = window.cursor_position()
    {
        let offset = match axis {
            Axis::Width => point.x - (window.width() - layout.width(window.width())),
            Axis::Height => {
                point.y
                    - (window.height()
                        - FOOT
                        - layout.inspector_height(window.height(), light)
                        - GRIP)
            }
        };
        layout.drag = Some(Drag {
            axis,
            offset,
            width: layout.preferred_width,
            height: layout.preferred_height,
        });
        focus.set(entity, FocusCause::Pressed);
    }
    if let Some(drag) = &layout.drag
        && let Some(point) = window.cursor_position()
    {
        match drag.axis {
            Axis::Width => {
                let (min, max) = SidebarLayout::width_bounds(window.width());
                layout.preferred_width =
                    Some((window.width() - point.x + drag.offset).clamp(min, max));
            }
            Axis::Height => {
                let (min, max) = SidebarLayout::height_bounds(window.height());
                layout.preferred_height =
                    Some((window.height() - FOOT - point.y + drag.offset - GRIP).clamp(min, max));
            }
        }
    }
    if allowed && layout.drag.is_none() && !layout.suppressed {
        let step = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            5.0
        } else {
            20.0
        };
        if let Some(entity) = focus.get()
            && let Ok((_, axis, ..)) = grips.get(entity)
        {
            if keys.just_pressed(KeyCode::Home) {
                match axis {
                    Axis::Width => layout.preferred_width = None,
                    Axis::Height => layout.preferred_height = None,
                }
            } else {
                let delta = match axis {
                    Axis::Width => {
                        (i8::from(keys.just_pressed(KeyCode::ArrowLeft))
                            - i8::from(keys.just_pressed(KeyCode::ArrowRight)))
                            as f32
                            * step
                    }
                    Axis::Height => {
                        (i8::from(keys.just_pressed(KeyCode::ArrowUp))
                            - i8::from(keys.just_pressed(KeyCode::ArrowDown)))
                            as f32
                            * step
                    }
                };
                if delta != 0.0 {
                    match axis {
                        Axis::Width => {
                            let (min, max) = SidebarLayout::width_bounds(window.width());
                            layout.preferred_width =
                                Some((layout.width(window.width()) + delta).clamp(min, max));
                        }
                        Axis::Height => {
                            let (min, max) = SidebarLayout::height_bounds(window.height());
                            layout.preferred_height = Some(
                                (layout.inspector_height(window.height(), light) + delta)
                                    .clamp(min, max),
                            );
                        }
                    }
                }
            }
        }
    }
    let active = layout
        .drag
        .as_ref()
        .map(|drag| drag.axis)
        .or(hover.map(|(_, axis)| axis));
    for (entity, axis, _, _, _, mut color) in &mut grips {
        let value = if layout.drag.as_ref().is_some_and(|drag| drag.axis == *axis) {
            ACCENT
        } else if hover.is_some_and(|(_, hovered)| hovered == *axis) || focus.get() == Some(entity)
        {
            BLUE
        } else {
            Color::srgba(0.19, 0.19, 0.22, 0.35)
        };
        color.set_if_neq(BackgroundColor(value));
        if allowed {
            commands.entity(entity).try_remove::<InteractionDisabled>();
        } else {
            commands.entity(entity).try_insert(InteractionDisabled);
            commands.entity(entity).try_remove::<Pressed>();
        }
        commands
            .entity(entity)
            .try_insert(TabIndex(if allowed { 0 } else { -1 }));
    }
    if let Some(axis) = active {
        if !layout.cursor_owned {
            layout.previous_cursor = cursor_icon.cloned();
            layout.cursor_owned = true;
        }
        let icon = CursorIcon::from(match axis {
            Axis::Width => SystemCursorIcon::EwResize,
            Axis::Height => SystemCursorIcon::NsResize,
        });
        if cursor_icon != Some(&icon) {
            commands.entity(window_entity).insert(icon);
        }
    } else if layout.cursor_owned {
        layout.cursor_owned = false;
        if drawer_size.hover {
            // The drawer has already installed its resize cursor this frame.
            // Pass it the original cursor rather than restoring our own icon.
            drawer_size.inherit_cursor(layout.previous_cursor.take());
        } else if let Some(previous) = layout.previous_cursor.take() {
            commands.entity(window_entity).insert(previous);
        } else {
            commands.entity(window_entity).remove::<CursorIcon>();
        }
    }
    for (mut node, sidebar) in &mut panes {
        if sidebar {
            node.width = px(layout.width(window.width()));
        } else {
            node.height = px(layout.inspector_height(window.height(), light));
            node.min_height = px(0);
            node.max_height = Val::Auto;
        }
    }
}

#[cfg(test)]
#[path = "sidebar_resize_tests.rs"]
mod tests;
