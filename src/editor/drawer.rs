//! The asset browser is an overlay: sliding it never changes the scene camera.
use super::*;
use crate::editor::{browser::Entry, image_previews::ImagePreviews, thumbnails::Thumbnails};

#[path = "drawer_resize.rs"]
mod resize;
pub(crate) use resize::{DrawerSize, resize as resize_drawer};

#[path = "asset_drag.rs"]
mod drag;
pub(crate) use drag::{AssetDrag, DropFolder, pointer as drag_assets, visuals as drag_visuals};

const DURATION: f32 = 0.24;
const CARD: f32 = 180.0;
const CARD_BORDER: f32 = 1.0;
const PREVIEW_WIDTH: f32 = CARD - CARD_BORDER * 2.0;
const PREVIEW_HEIGHT: f32 = PREVIEW_WIDTH / 1.6;
const CAPTION_HEIGHT: f32 = 64.0;
const CARD_HEIGHT: f32 = PREVIEW_HEIGHT + CAPTION_HEIGHT + CARD_BORDER * 2.0;

#[derive(Resource, Default)]
pub(crate) struct DrawerMotion {
    progress: f32,
    pub visible_height: f32,
    pub resizing: bool,
}

impl DrawerMotion {
    fn advance(&mut self, open: bool, delta: f32, height: f32) {
        let direction = if open { 1.0 } else { -1.0 };
        self.progress = (self.progress + direction * delta / DURATION).clamp(0.0, 1.0);
        let eased = self.progress * self.progress * (3.0 - 2.0 * self.progress);
        self.visible_height = height * eased;
    }
}

#[derive(Component)]
pub(crate) struct DrawerScroll;

#[derive(Component)]
pub(crate) struct AssetSearch;

#[derive(Component)]
pub(crate) struct FileCard(Entry, u64);

#[derive(Component)]
pub(crate) struct FileImage(usize, u64);

#[derive(Component)]
pub(crate) struct ImageInfo(usize, u64);

#[derive(Component)]
pub(crate) struct ImagePlaceholder(usize, u64);

#[derive(Resource)]
pub(crate) struct BrowserUi {
    font: Handle<Font>,
    panel: Entity,
    tree: Entity,
    breadcrumbs: Entity,
    grid: Entity,
    count: Entity,
    detail: Entity,
    placeholder: Entity,
    overlay: Entity,
    pub(super) selected: Option<usize>,
    pub(super) selected_model: Option<usize>,
}

fn folder_icon(commands: &mut Commands, parent: Entity, size: f32, expanded: bool) {
    icons::spawn(
        commands,
        parent,
        if expanded {
            icons::Glyph::FolderOpen
        } else {
            icons::Glyph::Folder
        },
        size,
        Color::srgb(0.83, 0.65, 0.36),
    );
}

pub(super) fn footer_toggle(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    let toggle = control(
        commands,
        parent,
        Action::Drawer,
        Look::Regular,
        Node {
            height: percent(100),
            padding: UiRect::horizontal(px(10)),
            flex_shrink: 0.0,
            ..row(8.0)
        },
    );
    commands
        .entity(toggle)
        .insert(AccessibleLabel("Файловый менеджер · Ctrl+Space".into()));
    folder_icon(commands, toggle, 16.0, false);
    text(commands, toggle, font, "Файловый менеджер", 13.0, TEXT);
    text(commands, toggle, font, "Ctrl+Space", 10.0, MUTED);
}

