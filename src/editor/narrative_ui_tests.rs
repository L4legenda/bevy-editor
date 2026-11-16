#[path = "narrative_answers_tests.rs"]
mod answers_workflow;
#[path = "narrative_drag_tests.rs"]
mod drag_workflow;
#[path = "narrative_effects_tests.rs"]
mod effects_workflow;
#[path = "narrative_portraits_tests.rs"]
mod portraits_workflow;
#[path = "narrative_reorder_tests.rs"]
mod reorder_workflow;
use super::*;
use crate::editor::{
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{fixture_project, mouse, move_pointer},
};
use bevy::input::ButtonState;

fn warm(app: &mut App, n: usize) {
    for _ in 0..n {
        frame(app);
    }
}
fn action(app: &mut App, a: NarrativeAction) {
    eprintln!("Narrative action: {a:?}");
    app.world_mut().write_message(Action::Narrative(a));
    warm(app, 4);
}
fn control_point(app: &mut App, label: &str) -> Vec2 {
    {
        let world = app.world_mut();
        let mut q = world.query::<(&AccessibleLabel, &ComputedNode, &UiGlobalTransform)>();
        q.iter(world)
            .find(|(l, n, _)| l.0 == label && !n.is_empty())
            .map(|(_, _, t)| t.translation)
            .unwrap_or_else(|| panic!("Visible control missing: {label}"))
    }
}
fn click(app: &mut App, window: Entity, label: &str) {
    let point = control_point(app, label);
    move_pointer(app, window, point);
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
}

fn right_click(app: &mut App, window: Entity, label: &str) {
    use bevy::picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput};
    let position = control_point(app, label);
    move_pointer(app, window, position);
    let target = {
        let world = app.world_mut();
        world
            .query_filtered::<&bevy::camera::RenderTarget, With<bevy::ui::IsDefaultUiCamera>>()
            .single(world)
            .unwrap()
            .normalize(Some(window))
            .unwrap()
    };
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut()
            .write_message(bevy::input::mouse::MouseButtonInput {
                window,
                button: MouseButton::Right,
                state,
            });
        app.world_mut().write_message(PointerInput {
            pointer_id: PointerId::Mouse,
            location: Location {
                target: target.clone(),
                position,
            },
            action: if state == ButtonState::Pressed {
                PointerAction::Press(PointerButton::Secondary)
            } else {
                PointerAction::Release(PointerButton::Secondary)
            },
        });
        warm(app, 3);
    }
    warm(app, 4);
}

fn branch_target(app: &App, node: u32, choice: usize) -> Option<u32> {
    match &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        Content::Dialogue(d) => d.node(node).unwrap().choices[choice].target,
        _ => unreachable!(),
    }
}

fn begin_connection(app: &mut App, window: Entity, node: u32, choice: usize) -> Vec2 {
    let source = control_point(app, &format!("Связать ответ {} ноды #{}", choice + 1, node));
    move_pointer(app, window, source);
    mouse(app, window, ButtonState::Pressed);
    assert!(
        app.world().resource::<links::State>().drag.is_some(),
        "Pressing an output must arm a connection gesture"
    );
    assert_eq!(app.world().resource::<links::State>().trace, None);
    source
}

fn emphasized_connections(app: &mut App) -> Vec<(u32, usize)> {
    let world = app.world_mut();
    let mut query = world.query::<(
        &connection_paint::Connection,
        &MaterialNode<connection_paint::Material>,
    )>();
    let materials = world.resource::<Assets<connection_paint::Material>>();
    let green = GREEN.to_linear();
    let tint = Vec4::new(green.red, green.green, green.blue, green.alpha);
    let mut result: Vec<_> = query
        .iter(world)
        .filter_map(|(connection, handle)| {
            materials.get(&handle.0).and_then(|material| {
                (material.tint == tint && material.metrics.z > 2.0)
                    .then_some((connection.from, connection.choice))
            })
        })
        .collect();
    result.sort_unstable();
    result
}

fn card_border(app: &mut App, id: u32) -> BorderColor {
    let world = app.world_mut();
    world
        .query::<(&nodes::Card, &BorderColor)>()
        .iter(world)
        .find(|(card, _)| card.0 == id)
        .unwrap()
        .1
        .clone()
}

fn hover_connections(
    app: &mut App,
    window: Entity,
    target: &Handle<Image>,
    output: &std::path::Path,
) {
    let (content, selection, history) = {
        let editor = app.world().resource::<Editor>();
        let doc = editor.narrative.current().unwrap();
        (doc.content.clone(), doc.selected, doc.undo.len())
    };
    let original_border = card_border(app, 2);
    let connections = app.world().resource::<NarrativeUi>().connections;
    let wire_entities = app.world().get::<Children>(connections).unwrap().to_vec();
    let source = control_point(app, "Связать ответ 1 ноды #1");
    move_pointer(app, window, source);
    warm(app, 4);
    assert_eq!(
        app.world().resource::<links::State>().trace,
        Some(links::Port::Output(1, 0))
    );
    assert_eq!(emphasized_connections(app), vec![(1, 0)]);
    assert_eq!(card_border(app, 2), BorderColor::all(GREEN));
    assert_ne!(card_border(app, 3), BorderColor::all(GREEN));
    assert_eq!(
        app.world().get::<Children>(connections).unwrap().to_vec(),
        wire_entities
    );
    capture_when(
        app,
        target,
        &output.join("connection-hover-output.png"),
        |_| true,
    );
    let empty = Vec2::new(650.0, 810.0);
    move_pointer(app, window, empty);
    warm(app, 4);
    assert_eq!(app.world().resource::<links::State>().trace, None);
    assert!(emphasized_connections(app).is_empty());
    assert_eq!(card_border(app, 2), original_border);
    let second = control_point(app, "Связать ответ 2 ноды #1");
    move_pointer(app, window, second);
    warm(app, 4);
    assert_eq!(emphasized_connections(app), vec![(1, 1)]);
    assert_eq!(card_border(app, 2), original_border);
    assert_eq!(card_border(app, 3), BorderColor::all(GREEN));
    // Preview overlays and window focus loss must clear the transient highlight.
    action(app, NarrativeAction::Preview);
    assert_eq!(app.world().resource::<links::State>().trace, None);
    assert!(emphasized_connections(app).is_empty());
    action(app, NarrativeAction::Dismiss);
    let input = control_point(app, "Вход ноды #2");
    move_pointer(app, window, input);
    warm(app, 4);
    assert_eq!(
        app.world().resource::<links::State>().trace,
        Some(links::Port::Input(2))
    );
    assert_eq!(emphasized_connections(app), vec![(1, 0)]);
    assert_eq!(card_border(app, 1), BorderColor::all(GREEN));
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    warm(app, 4);
    assert_eq!(app.world().resource::<links::State>().trace, None);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    warm(app, 4);
    {
        let editor = app.world().resource::<Editor>();
        let doc = editor.narrative.current().unwrap();
        assert_eq!(doc.content, content);
        assert_eq!(doc.selected, selection);
        assert_eq!(doc.undo.len(), history);
    }
    // A shared input reveals every incoming edge and each source card.
    action(app, NarrativeAction::Connect(0, 0, 2));
    action(app, NarrativeAction::Select(3));
    let input = control_point(app, "Вход ноды #2");
    move_pointer(app, window, input);
    warm(app, 4);
    assert_eq!(emphasized_connections(app), vec![(0, 0), (1, 0)]);
    assert_eq!(card_border(app, 0), BorderColor::all(GREEN));
    assert_eq!(card_border(app, 1), BorderColor::all(GREEN));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .selected,
        3
    );
    capture_when(
        app,
        target,
        &output.join("connection-hover-input.png"),
        |_| true,
    );
    action(app, NarrativeAction::Undo);
    action(app, NarrativeAction::Select(1));
    action(
        app,
        NarrativeAction::Node(1, nodes::NodeAction::Disconnect(0)),
    );
    for label in ["Связать ответ 1 ноды #1", "Вход ноды #2"] {
        let point = control_point(app, label);
        move_pointer(app, window, point);
        warm(app, 4);
        assert_eq!(app.world().resource::<links::State>().trace, None);
        assert!(emphasized_connections(app).is_empty());
    }
    capture_when(
        app,
        target,
        &output.join("connection-hover-unlinked.png"),
        |_| true,
    );
    action(app, NarrativeAction::Undo);
    action(app, NarrativeAction::Select(selection));
    move_pointer(app, window, empty);
    warm(app, 4);
    let editor = app.world().resource::<Editor>();
    let doc = editor.narrative.current().unwrap();
    assert_eq!(doc.content, content);
    assert_eq!(doc.undo.len(), history);
}

