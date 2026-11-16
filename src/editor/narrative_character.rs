//! A single, scrollable character sheet with editing beside each portrait.
use super::*;

#[derive(Component)]
pub(super) struct Stage(pub usize);

const PORTRAIT: f32 = 184.0;

fn panel(commands: &mut Commands, parent: Entity, layout: Node) -> Entity {
    let e = surface(commands, parent, layout, PANEL.with_alpha(0.94));
    commands.entity(e).insert(BorderColor::all(EDGE));
    e
}

pub(super) fn spawn(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    editor: &Editor,
    doc: &Document,
    character: &Character,
    server: &AssetServer,
    scroll: Vec2,
) {
    let stage = node(
        commands,
        parent,
        Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::all(px(20)),
            overflow: Overflow::scroll_y(),
            align_items: AlignItems::Center,
            ..column(0.0)
        },
    );
    commands.entity(stage).insert((
        Stage(doc.id),
        Scroller,
        ScrollPosition(scroll),
        AccessibleLabel("Досье персонажа".into()),
    ));
    let page = node(
        commands,
        stage,
        Node {
            width: percent(100),
            max_width: px(1080),
            flex_shrink: 0.0,
            ..column(16.0)
        },
    );
    text(commands, page, font, "ДОСЬЕ ПЕРСОНАЖА", 11.0, FOCUS);
    let profile = panel(
        commands,
        page,
        Node {
            width: percent(100),
            padding: UiRect::all(px(20)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(16)),
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Start,
            row_gap: px(20),
            ..row(24.0)
        },
    );
    let main = node(
        commands,
        profile,
        Node {
            width: px(PORTRAIT),
            flex_shrink: 0.0,
            align_items: AlignItems::Center,
            ..column(12.0)
        },
    );
    if let Some(p) = character.portraits.first() {
        portrait(commands, main, server, &editor.root, &p.path, PORTRAIT);
    } else {
        let placeholder = panel(
            commands,
            main,
            Node {
                width: px(PORTRAIT),
                height: px(PORTRAIT),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(12)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::all(px(18)),
                ..column(12.0)
            },
        );
        icons::spawn(commands, placeholder, icons::Glyph::Image, 42.0, MUTED);
        wrapped(
            commands,
            placeholder,
            font,
            "Портрет ещё не выбран",
            12.0,
            MUTED,
        );
    }
    let badge = node(commands, main, row(5.0));
    icons::spawn(commands, badge, icons::Glyph::Star, 12.0, FOCUS);
    text(commands, badge, font, "ОСНОВНОЙ ПОРТРЕТ", 10.0, FOCUS);
    let form = node(
        commands,
        profile,
        Node {
            flex_grow: 1.0,
            flex_basis: px(380),
            min_width: px(260),
            ..column(9.0)
        },
    );
    let name = field(
        commands,
        form,
        font,
        "Имя персонажа",
        &character.name,
        DraftField::Title,
        false,
    );
    commands.entity(name).insert(super::font(font, 24.0));
    commands.entity(name).entry::<Node>().and_modify(|mut n| {
        n.height = px(48);
    });
    let description = field(
        commands,
        form,
        font,
        "Описание персонажа",
        &character.description,
        DraftField::Description,
        true,
    );
    commands
        .entity(description)
        .entry::<Node>()
        .and_modify(|mut n| {
            n.height = px(92);
        });
    wrapped(
        commands,
        form,
        font,
        "Роль, характер и детали истории — всё, что поможет писать реплики.",
        11.0,
        MUTED,
    );

    let section = node(
        commands,
        page,
        Node {
            width: percent(100),
            ..column(8.0)
        },
    );
    let heading = node(
        commands,
        section,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(10),
            ..row(12.0)
        },
    );
    text(commands, heading, font, "Портреты и эмоции", 20.0, TEXT);
    text(
        commands,
        heading,
        font,
        format!("{}", character.portraits.len()),
        12.0,
        MUTED,
    );
    node(
        commands,
        heading,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    if !character.portraits.is_empty() {
        act(
            commands,
            heading,
            font,
            "+ Добавить портрет",
            NarrativeAction::AddPortrait,
            true,
        );
    }
    wrapped(
        commands,
        section,
        font,
        "Назовите выражения, например «Спокойствие» и «Удивление». В каждой реплике можно выбрать нужный портрет.",
        12.0,
        MUTED,
    );
    let gallery = node(
        commands,
        page,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(16),
            align_items: AlignItems::Start,
            ..row(16.0)
        },
    );
    if character.portraits.is_empty() {
        let empty = panel(
            commands,
            gallery,
            Node {
                width: percent(100),
                padding: UiRect::all(px(26)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(14)),
                align_items: AlignItems::Center,
                ..column(12.0)
            },
        );
        text(commands, empty, font, "Добавьте первый портрет", 17.0, TEXT);
        wrapped(
            commands,
            empty,
            font,
            "Выберите изображение из ассетов проекта. Оно станет основным портретом персонажа.",
            12.0,
            MUTED,
        );
        act(
            commands,
            empty,
            font,
            "+ Добавить портрет",
            NarrativeAction::AddPortrait,
            true,
        );
    }
    for (i, p) in character.portraits.iter().enumerate() {
        let card = panel(
            commands,
            gallery,
            Node {
                width: px(212),
                padding: UiRect::all(px(13)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(14)),
                ..column(10.0)
            },
        );
        commands.entity(card).insert((
            AccessibleLabel(format!("Портрет {}", i + 1)),
            BorderColor::all(if i == 0 { FOCUS.with_alpha(0.55) } else { EDGE }),
        ));
        let label = node(
            commands,
            card,
            Node {
                width: percent(100),
                ..row(5.0)
            },
        );
        if i == 0 {
            icons::spawn(commands, label, icons::Glyph::Star, 12.0, FOCUS);
        }
        text(
            commands,
            label,
            font,
            if i == 0 {
                "ОСНОВНОЙ"
            } else {
                "ВАРИАНТ ПОРТРЕТА"
            },
            10.0,
            if i == 0 { FOCUS } else { MUTED },
        );
        let image_row = node(
            commands,
            card,
            Node {
                width: percent(100),
                justify_content: JustifyContent::Center,
                ..row(0.0)
            },
        );
        portrait(commands, image_row, server, &editor.root, &p.path, 160.0);
        let input = field(
            commands,
            card,
            font,
            "Название / эмоция",
            &p.label,
            DraftField::PortraitLabel(i),
            false,
        );
        commands
            .entity(input)
            .insert(AccessibleLabel(format!("Название портрета {}", i + 1)));
        let actions = node(
            commands,
            card,
            Node {
                width: percent(100),
                ..row(5.0)
            },
        );
        if i > 0 {
            let primary = act(
                commands,
                actions,
                font,
                "Сделать основным",
                NarrativeAction::MainPortrait(i),
                false,
            );
            commands.entity(primary).insert(AccessibleLabel(format!(
                "Сделать портрет {} основным",
                i + 1
            )));
        } else {
            node(
                commands,
                actions,
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            );
        }
        icons::button(
            commands,
            actions,
            icons::Glyph::Trash,
            &format!("Убрать портрет {} из персонажа", i + 1),
            Action::Narrative(NarrativeAction::RemovePortrait(i)),
            Look::Regular,
        );
    }
}
