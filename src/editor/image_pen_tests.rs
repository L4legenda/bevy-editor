use super::*;
use bevy::{
    ecs::system::RunSystemOnce,
    input::touch::{ForceTouch, TouchInput, TouchPhase},
};

#[test]
fn pen_hover_shows_pencil_coverage_without_mouse_cursor_or_pressure() {
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]))
        .save(fixture.0.join("hover.png"))
        .unwrap();
    let mut app = fixture.app();
    app.add_systems(Update, tool_cursor::cursor.after(update));
    let index = file(&app, "hover.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Tool(
            crate::editor::photo_document::PhotoTool::Pencil,
        )));
    app.update();
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let rect = app
        .world_mut()
        .run_system_once(
            |canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
             window: Single<&Window, With<PrimaryWindow>>| {
                canvas_rect(&canvas, &window)
            },
        )
        .unwrap();
    let image = app
        .world()
        .resource::<ImageViewer>()
        .image_rect(UVec2::splat(64), rect);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    for point in [image.center(), image.center() + Vec2::new(31.0, 17.0)] {
        app.world_mut()
            .write_message(bevy::window::WindowEvent::TouchInput(TouchInput {
                window,
                id: 12,
                phase: TouchPhase::Moved,
                position: point,
                force: None,
            }));
        app.update();
        let viewer = app.world().resource::<ImageViewer>();
        let pixel = image.size() / 64.0;
        let expected = image.min + viewer.edit.snap_point((point - image.min) / pixel) * pixel;
        assert_eq!(viewer.brush_cursor_position, Some(expected));
        assert!(viewer.brush_cursor.is_some());
        assert!(
            !viewer.native_brush_cursor,
            "Pen preview must be rendered on the canvas"
        );
        app.update();
        assert_eq!(
            app.world().resource::<ImageViewer>().brush_cursor_position,
            Some(expected)
        );
    }
    // Moving the mouse takes ownership back, including its actual OS cursor position.
    let point = image.center();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(point));
    app.world_mut()
        .write_message(bevy::window::WindowEvent::CursorMoved(
            bevy::window::CursorMoved {
                window,
                position: point,
                delta: None,
            },
        ));
    app.update();
    assert_eq!(app.world().resource::<ImageViewer>().pen_cursor, None);
}

#[test]
fn pen_pressure_changes_painted_diameter_and_keeps_one_undo_per_contact() {
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]))
        .save(fixture.0.join("pen.png"))
        .unwrap();
    let mut app = fixture.app();
    let index = file(&app, "pen.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    for action in [
        edit::EditAction::Tool(crate::editor::photo_document::PhotoTool::Brush),
        edit::EditAction::Size(20.0),
        edit::EditAction::Hardness(1.0),
        edit::EditAction::Color([1.0, 0.0, 0.0, 1.0]),
    ] {
        app.world_mut().write_message(Action::ImageEdit(action));
    }
    app.update();
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let rect = app
        .world_mut()
        .run_system_once(
            |canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
             window: Single<&Window, With<PrimaryWindow>>| {
                canvas_rect(&canvas, &window)
            },
        )
        .unwrap();
    let center = app
        .world()
        .resource::<ImageViewer>()
        .image_rect(UVec2::splat(64), rect)
        .center();
    let mut counts = Vec::new();
    for force in [0.2, 1.0] {
        app.world_mut()
            .write_message(bevy::window::WindowEvent::TouchInput(TouchInput {
                window,
                id: 9,
                phase: TouchPhase::Started,
                position: center,
                force: Some(ForceTouch::Normalized(force)),
            }));
        app.update();
        app.update(); // Holding the pen without mouse events must not end the stroke.
        app.world_mut()
            .write_message(bevy::window::WindowEvent::TouchInput(TouchInput {
                window,
                id: 9,
                phase: TouchPhase::Ended,
                position: center,
                force: None,
            }));
        app.update();
        let viewer = app.world().resource::<ImageViewer>();
        let pixels = app
            .world()
            .resource::<Assets<Image>>()
            .get(viewer.image.as_ref().unwrap())
            .unwrap()
            .data
            .as_ref()
            .unwrap();
        counts.push(
            pixels
                .chunks_exact(4)
                .filter(|p| p[0] as u16 > p[1] as u16 + 40)
                .count(),
        );
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Undo));
        app.update();
        let viewer = app.world().resource::<ImageViewer>();
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(viewer.image.as_ref().unwrap())
                .unwrap()
                .data
                .as_ref()
                .unwrap()
                .iter()
                .all(|p| *p == 255)
        );
    }
    assert!(
        counts[0] > 0 && counts[1] > counts[0] * 2,
        "Pressure must change diameter: {counts:?}"
    );
}
