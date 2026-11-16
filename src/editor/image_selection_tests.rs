use super::*;
use crate::editor::photo_document::{PhotoTool, SelectionTool};
use bevy::ecs::system::RunSystemOnce;
use edit::selection::{SelectionAction as Op, TransformMode, frame_quad, transformed_quad};

fn editing() -> (Fixture, App) {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let image = file(&app, "d-small.png");
    app.world_mut().write_message(Action::ViewImage(image));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(8.0);
    (fixture, app)
}
fn action(app: &mut App, op: Op) {
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Selection(op)));
    app.update();
}
fn point(app: &mut App, p: Vec2) -> Vec2 {
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
    let image = viewer.image_rect(viewer.dimensions.unwrap(), rect);
    image.min + p / viewer.dimensions.unwrap().as_vec2() * image.size()
}
fn drag(app: &mut App, a: Vec2, b: Vec2) {
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    for (down, p) in [(true, a), (true, b), (false, b)] {
        let p = point(app, p);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(p));
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            if down {
                mouse.press(MouseButton::Left);
            } else {
                mouse.release(MouseButton::Left);
            }
        }
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
    }
}
fn coverage(app: &App, x: i32, y: i32) -> f32 {
    let viewer = app.world().resource::<ImageViewer>();
    viewer
        .edit
        .document(viewer.tab)
        .unwrap()
        .document
        .selection
        .as_ref()
        .map_or(0.0, |m| m.sample_channel(x, y, 0))
}

#[test]
fn image_selection_gestures_map_zoom_and_support_shape_modes_and_shortcuts() {
    let (_fixture, mut app) = editing();
    action(&mut app, Op::Tool(SelectionTool::Rectangle));
    drag(&mut app, Vec2::new(2.0, 2.0), Vec2::new(10.0, 9.0));
    assert!(coverage(&app, 3, 3) > 0.9);
    assert_eq!(coverage(&app, 1, 1), 0.0);
    action(&mut app, Op::Mode(edit::selection::Mode::Add));
    drag(&mut app, Vec2::new(12.0, 2.0), Vec2::new(18.0, 9.0));
    assert!(coverage(&app, 14, 4) > 0.9);
    assert!(coverage(&app, 3, 3) > 0.9);
    action(&mut app, Op::Mode(edit::selection::Mode::Subtract));
    drag(&mut app, Vec2::new(2.0, 2.0), Vec2::new(5.0, 5.0));
    assert_eq!(coverage(&app, 3, 3), 0.0);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyD);
    }
    app.update();
    assert_eq!(coverage(&app, 14, 4), 0.0);
}

#[test]
fn image_free_transform_preview_cancel_and_apply_use_one_undo_step() {
    let (_fixture, mut app) = editing();
    action(&mut app, Op::Tool(SelectionTool::Rectangle));
    drag(&mut app, Vec2::new(2.0, 2.0), Vec2::new(8.0, 8.0));
    action(&mut app, Op::Transform);
    action(&mut app, Op::Nearest);
    action(&mut app, Op::Nudge(4, 0));
    assert!(coverage(&app, 7, 4) > 0.9);
    assert_eq!(coverage(&app, 3, 4), 0.0);
    action(&mut app, Op::Cancel);
    assert!(coverage(&app, 3, 4) > 0.9);
    assert_eq!(coverage(&app, 11, 4), 0.0);
    assert_eq!(
        app.world().resource::<ImageViewer>().edit.tool,
        PhotoTool::Move
    );
    action(&mut app, Op::Transform);
    action(&mut app, Op::Nudge(4, 0));
    action(&mut app, Op::Apply);
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .selection
            .transform
            .is_none()
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    app.update();
    assert!(coverage(&app, 3, 4) > 0.9);
    assert_eq!(coverage(&app, 11, 4), 0.0);
}

#[test]
fn image_transform_geometry_scales_about_opposite_corner_and_alt_center_and_snaps_rotation() {
    let quad = frame_quad([0.0, 0.0, 20.0, 10.0]);
    let scaled = transformed_quad(
        quad,
        4,
        Vec2::new(20.0, 10.0),
        Vec2::new(40.0, 20.0),
        TransformMode::Free,
        true,
        false,
        false,
        false,
    );
    assert_eq!(scaled[0], Vec2::ZERO);
    assert_eq!(scaled[2], Vec2::new(40.0, 20.0));
    let centered = transformed_quad(
        quad,
        4,
        Vec2::new(20.0, 10.0),
        Vec2::new(30.0, 15.0),
        TransformMode::Scale,
        true,
        false,
        true,
        false,
    );
    assert_eq!(centered[0], Vec2::new(-10.0, -5.0));
    assert_eq!(centered[2], Vec2::new(30.0, 15.0));
    let rotated = transformed_quad(
        quad,
        8,
        Vec2::new(20.0, 5.0),
        Vec2::new(10.0, 15.0),
        TransformMode::Rotate,
        true,
        true,
        false,
        false,
    );
    assert!(rotated[0].abs_diff_eq(Vec2::new(15.0, -5.0), 0.001));
    let distorted = transformed_quad(
        quad,
        0,
        Vec2::ZERO,
        Vec2::new(-2.0, -3.0),
        TransformMode::Distort,
        true,
        false,
        false,
        false,
    );
    assert_eq!(distorted[0], Vec2::new(-2.0, -3.0));
    assert_eq!(distorted[1], quad[1]);
}

