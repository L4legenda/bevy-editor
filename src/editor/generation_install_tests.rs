use super::*;
use crate::editor::{
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture, mouse, move_pointer},
};
use bevy::input::ButtonState;

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
            .unwrap_or_else(|| panic!("Control missing: {label}"))
    };
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}
fn visible(app: &App, entity: Entity) -> bool {
    !app.world().get::<ComputedNode>(entity).unwrap().is_empty()
}

#[test]
#[ignore = "requires GPU; verifies missing model screen, real percentage binding, cancel, tab persistence and ready state"]
fn gpu_generation_model_missing_progress_cancel_and_ready() {
    let (mut app, window, target) = fixture();
    warm(&mut app, 45);
    click(&mut app, window, "Переключить 3D генерацию");
    app.world_mut().insert_resource(State {
        store: Some(Store {
            root: PathBuf::from("C:/Users/Test/AppData/Local/BevyEditor/system/models"),
        }),
        checked: true,
        ..default()
    });
    warm(&mut app, 4);
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/generation-install-qa");
    std::fs::create_dir_all(&output).unwrap();
    let (normal, panel, download, fill, amount) = {
        let ui = app.world().resource::<InstallUi>();
        (ui.normal, ui.panel, ui.download, ui.fill, ui.amount)
    };
    assert!(visible(&app, panel));
    assert!(visible(&app, download));
    assert!(!visible(&app, normal));
    capture_when(
        &mut app,
        &target,
        &output.join("01-model-missing.png"),
        |_| true,
    );
    let progress = Arc::new(Progress::default());
    progress.received.store(model::TOTAL / 4, Ordering::Relaxed);
    app.world_mut().resource_mut::<State>().job = Some(Job {
        progress: progress.clone(),
        task: IoTaskPool::get().spawn(std::future::pending()),
    });
    warm(&mut app, 4);
    assert!(!visible(&app, download));
    assert!(
        app.world()
            .get::<Text>(amount)
            .unwrap()
            .0
            .starts_with("25.0%")
    );
    assert!(
        matches!(app.world().get::<Node>(fill).unwrap().width,Val::Percent(v) if (v-25.0).abs()<0.01)
    );
    progress
        .received
        .store(model::TOTAL * 7 / 10, Ordering::Relaxed);
    warm(&mut app, 4);
    assert!(
        app.world()
            .get::<Text>(amount)
            .unwrap()
            .0
            .starts_with("70.0%")
    );
    capture_when(
        &mut app,
        &target,
        &output.join("02-model-downloading.png"),
        |_| true,
    );
    app.world_mut()
        .write_message(Action::Generation(GenerationAction::Close));
    warm(&mut app, 4);
    assert!(app.world().resource::<State>().job.is_some());
    click(&mut app, window, "Переключить 3D генерацию");
    assert!(app.world().resource::<State>().job.is_some());
    click(&mut app, window, "Отменить");
    assert!(progress.cancelled.load(Ordering::Relaxed));
    let cancel = app.world().resource::<InstallUi>().cancel;
    assert!(app.world().get::<InteractionDisabled>(cancel).is_some());
    {
        let mut state = app.world_mut().resource_mut::<State>();
        state.job = None;
        state.resumable = true;
        state.error = "Загрузка отменена. Можно продолжить позже.".into();
    }
    warm(&mut app, 4);
    assert!(visible(&app, download));
    capture_when(
        &mut app,
        &target,
        &output.join("03-model-resume.png"),
        |_| true,
    );
    app.world_mut().insert_resource(State::installed_fixture());
    warm(&mut app, 4);
    assert!(!visible(&app, panel));
    assert!(visible(&app, normal));
    let generate = {
        let world = app.world_mut();
        world
            .query::<(Entity, &Control)>()
            .iter(world)
            .find_map(|(e, c)| {
                matches!(c.action, Action::Generation(GenerationAction::Generate)).then_some(e)
            })
            .unwrap()
    };
    assert!(
        app.world().get::<InteractionDisabled>(generate).is_some(),
        "Model download does not pretend to connect generation"
    );
}
