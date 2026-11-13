//! In-editor HDRI browsing. Network and disk work stay on background workers.
use super::*;
use crate::{
    HdriAsset, HdriProvider,
    editor::{
        hdri_catalog::{self, CatalogEntry, LibraryRecord, Progress},
        image_previews,
    },
};
use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    tasks::{IoTaskPool, Task, block_on, poll_once},
    window::{RequestRedraw, WindowCloseRequested},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

const PAGE: usize = 9;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum HdriAction {
    Open,
    Close,
    Provider(HdriProvider),
    Downloaded,
    Search,
    Refresh,
    Previous,
    Next,
    Resolution(u32),
    Download(u64, usize),
    Cancel,
}

#[cfg(test)]
#[path = "hdri_library_tests.rs"]
mod tests;

#[derive(Component)]
pub(super) struct DownloadedList;
#[derive(Component)]
struct PreviewSlot(String);
#[derive(Component)]
struct LibraryChoice(HdriAction);
#[derive(Component)]
struct DownloadedChoice(usize);

struct Preview {
    image: Handle<Image>,
    size: Vec2,
}
struct ThumbnailJob {
    key: String,
    task: Task<Result<image_previews::DecodedImage, String>>,
}
struct DownloadJob {
    name: String,
    progress: Arc<Progress>,
    task: Task<Result<LibraryRecord, String>>,
}
impl Drop for DownloadJob {
    fn drop(&mut self) {
        self.progress.cancel();
    }
}

#[derive(Resource)]
pub(in crate::editor) struct HdriLibrary {
    root: PathBuf,
    records: Vec<LibraryRecord>,
    available: HashSet<String>,
    scan: Option<Task<Result<Vec<LibraryRecord>, String>>>,
    catalogs: HashMap<HdriProvider, Vec<CatalogEntry>>,
    requested: HashSet<HdriProvider>,
    catalog_job: Option<(
        HdriProvider,
        Task<Result<(Vec<CatalogEntry>, String), String>>,
    )>,
    provider: Option<HdriProvider>,
    resolution: u32,
    query: String,
    page: usize,
    download: Option<DownloadJob>,
    previews: HashMap<String, Preview>,
    thumbnails: Vec<ThumbnailJob>,
    failed_previews: HashSet<String>,
    notice: String,
    revision: u64,
    records_revision: u64,
    drawn: u64,
    listed: u64,
    was_open: bool,
    selected_asset: Option<String>,
    preparing: bool,
}

impl Default for HdriLibrary {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            records: Vec::new(),
            available: HashSet::new(),
            scan: None,
            catalogs: HashMap::new(),
            requested: HashSet::new(),
            catalog_job: None,
            provider: Some(HdriProvider::PolyHaven),
            resolution: 2048,
            query: String::new(),
            page: 0,
            download: None,
            previews: HashMap::new(),
            thumbnails: Vec::new(),
            failed_previews: HashSet::new(),
            notice: String::new(),
            revision: 1,
            records_revision: 1,
            drawn: 0,
            listed: 0,
            was_open: false,
            selected_asset: None,
            preparing: false,
        }
    }
}

#[derive(Clone)]
struct Row {
    entry: CatalogEntry,
    record: Option<usize>,
}

impl HdriLibrary {
    pub fn asset(&self, index: usize) -> Option<&HdriAsset> {
        self.records
            .get(index)
            .map(|record| &record.environment)
            .filter(|asset| self.available.contains(&asset.asset))
    }

    fn downloaded(&self, entry: &CatalogEntry, resolution: u32) -> Option<usize> {
        self.records.iter().position(|record| {
            record.environment.provider == entry.provider
                && record.environment.id == entry.id
                && record.environment.resolution == resolution
                && self.available.contains(&record.environment.asset)
        })
    }