pub(super) fn setup(
    commands: &mut Commands,
    root: Entity,
    font_handle: &Handle<Font>,
    stars: &starfield::Starfield,
) {
    let panel = surface(
        commands,
        root,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(FOOT - DRAWER),
            left: px(0),
            right: px(0),
            height: px(DRAWER),
            display: Display::None,
            overflow: Overflow::clip(),
            border: UiRect::top(px(2)),
            ..column(0.0)
        },
        PANEL,
    );
    starfield::background(commands, panel, &stars.chrome);
    commands
        .entity(panel)
        .insert((ZIndex(10), BorderColor::all(EDGE)));
    commands
        .entity(panel)
        .insert(BorderGradient::from(LinearGradient::to_right(vec![
            ColorStop::auto(FOCUS.with_alpha(0.65)),
            ColorStop::auto(Color::srgba(0.53, 0.44, 0.70, 0.4)),
            ColorStop::auto(EDGE),
        ])));

    let grip = surface(
        commands,
        panel,
        Node {
            position_type: PositionType::Absolute,
            top: px(0),
            left: px(0),
            right: px(0),
            height: px(resize::GRIP_HEIGHT),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        Color::NONE,
    );
    commands.entity(grip).insert((
        resize::ResizeGrip,
        ZIndex(5),
        AccessibleLabel("Изменить высоту файлового менеджера".into()),
    ));
    let mark = surface(
        commands,
        grip,
        Node {
            width: px(38),
            height: px(2),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        EDGE,
    );
    commands.entity(mark).insert(Pickable::IGNORE);

    let toolbar = surface(
        commands,
        panel,
        Node {
            height: px(44),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(10)),
            border: UiRect::bottom(px(1)),
            ..row(6.0)
        },
        CHROME,
    );
    for (glyph, spoken, action) in [
        (
            icons::Glyph::ChevronLeft,
            "Предыдущая папка",
            Action::BrowseBack,
        ),
        (
            icons::Glyph::ChevronRight,
            "Следующая папка",
            Action::BrowseForward,
        ),
        (icons::Glyph::ArrowUp, "На уровень выше", Action::BrowseUp),
    ] {
        icons::button(commands, toolbar, glyph, spoken, action, Look::Regular);
    }
    let breadcrumbs = node(
        commands,
        toolbar,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..row(2.0)
        },
    );
    let refresh = button(
        commands,
        toolbar,
        font_handle,
        "Обновить",
        Action::BrowseRefresh,
        Look::Regular,
    );
    icons::themed(commands, refresh, icons::Glyph::Refresh, 14.0);
    let collapse = button(
        commands,
        toolbar,
        font_handle,
        "Свернуть",
        Action::Drawer,
        Look::Regular,
    );
    icons::themed(commands, collapse, icons::Glyph::ChevronDown, 14.0);

    let body = node(
        commands,
        panel,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            ..default()
        },
    );
    let sidebar = surface(
        commands,
        body,
        Node {
            width: px(224),
            flex_shrink: 0.0,
            border: UiRect::right(px(1)),
            min_height: px(0),
            ..column(0.0)
        },
        WELL,
    );
    let heading = node(
        commands,
        sidebar,
        Node {
            height: px(41),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(14)),
            ..row(0.0)
        },
    );
    text(commands, heading, font_handle, "ПАПКИ ПРОЕКТА", 10.0, MUTED);
    let tree = node(
        commands,
        sidebar,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            padding: UiRect::horizontal(px(6)),
            overflow: Overflow::scroll_y(),
            ..column(2.0)
        },
    );
    commands.entity(tree).insert((Scroller, DrawerScroll));

    let content = node(
        commands,
        body,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            min_height: px(0),
            ..column(0.0)
        },
    );
    let filters = node(
        commands,
        content,
        Node {
            height: px(45),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(12)),
            ..row(10.0)
        },
    );
    let search_box = surface(
        commands,
        filters,
        Node {
            height: px(29),
            max_width: px(480),
            min_width: px(120),
            flex_grow: 1.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            padding: UiRect::horizontal(px(9)),
            ..row(7.0)
        },
        WELL,
    );
    icons::spawn(commands, search_box, icons::Glyph::Search, 16.0, MUTED);
    let input_area = node(
        commands,
        search_box,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: percent(100),
            ..default()
        },
    );
    commands.spawn((
        AssetSearch,
        TextInput,
        EditableText::new(""),
        TextCursorStyle::default(),
        AccessibleLabel("Поиск файлов и папок в текущей папке".into()),
        font(font_handle, 12.0),
        TextColor(TEXT),
        TextLayout::no_wrap(),
        TabIndex(10),
        Node {
            width: percent(100),
            height: percent(100),
            padding: UiRect::vertical(px(5)),
            overflow: Overflow::clip(),
            ..default()
        },
        ChildOf(input_area),
    ));
    let placeholder = text(
        commands,
        input_area,
        font_handle,
        "Поиск файлов и папок…",
        12.0,
        MUTED,
    );
    commands.entity(placeholder).insert(Node {
        position_type: PositionType::Absolute,
        top: px(5),
        ..default()
    });
    icons::button(
        commands,
        search_box,
        icons::Glyph::Close,
        "Очистить поиск",
        Action::BrowseClear,
        Look::Regular,
    );
    text(commands, filters, font_handle, "ВСЕ ФАЙЛЫ", 10.0, BLUE);

    let grid = node(
        commands,
        content,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            min_width: px(0),
            padding: UiRect::axes(px(12), px(2)),
            overflow: Overflow::scroll_y(),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::Start,
            align_items: AlignItems::Start,
            row_gap: px(10),
            column_gap: px(10),
            ..default()
        },
    );
    commands.entity(grid).insert((Scroller, DrawerScroll));
    let status = surface(
        commands,
        panel,
        Node {
            height: px(28),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(12)),
            border: UiRect::top(px(1)),
            overflow: Overflow::clip(),
            ..row(12.0)
        },
        CHROME,
    );
    let count = text(commands, status, font_handle, "", 11.0, MUTED);
    let detail_cell = node(
        commands,
        status,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            ..row(0.0)
        },
    );
    let detail = text(
        commands,
        detail_cell,
        font_handle,
        "Потяните верхнюю границу, чтобы изменить высоту панели",
        11.0,
        MUTED,
    );

    commands.insert_resource(BrowserUi {
        font: font_handle.clone(),
        panel,
        tree,
        breadcrumbs,
        grid,
        count,
        detail,
        placeholder,
        overlay: root,
        selected: None,
        selected_model: None,
    });
}

