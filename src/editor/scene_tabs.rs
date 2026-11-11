//! Document tabs mirror the editor's open scenes rather than the whole project.
use bevy::ui::{Selectable, Selected};

use super::*;

pub(super) const LABEL: Color = TEXT;
pub(super) const INACTIVE_LABEL: Color = MUTED;
pub(super) const HOVER: Color = Color::srgba(1.0, 0.88, 0.68, 0.045);
pub(super) const CLOSE_HOVER: Color = Color::srgba(1.0, 0.69, 0.48, 0.14);

fn background(active: bool) -> BackgroundGradient {
    gradient(
        LinearGradient::TO_BOTTOM_RIGHT,
        if active {
            [
                Color::srgb(0.19, 0.156, 0.125),
                Color::srgb(0.113, 0.110, 0.133),
                Color::srgb(0.073, 0.082, 0.104),
            ]
        } else {
            [
                Color::srgb(0.100, 0.100, 0.122),
                Color::srgb(0.078, 0.083, 0.105),
                Color::srgb(0.057, 0.066, 0.082),
            ]
        },
    )
}

fn frame(active: bool, focused: bool) -> BorderGradient {
    if focused {
        return LinearGradient::to_right(vec![
            ColorStop::auto(FOCUS),
            ColorStop::auto(FOCUS.with_alpha(0.86)),
        ])
        .into();
    }
    LinearGradient::to_bottom_right(vec![
        ColorStop::auto(FOCUS.with_alpha(if active { 0.54 } else { 0.13 })),
        ColorStop::auto(Color::srgba(
            0.54,
            0.50,
            0.66,
            if active { 0.32 } else { 0.20 },
        )),
        ColorStop::auto(FOCUS.with_alpha(if active { 0.20 } else { 0.06 })),
    ])
    .into()
}

fn scene_icon(commands: &mut Commands, parent: Entity) {
    let icon = surface(
        commands,
        parent,
        Node {
            width: px(20),
            height: px(20),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Color::srgba(0.83, 0.70, 0.49, 0.065),
    );
    commands
        .entity(icon)
        .insert((BorderColor::all(FOCUS.with_alpha(0.17)), Pickable::IGNORE));
    icons::spawn(
        commands,
        icon,
        icons::Glyph::Layers,
        14.0,
        FOCUS.with_alpha(0.80),
    );
}

fn close_icon(commands: &mut Commands, parent: Entity) -> Entity {
    let icon = icons::spawn(commands, parent, icons::Glyph::Close, 12.0, MUTED);
    commands
        .entity(icon)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.position_type = PositionType::Absolute;
            node.left = percent(50);
            node.top = percent(50);
        });
    commands
        .entity(icon)
        .insert(UiTransform::from_translation(Val2::percent(-50, -50)));
    icon
}

struct Tab {
    room: Room,
    group: Entity,
    open: Entity,
    close: Entity,
    close_icon: Entity,
    label: Entity,
    dirty_indicator: Entity,
    accent: Entity,
    caption: String,
    active: bool,
    dirty: bool,
    focused: bool,
    show_close: bool,
}

#[derive(Default)]
pub(in crate::editor) struct Tabs {
    rooms: Option<Vec<Room>>,
    controls: Vec<Tab>,
}

pub(super) fn setup(commands: &mut Commands, parent: Entity) -> Entity {
    let strip = node(
        commands,
        parent,
        Node {
            height: px(40),
            min_width: px(0),
            flex_grow: 1.0,
            overflow: Overflow::scroll_x(),
            ..row(6.0)
        },
    );
    commands.entity(strip).insert((
        Scroller,
        HorizontalScroller,
        AccessibleLabel("Открытые сцены".into()),
    ));
    strip
}

pub(super) fn empty_workspace(
    commands: &mut Commands,
    viewport: Entity,
    font: &Handle<Font>,
    stars: &starfield::Starfield,
) -> Entity {
    let empty = surface(
        commands,
        viewport,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            padding: UiRect::all(px(24)),
            ..column(12.0)
        },
        PANEL,
    );
    commands.entity(empty).insert((
        Pickable::IGNORE,
        gradient(
            LinearGradient::TO_BOTTOM_RIGHT,
            [
                Color::srgb(0.059, 0.067, 0.086),
                Color::srgb(0.025, 0.032, 0.041),
                Color::srgb(0.068, 0.055, 0.081),
            ],
        ),
    ));
    starfield::background(commands, empty, &stars.space);
    orbital::illustration(commands, empty);
    let title = text(commands, empty, font, "Нет открытых сцен", 23.0, TEXT);
    commands.entity(title).insert(TextLayout::default());
    empty
}

fn caption(editor: &Editor, room: Room) -> String {
    editor.project.scenes[room].label.clone()
}

fn accessible_label(editor: &Editor, room: Room, active: bool, dirty: bool) -> String {
    format!(
        "{}{}{}",
        editor.project.scenes[room].label,
        if active {
            " · активная сцена"
        } else {
            ""
        },
        if dirty {
            " · есть несохранённые изменения"
        } else {
            ""
        },
    )
}