fn drag_connections(
    app: &mut App,
    window: Entity,
    target: &Handle<Image>,
    output: &std::path::Path,
) {
    let answer = control_point(app, "Ответ 1 ноды #1");
    let delete = control_point(app, "Удалить ответ 1 ноды #1");
    let socket = control_point(app, "Связать ответ 1 ноды #1");
    assert!(
        (answer.y - socket.y).abs() < 1.0 && (delete.y - socket.y).abs() < 1.0,
        "The socket, delete icon and answer must share one row"
    );
    assert!(socket.x > delete.x && delete.x > answer.x);
    {
        let world = app.world_mut();
        assert!(
            !world
                .query::<(&DraftField, &nodes::Owner)>()
                .iter(world)
                .any(|(field, owner)| owner.0 == 0 && matches!(field, DraftField::Choice(_)))
        );
    }
    let before = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
        .clone();
    let source = begin_connection(app, window, 1, 1);
    let input = control_point(app, "Вход ноды #2");
    move_pointer(app, window, (source + input) * 0.5);
    assert!(
        app.world()
            .resource::<links::State>()
            .drag
            .as_ref()
            .unwrap()
            .moved
    );
    capture_when(app, target, &output.join("link-drag-preview.png"), |_| true);
    move_pointer(app, window, input);
    assert_eq!(
        app.world()
            .resource::<links::State>()
            .drag
            .as_ref()
            .unwrap()
            .target,
        Some(2)
    );
    capture_when(app, target, &output.join("link-drop-target.png"), |_| true);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
    assert_eq!(branch_target(app, 1, 1), Some(2));
    assert!(app.world().resource::<NarrativeUi>().linking.is_none());
    action(app, NarrativeAction::Undo);
    assert_eq!(branch_target(app, 1, 1), Some(3));
    action(app, NarrativeAction::Redo);
    assert_eq!(branch_target(app, 1, 1), Some(2));
    action(app, NarrativeAction::Undo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        before
    );
    let history = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len();
    let empty = Vec2::new(650.0, 810.0);
    // Empty space, return-to-source, and Escape cancel without losing the original edge.
    for mode in 0..3 {
        let source = begin_connection(app, window, 1, 1);
        move_pointer(app, window, empty);
        if mode == 1 {
            move_pointer(app, window, source);
        }
        if mode == 2 {
            key(
                app,
                window,
                KeyCode::Escape,
                bevy::input::keyboard::Key::Escape,
                None,
            );
        }
        mouse(app, window, ButtonState::Released);
        warm(app, 4);
        assert_eq!(branch_target(app, 1, 1), Some(3));
        assert_eq!(
            app.world()
                .resource::<Editor>()
                .narrative
                .current()
                .unwrap()
                .undo
                .len(),
            history
        );
        assert!(app.world().resource::<NarrativeUi>().linking.is_none());
        assert!(app.world().resource::<links::State>().drag.is_none());
    }
    begin_connection(app, window, 1, 1);
    move_pointer(app, window, empty);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    warm(app, 4);
    assert!(app.world().resource::<links::State>().drag.is_none());
    mouse(app, window, ButtonState::Released);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    warm(app, 4);
    assert_eq!(branch_target(app, 1, 1), Some(3));
    // Undo during a gesture restores the edge, and must cancel the transient preview.
    action(app, NarrativeAction::Connect(1, 1, 2));
    begin_connection(app, window, 1, 1);
    move_pointer(app, window, empty);
    action(app, NarrativeAction::Undo);
    assert!(app.world().resource::<links::State>().drag.is_none());
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
    assert_eq!(branch_target(app, 1, 1), Some(3));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    // The entry has no input, and cannot become a destination through a header drop.
    begin_connection(app, window, 1, 1);
    let start = control_point(app, "Выбрать НАЧАЛО");
    move_pointer(app, window, start);
    assert!(
        app.world()
            .resource::<links::State>()
            .drag
            .as_ref()
            .unwrap()
            .target
            .is_none()
    );
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
    assert_eq!(branch_target(app, 1, 1), Some(3));
    // The start output works at another zoom; an unchanged destination creates no undo step.
    action(app, NarrativeAction::Zoom(-0.2));
    capture_when(
        app,
        target,
        &output.join("connections-zoomed-out.png"),
        |_| true,
    );
    begin_connection(app, window, 0, 0);
    let line = control_point(app, "Вход ноды #1");
    move_pointer(app, window, line);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
    assert_eq!(branch_target(app, 0, 0), Some(1));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    action(app, NarrativeAction::Fit);
    let pan = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .pan;
    right_click(app, window, "Связать ответ 1 ноды #1");
    assert_eq!(branch_target(app, 1, 0), None);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .pan,
        pan
    );
    assert!(app.world().resource::<NarrativeUi>().linking.is_none());
    action(app, NarrativeAction::Undo);
    assert_eq!(branch_target(app, 1, 0), Some(2));
    let socket = {
        let world = app.world_mut();
        world
            .query::<(Entity, &links::Output)>()
            .iter(world)
            .find(|(_, output)| (output.0, output.1) == (1, 0))
            .unwrap()
            .0
    };
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(socket, bevy::input_focus::FocusCause::Auto);
    key(
        app,
        window,
        KeyCode::Delete,
        bevy::input::keyboard::Key::Delete,
        None,
    );
    assert_eq!(branch_target(app, 1, 0), None);
    action(app, NarrativeAction::Undo);
    assert_eq!(branch_target(app, 1, 0), Some(2));
}