    fn rows(&self) -> Box<dyn Iterator<Item = Row> + '_> {
        match self.provider {
            Some(provider) => Box::new(
                self.catalogs
                    .get(&provider)
                    .into_iter()
                    .flatten()
                    .filter(|entry| entry.matches(&self.query))
                    .map(|entry| Row {
                        record: self.downloaded(entry, self.resolution),
                        entry: entry.clone(),
                    }),
            ),
            None => Box::new(
                self.records
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| self.available.contains(&record.environment.asset))
                    .map(|(index, record)| Row {
                        entry: CatalogEntry {
                            provider: record.environment.provider,
                            id: record.environment.id.clone(),
                            name: format!(
                                "{} · {}K",
                                record.environment.name,
                                record.environment.resolution / 1024
                            ),
                            thumbnail: String::new(),
                            tags: Vec::new(),
                            downloads: Vec::new(),
                            popularity: 0,
                        },
                        record: Some(index),
                    })
                    .filter(|row| row.entry.matches(&self.query)),
            ),
        }
    }

    fn fetch(&mut self, refresh: bool) {
        let Some(provider) = self.provider else {
            self.rescan();
            return;
        };
        if self.catalog_job.is_some() {
            return;
        }
        let root = self.root.clone();
        self.requested.insert(provider);
        self.notice.clear();
        self.catalog_job = Some((
            provider,
            IoTaskPool::get().spawn(async move { hdri_catalog::catalog(&root, provider, refresh) }),
        ));
        self.revision += 1;
    }

    fn rescan(&mut self) {
        // A scan started mid-download could return an older snapshot after the
        // newly committed record, hiding it until the next refresh.
        if self.scan.is_none() && self.download.is_none() {
            let root = self.root.clone();
            self.scan = Some(IoTaskPool::get().spawn(async move { hdri_catalog::scan(&root) }));
        }
    }

    fn merge(&mut self, record: LibraryRecord) {
        self.available.insert(record.environment.asset.clone());
        if let Some(index) = self
            .records
            .iter()
            .position(|old| old.environment.asset == record.environment.asset)
        {
            self.records[index] = record;
        } else {
            self.records.push(record);
        }
    }
}

#[derive(Resource)]
struct LibraryUi {
    overlay: Entity,
    search: Entity,
    cards: Entity,
    status: Entity,
    page: Entity,
    previous: Entity,
    next: Entity,
    refresh: Entity,
    cancel: Entity,
    downloaded_label: Entity,
    inspector: Entity,
}

pub(in crate::editor) struct HdriLibraryPlugin;
impl Plugin for HdriLibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HdriLibrary>()
            .add_systems(Startup, setup.after(super::setup))
            .add_systems(
                Update,
                (actions, poll, scroll)
                    .chain()
                    .after(super::view_images)
                    .before(crate::editor::asset_placement::update),
            )
            .add_systems(Update, refresh.after(super::refresh));
    }
}

fn library_button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: HdriAction,
) -> Entity {
    let entity = button(
        commands,
        parent,
        font,
        label,
        Action::HdriLibrary(action),
        Look::Regular,
    );
    commands.entity(entity).insert(LibraryChoice(action));
    entity
}

