//! The asset browser is an overlay: sliding it never changes the scene camera.
use super::*;
use crate::data::ModelFormat;
use crate::editor::{browser::Entry, image_previews::ImagePreviews, thumbnails::Thumbnails};

#[path = "drawer_resize.rs"]
mod resize;
pub(crate) use resize::{DrawerSize, resize as resize_drawer};
pub(super) use resize::GRIP_HEIGHT;

#[path = "asset_drag.rs"]
mod drag;
pub(crate) use drag::{
    AssetDrag, DropFolder, DropTarget, pointer as drag_assets, visuals as drag_visuals,
};

const DURATION: f32 = 0.24;
const FILE_DROP_HINT: &str = "Перетащите модели и изображения сюда · ПКМ — создать папку";
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
    pub(super) paint_shelf_height: f32,
    pub(super) paint_shelf_settled: bool,
    pub resizing: bool,
}

impl DrawerMotion {
    pub(crate) fn input_rect(&self, mut rect: Rect, window_height: f32) -> Rect {
        rect.max.y = rect
            .max
            .y
            .min(window_height - FOOT - self.visible_height.max(self.paint_shelf_height))
            .max(rect.min.y);
        rect
    }

    pub(crate) fn covers(&self, point: Vec2, window: &Window) -> bool {
        Self::panel_covers(point, window, self.visible_height.max(self.paint_shelf_height))
    }

    pub(super) fn covers_files(&self, point: Vec2, window: &Window) -> bool {
        Self::panel_covers(point, window, self.visible_height)
    }

    fn panel_covers(point: Vec2, window: &Window, height: f32) -> bool {
        height > 0.0
            && point.x >= 0.0
            && point.x < window.width()
            && point.y >= window.height() - FOOT - height
            && point.y < window.height() - FOOT
    }

    pub(super) fn advance(&mut self, open: bool, delta: f32, height: f32) {
        let direction = if open { 1.0 } else { -1.0 };
        self.progress = (self.progress + direction * delta / DURATION).clamp(0.0, 1.0);
        let eased = self.progress * self.progress * (3.0 - 2.0 * self.progress);
        self.visible_height = height * eased;
    }

    pub(super) fn fully_open(&self) -> bool {
        self.progress >= 1.0
    }

    pub(super) fn position_panel(&self, mut panel: Mut<Node>, height: f32) {
        let layout = (
            px(height),
            px(FOOT - height + self.visible_height),
            if self.progress > 0.0 { Display::Flex } else { Display::None },
        );
        if (panel.height, panel.bottom, panel.display) != layout {
            (panel.height, panel.bottom, panel.display) = layout;
        }
    }
}

#[derive(Component)]
pub(crate) struct DrawerScroll;

#[derive(Component)]
pub(crate) struct AssetSearch;

#[derive(Component)]
pub(crate) struct FileCard(pub(super) Entry, pub(super) u64);

#[derive(Component)]
pub(crate) struct FileImage(usize, u64);

#[derive(Component)]
pub(crate) struct ImageInfo(usize, u64);

#[derive(Component)]
pub(crate) struct ImagePlaceholder(usize, u64);

#[derive(Resource)]
pub(crate) struct BrowserUi {
    font: Handle<Font>,
    pub(super) panel: Entity,
    tree: Entity,
    breadcrumbs: Entity,
    pub(super) grid: Entity,
    canvas: Entity,
    count: Entity,
    detail: Entity,
    placeholder: Entity,
    overlay: Entity,
    checker: Handle<Image>,
    pub(super) selected: Option<usize>,
    pub(super) selected_model: Option<usize>,
    pub(super) selected_entry: Option<Entry>,
}

