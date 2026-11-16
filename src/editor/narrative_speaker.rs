//! A searchable speaker dropdown anchored to the node that opened it.
use super::*;

#[derive(Component)]
pub(super) struct Anchor(pub u32);

struct Entry {
    index: usize,
    name: String,
    path: String,
    portrait: Option<String>,
    haystack: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
    document: usize,
    node: u32,
    generation: u64,
}
#[derive(Resource)]
pub(super) struct Menu {
    root: Entity,
    panel: Entity,
    search: Option<Entity>,
    list: Option<Entity>,
    context: Option<Context>,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    cursor: usize,
    revision: u64,
    query: String,
    repaint: bool,
}

fn normalized(value: &str) -> String {
    value.to_lowercase().replace('ё', "е")
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
    // A sibling backdrop closes the dropdown without activating the covered canvas.
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
            ..column(6.0)
        },
        PANEL,
    );
    commands.entity(panel).insert((
        ZIndex(1),
        BorderColor::all(FOCUS.with_alpha(0.5)),
        AccessibleLabel("Список персонажей".into()),
    ));
    commands.insert_resource(Menu {
        root,
        panel,
        search: None,
        list: None,
        context: None,
        entries: Vec::new(),
        filtered: Vec::new(),
        cursor: 0,
        revision: 0,
        query: String::new(),
        repaint: false,
    });
}

fn entries(editor: &Editor, browser: &Browser) -> Vec<Entry> {
    let mut result = Vec::new();
    for (i, file) in browser
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.extension.eq_ignore_ascii_case("dcharacter"))
    {
        let path = file.path.to_string_lossy().replace('\\', "/");
        let Some(character) = character(editor, &path) else {
            continue;
        };
        let name = if character.name.trim().is_empty() {
            file.filename.clone()
        } else {
            character.name
        };
        result.push(Entry {
            index: i,
            haystack: normalized(&format!("{name} {path}")),
            name,
            path,
            portrait: character.portraits.first().map(|p| p.path.clone()),
        });
    }
    result.sort_by_cached_key(|e| (normalized(&e.name), normalized(&e.path)));
    result
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
    }
}

