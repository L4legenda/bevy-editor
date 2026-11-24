use super::*;

fn search_field(commands: &mut Commands, parent: Entity, font: &Handle<Font>, state: &State) {
    let field = node(
        commands,
        parent,
        Node {
            flex_grow: 1.,
            flex_basis: px(150),
            min_width: px(0),
            height: px(36),
            ..default()
        },
    );
    let input = input(
        commands,
        field,
        font,
        "Поиск переменных",
        &state.search,
        Field::Search,
        state.focus,
    );
    commands.entity(input).entry::<Node>().and_modify(|mut n| {
        n.padding.left = px(34);
        n.padding.right = px(35);
    });
    let lens = icons::spawn(commands, field, icons::Glyph::Search, 16., MUTED);
    commands.entity(lens).entry::<Node>().and_modify(|mut n| {
        n.position_type = PositionType::Absolute;
        n.left = px(11);
        n.top = px(10);
    });
    let prompt = text(commands, field, font, "Найти переменную", 14., MUTED);
    commands.entity(prompt).insert((
        SearchPart::Placeholder,
        Node {
            position_type: PositionType::Absolute,
            left: px(38),
            right: px(5),
            top: px(9),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
    let clear = node(
        commands,
        field,
        Node {
            position_type: PositionType::Absolute,
            right: px(3),
            top: px(3),
            ..default()
        },
    );
    commands.entity(clear).insert(SearchPart::Clear);
    let button = icons::button(
        commands,
        clear,
        icons::Glyph::Close,
        "Очистить поиск переменных",
        Action::Narrative(NarrativeAction::Conditions(Edit::ClearSearch)),
        Look::Regular,
    );
    commands
        .entity(button)
        .insert(control_hints::Help("Очистить поиск".into()));
}

fn type_style(integer: bool) -> (icons::Glyph, Color, &'static str) {
    if integer {
        (icons::Glyph::Hash, BLUE, "Целое число")
    } else {
        (icons::Glyph::Flag, GREEN, "Флаг")
    }
}

fn result_row(
    commands: &mut Commands,
    list: Entity,
    font: &Handle<Font>,
    id: u64,
    var: &Variable,
    edit: Edit,
) {
    let library = matches!(edit, Edit::Select(_));
    let row_host = node(
        commands,
        list,
        Node {
            width: percent(100),
            min_width: px(0),
            flex_shrink: 0.,
            ..column(0.)
        },
    );
    commands.entity(row_host).insert(SearchResult(id));
    let item = control(
        commands,
        row_host,
        Action::Narrative(NarrativeAction::Conditions(edit)),
        Look::Regular,
        Node {
            width: percent(100),
            min_height: px(if library { 53 } else { 43 }),
            padding: UiRect::axes(px(11), px(if library { 6 } else { 3 })),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(10.)
        },
    );
    let value = var.default.label();
    commands.entity(item).insert(AccessibleLabel(if library {
        format!("{} · {value}", var.name)
    } else {
        var.name.clone()
    }));
    let mark = surface(
        commands,
        item,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            right: px(0),
            bottom: px(0),
            border_radius: BorderRadius::all(px(5)),
            ..default()
        },
        HOVER.with_alpha(0.72),
    );
    commands
        .entity(mark)
        .insert((SearchPart::Cursor(id), Pickable::IGNORE));

    let indicator = surface(
        commands,
        mark,
        Node {
            width: px(2),
            height: px(20),
            margin: UiRect::left(px(3)),
            align_self: AlignSelf::Center,
            border_radius: BorderRadius::all(px(1)),
            ..default()
        },
        FOCUS,
    );
    commands.entity(indicator).insert(Pickable::IGNORE);
    let (glyph, tint, kind) = type_style(matches!(var.default, Value::Number(_)));
    let side = if library { 31 } else { 27 };
    let icon = surface(
        commands,
        item,
        Node {
            width: px(side),
            height: px(side),
            flex_shrink: 0.,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        tint.with_alpha(0.12),
    );
    icons::spawn(commands, icon, glyph, 16., tint);
    let name = node(
        commands,
        item,
        Node {
            flex_grow: 1.,
            flex_basis: px(0),
            min_width: px(0),
            ..column(2.)
        },
    );
    let title = text(
        commands,
        name,
        font,
        &var.name,
        if library { 15. } else { 14. },
        TEXT,
    );
    commands.entity(title).insert(TextLayout::linebreak(
        bevy::text::LineBreak::WordOrCharacter,
    ));
    text(commands, name, font, kind, 12., MUTED);
    let trailing = node(
        commands,
        item,
        Node {
            flex_shrink: 0.,
            ..row(0.)
        },
    );
    text(commands, trailing, font, value, 14., TEXT);
    if library {
        icons::spawn(commands, item, icons::Glyph::ChevronRight, 14., MUTED);
    }
}

pub(super) fn search_list(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    state: &State,
    library: bool,
) {
    let toolbar = node(
        commands,
        parent,
        Node {
            width: percent(100),
            ..row(9.)
        },
    );
    search_field(commands, toolbar, font, state);
    if library {
        let add = control(
            commands,
            toolbar,
            Action::Narrative(NarrativeAction::Conditions(Edit::New)),
            Look::Primary,
            Node {
                height: px(36),
                flex_shrink: 0.,
                padding: UiRect::horizontal(px(12)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..row(6.)
            },
        );
        commands
            .entity(add)
            .insert(AccessibleLabel("Создать переменную".into()));
        icons::themed(commands, add, icons::Glyph::Plus, 15.);
        text(commands, add, font, "Создать", 14., ON_ACCENT);
    }
    let group = surface(
        commands,
        parent,
        Node {
            min_height: px(0),
            width: percent(100),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            overflow: Overflow::clip(),
            ..column(0.)
        },
        CHROME.with_alpha(0.8),
    );
    if library {
        let headings = surface(
            commands,
            group,
            Node {
                min_height: px(30),
                flex_shrink: 0.,
                padding: UiRect {
                    left: px(15),
                    right: px(40),
                    ..default()
                },
                border: UiRect::bottom(px(1)),
                justify_content: JustifyContent::SpaceBetween,
                ..row(6.)
            },
            WELL.with_alpha(0.65),
        );
        commands.entity(headings).insert(SearchPart::Headings);
        text(commands, headings, font, "Переменная", 12., MUTED);
        text(commands, headings, font, "По умолчанию", 12., MUTED);
    }
    let list = scroll(commands, group);
    commands
        .entity(list)
        .entry::<Node>()
        .and_modify(move |mut n| {
            n.row_gap = px(1);
            n.padding = UiRect::all(px(3));
            n.max_height = px(if library { 290 } else { 188 });
        });
    commands.entity(list).insert(ResultList::default());
    for id in state.matching_variables("") {
        let var = &state.catalog.variables[&id];
        let action = if library {
            Edit::Select(id)
        } else {
            state
                .chooser
                .flatten()
                .map_or(Edit::Add(id), |i| Edit::Variable(i, id))
        };
        result_row(commands, list, font, id, var, action);
    }
    let empty = node(
        commands,
        list,
        Node {
            min_height: px(57),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..row(8.)
        },
    );
    commands.entity(empty).insert(SearchPart::Empty);
    text(commands, empty, font, "Переменные не найдены", 13., MUTED);
}

fn type_caption(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    integer: bool,
) -> &'static str {
    let (glyph, tint, name) = type_style(integer);
    let name = if integer {
        name
    } else {
        "Флаг (да / нет)"
    };
    icons::spawn(commands, parent, glyph, 16., tint);
    text(commands, parent, font, name, 14., TEXT);
    name
}

fn type_picker(commands: &mut Commands, parent: Entity, font: &Handle<Font>, integer: bool) {
    use bevy::ui_widgets::popover::{Popover, PopoverPlacement, PopoverSide};

    let anchor = surface(
        commands,
        parent,
        Node {
            height: px(36),
            width: percent(100),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            ..column(0.)
        },
        WELL,
    );
    let trigger = control(
        commands,
        anchor,
        Action::Narrative(NarrativeAction::Conditions(Edit::ToggleTypeMenu)),
        Look::Regular,
        Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::horizontal(px(9)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(8.)
        },
    );
    let current = type_caption(commands, trigger, font, integer);
    commands
        .entity(trigger)
        .insert(AccessibleLabel("Тип переменной".into()))
        .entry::<bevy::a11y::AccessibilityNode>()
        .and_modify(move |mut node| {
            node.set_value(current);
            node.set_expanded(false);
        });
    let chevron = icons::spawn(commands, trigger, icons::Glyph::ChevronDown, 14., MUTED);
    commands
        .entity(chevron)
        .entry::<Node>()
        .and_modify(|mut n| n.margin.left = Val::Auto);

    let popup = surface(
        commands,
        anchor,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            padding: UiRect::all(px(4)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..column(2.)
        },
        PANEL,
    );
    commands.entity(popup).insert((
        AccessibleLabel("Список типов переменной".into()),
        GlobalZIndex(45),
        bevy::ui::OverrideClip,
        BoxShadow::new(Color::BLACK.with_alpha(0.4), px(0), px(4), px(0), px(12)),
        Popover {
            positions: [PopoverSide::Bottom, PopoverSide::Top]
                .map(|side| PopoverPlacement {
                    side,
                    gap: 4.,
                    ..default()
                })
                .into(),
            window_margin: 8.,
        },
    ));
    let options = [false, true].map(|number| {
        let item = control(
            commands,
            popup,
            Action::Narrative(NarrativeAction::Conditions(Edit::Type(number))),
            Look::Regular,
            Node {
                width: percent(100),
                min_height: px(32),
                padding: UiRect::horizontal(px(8)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                ..row(8.)
            },
        );
        let name = type_caption(commands, item, font, number);
        commands
            .entity(item)
            .insert(AccessibleLabel(name.into()))
            .entry::<bevy::a11y::AccessibilityNode>()
            .and_modify(move |mut node| node.set_selected(integer == number));
        if integer == number {
            let check = icons::spawn(commands, item, icons::Glyph::Check, 15., FOCUS);
            commands
                .entity(check)
                .entry::<Node>()
                .and_modify(|mut n| n.margin.left = Val::Auto);
        }
        item
    });
    commands.entity(popup).insert(TypeMenu {
        trigger,
        chevron,
        options,
    });
}

fn segment(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    edit: Edit,
    selected: bool,
) {
    let tile = surface(
        commands,
        parent,
        Node {
            flex_grow: 1.,
            flex_basis: px(0),
            min_width: px(0),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..column(0.)
        },
        if selected { HOVER } else { Color::NONE },
    );
    commands.entity(tile).insert(BorderColor::all(if selected {
        FOCUS.with_alpha(0.42)
    } else {
        Color::NONE
    }));
    let button = control(
        commands,
        tile,
        Action::Narrative(NarrativeAction::Conditions(edit)),
        Look::Regular,
        Node {
            width: percent(100),
            min_height: px(30),
            padding: UiRect::horizontal(px(5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            justify_content: JustifyContent::Center,
            ..row(6.)
        },
    );
    commands
        .entity(button)
        .insert(AccessibleLabel(label.into()));
    text(commands, button, font, label, 13., TEXT);
}

pub(super) fn form(commands: &mut Commands, parent: Entity, font: &Handle<Font>, state: &State) {
    let Some(draft) = &state.variable else { return };
    let fields = node(commands, parent, column(14.));
    let name = node(commands, fields, column(5.));
    text(commands, name, font, "Название", 12., MUTED);
    let name_field = node(commands, name, column(0.));
    input(
        commands,
        name_field,
        font,
        "Название переменной",
        &draft.name,
        Field::Name,
        state.focus,
    );
    if draft.id.is_none() {
        let example = text(commands, name_field, font, "", 13., MUTED);
        commands.entity(example).insert((
            StatusLabel::VariableExample,
            Node {
                position_type: PositionType::Absolute,
                left: px(14),
                top: px(10),
                ..default()
            },
        ));
    }
    variable_error(commands, name, font, draft, Field::Name);
    let type_field = node(commands, fields, column(5.));
    text(commands, type_field, font, "Тип переменной", 12., MUTED);
    type_picker(commands, type_field, font, draft.integer);
    let value = node(commands, fields, column(5.));
    text(commands, value, font, "Значение по умолчанию", 12., MUTED);
    if draft.integer {
        let field = input(
            commands,
            value,
            font,
            "Значение по умолчанию",
            &draft.number,
            Field::Default,
            state.focus,
        );
        commands
            .entity(field)
            .insert(bevy::ui_widgets::SelectAllOnFocus);
        variable_error(commands, value, font, draft, Field::Default);
    } else {
        let choices = group(commands, value);
        commands
            .entity(choices)
            .entry::<Node>()
            .and_modify(|mut n| n.width = px(150));
        for (flag, label) in [(false, "Нет"), (true, "Да")] {
            segment(
                commands,
                choices,
                font,
                label,
                Edit::Flag(flag),
                flag == draft.flag,
            );
        }
    }
    let footer = node(
        commands,
        parent,
        Node {
            width: percent(100),
            border: UiRect::top(px(1)),
            padding: UiRect::top(px(12)),
            justify_content: JustifyContent::FlexEnd,
            ..wrap()
        },
    );
    commands.entity(footer).insert(BorderColor::all(EDGE));
    if draft.id.is_some() {
        let remove = icons::button(
            commands,
            footer,
            icons::Glyph::Trash,
            "Удалить переменную",
            Action::Narrative(NarrativeAction::Conditions(Edit::RemoveVariable)),
            Look::Regular,
        );
        commands
            .entity(remove)
            .insert(control_hints::Help("Удалить переменную".into()));
        commands
            .entity(remove)
            .entry::<Node>()
            .and_modify(|mut n| n.margin.right = Val::Auto);
    }
    button(
        commands,
        footer,
        font,
        "Отмена",
        if state.nested {
            Edit::CancelNested
        } else {
            Edit::Back
        },
    );
    let label = if state.nested {
        "Создать и выбрать"
    } else if draft.id.is_some() {
        "Сохранить"
    } else {
        "Создать"
    };
    let submit = act(
        commands,
        footer,
        font,
        label,
        NarrativeAction::Conditions(Edit::SaveVariable),
        true,
    );
    if draft.id.is_some() {
        commands
            .entity(submit)
            .insert(AccessibleLabel("Сохранить переменную".into()));
    }
}
