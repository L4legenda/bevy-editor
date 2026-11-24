//! Full editor UI captures without an OS window or writes to project assets.
#![cfg(not(target_arch = "wasm32"))]

use std::{path::PathBuf, time::Duration};

use bevy::{
    camera::RenderTarget,
    input::{ButtonState, mouse::MouseButtonInput},
    picking::{
        input::PointerInputSettings,
        pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput},
    },
    prelude::*,
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{Extent3d, TextureFormat, TextureUsages},
    },
    time::TimeUpdateStrategy,
    ui::{CalculatedClip, IsDefaultUiCamera},
    window::{CursorMoved, ExitCondition, PrimaryWindow},
    winit::WinitPlugin,
};

use crate::{
    asset_paths::AssetRedirectPlugin,
    editor::{
        Action, Editor, EditorPlugin, Field, Selection,
        lighting::{EnvironmentField, LightField, LightingAction},
        selection_outline::render_tests::{CapturedImage, capture_when, frame},
        ui,
    },
    lighting::LightPreset,
    project::EditorProject,
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 900;

#[derive(Resource)]
struct Canvas(Handle<Image>);

fn retarget_ui(
    canvas: Res<Canvas>,
    mut cameras: Query<(&mut RenderTarget, &mut Camera), With<IsDefaultUiCamera>>,
) {
    let mut count = 0;
    for (mut target, mut camera) in &mut cameras {
        *target = RenderTarget::Image(canvas.0.clone().into());
        camera.order = 100;
        count += 1;
    }
    assert_eq!(
        count, 1,
        "The real editor must create its default UI camera"
    );
}

pub(in crate::editor) fn fixture() -> (App, Entity, Handle<Image>) {
    let project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    fixture_project(project)
}

pub(in crate::editor) fn fixture_project(project: EditorProject) -> (App, Entity, Handle<Image>) {
    fixture_project_with_pipeline_compilation(project, true)
}

pub(in crate::editor) fn fixture_project_with_pipeline_compilation(
    project: EditorProject,
    synchronous_pipeline_compilation: bool,
) -> (App, Entity, Handle<Image>) {
    let mut app = App::new();
    let mut plugins = DefaultPlugins
        .set(AssetPlugin {
            file_path: project.asset_root.to_string_lossy().into_owned(),
            ..default()
        })
        .set(WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            close_when_requested: false,
            ..default()
        })
        .set(RenderPlugin {
            synchronous_pipeline_compilation,
            ..default()
        })
        .disable::<WinitPlugin>();
    if synchronous_pipeline_compilation {
        plugins = plugins.disable::<PipelinedRenderingPlugin>();
    }
    app.add_plugins(AssetRedirectPlugin::new(&project.asset_root).unwrap())
        .add_plugins(plugins)
        .add_plugins(EditorPlugin::new(project).unwrap())
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .init_resource::<CapturedImage>()
        .insert_resource(PointerInputSettings {
            is_mouse_enabled: false,
            is_touch_enabled: false,
        })
        .add_systems(
            Startup,
            retarget_ui.after(crate::editor::setup).after(ui::setup),
        );

    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                resolution: (WIDTH, HEIGHT).into(),
                decorations: false,
                focused: true,
                ..default()
            },
        ))
        .id();
    let mut image = Image::new_target_texture(WIDTH, HEIGHT, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.insert_resource(Canvas(target.clone()));
    app.finish();
    app.cleanup();
    for _ in 0..480 {
        frame(&mut app);
        if app.world().resource::<Editor>().ready {
            break;
        }
    }
    assert!(
        app.world().resource::<Editor>().ready,
        "The actual example scene failed to load: {}",
        app.world().resource::<Editor>().status
    );
    warm(&mut app, 16);
    (app, window, target)
}

fn warm(app: &mut App, frames: usize) {
    for _ in 0..frames {
        frame(app);
    }
}

