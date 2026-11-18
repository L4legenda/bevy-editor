//! One compact picker for a line's character and its expression.
use super::*;
const ROW: f32 = 52.;
const PORTRAIT: f32 = 56.;
#[derive(Component)]
pub(super) struct Anchor(pub u32);
struct Entry {
    index: usize,
    name: String,
    path: String,
    portrait: Option<String>,
    haystack: String,
    current: bool,
    duplicate: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
    document: usize,
    node: u32,
    generation: u64,
}
#[derive(Component)]
pub(super) struct ChoiceRow(pub usize);
#[derive(Resource)]
pub(super) struct Menu {
    root: Entity,
    panel: Entity,
    search: Option<Entity>,
    placeholder: Option<Entity>,
    list: Option<Entity>,
    context: Option<Context>,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    cursor: usize,
    revision: u64,
    query: String,
    repaint: bool,
    scroll_to_cursor: bool,
    keyboard: bool,
    pointer: Option<Vec2>,
    opened: f64,
    down: bool,
    base: f32,
    before: Option<(Option<String>, usize)>,
}
fn normalized(value: &str) -> String {
    value.to_lowercase().replace('ё', "е")
}
fn signature(doc: &Document, id: u32) -> Option<(Option<String>, usize)> {
    let Content::Dialogue(d) = &doc.content else {
        return None;
    };
    d.node(id).map(|n| (n.speaker.clone(), n.portrait))
}

pub(super) struct Selection {
    pub changed: bool,
    pub portraits: bool,
}
pub(super) fn select(
    editor: &mut Editor,
    browser: &Browser,
    index: Option<usize>,
) -> Option<Selection> {
    let (path, portraits) = if let Some(index) = index {
        let file = browser
            .files
            .get(index)
            .filter(|f| f.extension.eq_ignore_ascii_case("dcharacter"))?;
        let path = file.path.to_string_lossy().replace('\\', "/");
        let portraits = character(editor, &path)?.portraits.len() > 1;
        (Some(path), portraits)
    } else {
        (None, false)
    };
    let doc = editor.narrative.current()?;
    let Content::Dialogue(d) = &doc.content else {
        return None;
    };
    let n = d.node(doc.selected).filter(|n| n.kind == NodeKind::Line)?;
    let changed = n.speaker != path;
    if changed {
        let doc = editor.narrative.current_mut()?;
        doc.checkpoint();
        if let Content::Dialogue(d) = &mut doc.content {
            let n = d.node_mut(doc.selected)?;
            n.speaker = path;
            n.portrait = 0;
        }
    }
    Some(Selection { changed, portraits })
}
pub(super) fn setup(commands: &mut Commands, parent: Entity) {
    let root = node(
        commands,
        parent,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
    );
    commands.entity(root).insert((
        ZIndex(40),
        TabGroup::modal(),
        AccessibleLabel("Выбор говорящего".into()),
    ));
    commands
        .spawn((
            Button,
            TabIndex(-1),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Color::NONE),
            ChildOf(root),
        ))
        .observe(|_: On<Activate>, mut actions: MessageWriter<Action>| {
            actions.write(Action::Narrative(NarrativeAction::Dismiss));
        });
    let panel = surface(
        commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::all(px(10)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            overflow: Overflow::clip(),
            ..column(6.)
        },
        PANEL,
    );
    commands.entity(panel).insert((
        ZIndex(1),
        BorderColor::all(FOCUS.with_alpha(0.32)),
        AccessibleLabel("Список персонажей".into()),
        BackgroundGradient::default(),
    ));
    commands.insert_resource(Menu {
        root,
        panel,
        search: None,
        placeholder: None,
        list: None,
        context: None,
        entries: vec![],
        filtered: vec![],
        cursor: 0,
        revision: 0,
        query: String::new(),
        repaint: false,
        scroll_to_cursor: false,
        keyboard: false,
        pointer: None,
        opened: 0.,
        down: true,
        base: 80.,
        before: None,
    });
}
fn entries(editor: &Editor, browser: &Browser, selected: Option<&str>) -> Vec<Entry> {
    let mut result = Vec::new();
    for (index, file) in browser
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.extension.eq_ignore_ascii_case("dcharacter"))
    {
        let path = file.path.to_string_lossy().replace('\\', "/");
        let Some(c) = character(editor, &path) else {
            continue;
        };
        let name = if c.name.trim().is_empty() {
            file.path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            c.name
        };
        result.push(Entry {
            index,
            haystack: normalized(&format!("{name} {path}")),
            name,
            path: path.clone(),
            portrait: c.portraits.first().map(|p| p.path.clone()),
            current: selected == Some(path.as_str()),
            duplicate: false,
        });
    }
    for i in 0..result.len() {
        result[i].duplicate = result
            .iter()
            .filter(|e| normalized(&e.name) == normalized(&result[i].name))
            .count()
            > 1;
    }
    result.sort_by_cached_key(|e| (!e.current, normalized(&e.name), normalized(&e.path)));
    result
}
fn dialogue_entries(browser: &Browser, selected: Option<&str>) -> Vec<Entry> {
    let mut result: Vec<_> = browser
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| is_dialogue_file(file))
        .map(|(index, file)| {
            let path = file.path.to_string_lossy().replace('\\', "/");
            let name = file
                .path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Entry {
                index,
                haystack: normalized(&format!("{name} {path}")),
                name,
                current: selected.is_some_and(|selected| same_asset_path(selected, &path)),
                path,
                portrait: None,
                duplicate: true,
            }
        })
        .collect();
    result.sort_by_cached_key(|e| (!e.current, normalized(&e.name), normalized(&e.path)));
    result
}