pub(super) fn input(
    ui: Res<NarrativeUi>,
    mut menu: ResMut<Menu>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: ResMut<InputFocus>,
    mut actions: MessageWriter<Action>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    if ui.picker != Some(Picker::Speaker) || menu.context.is_none() {
        return;
    }
    if !window.focused {
        actions.write(Action::Narrative(NarrativeAction::Dismiss));
        return;
    }
    if menu.query != ui.search {
        menu.filter(&ui.search);
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::ArrowUp) {
        if keys.just_pressed(KeyCode::ArrowDown) {
            menu.cursor = (menu.cursor + 1).min(menu.filtered.len().saturating_sub(1));
        } else {
            menu.cursor = menu.cursor.saturating_sub(1);
        }
        menu.repaint = true;
        if let Some(search) = menu.search {
            focus.set(search, bevy::input_focus::FocusCause::Auto);
        }
    }
    if keys.just_pressed(KeyCode::Enter) && focus.get() == menu.search {
        if let Some(entry) = menu
            .filtered
            .get(menu.cursor)
            .and_then(|i| menu.entries.get(*i))
        {
            actions.write(Action::Narrative(NarrativeAction::SetSpeaker(Some(
                entry.index,
            ))));
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
) {
    let doc = editor.narrative.current();
    let active = ui.picker == Some(Picker::Speaker) && editor.pending.is_none();
    let context = doc.filter(|_| active).and_then(|doc| match &doc.content {
        Content::Dialogue(d)
            if d.node(doc.selected)
                .is_some_and(|n| n.kind == NodeKind::Line) =>
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
        if let Ok(mut node) = nodes.get_mut(menu.root) {
            node.display = Display::None;
        }
        if let Some(previous) = menu.context.take() {
            restore_focus(&mut commands, previous);
        }
        return;
    };
    let doc = doc.unwrap();
    let selected = match &doc.content {
        Content::Dialogue(d) => d.node(context.node).and_then(|n| n.speaker.as_deref()),
        _ => None,
    };
    let Some((_, anchor)) = anchors
        .iter()
        .find(|(_, anchor)| anchor.0 == context.node)
        .and_then(|(e, _)| geometry.get(e).ok().map(|geometry| (e, geometry)))
    else {
        return;
    };
    let Ok((root_size, root_transform)) = geometry.get(ui.root) else {
        return;
    };
    let (computed, transform) = anchor;
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

    let opened = menu.context != Some(context);
    if opened {
        menu.context = Some(context);
        commands.entity(menu.panel).despawn_related::<Children>();
        let search = field(
            &mut commands,
            menu.panel,
            &ui.font,
            "Поиск персонажа",
            "",
            DraftField::Search,
            false,
        );
        menu.search = Some(search);
        let list = node(
            &mut commands,
            menu.panel,
            Node {
                width: percent(100),
                flex_grow: 1.0,
                min_height: px(0),
                overflow: Overflow::scroll_y(),
                ..column(2.0)
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
            "Enter — выбрать · Esc — закрыть",
            10.0,
            MUTED,
        );
        ui.search.clear();
        commands.queue(move |world: &mut World| {
            world
                .resource_mut::<InputFocus>()
                .set(search, bevy::input_focus::FocusCause::Auto);
        });
    }
    if opened || menu.revision != browser.scan_revision {
        menu.entries = entries(&editor, &browser);
        menu.revision = browser.scan_revision;
        menu.filter(&ui.search);
        if ui.search.is_empty() {
            menu.cursor = menu
                .filtered
                .iter()
                .position(|i| {
                    let entry = &menu.entries[*i];
                    selected == Some(entry.path.as_str())
                })
                .unwrap_or(0);
        }
    } else if menu.query != ui.search {
        menu.filter(&ui.search);
    }
    let width = visual_size
        .x
        .max(320.0)
        .min(420.0)
        .min((root_size.x - 16.0).max(100.0));
    let desired = (100.0 + menu.filtered.len().clamp(1, 5) as f32 * 54.0).min(370.0);
    let below = (root_size.y - 10.0 - anchor.max.y - 6.0).max(0.0);
    let above = (anchor.min.y - 6.0 - 8.0).max(0.0);
    let down = below >= desired || below >= above;
    let height = desired.min(if down { below } else { above }).max(100.0);
    let left = anchor
        .min
        .x
        .clamp(8.0, (root_size.x - width - 8.0).max(8.0));
    let top = if down {
        anchor.max.y + 6.0
    } else {
        anchor.min.y - height - 6.0
    };
    if let Ok(mut root) = nodes.get_mut(menu.root) {
        root.display = Display::Flex;
    }
    if let Ok(mut panel) = nodes.get_mut(menu.panel) {
        panel.left = px(left);
        panel.top = px(top);
        panel.width = px(width);
        panel.height = px(height);
    }
    if menu.repaint {
        menu.repaint = false;
        let list = menu.list.unwrap();
        commands.entity(list).despawn_related::<Children>();
        let viewport = (height - 100.0).max(0.0);
        let max = (menu.filtered.len() as f32 * 54.0 - viewport).max(0.0);
        let current = scrolls.get(list).map_or(0.0, |p| p.y);
        let first = menu.cursor as f32 * 54.0;
        let offset = if first < current {
            first
        } else if first + 52.0 > current + viewport {
            first + 52.0 - viewport
        } else {
            current
        }
        .clamp(0.0, max);
        if let Ok(mut scroll) = scrolls.get_mut(list) {
            scroll.0.y = offset;
        } else {
            commands
                .entity(list)
                .insert(ScrollPosition(Vec2::new(0.0, offset)));
        }
        for (position, index) in menu.filtered.iter().enumerate() {
            let entry = &menu.entries[*index];
            let highlighted = position == menu.cursor;
            let current = selected == Some(entry.path.as_str());
            let row = control(
                &mut commands,
                list,
                Action::Narrative(NarrativeAction::SetSpeaker(Some(entry.index))),
                if highlighted {
                    Look::Primary
                } else {
                    Look::Regular
                },
                Node {
                    width: percent(100),
                    height: px(52),
                    flex_shrink: 0.0,
                    padding: UiRect::all(px(8)),
                    border_radius: BorderRadius::all(px(6)),
                    overflow: Overflow::clip(),
                    ..row(9.0)
                },
            );
            commands.entity(row).insert(AccessibleLabel(format!(
                "Выбрать персонажа: {}",
                entry.name
            )));
            let avatar = surface(
                &mut commands,
                row,
                Node {
                    width: px(32),
                    height: px(32),
                    flex_shrink: 0.0,
                    border_radius: BorderRadius::all(px(16)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                WELL,
            );
            commands.entity(avatar).insert(Pickable::IGNORE);
            if let Some(portrait_path) = &entry.portrait {
                portrait(
                    &mut commands,
                    avatar,
                    &server,
                    &editor.root,
                    portrait_path,
                    32.0,
                );
            } else {
                text(
                    &mut commands,
                    avatar,
                    &ui.font,
                    entry
                        .name
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                    15.0,
                    BLUE,
                );
            }
            let details = node(
                &mut commands,
                row,
                Node {
                    flex_grow: 1.0,
                    flex_basis: px(0),
                    min_width: px(0),
                    overflow: Overflow::clip(),
                    ..column(3.0)
                },
            );
            commands.entity(details).insert(Pickable::IGNORE);
            text(
                &mut commands,
                details,
                &ui.font,
                &entry.name,
                13.0,
                if highlighted { ON_ACCENT } else { TEXT },
            );
            text(
                &mut commands,
                details,
                &ui.font,
                &entry.path,
                10.0,
                if highlighted {
                    ON_ACCENT.with_alpha(0.75)
                } else {
                    MUTED
                },
            );
            if current {
                text(
                    &mut commands,
                    row,
                    &ui.font,
                    "Выбран",
                    10.0,
                    if highlighted { ON_ACCENT } else { FOCUS },
                );
            }
        }
        if menu.filtered.is_empty() {
            wrapped(
                &mut commands,
                list,
                &ui.font,
                if menu.entries.is_empty() {
                    "Пока нет персонажей. Создайте их в файловом менеджере: Диалоги → Персонаж."
                } else {
                    "Персонажи не найдены. Попробуйте другое имя."
                },
                12.0,
                MUTED,
            );
        }
    }
}