#[test]
#[ignore = "requires GPU adapter; verifies drawer overlay keeps camera preview and pilot hints fixed"]
fn gpu_file_manager_overlays_camera_preview_and_pilot_hints() {
    let (mut app, _, target) = fixture();
    app.world_mut().resource_mut::<Editor>().drawer = false;
    app.world_mut()
        .write_message(Action::Select(Selection::Camera(0)));
    warm(&mut app, 25);
    let scene_image = {
        let world = app.world_mut();
        let image = world
            .resource::<crate::editor::viewport::Surface>()
            .0
            .clone();
        world
            .query::<(Entity, &ImageNode)>()
            .iter(world)
            .find(|(_, node)| node.image == image)
            .unwrap()
            .0
    };
    let preview_image = {
        let world = app.world_mut();
        let camera_target = world
            .query_filtered::<&RenderTarget, With<ui::PreviewCamera>>()
            .single(world)
            .unwrap();
        let RenderTarget::Image(image) = camera_target else {
            panic!("Missing camera preview")
        };
        let image = image.handle.clone();
        world
            .query::<(Entity, &ImageNode)>()
            .iter(world)
            .find(|(_, node)| node.image == image)
            .unwrap()
            .0
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/file-manager-overlay-qa");
    std::fs::create_dir_all(&output).unwrap();
    for pilot in [false, true] {
        if pilot {
            app.world_mut().write_message(Action::Pilot);
            warm(&mut app, 5);
        }
        let entity = if pilot {
            let world = app.world_mut();
            world
                .query::<(Entity, &Text)>()
                .iter(world)
                .find(|(_, text)| text.0 == "ПКМ + WASD · движение   Q/E · вверх/вниз")
                .unwrap()
                .0
        } else {
            preview_image
        };
        let geometry = |app: &App| {
            [scene_image, entity].map(|entity| {
                let node = app.world().get::<ComputedNode>(entity).unwrap();
                let transform = app.world().get::<UiGlobalTransform>(entity).unwrap();
                assert!(!node.is_empty());
                Rect::from_center_size(transform.affine().translation, node.size())
            })
        };
        let fixed = geometry(&app);
        let view = app.world().resource::<Editor>().view;
        app.world_mut().write_message(Action::Drawer);
        for _ in 0..25 {
            frame(&mut app);
            assert_eq!(geometry(&app), fixed);
        }
        app.world_mut()
            .resource_mut::<ui::DrawerSize>()
            .preferred_height = Some(500.0);
        warm(&mut app, 4);
        assert_eq!(geometry(&app), fixed);
        assert_eq!(app.world().resource::<Editor>().view, view);
        capture_when(
            &mut app,
            &target,
            &output.join(if pilot {
                "pilot.png"
            } else {
                "camera-preview.png"
            }),
            |_| true,
        );
        app.world_mut().write_message(Action::Drawer);
        for _ in 0..25 {
            frame(&mut app);
            assert_eq!(geometry(&app), fixed);
        }
    }
    println!("File manager overlay screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU adapter; opens, edits, saves and previews native .scene documents"]
fn gpu_native_scene_documents_open_edit_save_and_preview() {
    let directory =
        std::env::temp_dir().join(format!("bevy-editor-native-scene-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("models")).unwrap();
    let mut project = EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native/project.json"),
    )
    .unwrap();
    for model in &project.catalog {
        std::fs::copy(
            project.asset_root.join(&model.asset),
            directory.join(&model.asset),
        )
        .unwrap();
    }
    for index in 0..project.scenes.len() {
        let document = project.read_scene(index).unwrap();
        if let Some(hdri) = document
            .lighting
            .as_ref()
            .and_then(|lighting| lighting.hdri_asset.as_ref())
        {
            let destination = directory.join(&hdri.asset);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(project.asset_root.join(&hdri.asset), destination).unwrap();
        }
        document
            .write(&directory, &project.scenes[index].file)
            .unwrap();
    }
    project.asset_root = directory.clone();
    let (mut app, _, target) = fixture_project(project);
    warm(&mut app, 40);
    {
        let editor = app.world().resource::<Editor>();
        assert!(editor.imported.is_empty());
        assert_eq!(editor.edits.models.len(), 3);
        let scene = app.world().resource::<super::Scene>();
        assert_eq!(scene.models.len(), 3);
        assert!(
            app.world()
                .get::<WorldAssetRoot>(scene.root.unwrap())
                .is_none()
        );
    }
    app.world_mut()
        .write_message(Action::Select(Selection::Model(1)));
    warm(&mut app, 2);
    app.world_mut()
        .write_message(Action::Number(Field::Position(0), -3.0));
    warm(&mut app, 2);
    app.world_mut().write_message(Action::Save);
    warm(&mut app, 8);
    let saved = crate::LevelEdits::<String>::read(&directory, "scenes/workshop.scene").unwrap();
    assert_eq!(saved.models[0].pose.position[0], -3.0);
    assert!(!app.world().resource::<Editor>().dirty());
    app.world_mut().write_message(Action::Open(1));
    warm(&mut app, 40);
    assert!(app.world().resource::<Editor>().ready);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    app.world_mut().write_message(Action::OpenBrowser);
    warm(&mut app, 180);
    let previews = app
        .world()
        .resource::<super::thumbnails::Thumbnails>()
        .scene_images
        .clone();
    let regions: Vec<_> = {
        let world = app.world_mut();
        let mut nodes = world.query::<(&ImageNode, &ComputedNode, &bevy::ui::UiGlobalTransform)>();
        previews
            .iter()
            .map(|handle| {
                let (_, node, transform) = nodes
                    .iter(world)
                    .find(|(image, node, _)| {
                        image.image == *handle && node.size().min_element() > 0.0
                    })
                    .unwrap();
                let center = transform.affine().translation;
                Rect::from_corners(
                    center - node.size() * 0.5 + Vec2::splat(4.0),
                    center + node.size() * 0.5 - Vec2::splat(4.0),
                )
            })
            .collect()
    };
    // Screenshot redirects a target's next render; retained thumbnail cameras
    // are inactive, so inspect their images through the active UI camera.
    capture_when(&mut app, &target, &directory.join("editor.png"), |image| {
        regions.iter().all(|rect| {
            let mut colored = 0;
            for y in rect.min.y as u32..rect.max.y as u32 {
                for x in rect.min.x as u32..rect.max.x as u32 {
                    let pixel = image.get_pixel(x, y);
                    if pixel[0].abs_diff(pixel[2]) > 24 {
                        colored += 1;
                    }
                }
            }
            colored > 64
        })
    });
    println!("Native scene GPU artifacts: {}", directory.display());
}

#[test]
#[ignore = "requires GPU adapter; exercises the real hierarchy outliner, inspector and scene instances"]
fn gpu_scene_hierarchy_outliner_instances_and_affine_rendering() {
    use super::scene_hierarchy::{HierarchyAction as H, tests::fixture as hierarchy_fixture};
    let mut files = hierarchy_fixture();
    files.keep = true;
    let original_prefab = std::fs::read(files.root.join("lamp.scene")).unwrap();
    let (mut app, window, target) = fixture_project(files.project.clone());
    warm(&mut app, 60);
    let (mount, mesh, expected) = {
        let editor = app.world().resource::<Editor>();
        let h = editor.edits.hierarchy.as_ref().unwrap();
        (
            h.by_id("a").unwrap().key,
            h.by_id("a/mesh").unwrap().key,
            h.by_id("a/mesh").unwrap().affine(),
        )
    };
    let entity = app.world().resource::<super::Scene>().models[&mesh].0;
    assert!(
        app.world()
            .get::<GlobalTransform>(entity)
            .unwrap()
            .affine()
            .abs_diff_eq(expected, 1e-5)
    );
    let click = |app: &mut App, predicate: fn(Action, u32) -> bool, key: u32| {
        let center = {
            let world = app.world_mut();
            let mut controls = world.query::<(&ui::Control, &ComputedNode, &UiGlobalTransform)>();
            let (_, node, transform) = controls
                .iter(world)
                .find(|(control, node, _)| {
                    predicate(control.action, key) && node.size().min_element() > 0.0
                })
                .expect("Hierarchy control missing from the rendered UI");
            assert!(node.size().y > 10.0);
            transform.affine().translation
        };
        move_pointer(app, window, center);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 3);
    };
    click(
        &mut app,
        |a, k| matches!(a,Action::ToggleVisibility(Selection::Camera(shot)) if shot==k as usize),
        0,
    );
    assert!(
        app.world()
            .resource::<Editor>()
            .object_hidden(Selection::Camera(0))
    );
    click(
        &mut app,
        |a, k| matches!(a,Action::ToggleVisibility(Selection::Camera(shot)) if shot==k as usize),
        0,
    );
    assert!(
        !app.world()
            .resource::<Editor>()
            .object_hidden(Selection::Camera(0))
    );
    click(
        &mut app,
        |a, k| matches!(a,Action::Select(Selection::Node(key)) if key==k),
        mount,
    );
    assert_eq!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Node(mount))
    );
    capture_when(
        &mut app,
        &target,
        &files.root.join("hierarchy-expanded.png"),
        ui_pixels,
    );
    click(
        &mut app,
        |a, k| matches!(a,Action::Hierarchy(H::Fold(key)) if key==k),
        mount,
    );
    {
        let world = app.world_mut();
        let mut controls = world.query::<&ui::Control>();
        assert!(
            !controls
                .iter(world)
                .any(|c| matches!(c.action,Action::Select(Selection::Model(key)) if key==mesh))
        );
    }
    click(
        &mut app,
        |a, k| matches!(a,Action::Hierarchy(H::Fold(key)) if key==k),
        mount,
    );
    click(
        &mut app,
        |a, k| matches!(a,Action::Select(Selection::Model(key)) if key==k),
        mesh,
    );
    app.world_mut().write_message(Action::LocalSpace);
    warm(&mut app, 2);
    app.world_mut()
        .write_message(Action::Number(Field::Position(1), 1.3));
    warm(&mut app, 3);
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    app.world_mut().write_message(Action::Save);
    warm(&mut app, 4);
    assert!(!app.world().resource::<Editor>().dirty());
    let loaded = crate::LevelEdits::<String>::read(&files.root, "room.scene").unwrap();
    assert_eq!(
        loaded
            .hierarchy
            .as_ref()
            .unwrap()
            .node(mesh)
            .unwrap()
            .local()
            .position[1],
        1.3
    );
    assert_eq!(
        std::fs::read(files.root.join("lamp.scene")).unwrap(),
        original_prefab
    );
    app.world_mut()
        .write_message(Action::Hierarchy(H::ResetOverrides));
    warm(&mut app, 3);
    assert!(
        !app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 3);
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    app.world_mut()
        .write_message(Action::Select(Selection::Node(mount)));
    warm(&mut app, 3);
    capture_when(
        &mut app,
        &target,
        &files.root.join("hierarchy-overrides.png"),
        ui_pixels,
    );
    // Exercise the actual source tab command and return to the unsaved parent draft.
    app.world_mut()
        .write_message(Action::Hierarchy(H::OpenSource));
    warm(&mut app, 20);
    assert_eq!(app.world().resource::<Editor>().room, 1);
    app.world_mut().write_message(Action::Open(0));
    warm(&mut app, 25);
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    let (stand, sibling, other) = {
        let editor = app.world().resource::<Editor>();
        let graph = editor.edits.hierarchy.as_ref().unwrap();
        (
            graph.by_id("a/stand").unwrap().key,
            graph.by_id("b/stand").unwrap().key,
            graph.by_id("b").unwrap().key,
        )
    };
    let row_point = |app: &mut App, key, offset: f32| {
        let world = app.world_mut();
        let mut query = world.query::<(&ui::Control, &ComputedNode, &UiGlobalTransform)>();
        let (_, node, transform) = query
            .iter(world)
            .find(|(c, node, _)| {
                matches!(c.action, Action::Select(Selection::Node(id)) if id == key)
                    && !node.is_empty()
            })
            .unwrap();
        transform
            .affine()
            .transform_point2(Vec2::Y * offset * node.size().y)
    };
    let drag_row = |app: &mut App, source, destination: Vec2| {
        let start = row_point(app, source, 0.0);
        move_pointer(app, window, start);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, destination);
        mouse(app, window, ButtonState::Released);
        warm(app, 3);
    };
    let count = app.world().resource::<Editor>().undo.len();
    let destination = row_point(&mut app, other, 0.0);
    drag_row(&mut app, stand, destination);
    let graph = |app: &App| {
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .clone()
            .unwrap()
    };
    assert_eq!(graph(&app).parent(stand).unwrap().key, other);
    assert!(graph(&app).sibling_offset(stand, true).is_none());
    let destination = row_point(&mut app, sibling, -0.38);
    drag_row(&mut app, stand, destination);
    assert!(graph(&app).sibling_offset(stand, false).is_none());
    assert_eq!(app.world().resource::<Editor>().undo.len(), count + 2);
    drag_row(&mut app, stand, Vec2::new(400.0, 400.0));
    assert_eq!(app.world().resource::<Editor>().undo.len(), count + 2);
    app.world_mut().write_message(Action::Save);
    warm(&mut app, 5);
    let saved = crate::LevelEdits::<String>::read(&files.root, "room.scene")
        .unwrap()
        .hierarchy
        .unwrap();
    assert_eq!(saved.parent(stand).unwrap().key, other);
    assert!(saved.sibling_offset(stand, false).is_none());
    assert_eq!(
        std::fs::read(files.root.join("lamp.scene")).unwrap(),
        original_prefab
    );
    app.world_mut().write_message(Action::OpenBrowser);
    warm(&mut app, 180);
    assert!(
        app.world()
            .resource::<super::thumbnails::Thumbnails>()
            .scene_images
            .len()
            >= 2
    );
    capture_when(
        &mut app,
        &target,
        &files.root.join("hierarchy-browser.png"),
        ui_pixels,
    );
    println!("Hierarchy GPU artifacts: {}", files.root.display());
}