fn setup(
    mut commands: Commands,
    interface: Res<Interface>,
    downloaded: Query<Entity, With<DownloadedList>>,
) {
    let overlay = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.76)),
            GlobalZIndex(95),
            TabGroup::modal(),
            AccessibleLabel("Каталог HDRI".into()),
        ))
        .id();
    let panel = surface(
        &mut commands,
        overlay,
        Node {
            width: percent(92),
            max_width: px(1100),
            height: percent(92),
            max_height: px(1000),
            padding: UiRect::all(px(18)),
            border_radius: BorderRadius::all(px(10)),
            border: UiRect::all(px(1)),
            ..column(12.0)
        },
        PANEL,
    );
    let header = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
    let title = text(
        &mut commands,
        header,
        &interface.font,
        "Каталог HDRI",
        19.0,
        TEXT,
    );
    commands.entity(title).insert(Node {
        flex_grow: 1.0,
        ..default()
    });
    library_button(
        &mut commands,
        header,
        &interface.font,
        "Закрыть · Esc",
        HdriAction::Close,
    );
    let tabs = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            flex_wrap: FlexWrap::Wrap,
            flex_shrink: 0.0,
            ..row(7.0)
        },
    );
    for provider in HdriProvider::ALL {
        library_button(
            &mut commands,
            tabs,
            &interface.font,
            provider.label(),
            HdriAction::Provider(provider),
        );
    }
    let downloaded_button = control(
        &mut commands,
        tabs,
        Action::HdriLibrary(HdriAction::Downloaded),
        Look::Regular,
        Node {
            height: px(29),
            padding: UiRect::horizontal(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(5)),
            ..row(5.0)
        },
    );
    commands
        .entity(downloaded_button)
        .insert(LibraryChoice(HdriAction::Downloaded));
    let downloaded_label = text(
        &mut commands,
        downloaded_button,
        &interface.font,
        "Скачанные",
        13.0,
        TEXT,
    );
    let refresh_button = library_button(
        &mut commands,
        tabs,
        &interface.font,
        "Обновить",
        HdriAction::Refresh,
    );
    let search_row = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..row(8.0)
        },
    );
    let search = commands
        .spawn((
            TextInput,
            EditableText::new(""),
            TextCursorStyle::default(),
            font(&interface.font, 13.0),
            TextColor(TEXT),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            AccessibleLabel("Поиск HDRI".into()),
            TabIndex(0),
            ChildOf(search_row),
            Node {
                flex_grow: 1.0,
                min_width: px(80),
                height: px(32),
                padding: UiRect::axes(px(8), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(5)),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    library_button(
        &mut commands,
        search_row,
        &interface.font,
        "Найти",
        HdriAction::Search,
    );
    for size in [1024, 2048, 4096] {
        library_button(
            &mut commands,
            search_row,
            &interface.font,
            &format!("{}K", size / 1024),
            HdriAction::Resolution(size),
        );
    }
    text(
        &mut commands,
        panel,
        &interface.font,
        "Поиск по названию и тегам: forest, studio, sunset…",
        11.0,
        MUTED,
    );
    let cards = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::Start,
            align_items: AlignItems::Start,
            row_gap: px(10),
            column_gap: px(10),
            ..row(10.0)
        },
    );
    commands.entity(cards).insert(ScrollPosition::default());
    let footer = node(
        &mut commands,
        panel,
        Node {
            width: percent(100),
            flex_shrink: 0.0,
            ..row(8.0)
        },
    );
    let status = text(&mut commands, footer, &interface.font, "", 12.0, MUTED);
    commands.entity(status).insert(Node {
        flex_grow: 1.0,
        min_width: px(0),
        ..default()
    });
    let cancel = library_button(
        &mut commands,
        footer,
        &interface.font,
        "Отменить загрузку",
        HdriAction::Cancel,
    );
    let previous = library_button(
        &mut commands,
        footer,
        &interface.font,
        "←",
        HdriAction::Previous,
    );
    let page = text(&mut commands, footer, &interface.font, "1 / 1", 12.0, TEXT);
    let next = library_button(
        &mut commands,
        footer,
        &interface.font,
        "→",
        HdriAction::Next,
    );
    text(
        &mut commands,
        panel,
        &interface.font,
        "Источники: Poly Haven · ambientCG · CC0. Карты сохраняются в папке hdri проекта.",
        11.0,
        MUTED,
    );
    let inspector = downloaded.single().expect("HDRI inspector list is missing");
    commands.insert_resource(LibraryUi {
        overlay,
        search,
        cards,
        status,
        page,
        previous,
        next,
        refresh: refresh_button,
        cancel,
        downloaded_label,
        inspector,
    });
}

