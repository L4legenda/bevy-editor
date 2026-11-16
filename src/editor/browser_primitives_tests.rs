use super::*;

fn create(root: &Path, parent: &Path, kind: Primitive) -> Result<PathBuf, String> {
    super::create(root, parent, kind, kind.label(), &kind.defaults())
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "bevy-browser-primitives-{}",
            bevy::asset::uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(path.join("Meshes")).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let Ok(path) = self.0.canonicalize() else {
            return;
        };
        let temp = std::env::temp_dir().canonicalize().unwrap();
        if path.parent() == Some(temp.as_path())
            && path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("bevy-browser-primitives-")
        {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[test]
fn browser_primitives_all_eight_export_valid_glb_with_editable_polygon_topology() {
    let files = Files::new();
    for kind in Primitive::ALL {
        let path = create(&files.0, Path::new("Meshes"), kind).unwrap();
        let bytes = std::fs::read(files.0.join(&path)).unwrap();
        let gltf = bevy::gltf::gltf::Gltf::from_slice(&bytes).unwrap();
        assert_eq!(gltf.nodes().next().unwrap().name(), Some(kind.label()));
        let blob = gltf.blob.as_ref().unwrap();
        let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
        let reader = primitive.reader(|_| Some(blob.as_slice()));
        let positions: Vec<_> = reader.read_positions().unwrap().collect();
        assert!(positions.len() >= 3);
        assert!(positions.iter().flatten().all(|v| v.is_finite()));
        assert!(
            reader
                .read_indices()
                .unwrap()
                .into_u32()
                .all(|index| (index as usize) < positions.len())
        );
        let source = Glb::load(&files.0.join(path)).unwrap();
        let polygons = source.json["meshes"][0]["extras"]["bevy_editor_topology"]["primitives"][0]
            .as_array()
            .unwrap();
        assert!(!polygons.is_empty());
        if kind == Primitive::Cube {
            assert_eq!(polygons.len(), 6);
            assert!(
                polygons
                    .iter()
                    .all(|face| face.as_array().unwrap().len() == 4)
            );
        }
        if matches!(kind, Primitive::Plane | Primitive::Circle) {
            assert!(positions.iter().all(|p| p[1].abs() < 0.00001));
        }
    }
}

#[test]
fn browser_primitives_create_unique_names_and_reject_paths_outside_assets() {
    let files = Files::new();
    let parent = Path::new("Meshes");
    let first = create(&files.0, parent, Primitive::Cube).unwrap();
    let bytes = std::fs::read(files.0.join(&first)).unwrap();
    let second = create(&files.0, parent, Primitive::Cube).unwrap();
    assert_eq!(first, PathBuf::from("Meshes/Куб.glb"));
    assert_eq!(second, PathBuf::from("Meshes/Куб (2).glb"));
    assert_eq!(std::fs::read(files.0.join(first)).unwrap(), bytes);
    assert!(create(&files.0, Path::new("../outside"), Primitive::Torus).is_err());
    assert!(create(&files.0, Path::new("missing"), Primitive::Torus).is_err());
}

#[test]
fn browser_primitives_named_parameters_control_dimensions_and_reject_invalid_values() {
    let files = Files::new();
    let mut cube = Primitive::Cube.defaults();
    cube.size = Vec3::new(3.5, 2.25, 4.0);
    let path = super::create(
        &files.0,
        Path::new("Meshes"),
        Primitive::Cube,
        "Мой блок.GLB",
        &cube,
    )
    .unwrap();
    assert_eq!(path, PathBuf::from("Meshes/Мой блок.glb"));
    let source = Glb::load(&files.0.join(&path)).unwrap();
    assert_eq!(source.json["nodes"][0]["name"], "Мой блок");
    let accessor = source.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
        .as_u64()
        .unwrap() as usize;
    for (axis, size) in cube.size.to_array().into_iter().enumerate() {
        let bounds = &source.json["accessors"][accessor];
        assert!(
            ((bounds["max"][axis].as_f64().unwrap() - bounds["min"][axis].as_f64().unwrap())
                as f32
                - size)
                .abs()
                < 0.00001
        );
    }
    let mut sphere = Primitive::UvSphere.defaults();
    sphere.radius = 2.5;
    sphere.segments = 12;
    sphere.rings = 8;
    let geometry = Primitive::UvSphere.geometry(&sphere).unwrap();
    assert!(
        geometry
            .positions
            .iter()
            .all(|p| (p.length() - 2.5).abs() < 0.00001)
    );
    let mut finer = sphere;
    finer.segments = 24;
    assert!(Primitive::UvSphere.geometry(&finer).unwrap().faces.len() > geometry.faces.len());
    let mut invalid = Primitive::Torus.defaults();
    invalid.minor_radius = invalid.major_radius;
    assert!(invalid.validate(Primitive::Torus).is_err());
    invalid = Primitive::Torus.defaults();
    invalid.segments = 256;
    invalid.rings = 256;
    assert!(invalid.validate(Primitive::Torus).is_err());
    cube.size.x = 0.0;
    assert!(
        super::create(
            &files.0,
            Path::new("Meshes"),
            Primitive::Cube,
            "Некорректный",
            &cube
        )
        .is_err()
    );
    assert!(!files.0.join("Meshes/Некорректный.glb").exists());
    assert!(
        super::create(
            &files.0,
            Path::new("Meshes"),
            Primitive::Cube,
            "../outside",
            &Primitive::Cube.defaults()
        )
        .is_err()
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires GPU; clicks the cascading menu, creates all meshes and loads a new editable GLB"]
fn gpu_browser_primitives_cascade_create_and_open_editable_models() {
    use super::super::primitive_preview::{PreviewCamera, PreviewModel};
    use crate::editor::model_primitives::Parameter;
    use crate::editor::{
        selection_outline::render_tests::{capture_when, frame},
        tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
        ui::model_viewer::editing::State as ModelState,
    };
    use bevy::{
        camera::primitives::MeshAabb,
        input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
            mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel},
        },
    };

    fn copy(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target.join(entry.file_name()));
            } else if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
            }
        }
    }
    fn warm(app: &mut App, count: usize) {
        for _ in 0..count {
            frame(app);
        }
    }
    fn rect(app: &App, entity: Entity) -> Rect {
        let world = app.world();
        let node = world.get::<ComputedNode>(entity).unwrap();
        Rect::from_center_size(
            world
                .get::<UiGlobalTransform>(entity)
                .unwrap()
                .affine()
                .translation
                * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    }
    fn right_click(app: &mut App, window: Entity, point: Vec2) {
        move_pointer(app, window, point);
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(MouseButtonInput {
                window,
                button: MouseButton::Right,
                state,
            });
            warm(app, 2);
        }
    }
    fn escape(app: &mut App, window: Entity) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                window,
                key_code: KeyCode::Escape,
                logical_key: Key::Escape,
                state,
                text: None,
                repeat: false,
            });
            warm(app, 2);
        }
    }
    fn enter(app: &mut App, window: Entity) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(KeyboardInput {
                window,
                key_code: KeyCode::Enter,
                logical_key: Key::Enter,
                state,
                text: None,
                repeat: false,
            });
            warm(app, 2);
        }
    }
    fn set(app: &mut App, field: Entity, value: &str) {
        app.world_mut()
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text(value);
    }
    fn action(app: &mut App, kind: Primitive) -> Entity {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find_map(|(entity, control)| {
                matches!(control.action, Action::BrowserCreatePrimitive(value) if value == kind)
                    .then_some(entity)
            })
            .unwrap()
    }
    fn preview(app: &mut App) -> (Entity, Handle<Mesh>) {
        let world = app.world_mut();
        let camera = world
            .query_filtered::<Entity, With<PreviewCamera>>()
            .single(world)
            .unwrap();
        let mesh = world
            .query_filtered::<&Mesh3d, With<PreviewModel>>()
            .single(world)
            .unwrap()
            .0
            .clone();
        (camera, mesh)
    }
    fn visible_pixels(image: &image::RgbaImage, bounds: Rect) -> usize {
        let min = bounds.min.ceil().as_uvec2() + UVec2::splat(3);
        let max = bounds.max.floor().as_uvec2() - UVec2::splat(3);
        (min.y..max.y)
            .flat_map(|y| (min.x..max.x).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let [r, g, b, _] = image.get_pixel(x, y).0;
                r > 100 && g > 100 && b > 100
            })
            .count()
    }
    fn preview_size(app: &App, mesh: &Handle<Mesh>) -> Vec3 {
        Vec3::from(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(mesh)
                .unwrap()
                .get_aabb()
                .unwrap()
                .half_extents,
        ) * 2.0
    }
    let files = Files::new();
    let mut project = crate::EditorProject::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    copy(&project.asset_root, &files.0);
    project.asset_root = files.0.clone();
    let (mut app, window, target) = fixture_project(project);
    let edits = app.world().resource::<Editor>().edits.clone();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.drawer = true;
            browser.refresh(&mut editor.project);
            let folder = browser
                .folders
                .iter()
                .position(|folder| folder.path == Path::new("Meshes"))
                .unwrap();
            browser.navigate(folder);
        });
    warm(&mut app, 30);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/browser-primitives-qa");
    std::fs::create_dir_all(&output).unwrap();
    let trigger = app.world().resource::<ContextUi>().primitives.trigger;
    let submenu = app.world().resource::<ContextUi>().primitives.root;
    let menu = app.world().resource::<ContextUi>().menu;
    let grid = app.world().resource::<drawer::BrowserUi>().grid;
    for kind in Primitive::ALL {
        let anchor = rect(&app, grid).min + Vec2::splat(24.0);
        right_click(&mut app, window, anchor);
        assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
        let point = rect(&app, trigger).center();
        move_pointer(&mut app, window, point);
        warm(&mut app, 3);
        assert!(app.world().resource::<MenuState>().open);
        assert!(rect(&app, submenu).min.x >= rect(&app, menu).max.x);
        if kind == Primitive::Plane {
            capture_when(&mut app, &target, &output.join("01-submenu.png"), |_| true);
        }
        let button = action(&mut app, kind);
        let point = rect(&app, button).center();
        move_pointer(&mut app, window, point);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        assert!(app.world().resource::<BrowserContext>().phase == Phase::Primitive(kind));
        assert!(
            !files
                .0
                .join("Meshes")
                .join(format!("{}.glb", kind.label()))
                .exists(),
            "Opening the dialog must not create an asset"
        );
        let (name, submit, dialog) = {
            let ui = app.world().resource::<ContextUi>();
            (ui.name, ui.submit, ui.dialog)
        };
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(name));
        assert!(rect(&app, dialog).max.y < 900.0);
        let (camera, mesh) = preview(&mut app);
        assert!(app.world().get::<Camera>(camera).unwrap().is_active);
        let expected = kind.mesh(&kind.defaults()).unwrap();
        let actual = app.world().resource::<Assets<Mesh>>().get(&mesh).unwrap();
        assert_eq!(actual.get_aabb(), expected.get_aabb());
        assert_eq!(
            actual
                .indices()
                .map_or(actual.count_vertices(), |indices| indices.len()),
            expected
                .indices()
                .map_or(expected.count_vertices(), |indices| indices.len())
        );
        let pane = app.world().resource::<ContextUi>().parameters.preview;
        let area = rect(&app, pane);
        capture_when(
            &mut app,
            &target,
            &output.join(format!("preview-{kind:?}.png")),
            |image| visible_pixels(image, area) > 1200,
        );
        if kind == Primitive::Cone {
            capture_when(
                &mut app,
                &target,
                &output.join("05-cone-parameters.png"),
                |_| true,
            );
        }
        let point = rect(&app, submit).center();
        move_pointer(&mut app, window, point);
        mouse(&mut app, window, ButtonState::Pressed);
        mouse(&mut app, window, ButtonState::Released);
        for _ in 0..180 {
            frame(&mut app);
            let context = app.world().resource::<BrowserContext>();
            if context.pending.is_none()
                && context.queue.is_empty()
                && files
                    .0
                    .join("Meshes")
                    .join(format!("{}.glb", kind.label()))
                    .exists()
            {
                break;
            }
        }
        assert!(
            files
                .0
                .join("Meshes")
                .join(format!("{}.glb", kind.label()))
                .is_file()
        );
        assert!(!app.world().resource::<Editor>().browser_popup);
        assert!(!app.world().get::<Camera>(camera).unwrap().is_active);
        let index = app
            .world()
            .resource::<drawer::BrowserUi>()
            .selected_model
            .expect("New GLB must be selected");
        assert_eq!(
            app.world().resource::<Editor>().project.catalog[index].asset,
            PathBuf::from("Meshes")
                .join(format!("{}.glb", kind.label()))
                .to_string_lossy()
                .replace('\\', "/")
        );
        warm(&mut app, 4);
    }
    warm(&mut app, 60);
    capture_when(
        &mut app,
        &target,
        &output.join("02-created-models.png"),
        |_| true,
    );

    // Clamp the cascade inside the window when opened beside its right edge.
    let anchor = Vec2::new(1250.0, rect(&app, grid).min.y + 24.0);
    right_click(&mut app, window, anchor);
    let point = rect(&app, trigger).center();
    move_pointer(&mut app, window, point);
    warm(&mut app, 3);
    let bounds = rect(&app, submenu);
    assert!(bounds.min.x >= 8.0 && bounds.max.x <= 1272.0);
    assert!(bounds.max.y <= 892.0);
    assert!(bounds.max.x <= rect(&app, menu).min.x);
    capture_when(&mut app, &target, &output.join("03-edge-menu.png"), |_| {
        true
    });
    escape(&mut app, window);
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Menu);
    assert!(!app.world().resource::<MenuState>().open);
    escape(&mut app, window);
    assert!(!app.world().resource::<Editor>().browser_popup);

    // Custom names, decimal commas, cross-field checks and integer segment validation.
    app.world_mut()
        .write_message(Action::BrowserCreatePrimitive(Primitive::Torus));
    warm(&mut app, 3);
    let (preview_camera, preview_mesh) = preview(&mut app);
    let original_bounds = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&preview_mesh)
        .unwrap()
        .get_aabb();
    let (name, major, minor, segments, rings) = {
        let ui = app.world().resource::<ContextUi>();
        (
            ui.name,
            ui.parameters.input(Parameter::MajorRadius),
            ui.parameters.input(Parameter::MinorRadius),
            ui.parameters.input(Parameter::Segments),
            ui.parameters.input(Parameter::Rings),
        )
    };
    set(&mut app, name, "../outside");
    enter(&mut app, window);
    assert!(!app.world().resource::<BrowserContext>().error.is_empty());
    assert!(app.world().resource::<BrowserContext>().phase == Phase::Primitive(Primitive::Torus));
    set(&mut app, name, "Кольцо для двери.GLB");
    set(&mut app, major, "0,2");
    set(&mut app, minor, "0,4");
    enter(&mut app, window);
    assert!(
        app.world()
            .resource::<BrowserContext>()
            .error
            .contains("Большой радиус")
    );
    assert_eq!(
        app.world()
            .resource::<Assets<Mesh>>()
            .get(&preview_mesh)
            .unwrap()
            .get_aabb(),
        original_bounds
    );
    set(&mut app, major, "1,75");
    set(&mut app, minor, "0,35");
    set(&mut app, segments, "2.5");
    enter(&mut app, window);
    assert!(
        app.world()
            .resource::<BrowserContext>()
            .error
            .contains("целое")
    );
    set(&mut app, segments, "12");
    set(&mut app, rings, "8");
    warm(&mut app, 2);
    assert!(app.world().resource::<BrowserContext>().error.is_empty());
    let intermediate = preview_size(&app, &preview_mesh);
    assert!(intermediate.x > 2.5 && intermediate.x < 4.2);
    assert!(intermediate.y > 0.5 && intermediate.y < 0.7);
    warm(&mut app, 20);
    let mesh = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&preview_mesh)
        .unwrap();
    assert_eq!(mesh.indices().unwrap().len(), 12 * 8 * 6);
    let bounds = mesh.get_aabb().unwrap();
    assert!((bounds.half_extents.x - 2.1).abs() < 0.00001);
    assert!((bounds.half_extents.y - 0.35).abs() < 0.00001);
    let model = {
        let world = app.world_mut();
        world
            .query_filtered::<Entity, With<PreviewModel>>()
            .single(world)
            .unwrap()
    };
    assert_eq!(
        *app.world()
            .get::<bevy::camera::primitives::Aabb>(model)
            .unwrap(),
        bounds
    );
    // Incomplete typing keeps the last valid surface, then resumes immediately.
    set(&mut app, segments, "");
    warm(&mut app, 2);
    assert_eq!(
        app.world()
            .resource::<Assets<Mesh>>()
            .get(&preview_mesh)
            .unwrap()
            .indices()
            .unwrap()
            .len(),
        12 * 8 * 6
    );
    set(&mut app, segments, "12");
    warm(&mut app, 2);
    let pane = app.world().resource::<ContextUi>().parameters.preview;
    let center = rect(&app, pane).center();
    let before = *app.world().get::<Transform>(preview_camera).unwrap();
    move_pointer(&mut app, window, center);
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, center + Vec2::new(55.0, 24.0));
    mouse(&mut app, window, ButtonState::Released);
    let rotated = *app.world().get::<Transform>(preview_camera).unwrap();
    assert!(before.rotation.angle_between(rotated.rotation) > 0.1);
    assert!((before.translation.length() - rotated.translation.length()).abs() < 0.0001);
    app.world_mut().write_message(MouseWheel {
        window,
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: 2.0,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
    warm(&mut app, 2);
    let zoomed = *app.world().get::<Transform>(preview_camera).unwrap();
    assert!(zoomed.translation.length() < rotated.translation.length() * 0.9);
    assert_eq!(
        preview(&mut app).1,
        preview_mesh,
        "Editing must reuse the preview mesh asset"
    );
    capture_when(
        &mut app,
        &target,
        &output.join("06-custom-torus-parameters.png"),
        |_| true,
    );
    // Confirm during a retargeted transition: the file must contain the input
    // values rather than the intermediate preview surface.
    set(&mut app, major, "3");
    warm(&mut app, 4);
    set(&mut app, major, "1,75");
    warm(&mut app, 1);
    assert!(preview_size(&app, &preview_mesh).x > 4.2);
    enter(&mut app, window);
    let custom = files.0.join("Meshes/Кольцо для двери.glb");
    for _ in 0..180 {
        frame(&mut app);
        if custom.exists() && app.world().resource::<BrowserContext>().pending.is_none() {
            break;
        }
    }
    let bytes = std::fs::read(&custom).unwrap();
    let gltf = bevy::gltf::gltf::Gltf::from_slice(&bytes).unwrap();
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let reader = primitive.reader(|_| gltf.blob.as_deref());
    assert_eq!(
        reader.read_indices().unwrap().into_u32().count(),
        12 * 8 * 6
    );
    let positions: Vec<_> = reader.read_positions().unwrap().collect();
    let max_x = positions
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = positions
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((max_x - 2.1).abs() < 0.00001);
    assert!((max_y - 0.35).abs() < 0.00001);
    assert_eq!(
        gltf.nodes().next().unwrap().name(),
        Some("Кольцо для двери")
    );

    let count = std::fs::read_dir(files.0.join("Meshes")).unwrap().count();
    app.world_mut()
        .write_message(Action::BrowserCreatePrimitive(Primitive::Cube));
    warm(&mut app, 3);
    let name = app.world().resource::<ContextUi>().name;
    assert_eq!(
        app.world()
            .get::<EditableText>(name)
            .unwrap()
            .value()
            .to_string(),
        "Куб (2)"
    );
    set(&mut app, name, "Отменённая фигура");
    for (field, value) in [
        (Parameter::SizeX, "3.5"),
        (Parameter::SizeY, "2.25"),
        (Parameter::SizeZ, "4"),
    ] {
        let input = app.world().resource::<ContextUi>().parameters.input(field);
        set(&mut app, input, value);
    }
    warm(&mut app, 4);
    let intermediate = preview_size(&app, &preview_mesh);
    assert!(intermediate.x > 2.0 && intermediate.x < 3.5);
    assert!(intermediate.y > 2.0 && intermediate.y < 2.25);
    assert!(intermediate.z > 2.0 && intermediate.z < 4.0);
    capture_when(
        &mut app,
        &target,
        &output.join("09-cube-transition.png"),
        |_| true,
    );
    let x = app
        .world()
        .resource::<ContextUi>()
        .parameters
        .input(Parameter::SizeX);
    let visible = preview_size(&app, &preview_mesh);
    set(&mut app, x, "5");
    warm(&mut app, 1);
    let retargeted = preview_size(&app, &preview_mesh);
    assert!(retargeted.x > visible.x && retargeted.x < visible.x + 0.15);
    set(&mut app, x, "3.5");
    warm(&mut app, 20);
    let size = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&preview_mesh)
        .unwrap()
        .get_aabb()
        .unwrap()
        .half_extents
        * 2.0;
    assert!((Vec3::from(size) - Vec3::new(3.5, 2.25, 4.0)).length() < 0.00001);
    capture_when(
        &mut app,
        &target,
        &output.join("08-cube-live-preview.png"),
        |_| true,
    );
    escape(&mut app, window);
    assert!(!app.world().resource::<Editor>().browser_popup);
    assert!(!app.world().get::<Camera>(preview_camera).unwrap().is_active);
    assert_eq!(
        std::fs::read_dir(files.0.join("Meshes")).unwrap().count(),
        count
    );

    let index = app
        .world()
        .resource::<Editor>()
        .project
        .catalog
        .iter()
        .position(|model| model.asset == "Meshes/Тор.glb")
        .unwrap();
    app.world_mut().write_message(Action::ViewModel(index));
    for _ in 0..240 {
        frame(&mut app);
        if app
            .world()
            .resource::<ModelState>()
            .documents
            .contains_key(&index)
        {
            break;
        }
    }
    assert!(
        app.world()
            .resource::<ModelState>()
            .documents
            .contains_key(&index),
        "Created GLB must open as editable geometry"
    );
    assert!(
        !app.world().resource::<ModelState>().documents[&index].objects[0]
            .geometry
            .faces
            .is_empty()
    );
    assert_eq!(app.world().resource::<Editor>().edits, edits);
    capture_when(
        &mut app,
        &target,
        &output.join("04-editable-torus.png"),
        |_| true,
    );
}