#[test]
#[ignore = "requires GPU adapter; verifies lighting in the actual editor viewport"]
fn gpu_editor_viewport_local_light_changes_objects() {
    let output =
        std::env::temp_dir().join(format!("bevy-editor-viewport-light-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    let (mut app, _, target) = fixture();
    app.world_mut().resource_mut::<Editor>().selection = None;
    warm(&mut app, 16);
    capture_when(&mut app, &target, &output.join("original.png"), |_| true);
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::Add(LightPreset::Bulb)));
    warm(&mut app, 16);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let light = editor.edits.lights.last_mut().unwrap();
        light.pose.position = [0.0, 2.0, 2.0];
        light.temperature = None;
        light.color = [0.0, 0.0, 1.0];
        light.enabled = false;
        editor.selection = None;
    }
    warm(&mut app, 16);
    let baseline = capture_when(&mut app, &target, &output.join("baseline.png"), |_| true);
    app.world_mut()
        .resource_mut::<Editor>()
        .edits
        .lights
        .last_mut()
        .unwrap()
        .enabled = true;
    warm(&mut app, 16);
    let lit = capture_when(&mut app, &target, &output.join("blue.png"), |_| true);
    let point = {
        let world = app.world_mut();
        let (camera, transform) = world
            .query_filtered::<(&Camera, &GlobalTransform), With<super::viewport::EditorCamera>>()
            .single(world)
            .unwrap();
        camera
            .world_to_viewport(transform, Vec3::new(0.0, 0.6, 0.4))
            .unwrap()
            + Vec2::new(0.0, ui::TOP)
    };
    let sample = |image: &image::RgbaImage| {
        let mut sum = [0.0; 3];
        for y in point.y as u32 - 4..point.y as u32 + 4 {
            for x in point.x as u32 - 4..point.x as u32 + 4 {
                for (total, channel) in sum.iter_mut().zip(image.get_pixel(x, y).0) {
                    *total += channel as f32 / 64.0;
                }
            }
        }
        sum
    };
    println!(
        "Viewport lighting artifacts: {} baseline={:?} blue={:?}",
        output.display(),
        sample(&baseline),
        sample(&lit)
    );
    assert!(
        sample(&lit)[2] > sample(&baseline)[2] + 10.0,
        "Default local light must visibly affect the actual scene"
    );
}

fn labeled_rect(app: &mut App, wanted: &str) -> Rect {
    let world = app.world_mut();
    let mut query = world.query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>();
    let (_, node, transform) = query
        .iter(world)
        .find(|(label, node, _)| label.0 == wanted && !node.is_empty())
        .unwrap_or_else(|| panic!("No visible UI control: {wanted}"));
    let center = transform.affine().translation;
    Rect::from_corners(center - node.size() * 0.5, center + node.size() * 0.5)
}

