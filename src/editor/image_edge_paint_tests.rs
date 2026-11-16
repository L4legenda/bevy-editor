use super::*;
use crate::editor::photo_document::{self, PhotoTool};
use bevy::ecs::system::RunSystemOnce;

fn data(app: &App) -> Vec<u8> {
    let viewer = app.world().resource::<ImageViewer>();
    app.world()
        .resource::<Assets<Image>>()
        .get(viewer.image.as_ref().unwrap())
        .unwrap()
        .data
        .clone()
        .unwrap()
}

fn point_on_screen(app: &mut App, point: Vec2) -> Vec2 {
    let rect = app
        .world_mut()
        .run_system_once(
            |canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
             window: Single<&Window, With<PrimaryWindow>>| {
                canvas_rect(&canvas, &window)
            },
        )
        .unwrap();
    let viewer = app.world().resource::<ImageViewer>();
    let dimensions = viewer.dimensions.unwrap();
    let image = viewer.image_rect(dimensions, rect);
    let cursor = image.min + point / dimensions.as_vec2() * image.size();
    assert!(
        rect.contains(cursor),
        "The test stroke must start inside the workspace: {point:?} -> {cursor:?}, {rect:?}"
    );
    cursor
}

fn mouse(app: &mut App, window: Entity, down: bool, cursor: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(cursor));
    if down {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
    } else {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
    }
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

#[test]
fn captured_paint_survives_workspace_and_window_exits_and_releases_without_a_cursor() {
    use edit::{EditAction, EraserMode};
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]))
        .save(fixture.0.join("capture.png"))
        .unwrap();
    let mut app = fixture.app();
    let index = file(&app, "capture.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Color([1.0, 0.0, 0.0, 1.0])));
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
    let screen = |x, y| image.min + Vec2::new(x, y);
    let positions = [
        screen(12.0, 12.0),
        Vec2::new(rect.max.x + 20.0, screen(12.0, 12.0).y),
        Vec2::new(rect.max.x + 20.0, screen(48.0, 48.0).y),
        screen(48.0, 48.0),
        Vec2::new(screen(48.0, 48.0).x, rect.min.y - 20.0),
        Vec2::new(screen(16.0, 32.0).x, rect.min.y - 20.0),
        screen(16.0, 32.0),
        Vec2::new(rect.min.x - 20.0, screen(16.0, 32.0).y),
        Vec2::new(rect.min.x - 20.0, screen(32.0, 16.0).y),
        screen(32.0, 16.0),
        Vec2::new(screen(32.0, 16.0).x, rect.max.y + 20.0),
        Vec2::new(screen(48.0, 32.0).x, rect.max.y + 20.0),
        screen(48.0, 32.0),
    ];
    for (tool, mode) in [
        (PhotoTool::Brush, EraserMode::Brush),
        (PhotoTool::Pencil, EraserMode::Brush),
        (PhotoTool::Eraser, EraserMode::Brush),
        (PhotoTool::Eraser, EraserMode::Pixel),
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(tool)));
        if tool == PhotoTool::Eraser {
            app.world_mut()
                .write_message(Action::ImageEdit(EditAction::EraserMode(mode)));
        }
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Size(8.0)));
        app.update();
        let mut expected = {
            let viewer = app.world().resource::<ImageViewer>();
            viewer.edit.document(viewer.tab).unwrap().document.clone()
        };
        let points = positions.map(|p| {
            let p = app
                .world()
                .resource::<ImageViewer>()
                .edit
                .snap_point(p - image.min);
            photocraft_paint::StrokePoint::new(p.x as f64, p.y as f64, 1.0)
        });
        photocraft_paint::render_stroke_clipped(
            expected.layers[0].surface_mut().unwrap(),
            &photocraft_paint::BrushSettings {
                size: 8.0,
                color: [1.0, 0.0, 0.0, 1.0],
                erase: tool == PhotoTool::Eraser,
                aliased: tool == PhotoTool::Pencil || mode == EraserMode::Pixel,
                ..Default::default()
            },
            &points,
            None,
            false,
            1.0,
            Some(photocraft_geom::Rect::new(0, 0, 64, 64)),
        );
        mouse(&mut app, window, true, positions[0]);
        for position in positions.into_iter().skip(1) {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(position));
            app.world_mut()
                .write_message(bevy::window::WindowEvent::CursorMoved(
                    bevy::window::CursorMoved {
                        window,
                        position,
                        delta: None,
                    },
                ));
            app.update();
            // An absent cursor with no new events must not finish a held stroke.
            app.update();
        }
        mouse(&mut app, window, false, *positions.last().unwrap());
        let stroke = data(&app);
        assert_eq!(
            stroke,
            photo_document::composite(&expected).pixels,
            "{tool:?} {mode:?} lost part of the trajectory across a workspace boundary"
        );
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Undo));
        app.update();
        assert!(
            data(&app).iter().all(|v| *v == 255),
            "The whole captured stroke is one undo step"
        );
        {
            let viewer = app.world().resource::<ImageViewer>();
            assert!(!viewer.edit.document(viewer.tab).unwrap().can_undo());
        }
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Redo));
        app.update();
        assert_eq!(data(&app), stroke);
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Undo));
        app.update();

        // Releasing outside the OS window must end the stroke, even after
        // several frames with no position. The very next press starts a new one.
        mouse(&mut app, window, true, positions[0]);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(None);
        app.update();
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        let first = data(&app);
        mouse(&mut app, window, true, screen(48.0, 48.0));
        mouse(&mut app, window, false, screen(48.0, 48.0));
        assert_ne!(
            data(&app),
            first,
            "The next press must not inherit a stale capture"
        );
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Undo));
        app.update();
        assert_eq!(data(&app), first);
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Undo));
        app.update();
        assert!(data(&app).iter().all(|v| *v == 255));
    }
}

