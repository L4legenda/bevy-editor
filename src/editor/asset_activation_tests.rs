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
        commands.entity(sidebar).insert(AssetOpen);
        drawer::setup(
            &mut commands,
            parent,
            &Handle::default(),
            &starfield::Starfield {
                chrome: Handle::default(),
                space: Handle::default(),
            },
            &Handle::default(),
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

fn assert_action(actual: Action, expected: Action) {
    let same = match (actual, expected) {
        (Action::Browse(a), Action::Browse(b))
        | (Action::Open(a), Action::Open(b))
        | (Action::ViewImage(a), Action::ViewImage(b))
        | (Action::ViewModel(a), Action::ViewModel(b))
        | (Action::BrowseFile(a), Action::BrowseFile(b))
        | (
            Action::Narrative(NarrativeAction::File(a)),
            Action::Narrative(NarrativeAction::File(b)),
        ) => a == b,
        _ => false,
    };
    assert!(
        same,
        "The entry must dispatch its own open action and index"
    );
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
fn folder_sidebar_requires_double_click_navigation() {
    let (mut app, window, _, sidebar) = fixture();
    click(&mut app, window, sidebar, 1, PointerButton::Primary);
    assert!(take_actions(&mut app).is_empty());
    assert!(matches!(
        app.world().resource::<drawer::BrowserUi>().selected_entry,
        Some(browser::Entry::Folder(_))
    ));
    click(&mut app, window, sidebar, 2, PointerButton::Primary);
    assert_open(&mut app);
}

#[test]
fn browser_breadcrumbs_and_tabs_keep_single_click_activation() {
    let (mut app, window, _, _) = fixture();
    for (action, look) in [
        (Action::Browse(0), Look::Regular),
        (Action::Open(0), Look::Tab),
    ] {
        let parent = app.world_mut().spawn_empty().id();
        let button = control(
            &mut app.world_mut().commands(),
            parent,
            action,
            look,
            Node::default(),
        );
        app.world_mut().flush();
        click(&mut app, window, button, 1, PointerButton::Primary);
        let actions = take_actions(&mut app);
        assert_eq!(actions.len(), 1);
        assert_action(actions[0], action);
    }
}

#[test]
fn browser_file_types_select_on_single_click_and_open_once_on_double_click() {
    use browser::Entry;
    for (action, entry) in [
        (Action::ViewModel(0), Entry::Model(0)),
        (Action::Open(0), Entry::Scene { file: 7, room: 0 }),
        (Action::ViewImage(7), Entry::Image(7)),
        (Action::BrowseFile(7), Entry::File(7)),
        (Action::Narrative(NarrativeAction::File(7)), Entry::File(7)),
    ] {
        let (mut app, window, card, _) = fixture_for(Some(action));
        let scan = app.world().resource::<Browser>().scan_revision;
        app.world_mut()
            .entity_mut(card)
            .insert(drawer::FileCard(entry, scan));
        for _ in 0..2 {
            click(&mut app, window, card, 1, PointerButton::Primary);
            assert!(
                take_actions(&mut app).is_empty(),
                "Single click must only select the entry"
            );
            assert_eq!(
                app.world().resource::<drawer::BrowserUi>().selected_entry,
                Some(entry)
            );
            assert_eq!(app.world().resource::<InputFocus>().get(), Some(card));
            assert!(app.world().resource::<Editor>().viewing_image.is_none());
            assert!(app.world().resource::<Editor>().viewing_model.is_none());
        }
        click(&mut app, window, card, 2, PointerButton::Secondary);
        assert!(take_actions(&mut app).is_empty());
        click(&mut app, window, card, 2, PointerButton::Primary);
        let actions = take_actions(&mut app);
        assert_eq!(
            actions.len(),
            1,
            "Double click must open the entry only once"
        );
        assert_action(actions[0], action);
    }
}

#[test]
fn browser_file_types_keep_keyboard_activation_and_ignore_blocked_clicks() {
    for action in [
        Action::ViewModel(0),
        Action::Open(0),
        Action::BrowseFile(7),
        Action::Narrative(NarrativeAction::File(7)),
    ] {
        let (mut app, window, card, _) = fixture_for(Some(action));
        app.world_mut().resource_mut::<AssetDrag>().dragging = true;
        click(&mut app, window, card, 2, PointerButton::Primary);
        assert!(take_actions(&mut app).is_empty());
        app.world_mut().resource_mut::<AssetDrag>().dragging = false;
        app.world_mut().resource_mut::<Editor>().browser_popup = true;
        click(&mut app, window, card, 2, PointerButton::Primary);
        assert!(take_actions(&mut app).is_empty());
        app.world_mut().resource_mut::<Editor>().browser_popup = false;
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
            let actions = take_actions(&mut app);
            assert_eq!(actions.len(), 1);
            assert_action(actions[0], action);
        }
    }
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