#[test]
#[ignore = "requires GPU adapter; clicks the actual scene-tree header and captures collapsed/restored panes"]
fn gpu_scene_tree_header_collapses_the_list_and_restores_the_split() {
    let (mut app, window, target) = fixture();
    let before = app.world().resource::<Editor>().edits.clone();
    let history = app.world().resource::<Editor>().undo.len();
    let inspector = labeled_rect(&mut app, "Инспектор свойств");
    let tree = labeled_rect(&mut app, "Список объектов сцены");
    let header = labeled_rect(&mut app, "Дерево сцены");
    assert!(tree.height() > 100.0);
    move_pointer(&mut app, window, header.center());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 4);
    assert!(app.world().resource::<Editor>().sidebar.tree_collapsed());
    {
        let world = app.world_mut();
        let mut query = world.query::<(&AccessibleLabel, &ComputedNode, &Node)>();
        let (_, computed, node) = query
            .iter(world)
            .find(|(label, ..)| label.0 == "Список объектов сцены")
            .unwrap();
        assert!(computed.is_empty());
        assert_eq!(node.display, Display::None);
    }
    let expanded_inspector = labeled_rect(&mut app, "Инспектор свойств");
    assert!(expanded_inspector.height() > inspector.height() + 100.0);
    assert!((expanded_inspector.min.y - header.max.y).abs() < 2.0);
    assert!((expanded_inspector.max.y - inspector.max.y).abs() < 2.0);
    labeled_rect(&mut app, "+ Свет");
    let output =
        std::env::temp_dir().join(format!("bevy-editor-tree-collapse-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    capture_when(&mut app, &target, &output.join("collapsed.png"), ui_pixels);
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 4);
    assert!(!app.world().resource::<Editor>().sidebar.tree_collapsed());
    let restored_tree = labeled_rect(&mut app, "Список объектов сцены");
    let restored_inspector = labeled_rect(&mut app, "Инспектор свойств");
    assert!((restored_tree.height() - tree.height()).abs() < 2.0);
    assert!((restored_inspector.height() - inspector.height()).abs() < 2.0);
    assert_eq!(app.world().resource::<Editor>().edits, before);
    assert_eq!(app.world().resource::<Editor>().undo.len(), history);
    capture_when(&mut app, &target, &output.join("expanded.png"), ui_pixels);
    println!("Scene tree collapse screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU adapter; clicks the space toggle and captures the actual world/local axes"]
fn gpu_space_toggle_rotates_the_rendered_gizmo_without_moving_the_object() {
    let (mut app, window, target) = fixture();
    let selection = Selection::Imported(0);
    app.world_mut()
        .write_message(Action::Tool(crate::editor::Tool::Universal));
    app.world_mut().write_message(Action::Select(selection));
    app.world_mut()
        .write_message(Action::Tool(crate::editor::Tool::Move));
    warm(&mut app, 4);
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.set_pose(
            selection,
            Transform::from_xyz(0.0, 0.7, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        );
        let pivot = Vec3::new(0.0, 0.7, 0.0);
        editor.view = Transform::from_translation(pivot + Vec3::new(4.0, 3.0, 6.0))
            .looking_at(pivot, Vec3::Y);
        editor.pivot = pivot;
    }
    warm(&mut app, 12);
    let before = app.world().resource::<Editor>().edits.clone();
    let history = app.world().resource::<Editor>().undo.len();
    assert!(!app.world().resource::<Editor>().local_space);
    let toggle = labeled_rect(&mut app, "Использовать локальное пространство");
    assert!(toggle.min.y >= 0.0 && toggle.max.y <= ui::TOP);
    let output = std::env::temp_dir().join(format!("bevy-editor-spaces-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    let sample = |app: &mut App, axis: Vec3| {
        let world = app.world_mut();
        let mut query = world.query_filtered::<(&Camera, &Transform, &GlobalTransform), With<crate::editor::viewport::EditorCamera>>();
        let (camera, view, global) = query.single(world).unwrap();
        let pivot = Vec3::new(0.0, 0.7, 0.0);
        let origin = camera.world_to_viewport(global, pivot).unwrap();
        let end = camera
            .world_to_viewport(
                global,
                pivot + axis * crate::editor::viewport::length(camera, view, pivot),
            )
            .unwrap();
        (end - (end - origin).normalize() * 8.0 + Vec2::Y * ui::TOP)
            .round()
            .as_uvec2()
    };
    let red = |image: &image::RgbaImage, point: UVec2| {
        let pixel = image.get_pixel(point.x, point.y).0;
        pixel[0] > 180
            && pixel[0] > pixel[1].saturating_add(60)
            && pixel[0] > pixel[2].saturating_add(60)
    };
    let global_sample = sample(&mut app, Vec3::X);
    capture_when(&mut app, &target, &output.join("global.png"), |image| {
        red(image, global_sample)
    });
    move_pointer(&mut app, window, toggle.center());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    assert!(app.world().resource::<Editor>().local_space);
    let local_sample = sample(&mut app, Vec3::NEG_Z);
    assert!(local_sample.as_vec2().distance(global_sample.as_vec2()) > 30.0);
    capture_when(&mut app, &target, &output.join("local.png"), |image| {
        red(image, local_sample)
    });
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    let editor = app.world().resource::<Editor>();
    assert!(!editor.local_space);
    assert_eq!(editor.edits, before);
    assert_eq!(editor.undo.len(), history);
    println!("World/local space screenshots: {}", output.display());
}

#[test]
#[ignore = "requires GPU; exercises filtering, folding and actual property editing in the 3D inspector"]
fn gpu_scene_inspector_filters_and_reverts_properties() {
    use crate::editor::area_selection::Mode;
    use bevy::{input_focus::InputFocus, text::EditableText};

    let files = crate::editor::scene_hierarchy::tests::fixture();
    let source = std::fs::read(files.root.join("lamp.scene")).unwrap();
    let (mut app, window, target) = fixture_project(files.project.clone());
    let (mesh, a, b) = {
        let graph = app
            .world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap();
        (
            graph.by_id("a/mesh").unwrap().key,
            graph.by_id("a").unwrap().key,
            graph.by_id("b").unwrap().key,
        )
    };
    let click = |app: &mut App, label: &str| {
        let pos = labeled_rect(app, label).center();
        move_pointer(app, window, pos);
        mouse(app, window, ButtonState::Pressed);
        mouse(app, window, ButtonState::Released);
        warm(app, 2);
    };
    let search = |app: &mut App, query: &str| {
        click(app, "Поиск свойств");
        let field = app.world().resource::<InputFocus>().get().unwrap();
        let world = app.world_mut();
        assert_eq!(
            world.get::<AccessibleLabel>(field).unwrap().0,
            "Поиск свойств"
        );
        world
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text(query);
        warm(app, 3);
    };
    let send = |app: &mut App, action: Action| {
        app.world_mut().write_message(action);
        warm(app, 4);
    };
    send(&mut app, Action::Select(Selection::Model(mesh)));
    click(&mut app, "Пространство координат инспектора");
    assert!(app.world().resource::<Editor>().local_space);
    let search_rect = labeled_rect(&mut app, "Поиск свойств");
    assert!(visible_field(&mut app, Field::Position(0)));

    click(&mut app, "Трансформация");
    assert!(!visible_field(&mut app, Field::Position(0)));
    search(&mut app, "МАСШТАБ");
    assert!(visible_field(&mut app, Field::Scale(0)));
    assert!(!visible_field(&mut app, Field::Position(0)));
    assert!(!visible_field(&mut app, Field::Lens));
    click(&mut app, "Очистить поиск свойств");
    assert!(
        !visible_field(&mut app, Field::Scale(0)),
        "Clearing search restores the folded section"
    );
    click(&mut app, "Трансформация");
    assert!(visible_field(&mut app, Field::Position(0)));

    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/inspector-qa");
    std::fs::create_dir_all(&output).unwrap();
    send(&mut app, Action::Number(Field::Position(1), 1.3));
    send(&mut app, Action::Number(Field::Rotation(1), 35.0));
    let edited = {
        let editor = app.world().resource::<Editor>();
        editor
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .node(mesh)
            .unwrap()
            .local()
    };
    capture_when(
        &mut app,
        &target,
        &output.join("scene-transform.png"),
        ui_pixels,
    );
    click(&mut app, "Вернуть исходную позицию");
    {
        let editor = app.world().resource::<Editor>();
        let graph = editor.edits.hierarchy.as_ref().unwrap();
        let reverted = graph.node(mesh).unwrap().local();
        assert_eq!(reverted.position, [0.0, 0.5, 0.0]);
        assert_eq!(reverted.rotation, edited.rotation);
        assert!(graph.overridden(mesh));
    }
    scroll_to_label(&mut app, "Вернуть исходный поворот");
    assert_eq!(labeled_rect(&mut app, "Поиск свойств"), search_rect);
    click(&mut app, "Вернуть исходный поворот");
    assert!(
        !app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    send(&mut app, Action::Undo);
    assert!(
        app.world()
            .resource::<Editor>()
            .edits
            .hierarchy
            .as_ref()
            .unwrap()
            .overridden(mesh)
    );
    send(&mut app, Action::Redo);
    send(&mut app, Action::Save);
    assert_eq!(
        std::fs::read(files.root.join("lamp.scene")).unwrap(),
        source
    );
    assert!(
        !crate::LevelEdits::<String>::read(&files.root, "room.scene")
            .unwrap()
            .hierarchy
            .unwrap()
            .overridden(mesh)
    );

    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        editor.select_targets(vec![Selection::Node(a), Selection::Node(b)], Mode::Replace);
    }
    warm(&mut app, 4);
    assert!(visible_field(&mut app, Field::Position(0)));
    click(&mut app, "Позиция X");
    let field = app.world().resource::<InputFocus>().get().unwrap();
    assert_eq!(
        app.world()
            .get::<EditableText>(field)
            .unwrap()
            .value()
            .to_string(),
        "—"
    );
    app.world_mut()
        .get_mut::<EditableText>(field)
        .unwrap()
        .editor_mut()
        .set_text("2,5");
    let history = app.world().resource::<Editor>().undo.len();
    // Clicking the search field follows the normal blur-to-commit path.
    click(&mut app, "Поиск свойств");
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.undo.len(), history + 1);
    for key in [a, b] {
        assert_eq!(
            editor
                .edits
                .hierarchy
                .as_ref()
                .unwrap()
                .node(key)
                .unwrap()
                .local()
                .position[0],
            2.5
        );
    }

    send(&mut app, Action::Select(Selection::Camera(0)));
    search(&mut app, "fov");
    assert!(visible_field(&mut app, Field::Lens));
    assert!(!visible_field(&mut app, Field::Position(0)));
    capture_when(
        &mut app,
        &target,
        &output.join("camera-search.png"),
        ui_pixels,
    );

    send(
        &mut app,
        Action::Lighting(LightingAction::Add(LightPreset::Spot)),
    );
    assert!(matches!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Light(_))
    ));
    search(&mut app, "смещение глубины");
    assert!(visible_field(
        &mut app,
        Field::Light(LightField::ShadowDepthBias)
    ));
    assert!(!visible_field(
        &mut app,
        Field::Light(LightField::Intensity)
    ));
    capture_when(
        &mut app,
        &target,
        &output.join("light-search.png"),
        ui_pixels,
    );
}