fn actions(
    mut events: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut library: ResMut<HdriLibrary>,
    ui: Res<LibraryUi>,
    mut focus: ResMut<InputFocus>,
    input: Query<&EditableText>,
    numeric: Res<NumericInput>,
    mut close: MessageReader<WindowCloseRequested>,
) {
    if library.root != editor.root {
        *library = HdriLibrary {
            root: editor.root.clone(),
            ..default()
        };
        library.rescan();
    }
    if close.read().next().is_some() {
        editor.hdri_library_open = false;
        if let Some(job) = &library.download {
            job.progress.cancel();
        }
    }
    for event in events.read() {
        let Action::HdriLibrary(action) = *event else {
            continue;
        };
        if editor.pending.is_some() {
            continue;
        }
        if matches!(action, HdriAction::Open) {
            if !editor.has_scene()
                || !editor.ready
                || editor.asset_viewer_open()
                || editor.browser_popup
            {
                continue;
            }
            if let Some(error) = numeric.error_message() {
                editor.status = error.into();
                continue;
            }
            editor.finish_camera_navigation();
            editor.hdri_library_open = true;
            library.rescan();
            if library
                .provider
                .is_some_and(|provider| !library.requested.contains(&provider))
            {
                library.fetch(false);
            }
            continue;
        }
        if !editor.hdri_library_open {
            continue;
        }
        match action {
            HdriAction::Close => {
                editor.hdri_library_open = false;
                focus.clear();
            }
            HdriAction::Provider(provider) => {
                library.provider = Some(provider);
                library.page = 0;
                library.notice.clear();
                library.revision += 1;
                if !library.requested.contains(&provider) {
                    library.fetch(false);
                }
            }
            HdriAction::Downloaded => {
                library.provider = None;
                library.page = 0;
                library.notice.clear();
                library.revision += 1;
            }
            HdriAction::Search => {
                library.query = input
                    .get(ui.search)
                    .map(|input| input.value().to_string().chars().take(128).collect())
                    .unwrap_or_default();
                library.page = 0;
                library.revision += 1;
            }
            HdriAction::Refresh => {
                library.failed_previews.clear();
                library.fetch(true);
            }
            HdriAction::Previous => {
                library.page = library.page.saturating_sub(1);
                library.revision += 1;
            }
            HdriAction::Next => {
                let pages = library.rows().count().div_ceil(PAGE).max(1);
                library.page = (library.page + 1).min(pages - 1);
                library.revision += 1;
            }
            HdriAction::Resolution(size) if [1024, 2048, 4096].contains(&size) => {
                library.resolution = size;
                library.revision += 1;
            }
            HdriAction::Download(revision, index)
                if revision == library.revision
                    && library.download.is_none()
                    && library.scan.is_none() =>
            {
                let Some(row) = library.rows().nth(library.page * PAGE + index) else {
                    continue;
                };
                if row.record.is_some() {
                    continue;
                }
                let entry = row.entry.clone();
                let root = library.root.clone();
                let resolution = library.resolution;
                let progress = Arc::new(Progress::default());
                let worker = Arc::clone(&progress);
                let name = entry.name.clone();
                library.download = Some(DownloadJob {
                    name,
                    progress,
                    task: IoTaskPool::get().spawn(async move {
                        hdri_catalog::download(&root, &entry, resolution, worker)
                    }),
                });
                library.notice.clear();
                library.revision += 1;
            }
            HdriAction::Cancel => {
                if let Some(job) = &library.download {
                    job.progress.cancel();
                }
            }
            _ => {}
        }
    }
}

