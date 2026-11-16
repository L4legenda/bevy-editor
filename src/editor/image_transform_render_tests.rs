use super::*;

fn pending(app: &App) -> bool {
    app.world()
        .resource::<Assets<CanvasMaterial>>()
        .get(&super::super::super::canvas_material::HANDLE)
        .unwrap()
        .transform_style
        .x
        > 0.5
}

#[test]
#[ignore = "requires GPU; profiles a 2048px raster drag and verifies cached textures across successive gestures"]
fn gpu_large_image_transform_reuses_pixels_and_textures_during_drag() {
    use crate::editor::selection_outline::render_tests::{capture_when, frame};
    use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
    use bevy::input::ButtonState;
    use std::time::Instant;
    let fixture = Fixture::new();
    let path = fixture.0.join("large-transform.png");
    image::RgbaImage::from_fn(2048, 2048, |x, y| {
        image::Rgba([
            (30 + x * 190 / 2048) as u8,
            (40 + y * 150 / 2048) as u8,
            if x < 1024 { 220 } else { 90 },
            255,
        ])
    })
    .save(&path)
    .unwrap();

    let mut native = crate::editor::photo_document::PhotoDocument::load(&path).unwrap();
    let source_frame = native.transform_frame().unwrap();
    let mut edit = native.begin_move();
    let mut native_ms = vec![];
    for offset in [30.0, 40.0, 50.0] {
        let quad = transformed_quad(
            frame_quad(source_frame),
            4,
            Vec2::splat(2048.0),
            Vec2::splat(2048.0 + offset),
            TransformMode::Free,
            true,
            false,
            false,
            false,
        );
        let time = Instant::now();
        native
            .preview_transform(&mut edit, source_frame, quad.map(|p| p.to_array()), false)
            .unwrap();
        std::hint::black_box(native.composite());
        native_ms.push(time.elapsed().as_secs_f64() * 1000.0);
    }

    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let file_id = file(&app, "large-transform.png");
    app.world_mut().write_message(Action::ViewImage(file_id));
    for _ in 0..40 {
        frame(&mut app);
    }
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..240 {
        frame(&mut app);
        if app.world().resource::<ImageViewer>().edit.editing(Some(id)) {
            break;
        }
    }
    {
        let mut viewer = app.world_mut().resource_mut::<ImageViewer>();
        viewer.zoom = Zoom::Scale(0.2);
        viewer.pan = Vec2::ZERO;
    }
    action(&mut app, Op::Transform);
    for _ in 0..6 {
        frame(&mut app);
    }
    let original = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .composite()
        .pixels;
    let mut drag_ms = vec![];
    let mut planes = None;
    for gesture in 0..2 {
        let viewer = app.world().resource::<ImageViewer>();
        let quad = viewer.edit.selection.transform.as_ref().unwrap().quad;
        let p = point(&mut app, quad[2]);
        move_pointer(&mut app, window, p);
        mouse(&mut app, window, ButtonState::Pressed);
        let revision = app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .revision;
        for i in 1..=24 {
            let p = point(&mut app, quad[2] + Vec2::splat(i as f32 * 2.0));
            let time = Instant::now();
            move_pointer(&mut app, window, p);
            drag_ms.push(time.elapsed().as_secs_f64() * 1000.0);
            let material = app
                .world()
                .resource::<Assets<CanvasMaterial>>()
                .get(&super::super::super::canvas_material::HANDLE)
                .unwrap();
            assert_eq!(material.transform_style.x, 1.0);
            let current = [
                material.transform_under.clone(),
                material.transform_moving.clone(),
                material.transform_above.clone(),
            ];
            if let Some(planes) = &planes {
                assert_eq!(&current, planes);
            }
            planes = Some(current);
            assert_eq!(
                app.world()
                    .resource::<ImageViewer>()
                    .edit
                    .document(Some(id))
                    .unwrap()
                    .revision,
                revision
            );
        }
        if gesture == 0 {
            let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/image-selection-qa/05-large-during-drag.png");
            std::fs::create_dir_all(output.parent().unwrap()).unwrap();
            capture_when(&mut app, &target, &output, |_| true);
        }
        let time = Instant::now();
        mouse(&mut app, window, ButtonState::Released);
        eprintln!(
            "2048px transform release {}: {:.1} ms",
            gesture + 1,
            time.elapsed().as_secs_f64() * 1000.0
        );
    }
    let result = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .selection
        .transform
        .as_ref()
        .unwrap()
        .quad;
    for _ in 0..300 {
        frame(&mut app);
        if !pending(&app) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        !pending(&app),
        "The latest released gesture must finish its native resample in the background"
    );
    native
        .preview_transform(&mut edit, source_frame, result.map(|p| p.to_array()), false)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .composite()
            .pixels,
        native.composite().pixels,
        "The newest gesture must win over an older background result"
    );
    drag_ms.sort_by(f64::total_cmp);
    eprintln!(
        "2048px native resample + composite: {native_ms:?} ms; cached GPU move (2 UI frames): median {:.2} ms, p95 {:.2} ms",
        drag_ms[drag_ms.len() / 2],
        drag_ms[drag_ms.len() * 95 / 100]
    );
    let quad = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .selection
        .transform
        .as_ref()
        .unwrap()
        .quad;
    let p = point(&mut app, quad[2]);
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Pressed);
    let p = point(&mut app, quad[2] + Vec2::splat(20.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Released);
    action(&mut app, Op::Cancel);
    for _ in 0..30 {
        frame(&mut app);
    }
    assert_eq!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .composite()
            .pixels,
        original
    );
    let material = app
        .world()
        .resource::<Assets<CanvasMaterial>>()
        .get(&super::super::super::canvas_material::HANDLE)
        .unwrap();
    assert_eq!(material.transform_style.x, 0.0);
    assert!(material.transform_moving.is_none());
    assert!(app.world().resource::<ImageViewer>().edit.error.is_none());
}
