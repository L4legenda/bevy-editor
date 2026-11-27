//! Search and disclosure controls shared by the scene and model inspectors.
use super::*;
use crate::editor::ui::icons::Glyph;
use bevy::ui_widgets::SelectAllOnFocus;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum InspectorAction {
    ToggleTransform,
    Fold(Entity),
    ToggleSearch,
    ClearSearch,
}

#[derive(Resource)]
pub(in crate::editor) struct Panel {
    bar: Entity,
    search_toggle: Entity,
    search: Entity,
    clear: Entity,
    placeholder: Entity,
    icon: Entity,
    empty: Entity,
    content: Entity,
}

pub(super) struct Shell {
    pub content: Entity,
    pub name: Entity,
    pub kind: Entity,
}

pub(super) fn setup(commands: &mut Commands, parent: Entity, font_handle: &Handle<Font>) -> Shell {
    let top = surface(
        commands,
        parent,
        Node {
            flex_shrink: 0.0,
            min_width: px(0),
            padding: UiRect::axes(px(10), px(8)),
            border: UiRect::bottom(px(1)),
            ..column(7.0)
        },
        PANEL,
    );
    let identity = node(commands, top, row(8.0));
    let tile = surface(
        commands,
        identity,
        Node {
            width: px(30),
            height: px(30),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(7)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        ACTIVE,
    );
    let icon = icons::spawn(commands, tile, Glyph::Box, 16.0, FOCUS);
    let titles = node(
        commands,
        identity,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..column(1.0)
        },
    );
    let name = text(commands, titles, font_handle, "Выберите объект", 15.0, TEXT);
    let kind = text(commands, titles, font_handle, "", 11.0, MUTED);

    let search_toggle = node(commands, identity, row(0.0));
    icon_button(
        commands,
        search_toggle,
        Glyph::Search,
        "Найти свойство",
        Action::Inspector(InspectorAction::ToggleSearch),
    );

    let bar = surface(
        commands,
        top,
        Node {
            height: px(28),
            display: Display::None,
            min_width: px(0),
            padding: UiRect::horizontal(px(7)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(5.0)
        },
        WELL,
    );
    icons::spawn(commands, bar, Glyph::Search, 13.0, MUTED);
    let search = commands
        .spawn((
            TextInput,
            SelectAllOnFocus,
            EditableText::new(""),
            crate::ui_theme::text_cursor_style(),
            font(font_handle, 12.0),
            TextColor(TEXT),
            AccessibleLabel("Поиск свойств".into()),
            TabIndex(0),
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                height: px(24),
                padding: UiRect::top(px(3)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(bar),
        ))
        .id();
    let placeholder = text(commands, search, font_handle, "Поиск свойств", 12.0, MUTED);
    commands.entity(placeholder).insert(Node {
        position_type: PositionType::Absolute,
        top: px(3),
        ..default()
    });
    let clear = node(
        commands,
        bar,
        Node {
            display: Display::None,
            flex_shrink: 0.0,
            ..row(0.0)
        },
    );
    icon_button(
        commands,
        clear,
        Glyph::Close,
        "Очистить поиск свойств",
        Action::Inspector(InspectorAction::ClearSearch),
    );
    let content = node(
        commands,
        parent,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            min_width: px(0),
            padding: UiRect::axes(px(8), px(8)),
            overflow: Overflow::scroll_y(),
            ..column(6.0)
        },
    );
    commands.entity(content).insert((
        Scroller,
        AccessibleLabel("Свойства выбранного объекта".into()),
    ));
    let empty = text(
        commands,
        content,
        font_handle,
        "Свойства не найдены",
        12.0,
        MUTED,
    );
    commands.entity(empty).insert(Node {
        display: Display::None,
        padding: UiRect::all(px(8)),
        ..default()
    });
    commands.insert_resource(Panel {
        bar,
        search_toggle,
        search,
        clear,
        placeholder,
        icon,
        empty,
        content,
    });
    Shell {
        content,
        name,
        kind,
    }
}

pub(super) fn icon_button(
    commands: &mut Commands,
    parent: Entity,
    glyph: Glyph,
    label: &str,
    action: Action,
) -> Entity {
    let button = control(
        commands,
        parent,
        action,
        Look::Regular,
        Node {
            width: px(24),
            height: px(22),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(4)),
            justify_content: JustifyContent::Center,
            ..row(0.0)
        },
    );
    commands
        .entity(button)
        .insert(AccessibleLabel(label.into()));
    icons::themed(commands, button, glyph, 13.0);
    button
}

#[derive(Component)]
pub(super) struct Section {
    keywords: String,
    body: Entity,
    chevron: Entity,
    open: bool,
    folded_during_search: bool,
}

impl Section {
    pub(super) fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    pub(super) fn open(&self) -> bool {
        self.open
    }
}

pub(super) struct Group {
    pub root: Entity,
    pub button: Entity,
    pub title: Entity,
    pub body: Entity,
}

