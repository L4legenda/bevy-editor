use super::*;
use crate::editor::{
    RoomEdits,
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
};
use bevy::input::ButtonState;
use std::{
    fs,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Project(PathBuf);
impl Project {
    fn new() -> (Self, crate::EditorProject) {
        let mut project = crate::EditorProject::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "bevy-hunyuan-ui-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fn copy(source: &std::path::Path, destination: &std::path::Path) {
            fs::create_dir_all(destination).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &destination.join(entry.file_name()));
                } else {
                    fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
                }
            }
        }
        copy(&project.asset_root, &root);
        project.asset_root = root.clone();
        (Self(root), project)
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        if let (Ok(path), Ok(parent)) = (self.0.canonicalize(), std::env::temp_dir().canonicalize())
            && path.parent() == Some(parent.as_path())
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("bevy-hunyuan-ui-"))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}
fn warm(app: &mut App, n: usize) {
    for _ in 0..n {
        frame(app);
    }
}
fn click(app: &mut App, window: Entity, label: &str) {
    let point = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(l, n, _)| l.0 == label && !n.is_empty())
            .map(|(_, _, t)| t.translation)
            .unwrap_or_else(|| panic!("Missing control {label}"))
    };
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}
fn workflow(real: bool) {
    let (files, project) = Project::new();
    let (mut app, window, target) = fixture_project(project);
    warm(&mut app, 45);
    let before: RoomEdits = app.world().resource::<Editor>().edits.clone();
    click(&mut app, window, "Переключить 3D генерацию");
    if !real {
        app.world_mut().insert_resource(State::ready_fixture());
    }
    let image = PathBuf::from("G:/3D-AI-Lab/inputs/03_hair_no_background.png");
    assert!(
        image.is_file(),
        "Install the local lab input or change this ignored integration fixture"
    );
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let draft = &mut editor.generation;
        draft.source = Some(image);
        draft.name = "Generated Mini test".into();
        draft.steps = 20;
        draft.resolution = 128;
        draft.chunks = 8000;
        draft.target_faces = 8000;
        draft.obj = true;
    }
    let lifecycle_output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-native-qa");
    fs::create_dir_all(&lifecycle_output).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut start_requested = !real;
    loop {
        frame(&mut app);
        if !start_requested
            && app
                .world()
                .resource::<installation::State>()
                .found
                .is_some()
            && app.world().resource::<State>().runtime.is_some()
        {
            warm(&mut app, 3);
            let panel = app.world().resource::<ServiceUi>().panel;
            assert!(!app.world().get::<ComputedNode>(panel).unwrap().is_empty());
            capture_when(
                &mut app,
                &target,
                &lifecycle_output.join("01-model-off.png"),
                |_| true,
            );
            click(&mut app, window, "Включить 3D модель");
            assert!(app.world().resource::<Editor>().generation.engine_loading);
            capture_when(
                &mut app,
                &target,
                &lifecycle_output.join("02-model-loading.png"),
                |_| true,
            );
            start_requested = true;
        }
        if app.world().resource::<Editor>().generation.backend_ready
            && app.world().resource::<Editor>().generation.source_ready
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Model/runtime/source did not become ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    if real {
        warm(&mut app, 3);
        click(&mut app, window, "Сгенерировать 3D модель");
        assert!(app.world().resource::<Editor>().generation.running);
        let deadline = Instant::now() + Duration::from_secs(480);
        let mut last = String::new();
        while app.world().resource::<Editor>().generation.running {
            frame(&mut app);
            if let Some(job) = &app.world().resource::<State>().job {
                let status = job.progress.snapshot();
                let text = format!("{} {}/{}", status.stage, status.current, status.total);
                if text != last {
                    eprintln!("Hunyuan: {text}");
                    last = text;
                }
            }
            assert!(
                Instant::now() < deadline,
                "Timed out during real generation"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            app.world().resource::<State>().result.is_some(),
            "{}",
            app.world().resource::<State>().error
        );
        let directory = app
            .world()
            .resource::<State>()
            .result
            .as_ref()
            .unwrap()
            .directory
            .clone();
        fs::write(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-real-job.txt"),
            directory.to_string_lossy().as_bytes(),
        )
        .unwrap();
    } else {
        let directory = PathBuf::from(
            fs::read_to_string(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-real-job.txt"),
            )
            .expect("Run the worker or the real integration test first"),
        );
        let mut result: backend::ResultModel =
            serde_json::from_slice(&fs::read(directory.join("result.json")).unwrap()).unwrap();
        result.directory = directory;
        app.world_mut().resource_mut::<State>().result = Some(result);
    }
    warm(&mut app, 30);
    let result = app.world().resource::<State>().result.clone().unwrap();
    assert!(result.faces > 0 && result.vertices > 0);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-backend-qa");
    fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join(if real {
            "01-live-model.png"
        } else {
            "01-cached-model.png"
        }),
        |image| {
            image
                .enumerate_pixels()
                .filter(|(x, y, p)| {
                    *x > 420
                        && *x < 1220
                        && *y > 230
                        && *y < 650
                        && p.0[0] > 65
                        && p.0[1] > 65
                        && p.0[2] > 65
                })
                .count()
                > 1000
        },
    );
    let point = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>()
            .iter(world)
            .find(|(l, n, _)| l.0 == "3D предпросмотр результата генерации" && !n.is_empty())
            .map(|(_, _, t)| t.translation)
            .unwrap()
    };
    let before_camera = {
        let world = app.world_mut();
        world
            .query::<(&Camera, &Transform, &bevy::camera::visibility::RenderLayers)>()
            .iter(world)
            .find(|(_, _, layer)| **layer == bevy::camera::visibility::RenderLayers::layer(17))
            .map(|(_, pose, _)| *pose)
            .unwrap()
    };
    move_pointer(&mut app, window, point);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, point + Vec2::new(55.0, 20.0));
    mouse(&mut app, window, ButtonState::Released);
    warm(&mut app, 3);
    let after_camera = {
        let world = app.world_mut();
        world
            .query::<(&Camera, &Transform, &bevy::camera::visibility::RenderLayers)>()
            .iter(world)
            .find(|(_, _, layer)| **layer == bevy::camera::visibility::RenderLayers::layer(17))
            .map(|(_, pose, _)| *pose)
            .unwrap()
    };
    assert_ne!(
        before_camera, after_camera,
        "Dragging the preview must orbit the actual 3D camera"
    );
    move_pointer(&mut app, window, Vec2::new(20.0, 110.0));
    let solid = capture_when(&mut app, &target, &output.join("03-solid.png"), |_| true);
    click(&mut app, window, "Сетка");
    move_pointer(&mut app, window, Vec2::new(20.0, 110.0));
    assert!(app.world().resource::<Editor>().generation.wireframe);
    let different = |image: &image::RgbaImage| {
        image
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                *x > 430
                    && *x < 1210
                    && *y > 240
                    && *y < 640
                    && p.0
                        .iter()
                        .zip(solid.get_pixel(*x, *y).0)
                        .any(|(a, b)| a.abs_diff(b) > 30)
            })
            .count()
    };
    capture_when(
        &mut app,
        &target,
        &output.join("04-wireframe.png"),
        |image| different(image) > 800,
    );
    click(&mut app, window, "Сетка");
    move_pointer(&mut app, window, Vec2::new(20.0, 110.0));
    assert!(!app.world().resource::<Editor>().generation.wireframe);
    capture_when(
        &mut app,
        &target,
        &output.join("05-solid-restored.png"),
        |image| different(image) < 100,
    );
    // Import while the overlay is enabled: the exported asset must stay unchanged.
    click(&mut app, window, "Сетка");
    click(&mut app, window, "Переключить 3D генерацию");
    click(&mut app, window, "Переключить 3D генерацию");
    assert!(app.world().resource::<Editor>().generation.wireframe);
    click(&mut app, window, "Добавить в проект");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.world().resource::<State>().imported.is_none() {
        frame(&mut app);
        assert!(
            Instant::now() < deadline,
            "Import failed: {}",
            app.world().resource::<State>().error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let (relative, index) = app.world().resource::<State>().imported.clone().unwrap();
    assert!(relative.starts_with("models/generated"));
    assert_eq!(
        fs::read(files.0.join(&relative)).unwrap(),
        fs::read(result.glb()).unwrap()
    );
    assert!(files.0.join(relative.with_extension("obj")).is_file());
    assert_eq!(app.world().resource::<Editor>().edits, before);
    capture_when(
        &mut app,
        &target,
        &output.join("02-imported-model.png"),
        |_| true,
    );
    click(&mut app, window, "Открыть модель");
    warm(&mut app, 40);
    let editor = app.world().resource::<Editor>();
    assert_eq!(editor.viewing_model, Some(index));
    assert!(!editor.generation.active);
    assert_eq!(editor.edits, before);
    if real {
        assert!(
            app.world().resource::<State>().ready(),
            "Opening an imported model must retain the generator"
        );
        assert!(app.world().resource::<Editor>().generation.engine_active);
        capture_when(
            &mut app,
            &target,
            &lifecycle_output.join("03-service-outside-tab.png"),
            |_| true,
        );
        click(&mut app, window, "Остановить");
        assert!(!app.world().resource::<Editor>().generation.engine_active);
        assert!(app.world().resource::<State>().service.is_none());
        click(&mut app, window, "Переключить 3D генерацию");
        let panel = app.world().resource::<ServiceUi>().panel;
        assert!(!app.world().get::<ComputedNode>(panel).unwrap().is_empty());
        capture_when(
            &mut app,
            &target,
            &lifecycle_output.join("04-model-unloaded.png"),
            |_| true,
        );
    }
}

#[test]
#[ignore = "requires GPU and a real cached Hunyuan result; previews, orbits and imports the actual generated mesh"]
fn gpu_generation_cached_result_preview_and_project_import() {
    workflow(false);
}

#[test]
#[ignore = "requires local Hunyuan, CUDA and GPU; runs real image inference from the native button and imports GLB/OBJ"]
fn gpu_generation_live_hunyuan_image_to_project() {
    workflow(true);
}

#[test]
#[ignore = "requires GPU; verifies compact defaults, stable disclosure inputs, animated unknown stages and exact step progress without inference"]
fn gpu_generation_compact_sections_and_loading_activity() {
    let (mut app, window, target) = crate::editor::tests::lighting_ui_render::fixture();
    warm(&mut app, 45);
    click(&mut app, window, "Переключить 3D генерацию");
    app.world_mut()
        .insert_resource(installation::State::installed_fixture());
    let mut ready_state = State::default();
    ready_state.checked = true;
    ready_state.fixture_ready = true;
    ready_state.runtime = Some(backend::Runtime {
        executable: "fixture".into(),
    });
    app.world_mut().insert_resource(ready_state);
    warm(&mut app, 4);
    fn field(app: &mut App, kind: Field) -> Entity {
        let world = app.world_mut();
        world
            .query::<(Entity, &Field)>()
            .iter(world)
            .find_map(|(e, k)| (*k == kind).then_some(e))
            .unwrap()
    }
    fn visible(app: &App, entity: Entity) -> bool {
        !app.world().get::<ComputedNode>(entity).unwrap().is_empty()
    }
    let path = field(&mut app, Field::Path);
    let steps = field(&mut app, Field::Steps);
    let faces = field(&mut app, Field::Faces);
    let name = field(&mut app, Field::Name);
    assert!(!visible(&app, path));
    assert!(!visible(&app, steps));
    assert!(!visible(&app, faces));
    assert!(visible(&app, name));
    let before = app.world().resource::<Editor>().edits.clone();
    click(&mut app, window, "Путь…");
    assert!(visible(&app, path));
    app.world_mut()
        .get_mut::<EditableText>(path)
        .unwrap()
        .editor
        .set_text("C:/draft-image.png");
    click(&mut app, window, "Путь…");
    assert!(!visible(&app, path));
    click(&mut app, window, "Путь…");
    assert_eq!(
        app.world()
            .get::<EditableText>(path)
            .unwrap()
            .value()
            .to_string(),
        "C:/draft-image.png"
    );
    assert_eq!(
        field(&mut app, Field::Path),
        path,
        "Folding preserves the actual input entity and draft"
    );
    click(&mut app, window, "Путь…");
    click(&mut app, window, "Экспорт и сетка");
    assert!(visible(&app, faces));
    click(&mut app, window, "Экспорт и сетка");
    assert!(!visible(&app, faces));
    let system = app.world().resource::<GenerationUi>().system_settings;
    click(&mut app, window, "Система");
    assert!(visible(&app, system));
    click(&mut app, window, "Система");
    assert!(!visible(&app, system));
    assert_eq!(app.world().resource::<Editor>().edits, before);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-ux-qa");
    fs::create_dir_all(&output).unwrap();
    move_pointer(&mut app, window, Vec2::new(1220.0, 110.0));
    warm(&mut app, 4);
    capture_when(&mut app, &target, &output.join("01-compact.png"), |_| true);
    let progress = Arc::new(backend::Progress::default());
    progress.status.lock().unwrap().stage = "loading".into();
    app.world_mut().resource_mut::<State>().job = Some(Job {
        started: Instant::now(),
        progress: progress.clone(),
        task: IoTaskPool::get().spawn(std::future::pending()),
    });
    warm(&mut app, 4);
    let (bar, fill) = {
        let ui = app.world().resource::<RunUi>();
        (ui.bar, ui.fill)
    };
    assert!(
        visible(&app, bar),
        "The long weight-loading stage must show a bar"
    );
    assert!(matches!(
        app.world().get::<super::super::progress::Value>(fill),
        Some(super::super::progress::Value::Busy)
    ));
    let first = app.world().get::<Node>(fill).unwrap().left;
    warm(&mut app, 8);
    assert_ne!(
        first,
        app.world().get::<Node>(fill).unwrap().left,
        "Unknown progress must visibly move instead of looking stalled"
    );
    capture_when(&mut app, &target, &output.join("02-loading.png"), |_| true);
    {
        let mut stage = progress.status.lock().unwrap();
        stage.stage = "diffusion".into();
        stage.current = 9;
        stage.total = 30;
    }
    warm(&mut app, 4);
    assert!(
        matches!(app.world().get::<Node>(fill).unwrap().width,Val::Percent(value) if (value-30.0).abs()<0.01)
    );
    capture_when(&mut app, &target, &output.join("03-steps.png"), |_| true);
    for stage in ["conditioning", "surface", "optimizing", "exporting"] {
        *progress.status.lock().unwrap() = backend::Status {
            stage: stage.into(),
            ..default()
        };
        warm(&mut app, 3);
        assert!(
            visible(&app, bar),
            "Every processing stage needs an activity indicator"
        );
        assert!(matches!(
            app.world().get::<super::super::progress::Value>(fill),
            Some(super::super::progress::Value::Busy)
        ));
    }
    app.world_mut().resource_mut::<State>().job = None;
    warm(&mut app, 4);
    assert!(!visible(&app, bar));
    app.world_mut().resource_mut::<State>().probe =
        Some(IoTaskPool::get().spawn(std::future::pending()));
    warm(&mut app, 3);
    assert!(visible(&app, bar));
    app.world_mut().resource_mut::<State>().probe = None;
    app.world_mut().resource_mut::<State>().import_task =
        Some(IoTaskPool::get().spawn(std::future::pending()));
    warm(&mut app, 3);
    assert!(visible(&app, bar));
    assert_eq!(app.world().resource::<Editor>().edits, before);
}