pub(crate) fn animate(
    editor: Res<Editor>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    interface: Res<BrowserUi>,
    size: Res<DrawerSize>,
    mut motion: ResMut<DrawerMotion>,
    mut nodes: Query<&mut Node>,
) {
    let height = size.height(window.height());
    motion.advance(editor.drawer, time.delta_secs(), height);
    if let Ok(mut panel) = nodes.get_mut(interface.panel) {
        let display = if motion.progress > 0.0 {
            Display::Flex
        } else {
            Display::None
        };
        let bottom = px(FOOT - height + motion.visible_height);
        if panel.height != px(height) {
            panel.height = px(height);
        }
        if panel.bottom != bottom {
            panel.bottom = bottom;
        }
        if panel.display != display {
            panel.display = display;
        }
    }
}

pub(crate) fn actions(
    mut messages: MessageReader<Action>,
    mut browser: ResMut<Browser>,
    mut editor: ResMut<Editor>,
    pointer: Res<viewport::PointerState>,
    mut previews: ResMut<Thumbnails>,
    mut images: ResMut<Assets<Image>>,
    mut interface: ResMut<BrowserUi>,
    redirects: Option<Res<crate::asset_paths::AssetRedirects>>,
    numeric: Option<Res<NumericInput>>,
) {
    for action in messages.read() {
        if editor.pending.is_some() || pointer.dragging() {
            continue;
        }
        match *action {
            Action::OpenBrowser => {
                let folder = editor
                    .project
                    .scenes
                    .get(editor.room)
                    .and_then(|scene| {
                        let parent = std::path::Path::new(&scene.asset).parent()?;
                        browser
                            .folders
                            .iter()
                            .position(|folder| folder.path == parent)
                    })
                    .unwrap_or(0);
                browser.navigate(folder);
                interface.selected = None;
                interface.selected_model = None;
            }
            Action::Browse(0) => {
                browser.home();
                interface.selected = None;
                interface.selected_model = None;
            }
            Action::Browse(folder) => {
                browser.navigate(folder);
                interface.selected = None;
                interface.selected_model = None;
            }
            Action::BrowseBack => {
                browser.go_back();
                interface.selected = None;
                interface.selected_model = None;
            }
            Action::BrowseForward => {
                browser.go_forward();
                interface.selected = None;
                interface.selected_model = None;
            }
            Action::BrowseUp => {
                browser.up();
                interface.selected = None;
                interface.selected_model = None;
            }
            Action::BrowseClear => {
                browser.set_search(String::new());
            }
            Action::BrowseRefresh => {
                interface.selected = None;
                interface.selected_model = None;
                browser.refresh(&mut editor.project);
                previews.extend(&mut images, editor.project.catalog.len());
            }
            Action::BrowseToggle(folder) => {
                browser.toggle_folder(folder);
            }
            Action::BrowseFile(file) if file < browser.files.len() => {
                interface.selected = Some(file);
                interface.selected_model = None;
            }
            Action::BrowseModel(model) if model < editor.project.catalog.len() => {
                interface.selected = None;
                interface.selected_model = Some(model);
            }
            Action::MoveAsset(entry, folder) => {
                if let Some(error) = numeric.as_ref().and_then(|input| input.error_message()) {
                    editor.status = error.into();
                    continue;
                }
                let scene_asset = editor
                    .has_scene()
                    .then(|| editor.definition().asset.clone());
                match crate::editor::file_moves::move_entry(
                    &mut editor,
                    &mut browser,
                    entry,
                    folder,
                ) {
                    Ok(true) => {
                        if let Some(redirects) = &redirects
                            && let Err(error) = redirects.reload()
                        {
                            editor.status =
                                format!("Файл перемещён, ошибка обновления ссылок · {error}");
                            continue;
                        }
                        interface.selected = None;
                        interface.selected_model = None;
                        previews.extend(&mut images, editor.project.catalog.len());
                        if scene_asset.is_some_and(|asset| asset != editor.definition().asset) {
                            editor.restore_selection = match editor.selection {
                                Some(Selection::Imported(index)) => {
                                    editor.imported.get(index).map(|node| node.name.clone())
                                }
                                _ => None,
                            };
                            editor.reload = true;
                        }
                        editor.status = "Файл перемещён".into();
                    }
                    Ok(false) => {}
                    Err(error) => editor.status = format!("Не удалось переместить · {error}"),
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn search(
    mut browser: ResMut<Browser>,
    focus: Res<InputFocus>,
    interface: Res<BrowserUi>,
    mut inputs: Query<(Entity, &mut EditableText), With<AssetSearch>>,
    mut nodes: Query<&mut Node>,
) {
    let Ok((entity, mut input)) = inputs.single_mut() else {
        return;
    };
    let value = input.value().to_string();
    if focus.get() == Some(entity) {
        browser.set_search(value);
    } else if value != browser.search {
        input.editor_mut().set_text(&browser.search);
    }
    if let Ok(mut hint) = nodes.get_mut(interface.placeholder) {
        let display = if browser.search.is_empty() && focus.get() != Some(entity) {
            Display::Flex
        } else {
            Display::None
        };
        if hint.display != display {
            hint.display = display;
        }
    }
}

fn tree_folder(
    commands: &mut Commands,
    interface: &BrowserUi,
    browser: &Browser,
    index: usize,
    depth: usize,
) {
    let folder = &browser.folders[index];
    let line = node(
        commands,
        interface.tree,
        Node {
            width: percent(100),
            height: px(28),
            flex_shrink: 0.0,
            padding: UiRect::left(px(3.0 + depth as f32 * 14.0)),
            ..row(2.0)
        },
    );
    if index != 0 && !folder.children.is_empty() {
        let toggle = control(
            commands,
            line,
            Action::BrowseToggle(index),
            Look::Regular,
            Node {
                width: px(19),
                height: px(24),
                flex_shrink: 0.0,
                justify_content: JustifyContent::Center,
                ..row(0.0)
            },
        );
        commands.entity(toggle).insert(AccessibleLabel(format!(
            "{} папку {}",
            if browser.is_expanded(index) {
                "Свернуть"
            } else {
                "Раскрыть"
            },
            folder.label
        )));
        icons::themed(
            commands,
            toggle,
            if browser.is_expanded(index) {
                icons::Glyph::ChevronDown
            } else {
                icons::Glyph::ChevronRight
            },
            12.0,
        );
    } else {
        node(
            commands,
            line,
            Node {
                width: px(19),
                flex_shrink: 0.0,
                ..default()
            },
        );
    }
    let item = control(
        commands,
        line,
        Action::Browse(index),
        Look::Regular,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: px(28),
            padding: UiRect::horizontal(px(5)),
            border_radius: BorderRadius::all(px(4)),
            overflow: Overflow::clip(),
            ..row(7.0)
        },
    );
    commands.entity(item).insert((
        DropFolder(index, browser.scan_revision),
        AccessibleLabel(format!("Открыть папку {}", folder.label)),
    ));
    if index != 0 {
        commands
            .entity(item)
            .insert(FileCard(Entry::Folder(index), browser.scan_revision));
    }
    folder_icon(commands, item, 14.0, browser.is_expanded(index));
    text(commands, item, &interface.font, &folder.label, 12.0, TEXT);
    if browser.is_expanded(index) {
        for &child in &folder.children {
            tree_folder(
                commands,
                interface,
                browser,
                child,
                depth + usize::from(index != 0),
            );
        }
    }
}

fn asset_card(commands: &mut Commands, grid: Entity, action: Action) -> Entity {
    control(
        commands,
        grid,
        action,
        Look::Asset,
        Node {
            width: px(CARD),
            height: px(CARD_HEIGHT),
            flex_shrink: 0.0,
            border: UiRect::all(px(CARD_BORDER)),
            border_radius: BorderRadius::all(px(10)),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
    )
}

fn caption(
    commands: &mut Commands,
    card: Entity,
    font_handle: &Handle<Font>,
    filename: &str,
    kind: &str,
    accent: Color,
) -> Entity {
    let caption = node(
        commands,
        card,
        Node {
            width: percent(100),
            height: px(CAPTION_HEIGHT),
            flex_shrink: 0.0,
            border: UiRect::top(px(1)),
            padding: UiRect::axes(px(8), px(6)),
            overflow: Overflow::clip(),
            ..column(3.0)
        },
    );
    commands
        .entity(caption)
        .insert(BorderColor::all(accent.with_alpha(0.65)));
    let title = text(commands, caption, font_handle, filename, 13.0, TEXT);
    commands.entity(title).insert((
        TextLayout::linebreak(bevy::text::LineBreak::WordOrCharacter),
        Node {
            width: percent(100),
            height: px(32),
            overflow: Overflow::clip(),
            ..default()
        },
    ));
    text(commands, caption, font_handle, kind, 11.0, accent)
}

fn preview_well(commands: &mut Commands, card: Entity) -> Entity {
    surface(
        commands,
        card,
        Node {
            width: px(PREVIEW_WIDTH),
            height: px(PREVIEW_HEIGHT),
            flex_shrink: 0.0,
            border_radius: BorderRadius::top(px(9)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        WELL,
    )
}

fn file_icon(
    commands: &mut Commands,
    parent: Entity,
    font_handle: &Handle<Font>,
    extension: &str,
) -> Entity {
    let icon = node(
        commands,
        parent,
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            width: px(54),
            height: px(62),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    commands.entity(icon).insert((
        Pickable::IGNORE,
        UiTransform::from_translation(Val2::percent(-50, -50)),
    ));
    let sheet = icons::spawn(commands, icon, icons::Glyph::File, 54.0, MUTED);
    commands
        .entity(sheet)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.position_type = PositionType::Absolute;
            node.left = px(0);
            node.top = px(4);
        });
    text(
        commands,
        icon,
        font_handle,
        if extension.is_empty() {
            "FILE".into()
        } else {
            extension.chars().take(6).collect::<String>().to_uppercase()
        },
        12.0,
        MUTED,
    );
    icon
}

fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} Б")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} КБ", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МБ", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub(crate) fn refresh(
    mut commands: Commands,
    browser: Res<Browser>,
    editor: Res<Editor>,
    previews: Res<Thumbnails>,
    interface: Res<BrowserUi>,
    mut revision: Local<Option<u64>>,
    mut tree_revision: Local<Option<u64>>,
    mut labels: Query<&mut Text>,
) {
    if *tree_revision != Some(browser.tree_revision) {
        *tree_revision = Some(browser.tree_revision);
        commands.entity(interface.tree).despawn_children();
        tree_folder(&mut commands, &interface, &browser, 0, 0);
    }
    if *revision == Some(browser.revision) {
        return;
    }
    *revision = Some(browser.revision);
    for entity in [interface.breadcrumbs, interface.grid] {
        commands.entity(entity).despawn_children();
    }
    commands.entity(interface.grid).insert((
        ScrollPosition::default(),
        DropFolder(browser.current, browser.scan_revision),
    ));
    for (step, folder) in browser.breadcrumbs().into_iter().enumerate() {
        if step > 0 {
            icons::spawn(
                &mut commands,
                interface.breadcrumbs,
                icons::Glyph::ChevronRight,
                14.0,
                MUTED,
            );
        }
        let breadcrumb = button(
            &mut commands,
            interface.breadcrumbs,
            &interface.font,
            &browser.folders[folder].label,
            Action::Browse(folder),
            Look::Regular,
        );
        commands
            .entity(breadcrumb)
            .insert(DropFolder(folder, browser.scan_revision));
    }
    let entries = browser.entries(&editor.project.catalog);
    let mut folders = 0;
    for &entry in &entries {
        match entry {
            Entry::Folder(index) => {
                folders += 1;
                let folder = &browser.folders[index];
                let card = asset_card(&mut commands, interface.grid, Action::Browse(index));
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    DropFolder(index, browser.scan_revision),
                    AccessibleLabel(format!("Открыть папку {}", folder.label)),
                ));
                let well = preview_well(&mut commands, card);
                folder_icon(&mut commands, well, 57.0, false);
                caption(
                    &mut commands,
                    card,
                    &interface.font,
                    &folder.label,
                    "Папка",
                    MUTED,
                );
            }
            Entry::Model(index) => {
                let item = &editor.project.catalog[index];
                let filename = item.asset.rsplit('/').next().unwrap_or(&item.asset);
                let card = asset_card(&mut commands, interface.grid, Action::BrowseModel(index));
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    AccessibleLabel(format!("Выбрать модель {filename} · {}", item.label)),
                ));
                commands.spawn((
                    ImageNode::new(previews.images[index].clone()),
                    Pickable::IGNORE,
                    Node {
                        width: px(PREVIEW_WIDTH),
                        height: px(PREVIEW_HEIGHT),
                        flex_shrink: 0.0,
                        border_radius: BorderRadius::top(px(9)),
                        ..default()
                    },
                    ChildOf(card),
                ));
                caption(
                    &mut commands,
                    card,
                    &interface.font,
                    filename,
                    &format!("GLB · {}", item.label),
                    BLUE,
                );
            }
            Entry::Scene { file, room } => {
                let file = &browser.files[file];
                let scene = &editor.project.scenes[room];
                let card = asset_card(&mut commands, interface.grid, Action::Open(room));
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    AccessibleLabel(format!(
                        "Открыть сцену {} · {}",
                        scene.label,
                        file.path.display()
                    )),
                ));
                commands.spawn((
                    ImageNode::new(previews.scene_images[room].clone()),
                    Pickable::IGNORE,
                    Node {
                        width: px(PREVIEW_WIDTH),
                        height: px(PREVIEW_HEIGHT),
                        flex_shrink: 0.0,
                        border_radius: BorderRadius::top(px(9)),
                        ..default()
                    },
                    ChildOf(card),
                ));
                caption(
                    &mut commands,
                    card,
                    &interface.font,
                    &file.filename,
                    &format!("Сцена · {}", scene.label),
                    AMBER,
                );
            }
            Entry::Image(index) | Entry::File(index) => {
                let file = &browser.files[index];
                let card = asset_card(&mut commands, interface.grid, Action::BrowseFile(index));
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    AccessibleLabel(format!("Выбрать файл {}", file.filename)),
                ));
                let well = preview_well(&mut commands, card);
                let kind = if matches!(entry, Entry::Image(_)) {
                    let placeholder =
                        file_icon(&mut commands, well, &interface.font, &file.extension);
                    commands.entity(placeholder).insert((
                        ImagePlaceholder(index, browser.scan_revision),
                        Visibility::Inherited,
                    ));
                    commands.spawn((
                        FileImage(index, browser.scan_revision),
                        ImageNode::default(),
                        Pickable::IGNORE,
                        Node {
                            display: Display::None,
                            width: px(0),
                            height: px(0),
                            border_radius: BorderRadius::top(px(9)),
                            ..default()
                        },
                        ChildOf(well),
                    ));
                    format!("{} · Загрузка…", file.extension.to_uppercase())
                } else {
                    file_icon(&mut commands, well, &interface.font, &file.extension);
                    format!(
                        "{} · {}",
                        if file.extension.is_empty() {
                            "Файл".into()
                        } else {
                            file.extension.to_uppercase()
                        },
                        file_size(file.bytes)
                    )
                };
                let info = caption(
                    &mut commands,
                    card,
                    &interface.font,
                    &file.filename,
                    &kind,
                    if matches!(entry, Entry::Image(_)) {
                        GREEN
                    } else {
                        MUTED
                    },
                );
                if matches!(entry, Entry::Image(_)) {
                    commands
                        .entity(info)
                        .insert(ImageInfo(index, browser.scan_revision));
                }
            }
        }
    }
    if entries.is_empty() {
        let empty = node(
            &mut commands,
            interface.grid,
            Node {
                width: percent(100),
                padding: UiRect::vertical(px(34)),
                align_items: AlignItems::Center,
                ..column(8.0)
            },
        );
        text(
            &mut commands,
            empty,
            &interface.font,
            if browser.search.is_empty() {
                "Папка пуста"
            } else {
                "Ничего не найдено"
            },
            16.0,
            TEXT,
        );
        text(
            &mut commands,
            empty,
            &interface.font,
            if browser.search.is_empty() {
                "После добавления файлов нажмите «Обновить»"
            } else {
                "Попробуйте другое название или расширение файла"
            },
            12.0,
            MUTED,
        );
    }
    if let Ok(mut label) = labels.get_mut(interface.count) {
        label.0 = browser.error.clone().unwrap_or_else(|| {
            format!("Папок: {folders}   ·   Файлов: {}", entries.len() - folders)
        });
    }
}