pub(super) fn section(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    title: &str,
    aliases: &str,
    open: bool,
) -> Group {
    let root = surface(
        commands,
        parent,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            min_width: px(0),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
        PANEL,
    );
    let header = surface(
        commands,
        root,
        Node {
            min_height: px(29),
            min_width: px(0),
            padding: UiRect::right(px(5)),
            ..row(2.0)
        },
        Color::srgb(0.11, 0.112, 0.135),
    );
    let button = control(
        commands,
        header,
        Action::Inspector(InspectorAction::Fold(root)),
        Look::Regular,
        Node {
            height: px(29),
            min_width: px(0),
            flex_grow: 1.0,
            padding: UiRect::horizontal(px(7)),
            overflow: Overflow::clip(),
            ..row(5.0)
        },
    );
    commands
        .entity(button)
        .insert(AccessibleLabel(title.into()));
    let chevron = icons::spawn(commands, button, disclosure(open), 12.0, MUTED);
    let title = text(commands, button, font, title, 13.0, TEXT);
    let body = node(
        commands,
        root,
        Node {
            display: display(open),
            min_width: px(0),
            padding: UiRect::axes(px(8), px(6)),
            ..column(5.0)
        },
    );
    commands.entity(root).insert(Section {
        keywords: normalize(&format!("{title} {aliases}")),
        body,
        chevron,
        open,
        folded_during_search: false,
    });
    Group {
        root,
        button,
        title,
        body,
    }
}

fn disclosure(open: bool) -> Glyph {
    if open {
        Glyph::ChevronDown
    } else {
        Glyph::ChevronRight
    }
}

fn display(show: bool) -> Display {
    if show { Display::Flex } else { Display::None }
}

fn show(nodes: &mut Query<&mut Node>, entity: Entity, visible: bool) {
    if let Ok(mut node) = nodes.get_mut(entity)
        && node.display != display(visible)
    {
        node.display = display(visible);
    }
}

/// Mark a row whose visibility belongs to the inspector. Context-specific
/// visibility belongs to its parent, so search cannot expose inapplicable fields.
#[derive(Component)]
pub(super) struct Property(String);

pub(super) fn searchable(commands: &mut Commands, entity: Entity, keywords: &str) {
    commands
        .entity(entity)
        .insert(Property(normalize(keywords)));
}

fn normalize(text: &str) -> String {
    text.to_lowercase().replace('ё', "е")
}

#[derive(bevy::ecs::system::SystemParam)]
pub(in crate::editor) struct Tree<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    sections: Query<'w, 's, &'static mut Section>,
    properties: Query<'w, 's, &'static Property>,
    parents: Query<'w, 's, &'static ChildOf>,
}