pub(super) fn refresh(
    commands: &mut Commands,
    editor: &Editor,
    numeric: &NumericInput,
    interface: &Interface,
    tabs: &mut Tabs,
    labels: &mut Query<(&mut Text, &mut TextColor)>,
    focused: Option<Entity>,
    hovered: impl Fn(Entity) -> bool,
) {
    let document_dirty =
        |room| editor.document_dirty(room) || (room == editor.room && numeric.has_changes(editor));
    if tabs.rooms.as_ref() != Some(&editor.open_tabs) {
        commands
            .entity(interface.tabs)
            .despawn_related::<Children>();
        tabs.controls.clear();
        tabs.rooms = Some(editor.open_tabs.clone());
        if editor.open_tabs.is_empty() {
            text(
                commands,
                interface.tabs,
                &interface.font,
                "Нет открытых сцен",
                12.0,
                MUTED,
            );
        }
        for &room in &editor.open_tabs {
            let group = node(
                commands,
                interface.tabs,
                Node {
                    height: px(36),
                    min_width: px(160),
                    max_width: px(280),
                    flex_shrink: 0.0,
                    padding: UiRect::all(px(3)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(10)),
                    overflow: Overflow::clip(),
                    ..row(2.0)
                },
            );
            let open = control(
                commands,
                group,
                Action::Open(room),
                Look::Tab,
                Node {
                    height: percent(100),
                    min_width: px(0),
                    max_width: px(246),
                    flex_grow: 1.0,
                    padding: UiRect {
                        left: px(5),
                        right: px(5),
                        ..default()
                    },
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(6)),
                    overflow: Overflow::clip(),
                    ..row(7.0)
                },
            );
            scene_icon(commands, open);
            let active = editor.has_scene() && editor.room == room;
            let caption = caption(editor, room);
            let title = node(
                commands,
                open,
                Node {
                    min_width: px(0),
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    overflow: Overflow::clip(),
                    ..default()
                },
            );
            commands.entity(title).insert(Pickable::IGNORE);
            let label = text(
                commands,
                title,
                &interface.font,
                &caption,
                13.0,
                if active { LABEL } else { INACTIVE_LABEL },
            );
            let dirty = document_dirty(room);
            commands
                .entity(group)
                .insert((background(active), frame(active, false)));
            commands.entity(open).insert((
                Selectable,
                AccessibleLabel(accessible_label(editor, room, active, dirty)),
            ));
            if active {
                commands.entity(open).insert(Selected);
            }
            let close = control(
                commands,
                group,
                Action::CloseTab(room),
                Look::TabClose,
                Node {
                    width: px(24),
                    height: px(24),
                    flex_shrink: 0.0,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(6)),
                    ..row(0.0)
                },
            );
            commands.entity(close).insert(AccessibleLabel(format!(
                "Закрыть сцену {}",
                editor.project.scenes[room].label,
            )));
            let close_icon = close_icon(commands, close);
            commands.entity(close_icon).insert(if dirty {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
            let dirty_indicator = surface(
                commands,
                close,
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(50),
                    top: percent(50),
                    width: px(5),
                    height: px(5),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                FOCUS,
            );
            commands.entity(dirty_indicator).insert((
                UiTransform::from_translation(Val2::percent(-50, -50)),
                Pickable::IGNORE,
                if dirty {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ));
            let accent = node(
                commands,
                group,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: px(0),
                    left: px(12),
                    right: px(12),
                    height: px(1),
                    ..default()
                },
            );
            commands.entity(accent).insert((
                BackgroundGradient::from(LinearGradient::to_right(vec![
                    ColorStop::auto(FOCUS.with_alpha(0.0)),
                    ColorStop::auto(FOCUS.with_alpha(0.88)),
                    ColorStop::auto(FOCUS.with_alpha(0.0)),
                ])),
                Pickable::IGNORE,
                if active {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ));
            tabs.controls.push(Tab {
                room,
                group,
                open,
                close,
                close_icon,
                label,
                dirty_indicator,
                accent,
                caption,
                active,
                dirty,
                focused: false,
                show_close: !dirty,
            });
        }
    }
    for tab in &mut tabs.controls {
        let caption = caption(editor, tab.room);
        let active = editor.has_scene() && editor.room == tab.room;
        let dirty = document_dirty(tab.room);
        let tab_focused = focused.is_some_and(|entity| entity == tab.open || entity == tab.close);
        if tab.caption != caption
            || tab.active != active
            || tab.dirty != dirty
            || tab.focused != tab_focused
        {
            commands
                .entity(tab.group)
                .insert((background(active), frame(active, tab_focused)));
            if let Ok((mut label, mut color)) = labels.get_mut(tab.label) {
                label.0.clone_from(&caption);
                color.set_if_neq(TextColor(if active { LABEL } else { INACTIVE_LABEL }));
            }
            commands.entity(tab.accent).insert(if active {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            commands
                .entity(tab.open)
                .insert(AccessibleLabel(accessible_label(
                    editor, tab.room, active, dirty,
                )));
            if active {
                commands.entity(tab.open).insert(Selected);
            } else {
                commands.entity(tab.open).remove::<Selected>();
            }
            tab.caption = caption;
            tab.active = active;
            tab.dirty = dirty;
            tab.focused = tab_focused;
        }
        let show_close =
            !dirty || hovered(tab.open) || hovered(tab.close) || focused == Some(tab.close);
        if tab.show_close != show_close {
            commands.entity(tab.close_icon).insert(if show_close {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
            commands.entity(tab.dirty_indicator).insert(if show_close {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
            tab.show_close = show_close;
        }
    }
}