fn visible_field(app: &mut App, wanted: Field) -> bool {
    let world = app.world_mut();
    let mut query = world.query::<(
        &Field,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
    )>();
    query.iter(world).any(|(field, node, transform, clip)| {
        *field == wanted
            && node.size().min_element() > 0.0
            && clip.is_none_or(|clip| clip.contains_point(transform.affine().translation))
    })
}

fn scroll_to_field(app: &mut App, wanted: Field) {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &Field)>();
    let (entity, _) = query
        .iter(world)
        .find(|(_, field)| **field == wanted)
        .expect("Expected lighting control is missing");
    scroll_to_entity(app, entity);
}

fn scroll_to_label(app: &mut App, wanted: &str) {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &AccessibleLabel)>();
    let (entity, _) = query
        .iter(world)
        .find(|(_, label)| label.0 == wanted)
        .expect("Expected color control is missing");
    scroll_to_entity(app, entity);
}

pub(in crate::editor) fn scroll_to_entity(app: &mut App, entity: Entity) {
    let world = app.world_mut();
    let center = world
        .get::<UiGlobalTransform>(entity)
        .unwrap()
        .affine()
        .translation;
    let mut current = entity;
    let scroller = loop {
        let Some(parent) = world.get::<ChildOf>(current).map(ChildOf::parent) else {
            // Splitters live outside the scrollable inspector.
            return;
        };
        if world.get::<ui::Scroller>(parent).is_some() {
            break parent;
        }
        current = parent;
    };
    let node = world.get::<ComputedNode>(scroller).unwrap();
    let transform = world.get::<UiGlobalTransform>(scroller).unwrap();
    let top = transform.affine().translation.y - node.size().y * 0.5;
    let bottom = transform.affine().translation.y + node.size().y * 0.5;
    let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor()).max(0.0);
    let delta = if center.y < top + 40.0 {
        center.y - top - 40.0
    } else {
        (center.y - bottom + 45.0).max(0.0)
    };
    let mut position = world.get_mut::<ScrollPosition>(scroller).unwrap();
    position.y = (position.y + delta).clamp(0.0, max);
    warm(app, 4);
}

pub(in crate::editor) fn move_pointer(app: &mut App, window: Entity, position: Vec2) {
    let old = app
        .world()
        .get::<Window>(window)
        .unwrap()
        .cursor_position()
        .unwrap_or(position);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(position));
    app.world_mut().write_message(CursorMoved {
        window,
        position,
        delta: None,
    });
    pointer(
        app,
        window,
        position,
        PointerAction::Move {
            delta: position - old,
        },
    );
    warm(app, 2);
}

pub(in crate::editor) fn mouse(app: &mut App, window: Entity, state: ButtonState) {
    app.world_mut().write_message(MouseButtonInput {
        window,
        button: MouseButton::Left,
        state,
    });
    let position = app
        .world()
        .get::<Window>(window)
        .unwrap()
        .cursor_position()
        .unwrap();
    pointer(
        app,
        window,
        position,
        if state == ButtonState::Pressed {
            PointerAction::Press(PointerButton::Primary)
        } else {
            PointerAction::Release(PointerButton::Primary)
        },
    );
    warm(app, 2);
}

fn pointer(app: &mut App, window: Entity, position: Vec2, action: PointerAction) {
    let target = {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&RenderTarget, With<IsDefaultUiCamera>>();
        cameras
            .single(world)
            .unwrap()
            .normalize(Some(window))
            .unwrap()
    };
    app.world_mut().write_message(PointerInput {
        pointer_id: PointerId::Mouse,
        location: Location { target, position },
        action,
    });
}

fn drag_control(app: &mut App, window: Entity, label: &str, start: Vec2, end: Vec2) {
    scroll_to_label(app, label);
    let rect = labeled_rect(app, label);
    move_pointer(app, window, rect.min + rect.size() * start);
    mouse(app, window, ButtonState::Pressed);
    move_pointer(app, window, rect.min + rect.size() * end);
    mouse(app, window, ButtonState::Released);
}

fn ui_pixels(image: &image::RgbaImage) -> bool {
    ui_pixels_from(image, WIDTH - ui::SIDE as u32)
}

fn ui_pixels_from(image: &image::RgbaImage, left: u32) -> bool {
    let mut text_pixels = 0;
    for y in ui::TOP as u32..image.height() - ui::FOOT as u32 {
        for x in left..image.width() {
            let [r, g, b, _] = image.get_pixel(x, y).0;
            if r > 170 && g > 160 && b > 150 {
                text_pixels += 1;
            }
        }
    }
    text_pixels > 180
}

#[test]
#[ignore = "requires GPU adapter; exercises the real HDRI inspector controls"]
fn gpu_hdri_environment_picker_and_sliders_work_in_the_editor() {
    let output = std::env::temp_dir().join(format!("bevy-editor-hdri-ui-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("HDRI UI artifacts: {}", output.display());
    let (mut app, window, target) = fixture();
    assert_eq!(
        app.world().resource::<Editor>().environment().hdri,
        Some(crate::EnvironmentPreset::Cloudy)
    );
    assert!(!app.world().resource::<Editor>().dirty());
    capture_when(
        &mut app,
        &target,
        &output.join("00-default-cloudy.png"),
        |image| {
            ui_pixels(image)
                && image
                    .enumerate_pixels()
                    .filter(|(x, y, pixel)| {
                        let [red, green, blue, _] = pixel.0;
                        *x < 960
                            && *y > 150
                            && *y < 850
                            && red > 230
                            && green > 130
                            && blue < 180
                            && red.saturating_sub(green) > 15
                            && green.saturating_sub(blue) > 20
                    })
                    .count()
                    > 2000
        },
    );
    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    warm(&mut app, 8);
    for preset in crate::EnvironmentPreset::ALL {
        let rect = labeled_rect(&mut app, &format!("HDRI: {}", preset.label()));
        assert!(
            rect.width() > 10.0,
            "Every built-in environment must have a visible choice"
        );
    }
    scroll_to_label(&mut app, "HDRI: Студия");
    let rect = labeled_rect(&mut app, "HDRI: Студия");
    move_pointer(&mut app, window, rect.center());
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(
        app.world().resource::<Editor>().environment().hdri,
        Some(crate::EnvironmentPreset::SmallStudio)
    );
    capture_when(
        &mut app,
        &target,
        &output.join("01-environments.png"),
        ui_pixels,
    );
    let history = app.world().resource::<Editor>().undo.len();
    drag_control(
        &mut app,
        window,
        "Поворот HDRI",
        Vec2::new(0.5, 0.5),
        Vec2::new(0.8, 0.5),
    );
    assert!(app.world().resource::<Editor>().environment().hdri_rotation > 80.0);
    assert_eq!(app.world().resource::<Editor>().undo.len(), history + 1);
    let original_brightness = app
        .world()
        .resource::<Editor>()
        .environment()
        .hdri_brightness;
    drag_control(
        &mut app,
        window,
        "Яркость HDRI",
        Vec2::new(0.6, 0.5),
        Vec2::new(0.7, 0.5),
    );
    assert!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_brightness
            > 2000.0
    );
    capture_when(
        &mut app,
        &target,
        &output.join("02-rotation-brightness.png"),
        ui_pixels,
    );
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 4);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_brightness,
        original_brightness
    );
}