#[test]
fn image_perspective_and_skew_handles_follow_both_axes_and_cannot_fold_the_quad() {
    let quad = frame_quad([0.0, 0.0, 20.0, 10.0]);
    let drag = |hit, delta, mode| {
        transformed_quad(
            quad,
            hit,
            Vec2::ZERO,
            delta,
            mode,
            true,
            false,
            false,
            false,
        )
    };
    let horizontal = drag(2, Vec2::new(-3.0, 1.0), TransformMode::Perspective);
    assert_eq!(
        horizontal,
        [Vec2::new(3.0, 0.0), Vec2::new(17.0, 0.0), quad[2], quad[3]]
    );
    let vertical = drag(2, Vec2::new(1.0, 2.0), TransformMode::Perspective);
    assert_eq!(
        vertical,
        [quad[0], Vec2::new(20.0, 2.0), Vec2::new(20.0, 8.0), quad[3]]
    );
    let corner = drag(0, Vec2::new(3.0, 1.0), TransformMode::Skew);
    assert_eq!(corner, [Vec2::new(3.0, 0.0), quad[1], quad[2], quad[3]]);
    let edge = drag(1, Vec2::new(3.0, 2.0), TransformMode::Skew);
    assert_eq!(
        edge,
        [Vec2::new(3.0, 0.0), Vec2::new(23.0, 0.0), quad[2], quad[3]]
    );
    let folded = drag(0, Vec2::new(60.0, 20.0), TransformMode::Distort);
    for i in 0..4 {
        assert!(
            (folded[(i + 1) % 4] - folded[i]).perp_dot(folded[(i + 2) % 4] - folded[(i + 1) % 4])
                > 0.0
        );
    }
    let edge_scale = drag(3, Vec2::new(20.0, 0.0), TransformMode::Scale);
    assert_eq!(edge_scale[0], Vec2::new(0.0, -5.0));
    assert_eq!(edge_scale[2], Vec2::new(40.0, 15.0));
}

#[test]
fn image_selection_tools_dispatch_native_shapes_samples_and_polygon_points() {
    let (_fixture, mut app) = editing();
    for (tool, start, end) in [
        (
            SelectionTool::Ellipse,
            Vec2::new(2.0, 1.0),
            Vec2::new(14.0, 11.0),
        ),
        (SelectionTool::Row, Vec2::new(6.0, 5.0), Vec2::new(6.0, 5.0)),
        (
            SelectionTool::Column,
            Vec2::new(6.0, 5.0),
            Vec2::new(6.0, 5.0),
        ),
        (
            SelectionTool::Wand,
            Vec2::new(6.0, 5.0),
            Vec2::new(6.0, 5.0),
        ),
        (
            SelectionTool::Quick,
            Vec2::new(6.0, 5.0),
            Vec2::new(7.0, 5.0),
        ),
    ] {
        action(&mut app, Op::Tool(tool));
        drag(&mut app, start, end);
        assert!(coverage(&app, 6, 5) > 0.5, "{tool:?}");
        assert!(
            app.world().resource::<ImageViewer>().edit.error.is_none(),
            "{tool:?}"
        );
        if matches!(
            tool,
            SelectionTool::Ellipse | SelectionTool::Row | SelectionTool::Column
        ) {
            assert_eq!(coverage(&app, 0, 0), 0.0, "{tool:?}");
        }
    }
    action(&mut app, Op::Tool(SelectionTool::Polygon));
    for p in [
        Vec2::new(2.0, 2.0),
        Vec2::new(14.0, 2.0),
        Vec2::new(14.0, 10.0),
        Vec2::new(2.0, 10.0),
    ] {
        drag(&mut app, p, p);
    }
    action(&mut app, Op::FinishPolygon);
    assert!(coverage(&app, 6, 5) > 0.9);
    assert_eq!(coverage(&app, 0, 0), 0.0);
    assert!(app.world().resource::<ImageViewer>().edit.error.is_none());
}

