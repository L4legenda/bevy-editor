//! Keep scene transitions explicit when there are unsaved edits.
use super::*;
use crate::editor::PendingAction;
use bevy::input_focus::FocusCause;

#[derive(Resource)]
pub(in crate::editor) struct Confirmation {
    overlay: Entity,
    message: Entity,
    error: Entity,
    cancel: Entity,
    previous_focus: Option<Entity>,
    shown: Option<(PendingAction, Room)>,
}

pub(super) fn setup(commands: &mut Commands, root: Entity, font: &Handle<Font>) {
    let overlay = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Color::srgba(0.0, 0.0, 0.0, 0.65),
    );
    commands
        .entity(overlay)
        .insert((GlobalZIndex(100), TabGroup::modal()));
    let card = surface(
        commands,
        overlay,
        Node {
            width: px(480),
            max_width: percent(90),
            padding: UiRect::all(px(24)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            ..column(16.0)
        },
        PANEL,
    );
    text(commands, card, font, "Сохранить изменения?", 21.0, TEXT);
    let message = text(commands, card, font, "", 14.0, TEXT);
    commands.entity(message).insert(TextLayout::default());
    let error = text(commands, card, font, "", 12.0, AMBER);
    commands.entity(error).insert(TextLayout::default());
    let actions = node(
        commands,
        card,
        Node {
            justify_content: JustifyContent::End,
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(8),
            ..row(8.0)
        },
    );
    let cancel = button(
        commands,
        actions,
        font,
        "Отмена",
        Action::CancelPending,
        Look::Regular,
    );
    let discard = button(
        commands,
        actions,
        font,
        "Не сохранять",
        Action::DiscardAndContinue,
        Look::Regular,
    );
    let save = button(
        commands,
        actions,
        font,
        "Сохранить",
        Action::SaveAndContinue,
        Look::Primary,
    );
    for (index, entity) in [cancel, discard, save].into_iter().enumerate() {
        commands.entity(entity).insert(TabIndex(index as i32));
    }
    commands.insert_resource(Confirmation {
        overlay,
        message,
        error,
        cancel,
        previous_focus: None,
        shown: None,
    });
}

pub(in crate::editor) fn update(
    editor: Res<Editor>,
    mut dialog: ResMut<Confirmation>,
    mut focus: ResMut<InputFocus>,
    mut panes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    entities: Query<()>,
    parents: Query<&ChildOf>,
) {
    let request = editor.pending.map(|action| (action, editor.room));
    if dialog.shown != request {
        if let Ok(mut pane) = panes.get_mut(dialog.overlay) {
            pane.display = if editor.pending.is_some() {
                Display::Flex
            } else {
                Display::None
            };
        }
        if editor.pending.is_some() {
            if dialog.shown.is_none() {
                dialog.previous_focus = focus.get();
            }
            focus.set(dialog.cancel, FocusCause::Auto);
        } else if let Some(entity) = dialog
            .previous_focus
            .take()
            .filter(|&entity| entities.contains(entity))
        {
            focus.set(entity, FocusCause::Auto);
        } else {
            focus.clear();
        }
        dialog.shown = request;
    }
    let Some(pending) = editor.pending else {
        return;
    };
    // Clicking the backdrop or a failed save can leave focus outside the modal.
    // TabGroup traps navigation only while focus is already one of its descendants.
    if !focus.get().is_some_and(|entity| {
        parents
            .iter_ancestors(entity)
            .any(|ancestor| ancestor == dialog.overlay)
    }) {
        focus.set(dialog.cancel, FocusCause::Auto);
    }
    let destination = match pending {
        PendingAction::CloseTab(_) | PendingAction::CloseImageTab(_) => "закрытием вкладки",
        PendingAction::Close | PendingAction::CloseImageEditor(_) => "закрытием редактора",
    };
    let message = if let PendingAction::CloseImageTab(id) | PendingAction::CloseImageEditor(id) =
        pending
    {
        format!(
            "В изображении «{}» есть несохранённые изменения. Сохранить слои .pcraft и обновить исходное изображение перед {}?",
            editor
                .image_tab(id)
                .map(|tab| tab.path.display().to_string())
                .unwrap_or_default(),
            destination
        )
    } else {
        format!(
            "В сцене «{}» есть несохранённые изменения. Сохранить их перед {}?",
            editor.definition().label,
            destination
        )
    };
    let error = if editor.status.starts_with("Не удалось") || editor.status.starts_with("Дождитесь")
    {
        editor.status.as_str()
    } else {
        ""
    };
    for (entity, content) in [(dialog.message, message.as_str()), (dialog.error, error)] {
        if let Ok(mut label) = labels.get_mut(entity) {
            if label.0 != content {
                label.0 = content.to_owned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data::LevelEdits, project::EditorProject};
    use bevy::{
        ecs::system::SystemState,
        input_focus::tab_navigation::{NavAction, TabNavigation},
    };

    #[test]
    fn modal_recovers_lost_focus_and_releases_it_when_closed() {
        let project = EditorProject::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/legacy/project.json"),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Editor::new(project, LevelEdits::default()))
            .add_systems(Update, update);
        let root = app.world_mut().spawn(TabGroup::default()).id();
        let outside = app.world_mut().spawn((TabIndex(0), ChildOf(root))).id();
        let overlay = app
            .world_mut()
            .spawn((
                Node {
                    display: Display::None,
                    ..default()
                },
                TabGroup::modal(),
                ChildOf(root),
            ))
            .id();
        let cancel = app.world_mut().spawn((TabIndex(0), ChildOf(overlay))).id();
        let save = app.world_mut().spawn((TabIndex(1), ChildOf(overlay))).id();
        let message = app.world_mut().spawn(Text::new("")).id();
        let error = app.world_mut().spawn(Text::new("")).id();
        app.insert_resource(InputFocus::from_entity(outside));
        app.insert_resource(Confirmation {
            overlay,
            message,
            error,
            cancel,
            previous_focus: None,
            shown: None,
        });

        app.world_mut().resource_mut::<Editor>().pending = Some(PendingAction::Close);
        app.update();
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(cancel));

        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(save, FocusCause::Navigated);
        app.update();
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(save));

        // Closing the app can advance to another dirty scene without changing the action.
        app.world_mut().resource_mut::<Editor>().room = 1;
        app.update();
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(cancel));

        // Reproduce backdrop clicks and a failed save without changing the pending action.
        for escaped_focus in [None, Some(outside)] {
            *app.world_mut().resource_mut::<InputFocus>() = escaped_focus
                .map(InputFocus::from_entity)
                .unwrap_or_default();
            app.update();
            assert_eq!(app.world().resource::<InputFocus>().get(), Some(cancel));
        }

        app.world_mut().resource_mut::<Editor>().pending = None;
        app.update();
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(outside));
        assert_eq!(
            app.world().get::<Node>(overlay).unwrap().display,
            Display::None
        );
        let mut navigation = SystemState::<TabNavigation>::new(app.world_mut());
        assert_eq!(
            navigation
                .get(app.world())
                .unwrap()
                .navigate(&InputFocus::from_entity(outside), NavAction::Next)
                .unwrap(),
            outside,
            "A closed modal must not participate in the background tab sequence",
        );
    }
}
