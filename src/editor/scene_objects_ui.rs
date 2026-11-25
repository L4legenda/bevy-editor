//! Commands on scene instances, independent of their shared source assets.
use super::*;
use crate::editor::scene_hierarchy::{self, HierarchyAction};
use bevy::input_focus::FocusCause;

#[cfg(test)]
#[path = "scene_objects_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum ObjectAction {
    Menu(Selection),
    Rename(Selection),
    EditName,
    Hierarchy(HierarchyAction),
    Duplicate,
    Remove,
    Confirm,
    Cancel,
}

#[derive(Clone, Copy)]
struct Bound {
    room: Room,
    target: Selection,
}

#[derive(Resource)]
struct ObjectUi {
    root: Entity,
    menu: Entity,
    hierarchy_menu: Entity,
    dialog: Entity,
    field: Entity,
    error: Entity,
    bound: Option<Bound>,
    rename: bool,
    confirm: bool,
    suppress_click: bool,
    message: String,
}

pub(in crate::editor) struct SceneObjectsPlugin;
impl Plugin for SceneObjectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup.after(super::setup))
            .add_systems(Update, input.after(super::input).before(super::shortcuts));
    }
}

pub(super) fn available(editor: &Editor, action: ObjectAction) -> bool {
    editor.has_scene()
        && editor.ready
        && !editor.asset_viewer_open()
        && editor.pending.is_none()
        && !editor.browser_popup
        && match action {
            ObjectAction::Menu(target) | ObjectAction::Rename(target) => {
                editor.object_editable(target)
            }
            ObjectAction::Hierarchy(action) => {
                editor.scene_object_popup
                    && !editor.multiple_selected()
                    && scene_hierarchy::available(editor, action)
            }
            ObjectAction::Duplicate => editor.scene_object_popup && editor.batch_editable(),
            _ => editor.scene_object_popup,
        }
}

pub(super) fn pointer_menu(
    mut event: On<PointerClick>,
    controls: Query<&Control>,
    editor: Res<Editor>,
    mut actions: MessageWriter<Action>,
) {
    if event.button != PointerButton::Secondary {
        return;
    }
    if let Ok(Control {
        action: Action::Select(target),
        ..
    }) = controls.get(event.entity)
        && available(&editor, ObjectAction::Menu(*target))
    {
        event.propagate(false);
        actions.write(Action::SceneObject(ObjectAction::Menu(*target)));
    }
}

fn menu_button(
    commands: &mut Commands,
    menu: Entity,
    font: &Handle<Font>,
    label: &str,
    glyph: icons::Glyph,
    action: ObjectAction,
) {
    let item = button(
        commands,
        menu,
        font,
        label,
        Action::SceneObject(action),
        Look::Regular,
    );
    commands
        .entity(item)
        .entry::<Node>()
        .and_modify(|mut node| node.width = percent(100));
    let icon = icons::themed(commands, item, glyph, 15.0);
    commands.entity(item).insert_children(0, &[icon]);
}

const HIERARCHY_ACTIONS: [(&str, icons::Glyph, HierarchyAction); 4] = [
    (
        "Сменить родителя",
        icons::Glyph::FolderOpen,
        HierarchyAction::PickParent,
    ),
    ("В корень", icons::Glyph::ArrowUp, HierarchyAction::ToRoot),
    (
        "Переместить выше",
        icons::Glyph::ChevronUp,
        HierarchyAction::SiblingEarlier,
    ),
    (
        "Переместить ниже",
        icons::Glyph::ChevronDown,
        HierarchyAction::SiblingLater,
    ),
];