pub(crate) fn update_files(
    browser: Res<Browser>,
    editor: Res<Editor>,
    previews: Res<ImagePreviews>,
    interface: Res<BrowserUi>,
    cards: Query<(&FileCard, &Hovered)>,
    mut pictures: Query<(&FileImage, &mut ImageNode, &mut Node)>,
    mut placeholders: Query<(&ImagePlaceholder, &mut Visibility)>,
    mut labels: Query<(Entity, &mut Text, Option<&ImageInfo>)>,
) {
    for (placeholder, mut visibility) in &mut placeholders {
        let ready = placeholder.1 == browser.scan_revision
            && previews
                .get(placeholder.0)
                .is_some_and(|preview| preview.image.is_some());
        visibility.set_if_neq(if ready {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    for (file, mut image, mut node) in &mut pictures {
        if file.1 != browser.scan_revision {
            continue;
        }
        if let Some(preview) = previews.get(file.0)
            && let (Some(handle), Some(dimensions)) = (&preview.image, preview.dimensions)
        {
            if image.image != *handle {
                image.image = handle.clone();
            }
            let scale = (PREVIEW_WIDTH / dimensions.x.max(1) as f32)
                .min(PREVIEW_HEIGHT / dimensions.y.max(1) as f32);
            let width = px(dimensions.x as f32 * scale);
            let height = px(dimensions.y as f32 * scale);
            if node.width != width {
                node.width = width;
            }
            if node.height != height {
                node.height = height;
            }
            if node.display != Display::Flex {
                node.display = Display::Flex;
            }
        }
    }
    let selected = cards
        .iter()
        .find_map(|(card, hovered)| {
            (card.1 == browser.scan_revision && hovered.0).then_some(card.0)
        })
        .or_else(|| {
            interface
                .selected_model
                .filter(|&index| index < editor.project.catalog.len())
                .map(Entry::Model)
        })
        .or_else(|| {
            interface
                .selected
                .and_then(|index| browser.files.get(index).map(|_| Entry::File(index)))
        });
    let detail = selected
        .map(|entry| match entry {
            Entry::Folder(index) => format!("Папка · {}", browser.folders[index].path.display()),
            Entry::Model(index) => format!(
                "{} · GLB · {} · Перетащите на сцену, чтобы добавить",
                editor.project.catalog[index].asset, editor.project.catalog[index].label
            ),
            Entry::Scene { file, room } => format!(
                "{} · Сцена «{}» · Нажмите, чтобы открыть во вкладке",
                browser.files[file].path.display(),
                editor.project.scenes[room].label,
            ),
            Entry::Image(index) | Entry::File(index) => {
                let file = &browser.files[index];
                let dimensions = previews
                    .get(index)
                    .and_then(|preview| preview.dimensions)
                    .map_or(String::new(), |size| {
                        format!(" · {} × {} px", size.x, size.y)
                    });
                format!(
                    "{} · {}{}",
                    file.path.display(),
                    file_size(file.bytes),
                    dimensions
                )
            }
        })
        .unwrap_or_else(|| "Потяните верхнюю границу, чтобы изменить высоту панели".into());
    for (entity, mut text, image_info) in &mut labels {
        let value = if entity == interface.detail {
            Some(detail.clone())
        } else if let Some(info) = image_info.filter(|info| info.1 == browser.scan_revision) {
            browser.files.get(info.0).map(|file| {
                let ext = file.extension.to_uppercase();
                match previews.get(info.0) {
                    Some(preview) => match preview.dimensions {
                        Some(size) => format!("{ext} · {} × {}", size.x, size.y),
                        None if preview.error.is_some() => format!("{ext} · Нет превью"),
                        _ => format!("{ext} · Загрузка…"),
                    },
                    None => format!("{ext} · Загрузка…"),
                }
            })
        } else {
            None
        };
        if let Some(value) = value
            && text.0 != value
        {
            text.0 = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawer_slides_reversibly_and_stops_at_both_ends() {
        let mut motion = DrawerMotion::default();
        motion.advance(true, DURATION / 2.0, 350.0);
        assert!((motion.visible_height - 175.0).abs() < 0.001);
        motion.advance(false, DURATION / 4.0, 350.0);
        assert!(motion.visible_height > 0.0 && motion.visible_height < 175.0);
        motion.advance(false, DURATION, 350.0);
        assert_eq!(motion.visible_height, 0.0);
        motion.advance(true, DURATION * 2.0, 350.0);
        assert_eq!(motion.visible_height, 350.0);
    }

    #[test]
    fn drawer_blocks_scene_input_without_changing_the_render_area() {
        let window = Window {
            resolution: (1280, 760).into(),
            ..default()
        };
        let render = viewport_rect(&window);
        let mut motion = DrawerMotion::default();
        assert_eq!(scene_input_rect(&window, &motion), render);
        motion.advance(true, DURATION / 2.0, 350.0);
        let input = scene_input_rect(&window, &motion);
        assert_eq!(input.max.y, render.max.y - 175.0);
        assert!(!input.contains(Vec2::new(400.0, render.max.y - 80.0)));
        assert_eq!(viewport_rect(&window), render);
    }
}