fn edit_text(app: &mut App, kind: DraftField, value: &str) -> Entity {
    let entity = {
        let world = app.world_mut();
        let selected = world
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .selected;
        world
            .query::<(Entity, &DraftField, Option<&nodes::Owner>, &ComputedNode)>()
            .iter(world)
            .find(|(_, f, owner, computed)| {
                !computed.is_empty()
                    && **f == kind
                    && (kind == DraftField::Title || owner.is_none_or(|o| o.0 == selected))
            })
            .unwrap()
            .0
    };
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(entity, bevy::input_focus::FocusCause::Auto);
    app.world_mut()
        .get_mut::<EditableText>(entity)
        .unwrap()
        .editor_mut()
        .set_text(value);
    warm(app, 3);
    entity
}

fn key(
    app: &mut App,
    window: Entity,
    code: KeyCode,
    logical: bevy::input::keyboard::Key,
    typed: Option<&str>,
) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut()
            .write_message(bevy::input::keyboard::KeyboardInput {
                window,
                key_code: code,
                logical_key: logical.clone(),
                state,
                text: typed.map(Into::into),
                repeat: false,
            });
        warm(app, 3);
    }
}

fn named_input(app: &mut App, name: &str) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &AccessibleLabel, &ComputedNode, &EditableText)>()
        .iter(world)
        .find(|(_, label, node, _)| label.0 == name && !node.is_empty())
        .unwrap()
        .0
}

fn edit_named(app: &mut App, name: &str, value: &str) -> Entity {
    let entity = named_input(app, name);
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(entity, bevy::input_focus::FocusCause::Auto);
    app.world_mut()
        .get_mut::<EditableText>(entity)
        .unwrap()
        .editor
        .set_text(value);
    warm(app, 4);
    entity
}

fn editor_shortcut(app: &mut App, window: Entity, code: KeyCode, letter: &str) {
    app.world_mut()
        .write_message(bevy::input::keyboard::KeyboardInput {
            window,
            key_code: KeyCode::ControlLeft,
            logical_key: bevy::input::keyboard::Key::Control,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
        });
    warm(app, 3);
    key(
        app,
        window,
        code,
        bevy::input::keyboard::Key::Character(letter.into()),
        None,
    );
    app.world_mut()
        .write_message(bevy::input::keyboard::KeyboardInput {
            window,
            key_code: KeyCode::ControlLeft,
            logical_key: bevy::input::keyboard::Key::Control,
            state: ButtonState::Released,
            text: None,
            repeat: false,
        });
    warm(app, 4);
}

fn select_editor_text(app: &mut App, range: std::ops::Range<usize>) {
    let entity = named_input(app, "Текст в большом редакторе");
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(entity, bevy::input_focus::FocusCause::Auto);
    app.world_mut()
        .resource_scope(|world, mut fonts: Mut<bevy::text::FontCx>| {
            world.resource_scope(|world, mut layout: Mut<bevy::text::LayoutCx>| {
                world
                    .get_mut::<EditableText>(entity)
                    .unwrap()
                    .editor
                    .driver(&mut fonts, &mut layout.0)
                    .select_byte_range(range.start, range.end);
            });
        });
    warm(app, 4);
}