#[test]
#[ignore = "requires GPU adapter; verifies the HDRI panorama behind the actual editor scene"]
fn gpu_hdri_panorama_background_rotates_hides_and_restores_with_undo() {
    use crate::editor::{viewport::EditorCamera, viewport_background::Background};
    use bevy::light::{EnvironmentMapLight, Skybox};
    let output = std::env::temp_dir().join(format!(
        "bevy-editor-hdri-background-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&output).unwrap();
    println!("HDRI panorama artifacts: {}", output.display());
    let (mut app, _, target) = fixture();
    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetEnvironment(None)));
    warm(&mut app, 4);
    let camera = app
        .world_mut()
        .query_filtered::<Entity, With<EditorCamera>>()
        .single(app.world())
        .unwrap();
    let backdrop = app
        .world_mut()
        .query_filtered::<Entity, With<Background>>()
        .single(app.world())
        .unwrap();
    let background_difference = |a: &image::RgbaImage, b: &image::RgbaImage| {
        let mut total = 0_u64;
        let mut count = 0_u64;
        for y in 150..210 {
            for x in 80..800 {
                for (a, b) in a.get_pixel(x, y).0[..3]
                    .iter()
                    .zip(&b.get_pixel(x, y).0[..3])
                {
                    total += u64::from(a.abs_diff(*b));
                    count += 1;
                }
            }
        }
        total as f64 / count as f64
    };
    let baseline = capture_when(
        &mut app,
        &target,
        &output.join("01-gradient.png"),
        ui_pixels,
    );
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetEnvironment(Some(
            crate::EnvironmentPreset::Forest,
        ))));
    let forest = capture_when(
        &mut app,
        &target,
        &output.join("02-forest-panorama.png"),
        |image| ui_pixels(image) && background_difference(image, &baseline) > 15.0,
    );
    assert!(app.world().get::<Skybox>(camera).unwrap().image.is_some());
    assert_eq!(
        app.world().get::<Visibility>(backdrop),
        Some(&Visibility::Hidden)
    );
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetNumber(
            Field::Environment(EnvironmentField::HdriRotation),
            120.0,
        )));
    let rotated = capture_when(
        &mut app,
        &target,
        &output.join("03-rotated-panorama.png"),
        |image| background_difference(image, &forest) > 8.0,
    );
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::ToggleHdriBackground));
    capture_when(
        &mut app,
        &target,
        &output.join("04-hidden-panorama.png"),
        |image| background_difference(image, &baseline) < 0.5,
    );
    assert!(app.world().get::<Skybox>(camera).is_none());
    assert!(app.world().get::<EnvironmentMapLight>(camera).is_some());
    assert_eq!(
        app.world().get::<Visibility>(backdrop),
        Some(&Visibility::Visible)
    );
    app.world_mut().write_message(Action::Undo);
    capture_when(
        &mut app,
        &target,
        &output.join("05-undo-panorama.png"),
        |image| background_difference(image, &rotated) < 0.5,
    );
    assert!(app.world().get::<Skybox>(camera).is_some());
    assert!(
        app.world()
            .resource::<Editor>()
            .environment()
            .hdri_background
    );
}

#[test]
#[ignore = "requires GPU adapter; writes editor layout screenshots under /tmp"]
fn gpu_sidebar_resize_changes_panes_and_keeps_the_rendered_viewport_aligned() {
    let output =
        PathBuf::from("/tmp").join(format!("bevy-editor-sidebar-resize-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Sidebar resize GPU artifacts: {}", output.display());
    let (mut app, window, target) = fixture();
    let initial = app.world().resource::<Editor>().edits.clone();
    let original_side = labeled_rect(&mut app, "Правая панель");
    let original_inspector = labeled_rect(&mut app, "Инспектор свойств");
    capture_when(
        &mut app,
        &target,
        &output.join("01-default-layout.png"),
        ui_pixels,
    );

    drag_control(
        &mut app,
        window,
        "Изменить ширину правой панели",
        Vec2::splat(0.5),
        Vec2::new(0.5 - 180.0 / 6.0, 0.5),
    );
    let side = labeled_rect(&mut app, "Правая панель");
    assert!((side.width() - original_side.width() - 180.0).abs() < 1.0);
    let viewport = {
        let editor = app.world().resource::<Editor>();
        ui::viewport_rect(app.world().get::<Window>(window).unwrap(), &editor.sidebar)
    };
    assert!(
        (viewport.max.x - side.min.x).abs() < 1.0,
        "The scene and sidebar must meet at the same border"
    );
    let surface = &app.world().resource::<crate::editor::viewport::Surface>().0;
    let size = app
        .world()
        .resource::<Assets<Image>>()
        .get(surface)
        .unwrap()
        .texture_descriptor
        .size;
    assert_eq!(size.width, viewport.width().round() as u32);
    assert_eq!(size.height, viewport.height().round() as u32);
    capture_when(
        &mut app,
        &target,
        &output.join("02-wide-sidebar.png"),
        |image| ui_pixels_from(image, side.min.x as u32),
    );

    drag_control(
        &mut app,
        window,
        "Изменить высоту инспектора",
        Vec2::splat(0.5),
        Vec2::new(0.5, 0.5 - 200.0 / 6.0),
    );
    let inspector = labeled_rect(&mut app, "Инспектор свойств");
    assert!((inspector.height() - original_inspector.height() - 200.0).abs() < 1.0);
    let editor = app.world().resource::<Editor>();
    assert_eq!(
        editor.edits, initial,
        "Layout changes must not edit scene data"
    );
    assert!(editor.undo.is_empty());
    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    warm(&mut app, 4);
    assert!(
        (labeled_rect(&mut app, "Инспектор свойств").height() - inspector.height()).abs() < 1.0,
        "Selecting lighting must preserve the user's panel split"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("03-taller-inspector.png"),
        |image| ui_pixels_from(image, side.min.x as u32),
    );

    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(1000.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: 1000,
            height: 620,
            depth_or_array_layers: 1,
        });
    warm(&mut app, 5);
    let small = labeled_rect(&mut app, "Инспектор свойств");
    assert!(small.height() < inspector.height());
    assert!(small.max.y <= 620.0 - ui::FOOT + 1.0);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(WIDTH as f32, HEIGHT as f32);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        });
    warm(&mut app, 5);
    assert!(
        (labeled_rect(&mut app, "Инспектор свойств").height() - inspector.height()).abs() < 1.0
    );
    assert!((labeled_rect(&mut app, "Правая панель").width() - side.width()).abs() < 1.0);
    capture_when(
        &mut app,
        &target,
        &output.join("04-restored-layout.png"),
        |image| ui_pixels_from(image, side.min.x as u32),
    );

    // Passing between independent splitters must not leave a resize cursor
    // installed over the scene after both controls release it.
    use bevy::window::{CursorIcon, SystemCursorIcon};
    move_pointer(&mut app, window, Vec2::new(100.0, 200.0));
    let original_cursor = app.world().get::<CursorIcon>(window).cloned();
    app.world_mut().write_message(Action::Drawer);
    warm(&mut app, 20);
    let width_grip = labeled_rect(&mut app, "Изменить ширину правой панели");
    move_pointer(
        &mut app,
        window,
        Vec2::new(width_grip.center().x, ui::TOP + 50.0),
    );
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::EwResize))
    );
    let drawer_top =
        HEIGHT as f32 - ui::FOOT - app.world().resource::<ui::DrawerMotion>().visible_height;
    move_pointer(&mut app, window, Vec2::new(400.0, drawer_top + 3.0));
    assert_eq!(
        app.world().get::<CursorIcon>(window),
        Some(&CursorIcon::from(SystemCursorIcon::NsResize))
    );
    move_pointer(&mut app, window, Vec2::new(100.0, 200.0));
    assert_eq!(
        app.world().get::<CursorIcon>(window).cloned(),
        original_cursor
    );
}