#[test]
fn outside_press_keeps_one_stroke_until_release_and_empty_drags_preserve_redo() {
    use edit::{EditAction, EraserMode};
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]))
        .save(fixture.0.join("outside.png"))
        .unwrap();
    let mut app = fixture.app();
    let index = file(&app, "outside.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Color([1.0, 0.0, 0.0, 1.0])));
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let move_held = |app: &mut App, point: Vec2| {
        let cursor = point_on_screen(app, point);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(cursor));
        app.update();
    };
    for (tool, mode) in [
        (PhotoTool::Brush, EraserMode::Brush),
        (PhotoTool::Pencil, EraserMode::Brush),
        (PhotoTool::Eraser, EraserMode::Brush),
        (PhotoTool::Eraser, EraserMode::Pixel),
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(tool)));
        if tool == PhotoTool::Eraser {
            app.world_mut()
                .write_message(Action::ImageEdit(EditAction::EraserMode(mode)));
        }
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Size(8.0)));
        app.update();
        let outside = point_on_screen(&mut app, Vec2::new(-40.0, 16.0));
        mouse(&mut app, window, true, outside);
        move_held(&mut app, Vec2::new(-20.0, 16.0));
        {
            let viewer = app.world().resource::<ImageViewer>();
            let doc = viewer.edit.document(viewer.tab).unwrap();
            assert!(
                !doc.dirty() && !doc.can_undo(),
                "A press on the pasteboard must not create history"
            );
        }
        move_held(&mut app, Vec2::new(12.0, 16.0));
        let live = data(&app);
        for x in [0, 8] {
            let pixel = &live[(16 * 64 + x) * 4..][..4];
            if tool == PhotoTool::Eraser {
                assert_eq!(pixel[3], 0, "{tool:?} {mode:?} did not enter the image");
            } else {
                assert_eq!(pixel, &[255, 0, 0, 255], "{tool:?} did not enter the image");
            }
        }
        move_held(&mut app, Vec2::new(104.0, 16.0));
        move_held(&mut app, Vec2::new(20.0, 40.0));
        let end = point_on_screen(&mut app, Vec2::new(20.0, 40.0));
        mouse(&mut app, window, false, end);
        let stroke = data(&app);
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Undo));
        app.update();
        assert!(
            data(&app).iter().all(|v| *v == 255),
            "One undo must remove the whole outside-to-inside stroke"
        );
        {
            let viewer = app.world().resource::<ImageViewer>();
            assert!(!viewer.edit.document(viewer.tab).unwrap().can_undo());
        }
        // Neither an empty pasteboard drag nor dragging in from a toolbar press
        // may clear Redo or create a new undo step.
        mouse(&mut app, window, true, outside);
        move_held(&mut app, Vec2::new(-30.0, 32.0));
        mouse(&mut app, window, false, outside);
        mouse(&mut app, window, true, Vec2::new(20.0, 20.0));
        move_held(&mut app, Vec2::new(20.0, 20.0));
        let inside = point_on_screen(&mut app, Vec2::new(20.0, 20.0));
        mouse(&mut app, window, false, inside);
        assert!(data(&app).iter().all(|v| *v == 255));
        {
            let viewer = app.world().resource::<ImageViewer>();
            let doc = viewer.edit.document(viewer.tab).unwrap();
            assert!(!doc.dirty() && !doc.can_undo() && doc.can_redo());
        }
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Redo));
        app.update();
        assert_eq!(data(&app), stroke);
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Undo));
        app.update();
    }

    // Preserve the real approach angle even when the press is far from the
    // image. Clamping coordinates to a margin bends this diagonal at its edge.
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(0.5);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Tool(PhotoTool::Pencil)));
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Size(1.0)));
    app.update();
    let endpoints = [Vec2::new(-400.0, -40.0), Vec2::new(464.0, 104.0)];
    let mut expected = {
        let viewer = app.world().resource::<ImageViewer>();
        viewer.edit.document(viewer.tab).unwrap().document.clone()
    };
    let points = endpoints.map(|p| {
        let p = app.world().resource::<ImageViewer>().edit.snap_point(p);
        photocraft_paint::StrokePoint::new(p.x as f64, p.y as f64, 1.0)
    });
    photocraft_paint::render_stroke_clipped(
        expected.layers[0].surface_mut().unwrap(),
        &photocraft_paint::BrushSettings {
            size: 1.0,
            color: [1.0, 0.0, 0.0, 1.0],
            aliased: true,
            ..Default::default()
        },
        &points,
        None,
        false,
        1.0,
        Some(photocraft_geom::Rect::new(0, 0, 64, 64)),
    );
    let start = point_on_screen(&mut app, endpoints[0]);
    mouse(&mut app, window, true, start);
    move_held(&mut app, endpoints[1]);
    let end = point_on_screen(&mut app, endpoints[1]);
    mouse(&mut app, window, false, end);
    let expected = photo_document::composite(&expected).pixels;
    assert!(expected.chunks_exact(4).any(|p| p == [255, 0, 0, 255]));
    assert_eq!(
        data(&app),
        expected,
        "A distant press must keep the original trajectory on entering the image"
    );
}