fn choose(picker: Option<Picker>, index: usize, revision: u64) -> NarrativeAction {
    if picker == Some(Picker::Dialogue) {
        NarrativeAction::SetDialogue(Some(index), revision)
    } else {
        NarrativeAction::SetSpeaker(Some(index))
    }
}

impl Menu {
    fn filter(&mut self, query: &str) {
        self.query = query.to_owned();
        let query = normalized(query);
        let terms: Vec<_> = query.split_whitespace().collect();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| terms.iter().all(|t| e.haystack.contains(t)))
            .map(|(i, _)| i)
            .collect();
        self.cursor = 0;
        self.repaint = true;
        self.scroll_to_cursor = true;
    }
    pub(super) fn panel(&self) -> Entity {
        self.panel
    }
    pub(super) fn visible(&self) -> bool {
        self.context.is_some()
    }
    pub(super) fn opens_down(&self) -> bool {
        self.down
    }
    pub(super) fn keyboard_row(&self, row: usize) -> bool {
        self.keyboard && self.cursor == row
    }
    pub(super) fn progress(&self, now: f64) -> f32 {
        if self.context.is_none() {
            0.
        } else {
            let t = ((now - self.opened) / 0.20).clamp(0., 1.) as f32;
            1. - (1. - t).powi(3)
        }
    }
}
pub(super) fn input(
    ui: Res<NarrativeUi>,
    mut menu: ResMut<Menu>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
    window: Single<&Window, With<PrimaryWindow>>,
    rows: Query<(&ChoiceRow, &Hovered)>,
) {
    if !matches!(
        ui.picker,
        Some(Picker::Speaker | Picker::Portrait | Picker::Dialogue)
    ) || menu.context.is_none()
    {
        return;
    }
    if !window.focused {
        actions.write(Action::Narrative(NarrativeAction::Dismiss));
        return;
    }
    if menu.query != ui.search {
        menu.filter(&ui.search);
        menu.keyboard = false;
    }
    let pointer = window.cursor_position();
    if pointer != menu.pointer {
        menu.pointer = pointer;
        menu.keyboard = false;
        if let Some((row, _)) = rows.iter().find(|(_, h)| h.0) {
            menu.cursor = row.0;
        }
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::ArrowUp) {
        menu.cursor = if keys.just_pressed(KeyCode::ArrowDown) {
            (menu.cursor + 1).min(menu.filtered.len().saturating_sub(1))
        } else {
            menu.cursor.saturating_sub(1)
        };
        menu.keyboard = true;
        menu.scroll_to_cursor = true;
        if let Some(search) = menu.search {
            focus.set(search, bevy::input_focus::FocusCause::Auto);
        }
    }
    if keys.just_pressed(KeyCode::Enter) && focus.get() == menu.search {
        if let Some(e) = menu
            .filtered
            .get(menu.cursor)
            .and_then(|i| menu.entries.get(*i))
        {
            actions.write(Action::Narrative(choose(ui.picker, e.index, menu.revision)));
        }
    }
}
fn restore_focus(commands: &mut Commands, context: Context) {
    commands.queue(move |world: &mut World| {
        let ui = world.resource::<NarrativeUi>();
        if ui.picker.is_some()
            || ui.preview
            || world.resource::<Editor>().pending.is_some()
            || world.resource::<Editor>().narrative.active != Some(context.document)
        {
            return;
        }
        let anchor = world
            .query::<(Entity, &Anchor)>()
            .iter(world)
            .find(|(_, a)| a.0 == context.node)
            .map(|(e, _)| e);
        if let Some(anchor) = anchor {
            world
                .resource_mut::<InputFocus>()
                .set(anchor, bevy::input_focus::FocusCause::Auto);
        }
    });
}
fn expressions(
    commands: &mut Commands,
    panel: Entity,
    font: &Handle<Font>,
    editor: &Editor,
    doc: &Document,
    browser: &Browser,
    server: &AssetServer,
    width: f32,
    choosing: bool,
) -> f32 {
    let Content::Dialogue(d) = &doc.content else {
        return 0.;
    };
    let Some(n) = d.node(doc.selected) else {
        return 0.;
    };
    let Some(path) = n.speaker.as_deref() else {
        return 0.;
    };
    let Some(c) = character(editor, path) else {
        return 0.;
    };
    if c.portraits.is_empty() {
        if let Some(index) = browser
            .files
            .iter()
            .position(|f| f.path.to_string_lossy().replace('\\', "/") == path)
        {
            let link = act(
                commands,
                panel,
                font,
                "Добавить портрет",
                NarrativeAction::File(index),
                false,
            );
            commands
                .entity(link)
                .insert(AccessibleLabel("Открыть карточку персонажа".into()));
            return 28.;
        }
        return 0.;
    }
    if c.portraits.len() == 1 {
        return 0.;
    }
    let section = node(
        commands,
        panel,
        Node {
            width: percent(100),
            flex_shrink: 0.,
            ..column(5.)
        },
    );
    text(
        commands,
        section,
        font,
        if choosing {
            "ВЫБЕРИТЕ ПОРТРЕТ"
        } else {
            "ПОРТРЕТ"
        },
        10.,
        if choosing { FOCUS } else { MUTED },
    );
    let strip = node(
        commands,
        section,
        Node {
            width: percent(100),
            max_height: px(120),
            overflow: Overflow::scroll_y(),
            flex_shrink: 0.,
            flex_wrap: FlexWrap::Wrap,
            row_gap: px(6),
            align_content: AlignContent::FlexStart,
            ..row(6.)
        },
    );
    commands.entity(strip).insert((
        Scroller,
        ScrollPosition::default(),
        AccessibleLabel("Портреты персонажа".into()),
    ));
    let current = if n.portrait < c.portraits.len() {
        n.portrait
    } else {
        0
    };
    for (i, p) in c.portraits.iter().enumerate() {
        let tile = control(
            commands,
            strip,
            Action::Narrative(NarrativeAction::SetPortrait(i)),
            Look::Regular,
            Node {
                width: px(PORTRAIT),
                height: px(PORTRAIT),
                flex_shrink: 0.,
                padding: UiRect::all(px(2)),
                border_radius: BorderRadius::all(px(7)),
                justify_content: JustifyContent::Center,
                ..row(0.)
            },
        );
        commands.entity(tile).insert((
            AccessibleLabel(format!("Выбрать портрет {}: {}", i + 1, p.label)),
            super::super::control_hints::Help(p.label.clone()),
        ));
        portrait(commands, tile, server, &editor.root, &p.path, 50.);
        picker_fx::layer(commands, tile, 1, i == current, None, n.id);
    }
    let cols = ((width - 14.) / (PORTRAIT + 6.)).floor().max(1.) as usize;
    let rows = c.portraits.len().div_ceil(cols).min(2);
    17. + rows as f32 * PORTRAIT + rows.saturating_sub(1) as f32 * 6.
}
pub(super) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    browser: Res<Browser>,
    mut ui: ResMut<NarrativeUi>,
    mut menu: ResMut<Menu>,
    window: Single<&Window, With<PrimaryWindow>>,
    anchors: Query<(Entity, &Anchor)>,
    geometry: Query<(&ComputedNode, &UiGlobalTransform)>,
    mut nodes: Query<&mut Node>,
    mut scrolls: Query<&mut ScrollPosition>,
    server: Res<AssetServer>,
    time: Res<Time>,
    mut fx: ResMut<picker_fx::State>,
    drawer: Res<DrawerMotion>,
) {
    let dialogue = ui.picker == Some(Picker::Dialogue);
    let doc = editor.narrative.current();
    let active = matches!(
        ui.picker,
        Some(Picker::Speaker | Picker::Portrait | Picker::Dialogue)
    ) && editor.pending.is_none();
    let context = doc.filter(|_| active).and_then(|doc| match &doc.content {
        Content::Dialogue(d)
            if d.node(doc.selected).is_some_and(|n| {
                n.kind
                    == if dialogue {
                        NodeKind::Jump
                    } else {
                        NodeKind::Line
                    }
            }) =>
        {
            Some(Context {
                document: doc.id,
                node: doc.selected,
                generation: ui.generation,
            })
        }
        _ => None,
    });
    let Some(context) = context else {
        if let Ok(mut n) = nodes.get_mut(menu.root) {
            n.display = Display::None;
        }
        if let Some(previous) = menu.context.take() {
            if let Some(doc) = doc.filter(|doc| doc.id == previous.document) {
                if signature(doc, previous.node) != menu.before {
                    fx.confirmation = Some((doc.id, previous.node, time.elapsed_secs_f64()));
                }
            }
            restore_focus(&mut commands, previous);
        }
        return;
    };
    let doc = doc.unwrap();
    let selected = match &doc.content {
        Content::Dialogue(d) => d.node(context.node).and_then(|n| {
            if dialogue {
                n.dialogue.as_deref()
            } else {
                n.speaker.as_deref()
            }
        }),
        _ => None,
    };
    let Some((computed, transform)) = anchors
        .iter()
        .find(|(_, a)| a.0 == context.node)
        .and_then(|(e, _)| geometry.get(e).ok())
    else {
        return;
    };
    let Ok((root_size, root_transform)) = geometry.get(ui.root) else {
        return;
    };
    if computed.is_empty() || root_size.is_empty() {
        return;
    }
    let scale = window.scale_factor();
    let root_min = (root_transform.translation - root_size.size() * 0.5) / scale;
    let root_size = root_size.size() / scale;
    let affine = transform.affine();
    let visual_size = computed.size()
        * Vec2::new(
            affine.matrix2.x_axis.length(),
            affine.matrix2.y_axis.length(),
        )
        / scale;
    let anchor = Rect::from_center_size(transform.translation / scale - root_min, visual_size);
    let width = visual_size
        .x
        .max(360.)
        .min(420.)
        .min((root_size.x - 16.).max(100.));
    let opened =
        menu.context != Some(context) || (dialogue && menu.revision != browser.scan_revision);
    let continuing = menu
        .context
        .is_some_and(|old| old.document == context.document && old.node == context.node);
    let new_session = opened && !continuing;
    if opened {
        commands
            .entity(menu.root)
            .insert(AccessibleLabel(if dialogue {
                "Выбор диалога".into()
            } else {
                "Выбор говорящего".into()
            }));
        commands
            .entity(menu.panel)
            .insert(AccessibleLabel(if dialogue {
                "Список диалогов".into()
            } else {
                "Список персонажей".into()
            }));
        menu.context = Some(context);
        if !continuing {
            menu.opened = time.elapsed_secs_f64();
            menu.before = signature(doc, context.node);
        }
        menu.keyboard = false;
        menu.pointer = window.cursor_position();
        commands.entity(menu.panel).despawn_related::<Children>();
        let search_row = node(
            &mut commands,
            menu.panel,
            Node {
                width: percent(100),
                height: px(36),
                flex_shrink: 0.,
                ..row(6.)
            },
        );
        let search = input_field(
            &mut commands,
            search_row,
            &ui.font,
            if dialogue {
                "Поиск диалога"
            } else {
                "Поиск персонажа"
            },
            &ui.search,
            DraftField::Search,
            false,
        );
        if dialogue && selected.is_some() {
            commands.entity(search).entry::<Node>().and_modify(|mut n| {
                n.width = px(0);
                n.flex_grow = 1.;
                n.min_width = px(0);
            });
            act(
                &mut commands,
                search_row,
                &ui.font,
                "Сбросить",
                NarrativeAction::SetDialogue(None, browser.scan_revision),
                false,
            );
        }
        menu.search = Some(search);
        let placeholder = text(
            &mut commands,
            search_row,
            &ui.font,
            if dialogue {
                "Найти диалог…"
            } else {
                "Найти персонажа…"
            },
            13.,
            MUTED,
        );
        commands.entity(placeholder).insert((
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(11),
                top: px(9),
                ..default()
            },
        ));
        menu.placeholder = Some(placeholder);
        let extra = if dialogue {
            0.
        } else {
            expressions(
                &mut commands,
                menu.panel,
                &ui.font,
                &editor,
                doc,
                &browser,
                &server,
                width,
                ui.picker == Some(Picker::Portrait),
            )
        };
        menu.base = 80. + if extra > 0. { extra + 6. } else { 0. };
        let list = node(
            &mut commands,
            menu.panel,
            Node {
                width: percent(100),
                flex_grow: 1.,
                min_height: px(0),
                overflow: Overflow::scroll_y(),
                ..column(2.)
            },
        );
        commands
            .entity(list)
            .insert((Scroller, ScrollPosition::default()));
        menu.list = Some(list);
        text(
            &mut commands,
            menu.panel,
            &ui.font,
            "↑ ↓  выбрать  ·  Enter  применить  ·  Esc  закрыть",
            10.,
            MUTED,
        );
        picker_fx::layer(&mut commands, menu.panel, 0, false, None, context.node);
        if !dialogue {
            ui.search.clear();
        }
        let portrait = if ui.picker == Some(Picker::Portrait) {
            match &doc.content {
                Content::Dialogue(d) => d.node(context.node).map(|n| n.portrait),
                _ => None,
            }
        } else {
            None
        };
        let panel = menu.panel;
        commands.queue(move |world: &mut World| {
            let mut target = search;
            if let Some(portrait) = portrait {
                let mut pending = vec![panel];
                while let Some(entity) = pending.pop() {
                    if world.get::<Control>(entity).is_some_and(|c| matches!(c.action, Action::Narrative(NarrativeAction::SetPortrait(i)) if i == portrait)) {
                        target = entity;
                        break;
                    }
                    if let Some(children) = world.get::<Children>(entity) { pending.extend(children.iter()); }
                }
            }
            world
                .resource_mut::<InputFocus>()
                .set(target, bevy::input_focus::FocusCause::Auto);
        });
    }
    if opened || menu.revision != browser.scan_revision {
        menu.entries = if dialogue {
            dialogue_entries(&browser, selected)
        } else {
            entries(&editor, &browser, selected)
        };
        menu.revision = browser.scan_revision;
        menu.filter(&ui.search);
    } else if menu.query != ui.search {
        menu.filter(&ui.search);
    }
    if let Some(placeholder) = menu.placeholder {
        if let Ok(mut n) = nodes.get_mut(placeholder) {
            n.display = if ui.search.is_empty() {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    let desired = menu.base + menu.entries.len().clamp(1, 5) as f32 * ROW;
    let usable_height = if dialogue {
        root_size
            .y
            .min(window.height() - FOOT - drawer.visible_height - root_min.y)
    } else {
        root_size.y
    };
    let below = (usable_height - 10. - anchor.max.y - 6.).max(0.);
    let above = (anchor.min.y - 14.).max(0.);
    let down = if new_session {
        below >= desired || below >= above
    } else {
        menu.down
    };
    menu.down = down;
    let height = desired.min(if down { below } else { above }).max(100.);
    let left = anchor.min.x.clamp(8., (root_size.x - width - 8.).max(8.));
    let top = if down {
        anchor.max.y + 6.
    } else {
        anchor.min.y - height - 6.
    };
    if let Ok(mut n) = nodes.get_mut(menu.root) {
        n.display = Display::Flex;
    }
    if let Ok(mut n) = nodes.get_mut(menu.panel) {
        n.left = px(left);
        n.top = px(top);
        n.width = px(width);
        n.height = px(height);
    }
    if menu.repaint {
        menu.repaint = false;
        let list = menu.list.unwrap();
        commands.entity(list).despawn_related::<Children>();
        for (position, index) in menu.filtered.iter().enumerate() {
            let e = &menu.entries[*index];
            let row = control(
                &mut commands,
                list,
                Action::Narrative(choose(ui.picker, e.index, menu.revision)),
                Look::Regular,
                Node {
                    width: percent(100),
                    height: px(50),
                    flex_shrink: 0.,
                    padding: UiRect::all(px(7)),
                    border_radius: BorderRadius::all(px(7)),
                    overflow: Overflow::clip(),
                    ..row(9.)
                },
            );
            commands.entity(row).insert((
                ChoiceRow(position),
                AccessibleLabel(if dialogue {
                    e.path.clone()
                } else {
                    format!("Выбрать персонажа: {}", e.name)
                }),
            ));
            let avatar = surface(
                &mut commands,
                row,
                Node {
                    width: px(32),
                    height: px(32),
                    flex_shrink: 0.,
                    border_radius: BorderRadius::all(px(16)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                WELL,
            );
            commands.entity(avatar).insert(Pickable::IGNORE);
            if dialogue {
                icons::spawn(&mut commands, avatar, icons::Glyph::DialogueFile, 18., BLUE);
            } else if let Some(path) = &e.portrait {
                portrait(&mut commands, avatar, &server, &editor.root, path, 32.);
            } else {
                text(
                    &mut commands,
                    avatar,
                    &ui.font,
                    e.name
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                    15.,
                    BLUE,
                );
            }
            let details = node(
                &mut commands,
                row,
                Node {
                    flex_grow: 1.,
                    flex_basis: px(0),
                    min_width: px(0),
                    overflow: Overflow::clip(),
                    ..column(3.)
                },
            );
            commands.entity(details).insert(Pickable::IGNORE);
            text(&mut commands, details, &ui.font, &e.name, 13., TEXT);
            if e.duplicate {
                text(&mut commands, details, &ui.font, &e.path, 10., MUTED);
            }
            if e.current {
                text(&mut commands, row, &ui.font, "Текущий", 10., FOCUS);
            }
            picker_fx::layer(
                &mut commands,
                row,
                1,
                e.current,
                Some(position),
                context.node,
            );
        }
        if menu.filtered.is_empty() {
            wrapped(
                &mut commands,
                list,
                &ui.font,
                if menu.entries.is_empty() {
                    if dialogue {
                        "В проекте пока нет других диалогов."
                    } else {
                        "Создайте персонажа в файловом менеджере."
                    }
                } else {
                    "Ничего не найдено. Попробуйте другое имя."
                },
                12.,
                MUTED,
            );
        }
        picker_fx::tag(&mut commands, menu.panel);
    }
    if menu.scroll_to_cursor {
        menu.scroll_to_cursor = false;
        let viewport = (height - menu.base).max(0.);
        let max = (menu.filtered.len() as f32 * ROW - viewport).max(0.);
        let list = menu.list.unwrap();
        let current = scrolls.get(list).map_or(0., |p| p.y);
        let first = menu.cursor as f32 * ROW;
        let offset = if first < current {
            first
        } else if first + 50. > current + viewport {
            first + 50. - viewport
        } else {
            current
        }
        .clamp(0., max);
        if let Ok(mut scroll) = scrolls.get_mut(list) {
            scroll.0.y = offset;
        }
    }
}
