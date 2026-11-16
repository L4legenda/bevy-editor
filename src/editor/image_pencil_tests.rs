use super::*;
use crate::editor::{
    photo_document::PhotoTool,
    selection_outline::render_tests::{capture_when, frame},
};
use bevy::{ecs::system::RunSystemOnce, input::ButtonState};
use edit::{EditAction, PencilShape};

fn issue(app: &mut App, action: EditAction) {
    app.world_mut().write_message(Action::ImageEdit(action));
    for _ in 0..4 {
        frame(app);
    }
}
fn doc_point(app: &mut App, p: Vec2) -> Vec2 {
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
    image.min + p / dimensions.as_vec2() * image.size()
}

#[test]
#[ignore = "requires GPU; checks pencil shape controls, hidden hardness, exact preview cells, DPI and unchanged history on hover"]
fn gpu_pencil_preview_matches_painted_cells_and_shape_controls() {
    use lighting_ui_render::{mouse, move_pointer};
    let fixture = Fixture::new();
    image::RgbaImage::new(48, 48)
        .save(fixture.0.join("pencil.png"))
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
    let index = file(&app, "pencil.png");
    app.world_mut().write_message(Action::ViewImage(index));
    for _ in 0..35 {
        frame(&mut app);
    }
    issue(&mut app, EditAction::Edit);
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
    let id = app.world().resource::<Editor>().viewing_image.unwrap();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/pencil-qa");
    std::fs::create_dir_all(&output).unwrap();
    issue(&mut app, EditAction::Tool(PhotoTool::Pencil));
    issue(&mut app, EditAction::Color([1.0; 4]));
    issue(&mut app, EditAction::Hardness(0.2));
    for (scale, size, shape, name) in [
        (1.0, 1.0, PencilShape::Round, "one-pixel"),
        (1.0, 9.0, PencilShape::Round, "round"),
        (1.0, 9.0, PencilShape::Square, "square"),
        (1.5, 6.0, PencilShape::Round, "round-dpi150"),
        (1.5, 6.0, PencilShape::Square, "square-dpi150"),
    ] {
        super::panel_sizing::resize_target(&mut app, window, &target, 1280.0, scale);
        app.world_mut().resource_mut::<ImageViewer>().zoom = Zoom::Scale(8.0);
        issue(&mut app, EditAction::Size(size));
        issue(&mut app, EditAction::SizeMenu);
        let visible = |app: &mut App, label: &str| {
            let world = app.world_mut();
            world
                .query::<(&AccessibleLabel, &ComputedNode)>()
                .iter(world)
                .any(|(l, n)| l.0 == label && !n.is_empty())
        };
        assert!(!visible(&mut app, "Жёсткость"));
        assert!(visible(&mut app, "Форма карандаша · Круг"));
        assert!(visible(&mut app, "Форма карандаша · Квадрат"));
        click_action(
            &mut app,
            window,
            |a| matches!(a,Action::ImageEdit(EditAction::PencilShape(s)) if s==shape),
        );
        assert_eq!(
            app.world().resource::<ImageViewer>().edit.pencil_shape,
            shape
        );
        if name == "square" {
            capture_when(&mut app, &target, &output.join("settings.png"), |_| true);
        }
        issue(&mut app, EditAction::SizeMenu);
        let center = doc_point(&mut app, Vec2::new(24.2, 24.3));
        let revision = app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .revision;
        move_pointer(&mut app, window, center);
        assert_eq!(
            app.world()
                .resource::<ImageViewer>()
                .edit
                .document(Some(id))
                .unwrap()
                .revision,
            revision
        );
        let material = app
            .world()
            .resource::<Assets<CanvasMaterial>>()
            .get(&canvas_material::HANDLE)
            .unwrap();
        if material.pencil_rect.z <= 0.0 {
            let viewer = app.world().resource::<ImageViewer>();
            eprintln!(
                "{name}: tool={:?}, owned={}, tip={:?}, position={:?}, size_menu={}, paint_menu={}, panel={}, actual_cursor={:?}, editing={}",
                viewer.edit.tool,
                viewer.cursor_owned,
                viewer.brush_cursor,
                viewer.brush_cursor_position,
                viewer.edit.size_menu,
                viewer.edit.paint_menu,
                viewer.panel.blocks_canvas(),
                app.world().get::<Window>(window).unwrap().cursor_position(),
                viewer.edit.editing(Some(id))
            );
            capture_when(&mut app, &target, &output.join("inactive.png"), |_| true);
        }
        let material = app
            .world()
            .resource::<Assets<CanvasMaterial>>()
            .get(&canvas_material::HANDLE)
            .unwrap();
        assert!(material.pencil_rect.z > 0.0, "{name}");
        let bounds = material.pencil_rect;
        let mask = app
            .world()
            .resource::<Assets<Image>>()
            .get(material.pencil_mask.as_ref().unwrap())
            .unwrap();
        let mask_width = mask.width();
        let cells = mask.data.clone().unwrap();
        let sample = doc_point(&mut app, Vec2::new(24.5, 24.5)) * scale;
        capture_when(
            &mut app,
            &target,
            &output.join(format!("{name}.png")),
            |im| {
                let p = im.get_pixel(sample.x as u32, sample.y as u32).0;
                p[0] > 180 && p[1] > 180 && p[2] > 180
            },
        );
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        let pixels = app
            .world()
            .resource::<ImageViewer>()
            .edit
            .document(Some(id))
            .unwrap()
            .composite();
        for y in 0..48 {
            for x in 0..48 {
                let local = Vec2::new(x as f32, y as f32) - Vec2::new(bounds.x, bounds.y);
                let expected = local.cmpge(Vec2::ZERO).all()
                    && local.cmplt(Vec2::new(bounds.z, bounds.w)).all()
                    && cells[(local.y as u32 * mask_width + local.x as u32) as usize] > 0;
                assert_eq!(
                    pixels.pixels[(y * 48 + x) * 4 + 3] > 0,
                    expected,
                    "{name}: {x},{y}"
                );
            }
        }
        issue(&mut app, EditAction::Undo);
    }
    issue(&mut app, EditAction::Tool(PhotoTool::Brush));
    issue(&mut app, EditAction::SizeMenu);
    let world = app.world_mut();
    assert!(
        world
            .query::<(&AccessibleLabel, &ComputedNode)>()
            .iter(world)
            .any(|(l, n)| l.0 == "Жёсткость" && !n.is_empty())
    );
    assert_eq!(world.resource::<ImageViewer>().edit.hardness, 0.2);
    assert_eq!(
        world
            .resource::<Assets<CanvasMaterial>>()
            .get(&canvas_material::HANDLE)
            .unwrap()
            .pencil_rect,
        Vec4::ZERO
    );
}