#[test]
fn edge_paint_accepts_overlapping_tips_at_all_edges_and_keeps_other_tools_inside() {
    use edit::{EditAction, EraserMode};
    let fixture = Fixture::new();
    let source = fixture.0.join("edge.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]))
        .save(&source)
        .unwrap();
    let original_file = std::fs::read(&source).unwrap();
    let mut app = fixture.app();
    let index = file(&app, "edge.png");
    app.world_mut().write_message(Action::ViewImage(index));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(EditAction::Color([1.0, 0.0, 0.0, 1.0])));
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let cases = [
        (Vec2::new(-4.0, 32.0), (0, 32)),
        (Vec2::new(68.0, 32.0), (63, 32)),
        (Vec2::new(32.0, -4.0), (32, 0)),
        (Vec2::new(32.0, 68.0), (32, 63)),
        (Vec2::new(-4.0, -4.0), (0, 0)),
        (Vec2::new(68.0, 68.0), (63, 63)),
        (Vec2::new(68.0, -4.0), (63, 0)),
        (Vec2::new(-4.0, 68.0), (0, 63)),
    ];
    for (tool, mode) in [
        (PhotoTool::Brush, EraserMode::Brush),
        (PhotoTool::Eraser, EraserMode::Brush),
        (PhotoTool::Pencil, EraserMode::Brush),
        (PhotoTool::Eraser, EraserMode::Pixel),
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(tool)));
        if tool == PhotoTool::Eraser {
            app.world_mut()
                .write_message(Action::ImageEdit(EditAction::EraserMode(mode)));
        }
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Size(16.0)));
        app.update();
        for scale in [1.0, 2.0] {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set_scale_factor_override(Some(scale));
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set(1280.0, 760.0);
            for zoom in [0.5, 1.0, 8.0] {
                app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(zoom);
                app.update();
                for (point, (x, y)) in cases {
                    let cursor = point_on_screen(&mut app, point);
                    mouse(&mut app, window, true, cursor);
                    let live = data(&app);
                    let pixel = &live[(y * 64 + x) * 4..][..4];
                    if tool == PhotoTool::Eraser {
                        assert!(
                            pixel[3] < 20,
                            "No edge erasure: {mode:?} {point:?} zoom={zoom} dpi={scale} {pixel:?}"
                        );
                    } else {
                        assert!(
                            pixel[1] < 30 && pixel[2] < 30,
                            "No edge paint: {tool:?} {point:?} zoom={zoom} dpi={scale} {pixel:?}"
                        );
                    }
                    assert_eq!(&live[(32 * 64 + 32) * 4..][..4], &[255; 4]);
                    {
                        let viewer = app.world().resource::<ImageViewer>();
                        let doc = viewer.edit.document(viewer.tab).unwrap();
                        assert_eq!(doc.size(), photocraft_geom::Size::new(64, 64));
                        assert_eq!(live.len(), 64 * 64 * 4);
                    }
                    mouse(&mut app, window, false, cursor);
                    assert_eq!(data(&app), live, "Release must not shift the edge stroke");
                    app.world_mut()
                        .write_message(Action::ImageEdit(EditAction::Undo));
                    app.update();
                    assert!(data(&app).iter().all(|v| *v == 255));
                    app.world_mut()
                        .write_message(Action::ImageEdit(EditAction::Redo));
                    app.update();
                    assert_eq!(data(&app), live);
                    app.world_mut()
                        .write_message(Action::ImageEdit(EditAction::Undo));
                    app.update();
                }
            }
        }
    }
    // Small pixel tools use the snapped square footprint, including a centre
    // just outside the document that snaps onto its boundary.
    for tool in [PhotoTool::Pencil, PhotoTool::Eraser] {
        for (size, point) in [(2.0, Vec2::new(-0.1, 32.0)), (3.0, Vec2::new(-0.1, -0.1))] {
            app.world_mut()
                .write_message(Action::ImageEdit(EditAction::Tool(tool)));
            if tool == PhotoTool::Eraser {
                app.world_mut()
                    .write_message(Action::ImageEdit(EditAction::EraserMode(EraserMode::Pixel)));
            }
            app.world_mut()
                .write_message(Action::ImageEdit(EditAction::Size(size)));
            app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
            app.update();
            let cursor = point_on_screen(&mut app, point);
            mouse(&mut app, window, true, cursor);
            let live = data(&app);
            let y = if point.y > 0.0 { 32 } else { 0 };
            let pixel = &live[y * 64 * 4..][..4];
            assert_eq!(pixel[3], if tool == PhotoTool::Eraser { 0 } else { 255 });
            if tool == PhotoTool::Pencil {
                assert_eq!(pixel, &[255, 0, 0, 255]);
            }
            mouse(&mut app, window, false, cursor);
            app.world_mut()
                .write_message(Action::ImageEdit(EditAction::Undo));
            app.update();
            assert!(data(&app).iter().all(|v| *v == 255));
        }
    }
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
    for tool in [
        PhotoTool::Brush,
        PhotoTool::Eraser,
        PhotoTool::Pencil,
        PhotoTool::Eyedropper,
        PhotoTool::Fill,
        PhotoTool::Move,
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Tool(tool)));
        app.world_mut()
            .write_message(Action::ImageEdit(EditAction::Size(16.0)));
        app.update();
        let point = if matches!(
            tool,
            PhotoTool::Brush | PhotoTool::Eraser | PhotoTool::Pencil
        ) {
            Vec2::new(-30.0, 32.0)
        } else {
            Vec2::new(-4.0, 32.0)
        };
        let cursor = point_on_screen(&mut app, point);
        mouse(&mut app, window, true, cursor);
        mouse(&mut app, window, false, cursor);
        let viewer = app.world().resource::<ImageViewer>();
        let doc = viewer.edit.document(viewer.tab).unwrap();
        assert!(
            !doc.dirty() && !doc.can_undo(),
            "Outside {tool:?} must not change pixels or history"
        );
        assert!(data(&app).iter().all(|v| *v == 255));
        assert_eq!(viewer.edit.color(), [1.0, 0.0, 0.0, 1.0]);
    }
    assert_eq!(std::fs::read(source).unwrap(), original_file);
}