fn setup(mut commands: Commands, interface: Res<Interface>) {
    let root = node(
        &mut commands,
        interface.body,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            ..default()
        },
    );
    commands
        .entity(root)
        .insert((GlobalZIndex(90), BackgroundColor(Color::NONE)));
    let menu = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            width: px(220),
            padding: UiRect::all(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(3.0)
        },
        PANEL,
    );
    commands.entity(menu).insert((
        TabGroup::modal(),
        AccessibleLabel("Действия с объектом".into()),
    ));
    for (label, glyph, action) in [
        (
            "Переименовать · F2",
            icons::Glyph::Pencil,
            ObjectAction::EditName,
        ),
        (
            "Дублировать",
            icons::Glyph::Copy,
            ObjectAction::Duplicate,
        ),
        (
            "Удалить · Delete",
            icons::Glyph::Trash,
            ObjectAction::Remove,
        ),
    ] {
        menu_button(&mut commands, menu, &interface.font, label, glyph, action);
    }
    let hierarchy_menu = surface(
        &mut commands,
        menu,
        Node {
            display: Display::None,
            padding: UiRect::top(px(3)),
            border: UiRect::top(px(1)),
            ..column(3.0)
        },
        Color::NONE,
    );
    commands
        .entity(hierarchy_menu)
        .insert(BorderColor::all(EDGE));
    for (label, glyph, action) in HIERARCHY_ACTIONS {
        menu_button(
            &mut commands,
            hierarchy_menu,
            &interface.font,
            label,
            glyph,
            ObjectAction::Hierarchy(action),
        );
    }
    let dialog = surface(
        &mut commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            width: px(300),
            padding: UiRect::all(px(14)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..column(10.0)
        },
        PANEL,
    );
    commands.entity(dialog).insert((
        TabGroup::modal(),
        AccessibleLabel("Переименование объекта".into()),
    ));
    text(
        &mut commands,
        dialog,
        &interface.font,
        "Переименовать объект",
        14.0,
        TEXT,
    );
    let field = commands
        .spawn((
            TextInput,
            EditableText::new(""),
            bevy::ui_widgets::SelectAllOnFocus,
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel("Имя объекта".into()),
            TabIndex(0),
            font(&interface.font, 13.0),
            TextColor(TEXT),
            TextLayout::no_wrap(),
            Node {
                width: percent(100),
                height: px(30),
                padding: UiRect::axes(px(7), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            ChildOf(dialog),
        ))
        .id();
    let error = text(&mut commands, dialog, &interface.font, "", 11.0, ERROR);
    commands.entity(error).insert(TextLayout::default());
    let buttons = node(
        &mut commands,
        dialog,
        Node {
            width: percent(100),
            justify_content: JustifyContent::End,
            ..row(6.0)
        },
    );
    for (label, action, look) in [
        ("Отмена", ObjectAction::Cancel, Look::Regular),
        ("Переименовать", ObjectAction::Confirm, Look::Primary),
    ] {
        button(
            &mut commands,
            buttons,
            &interface.font,
            label,
            Action::SceneObject(action),
            look,
        );
    }
    commands.insert_resource(ObjectUi {
        root,
        menu,
        hierarchy_menu,
        dialog,
        field,
        error,
        bound: None,
        rename: false,
        confirm: false,
        suppress_click: false,
        message: String::new(),
    });
}

fn input(
    mut editor: ResMut<Editor>,
    mut ui: ResMut<ObjectUi>,
    mut actions: MessageReader<Action>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut focus: ResMut<InputFocus>,
    mut inputs: Query<&mut EditableText>,
    mut nodes: Query<&mut Node>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut labels: Query<&mut Text>,
    mut backgrounds: Query<&mut BackgroundColor>,
    mut borders: Query<&mut BorderColor>,
) {
    if !mouse.any_pressed([MouseButton::Left, MouseButton::Right])
        && !mouse.any_just_released([MouseButton::Left, MouseButton::Right])
    {
        ui.suppress_click = false;
    }
    let confirm = std::mem::take(&mut ui.confirm);
    let mut rename = None;
    for action in actions.read() {
        match *action {
            Action::SceneObject(ObjectAction::Menu(target))
                if available(&editor, ObjectAction::Menu(target)) =>
            {
                if !editor.is_selected(target) {
                    editor.select(Some(target));
                }
                ui.bound = Some(Bound {
                    room: editor.room,
                    target,
                });
                ui.rename = false;
                ui.message.clear();
                focus.clear();
                let cursor = window
                    .cursor_position()
                    .unwrap_or(Vec2::new(window.width() - 230.0, TOP + 20.0));
                let hierarchy_height =
                    if !editor.multiple_selected() && editor.hierarchy_key(target).is_some() {
                        32.0 * HIERARCHY_ACTIONS.len() as f32 + 4.0
                    } else {
                        0.0
                    };
                if let Ok(mut menu) = nodes.get_mut(ui.menu) {
                    menu.left = px(cursor.x.clamp(4.0, (window.width() - 224.0).max(4.0)));
                    menu.top = px((cursor.y - TOP).clamp(
                        4.0,
                        (window.height() - TOP - FOOT - 114.0 - hierarchy_height).max(4.0),
                    ));
                }
            }
            Action::SceneObject(ObjectAction::Rename(target))
                if available(&editor, ObjectAction::Rename(target)) =>
            {
                rename = Some(target);
            }
            Action::SceneObject(ObjectAction::EditName) => {
                rename = ui.bound.map(|bound| bound.target);
            }
            Action::SceneObject(ObjectAction::Hierarchy(command))
                if !ui.rename && available(&editor, ObjectAction::Hierarchy(command)) =>
            {
                if ui.bound.is_some_and(|bound| {
                    bound.room == editor.room && editor.selection == Some(bound.target)
                }) {
                    editor.hierarchy_action(command);
                    ui.bound = None;
                    ui.suppress_click = true;
                    focus.clear();
                }
            }
            Action::SceneObject(ObjectAction::Confirm) => {
                ui.confirm = ui.rename;
            }
            Action::SceneObject(ObjectAction::Cancel) => {
                ui.bound = None;
                ui.suppress_click = true;
                focus.clear();
            }
            Action::SceneObject(command @ (ObjectAction::Duplicate | ObjectAction::Remove)) => {
                let duplicate = matches!(command, ObjectAction::Duplicate);
                if let Some(bound) = ui.bound.filter(|bound| {
                    bound.room == editor.room && editor.object_editable(bound.target)
                }) {
                    if !editor.is_selected(bound.target) {
                        editor.select(Some(bound.target));
                    }
                    if !duplicate || editor.batch_editable() {
                        editor.finish_camera_navigation();
                        let before = editor.edits.clone();
                        if duplicate {
                            editor.duplicate_targets();
                        } else {
                            editor.delete_targets();
                            editor.status = "Объект удалён · Ctrl+Z — восстановить".into();
                        }
                        editor.finish_edit(before);
                    }
                }
                ui.bound = None;
                ui.suppress_click = true;
                focus.clear();
            }
            Action::Open(_) | Action::CloseTab(_) | Action::ViewModel(_) | Action::ViewImage(_) => {
                ui.bound = None;
                ui.confirm = false;
                focus.clear();
            }
            _ => {}
        }
    }
    if let Some(target) = rename.filter(|&target| editor.object_editable(target)) {
        editor.select(Some(target));
        ui.bound = Some(Bound {
            room: editor.room,
            target,
        });
        ui.rename = true;
        ui.message.clear();
        ui.confirm = false;
        if let Ok(mut input) = inputs.get_mut(ui.field) {
            input.editor_mut().set_text(&editor.label(target));
        }
        focus.set(ui.field, FocusCause::Auto);
    }
    if ui
        .bound
        .is_some_and(|bound| bound.room != editor.room || !editor.object_editable(bound.target))
        || !window.focused
        || editor.pending.is_some()
        || editor.asset_viewer_open()
        || editor.browser_popup
        || !editor.has_scene()
    {
        if ui.bound.take().is_some() {
            focus.clear();
        }
    }
    if ui.bound.is_some() {
        if keys.just_pressed(KeyCode::Escape) {
            keys.clear_just_pressed(KeyCode::Escape);
            ui.bound = None;
            ui.confirm = false;
            focus.clear();
        } else if ui.rename && keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
            // Read the final character after native PostUpdate text input, including
            // when the last character and Enter arrive in one OS input batch.
            ui.confirm = true;
        }
        if confirm
            && ui.rename
            && let Some(bound) = ui.bound
        {
            if let Ok(input) = inputs.get(ui.field) {
                match editor.rename_object(bound.target, &input.value().to_string()) {
                    Ok(()) => {
                        ui.bound = None;
                        ui.suppress_click = true;
                        focus.clear();
                    }
                    Err(message) => {
                        ui.message = message.into();
                        focus.set(ui.field, FocusCause::Auto);
                    }
                }
            }
        }
        let shown = if ui.rename { ui.dialog } else { ui.menu };
        if mouse.any_just_pressed([MouseButton::Left, MouseButton::Right])
            && window.cursor_position().is_some_and(|cursor| {
                geometry.get(shown).is_ok_and(|(node, transform)| {
                    !node.is_empty()
                        && !node.contains_point(*transform, cursor / node.inverse_scale_factor())
                })
            })
        {
            ui.bound = None;
            ui.confirm = false;
            ui.suppress_click = true;
            focus.clear();
        }
    }
    editor.scene_object_popup = ui.bound.is_some() || ui.suppress_click;
    let visible = ui.bound.is_some();
    let hierarchy_visible = !ui.rename
        && !editor.multiple_selected()
        && ui
            .bound
            .is_some_and(|bound| editor.hierarchy_key(bound.target).is_some());
    for (entity, shown) in [
        (ui.root, visible),
        (ui.menu, visible && !ui.rename),
        (ui.hierarchy_menu, hierarchy_visible),
        (ui.dialog, visible && ui.rename),
        (ui.error, !ui.message.is_empty()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if shown { Display::Flex } else { Display::None };
        }
    }
    if let Ok(mut dialog) = nodes.get_mut(ui.dialog) {
        dialog.left = px(((window.width() - 300.0) * 0.5).max(4.0));
        dialog.top = px(((window.height() - TOP - FOOT - 160.0) * 0.5).max(4.0));
    }
    if let Ok(mut background) = backgrounds.get_mut(ui.root) {
        background.0 = if ui.rename {
            Color::srgba(0.0, 0.0, 0.0, 0.35)
        } else {
            Color::NONE
        };
    }
    if let Ok(mut error) = labels.get_mut(ui.error) {
        error.0.clone_from(&ui.message);
    }
    if let Ok(mut border) = borders.get_mut(ui.field) {
        border.set_if_neq(BorderColor::all(if ui.message.is_empty() {
            FOCUS
        } else {
            ERROR
        }));
    }
}
