use super::*;
use bevy::{
    camera::RenderTarget,
    input::keyboard::Key,
    picking::{
        backend::HitData,
        events::Pointer,
        pointer::{Location, PointerId},
    },
    ui_widgets::ButtonPlugin,
    window::WindowRef,
};

fn fixture_for(action: Option<Action>) -> (App, Entity, Entity, Entity) {
    let mut project = crate::project::EditorProject::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    let browser = Browser::new(&mut project);
    let folder = browser
        .folders
        .iter()
        .position(|folder| folder.label == "models")
        .unwrap();
    let mut app = App::new();
    app.add_plugins(ButtonPlugin)
        .insert_resource(Editor::new(project, default()))
        .insert_resource(browser)
        .init_resource::<AssetDrag>()
        .init_resource::<InputFocus>()
        .add_message::<Action>();
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let parent = app.world_mut().spawn_empty().id();
    let (card, sidebar) = {
        let mut commands = app.world_mut().commands();
        let card = control(
            &mut commands,
            parent,
            action.unwrap_or(Action::Browse(folder)),
            Look::Asset,
            Node::default(),
        );
        let sidebar = control(
            &mut commands,
            parent,
            Action::Browse(folder),
            Look::Regular,
            Node::default(),
        );
        (card, sidebar)
    };
    app.world_mut().flush();
    (app, window, card, sidebar)
}

fn fixture() -> (App, Entity, Entity, Entity) {
    fixture_for(None)
}

fn click(app: &mut App, window: Entity, entity: Entity, count: u8, button: PointerButton) {
    app.world_mut().entity_mut(entity).insert(Pressed);
    app.world_mut().trigger(PointerClick {
        entity,
        pointer: Pointer::new(
            PointerId::Mouse,
            Location {
                target: RenderTarget::Window(WindowRef::Entity(window))
                    .normalize(Some(window))
                    .unwrap(),
                position: Vec2::ZERO,
            },
        ),
        button,
        hit: HitData::new(window, 0.0, None, None),
        duration: std::time::Duration::from_millis(40),
        count,
    });
    app.world_mut().flush();
}

fn take_actions(app: &mut App) -> Vec<Action> {
    app.world_mut()
        .resource_mut::<Messages<Action>>()
        .drain()
        .collect()
}

fn assert_open(app: &mut App) {
    let actions = take_actions(app);
    assert!(matches!(actions.as_slice(), [Action::Browse(_)]));
}

#[test]
fn folder_cards_require_a_primary_double_click() {
    let (mut app, window, card, _) = fixture();
    for _ in 0..2 {
        click(&mut app, window, card, 1, PointerButton::Primary);
        assert!(take_actions(&mut app).is_empty());
    }
    click(&mut app, window, card, 2, PointerButton::Secondary);
    assert!(take_actions(&mut app).is_empty());
    click(&mut app, window, card, 2, PointerButton::Primary);
    assert_open(&mut app);
}

#[test]
fn folder_sidebar_keeps_single_click_navigation() {
    let (mut app, window, _, sidebar) = fixture();
    click(&mut app, window, sidebar, 1, PointerButton::Primary);
    assert_open(&mut app);
}

#[test]
fn folder_cards_keep_enter_and_space_activation() {
    let (mut app, window, card, _) = fixture();
    for key_code in [KeyCode::Enter, KeyCode::Space] {
        for (state, repeat, opens) in [
            (ButtonState::Pressed, false, true),
            (ButtonState::Pressed, true, false),
            (ButtonState::Released, false, false),
        ] {
            app.world_mut().trigger(FocusedInput::new(
                card,
                KeyboardInput {
                    key_code,
                    logical_key: Key::Enter,
                    state,
                    text: None,
                    repeat,
                    window,
                },
                window,
            ));
            app.world_mut().flush();
            if opens {
                assert_open(&mut app);
            } else {
                assert!(take_actions(&mut app).is_empty());
            }
        }
    }
}

#[test]
fn folder_double_click_does_not_open_during_drag_or_a_popup() {
    let (mut app, window, card, _) = fixture();
    app.world_mut().resource_mut::<AssetDrag>().dragging = true;
    click(&mut app, window, card, 2, PointerButton::Primary);
    assert!(take_actions(&mut app).is_empty());
    app.world_mut().resource_mut::<AssetDrag>().dragging = false;
    app.world_mut().resource_mut::<Editor>().browser_popup = true;
    click(&mut app, window, card, 2, PointerButton::Primary);
    assert!(take_actions(&mut app).is_empty());
}

#[test]
fn image_cards_open_only_on_primary_double_click() {
    let (mut app, window, card, _) = fixture_for(Some(Action::ViewImage(7)));
    for _ in 0..2 {
        click(&mut app, window, card, 1, PointerButton::Primary);
        assert!(take_actions(&mut app).is_empty());
    }
    click(&mut app, window, card, 2, PointerButton::Secondary);
    assert!(take_actions(&mut app).is_empty());
    click(&mut app, window, card, 2, PointerButton::Primary);
    assert!(matches!(
        take_actions(&mut app).as_slice(),
        [Action::ViewImage(7)]
    ));
}

#[test]
fn image_cards_keep_keyboard_activation() {
    let (mut app, window, card, _) = fixture_for(Some(Action::ViewImage(7)));
    for key_code in [KeyCode::Enter, KeyCode::Space] {
        app.world_mut().trigger(FocusedInput::new(
            card,
            KeyboardInput {
                key_code,
                logical_key: Key::Enter,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window,
            },
            window,
        ));
        app.world_mut().flush();
        assert!(matches!(
            take_actions(&mut app).as_slice(),
            [Action::ViewImage(7)]
        ));
    }
}

#[test]
fn image_double_click_does_not_open_during_drag_or_a_popup() {
    let (mut app, window, card, _) = fixture_for(Some(Action::ViewImage(7)));
    app.world_mut().resource_mut::<AssetDrag>().dragging = true;
    click(&mut app, window, card, 2, PointerButton::Primary);
    assert!(take_actions(&mut app).is_empty());
    app.world_mut().resource_mut::<AssetDrag>().dragging = false;
    app.world_mut().resource_mut::<Editor>().browser_popup = true;
    click(&mut app, window, card, 2, PointerButton::Primary);
    assert!(take_actions(&mut app).is_empty());
}
