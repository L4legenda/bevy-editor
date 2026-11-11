//! Height selection and pointer capture for the drawer's top resize strip.
use super::*;
use bevy::window::{CursorIcon, SystemCursorIcon};

pub(crate) const GRIP_HEIGHT: f32 = 6.0;
const MIN_HEIGHT: f32 = 220.0;

#[derive(Resource, Default)]
pub(crate) struct DrawerSize {
    pub preferred_height: Option<f32>,
    pub dragging: bool,
    pub hover: bool,
    grab_offset: f32,
    cursor_owned: bool,
    previous_cursor: Option<CursorIcon>,
}

impl DrawerSize {
    fn bounds(window_height: f32) -> (f32, f32) {
        let max = (window_height - TOP - FOOT - 80.0).max(0.0);
        (MIN_HEIGHT.min(max), max)
    }

    pub fn height(&self, window_height: f32) -> f32 {
        let (min, max) = Self::bounds(window_height);
        self.preferred_height
            .filter(|height| height.is_finite())
            .unwrap_or(window_height * 0.46)
            .clamp(min, max)
    }

    fn drag_to(&mut self, pointer_y: f32, window_height: f32) {
        let (min, max) = Self::bounds(window_height);
        self.preferred_height =
            Some((window_height - FOOT - pointer_y + self.grab_offset).clamp(min, max));
    }
}

#[derive(Component)]
pub(crate) struct ResizeGrip;

pub(crate) fn resize(
    mut commands: Commands,
    editor: Res<Editor>,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut size: ResMut<DrawerSize>,
    mut motion: ResMut<DrawerMotion>,
    mut focus: Option<ResMut<InputFocus>>,
    mut grips: Query<&mut BackgroundColor, With<ResizeGrip>>,
) {
    let (window_entity, window, cursor_icon) = *window;
    let cursor = window.cursor_position();
    let top = window.height() - FOOT - motion.visible_height;
    let settled =
        editor.pending.is_none() && editor.drawer && motion.progress >= 1.0 && window.focused;

    if editor.pending.is_some()
        || !editor.drawer
        || !window.focused
        || !mouse.pressed(MouseButton::Left)
    {
        size.dragging = false;
    }
    size.hover = settled
        && cursor.is_some_and(|point| {
            point.x >= 0.0
                && point.x < window.width()
                && point.y >= top
                && point.y < top + GRIP_HEIGHT
        });

    if size.hover && mouse.just_pressed(MouseButton::Left) {
        size.dragging = true;
        if let Some(cursor) = cursor {
            size.grab_offset = cursor.y - top;
        }
        if let Some(focus) = focus.as_mut() {
            focus.clear();
        }
    }
    if size.dragging
        && let Some(cursor) = cursor
    {
        size.drag_to(cursor.y, window.height());
    }
    if motion.resizing != size.dragging {
        motion.resizing = size.dragging;
    }

    let active = size.hover || size.dragging;
    let color = if size.dragging {
        ACCENT
    } else if size.hover {
        BLUE
    } else {
        Color::NONE
    };
    for mut background in &mut grips {
        background.set_if_neq(BackgroundColor(color));
    }

    if active {
        if !size.cursor_owned {
            size.previous_cursor = cursor_icon.cloned();
            size.cursor_owned = true;
        }
        let resize_cursor = CursorIcon::from(SystemCursorIcon::NsResize);
        if cursor_icon != Some(&resize_cursor) {
            commands.entity(window_entity).insert(resize_cursor);
        }
    } else if size.cursor_owned {
        size.cursor_owned = false;
        if let Some(previous) = size.previous_cursor.take() {
            commands.entity(window_entity).insert(previous);
        } else {
            commands.entity(window_entity).remove::<CursorIcon>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_height_scales_with_the_window_and_keeps_scene_space() {
        let size = DrawerSize::default();
        assert!((size.height(760.0) - 349.6).abs() < 0.001);
        for window_height in [100.0, 300.0, 620.0, 1080.0, 2160.0] {
            let (min, max) = DrawerSize::bounds(window_height);
            assert!((min..=max).contains(&size.height(window_height)));
        }
    }

    #[test]
    fn dragging_clamps_height_at_both_bounds() {
        let mut size = DrawerSize::default();
        size.drag_to(-1000.0, 760.0);
        assert_eq!(size.height(760.0), 760.0 - TOP - FOOT - 80.0);
        size.drag_to(2000.0, 760.0);
        assert_eq!(size.height(760.0), MIN_HEIGHT);
    }

    #[test]
    fn preferred_height_survives_temporary_window_shrinking_and_reopening() {
        let size = DrawerSize {
            preferred_height: Some(420.0),
            ..default()
        };
        assert_eq!(size.height(760.0), 420.0);
        assert_eq!(size.height(400.0), 400.0 - TOP - FOOT - 80.0);
        assert_eq!(size.height(1080.0), 420.0);
        let mut motion = DrawerMotion::default();
        motion.advance(true, DURATION, size.height(760.0));
        motion.advance(false, DURATION, size.height(760.0));
        assert_eq!(motion.visible_height, 0.0);
        motion.advance(true, DURATION, size.height(760.0));
        assert_eq!(motion.visible_height, 420.0);
        assert_eq!(size.preferred_height, Some(420.0));
    }

    #[test]
    fn resize_system_captures_drags_and_releases_the_mouse() {
        let project = crate::project::EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/project.json"),
        )
        .unwrap();
        let mut editor = Editor::new(project, crate::data::LevelEdits::default());
        editor.drawer = true;
        let size = DrawerSize::default();
        let initial_height = size.height(760.0);
        let mut motion = DrawerMotion::default();
        motion.advance(true, DURATION, initial_height);
        let grip_cursor = Vec2::new(400.0, 760.0 - FOOT - initial_height + 3.0);
        let mut window = Window {
            resolution: (1280, 760).into(),
            focused: true,
            ..default()
        };
        window.set_cursor_position(Some(grip_cursor));

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(editor)
            .insert_resource(size)
            .insert_resource(motion)
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, resize);
        let window_entity = app
            .world_mut()
            .spawn((
                window,
                PrimaryWindow,
                CursorIcon::from(SystemCursorIcon::Crosshair),
            ))
            .id();
        let grip = app
            .world_mut()
            .spawn((ResizeGrip, BackgroundColor(Color::NONE)))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();

        assert!(app.world().resource::<DrawerSize>().dragging);
        assert!(app.world().resource::<DrawerMotion>().resizing);
        assert_eq!(
            app.world().get::<CursorIcon>(window_entity),
            Some(&CursorIcon::from(SystemCursorIcon::NsResize))
        );
        assert_eq!(app.world().get::<BackgroundColor>(grip).unwrap().0, ACCENT);

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear_just_pressed(MouseButton::Left);
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(grip_cursor - Vec2::Y * 100.0));
        app.update();
        assert!(app.world().resource::<DrawerSize>().dragging);
        assert!(
            (app.world().resource::<DrawerSize>().height(760.0) - initial_height - 100.0).abs()
                < 0.001
        );

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 100.0)));
        app.update();
        assert!(!app.world().resource::<DrawerSize>().dragging);
        assert!(!app.world().resource::<DrawerMotion>().resizing);
        assert_eq!(
            app.world().get::<CursorIcon>(window_entity),
            Some(&CursorIcon::from(SystemCursorIcon::Crosshair))
        );
        assert_eq!(
            app.world().get::<BackgroundColor>(grip).unwrap().0,
            Color::NONE
        );
    }
}