fn text_editor_workflow(
    app: &mut App,
    window: Entity,
    target: &Handle<Image>,
    output: &std::path::Path,
) {
    let (before, history, selection, root, path) = {
        let editor = app.world().resource::<Editor>();
        let doc = editor.narrative.current().unwrap();
        (
            doc.content.clone(),
            doc.undo.len(),
            doc.selected,
            editor.root.clone(),
            doc.path.clone(),
        )
    };
    click(app, window, "Открыть редактор текста ноды #1");
    assert!(app.world().resource::<text_editor::State>().is_open());
    assert_eq!(
        app.world().resource::<NarrativeUi>().picker,
        Some(Picker::Text)
    );
    let value = "Привет, капитан!\nСигнал из туманности.\nПривет, команда!";
    let long = (0..80)
        .map(|i| format!("Строка {}: проверка длинной реплики.\n", i + 1))
        .collect::<String>()
        + "В конце находится маяк.";
    let long_body = edit_named(app, "Текст в большом редакторе", &long);
    select_editor_text(app, 0..long.len());
    click(app, window, "Подчёркивание · Ctrl+U");
    click(app, window, "Поиск и замена · Ctrl+F");
    edit_named(app, "Найти в тексте", "маяк");
    click(app, window, "Следующее совпадение · F3");
    warm(app, 4);
    assert!(
        app.world()
            .get::<EditableText>(long_body)
            .unwrap()
            .viewport
            .offset
            .y
            > 100.0
    );
    capture_when(
        app,
        target,
        &output.join("text-editor-long-search.png"),
        |_| true,
    );
    click(app, window, "Закрыть поиск и замену");
    select_editor_text(app, 0..long.len());
    click(app, window, "Сбросить оформление текста");
    let body = edit_named(app, "Текст в большом редакторе", value);
    select_editor_text(app, "Привет, ".len().."Привет, капитан".len());
    editor_shortcut(app, window, KeyCode::KeyB, "b");
    edit_named(app, "Размер шрифта", "28");
    for label in [
        "Курсив · Ctrl+I",
        "Подчёркивание · Ctrl+U",
        "Выравнивание: Центр",
        "Изменить межстрочный интервал",
    ] {
        click(app, window, label);
    }
    click(app, window, "Выбрать цвет выделенного текста");
    edit_named(app, "HEX цвета", "#XYZ");
    action(
        app,
        NarrativeAction::Text(text_editor::TextAction::Picker(
            text_editor::color_menu::PickerAction::Confirm,
        )),
    );
    assert!(app.world().resource::<text_editor::State>().is_open());
    edit_named(app, "HEX цвета", "#7FB7FF");
    capture_when(
        app,
        target,
        &output.join("text-editor-color-picker.png"),
        |_| true,
    );
    click(app, window, "Применить выбранный цвет");
    select_editor_text(app, "Привет, ".len().."Привет, капитан".len());
    assert_eq!(named_input(app, "Текст в большом редакторе"), body);
    let info = app.world().get::<bevy::text::TextLayoutInfo>(body).unwrap();
    assert!(info.run_geometry.iter().any(|r| r.bounds.height() > 25.0));
    let native_positions = app
        .world()
        .get::<text_editor::NativeGlyphs>(body)
        .unwrap()
        .0
        .iter()
        .map(|g| g.position)
        .collect::<Vec<_>>();
    let world = app.world_mut();
    let display = world
        .query::<(&Text, &ChildOf, &bevy::text::TextLayoutInfo)>()
        .iter(world)
        .find(|(_, parent, _)| {
            world
                .get::<ChildOf>(parent.parent())
                .is_some_and(|p| p.parent() == body)
        })
        .unwrap()
        .2;
    assert_eq!(native_positions.len(), display.glyphs.len());
    for (native, shown) in native_positions.iter().zip(&display.glyphs) {
        assert!(
            native.distance(shown.position) < 1.1,
            "Rich display and cursor layout disagree: {native:?} vs {:?}",
            shown.position
        );
    }

    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        before
    );
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    capture_when(app, target, &output.join("text-editor-format.png"), |_| {
        true
    });
    click(app, window, "Поиск и замена · Ctrl+F");
    edit_named(app, "Найти в тексте", "ПРИВЕТ");
    edit_named(app, "Заменить на", "Здравствуй");
    click(app, window, "Следующее совпадение · F3");
    let range = app
        .world()
        .get::<EditableText>(body)
        .unwrap()
        .editor
        .raw_selection()
        .text_range();
    assert_eq!(&value[range], "Привет");
    capture_when(app, target, &output.join("text-editor-search.png"), |_| {
        true
    });
    click(app, window, "Заменить все совпадения");
    let expected = value.replace("Привет", "Здравствуй");
    assert_eq!(
        app.world()
            .get::<EditableText>(body)
            .unwrap()
            .value()
            .to_string(),
        expected
    );
    editor_shortcut(app, window, KeyCode::KeyZ, "z");
    assert_eq!(
        app.world()
            .get::<EditableText>(body)
            .unwrap()
            .value()
            .to_string(),
        value
    );
    click(app, window, "Вернуть изменение текста · Ctrl+Shift+Z");
    assert_eq!(
        app.world()
            .get::<EditableText>(body)
            .unwrap()
            .value()
            .to_string(),
        expected
    );
    select_editor_text(app, 0.."Здравствуй, капитан!".len());
    edit_named(app, "Размер шрифта", "999");
    action(app, NarrativeAction::Text(text_editor::TextAction::Apply));
    assert!(app.world().resource::<text_editor::State>().is_open());
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        before
    );
    edit_named(app, "Размер шрифта", "28");
    click(app, window, "Выбрать цвет выделенного текста");
    edit_named(app, "HEX цвета", "#71D1B0");
    key(
        app,
        window,
        KeyCode::Escape,
        bevy::input::keyboard::Key::Escape,
        None,
    );
    assert!(app.world().resource::<text_editor::State>().is_open());
    resize_qa(app, window, target, 760, 900);
    assert_eq!(named_input(app, "Текст в большом редакторе"), body);
    capture_when(app, target, &output.join("text-editor-narrow.png"), |_| {
        true
    });
    resize_qa(app, window, target, 1280, 900);
    click(app, window, "Переключить предпросмотр текста");
    capture_when(app, target, &output.join("text-editor-preview.png"), |_| {
        true
    });
    click(app, window, "Применить текст реплики");
    assert!(!app.world().resource::<text_editor::State>().is_open());
    {
        let editor = app.world().resource::<Editor>();
        let doc = editor.narrative.current().unwrap();
        assert_eq!(doc.undo.len(), history + 1);
        let Content::Dialogue(dialogue) = &doc.content else {
            panic!()
        };
        let node = dialogue.node(1).unwrap();
        assert_eq!(node.text, expected);
        assert_eq!(node.text_format.font_size, 17.0);
        let runs =
            crate::editor::dialogue::text::runs(&node.text, &node.text_format, &node.text_spans);
        let styled = runs
            .iter()
            .find(|s| s.format.color == Some([127, 183, 255]))
            .unwrap();
        assert_eq!(&node.text[styled.start..styled.end], "капитан");
        assert_eq!(styled.format.font_size, 28.0);
        assert!(styled.format.bold && styled.format.italic && styled.format.underline);
        assert!(
            runs.iter()
                .any(|s| s.format.font_size == 17.0 && !s.format.bold && s.format.color.is_none())
        );
        assert_eq!(node.text_format.alignment, TextAlign::Center);
        assert_eq!(node.text_format.line_height, 1.5);
    }
    action(app, NarrativeAction::Save);
    let saved = Content::read(&root, &path).unwrap();
    assert_eq!(
        saved,
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content
    );
    click(app, window, "Открыть редактор текста ноды #1");
    let reopened = named_input(app, "Текст в большом редакторе");
    assert_eq!(
        app.world()
            .get::<EditableText>(reopened)
            .unwrap()
            .value()
            .to_string(),
        expected
    );
    let stable_text = {
        let world = app.world_mut();
        let (entity, display) = world
            .query::<(Entity, &Text, &ChildOf, &bevy::text::TextLayoutInfo)>()
            .iter(world)
            .find(|(_, _, parent, _)| {
                world
                    .get::<ChildOf>(parent.parent())
                    .is_some_and(|p| p.parent() == reopened)
            })
            .map(|(e, _, _, i)| (e, i.clone()))
            .unwrap();
        let children = world.get::<Children>(entity).unwrap();
        let text = children
            .iter()
            .map(|e| world.get::<bevy::text::TextSpan>(e).unwrap().0.clone())
            .collect::<String>();
        assert_eq!(text, expected);
        let native = &world.get::<text_editor::NativeGlyphs>(reopened).unwrap().0;
        assert_eq!(native.len(), display.glyphs.len());
        for (native, shown) in native.iter().zip(&display.glyphs) {
            assert!(
                native.position.distance(shown.position) < 1.1,
                "Reopened display mismatch: {:?} vs {:?}",
                native.position,
                shown.position
            );
        }
        entity
    };
    capture_when(
        app,
        target,
        &output.join("text-editor-reopened-immediate.png"),
        |_| true,
    );
    warm(app, 70);
    assert!(
        app.world().get::<Text>(stable_text).is_some(),
        "Cursor blinking must preserve the shaped text entity"
    );
    capture_when(
        app,
        target,
        &output.join("text-editor-reopened.png"),
        |_| true,
    );
    click(app, window, "Отмена редактирования текста");
    action(app, NarrativeAction::Preview);
    capture_when(
        app,
        target,
        &output.join("text-editor-conversation.png"),
        |_| true,
    );
    action(app, NarrativeAction::Dismiss);
    action(app, NarrativeAction::Undo);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        before
    );
    action(app, NarrativeAction::Save);
    click(app, window, "Открыть редактор текста ноды #1");
    edit_named(app, "Текст в большом редакторе", "Сохранено по Ctrl+S.");
    editor_shortcut(app, window, KeyCode::KeyS, "s");
    assert!(!app.world().resource::<text_editor::State>().is_open());
    assert!(
        !app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .dirty()
    );
    assert_eq!(
        Content::read(&root, &path).unwrap(),
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content
    );
    action(app, NarrativeAction::Undo);
    action(app, NarrativeAction::Save);
    click(app, window, "Открыть редактор текста ноды #1");
    edit_named(
        app,
        "Текст в большом редакторе",
        "Черновик, который нужно отменить.",
    );
    key(
        app,
        window,
        KeyCode::Escape,
        bevy::input::keyboard::Key::Escape,
        None,
    );
    assert!(app.world().resource::<text_editor::State>().is_open());
    click(app, window, "Продолжить редактирование текста");
    app.world_mut()
        .write_message(bevy::window::WindowCloseRequested { window });
    warm(app, 4);
    assert!(app.world().resource::<text_editor::State>().is_open());
    assert!(app.world().resource::<Editor>().pending.is_none());
    click(app, window, "Продолжить редактирование текста");
    let body = named_input(app, "Текст в большом редакторе");
    assert_eq!(
        app.world()
            .get::<EditableText>(body)
            .unwrap()
            .value()
            .to_string(),
        "Черновик, который нужно отменить."
    );
    click(app, window, "Отмена редактирования текста");
    assert!(!app.world().resource::<text_editor::State>().is_open());
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content,
        before
    );
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    action(app, NarrativeAction::Select(selection));
    move_pointer(app, window, Vec2::new(650.0, 810.0));
    warm(app, 4);
}