#[test]
#[ignore = "requires GPU adapter; writes full editor UI screenshots under /tmp"]
fn gpu_lighting_menu_and_inspectors_fit_the_real_editor() {
    let output =
        PathBuf::from("/tmp").join(format!("bevy-editor-lighting-ui-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Lighting UI GPU artifacts: {}", output.display());
    let (mut app, window, target) = fixture();

    app.world_mut().write_message(Action::LightMenu);
    warm(&mut app, 8);
    assert!(app.world().resource::<ui::LightingUi>().is_menu_open());
    capture_when(
        &mut app,
        &target,
        &output.join("01-add-light-menu.png"),
        ui_pixels,
    );
    let menu = labeled_rect(&mut app, "Добавление света и схемы освещения");
    let add = labeled_rect(&mut app, "+ Свет");
    assert!(add.min.x >= WIDTH as f32 - ui::SIDE && add.min.y >= ui::TOP);
    assert!(
        menu.min.y >= add.max.y + 4.0,
        "The light menu must open below the scene-tree button"
    );
    assert!(menu.min.x >= 0.0 && menu.max.x <= WIDTH as f32);
    assert!(menu.min.y >= ui::TOP - 3.0 && menu.max.y <= HEIGHT as f32 - ui::FOOT);
    let mut cards = Vec::new();
    for label in ["Точечный", "Прожектор", "Солнце", "Панель"] {
        let card = labeled_rect(&mut app, &format!("Добавить: {label}"));
        assert!(
            card.min.x > menu.min.x && card.max.x < menu.max.x,
            "Type card spills outside the popover: {label}"
        );
        assert!(card.min.y >= menu.min.y && card.max.y < menu.max.y);
        for previous in &cards {
            let previous: &Rect = previous;
            assert!(
                previous.max.x <= card.min.x || card.max.x <= previous.min.x,
                "Light type cards overlap"
            );
        }
        cards.push(card);
    }

    app.world_mut()
        .write_message(Action::Lighting(LightingAction::Add(LightPreset::Spot)));
    warm(&mut app, 8);
    assert!(!app.world().resource::<ui::LightingUi>().is_menu_open());
    assert!(matches!(
        app.world().resource::<Editor>().selection,
        Some(Selection::Light(_))
    ));
    let name = labeled_rect(&mut app, "Имя источника света");
    assert!(name.min.x >= WIDTH as f32 - ui::SIDE && name.max.x <= WIDTH as f32);
    capture_when(
        &mut app,
        &target,
        &output.join("02-spot-inspector.png"),
        ui_pixels,
    );
    let color = labeled_rect(&mut app, "Цвет источника: насыщенность и яркость");
    assert!(color.min.x >= WIDTH as f32 - ui::SIDE && color.max.x <= WIDTH as f32);
    assert!(visible_field(&mut app, Field::Light(LightField::Intensity)));
    let intensity_before = app.world().resource::<Editor>().edits.lights[0].intensity;
    let history_before = app.world().resource::<Editor>().undo.len();
    drag_control(
        &mut app,
        window,
        "Интенсивность",
        Vec2::new(0.45, 0.5),
        Vec2::new(0.6, 0.5),
    );
    let editor = app.world().resource::<Editor>();
    assert!(
        editor.edits.lights[0].intensity > intensity_before * 2.0,
        "The real slider must edit the light"
    );
    assert_eq!(
        editor.undo.len(),
        history_before + 1,
        "A pointer drag must be one undo action"
    );
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 2);
    assert_eq!(
        app.world().resource::<Editor>().edits.lights[0].intensity,
        intensity_before
    );

    app.world_mut()
        .write_message(Action::Lighting(LightingAction::SetColor([255; 3])));
    warm(&mut app, 2);
    drag_control(
        &mut app,
        window,
        "Цвет источника: оттенок",
        Vec2::new(0.66, 0.5),
        Vec2::new(0.66, 0.5),
    );
    drag_control(
        &mut app,
        window,
        "Цвет источника: насыщенность и яркость",
        Vec2::new(0.6, 0.4),
        Vec2::new(0.8, 0.2),
    );
    let editor = app.world().resource::<Editor>();
    let [red, green, blue] = editor.edits.lights[0].color;
    assert!(
        blue > red + 0.4 && blue > green + 0.4,
        "The hue bar and color plane must select blue"
    );
    assert!(editor.edits.lights[0].temperature.is_none());
    capture_when(
        &mut app,
        &target,
        &output.join("02-color-picker-edited.png"),
        ui_pixels,
    );
    scroll_to_field(&mut app, Field::Light(LightField::OuterAngle));
    assert!(visible_field(
        &mut app,
        Field::Light(LightField::InnerAngle)
    ));
    assert!(visible_field(
        &mut app,
        Field::Light(LightField::OuterAngle)
    ));
    capture_when(
        &mut app,
        &target,
        &output.join("03-spot-cone-settings.png"),
        ui_pixels,
    );

    app.world_mut()
        .write_message(Action::Select(Selection::Environment));
    warm(&mut app, 8);
    scroll_to_field(&mut app, Field::Environment(EnvironmentField::Exposure));
    assert!(visible_field(
        &mut app,
        Field::Environment(EnvironmentField::Exposure)
    ));
    assert!(!visible_field(
        &mut app,
        Field::Light(LightField::OuterAngle)
    ));
    let environment_image = capture_when(
        &mut app,
        &target,
        &output.join("04-environment-inspector.png"),
        ui_pixels,
    );
    let plane = labeled_rect(&mut app, "Цвет фонового света: насыщенность и яркость");
    let sample_y = (plane.min.y + plane.height() * 0.1) as u32;
    let left = environment_image
        .get_pixel((plane.min.x + 4.0) as u32, sample_y)
        .0;
    let right = environment_image
        .get_pixel((plane.max.x - 4.0) as u32, sample_y)
        .0;
    assert!(
        left[0] > 200 && left[1] > 180 && left[2] > 180,
        "The color plane must fill its pointer area, including the white edge"
    );
    assert!(
        right[0] > 200 && right[1] < 40 && right[2] < 40,
        "The color plane must fill its pointer area, including the saturated edge"
    );
    app.world_mut()
        .write_message(Action::Lighting(LightingAction::ToggleFog));
    warm(&mut app, 3);
    let ambient = app.world().resource::<Editor>().environment().ambient_color;
    drag_control(
        &mut app,
        window,
        "Цвет тумана: оттенок",
        Vec2::new(0.66, 0.5),
        Vec2::new(0.66, 0.5),
    );
    drag_control(
        &mut app,
        window,
        "Цвет тумана: насыщенность и яркость",
        Vec2::new(0.7, 0.2),
        Vec2::new(0.8, 0.1),
    );
    let environment = app.world().resource::<Editor>().environment();
    let [red, green, blue] = environment.fog.as_ref().unwrap().color;
    assert!(blue > red + 0.4 && blue > green + 0.4);
    assert_eq!(
        environment.ambient_color, ambient,
        "Fog color must not edit ambient light"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-fog-color-picker.png"),
        ui_pixels,
    );

    app.world_mut()
        .write_message(Action::Lighting(LightingAction::Add(LightPreset::Softbox)));
    warm(&mut app, 8);
    scroll_to_field(&mut app, Field::Light(LightField::Height));
    assert!(visible_field(&mut app, Field::Light(LightField::Width)));
    assert!(visible_field(&mut app, Field::Light(LightField::Height)));
    assert!(!visible_field(
        &mut app,
        Field::Light(LightField::ShadowNear)
    ));
    capture_when(
        &mut app,
        &target,
        &output.join("05-rect-light-inspector.png"),
        ui_pixels,
    );

    // The real popover must remain inside the window and scroll on short layouts.
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(WIDTH as f32, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: WIDTH,
            height: 620,
            depth_or_array_layers: 1,
        });
    app.world_mut().write_message(Action::LightMenu);
    warm(&mut app, 8);
    let short_menu = labeled_rect(&mut app, "Добавление света и схемы освещения");
    assert!(short_menu.max.y <= 620.0 - ui::FOOT);
    let last_choice = labeled_rect(
        &mut app,
        "Показывать значки и контуры источников в редакторе. Это настройка интерфейса, она не меняет освещение сцены.",
    );
    assert!(
        last_choice.min.y >= short_menu.min.y && last_choice.max.y <= short_menu.max.y,
        "All light choices should remain reachable in a short window"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("06-short-window-menu.png"),
        ui_pixels,
    );
}

