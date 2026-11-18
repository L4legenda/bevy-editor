use super::*;
use crate::editor::{
    selection_outline::render_tests::{capture_when, frame},
    tests::lighting_ui_render::{mouse, move_pointer},
};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    input_focus::InputFocus,
};
use edit::layers::{LayerAction as L, ui::InlineName};

fn warm(app: &mut App, count: usize) {
    for _ in 0..count {
        frame(app);
    }
}
fn layer(app: &mut App, action: L) {
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Layers(action)));
    warm(app, 4);
}
fn document(app: &App) -> &crate::editor::photo_document::PhotoDocument {
    let viewer = app.world().resource::<ImageViewer>();
    viewer.edit.document(viewer.tab).unwrap()
}
fn field(app: &mut App) -> Entity {
    let world = app.world_mut();
    world
        .query_filtered::<Entity, With<InlineName>>()
        .single(world)
        .expect("Name edits in the layer row")
}
fn draft(app: &mut App, text: &str) {
    let entity = field(app);
    app.world_mut()
        .get_mut::<EditableText>(entity)
        .unwrap()
        .editor_mut()
        .set_text(text);
    warm(app, 2);
}
fn key(app: &mut App, window: Entity, key_code: KeyCode, logical_key: Key) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key: logical_key.clone(),
            state,
            text: None,
            repeat: false,
        });
        warm(app, 2);
    }
}
#[track_caller]
fn closed(app: &mut App) {
    assert!(
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .is_none(),
        "Remaining form: {:?}, error: {:?}, focus: {:?}",
        app.world()
            .resource::<ImageViewer>()
            .edit
            .layers
            .form
            .as_ref()
            .map(|f| (
                f.command,
                &f.params,
                f.fields
                    .iter()
                    .map(|field| (&field.key, &field.text, field.enabled))
                    .collect::<Vec<_>>()
            )),
        app.world().resource::<ImageViewer>().edit.error,
        app.world().resource::<InputFocus>().get(),
    );
    let world = app.world_mut();
    assert_eq!(
        world
            .query_filtered::<Entity, With<InlineName>>()
            .iter(world)
            .count(),
        0
    );
}

#[test]
fn image_layer_rename_is_pinned_even_with_multiple_layers_selected() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let id = file(&app, "d-small.png");
    app.world_mut().write_message(Action::ViewImage(id));
    wait_for_image(&mut app);
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Edit));
    wait_for_edit(&mut app);
    for action in [
        L::Quick("layer.duplicate"),
        L::Select(0, false, false),
        L::Select(1, true, false),
        L::Command("layer.renameLayer"),
    ] {
        app.world_mut()
            .write_message(Action::ImageEdit(edit::EditAction::Layers(action)));
        app.update();
    }
    assert_eq!(document(&app).selected.len(), 2);
    let id = document(&app).active_id().unwrap();
    let viewer = app.world().resource::<ImageViewer>();
    assert_eq!(
        viewer.edit.layers.form.as_ref().unwrap().params["layer"].as_u64(),
        Some(id.0)
    );
}