impl BrowserUi {
    pub(super) fn select(&mut self, entry: Option<Entry>) {
        self.selected_entry = entry;
        self.selected = match entry {
            Some(Entry::Scene { file, .. } | Entry::Image(file) | Entry::File(file)) => Some(file),
            _ => None,
        };
        self.selected_model = match entry {
            Some(Entry::Model(index)) => Some(index),
            _ => None,
        };
    }
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

pub(super) fn library_resize_grip(commands: &mut Commands, panel: Entity) {
    let grip = resize_grip(commands, panel, "Изменить высоту библиотеки");
    commands.entity(grip).insert(resize::LibraryResizeGrip);
}

fn resize_grip(commands: &mut Commands, panel: Entity, label: &str) -> Entity {
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
        AccessibleLabel(label.into()),
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

    grip
}

pub(super) fn setup(
    commands: &mut Commands,
    root: Entity,
    font_handle: &Handle<Font>,
    stars: &starfield::Starfield,
    checker: &Handle<Image>,
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

    resize_grip(commands, panel, "Изменить высоту файлового менеджера");

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
        crate::ui_theme::text_cursor_style(),
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
    let grid = node(
        commands,
        content,
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            min_width: px(0),
            padding: UiRect::axes(px(12), px(2)),
            overflow: Overflow::scroll_y(),
            flex_direction: FlexDirection::Column,
            ..default()
        },
    );
    commands.entity(grid).insert((Scroller, DrawerScroll));
    let canvas = node(
        commands,
        grid,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..default()
        },
    );
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
        FILE_DROP_HINT,
        11.0,
        MUTED,
    );

    commands.insert_resource(BrowserUi {
        font: font_handle.clone(),
        panel,
        tree,
        breadcrumbs,
        grid,
        canvas,
        count,
        detail,
        placeholder,
        overlay: root,
        checker: checker.clone(),
        selected: None,
        selected_model: None,
        selected_entry: None,
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
    if let Ok(panel) = nodes.get_mut(interface.panel) {
        motion.position_panel(panel, height);
    }
}

pub(crate) fn actions(
    mut messages: MessageReader<Action>,
    mut browser: ResMut<Browser>,
    mut editor: ResMut<Editor>,
    pointer: Res<viewport::PointerState>,
    mut previews: ResMut<Thumbnails>,
    mut interface: ResMut<BrowserUi>,
    redirects: Option<Res<crate::asset_paths::AssetRedirects>>,
    numeric: Option<Res<NumericInput>>,
) {
    for action in messages.read() {
        if editor.pending.is_some()
            || editor.hdri_library_open
            || editor.browser_popup
            || pointer.dragging()
        {
            continue;
        }
        match *action {
            Action::OpenBrowser => {
                let folder = editor
                    .project
                    .scenes
                    .get(editor.room)
                    .and_then(|scene| {
                        let parent = std::path::Path::new(&scene.file).parent()?;
                        browser
                            .folders
                            .iter()
                            .position(|folder| folder.path == parent)
                    })
                    .unwrap_or(0);
                browser.navigate(folder);
                interface.select(None);
            }
            Action::Browse(0) => {
                browser.home();
                interface.select(None);
            }
            Action::Browse(folder) => {
                browser.navigate(folder);
                interface.select(None);
            }
            Action::BrowseBack => {
                browser.go_back();
                interface.select(None);
            }
            Action::BrowseForward => {
                browser.go_forward();
                interface.select(None);
            }
            Action::BrowseUp => {
                browser.up();
                interface.select(None);
            }
            Action::BrowseClear => {
                browser.set_search(String::new());
            }
            Action::BrowseRefresh => {
                interface.select(None);
                browser.refresh(&mut editor.project);
                previews.extend(editor.project.catalog.len());
            }
            Action::BrowseToggle(folder) => {
                browser.toggle_folder(folder);
            }
            Action::BrowseFile(file) | Action::ViewImage(file) if file < browser.files.len() => {
                interface.select(Some(
                    if browser.files[file].kind == browser::FileKind::Image {
                        Entry::Image(file)
                    } else {
                        Entry::File(file)
                    },
                ));
            }
            Action::ViewModel(model) if model < editor.project.catalog.len() => {
                interface.select(Some(Entry::Model(model)));
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
                        interface.select(None);
                        previews.extend(editor.project.catalog.len());
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
    editor: Res<Editor>,
    mut browser: ResMut<Browser>,
    focus: Res<InputFocus>,
    interface: Res<BrowserUi>,
    mut inputs: Query<(Entity, &mut EditableText), With<AssetSearch>>,
    mut nodes: Query<&mut Node>,
) {
    if editor.hdri_library_open || editor.browser_popup {
        return;
    }
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
        AssetOpen,
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
            flex_shrink: 0.0,
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
    let artwork = if extension.eq_ignore_ascii_case("dcharacter") {
        Some(icons::Glyph::CharacterFile)
    } else if extension.eq_ignore_ascii_case("dialog") {
        Some(icons::Glyph::DialogueFile)
    } else {
        None
    };
    if let Some(glyph) = artwork {
        let icon = icons::spawn(commands, parent, glyph, PREVIEW_HEIGHT * 0.9, Color::WHITE);
        commands
            .entity(icon)
            .insert(UiTransform::from_translation(Val2::percent(-50, -50)))
            .entry::<Node>()
            .and_modify(|mut node| {
                node.position_type = PositionType::Absolute;
                node.left = percent(50);
                node.top = percent(50);
            });
        return icon;
    }
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

fn file_picture(commands: &mut Commands, well: Entity, file: usize, revision: u64) {
    commands.spawn((
        FileImage(file, revision),
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
}

pub(super) fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} Б")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} КБ", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МБ", bytes as f64 / (1024.0 * 1024.0))
    }
}

const CARD_GAP: f32 = 10.0;
const ROW_HEIGHT: f32 = CARD_HEIGHT + CARD_GAP;

/// One list drives layout, thumbnail requests and scroll-to-file focus.
#[derive(Resource, Default)]
pub(crate) struct GridWindow {
    entries: Vec<Entry>,
    range: std::ops::Range<usize>,
    columns: usize,
    folders: usize,
    listing: Option<u64>,
    last_folder_revision: Option<u64>,
    pub(crate) revision: u64,
}

impl GridWindow {
    #[cfg(test)]
    pub(crate) fn for_entries(entries: Vec<Entry>) -> Self {
        Self {
            range: 0..entries.len(),
            entries,
            ..default()
        }
    }
    pub(crate) fn visible(&self) -> &[Entry] {
        &self.entries[self.range.clone()]
    }
}

pub(crate) fn update_grid(
    editor: Res<Editor>,
    browser: Res<Browser>,
    interface: Res<BrowserUi>,
    motion: Res<DrawerMotion>,
    context: Res<BrowserContext>,
    mut grid: ResMut<GridWindow>,
    mut scrolls: Query<(&ComputedNode, &mut ScrollPosition)>,
) {
    let Ok((node, mut scroll)) = scrolls.get_mut(interface.grid) else {
        return;
    };
    let open = editor.drawer || motion.visible_height > 0.0;
    if !open {
        if grid.listing.take().is_some() {
            grid.entries.clear();
            grid.range = 0..0;
            grid.revision += 1;
        }
        return;
    }
    let listing_changed = grid.listing != Some(browser.revision);
    if grid.last_folder_revision != Some(browser.revision) {
        scroll.y = 0.0;
        grid.last_folder_revision = Some(browser.revision);
    }
    if listing_changed {
        grid.entries = browser.entries(&editor.project.catalog);
        grid.folders = grid
            .entries
            .iter()
            .filter(|e| matches!(e, Entry::Folder(_)))
            .count();
        grid.listing = Some(browser.revision);
    }
    let size = node.content_box().size() * node.inverse_scale_factor();
    let columns = ((size.x + CARD_GAP) / (CARD + CARD_GAP)).floor().max(1.0) as usize;
    let rows = grid.entries.len().div_ceil(columns);
    let mut y = scroll
        .y
        .min((rows as f32 * ROW_HEIGHT - size.y).max(0.0))
        .max(0.0);
    if let Some(path) = context.focus_after.as_ref()
        && let Some(index) = grid.entries.iter().position(|&e| {
            crate::editor::file_moves::entry_path(&editor, &browser, e).is_ok_and(|p| p == *path)
        })
    {
        let top = (index / columns) as f32 * ROW_HEIGHT;
        let target = y.min(top).max(top + CARD_HEIGHT - size.y).max(0.0);
        if (target - y).abs() > 0.5 && size.y > 0.0 {
            y = target;
            scroll.y = target;
        }
    }
    let first = ((y / ROW_HEIGHT) as usize).saturating_sub(1);
    let end = (((y + size.y) / ROW_HEIGHT).ceil() as usize + 1).min(rows);
    let range = if size.x > 0.0 && size.y > 0.0 {
        (first * columns).min(grid.entries.len())..(end * columns).min(grid.entries.len())
    } else {
        0..0
    };
    if listing_changed || grid.columns != columns || grid.range != range {
        grid.columns = columns;
        grid.range = range;
        grid.revision += 1;
    }
}

#[derive(Component)]
pub(crate) struct PreviewCard(pub(crate) Entry);

pub(crate) fn refresh(
    mut commands: Commands,
    browser: Res<Browser>,
    editor: Res<Editor>,
    previews: Res<Thumbnails>,
    interface: Res<BrowserUi>,
    grid: Res<GridWindow>,
    mut window_revision: Local<Option<u64>>,
    mut cards: Local<std::collections::HashMap<usize, Entity>>,
    mut empty_state: Local<Option<Entity>>,
    mut revision: Local<Option<u64>>,
    mut tree_revision: Local<Option<(u64, u64)>>,
    mut labels: Query<&mut Text>,
) {
    // Tree entries carry scan generations for activation and drag/drop, even
    // when a file-only change leaves the folder hierarchy unchanged.
    let tree_stamp = (browser.tree_revision, browser.scan_revision);
    if *tree_revision != Some(tree_stamp) {
        *tree_revision = Some(tree_stamp);
        commands.entity(interface.tree).despawn_children();
        tree_folder(&mut commands, &interface, &browser, 0, 0);
    }
    let listing_changed = *revision != Some(browser.revision);
    if listing_changed {
        *revision = Some(browser.revision);
        commands.entity(interface.breadcrumbs).despawn_children();
        commands
            .entity(interface.grid)
            .insert(DropFolder(browser.current, browser.scan_revision));
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
    }
    if !listing_changed && *window_revision == Some(grid.revision) {
        return;
    }
    *window_revision = Some(grid.revision);
    cards.retain(|index, entity| {
        let keep = !listing_changed && grid.range.contains(index);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
    let cols = grid.columns.max(1);
    let height = (grid.entries.len().div_ceil(cols) as f32 * ROW_HEIGHT - CARD_GAP).max(0.0);
    commands
        .entity(interface.canvas)
        .entry::<Node>()
        .and_modify(move |mut n| {
            n.height = px(height);
        });
    for index in grid.range.clone() {
        let x = (index % cols) as f32 * (CARD + CARD_GAP);
        let y = (index / cols) as f32 * ROW_HEIGHT;
        let position = move |mut n: Mut<Node>| {
            n.position_type = PositionType::Absolute;
            n.left = px(x);
            n.top = px(y);
        };
        if let Some(&entity) = cards.get(&index) {
            commands.entity(entity).entry::<Node>().and_modify(position);
            continue;
        }
        let entry = grid.entries[index];
        let card = match entry {
            Entry::Folder(index) => {
                let folder = &browser.folders[index];
                let card = asset_card(&mut commands, interface.canvas, Action::Browse(index));
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
                card
            }
            Entry::Model(index) => {
                let item = &editor.project.catalog[index];
                let format = ModelFormat::from_path(&item.asset).map_or("3D", ModelFormat::label);
                let filename = item.asset.rsplit('/').next().unwrap_or(&item.asset);
                let card = asset_card(&mut commands, interface.canvas, Action::ViewModel(index));
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    AccessibleLabel(format!("Открыть модель {filename} · {}", item.label)),
                ));
                let well = preview_well(&mut commands, card);
                asset_loading::attach(
                    &mut commands,
                    well,
                    entry,
                    browser.scan_revision,
                    &interface.font,
                );
                commands.spawn((
                    PreviewCard(entry),
                    ImageNode::new(previews.images[index].clone()),
                    Pickable::IGNORE,
                    Node {
                        width: px(PREVIEW_WIDTH),
                        height: px(PREVIEW_HEIGHT),
                        flex_shrink: 0.0,
                        border_radius: BorderRadius::top(px(9)),
                        ..default()
                    },
                    ChildOf(well),
                ));
                caption(
                    &mut commands,
                    card,
                    &interface.font,
                    filename,
                    &format!("{format} · {}", item.label),
                    BLUE,
                );
                card
            }
            Entry::Scene { file, room } => {
                let file = &browser.files[file];
                let scene = &editor.project.scenes[room];
                let card = asset_card(&mut commands, interface.canvas, Action::Open(room));
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    AccessibleLabel(format!(
                        "Открыть сцену {} · {}",
                        scene.label,
                        file.path.display()
                    )),
                ));
                let well = preview_well(&mut commands, card);
                asset_loading::attach(
                    &mut commands,
                    well,
                    entry,
                    browser.scan_revision,
                    &interface.font,
                );
                commands.spawn((
                    PreviewCard(entry),
                    ImageNode::new(previews.scene_images[room].clone()),
                    Pickable::IGNORE,
                    Node {
                        width: px(PREVIEW_WIDTH),
                        height: px(PREVIEW_HEIGHT),
                        flex_shrink: 0.0,
                        border_radius: BorderRadius::top(px(9)),
                        ..default()
                    },
                    ChildOf(well),
                ));
                caption(
                    &mut commands,
                    card,
                    &interface.font,
                    &file.filename,
                    &format!("Сцена · {}", scene.label),
                    AMBER,
                );
                card
            }
            Entry::Image(index) | Entry::File(index) => {
                let file = &browser.files[index];
                let title = file.environment_label.as_deref().unwrap_or(&file.filename);
                let is_image = matches!(entry, Entry::Image(_));
                let narrative = file.extension.eq_ignore_ascii_case("dialog")
                    || file.extension.eq_ignore_ascii_case("dcharacter");
                let action = if narrative {
                    Action::Narrative(NarrativeAction::File(index))
                } else if is_image {
                    Action::ViewImage(index)
                } else {
                    Action::BrowseFile(index)
                };
                let card = asset_card(&mut commands, interface.canvas, action);
                commands.entity(card).insert((
                    FileCard(entry, browser.scan_revision),
                    AccessibleLabel(format!(
                        "{} {}",
                        if is_image {
                            "Открыть изображение"
                        } else {
                            "Выбрать файл"
                        },
                        title
                    )),
                ));
                let well = preview_well(&mut commands, card);
                let kind = if is_image {
                    asset_loading::attach(
                        &mut commands,
                        well,
                        entry,
                        browser.scan_revision,
                        &interface.font,
                    );
                    // Tiled UI images ignore border radii. Repeat the texture through
                    // UVs instead so the checker cannot cover the card's curved border.
                    commands.spawn((
                        ImageNode {
                            image: interface.checker.clone(),
                            image_mode: NodeImageMode::Stretch,
                            rect: Some(Rect::from_corners(
                                Vec2::ZERO,
                                Vec2::new(PREVIEW_WIDTH, PREVIEW_HEIGHT),
                            )),
                            ..default()
                        },
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(100),
                            height: percent(100),
                            border_radius: BorderRadius::top(px(9)),
                            ..default()
                        },
                        Pickable::IGNORE,
                        ChildOf(well),
                    ));
                    let placeholder =
                        file_icon(&mut commands, well, &interface.font, &file.extension);
                    commands.entity(placeholder).insert((
                        ImagePlaceholder(index, browser.scan_revision),
                        Visibility::Inherited,
                    ));
                    file_picture(&mut commands, well, index, browser.scan_revision);
                    if crate::editor::photo_document::source_format(&file.path).is_ok() {
                        let edit = control(
                            &mut commands,
                            well,
                            Action::EditBrowserImage(index, browser.scan_revision),
                            Look::Regular,
                            Node {
                                position_type: PositionType::Absolute,
                                right: px(5),
                                bottom: px(5),
                                width: px(28),
                                height: px(28),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border_radius: BorderRadius::all(px(6)),
                                border: UiRect::all(px(1)),
                                ..default()
                            },
                        );
                        commands.entity(edit).insert((
                            ZIndex(1),
                            AccessibleLabel(format!("Редактировать {}", file.filename)),
                        ));
                        icons::themed(&mut commands, edit, icons::Glyph::Pencil, 15.0);
                    }
                    format!("{} · Загрузка…", file.extension.to_uppercase())
                } else if file.environment_label.is_some() {
                    icons::themed(&mut commands, well, icons::Glyph::Globe, 40.0);
                    format!("Окружение · {}", file_size(file.bytes))
                } else {
                    let placeholder =
                        file_icon(&mut commands, well, &interface.font, &file.extension);
                    if file.extension.eq_ignore_ascii_case("dcharacter") {
                        commands.entity(placeholder).insert((
                            ImagePlaceholder(index, browser.scan_revision),
                            Visibility::Inherited,
                        ));
                        file_picture(&mut commands, well, index, browser.scan_revision);
                    }
                    format!(
                        "{} · {}",
                        if file.extension.eq_ignore_ascii_case("dialog") {
                            "Диалог".into()
                        } else if file.extension.eq_ignore_ascii_case("dcharacter") {
                            "Персонаж".into()
                        } else if file.extension.is_empty() {
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
                    title,
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
                card
            }
        };
        commands.entity(card).entry::<Node>().and_modify(position);
        cards.insert(index, card);
    }
    let empty_folder = grid.listing.is_some() && grid.entries.is_empty();
    if (!empty_folder || listing_changed)
        && let Some(entity) = empty_state.take()
    {
        commands.entity(entity).despawn();
    }
    if empty_folder && empty_state.is_none() {
        let empty = node(
            &mut commands,
            interface.canvas,
            Node {
                width: percent(100),
                padding: UiRect::vertical(px(34)),
                align_items: AlignItems::Center,
                ..column(8.0)
            },
        );
        *empty_state = Some(empty);
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
                FILE_DROP_HINT
            } else {
                "Попробуйте другое название или расширение файла"
            },
            12.0,
            MUTED,
        );
    }
    if let Ok(mut label) = labels.get_mut(interface.count) {
        label.0 = browser.error.clone().unwrap_or_else(|| {
            format!(
                "Папок: {}   ·   Файлов: {}",
                grid.folders,
                grid.entries.len().saturating_sub(grid.folders)
            )
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
            node.display = Display::None;
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
        } else {
            // Reveal the SVG immediately when a portrait changes or is removed.
            node.display = Display::None;
            image.image = Handle::default();
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
            Entry::Model(index) => {
                let item = &editor.project.catalog[index];
                let format = ModelFormat::from_path(&item.asset).map_or("3D", ModelFormat::label);
                format!(
                    "{} · {format} · {} · Перетащите на сцену, чтобы добавить",
                    item.asset, item.label
                )
            }
            Entry::Scene { file, room } => format!(
                "{} · Сцена «{}» · Нажмите, чтобы открыть во вкладке",
                browser.files[file].path.display(),
                editor.project.scenes[room].label,
            ),
            Entry::Image(index) | Entry::File(index) => {
                let file = &browser.files[index];
                let dimensions = previews
                    .get(index)
                    .filter(|_| matches!(entry, Entry::Image(_)))
                    .and_then(|preview| preview.dimensions)
                    .map_or(String::new(), |size| {
                        format!(" · {} × {} px", size.x, size.y)
                    });
                format!(
                    "{} · {}{}{}",
                    file.path.display(),
                    file_size(file.bytes),
                    dimensions,
                    if matches!(entry, Entry::Image(_)) {
                        " · Двойной щелчок — открыть"
                    } else {
                        ""
                    }
                )
            }
        })
        .unwrap_or_else(|| FILE_DROP_HINT.into());
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "requires a GPU adapter for the real drawer layout and scrolling"]
    fn large_folder_recycles_cards_on_scroll_and_unmounts_them_on_close() {
        use crate::editor::{
            scene_hierarchy::tests::fixture, selection_outline::render_tests::frame,
            tests::lighting_ui_render::fixture_project,
        };
        let f = fixture();
        let dir = f.root.join("many");
        std::fs::create_dir(&dir).unwrap();
        for n in 0..300 {
            std::fs::write(dir.join(format!("{n:03}.txt")), []).unwrap();
        }
        let (mut app, _, _) = fixture_project(f.project.clone());
        let folder = app
            .world()
            .resource::<Browser>()
            .folders
            .iter()
            .position(|f| f.path == std::path::Path::new("many"))
            .unwrap();
        app.world_mut().resource_mut::<Browser>().navigate(folder);
        app.world_mut().resource_mut::<Editor>().drawer = true;
        for _ in 0..20 {
            frame(&mut app);
        }
        let viewport = app.world().resource::<BrowserUi>().grid;
        let canvas = app.world().resource::<BrowserUi>().canvas;
        let cards = |app: &mut App| {
            let w = app.world_mut();
            w.query::<(&FileCard, &ChildOf)>()
                .iter(w)
                .filter(|(_, parent)| parent.parent() == canvas)
                .map(|(card, _)| card.0)
                .collect::<Vec<_>>()
        };
        let first = cards(&mut app);
        assert!(!first.is_empty() && first.len() < 60);
        app.world_mut()
            .get_mut::<ScrollPosition>(viewport)
            .unwrap()
            .y = 10_000.;
        for _ in 0..4 {
            frame(&mut app);
        }
        let last = cards(&mut app);
        assert!(!last.is_empty() && last.len() < 60);
        assert!(last.iter().all(|c| !first.contains(c)));
        let visible = app.world().resource::<GridWindow>().visible();
        assert_eq!(visible.len(), last.len());
        assert!(visible.iter().all(|entry| last.contains(entry)));
        app.world_mut().resource_mut::<Editor>().drawer = false;
        for _ in 0..30 {
            frame(&mut app);
        }
        assert!(cards(&mut app).is_empty());
    }

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
        let render = viewport_rect(&window, &SidebarLayout::default());
        let mut motion = DrawerMotion::default();
        assert_eq!(
            scene_input_rect(&window, &motion, &SidebarLayout::default()),
            render
        );
        motion.advance(true, DURATION / 2.0, 350.0);
        let input = scene_input_rect(&window, &motion, &SidebarLayout::default());
        assert_eq!(input.max.y, render.max.y - 175.0);
        assert!(!input.contains(Vec2::new(400.0, render.max.y - 80.0)));
        assert_eq!(viewport_rect(&window, &SidebarLayout::default()), render);
    }
}