#[test]
#[ignore = "requires GPU adapter; captures real drag placement and Retina coordinates under /tmp"]
fn gpu_dragged_model_preview_tracks_cursor_and_release_matches_the_preview() {
    use crate::editor::{
        asset_placement::{PlacementGhost, PlacementPreview},
        browser::Browser,
        viewport::EditorCamera,
    };
    let output =
        PathBuf::from("/tmp").join(format!("bevy-editor-placement-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    println!("Placement GPU artifacts: {}", output.display());
    let (mut app, window, target) = fixture();
    let initial = app.world().resource::<Editor>().edits.clone();
    app.world_mut().write_message(Action::Drawer);
    let models = app
        .world()
        .resource::<Browser>()
        .folders
        .iter()
        .position(|folder| folder.path == std::path::Path::new("models"))
        .unwrap();
    app.world_mut().write_message(Action::Browse(models));
    warm(&mut app, 24);
    let card_label = "Открыть модель block.glb · Блок";

    fn cursor_at(app: &mut App, window: Entity, point: Vec3) -> Vec2 {
        let world = app.world_mut();
        let mut query = world.query_filtered::<(&Camera, &GlobalTransform), With<EditorCamera>>();
        let (camera, transform) = query.single(world).unwrap();
        let cursor = camera.world_to_viewport(transform, point).unwrap() + Vec2::new(0.0, ui::TOP);
        let editor = world.resource::<Editor>();
        assert!(
            ui::scene_input_rect(
                world.get::<Window>(window).unwrap(),
                world.resource::<ui::DrawerMotion>(),
                &editor.sidebar
            )
            .contains(cursor),
            "Fixture drop point is behind UI: {cursor:?}"
        );
        cursor
    }
    fn begin(app: &mut App, window: Entity, label: &str, cursor: Vec2) {
        let scale = app.world().get::<Window>(window).unwrap().scale_factor();
        let card = labeled_rect(app, label).center() / scale;
        move_pointer(app, window, card);
        mouse(app, window, ButtonState::Pressed);
        move_pointer(app, window, cursor);
        for _ in 0..240 {
            frame(app);
            if app
                .world()
                .resource::<PlacementPreview>()
                .position()
                .is_some()
            {
                break;
            }
        }
        assert!(
            app.world().resource::<ui::AssetDrag>().active(),
            "The real file-card gesture did not start"
        );
        assert!(
            app.world()
                .resource::<PlacementPreview>()
                .position()
                .is_some(),
            "Preview geometry did not load: {}",
            app.world().resource::<Editor>().status
        );
    }
    fn no_ghosts(app: &mut App) {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<PlacementGhost>>();
        assert_eq!(query.iter(world).count(), 0);
    }
    let first = cursor_at(&mut app, window, Vec3::new(-3.0, 0.0, -5.0));
    begin(&mut app, window, card_label, first);
    let a = app
        .world()
        .resource::<PlacementPreview>()
        .position()
        .unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("00-initial-preview.png"),
        ui_pixels,
    );
    {
        let world = app.world_mut();
        let mut roots = world.query_filtered::<Entity, With<PlacementGhost>>();
        let root = roots.single(world).unwrap();
        let mut meshes = world.query::<(Entity, &MeshMaterial3d<StandardMaterial>)>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        let mut count = 0;
        for (entity, handle) in meshes.iter(world) {
            let mut ancestor = Some(entity);
            let mut belongs_to_preview = false;
            while let Some(entity) = ancestor {
                if entity == root {
                    belongs_to_preview = true;
                    break;
                }
                ancestor = world.get::<ChildOf>(entity).map(ChildOf::parent);
            }
            if belongs_to_preview {
                let material = materials.get(&handle.0).unwrap();
                let color = material.base_color.to_srgba();
                assert!(
                    color.red > color.green && color.green > color.blue,
                    "The GLB's warm material must stay warm"
                );
                assert!((color.alpha - 0.8).abs() < 1e-6);
                assert!(!material.unlit);
                count += 1;
            }
        }
        assert!(count > 0);
    }
    assert!(
        a.abs_diff_eq(Vec3::new(-3.0, 0.0, -5.0), 0.003),
        "Unexpected placement {a:?}; contact {:?}",
        app.world().resource::<PlacementPreview>().contact()
    );
    assert_eq!(app.world().resource::<Editor>().edits, initial);
    capture_when(
        &mut app,
        &target,
        &output.join("01-preview-left.png"),
        ui_pixels,
    );
    let second = cursor_at(&mut app, window, Vec3::new(3.0, 0.0, -5.0));
    move_pointer(&mut app, window, second);
    warm(&mut app, 4);
    let b = app
        .world()
        .resource::<PlacementPreview>()
        .position()
        .unwrap();
    assert!(
        a.distance(b) > 3.5,
        "The preview must move with the pointer"
    );
    assert!(b.abs_diff_eq(Vec3::new(3.0, 0.0, -5.0), 0.003));
    assert_eq!(app.world().resource::<Editor>().edits, initial);
    assert!(app.world().resource::<Editor>().undo.is_empty());
    capture_when(
        &mut app,
        &target,
        &output.join("02-preview-right.png"),
        ui_pixels,
    );
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 8);
    {
        let editor = app.world().resource::<Editor>();
        assert_eq!(editor.edits.models.len(), 1);
        assert!(Vec3::from(editor.edits.models[0].pose.position).abs_diff_eq(b, 0.003));
        assert!(editor.edits.models[0].bounds.is_some());
        assert_eq!(editor.undo.len(), 1);
    }
    no_ghosts(&mut app);
    capture_when(&mut app, &target, &output.join("03-placed.png"), ui_pixels);

    let top = cursor_at(&mut app, window, Vec3::new(-2.0, 1.0, 0.0));
    begin(&mut app, window, card_label, top);
    assert!(
        (app.world()
            .resource::<PlacementPreview>()
            .contact()
            .unwrap()
            .y
            - 1.0)
            .abs()
            < 0.003
    );
    capture_when(
        &mut app,
        &target,
        &output.join("04-on-object.png"),
        ui_pixels,
    );
    app.world_mut()
        .write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: bevy::input::keyboard::Key::Escape,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
    frame(&mut app);
    app.world_mut()
        .write_message(bevy::input::keyboard::KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: bevy::input::keyboard::Key::Escape,
            state: ButtonState::Released,
            text: None,
            repeat: false,
            window,
        });
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    no_ghosts(&mut app);

    // Image targets and pointer coordinates use independent scale factors.
    // Recreate Retina while preserving the same logical editor dimensions.
    {
        let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
        window.resolution.set_scale_factor_override(Some(2.0));
        window.resolution.set(WIDTH as f32, HEIGHT as f32);
    }
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: WIDTH * 2,
            height: HEIGHT * 2,
            depth_or_array_layers: 1,
        });
    {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&mut RenderTarget, With<IsDefaultUiCamera>>();
        *cameras.single_mut(world).unwrap() =
            RenderTarget::Image(bevy::camera::ImageRenderTarget {
                handle: target.clone(),
                scale_factor: 2.0,
            });
    }
    app.world_mut().resource_mut::<Editor>().snap = true;
    warm(&mut app, 12);
    let retina = cursor_at(&mut app, window, Vec3::new(-2.13, 0.0, -4.78));
    begin(&mut app, window, card_label, retina);
    let expected = app
        .world()
        .resource::<PlacementPreview>()
        .position()
        .unwrap();
    assert!(
        expected.abs_diff_eq(Vec3::new(-2.25, 0.0, -4.75), 0.003),
        "Retina drop must retain logical coordinates and grid snapping: {expected:?}"
    );
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 4);
    let placed = app.world().resource::<Editor>().edits.models.clone();
    assert_eq!(placed.len(), 2);
    assert!(Vec3::from(placed[1].pose.position).abs_diff_eq(expected, 0.003));
    no_ghosts(&mut app);
    app.world_mut().write_message(Action::Undo);
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<Editor>().edits.models.len(), 1);
    app.world_mut().write_message(Action::Redo);
    warm(&mut app, 4);
    assert_eq!(app.world().resource::<Editor>().edits.models, placed);
}
