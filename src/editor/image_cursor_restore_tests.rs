use super::*;
use bevy::window::{CustomCursor, SystemCursorIcon};

#[derive(Resource, Default)]
struct LateRestore(Option<CursorIcon>);

fn restore_after_ui(
    mut commands: Commands,
    mut restore: ResMut<LateRestore>,
    window: Single<Entity, With<PrimaryWindow>>,
) {
    if let Some(cursor) = restore.0.take() {
        commands.entity(*window).insert(cursor);
    }
}

#[test]
fn photo_cursor_restoration_survives_panel_handoffs_and_editor_exit() {
    for original in [None, Some(CursorIcon::from(SystemCursorIcon::Crosshair))] {
        let fixture = Fixture::new();
        let mut app = fixture.app();
        app.init_resource::<LateRestore>()
            .add_systems(
                Update,
                tool_cursor::cursor.after(crate::editor::handle_actions),
            )
            .add_systems(PostUpdate, restore_after_ui)
            .add_systems(Last, tool_cursor::cleanup);
        let index = file(&app, "b-transparent.png");
        app.world_mut().write_message(Action::ViewImage(index));
        wait_for_image(&mut app);
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Edit));
        wait_for_edit(&mut app);
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap();
        if let Some(original) = &original {
            app.world_mut().entity_mut(window).insert(original.clone());
        }
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(600.0, 350.0)));
        app.update();
        let stale = app.world().get::<CursorIcon>(window).unwrap().clone();
        let CursorIcon::Custom(CustomCursor::Image(old_bitmap)) = &stale else {
            panic!("Expected brush cursor");
        };
        for size in 40..60 {
            app.world_mut()
                .write_message(Action::ImageEdit(edit::EditAction::Size(size as f32)));
            app.update();
        }
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&old_bitmap.handle)
                .is_none(),
            "The stale cursor must be recognised even after cache eviction"
        );
        // A panel takes over while the pointer leaves the canvas. Its resize cursor
        // must survive the image viewer releasing its own cursor in the same frame.
        let resize = CursorIcon::from(SystemCursorIcon::NsResize);
        app.world_mut().entity_mut(window).insert(resize.clone());
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(20.0, 20.0)));
        app.update();
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&resize));
        assert!(!app.world().resource::<ImageViewer>().cursor_owned);
        // Return before the panel releases its cursor. Do not save its temporary
        // resize cursor as the underlying cursor for the rest of the application.
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(600.0, 350.0)));
        app.update();
        assert!(matches!(
            app.world().get::<CursorIcon>(window),
            Some(CursorIcon::Custom(_))
        ));
        app.world_mut().write_message(Action::Open(0));
        app.update();
        let expected = original.unwrap_or_default();
        assert!(app.world().resource::<Editor>().viewing_image.is_none());
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&expected));
        assert!(app.world().resource::<ImageViewer>().brush_cursor.is_none());
        // Window chrome can restore a remembered brush in PostUpdate. Last must
        // replace it before Winit has a chance to display it on the OS window.
        app.world_mut().resource_mut::<LateRestore>().0 = Some(stale);
        app.update();
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&expected));
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().get::<CursorIcon>(window), Some(&expected));
    }
}