#[test]
#[ignore = "requires GPU: native inline rename, text focus, Enter, Escape, blur and undo"]
fn image_layer_inline_rename_stays_in_row_and_preserves_text_and_history() {
    let fixture = Fixture::new();
    let (mut app, window, target) = lighting_ui_render::fixture();
    app.world_mut()
        .resource_scope(|world, mut browser: Mut<Browser>| {
            let mut editor = world.resource_mut::<Editor>();
            editor.root = fixture.0.clone();
            editor.project.asset_root = fixture.0.clone();
            editor.drawer = false;
            browser.refresh(&mut editor.project);
        });
    let id = file(&app, "d-small.png");
    app.world_mut().write_message(Action::ViewImage(id));
    warm(&mut app, 40);
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
    warm(&mut app, 8);
    layer(&mut app, L::Quick("layer.duplicate"));
    let original = document(&app).active_layer().unwrap().name.clone();
    let renamed = document(&app).active_id().unwrap();
    let index = document(&app).active;
    // Two real clicks enter editing without a separate property window.
    for _ in 0..2 {
        click_action(
            &mut app,
            window,
            |a| matches!(a, Action::ImageEdit(edit::EditAction::SelectLayer(i)) if i == index),
        );
    }
    let entity = field(&mut app);
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(entity));
    let field_rect = {
        let node = app.world().get::<ComputedNode>(entity).unwrap();
        let transform = app.world().get::<UiGlobalTransform>(entity).unwrap();
        assert!(!node.is_empty());
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    let row_rect = {
        let world = app.world_mut();
        let (node, transform) = world
            .query::<(
                &edit::layers::ui::drag_ui::DropRow,
                &ComputedNode,
                &UiGlobalTransform,
            )>()
            .iter(world)
            .find_map(|(row, node, t)| (row.id == renamed.0).then_some((node, t)))
            .unwrap();
        Rect::from_center_size(
            transform.affine().translation * node.inverse_scale_factor(),
            node.size() * node.inverse_scale_factor(),
        )
    };
    assert!(row_rect.contains(field_rect.center()));
    {
        let world = app.world_mut();
        assert!(
            !world
                .query::<(&Control, &ComputedNode)>()
                .iter(world)
                .any(|(c, n)| {
                    !n.is_empty()
                        && matches!(
                            c.action,
                            Action::ImageEdit(edit::EditAction::Layers(L::Apply))
                        )
                }),
            "Rename must not show the modal's Apply button"
        );
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/layer-rename-qa");
    std::fs::create_dir_all(&output).unwrap();
    capture_when(
        &mut app,
        &target,
        &output.join("01-inline-name.png"),
        |_| true,
    );

    // All text starts selected. A character + Enter in one event batch replaces it
    // and keeps that final character despite native PostUpdate text processing.
    for (key_code, logical_key, text) in [
        (
            KeyCode::KeyA,
            Key::Character("Небо".into()),
            Some("Небо".into()),
        ),
        (KeyCode::Enter, Key::Enter, None),
    ] {
        app.world_mut().write_message(KeyboardInput {
            window,
            key_code,
            logical_key,
            state: ButtonState::Pressed,
            text,
            repeat: false,
        });
    }
    warm(&mut app, 3);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    closed(&mut app);
    assert_eq!(document(&app).document.layer(renamed).unwrap().name, "Небо");
    app.world_mut()
        .write_message(Action::ImageEdit(edit::EditAction::Undo));
    warm(&mut app, 4);
    assert_eq!(
        document(&app).document.layer(renamed).unwrap().name,
        original
    );

    layer(&mut app, L::Command("layer.renameLayer"));
    draft(&mut app, "Отменённое имя");
    let entity = field(&mut app);
    // Selecting text inside the field cannot start moving the layer or rebuild it.
    move_pointer(&mut app, window, field_rect.center());
    mouse(&mut app, window, ButtonState::Pressed);
    move_pointer(&mut app, window, field_rect.center() + Vec2::new(16.0, 0.0));
    mouse(&mut app, window, ButtonState::Released);
    assert_eq!(field(&mut app), entity);
    assert!(
        !app.world()
            .resource::<edit::layers::ui::drag_ui::LayerDrag>()
            .active
    );
    key(&mut app, window, KeyCode::Escape, Key::Escape);
    closed(&mut app);
    assert_eq!(
        document(&app).document.layer(renamed).unwrap().name,
        original
    );

    // Clicking elsewhere saves the draft to its own stable layer ID.
    layer(&mut app, L::Command("layer.renameLayer"));
    draft(&mut app, "Передний план");
    let other = if index == 0 { 1 } else { 0 };
    click_action(
        &mut app,
        window,
        |a| matches!(a, Action::ImageEdit(edit::EditAction::SelectLayer(i)) if i == other),
    );
    warm(&mut app, 4);
    closed(&mut app);
    assert_eq!(
        document(&app).document.layer(renamed).unwrap().name,
        "Передний план"
    );
    assert_ne!(document(&app).active_id(), Some(renamed));

    // Empty names and unchanged names close editing without a history entry.
    let revision = document(&app).revision;
    for name in [
        "   ".to_owned(),
        document(&app).active_layer().unwrap().name.clone(),
    ] {
        layer(&mut app, L::Command("layer.renameLayer"));
        draft(&mut app, &name);
        key(&mut app, window, KeyCode::Enter, Key::Enter);
        closed(&mut app);
        assert_eq!(document(&app).revision, revision);
    }
    layer(&mut app, L::Command("layer.renameLayer"));
    draft(&mut app, "Фон");
    move_pointer(&mut app, window, Vec2::new(field_rect.center().x, 720.0));
    mouse(&mut app, window, ButtonState::Pressed);
    mouse(&mut app, window, ButtonState::Released);
    closed(&mut app);
    assert_eq!(document(&app).active_layer().unwrap().name, "Фон");
    // Groups use exactly the same inline editor.
    layer(&mut app, L::Quick("layer.new.group"));
    layer(&mut app, L::Command("layer.renameLayer"));
    draft(&mut app, "Окружение");
    key(&mut app, window, KeyCode::Enter, Key::Enter);
    closed(&mut app);
    assert_eq!(document(&app).active_layer().unwrap().name, "Окружение");
    capture_when(&mut app, &target, &output.join("02-renamed.png"), |_| true);
}