fn poll(
    mut library: ResMut<HdriLibrary>,
    mut editor: ResMut<Editor>,
    mut images: ResMut<Assets<Image>>,
    load_status: Res<crate::hdri::HdriLoadStatus>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    if let Some(result) = library
        .scan
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        library.scan = None;
        match result {
            Ok(records) => {
                library.available.clear();
                for record in records {
                    library.merge(record);
                }
                library.records_revision += 1;
                library.revision += 1;
            }
            Err(error) => library.notice = error,
        }
    }
    if let Some(result) = library
        .catalog_job
        .as_mut()
        .and_then(|(_, task)| block_on(poll_once(task)))
    {
        let (provider, _) = library.catalog_job.take().unwrap();
        match result {
            Ok((entries, notice)) => {
                library.catalogs.insert(provider, entries);
                library.notice = notice;
            }
            Err(error) => {
                library.notice = format!("Каталог недоступен. Нажмите «Обновить»: {error}")
            }
        }
        library.page = 0;
        library.revision += 1;
    }
    if editor.hdri_library_open
        && library.catalog_job.is_none()
        && library
            .provider
            .is_some_and(|provider| !library.requested.contains(&provider))
    {
        library.fetch(false);
    }
    if let Some(result) = library
        .download
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)))
    {
        library.download = None;
        match result {
            Ok(record) => {
                library.notice = format!("Добавлено в список: {}", record.environment.name);
                editor.status = format!("HDRI скачана · {}", record.environment.name);
                library.merge(record);
                library.records_revision += 1;
            }
            Err(error) => library.notice = error,
        }
        library.revision += 1;
    }
    let mut index = 0;
    while index < library.thumbnails.len() {
        let Some(result) = block_on(poll_once(&mut library.thumbnails[index].task)) else {
            index += 1;
            continue;
        };
        let job = library.thumbnails.remove(index);
        match result {
            Ok(decoded) => {
                let size = decoded.size.as_vec2();
                let handle = image_previews::texture(decoded, &mut images);
                library.previews.insert(
                    job.key,
                    Preview {
                        image: handle,
                        size,
                    },
                );
            }
            Err(_) => {
                library.failed_previews.insert(job.key);
            }
        }
    }
    if editor.hdri_library_open {
        let rows = library.rows();
        let visible: Vec<_> = rows
            .into_iter()
            .skip(library.page * PAGE)
            .take(PAGE)
            .collect();
        if library.previews.len() > 64 {
            let keys: HashSet<_> = visible.iter().map(|row| row.entry.key()).collect();
            library.previews.retain(|key, _| keys.contains(key));
        }
        for row in visible {
            let key = row.entry.key();
            if library.thumbnails.len() >= 3 {
                break;
            }
            if library.previews.contains_key(&key)
                || library.failed_previews.contains(&key)
                || library.thumbnails.iter().any(|job| job.key == key)
            {
                continue;
            }
            let root = library.root.clone();
            let local = row
                .record
                .and_then(|index| library.records.get(index))
                .map(|record| root.join(&record.preview));
            let entry = row.entry;
            library.thumbnails.push(ThumbnailJob {
                key,
                task: IoTaskPool::get().spawn(async move {
                    if let Some(path) = local
                        && path.is_file()
                    {
                        image_previews::decode_image(&path, UVec2::new(256, 160))
                    } else {
                        hdri_catalog::thumbnail(&root, &entry)
                    }
                }),
            });
        }
    }
    let selected = editor
        .environment()
        .hdri_asset
        .as_ref()
        .map(|asset| asset.asset.clone());
    if selected != library.selected_asset {
        library.selected_asset = selected.clone();
        library.preparing = selected.is_some();
    }
    if library.preparing
        && let Some(path) = selected
        && let Some(status) = load_status.0.get(&path)
    {
        match status {
            Ok(true) => {
                editor.status = "HDRI готова · фон, свет и отражения обновлены".into();
                library.preparing = false;
            }
            Err(error) => {
                editor.status = format!("Не удалось подготовить HDRI · {error}");
                library.preparing = false;
            }
            _ => {}
        }
    }
    if library.scan.is_some()
        || library.catalog_job.is_some()
        || library.download.is_some()
        || !library.thumbnails.is_empty()
        || library.preparing
    {
        redraw.write(RequestRedraw);
    }
}

