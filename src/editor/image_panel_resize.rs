//! Width and pointer capture for the image editor's inspector.
use super::*;
use bevy::window::SystemCursorIcon;

pub(super) const GRIP: f32 = 6.0;
const DEFAULT_WIDTH: f32 = 248.0;

struct Drag {
    offset: f32,
    original: Option<f32>,
}

#[derive(Default)]
pub(super) struct Panel {
    preferred_width: Option<f32>,
    drag: Option<Drag>,
    hover: bool,
    suppressed: bool,
    cursor_owned: bool,
    previous_cursor: Option<CursorIcon>,
}

impl Panel {
    fn bounds(window_width: f32) -> (f32, f32) {
        let max = (window_width - 54.0 - 320.0).clamp(0.0, 600.0);
        (220.0_f32.min(max), max)
    }

    pub fn width(&self, window_width: f32) -> f32 {
        let (min, max) = Self::bounds(window_width);
        self.preferred_width
            .unwrap_or(DEFAULT_WIDTH)
            .clamp(min, max)
    }

    pub fn capturing(&self) -> bool {
        self.drag.is_some() || self.suppressed
    }

    pub fn blocks_canvas(&self) -> bool {
        self.capturing() || self.hover
    }
}

#[derive(Component)]
pub(in crate::editor) struct ResizeGrip;

pub(super) fn setup(commands: &mut Commands, panel: Entity) {
    let grip = surface(
        commands,
        panel,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: px(GRIP),
            height: percent(100),
            justify_content: JustifyContent::Center,
            ..row(0.0)
        },
        Color::NONE,
    );
    commands.entity(grip).insert((
        ResizeGrip,
        ZIndex(20),
        AccessibleLabel("Изменить ширину панели изображения".into()),
    ));
    let marker = surface(
        commands,
        grip,
        Node {
            width: px(2),
            height: px(28),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        EDGE,
    );
    commands.entity(marker).insert(Pickable::IGNORE);
}

pub(in crate::editor) fn resize(
    mut commands: Commands,
    editor: Res<Editor>,
    mut viewer: ResMut<ImageViewer>,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    drawer: Option<Res<DrawerMotion>>,
    assets: Option<Res<AssetDrag>>,
    mut focus: ResMut<InputFocus>,
    mut grips: Query<(&ComputedNode, &UiGlobalTransform, &mut BackgroundColor), With<ResizeGrip>>,
) {
    let (window_entity, window, current) = *window;
    let allowed = viewer.edit.editing(editor.viewing_image)
        && editor.pending.is_none()
        && !editor.browser_popup
        && !editor.hdri_library_open
        && !viewer.edit.busy()
        && !viewer.edit.menu
        && !viewer.edit.paint_menu
        && viewer.edit.color_picker.is_none()
        && !viewer.edit.layers.menu
        && viewer.edit.layers.form.is_none()
        && window.focused
        && !drawer.as_ref().is_some_and(|drawer| drawer.resizing)
        && !assets.as_ref().is_some_and(|assets| assets.active());
    if !mouse.pressed(MouseButton::Left) && !mouse.just_released(MouseButton::Left) {
        viewer.panel.suppressed = false;
    }
    if !allowed || !mouse.pressed(MouseButton::Left) {
        viewer.panel.drag = None;
    }
    if keys.just_pressed(KeyCode::Escape)
        && let Some(drag) = viewer.panel.drag.take()
    {
        viewer.panel.preferred_width = drag.original;
    }
    let hover = allowed
        && (!viewer.panel.suppressed || mouse.just_released(MouseButton::Left))
        && (!mouse.pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Left))
        && window.cursor_position().is_some_and(|point| {
            if point.y
                >= window.height()
                    - FOOT
                    - drawer.as_ref().map_or(0.0, |drawer| drawer.visible_height)
            {
                return false;
            }
            grips.iter().any(|(node, transform, _)| {
                !node.is_empty()
                    && node.contains_point(*transform, point / node.inverse_scale_factor())
            })
        });
    viewer.panel.hover = hover;
    if hover
        && mouse.just_pressed(MouseButton::Left)
        && let Some(point) = window.cursor_position()
    {
        viewer.panel.drag = Some(Drag {
            offset: point.x - (window.width() - viewer.panel.width(window.width())),
            original: viewer.panel.preferred_width,
        });
        viewer.panel.suppressed = true;
        focus.clear();
    }
    if let Some(drag) = &viewer.panel.drag
        && let Some(point) = window.cursor_position()
    {
        let (min, max) = Panel::bounds(window.width());
        viewer.panel.preferred_width =
            Some((window.width() - point.x + drag.offset).clamp(min, max));
    }
    let active = hover || viewer.panel.drag.is_some();
    for (_, _, mut color) in &mut grips {
        color.set_if_neq(BackgroundColor(if viewer.panel.drag.is_some() {
            ACCENT
        } else if hover {
            BLUE.with_alpha(0.4)
        } else {
            Color::NONE
        }));
    }
    if active {
        if !viewer.panel.cursor_owned {
            // Take over the original cursor, not the brush bitmap that happens to be installed.
            viewer.panel.previous_cursor = if viewer.cursor_owned {
                viewer.cursor_owned = false;
                viewer.previous_cursor.take()
            } else {
                current.cloned()
            };
            viewer.panel.cursor_owned = true;
        }
        let icon = CursorIcon::from(SystemCursorIcon::EwResize);
        if current != Some(&icon) {
            commands.entity(window_entity).insert(icon);
        }
    } else if viewer.panel.cursor_owned {
        viewer.panel.cursor_owned = false;
        if let Some(previous) = viewer.panel.previous_cursor.take() {
            commands.entity(window_entity).insert(previous);
        } else {
            commands.entity(window_entity).remove::<CursorIcon>();
        }
    }
}
