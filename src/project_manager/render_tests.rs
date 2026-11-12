//! Capture the actual manager UI at regular and minimum supported sizes.
use super::*;
use crate::editor::selection_outline::render_tests::{CapturedImage, capture_when, frame};
use bevy::{
    camera::RenderTarget,
    render::{
        RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{TextureFormat, TextureUsages},
    },
    text::EditableText,
    ui::{CalculatedClip, IsDefaultUiCamera, UiGlobalTransform},
    ui_widgets::Activate,
    window::{ExitCondition, PrimaryWindow},
    winit::WinitPlugin,
};

#[derive(Resource)]
struct Canvas(Handle<Image>);
fn retarget(canvas: Res<Canvas>, mut cameras: Query<&mut RenderTarget, With<IsDefaultUiCamera>>) {
    *cameras.single_mut().unwrap() = RenderTarget::Image(canvas.0.clone().into());
}
fn warm(app: &mut App) {
    for _ in 0..12 {
        frame(app);
    }
}
fn activate(app: &mut App, label: &str) {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &AccessibleLabel)>();
    let (entity, _) = query
        .iter(world)
        .find(|(_, name)| name.0 == label)
        .unwrap_or_else(|| panic!("Missing button: {label}"));
    world.trigger(Activate { entity });
    warm(app);
}
fn set(app: &mut App, field: ui::Input, value: &str) {
    let world = app.world_mut();
    let mut query = world.query::<(Entity, &ui::Input, &mut EditableText)>();
    let mut focused = None;
    for (entity, input, mut text) in query.iter_mut(world) {
        if *input == field {
            text.editor_mut().set_text(value);
            focused = Some(entity);
        }
    }
    if let Some(entity) = focused {
        world
            .resource_mut::<InputFocus>()
            .set(entity, bevy::input_focus::FocusCause::Pressed);
    }
    warm(app);
}
fn assert_controls_fit(app: &mut App, width: f32, height: f32) {
    let world = app.world_mut();
    let modal = world.resource::<Manager>().form.is_some();
    let mut query = world.query::<(
        Entity,
        &ui::Control,
        &AccessibleLabel,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&CalculatedClip>,
    )>();
    for (entity, control, label, computed, transform, clip) in query.iter(world) {
        if modal != control.modal
            || (modal && !["Отмена", "×", "Создать", "Открыть"].contains(&label.0.as_str()))
        {
            continue;
        }
        let center = transform.affine().translation;
        let bounds = Rect::from_center_size(center, computed.size());
        // Scrollable content may extend past its viewport; toolbar and dialog
        // footer controls must fit without scrolling.
        let mut parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
        let mut scrollable = false;
        while let Some(entity) = parent {
            scrollable |= world.get::<ui::ScrollArea>(entity).is_some();
            parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
        }
        if scrollable {
            continue;
        }
        assert!(
            bounds.min.x >= -1.0
                && bounds.min.y >= -1.0
                && bounds.max.x <= width + 1.0
                && bounds.max.y <= height + 1.0,
            "A manager control is outside the window: {bounds:?}"
        );
        assert!(
            clip.is_none_or(|clip| clip.contains_point(center)),
            "A manager control is clipped"
        );
    }
}
fn screenshot(app: &mut App, target: &Handle<Image>, output: &Path) {
    capture_when(app, target, output, |image| {
        image
            .pixels()
            .filter(|pixel| pixel.0[0] > 160 && pixel.0[1] > 150)
            .count()
            > 1500
    });
}

#[test]
#[ignore = "requires a GPU adapter; writes project-manager screenshots under /tmp"]
fn gpu_project_manager_creation_search_open_errors_and_minimum_size() {
    let scratch = Scratch::new();
    let output = PathBuf::from("/tmp").join(format!(
        "bevy-editor-project-manager-{}",
        std::process::id()
    ));
    fs::create_dir_all(&output).unwrap();
    println!("Project manager captures: {}", output.display());
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>(),
    )
    .init_resource::<CapturedImage>()
    .add_systems(Startup, retarget.after(ui::setup));
    let manager = Manager::load(
        scratch.0.join("history.json"),
        PathBuf::from("/usr/bin/true"),
    );
    install(&mut app, manager);
    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                resolution: (1100, 760).into(),
                focused: true,
                ..default()
            },
        ))
        .id();
    let mut image = Image::new_target_texture(1100, 760, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.insert_resource(Canvas(target.clone()));
    app.finish();
    app.cleanup();
    warm(&mut app);
    screenshot(&mut app, &target, &output.join("01-empty.png"));
    assert_controls_fit(&mut app, 1100.0, 760.0);
    activate(&mut app, "+ Новый проект");
    set(&mut app, ui::Input::Title, "Моя первая сцена");
    set(&mut app, ui::Input::Folder, &scratch.0.to_string_lossy());
    assert_controls_fit(&mut app, 1100.0, 760.0);
    screenshot(&mut app, &target, &output.join("02-create.png"));
    activate(&mut app, "Создать");
    assert!(app.world().resource::<Manager>().form.is_none());
    let file = scratch
        .0
        .join("Моя первая сцена/project.bevyeditor")
        .canonicalize()
        .unwrap();
    assert!(EditorProject::load(&file).is_ok());
    assert_eq!(app.world().resource::<Manager>().recent.projects.len(), 1);
    screenshot(&mut app, &target, &output.join("03-recent.png"));
    set(&mut app, ui::Input::Search, "no matching projects");
    assert!(app.world().resource::<Manager>().selected.is_none());
    screenshot(&mut app, &target, &output.join("04-search-empty.png"));
    set(&mut app, ui::Input::Search, "");
    assert!(app.world().resource::<Manager>().selected.is_some());
    activate(&mut app, "Открыть…");
    set(&mut app, ui::Input::Folder, &scratch.0.to_string_lossy());
    activate(&mut app, "Открыть");
    assert!(
        !app.world()
            .resource::<Manager>()
            .form
            .as_ref()
            .unwrap()
            .error
            .is_empty()
    );
    screenshot(&mut app, &target, &output.join("05-open-error.png"));
    activate(&mut app, "Отмена");
    {
        let mut manager = app.world_mut().resource_mut::<Manager>();
        manager.recent.remember(&file, "Моя первая сцена");
        manager.recent.projects[0].favorite = true;
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/project.json")
            .canonicalize()
            .unwrap();
        manager.recent.remember(&example, "Мастерская");
        manager.revision += 1;
    }
    warm(&mut app);
    screenshot(&mut app, &target, &output.join("08-project-list.png"));
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(880.0, 620.0);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(bevy::render::render_resource::Extent3d {
            width: 880,
            height: 620,
            depth_or_array_layers: 1,
        });
    warm(&mut app);
    screenshot(&mut app, &target, &output.join("06-minimum-size.png"));
    assert_controls_fit(&mut app, 880.0, 620.0);
    activate(&mut app, "+ Новый проект");
    warm(&mut app);
    screenshot(&mut app, &target, &output.join("07-create-minimum.png"));
    assert_controls_fit(&mut app, 880.0, 620.0);
    for (_, mut child) in app.world_mut().resource_mut::<Manager>().running.drain(..) {
        child.wait().unwrap();
    }
}
