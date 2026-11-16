use super::*;
use crate::editor::selection_outline::render_tests::{capture_when, frame};
use bevy::{
    asset::AssetEvent,
    ecs::message::MessageCursor,
    input::{ButtonState, mouse::MouseButtonInput},
    window::{CursorMoved, WindowEvent},
};

fn layer_panel_nodes(app: &mut App) -> Vec<Entity> {
    let world = app.world_mut();
    let row = world
        .query::<(Entity, &Control)>()
        .iter(world)
        .find_map(|(entity, control)| {
            matches!(
                control.action,
                Action::ImageEdit(edit::EditAction::SelectLayer(0))
            )
            .then_some(entity)
        })
        .expect("Visible layer row");
    let outer = world.get::<ChildOf>(row).unwrap().parent();
    let host = world.get::<ChildOf>(outer).unwrap().parent();
    let mut nodes = vec![host];
    let mut index = 0;
    while index < nodes.len() {
        if let Some(children) = world.get::<Children>(nodes[index]) {
            nodes.extend(children.iter());
        }
        index += 1;
    }
    nodes
}

#[test]
#[ignore = "requires GPU; verifies same-frame stroke tails, batched pointer input and region uploads"]
fn gpu_photo_paint_updates_current_frame_and_preserves_batched_pointer_samples() {
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(1024, 1024, image::Rgba([255; 4]))
        .save(fixture.0.join("paint.png"))
        .unwrap();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    for _ in 0..30 {
        frame(&mut app);
    }
    let index = file(&app, "paint.png");
    app.world_mut().write_message(Action::ViewImage(index));
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
    for action in [
        Action::ImageView(ImageAction::ActualSize),
        Action::ImageEdit(edit::EditAction::Size(12.0)),
        Action::ImageEdit(edit::EditAction::Color([1.0, 0.0, 0.0, 1.0])),
    ] {
        app.world_mut().write_message(action);
    }
    for _ in 0..8 {
        frame(&mut app);
    }
    let (handle, image_min) = {
        let world = app.world_mut();
        let mut canvas =
            world.query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>();
        let (node, transform) = canvas.single(world).unwrap();
        let center = transform.affine().translation * node.inverse_scale_factor();
        (
            world.resource::<ImageViewer>().image.clone().unwrap(),
            center - Vec2::splat(512.0),
        )
    };
    let move_to = |app: &mut App, point: Vec2| {
        let position = image_min + point;
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(position));
        app.world_mut()
            .write_message(WindowEvent::CursorMoved(CursorMoved {
                window,
                position,
                delta: None,
            }));
    };
    let button = |app: &mut App, state| {
        app.world_mut()
            .write_message(WindowEvent::MouseButtonInput(MouseButtonInput {
                window,
                button: MouseButton::Left,
                state,
            }));
        app.world_mut().write_message(MouseButtonInput {
            window,
            button: MouseButton::Left,
            state,
        });
    };
    let pixel = |app: &App, x: usize, y: usize| {
        let image = app
            .world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap();
        image.data.as_ref().unwrap()[(y * 1024 + x) * 4..(y * 1024 + x) * 4 + 4].to_vec()
    };
    let red = |p: &[u8]| p[0] > 230 && p[1] < 50 && p[2] < 50;
    let mut events = MessageCursor::<AssetEvent<Image>>::default();
    events.clear(app.world().resource::<Messages<AssetEvent<Image>>>());
    let panel_nodes = layer_panel_nodes(&mut app);
    let (thumbnail_entity, thumbnail_handle, thumbnail_rect) = {
        let world = app.world_mut();
        let (entity, image, node, transform) = world.query_filtered::<
            (Entity, &ImageNode, &ComputedNode, &UiGlobalTransform),
            With<edit::layers::ui::LayerThumbnail>,
        >().single(world).unwrap();
        let center = transform.affine().translation * node.inverse_scale_factor();
        let half_size = node.size() * node.inverse_scale_factor() * 0.5;
        (
            entity,
            image.image.clone(),
            Rect::from_corners(center - half_size, center + half_size),
        )
    };
    let original_thumbnail = app
        .world()
        .resource::<Assets<Image>>()
        .get(&thumbnail_handle)
        .unwrap()
        .data
        .clone()
        .unwrap();
    assert_ne!(
        thumbnail_handle.id(),
        Handle::<Image>::default().id(),
        "Layer previews must never overwrite Bevy's shared white UI texture"
    );

    move_to(&mut app, Vec2::new(400.0, 500.0));
    button(&mut app, ButtonState::Pressed);
    frame(&mut app);
    assert!(
        red(&pixel(&app, 400, 500)),
        "The first dab must reach the texture in the input frame, before release"
    );
    assert_eq!(
        layer_panel_nodes(&mut app),
        panel_nodes,
        "A dab must not recreate the inspector widgets"
    );
    for point in [
        Vec2::new(400.0, 420.0),
        Vec2::new(520.0, 420.0),
        Vec2::new(520.0, 520.0),
    ] {
        move_to(&mut app, point);
    }
    frame(&mut app);
    assert_eq!(
        layer_panel_nodes(&mut app),
        panel_nodes,
        "Each held stroke frame must keep the layer tree alive"
    );
    for (x, y) in [(400, 420), (520, 420), (520, 520)] {
        assert!(
            red(&pixel(&app, x, y)),
            "A pointer sample or live tail is missing at {x},{y}"
        );
    }
    assert_eq!(
        app.world().resource::<ImageViewer>().image.as_ref(),
        Some(&handle)
    );
    assert!(
        !events
            .read(app.world().resource::<Messages<AssetEvent<Image>>>())
            .any(|event| matches!(event, AssetEvent::Modified { id } if *id == handle.id() || *id == thumbnail_handle.id())),
        "Painting must preserve both canvas and thumbnail GPU images"
    );
    assert_ne!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&thumbnail_handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &original_thumbnail,
        "The thumbnail must still update while painting"
    );
    assert_eq!(
        app.world()
            .get::<ImageNode>(thumbnail_entity)
            .unwrap()
            .image,
        thumbnail_handle
    );
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/photo-paint-qa");
    std::fs::create_dir_all(&output).unwrap();
    {
        let thumbnail = app
            .world()
            .resource::<Assets<Image>>()
            .get(&thumbnail_handle)
            .unwrap();
        image::save_buffer(
            output.join("01-live-thumbnail.png"),
            thumbnail.data.as_ref().unwrap(),
            thumbnail.width(),
            thumbnail.height(),
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    let point = image_min + Vec2::new(400.0, 420.0);
    let rendered = capture_when(
        &mut app,
        &target,
        &output.join("01-live-stroke.png"),
        |image| red(&image.get_pixel(point.x as u32, point.y as u32).0),
    );
    assert!(red(&rendered.get_pixel(point.x as u32, point.y as u32).0));
    assert!(
        (thumbnail_rect.min.y.ceil() as u32..thumbnail_rect.max.y.floor() as u32).any(|y| {
            (thumbnail_rect.min.x.ceil() as u32..thumbnail_rect.max.x.floor() as u32).any(|x| {
                let pixel = rendered.get_pixel(x, y).0;
                pixel[0] > 200 && pixel[1] < pixel[0].saturating_sub(20)
            })
        }),
        "The live thumbnail update must reach the rendered sidebar"
    );
    let preview = app
        .world()
        .resource::<Assets<Image>>()
        .get(&handle)
        .unwrap()
        .data
        .clone()
        .unwrap();
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    assert_eq!(
        layer_panel_nodes(&mut app),
        panel_nodes,
        "Finishing a stroke must not rebuild the panel either"
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &preview,
        "Release must not catch up or change the displayed stroke"
    );

    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    frame(&mut app);
    assert_eq!(
        layer_panel_nodes(&mut app),
        panel_nodes,
        "Undoing pixels keeps the inspector stable"
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&thumbnail_handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &original_thumbnail
    );
    assert!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .iter()
            .all(|v| *v == 255)
    );
    capture_when(&mut app, &target, &output.join("02-undo.png"), |image| {
        image.get_pixel(point.x as u32, point.y as u32).0[..3] == [255; 3]
    });
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Redo));
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &preview
    );
    capture_when(&mut app, &target, &output.join("03-redo.png"), |image| {
        red(&image.get_pixel(point.x as u32, point.y as u32).0)
    });

    move_to(&mut app, Vec2::new(560.0, 560.0));
    button(&mut app, ButtonState::Pressed);
    move_to(&mut app, Vec2::new(590.0, 560.0));
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    assert!(
        red(&pixel(&app, 560, 560)) && red(&pixel(&app, 590, 560)),
        "A complete click/drag between frames must not disappear"
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &preview
    );
    // A complete swipe can begin AND end on the pasteboard in one input batch.
    // It must still cross the image and remain a single undo step.
    app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(0.5);
    for _ in 0..5 {
        frame(&mut app);
    }
    let rect = {
        let world = app.world_mut();
        let (node, transform) = world
            .query_filtered::<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>()
            .single(world)
            .unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    let image = app
        .world()
        .resource::<ImageViewer>()
        .image_rect(UVec2::splat(1024), rect);
    let screen = |point: Vec2| image.min + point / 1024.0 * image.size();
    let move_outside = |app: &mut App, point: Vec2| {
        let position = screen(point);
        assert!(
            rect.contains(position),
            "The pasteboard is part of the workspace"
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(position));
        app.world_mut()
            .write_message(WindowEvent::CursorMoved(CursorMoved {
                window,
                position,
                delta: None,
            }));
    };
    let before = app
        .world()
        .resource::<Assets<Image>>()
        .get(&handle)
        .unwrap()
        .data
        .clone()
        .unwrap();
    move_outside(&mut app, Vec2::new(-80.0, 100.0));
    button(&mut app, ButtonState::Pressed);
    move_outside(&mut app, Vec2::new(1104.0, 100.0));
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    for x in [1, 128, 512, 900, 1022] {
        assert!(
            red(&pixel(&app, x, 100)),
            "Outside-to-outside swipe missed {x},100"
        );
    }
    let stroke = app
        .world()
        .resource::<Assets<Image>>()
        .get(&handle)
        .unwrap()
        .data
        .clone()
        .unwrap();
    let middle = screen(Vec2::new(512.0, 100.0));
    capture_when(
        &mut app,
        &target,
        &output.join("04-from-outside.png"),
        |image| red(&image.get_pixel(middle.x as u32, middle.y as u32).0),
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &before
    );
    let revision = {
        let viewer = app.world().resource::<ImageViewer>();
        viewer.edit.document(viewer.tab).unwrap().revision
    };
    move_outside(&mut app, Vec2::new(-80.0, 100.0));
    button(&mut app, ButtonState::Pressed);
    move_outside(&mut app, Vec2::new(-60.0, 140.0));
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    {
        let viewer = app.world().resource::<ImageViewer>();
        let doc = viewer.edit.document(viewer.tab).unwrap();
        assert_eq!(doc.revision, revision);
        assert!(doc.can_redo(), "An empty outside drag must preserve Redo");
    }
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Redo));
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &stroke
    );
    // Capture survives the inspector and the OS window boundary. Keep the
    // actual outside trajectory so returning does not draw a diagonal shortcut.
    let start = screen(Vec2::new(400.0, 650.0));
    let end = screen(Vec2::new(700.0, 800.0));
    let outside_window = Vec2::new(
        app.world().get::<Window>(window).unwrap().width() + 30.0,
        end.y,
    );
    let raw_move = |app: &mut App, position: Vec2, cursor_present: bool| {
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(cursor_present.then_some(position));
        app.world_mut()
            .write_message(WindowEvent::CursorMoved(CursorMoved {
                window,
                position,
                delta: None,
            }));
    };
    raw_move(&mut app, start, true);
    button(&mut app, ButtonState::Pressed);
    frame(&mut app);
    raw_move(&mut app, Vec2::new(rect.max.x + 40.0, start.y), true);
    frame(&mut app);
    raw_move(&mut app, outside_window, false);
    app.world_mut()
        .write_message(WindowEvent::CursorLeft(bevy::window::CursorLeft { window }));
    frame(&mut app);
    frame(&mut app);
    raw_move(&mut app, end, true);
    frame(&mut app);
    assert!(
        red(&pixel(&app, 700, 800)),
        "The held stroke must resume on return"
    );
    assert_eq!(
        pixel(&app, 550, 725),
        [255; 4],
        "Keep the path around the image, without a diagonal shortcut"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("05-workspace-exit.png"),
        |image| red(&image.get_pixel(end.x as u32, end.y as u32).0),
    );
    raw_move(&mut app, outside_window, false);
    frame(&mut app);
    frame(&mut app);
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    raw_move(&mut app, screen(Vec2::new(128.0, 900.0)), true);
    frame(&mut app);
    assert_eq!(
        pixel(&app, 128, 900),
        [255; 4],
        "Release outside the window must end the capture"
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &stroke
    );

    // The window can lose its cursor position before the frame processes the
    // initial press and final release. Preserve those already queued samples.
    raw_move(&mut app, start, true);
    button(&mut app, ButtonState::Pressed);
    raw_move(&mut app, outside_window, false);
    button(&mut app, ButtonState::Released);
    frame(&mut app);
    assert!(
        red(&pixel(&app, 400, 650)),
        "A batched swipe must render even with no current window cursor"
    );
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    frame(&mut app);
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&handle)
            .unwrap()
            .data
            .as_ref()
            .unwrap(),
        &stroke
    );
    println!("Live painting captures: {}", output.display());
}