fn filter_children<'a>(
    tree: &'a Tree,
    nodes: &mut Query<&mut Node>,
    parent: Entity,
    terms: &[&str],
    categories: &mut Vec<&'a str>,
    commands: &mut Commands,
) -> bool {
    let Ok(children) = tree.children.get(parent) else {
        return false;
    };
    let mut found = false;
    for entity in children.iter() {
        if let Ok(section) = tree.sections.get(entity) {
            categories.push(section.keywords.as_str());
            let matched = filter_children(tree, nodes, section.body, terms, categories, commands);
            categories.pop();
            let visible = terms.is_empty() || matched;
            let expanded = if terms.is_empty() {
                section.open
            } else {
                matched && !section.folded_during_search
            };
            show(nodes, entity, visible);
            if nodes
                .get(section.body)
                .is_ok_and(|n| n.display != display(expanded))
            {
                show(nodes, section.body, expanded);
                icons::set_glyph(commands, section.chevron, disclosure(expanded));
            }
            found |= visible;
        } else if let Ok(property) = tree.properties.get(entity) {
            let matched = terms.iter().all(|term| {
                property.0.contains(term) || categories.iter().any(|s| s.contains(term))
            });
            show(nodes, entity, matched);
            found |= matched;
        } else if terms.is_empty() || nodes.get(entity).is_ok_and(|n| n.display != Display::None) {
            found |= filter_children(tree, nodes, entity, terms, categories, commands);
        }
    }
    found
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    panel: Res<Panel>,
    mut focus: ResMut<InputFocus>,
    keys: Res<ButtonInput<KeyCode>>,
    mut actions: MessageReader<Action>,
    mut edit: Query<&mut EditableText>,
    mut scroll: Query<&mut ScrollPosition>,
    mut borders: Query<&mut BorderColor>,
    mut tree: Tree,
    mut nodes: Query<&mut Node>,
    mut previous_query: Local<String>,
    mut previous_model: Local<bool>,
    mut previous_icon: Local<Option<Glyph>>,
    mut was_search_focused: Local<bool>,
    mut search_open: Local<bool>,
) {
    let model = editor.viewing_model.is_some();
    let active_scene =
        editor.has_scene() && !editor.asset_viewer_open() && editor.selection.is_some();
    let search_available =
        model && editor.viewing_image.is_none() && !editor.hdri_library_open || active_scene;
    let environment = active_scene && editor.selection == Some(Selection::Environment);
    let can_search = search_available && !environment;
    let glyph = if model {
        Glyph::Box
    } else {
        match editor.selection {
            Some(Selection::Light(_)) => Glyph::Lightbulb,
            Some(Selection::Camera(_)) => Glyph::Camera,
            Some(Selection::Environment) => Glyph::Globe,
            Some(Selection::Node(_)) => Glyph::Layers,
            Some(Selection::Model(_) | Selection::Imported(_)) => Glyph::Box,
            None => Glyph::Cursor,
        }
    };
    if *previous_icon != Some(glyph) {
        icons::set_glyph(&mut commands, panel.icon, glyph);
        *previous_icon = Some(glyph);
    }
    let Ok(mut input) = edit.get_mut(panel.search) else {
        return;
    };
    let mut changed_fold = false;
    let mut clear =
        *previous_model != model || environment && !input.value().to_string().is_empty();
    if !can_search || *previous_model != model {
        *search_open = false;
    }
    for action in actions.read() {
        match *action {
            Action::Inspector(InspectorAction::Fold(entity))
                if search_available && editor.pending.is_none() =>
            {
                if let Ok(mut section) = tree.sections.get_mut(entity) {
                    if input.value().to_string().trim().is_empty() {
                        section.open = !section.open;
                    } else {
                        section.folded_during_search = !section.folded_during_search;
                    }
                    changed_fold = true;
                }
            }
            Action::Inspector(InspectorAction::ClearSearch) => clear = true,
            Action::Inspector(InspectorAction::ToggleSearch)
                if can_search && editor.pending.is_none() =>
            {
                *search_open = !*search_open;
                if *search_open {
                    focus.set(panel.search, bevy::input_focus::FocusCause::Auto);
                } else {
                    clear = true;
                }
            }
            _ => {}
        }
    }
    *previous_model = model;
    let search_focused = focus.get() == Some(panel.search);
    let escape = (search_focused || *was_search_focused) && keys.just_pressed(KeyCode::Escape);
    if escape {
        *search_open = false;
    }
    let search_visible = can_search && *search_open;
    if search_focused && (!search_visible || editor.pending.is_some()) {
        focus.clear();
    }
    *was_search_focused = focus.get() == Some(panel.search);
    clear |= escape;
    if clear {
        input.editor_mut().set_text("");
    }
    let value = input.value().to_string();
    let query = normalize(value.trim());
    show(&mut nodes, panel.search_toggle, can_search);
    show(&mut nodes, panel.bar, search_visible);
    show(&mut nodes, panel.placeholder, value.is_empty());
    show(&mut nodes, panel.clear, !value.is_empty());
    if let Ok(mut border) = borders.get_mut(panel.bar) {
        border.set_if_neq(BorderColor::all(if focus.get() == Some(panel.search) {
            FOCUS
        } else {
            EDGE
        }));
    }
    if *previous_query != query {
        for mut section in &mut tree.sections {
            if section.folded_during_search {
                section.folded_during_search = false;
            }
        }
        if let Ok(mut position) = scroll.get_mut(panel.content) {
            position.y = 0.0;
        }
    }
    let sections_changed = tree.sections.iter_mut().any(|section| section.is_changed());
    if changed_fold || sections_changed || !query.is_empty() || !previous_query.is_empty() {
        let terms: Vec<_> = query.split_whitespace().collect();
        let found = filter_children(
            &tree,
            &mut nodes,
            panel.content,
            &terms,
            &mut Vec::new(),
            &mut commands,
        );
        show(
            &mut nodes,
            panel.empty,
            search_available && !terms.is_empty() && !found,
        );
    }
    if let Some(mut entity) = focus.get().filter(|&entity| entity != panel.search) {
        let mut hidden = !search_available;
        while let Ok(parent) = tree.parents.get(entity) {
            hidden |= nodes
                .get(entity)
                .is_ok_and(|node| node.display == Display::None);
            entity = parent.parent();
            if entity == panel.content {
                if hidden {
                    focus.clear();
                }
                break;
            }
        }
    }
    if *previous_query != query {
        *previous_query = query;
    }
}

pub(super) fn axis_cell(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    axis: usize,
) -> Entity {
    let cell = node(
        commands,
        parent,
        Node {
            flex_basis: px(0),
            flex_grow: 1.0,
            min_width: px(0),
            ..row(4.0)
        },
    );
    text(
        commands,
        cell,
        font,
        ["X", "Y", "Z"][axis],
        11.0,
        AXIS[axis],
    );
    cell
}

pub(super) fn vector_row(commands: &mut Commands, parent: Entity) -> Entity {
    node(
        commands,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            ..row(6.0)
        },
    )
}