#[test]
#[ignore = "requires GPU; verifies native selection menus, marching ants, transform handles, pixel preview, cancellation and persistence"]
fn gpu_image_selection_and_transform_tools_match_canvas_and_preserve_history() {
    use crate::editor::selection_outline::render_tests::{capture_when, frame};
    use crate::editor::tests::lighting_ui_render::{mouse, move_pointer};
    use bevy::input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    };
    let fixture = Fixture::new();
    image::RgbaImage::from_fn(480, 320, |x, y| {
        if (60..240).contains(&x) && (40..200).contains(&y) {
            image::Rgba([55, 125, 225, 255])
        } else if (280..420).contains(&x) && (90..250).contains(&y) {
            image::Rgba([230, 120, 70, 255])
        } else {
            image::Rgba([245, 245, 245, 255])
        }
    })
    .save(fixture.0.join("selection.png"))
    .unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture();
    let scene = app.world().resource::<Editor>().edits.clone();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let image = file(&app, "selection.png");
    app.world_mut().write_message(Action::ViewImage(image));
    for _ in 0..30 {
        frame(&mut app);
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    for _ in 0..120 {
        frame(&mut app);
        if app
            .world()
            .resource::<ImageViewer>()
            .edit
            .editing(app.world().resource::<Editor>().viewing_image)
        {
            break;
        }
    }
    for _ in 0..15 {
        frame(&mut app);
    }
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(1.0);
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/image-selection-qa");
    std::fs::create_dir_all(&output).unwrap();
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Selection(Op::Menu)))
    });
    for _ in 0..3 {
        frame(&mut app);
    }
    capture_when(&mut app, &target, &output.join("00-tool-menu.png"), |_| {
        true
    });
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Selection(Op::Tool(
                SelectionTool::Rectangle
            )))
        )
    });
    let p = point(&mut app, Vec2::new(80.0, 60.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Pressed);
    let p = point(&mut app, Vec2::new(220.0, 170.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Released);
    for _ in 0..4 {
        frame(&mut app);
    }
    assert!(coverage(&app, 100, 90) > 0.9);
    assert_eq!(coverage(&app, 20, 20), 0.0);
    capture_when(&mut app, &target, &output.join("01-selection.png"), |_| {
        true
    });
    let original = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .document(Some(id))
        .unwrap()
        .composite()
        .pixels;
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Selection(Op::Transform))
        )
    });
    for _ in 0..3 {
        frame(&mut app);
    }
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
    let p = point(&mut app, quad[2] + Vec2::new(70.0, 55.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Released);
    assert!(coverage(&app, 270, 190) > 0.5);
    capture_when(
        &mut app,
        &target,
        &output.join("02-free-transform.png"),
        |_| true,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Selection(Op::TransformMenu))
        )
    });
    for _ in 0..3 {
        frame(&mut app);
    }
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Selection(Op::TransformMode(
                TransformMode::Perspective
            )))
        )
    });
    let quad = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .selection
        .transform
        .as_ref()
        .unwrap()
        .quad;
    let p = point(&mut app, quad[1]);
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Pressed);
    let p = point(&mut app, quad[1] - Vec2::new(25.0, 0.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Released);
    let after = app
        .world()
        .resource::<ImageViewer>()
        .edit
        .selection
        .transform
        .as_ref()
        .unwrap()
        .quad;
    assert!(after[0].abs_diff_eq(quad[0] + Vec2::new(25.0, 0.0), 0.1));
    assert!(after[1].abs_diff_eq(quad[1] - Vec2::new(25.0, 0.0), 0.1));
    assert!(after[2].abs_diff_eq(quad[2], 0.1));
    capture_when(
        &mut app,
        &target,
        &output.join("04-perspective.png"),
        |_| true,
    );
    click_action(&mut app, window, |a| {
        matches!(
            a,
            Action::ImageEdit(edit::EditAction::Selection(Op::Cancel))
        )
    });
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
    assert!(coverage(&app, 100, 90) > 0.9);
    assert_eq!(coverage(&app, 270, 190), 0.0);
    for state in [ButtonState::Pressed, ButtonState::Released] {
        for (key_code, logical_key) in [
            (KeyCode::ControlLeft, Key::Control),
            (KeyCode::KeyT, Key::Character("t".into())),
        ] {
            app.world_mut().write_message(KeyboardInput {
                window,
                key_code,
                logical_key,
                state,
                text: None,
                repeat: false,
            });
        }
        for _ in 0..2 {
            frame(&mut app);
        }
    }
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .selection
            .transform
            .is_some()
    );
    let p = point(&mut app, Vec2::new(150.0, 115.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Pressed);
    let p = point(&mut app, Vec2::new(240.0, 135.0));
    move_pointer(&mut app, window, p);
    mouse(&mut app, window, ButtonState::Released);
    capture_when(
        &mut app,
        &target,
        &output.join("03-move-selected-pixels.png"),
        |_| true,
    );
    click_action(&mut app, window, |a| {
        matches!(a, Action::ImageEdit(edit::EditAction::Selection(Op::Apply)))
    });
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .selection
            .transform
            .is_none()
    );
    assert!(coverage(&app, 280, 100) > 0.9);
    assert_eq!(coverage(&app, 100, 90), 0.0);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Save));
    for _ in 0..180 {
        frame(&mut app);
        if !app.world().resource::<ImageViewer>().edit.busy() {
            break;
        }
    }
    let native =
        crate::editor::photo_document::PhotoDocument::load(&fixture.0.join("selection.png"))
            .unwrap();
    assert!(
        native
            .document
            .selection
            .unwrap()
            .sample_channel(280, 100, 0)
            > 0.9
    );
    assert_eq!(app.world().resource::<Editor>().edits, scene);
    assert!(app.world().resource::<ImageViewer>().edit.error.is_none());
}