fn scroll_character(app: &mut App, window: Entity, lines: f32) {
    let w = app.world().get::<Window>(window).unwrap();
    let point = Vec2::new(w.width() * 0.5, w.height() * 0.6);
    move_pointer(app, window, point);
    app.world_mut()
        .write_message(bevy::input::mouse::MouseWheel {
            window,
            phase: bevy::input::touch::TouchPhase::Moved,
            unit: bevy::input::mouse::MouseScrollUnit::Line,
            x: 0.0,
            y: lines,
        });
    warm(app, 4);
}

fn character_offset(app: &mut App) -> f32 {
    let world = app.world_mut();
    world
        .query_filtered::<&ScrollPosition, With<character_sheet::Stage>>()
        .single(world)
        .unwrap()
        .y
}

fn resize_qa(app: &mut App, window: Entity, target: &Handle<Image>, width: u32, height: u32) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(width as f32, height as f32);
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(target)
        .unwrap()
        .resize(bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        });
    {
        let world = app.world_mut();
        *world
            .query_filtered::<&mut bevy::camera::RenderTarget, With<bevy::ui::IsDefaultUiCamera>>()
            .single_mut(world)
            .unwrap() = bevy::camera::RenderTarget::Image(target.clone().into());
    }
    warm(app, 12);
}

fn visible_speakers(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let mut labels: Vec<_> = world
        .query::<(&AccessibleLabel, &ComputedNode)>()
        .iter(world)
        .filter(|(label, node)| !node.is_empty() && label.0.starts_with("Выбрать персонажа: "))
        .map(|(label, _)| label.0.clone())
        .collect();
    labels.sort();
    labels
}

fn selected_speaker(app: &App) -> Option<String> {
    match &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        Content::Dialogue(d) => d.node(1).unwrap().speaker.clone(),
        _ => unreachable!(),
    }
}

fn speaker_dropdown(
    app: &mut App,
    window: Entity,
    target: &Handle<Image>,
    output: &std::path::Path,
) {
    action(app, NarrativeAction::Select(2));
    click(app, window, "Выбрать говорящего ноды #1");
    assert_eq!(
        app.world().resource::<NarrativeUi>().picker,
        Some(Picker::Speaker)
    );
    let modal = app.world().resource::<NarrativeUi>().overlay;
    assert_eq!(
        app.world().get::<Node>(modal).unwrap().display,
        Display::None
    );
    let search = app.world().resource::<InputFocus>().get().unwrap();
    assert_eq!(
        app.world().get::<DraftField>(search),
        Some(&DraftField::Search)
    );
    capture_when(app, target, &output.join("speaker-dropdown.png"), |_| true);
    assert_eq!(edit_text(app, DraftField::Search, "ВИК"), search);
    assert_eq!(
        visible_speakers(app),
        vec!["Выбрать персонажа: Виктор", "Выбрать персонажа: Виктория"]
    );
    capture_when(app, target, &output.join("speaker-search.png"), |_| true);
    key(
        app,
        window,
        KeyCode::ArrowDown,
        bevy::input::keyboard::Key::ArrowDown,
        None,
    );
    key(
        app,
        window,
        KeyCode::Enter,
        bevy::input::keyboard::Key::Enter,
        None,
    );
    assert!(
        selected_speaker(app)
            .unwrap()
            .ends_with("Виктория.dcharacter")
    );
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    action(app, NarrativeAction::Undo);
    assert_eq!(selected_speaker(app), None);
    click(app, window, "Выбрать говорящего ноды #1");
    edit_text(app, DraftField::Search, "несуществующий");
    assert!(visible_speakers(app).is_empty());
    capture_when(app, target, &output.join("speaker-empty.png"), |_| true);
    key(
        app,
        window,
        KeyCode::Enter,
        bevy::input::keyboard::Key::Enter,
        None,
    );
    assert_eq!(selected_speaker(app), None);
    assert_eq!(
        app.world().resource::<NarrativeUi>().picker,
        Some(Picker::Speaker)
    );
    key(
        app,
        window,
        KeyCode::Escape,
        bevy::input::keyboard::Key::Escape,
        None,
    );
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    let focus = app.world().resource::<InputFocus>().get().unwrap();
    assert_eq!(app.world().get::<speaker_menu::Anchor>(focus).unwrap().0, 1);
    click(app, window, "Выбрать говорящего ноды #1");
    let search = edit_text(app, DraftField::Search, "федо");
    key(
        app,
        window,
        KeyCode::End,
        bevy::input::keyboard::Key::End,
        None,
    );
    key(
        app,
        window,
        KeyCode::KeyR,
        bevy::input::keyboard::Key::Character("р".into()),
        Some("р"),
    );
    assert_eq!(
        app.world()
            .get::<EditableText>(search)
            .unwrap()
            .value()
            .to_string(),
        "федор"
    );
    assert_eq!(visible_speakers(app), vec!["Выбрать персонажа: Фёдор"]);
    move_pointer(app, window, Vec2::new(40.0, 600.0));
    mouse(app, window, ButtonState::Pressed);
    mouse(app, window, ButtonState::Released);
    warm(app, 4);
    assert!(app.world().resource::<NarrativeUi>().picker.is_none());
    assert_eq!(selected_speaker(app), None);
    click(app, window, "Выбрать говорящего ноды #1");
    edit_text(app, DraftField::Search, "страж");
    for _ in 0..8 {
        key(
            app,
            window,
            KeyCode::ArrowDown,
            bevy::input::keyboard::Key::ArrowDown,
            None,
        );
    }
    let row = control_point(app, "Выбрать персонажа: Страж 08");
    let panel = control_point(app, "Список персонажей");
    assert!(
        (row.y - panel.y).abs() < 170.0,
        "Arrow navigation must keep the selected row visible"
    );
    capture_when(app, target, &output.join("speaker-scrolled.png"), |_| true);
    key(
        app,
        window,
        KeyCode::Enter,
        bevy::input::keyboard::Key::Enter,
        None,
    );
    assert!(
        selected_speaker(app)
            .unwrap()
            .ends_with("Страж 08.dcharacter")
    );
    action(app, NarrativeAction::Undo);
    assert_eq!(selected_speaker(app), None);
    // Opening upward keeps a dropdown near the lower edge inside the workspace.
    let pan = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .pan;
    app.world_mut()
        .resource_mut::<Editor>()
        .narrative
        .current_mut()
        .unwrap()
        .pan
        .y += 300.0;
    warm(app, 4);
    click(app, window, "Выбрать говорящего ноды #1");
    let who = control_point(app, "Выбрать говорящего ноды #1");
    let panel = control_point(app, "Список персонажей");
    assert!(panel.y < who.y);
    capture_when(
        app,
        target,
        &output.join("speaker-dropdown-above.png"),
        |_| true,
    );
    edit_text(app, DraftField::Search, "Астр");
    click(app, window, "Выбрать персонажа: Астра");
    assert_eq!(selected_speaker(app), Some("Астра.dcharacter".into()));
    action(app, NarrativeAction::SetPortrait(0));
    app.world_mut()
        .resource_mut::<Editor>()
        .narrative
        .current_mut()
        .unwrap()
        .pan = pan;
    warm(app, 4);
}