fn scroll(
    editor: Res<Editor>,
    ui: Res<LibraryUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut area: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition)>,
) {
    if !editor.hdri_library_open || scroll.delta == Vec2::ZERO {
        return;
    }
    let Some(cursor) = window
        .cursor_position()
        .map(|point| point * window.scale_factor())
    else {
        return;
    };
    let Ok((computed, transform, mut offset)) = area.get_mut(ui.cards) else {
        return;
    };
    if (cursor - transform.affine().translation)
        .abs()
        .cmpgt(computed.size() * 0.5)
        .any()
    {
        return;
    }
    let scale = if scroll.unit == MouseScrollUnit::Line {
        33.0
    } else {
        1.0
    };
    let max = ((computed.content_size().y - computed.size().y) * computed.inverse_scale_factor())
        .max(0.0);
    offset.y = (offset.y - scroll.delta.y * scale).clamp(0.0, max);
}

fn draw_cards(commands: &mut Commands, library: &HdriLibrary, ui: &LibraryUi, font: &Handle<Font>) {
    commands
        .entity(ui.cards)
        .despawn_related::<Children>()
        .insert(ScrollPosition::default());
    for (index, row) in library
        .rows()
        .into_iter()
        .skip(library.page * PAGE)
        .take(PAGE)
        .enumerate()
    {
        let card = surface(
            commands,
            ui.cards,
            Node {
                width: percent(31.5),
                min_width: px(190),
                flex_grow: 1.0,
                padding: UiRect::all(px(9)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                overflow: Overflow::clip(),
                ..column(6.0)
            },
            WELL,
        );
        let preview = surface(
            commands,
            card,
            Node {
                width: percent(100),
                height: px(112),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            Color::srgb(0.025, 0.03, 0.04),
        );
        commands.spawn((
            ImageNode::new(Handle::default()),
            PreviewSlot(row.entry.key()),
            Node {
                width: px(1),
                height: px(1),
                ..default()
            },
            ChildOf(preview),
        ));
        let title = text(commands, card, font, &row.entry.name, 12.0, TEXT);
        commands.entity(title).insert(Node {
            height: px(28),
            max_width: percent(100),
            overflow: Overflow::clip(),
            ..default()
        });
        if let Some(record) = row.record {
            let choice = button(
                commands,
                card,
                font,
                "Использовать",
                Action::UseDownloadedHdri(record),
                Look::Primary,
            );
            commands.entity(choice).insert(DownloadedChoice(record));
        } else {
            library_button(
                commands,
                card,
                font,
                &format!("Скачать · {}K", library.resolution / 1024),
                HdriAction::Download(library.revision, index),
            );
        }
    }
}

fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    mut library: ResMut<HdriLibrary>,
    ui: Res<LibraryUi>,
    interface: Res<Interface>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut previews: Query<(Entity, &PreviewSlot, &mut ImageNode)>,
    choices: Query<
        (Entity, Option<&LibraryChoice>, Option<&DownloadedChoice>),
        Or<(With<LibraryChoice>, With<DownloadedChoice>)>,
    >,
    mut colors: Query<&mut BackgroundColor>,
    mut focus: ResMut<InputFocus>,
) {
    if let Ok(mut overlay) = nodes.get_mut(ui.overlay) {
        overlay.display = if editor.hdri_library_open {
            Display::Flex
        } else {
            Display::None
        };
    }
    if editor.hdri_library_open && !library.was_open {
        focus.set(ui.search, bevy::input_focus::FocusCause::Auto);
    } else if !editor.hdri_library_open && library.was_open {
        focus.clear();
    }
    library.was_open = editor.hdri_library_open;
    if editor.hdri_library_open && library.drawn != library.revision {
        draw_cards(&mut commands, &library, &ui, &interface.font);
        library.drawn = library.revision;
    }
    if library.listed != library.records_revision {
        commands.entity(ui.inspector).despawn_related::<Children>();
        if !library.available.is_empty() {
            text(
                &mut commands,
                ui.inspector,
                &interface.font,
                "СКАЧАННЫЕ ОКРУЖЕНИЯ",
                10.0,
                MUTED,
            );
        }
        for (index, record) in library
            .records
            .iter()
            .enumerate()
            .filter(|(_, record)| library.available.contains(&record.environment.asset))
        {
            let choice = button(
                &mut commands,
                ui.inspector,
                &interface.font,
                &format!(
                    "{} · {}K",
                    record.environment.name,
                    record.environment.resolution / 1024
                ),
                Action::UseDownloadedHdri(index),
                Look::Regular,
            );
            commands.entity(choice).insert((
                DownloadedChoice(index),
                Node {
                    width: percent(100),
                    min_height: px(29),
                    padding: UiRect::axes(px(8), px(5)),
                    border: UiRect::all(px(1)),
                    overflow: Overflow::clip(),
                    ..row(5.0)
                },
            ));
        }
        library.listed = library.records_revision;
    }
    for (entity, slot, mut image) in &mut previews {
        if let Some(preview) = library.previews.get(&slot.0) {
            image.image = preview.image.clone();
            if let Ok(mut node) = nodes.get_mut(entity) {
                let scale = (108.0 / preview.size.y).min(1.0);
                node.width = px(preview.size.x * scale);
                node.height = px(preview.size.y * scale);
                node.max_width = percent(100);
            }
        }
    }
    for (entity, choice, downloaded) in &choices {
        let active = if let Some(choice) = choice {
            match choice.0 {
                HdriAction::Provider(provider) => library.provider == Some(provider),
                HdriAction::Downloaded => library.provider.is_none(),
                HdriAction::Resolution(size) => library.resolution == size,
                _ => false,
            }
        } else if let Some(choice) = downloaded {
            editor
                .environment()
                .hdri_asset
                .as_ref()
                .is_some_and(|asset| {
                    library
                        .asset(choice.0)
                        .is_some_and(|candidate| candidate.asset == asset.asset)
                })
        } else {
            false
        };
        if active && let Ok(mut color) = colors.get_mut(entity) {
            *color = BackgroundColor(ACTIVE);
        }
        if let Some(choice) = choice
            && matches!(choice.0, HdriAction::Download(..))
        {
            if library.download.is_some() || library.scan.is_some() {
                commands
                    .entity(entity)
                    .try_insert((InteractionDisabled, TabIndex(-1)));
            }
        }
    }
    let total = library.rows().count();
    let pages = total.div_ceil(PAGE).max(1);
    if let Ok(mut label) = labels.get_mut(ui.page) {
        label.0 = format!("{} / {}", library.page + 1, pages);
    }
    if let Ok(mut label) = labels.get_mut(ui.downloaded_label) {
        label.0 = format!("Скачанные ({})", library.available.len());
    }
    let status = if let Some(job) = &library.download {
        let received = job.progress.received.load(Ordering::Relaxed) as f64 / 1048576.0;
        let total = job.progress.total.load(Ordering::Relaxed) as f64 / 1048576.0;
        match job.progress.phase.load(Ordering::Relaxed) {
            0 => format!("Получаю ссылку · {}", job.name),
            1 => format!("{} · {:.1} / {:.1} МБ", job.name, received, total),
            2 => "Распаковка HDRI…".into(),
            3 => "Проверка HDRI…".into(),
            _ => "Добавление в список…".into(),
        }
    } else if !library.notice.is_empty() {
        library.notice.clone()
    } else if library.catalog_job.is_some() {
        "Загрузка каталога…".into()
    } else if library.scan.is_some() {
        "Чтение скачанных карт…".into()
    } else if total == 0 {
        "Ничего не найдено. Измените поиск или выберите другой источник.".into()
    } else {
        format!("Найдено: {total} · выберите карту и разрешение")
    };
    if let Ok(mut label) = labels.get_mut(ui.status) {
        label.0 = status;
    }
    if let Ok(mut cancel) = nodes.get_mut(ui.cancel) {
        cancel.display = if library.download.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (entity, disabled) in [
        (ui.previous, library.page == 0),
        (ui.next, library.page + 1 >= pages),
        (ui.refresh, library.catalog_job.is_some()),
    ] {
        if disabled {
            commands
                .entity(entity)
                .try_insert((InteractionDisabled, TabIndex(-1)));
        }
    }
}