#[test]
#[ignore = "GPU narrative workflow and real UI screenshots"]
fn gpu_narrative_workflow() {
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/narrative-qa")
        .join(std::env::var("NARRATIVE_QA_PASS").unwrap_or("pass-1".into()));
    std::fs::create_dir_all(&output).unwrap();
    // All writable files live in a unique QA project directory.
    let lab = output.join(format!("project-{}", std::process::id()));
    std::fs::create_dir_all(&lab).unwrap();
    let mut project = crate::project::EditorProject::load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy/project.json"),
    )
    .unwrap();
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for item in std::fs::read_dir(from).unwrap() {
            let item = item.unwrap();
            if item.file_type().unwrap().is_dir() {
                copy_tree(&item.path(), &to.join(item.file_name()));
            } else {
                std::fs::copy(item.path(), to.join(item.file_name())).unwrap();
            }
        }
    }
    copy_tree(&project.asset_root, &lab);
    for (name, tint) in [
        ("calm.png", [65u8, 120, 184]),
        ("alert.png", [177u8, 115, 60]),
    ] {
        let width = if name == "alert.png" { 192 } else { 256 };
        let image = image::RgbaImage::from_fn(width, 256, |x, y| {
            let px = x as f32 - width as f32 * 0.5;
            let py = y as f32 - 118.0;
            let face = (px * px / 2704.0 + py * py / 3600.0) < 1.0;
            let helmet = (px * px + py * py) < 94.0 * 94.0;
            let eye = py.abs() < 5.0 && (px - 22.0).abs() < 5.0
                || py.abs() < 5.0 && (px + 22.0).abs() < 5.0;
            image::Rgba(if eye {
                [244, 220, 164, 255]
            } else if face {
                [tint[0], tint[1], tint[2], 255]
            } else if helmet {
                [47, 54, 73, 255]
            } else {
                [13 + (x / 18) as u8, 18 + (y / 22) as u8, 30, 255]
            })
        });
        image.save(lab.join(name)).unwrap();
    }
    project.asset_root = lab.clone();
    std::fs::create_dir_all(lab.join("NPC")).unwrap();
    let mut names = vec!["Виктор".to_string(), "Виктория".into(), "Фёдор".into()];
    names.extend((0..10).map(|i| format!("Страж {i:02}")));
    for name in names {
        Content::Character(Character {
            version: VERSION,
            name: name.clone(),
            description: String::new(),
            portraits: vec![],
        })
        .save(
            &lab,
            &std::path::PathBuf::from(format!("NPC/{name}.dcharacter")),
        )
        .unwrap();
    }
    let (mut app, window, target) = fixture_project(project);
    app.world_mut().resource_mut::<Editor>().drawer = false;
    action(&mut app, NarrativeAction::Create(false));
    assert_eq!(app.world().resource::<Editor>().narrative.tabs.len(), 1);
    assert!(
        !app.world_mut()
            .query::<&AccessibleLabel>()
            .iter(app.world())
            .any(|label| label.0 == "Свойства диалога")
    );
    edit_text(&mut app, DraftField::Title, "Сигнал");
    let history = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len();
    edit_text(&mut app, DraftField::Title, "Сигнал из туманности");
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    click(&mut app, window, "+ Реплика");
    let input = edit_text(&mut app, DraftField::Text, "Первая строка");
    assert_eq!(
        edit_text(&mut app, DraftField::Text, "Первая строка\nВторая строка"),
        input
    );
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    action(&mut app, NarrativeAction::Zoom(-0.1));
    app.world_mut()
        .resource_mut::<Editor>()
        .narrative
        .current_mut()
        .unwrap()
        .pan += Vec2::new(12.0, 8.0);
    warm(&mut app, 3);
    assert!(
        app.world().get::<EditableText>(input).is_some(),
        "Navigation must keep the editing entity alive"
    );
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().text, "Первая строка\nВторая строка");
    }
    click(&mut app, window, "Текст реплики ноды #1");
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    key(
        &mut app,
        window,
        KeyCode::End,
        bevy::input::keyboard::Key::End,
        None,
    );
    key(
        &mut app,
        window,
        KeyCode::Digit1,
        bevy::input::keyboard::Key::Character("!".into()),
        Some("!"),
    );
    assert!(
        app.world()
            .get::<EditableText>(input)
            .unwrap()
            .value()
            .to_string()
            .contains('!')
    );
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    assert!(app.world().resource::<NarrativeUi>().drag_node.is_none());
    edit_text(&mut app, DraftField::Text, "Первая строка\nВторая строка");
    let header = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &UiGlobalTransform)>()
            .iter(world)
            .find(|(label, _)| label.0 == "Выбрать РЕПЛИКА #1")
            .unwrap()
            .1
            .translation
    };
    let before = match &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        Content::Dialogue(d) => d.node(1).unwrap().position,
        _ => unreachable!(),
    };
    move_pointer(&mut app, window, header);
    mouse(&mut app, window, ButtonState::Pressed);
    app.world_mut()
        .write_message(bevy::input::mouse::MouseMotion {
            delta: Vec2::new(24.0, 18.0),
        });
    move_pointer(&mut app, window, header + Vec2::new(24.0, 18.0));
    mouse(&mut app, window, ButtonState::Released);
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert!(d.node(1).unwrap().position[0] > before[0]);
    }
    assert!(app.world().get::<EditableText>(input).is_some());
    action(&mut app, NarrativeAction::Undo);
    click(&mut app, window, "+ Вариант ответа");
    // Creating from output 2 must leave the first free output untouched.
    action(&mut app, NarrativeAction::Link(1, 1));
    action(&mut app, NarrativeAction::Add(NodeKind::End));
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().choices[0].target, None);
        assert_eq!(d.node(1).unwrap().choices[1].target, Some(2));
    }
    action(&mut app, NarrativeAction::Undo);
    action(&mut app, NarrativeAction::Select(1));
    click(&mut app, window, "+ Конец");
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        let doc = editor.narrative.current_mut().unwrap();
        if let Content::Dialogue(d) = &mut doc.content {
            d.title = "Сигнал из туманности".into();
            d.node_mut(1).unwrap().text =
                "Капитан, мы поймали сигнал. Исследуем источник или продолжим маршрут?".into();
            d.node_mut(1).unwrap().choices[0].text = "Исследовать сигнал".into();
            d.node_mut(1).unwrap().choices[1].text = "Продолжить маршрут".into();
            let third = d.add(NodeKind::End, 1);
            nodes::place(d, third, 1);
            d.node_mut(third).unwrap().text = "Корабль продолжает путь".into();
            d.node_mut(2).unwrap().text = "Курс на источник сигнала".into();
        }
        doc.selected = 1;
    }
    app.world_mut().resource_mut::<NarrativeUi>().generation += 1;
    action(&mut app, NarrativeAction::Fit);
    warm(&mut app, 10);
    hover_connections(&mut app, window, &target, &output);
    drag_connections(&mut app, window, &target, &output);
    // Capture the hovered header too: its highlight must respect the card's inner corners.
    action(&mut app, NarrativeAction::Select(0));
    let header_point = {
        let world = app.world_mut();
        world
            .query::<(&AccessibleLabel, &UiGlobalTransform)>()
            .iter(world)
            .find(|(label, _)| label.0 == "Выбрать НАЧАЛО")
            .unwrap()
            .1
            .translation
    };
    move_pointer(&mut app, window, header_point);
    warm(&mut app, 16);
    capture_when(
        &mut app,
        &target,
        &output.join("node-header-hover.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Select(1));
    capture_when(&mut app, &target, &output.join("dialogue.png"), |_| true);
    action(&mut app, NarrativeAction::Zoom(0.3));
    capture_when(
        &mut app,
        &target,
        &output.join("connections-zoomed-in.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Fit);
    let input = edit_text(
        &mut app,
        DraftField::Text,
        "Капитан, мы поймали сигнал. Исследуем источник или продолжим маршрут?",
    );
    action(&mut app, NarrativeAction::Select(2));
    // Typing in an unselected card must still edit that card and select it.
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(input, bevy::input_focus::FocusCause::Auto);
    warm(&mut app, 3);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .selected,
        1
    );
    click(&mut app, window, "Проверить");
    assert_eq!(app.world().resource::<NarrativeUi>().playing, Some(1));
    capture_when(&mut app, &target, &output.join("preview.png"), |_| true);
    click(&mut app, window, "Исследовать сигнал");
    assert_eq!(app.world().resource::<NarrativeUi>().playing, Some(2));
    action(&mut app, NarrativeAction::Restart);
    action(&mut app, NarrativeAction::Advance(1));
    assert_eq!(app.world().resource::<NarrativeUi>().playing, Some(3));
    action(&mut app, NarrativeAction::Dismiss);
    action(&mut app, NarrativeAction::Save);
    let id = app.world().resource::<Editor>().narrative.active.unwrap();
    assert!(
        !app.world()
            .resource::<Editor>()
            .narrative
            .get(id)
            .unwrap()
            .dirty()
    );
    action(&mut app, NarrativeAction::Create(true));
    let character_id = app.world().resource::<Editor>().narrative.active.unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("character-empty.png"),
        |_| true,
    );
    {
        let mut e = app.world_mut().resource_mut::<Editor>();
        let doc = e.narrative.current_mut().unwrap();
        doc.checkpoint();
        if let Content::Character(c) = &mut doc.content {
            c.description =
                "Навигатор исследовательского корабля. Слышит звёзды раньше остальных.".into();
        }
    }
    app.world_mut().resource_mut::<NarrativeUi>().generation += 1;
    warm(&mut app, 6);
    let original_character_path = app
        .world()
        .resource::<Editor>()
        .narrative
        .get(character_id)
        .unwrap()
        .path
        .clone();
    let name_input = edit_text(&mut app, DraftField::Title, "Астр");
    assert!(lab.join(&original_character_path).exists());
    assert_eq!(edit_text(&mut app, DraftField::Title, "Астра"), name_input);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(name_input));
    warm(&mut app, 30);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .get(character_id)
            .unwrap()
            .path,
        std::path::PathBuf::from("Астра.dcharacter")
    );
    assert!(lab.join("Астра.dcharacter").exists());
    assert!(!lab.join("Астр.dcharacter").exists());
    assert!(!lab.join(&original_character_path).exists());
    assert!(
        app.world()
            .resource::<Editor>()
            .narrative
            .get(character_id)
            .unwrap()
            .dirty()
    );
    assert!(
        app.world()
            .resource::<Browser>()
            .files
            .iter()
            .any(|f| f.filename == "Астра.dcharacter")
    );
    app.world_mut()
        .resource_mut::<Browser>()
        .set_search("Астра".into());
    app.world_mut().write_message(Action::Drawer);
    warm(&mut app, 30);
    capture_when(
        &mut app,
        &target,
        &output.join("character-renamed-file.png"),
        |_| true,
    );
    app.world_mut().write_message(Action::Drawer);
    warm(&mut app, 30);
    app.world_mut()
        .resource_mut::<Browser>()
        .set_search(String::new());
    action(&mut app, NarrativeAction::Undo);
    assert!(lab.join(&original_character_path).exists());
    action(&mut app, NarrativeAction::Redo);
    assert!(lab.join("Астра.dcharacter").exists());
    edit_text(&mut app, DraftField::Title, "Астра/тест");
    warm(&mut app, 30);
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .get(character_id)
            .unwrap()
            .path,
        std::path::PathBuf::from("Астра.dcharacter")
    );
    assert!(
        app.world()
            .resource::<Editor>()
            .status
            .starts_with("Не удалось")
    );
    edit_text(&mut app, DraftField::Title, "Астра");
    warm(&mut app, 3);
    let image_indices: Vec<_> = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.kind == crate::editor::browser::FileKind::Image)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(image_indices.len(), 2);
    for index in image_indices {
        click(&mut app, window, "+ Добавить портрет");
        action(&mut app, NarrativeAction::PickImage(index));
    }
    warm(&mut app, 12);
    // A non-square photograph must retain its proportions in the cover and gallery.
    let alert: Handle<Image> = app.world().resource::<AssetServer>().load("alert.png");
    {
        let world = app.world_mut();
        let mut query = world.query::<(&ImageNode, &ComputedNode)>();
        let photos: Vec<_> = query
            .iter(world)
            .filter(|(image, node)| image.image == alert && !node.is_empty())
            .collect();
        assert!(photos.len() >= 2);
        for (_, node) in photos {
            assert!(
                (node.size().x / node.size().y - 0.75).abs() < 0.01,
                "Visible photograph was distorted: {:?}",
                node.size()
            );
        }
    }
    resize_qa(&mut app, window, &target, 1280, 700);
    scroll_character(&mut app, window, -12.0);
    let offset = character_offset(&mut app);
    assert!(
        offset > 0.0,
        "The whole sheet must scroll to reach portrait controls"
    );
    click(&mut app, window, "Название портрета 2");
    let label_input = edit_text(&mut app, DraftField::PortraitLabel(1), "Спокойствие");
    key(
        &mut app,
        window,
        KeyCode::End,
        bevy::input::keyboard::Key::End,
        None,
    );
    key(
        &mut app,
        window,
        KeyCode::Digit1,
        bevy::input::keyboard::Key::Character("!".into()),
        Some("!"),
    );
    assert_eq!(
        app.world().resource::<InputFocus>().get(),
        Some(label_input)
    );
    if let Content::Character(c) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert!(c.portraits[1].label.contains('!'));
    }
    edit_text(&mut app, DraftField::PortraitLabel(1), "Спокойствие");
    let original_portraits = match &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        Content::Character(c) => c.portraits.clone(),
        _ => unreachable!(),
    };
    click(&mut app, window, "Сделать портрет 2 основным");
    assert!(
        (character_offset(&mut app) - offset).abs() < 1.0,
        "Changing the main portrait must retain the scroll position"
    );
    if let Content::Character(c) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(c.portraits[0], original_portraits[1]);
    }
    capture_when(
        &mut app,
        &target,
        &output.join("character-gallery.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Undo);
    click(&mut app, window, "Убрать портрет 2 из персонажа");
    if let Content::Character(c) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(c.portraits.len(), 1);
    }
    action(&mut app, NarrativeAction::Undo);
    action(&mut app, NarrativeAction::Redo);
    action(&mut app, NarrativeAction::Undo);
    resize_qa(&mut app, window, &target, 1280, 900);
    scroll_character(&mut app, window, 20.0);
    capture_when(&mut app, &target, &output.join("character.png"), |_| true);
    let stable_input = edit_text(
        &mut app,
        DraftField::Description,
        "Навигатор исследовательского корабля. Слышит звёзды раньше остальных.",
    );
    resize_qa(&mut app, window, &target, 760, 900);
    assert!(app.world().get::<EditableText>(stable_input).is_some());
    assert_eq!(
        app.world().resource::<InputFocus>().get(),
        Some(stable_input)
    );
    capture_when(
        &mut app,
        &target,
        &output.join("character-narrow.png"),
        |_| true,
    );
    resize_qa(&mut app, window, &target, 1920, 1080);
    capture_when(
        &mut app,
        &target,
        &output.join("character-wide.png"),
        |_| true,
    );
    resize_qa(&mut app, window, &target, 1280, 900);
    action(&mut app, NarrativeAction::Close(character_id));
    assert_eq!(
        app.world().resource::<Editor>().pending,
        Some(PendingAction::CloseNarrative(character_id))
    );
    app.world_mut().write_message(Action::CancelPending);
    warm(&mut app, 4);
    assert!(app.world().resource::<Editor>().pending.is_none());
    action(&mut app, NarrativeAction::Save);
    action(&mut app, NarrativeAction::Open(id));
    action(&mut app, NarrativeAction::Select(1));
    speaker_dropdown(&mut app, window, &target, &output);
    click(&mut app, window, "Выбрать портрет ноды #1");
    action(&mut app, NarrativeAction::SetPortrait(1));
    let history = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .undo
        .len();
    click(&mut app, window, "Выбрать говорящего ноды #1");
    click(&mut app, window, "Выбрать персонажа: Астра");
    action(&mut app, NarrativeAction::SetPortrait(1));
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().portrait, 1);
    }
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .undo
            .len(),
        history
    );
    action(&mut app, NarrativeAction::Fit);
    capture_when(
        &mut app,
        &target,
        &output.join("inline-portrait.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Select(2));
    click(&mut app, window, "Удалить ответ 2 ноды #1");
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().choices.len(), 1);
        assert!(d.node(2).unwrap().choices.is_empty());
    }
    action(&mut app, NarrativeAction::Undo);
    click(&mut app, window, "Связать ответ 1 ноды #1");
    click(&mut app, window, "Разорвать");
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().choices[0].target, None);
    }
    click(&mut app, window, "Связать ответ 1 ноды #1");
    capture_when(
        &mut app,
        &target,
        &output.join("inline-connecting.png"),
        |_| true,
    );
    click(&mut app, window, "Выбрать ЗАВЕРШЕНИЕ #2");
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().choices[0].target, Some(2));
    }
    click(&mut app, window, "Удалить ноду #3");
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert!(d.node(3).is_none());
        assert_eq!(d.node(1).unwrap().choices[1].target, None);
    }
    action(&mut app, NarrativeAction::Undo);
    action(&mut app, NarrativeAction::Select(1));
    action(&mut app, NarrativeAction::Preview);
    capture_when(
        &mut app,
        &target,
        &output.join("preview-portrait.png"),
        |_| true,
    );
    action(&mut app, NarrativeAction::Dismiss);
    action(&mut app, NarrativeAction::AddChoice);
    let count = match &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        Content::Dialogue(d) => d.node(1).unwrap().choices.len(),
        _ => 0,
    };
    assert_eq!(count, 3);
    action(&mut app, NarrativeAction::Undo);
    let count = match &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        Content::Dialogue(d) => d.node(1).unwrap().choices.len(),
        _ => 0,
    };
    assert_eq!(count, 2);
    // Verify tab switches preserve the independent character document.
    action(&mut app, NarrativeAction::Open(character_id));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content
            .title(),
        "Астра"
    );
    action(&mut app, NarrativeAction::Open(id));
    action(&mut app, NarrativeAction::Link(1, 1));
    action(&mut app, NarrativeAction::Select(2));
    if let Content::Dialogue(d) = &app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap()
        .content
    {
        assert_eq!(d.node(1).unwrap().choices[1].target, Some(2));
    }
    action(&mut app, NarrativeAction::Undo);
    action(&mut app, NarrativeAction::Save);
    capture_when(
        &mut app,
        &target,
        &output.join("dialogue-final.png"),
        |_| true,
    );
    text_editor_workflow(&mut app, window, &target, &output);
    // Returning to a scene and opening narrative tabs must keep their independent content.
    app.world_mut().write_message(Action::Open(0));
    warm(&mut app, 5);
    assert!(app.world().resource::<Editor>().narrative.active.is_none());
    action(&mut app, NarrativeAction::Open(id));
    let path = app
        .world()
        .resource::<Editor>()
        .narrative
        .get(id)
        .unwrap()
        .path
        .clone();
    action(&mut app, NarrativeAction::Close(id));
    assert!(app.world().resource::<Editor>().narrative.get(id).is_none());
    let file = app
        .world()
        .resource::<Browser>()
        .files
        .iter()
        .position(|f| f.path == path)
        .unwrap();
    action(&mut app, NarrativeAction::File(file));
    assert_eq!(
        app.world()
            .resource::<Editor>()
            .narrative
            .current()
            .unwrap()
            .content
            .title(),
        "Сигнал из туманности"
    );
    let doc = app
        .world()
        .resource::<Editor>()
        .narrative
        .current()
        .unwrap();
    if let Content::Dialogue(d) = &doc.content {
        assert!(d.warnings().is_empty());
        assert_eq!(d.node(1).unwrap().portrait, 1);
    }
    let reopened = app.world().resource::<Editor>().narrative.active.unwrap();
    action(&mut app, NarrativeAction::Preview);
    action(&mut app, NarrativeAction::CloseCurrent);
    assert!(!app.world().resource::<NarrativeUi>().preview);
    action(&mut app, NarrativeAction::File(file));
    let _ = reopened;
    edit_text(&mut app, DraftField::Title, "Сигнал из туманности · финал");
    {
        let mut editor = app.world_mut().resource_mut::<Editor>();
        if let Content::Character(c) = &mut editor
            .narrative
            .tabs
            .iter_mut()
            .find(|d| d.id == character_id)
            .unwrap()
            .content
        {
            c.description.push('!');
        }
        editor.request_exit();
    }
    assert_eq!(
        app.world().resource::<Editor>().pending,
        Some(PendingAction::ExitNarrative(character_id))
    );
    app.world_mut().write_message(Action::SaveAndContinue);
    warm(&mut app, 4);
    assert!(
        matches!(app.world().resource::<Editor>().pending,Some(PendingAction::ExitNarrative(next)) if next!=character_id)
    );
    app.world_mut().write_message(Action::CancelPending);
    warm(&mut app, 4);
    assert!(app.world().resource::<Editor>().pending.is_none());
}
