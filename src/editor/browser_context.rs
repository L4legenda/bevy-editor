//! Native file drops and the asset browser's folder-creation menu.
use super::*;
#[path = "browser_delete.rs"]
mod deletion;
#[path = "browser_image.rs"]
mod image_creation;
use crate::editor::{browser_files, thumbnails::Thumbnails};
use bevy::{
    input_focus::FocusCause,
    tasks::{IoTaskPool, Task, block_on, poll_once},
    text::TextEdit,
    ui::CalculatedClip,
    window::{FileDragAndDrop, RequestRedraw},
    winit::WinitWindows,
};
pub(in crate::editor) use deletion::BrowserDeletionPlugin;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Closed,
    Menu,
    Folder,
    Image,
}

enum Work {
    Import(Vec<PathBuf>),
    Image {
        name: String,
        width: u32,
        height: u32,
    },
}

struct Batch {
    parent: PathBuf,
    revision: u64,
    work: Work,
}
struct Outcome {
    report: browser_files::ImportReport,
    status: String,
}

impl Work {
    fn run(self, root: PathBuf, parent: PathBuf) -> Outcome {
        match self {
            Self::Import(paths) => {
                let report = browser_files::import_images(&root, &parent, paths);
                let status = report.status();
                Outcome { report, status }
            }
            Self::Image {
                name,
                width,
                height,
            } => match browser_files::create_image(&root, &parent, &name, width, height) {
                Ok(path) => {
                    let status = format!(
                        "Создано изображение «{}» · {width} × {height}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    Outcome {
                        report: browser_files::ImportReport {
                            imported: vec![path],
                            ..default()
                        },
                        status,
                    }
                }
                Err(error) => Outcome {
                    report: browser_files::ImportReport {
                        errors: vec![error.clone()],
                        ..default()
                    },
                    status: error,
                },
            },
        }
    }
}
struct Import {
    parent: PathBuf,
    revision: u64,
    task: Task<Outcome>,
}
struct Completed {
    parent: PathBuf,
    revision: u64,
    report: browser_files::ImportReport,
    status: String,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct BrowserContext {
    phase: Phase,
    parent: PathBuf,
    anchor: Vec2,
    error: String,
    previous_focus: Option<Entity>,
    focus_after: Option<PathBuf>,
    suppress_release: bool,
    hovered: HashSet<PathBuf>,
    target: Option<(usize, Rect)>,
    queue: VecDeque<Batch>,
    pending: Option<Import>,
    completed: Option<Completed>,
}

impl BrowserContext {
    #[cfg(test)]
    pub(super) fn folder_dialog(&self) -> bool {
        self.phase == Phase::Folder
    }
}

#[derive(Resource)]
pub(in crate::editor) struct ContextUi {
    overlay: Entity,
    menu: Entity,
    menu_title: Entity,
    new_folder: Entity,
    dialog: Entity,
    dialog_title: Entity,
    name_label: Entity,
    size: image_creation::SizeUi,
    submit: Entity,
    parent: Entity,
    name: Entity,
    error: Entity,
    indicator: Entity,
    drop_hint: Entity,
    drop_label: Entity,
    phase: Phase,
}

pub(super) fn setup(commands: &mut Commands, root: Entity, font_handle: &Handle<Font>) {
    let overlay = node(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    );
    commands.entity(overlay).insert((
        GlobalZIndex(70),
        TabGroup::modal(),
        BackgroundColor(Color::NONE),
    ));
    let menu = surface(
        commands,
        overlay,
        Node {
            position_type: PositionType::Absolute,
            width: px(256),
            padding: UiRect::all(px(6)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..column(6.0)
        },
        PANEL,
    );
    commands
        .entity(menu)
        .insert(AccessibleLabel("Меню файлового менеджера".into()));
    let menu_title = text(commands, menu, font_handle, "", 11.0, MUTED);
    commands.entity(menu_title).insert(Node {
        width: percent(100),
        padding: UiRect::axes(px(7), px(4)),
        overflow: Overflow::clip(),
        ..default()
    });
    let new_folder = button(
        commands,
        menu,
        font_handle,
        "Создать папку",
        Action::BrowserNewFolder,
        Look::Regular,
    );
    commands
        .entity(new_folder)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = percent(100);
            node.height = px(34);
        });
    let icon = icons::themed(commands, new_folder, icons::Glyph::Folder, 16.0);
    commands.entity(new_folder).insert_children(0, &[icon]);
    node(
        commands,
        new_folder,
        Node {
            flex_grow: 1.0,
            ..default()
        },
    );
    text(
        commands,
        new_folder,
        font_handle,
        "Ctrl+Shift+N",
        10.0,
        MUTED,
    );

    let new_image = button(
        commands,
        menu,
        font_handle,
        "Создать изображение",
        Action::BrowserNewImage,
        Look::Regular,
    );
    commands
        .entity(new_image)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = percent(100);
            node.height = px(34);
        });
    let icon = icons::themed(commands, new_image, icons::Glyph::Image, 16.0);
    commands.entity(new_image).insert_children(0, &[icon]);

    let dialog = surface(
        commands,
        overlay,
        Node {
            width: px(440),
            max_width: percent(90),
            padding: UiRect::all(px(22)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            ..column(13.0)
        },
        PANEL,
    );
    commands
        .entity(dialog)
        .insert(AccessibleLabel("Создание папки".into()));
    let dialog_title = text(commands, dialog, font_handle, "Создать папку", 21.0, TEXT);
    let parent = text(commands, dialog, font_handle, "", 12.0, MUTED);
    commands.entity(parent).insert((
        TextLayout::default(),
        Node {
            max_width: percent(100),
            ..default()
        },
    ));
    let name_label = text(commands, dialog, font_handle, "Название папки", 12.0, TEXT);
    let name = commands
        .spawn((
            TextInput,
            EditableText::new(""),
            TextCursorStyle::default(),
            AccessibleLabel("Название новой папки".into()),
            font(font_handle, 14.0),
            TextColor(TEXT),
            TextLayout::no_wrap(),
            TabIndex(0),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                width: percent(100),
                height: px(36),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(9), px(7)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(dialog),
        ))
        .id();
    let size = image_creation::setup(commands, dialog, font_handle);
    let error = text(commands, dialog, font_handle, "", 12.0, ERROR);
    commands.entity(error).insert(TextLayout::default());
    let actions = node(
        commands,
        dialog,
        Node {
            justify_content: JustifyContent::End,
            ..row(8.0)
        },
    );
    button(
        commands,
        actions,
        font_handle,
        "Отмена",
        Action::BrowserCancelPopup,
        Look::Regular,
    );
    let submit = button(
        commands,
        actions,
        font_handle,
        "Создать",
        Action::BrowserCreateFolder,
        Look::Primary,
    );

    let indicator = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BLUE.with_alpha(0.08),
    );
    commands
        .entity(indicator)
        .insert((GlobalZIndex(55), Pickable::IGNORE, BorderColor::all(BLUE)));
    let drop_hint = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: percent(50),
            max_width: percent(85),
            padding: UiRect::axes(px(15), px(11)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(8)),
            ..column(4.0)
        },
        CHROME,
    );
    commands.entity(drop_hint).insert((
        GlobalZIndex(60),
        Pickable::IGNORE,
        UiTransform::from_translation(Val2::percent(-50, 0)),
        BorderColor::all(BLUE),
    ));
    let drop_label = text(commands, drop_hint, font_handle, "", 14.0, TEXT);
    text(
        commands,
        drop_hint,
        font_handle,
        "Отпустите файлы, чтобы добавить копии в проект",
        11.0,
        MUTED,
    );
    commands.insert_resource(ContextUi {
        overlay,
        menu,
        menu_title,
        new_folder,
        dialog,
        dialog_title,
        name_label,
        size,
        submit,
        parent,
        name,
        error,
        indicator,
        drop_hint,
        drop_label,
        phase: Phase::Closed,
    });
}

type Geometry<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ComputedNode,
        &'static UiGlobalTransform,
        Option<&'static CalculatedClip>,
    ),
>;

fn hit(geometry: &Geometry, entity: Entity, point: Vec2) -> bool {
    geometry
        .get(entity)
        .is_ok_and(|(_, node, transform, clip)| {
            !node.is_empty()
                && node.contains_point(*transform, point)
                && clip.is_none_or(|clip| clip.contains_point(point))
        })
}

fn bounds(geometry: &Geometry, entity: Entity) -> Option<Rect> {
    let (_, node, transform, _) = geometry.get(entity).ok()?;
    if node.is_empty() {
        return None;
    }
    let center = transform.affine().translation;
    Some(Rect::from_center_size(center, node.size()))
}

fn destination(
    browser: &Browser,
    geometry: &Geometry,
    folders: &Query<&drawer::DropFolder>,
    point: Vec2,
) -> Option<(usize, Rect)> {
    geometry
        .iter()
        .filter_map(|(entity, node, _, _)| {
            let folder = folders.get(entity).ok()?;
            (folder.1 == browser.scan_revision && hit(geometry, entity, point)).then(|| {
                (
                    folder.0,
                    bounds(geometry, entity).unwrap(),
                    node.size().element_product(),
                )
            })
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(folder, rect, _)| (folder, rect))
}

// OLE file dragging on Windows does not send CursorMoved, and Winit drops the
// coordinates attached to native drag events. Query the current client position
// only for external drags; headless tests and other platforms use Bevy's cursor.
fn drop_cursor(entity: Entity, window: &Window, windows: Option<&WinitWindows>) -> Option<Vec2> {
    #[cfg(target_os = "windows")]
    if let Some(native) = windows.and_then(|windows| windows.get_window(entity))
        && let Ok(origin) = native.inner_position()
    {
        #[repr(C)]
        struct Point {
            x: i32,
            y: i32,
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetCursorPos(point: *mut Point) -> i32;
        }
        let mut point = Point { x: 0, y: 0 };
        // SAFETY: GetCursorPos writes one initialized Win32 POINT; no retained pointers.
        if unsafe { GetCursorPos(&mut point) } != 0 {
            return Some(
                Vec2::new((point.x - origin.x) as f32, (point.y - origin.y) as f32)
                    / window.scale_factor(),
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (entity, windows);
    window.cursor_position()
}

pub(in crate::editor) fn input(
    mut editor: ResMut<Editor>,
    browser: Res<Browser>,
    mut context: ResMut<BrowserContext>,
    interfaces: (Res<ContextUi>, Res<drawer::BrowserUi>),
    motion: Res<DrawerMotion>,
    window: Single<(Entity, &Window), With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    drag: Res<AssetDrag>,
    focus: Res<InputFocus>,
    mut drops: MessageReader<FileDragAndDrop>,
    geometry: Geometry,
    folders: Query<&drawer::DropFolder>,
    inputs: Query<(), With<TextInput>>,
    windows: Option<NonSend<WinitWindows>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let (entity, window) = *window;
    let (interface, browser_ui) = interfaces;
    context.suppress_release &=
        mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left);
    let allowed = editor.drawer
        && editor.pending.is_none()
        && !editor.hdri_library_open
        && !motion.resizing
        && !editor.sidebar.resizing()
        && !drag.active();
    if !allowed {
        context.phase = Phase::Closed;
        context.hovered.clear();
    }
    if allowed && window.focused {
        if mouse.just_pressed(MouseButton::Right)
            && !matches!(context.phase, Phase::Folder | Phase::Image)
            && let Some(point) = window
                .cursor_position()
                .map(|point| point * window.scale_factor())
            && hit(&geometry, browser_ui.panel, point)
            && !geometry
                .iter()
                .any(|(entity, _, _, _)| inputs.contains(entity) && hit(&geometry, entity, point))
        {
            let folder = destination(&browser, &geometry, &folders, point)
                .map_or(browser.current, |(folder, _)| folder);
            if context.phase == Phase::Closed {
                context.previous_focus = focus.get();
            }
            context.parent = browser.folders[folder].path.clone();
            context.anchor = point / window.scale_factor();
            context.phase = Phase::Menu;
        }
        if context.phase == Phase::Menu
            && mouse.just_pressed(MouseButton::Left)
            && window
                .cursor_position()
                .is_none_or(|point| !hit(&geometry, interface.menu, point * window.scale_factor()))
        {
            context.phase = Phase::Closed;
            context.suppress_release = true;
        }
    } else if !window.focused && context.phase == Phase::Menu {
        context.phase = Phase::Closed;
    }

    let mut paths = Vec::new();
    for event in drops.read() {
        match event {
            FileDragAndDrop::HoveredFile { window, path_buf } if *window == entity => {
                context.hovered.insert(path_buf.clone());
            }
            FileDragAndDrop::DroppedFile { window, path_buf } if *window == entity => {
                paths.push(path_buf.clone())
            }
            FileDragAndDrop::HoveredFileCanceled { window } if *window == entity => {
                context.hovered.clear()
            }
            _ => {}
        }
    }
    let received = !paths.is_empty();
    context.target = None;
    if allowed
        && context.phase == Phase::Closed
        && (!context.hovered.is_empty() || !paths.is_empty())
    {
        let point = drop_cursor(entity, window, windows.as_deref())
            .map(|point| point * window.scale_factor());
        context.target = point.and_then(|point| {
            if !hit(&geometry, browser_ui.panel, point) {
                return None;
            }
            destination(&browser, &geometry, &folders, point)
                .or_else(|| bounds(&geometry, browser_ui.grid).map(|rect| (browser.current, rect)))
        });
        if !paths.is_empty() {
            if let Some((folder, _)) = context.target {
                context.queue.push_back(Batch {
                    parent: browser.folders[folder].path.clone(),
                    revision: browser.revision,
                    work: Work::Import(paths),
                });
            } else {
                editor.status =
                    "Перетащите изображения в папку или свободное место файлового менеджера".into();
            }
            context.hovered.clear();
            context.target = None;
        }
    }
    if received {
        if !allowed || context.phase != Phase::Closed {
            editor.status = "Закройте открытое меню или просмотр и перетащите изображения в папку файлового менеджера".into();
        }
        context.hovered.clear();
        context.target = None;
    }
    if !context.hovered.is_empty() {
        redraw.write(RequestRedraw);
    }
    editor.browser_popup = (context.phase != Phase::Closed || context.suppress_release)
        && editor.pending.is_none()
        && !editor.hdri_library_open;
}

pub(super) fn shortcuts(
    keys: &ButtonInput<KeyCode>,
    context: Option<&BrowserContext>,
    actions: &mut MessageWriter<Action>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        actions.write(Action::BrowserCancelPopup);
    } else if keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        match context.map(|context| context.phase) {
            Some(Phase::Folder) => {
                actions.write(Action::BrowserCreateFolder);
            }
            Some(Phase::Image) => {
                actions.write(Action::BrowserCreateImage);
            }
            _ => {}
        }
    }
}

pub(in crate::editor) fn update(
    mut actions: MessageReader<Action>,
    mut context: ResMut<BrowserContext>,
    interface: Res<ContextUi>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    mut browser_ui: ResMut<drawer::BrowserUi>,
    mut previews: ResMut<Thumbnails>,
    mut images: ResMut<Assets<Image>>,
    mut focus: ResMut<InputFocus>,
    mut inputs: Query<&mut EditableText>,
    mut last_name: Local<String>,
) {
    let allowed = editor.drawer && editor.pending.is_none() && !editor.hdri_library_open;
    if matches!(context.phase, Phase::Folder | Phase::Image)
        && let Ok(input) = inputs.get(interface.name)
    {
        let name = format!("{}:{}", input.value(), interface.size.signature(&inputs));
        if *last_name != name {
            context.error.clear();
            *last_name = name;
        }
    }
    for action in actions.read() {
        match *action {
            Action::BrowserNewFolder | Action::BrowserNewImage if allowed => {
                if context.phase == Phase::Closed {
                    context.parent = browser.folders[browser.current].path.clone();
                    context.previous_focus = focus.get();
                }
                let image = matches!(action, Action::BrowserNewImage);
                context.phase = if image { Phase::Image } else { Phase::Folder };
                context.error.clear();
                if let Ok(mut input) = inputs.get_mut(interface.name) {
                    let name = if image {
                        browser_files::suggested_image_name(&editor.root, &context.parent)
                    } else {
                        browser_files::suggested_name(&editor.root, &context.parent)
                    };
                    input.editor_mut().set_text(&name);
                    input.queue_edit(TextEdit::SelectAll);
                }
                if image {
                    interface.size.set(&mut inputs, 1024, 1024);
                    if let Ok(mut input) = inputs.get_mut(interface.size.width) {
                        input.queue_edit(TextEdit::SelectAll);
                    }
                }
            }
            Action::BrowserImageSize(width, height) if allowed && context.phase == Phase::Image => {
                interface.size.set(&mut inputs, width, height);
                context.error.clear();
            }
            Action::BrowserCreateImage if allowed && context.phase == Phase::Image => {
                let name = inputs
                    .get(interface.name)
                    .map(|input| input.value().to_string())
                    .unwrap_or_default();
                let result =
                    browser_files::image_stem(&name).and_then(|_| interface.size.read(&inputs));
                match result {
                    Ok((width, height)) => {
                        if let Some(parent) = browser
                            .folders
                            .iter()
                            .position(|folder| folder.path == context.parent)
                        {
                            browser.navigate(parent);
                        }
                        browser.set_search(String::new());
                        let batch = Batch {
                            parent: context.parent.clone(),
                            revision: browser.revision,
                            work: Work::Image {
                                name,
                                width,
                                height,
                            },
                        };
                        context.queue.push_back(batch);
                        context.phase = Phase::Closed;
                        context.error.clear();
                    }
                    Err(error) => {
                        context.error = error;
                    }
                }
            }
            Action::BrowserCancelPopup => {
                context.phase = Phase::Closed;
                context.error.clear();
            }
            Action::BrowserCreateFolder if allowed && context.phase == Phase::Folder => {
                let name = inputs
                    .get(interface.name)
                    .map(|input| input.value().to_string())
                    .unwrap_or_default();
                match browser_files::create_folder(&editor.root, &context.parent, &name) {
                    Ok(path) => {
                        browser.refresh(&mut editor.project);
                        previews.extend(&mut images, editor.project.catalog.len());
                        if let Some(parent) = browser
                            .folders
                            .iter()
                            .position(|folder| folder.path == context.parent)
                        {
                            browser.navigate(parent);
                        }
                        browser_ui.selected = None;
                        browser_ui.selected_model = None;
                        context.focus_after = Some(path);
                        context.phase = Phase::Closed;
                        editor.status = format!("Создана папка «{name}»");
                    }
                    Err(error) => {
                        context.error = error;
                        focus.set(interface.name, FocusCause::Auto);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some((parent, revision, outcome)) = context.pending.as_mut().and_then(|pending| {
        block_on(poll_once(&mut pending.task))
            .map(|outcome| (pending.parent.clone(), pending.revision, outcome))
    }) {
        context.pending = None;
        context.completed = Some(Completed {
            parent,
            revision,
            report: outcome.report,
            status: outcome.status,
        });
    }
    if editor.pending.is_none()
        && !editor.hdri_library_open
        && context.phase == Phase::Closed
        && let Some(completed) = context.completed.take()
    {
        let reveal = browser.revision == completed.revision
            && browser.folders[browser.current].path == completed.parent;
        if !completed.report.imported.is_empty() {
            browser.refresh(&mut editor.project);
            previews.extend(&mut images, editor.project.catalog.len());
            if reveal && let Some(path) = completed.report.imported.last() {
                browser_ui.selected = browser.files.iter().position(|file| file.path == *path);
                browser_ui.selected_model = None;
                context.focus_after = Some(path.clone());
            }
        }
        editor.status = completed.status;
    }
    if context.pending.is_none()
        && context.completed.is_none()
        && let Some(mut batch) = context.queue.pop_front()
    {
        if browser.revision == batch.revision
            && browser.folders[browser.current].path == batch.parent
        {
            browser.set_search(String::new());
            batch.revision = browser.revision;
        }
        let root = editor.root.clone();
        let parent = batch.parent.clone();
        let creating = matches!(batch.work, Work::Image { .. });
        context.pending = Some(Import {
            parent: batch.parent,
            revision: batch.revision,
            task: IoTaskPool::get().spawn(async move { batch.work.run(root, parent) }),
        });
        editor.status = if creating {
            "Создание изображения…"
        } else {
            "Добавление изображений…"
        }
        .into();
    }
    editor.browser_popup = (context.phase != Phase::Closed || context.suppress_release)
        && editor.pending.is_none()
        && !editor.hdri_library_open;
}

pub(in crate::editor) fn refresh(
    mut commands: Commands,
    editor: Res<Editor>,
    browser: Res<Browser>,
    mut context: ResMut<BrowserContext>,
    mut interface: ResMut<ContextUi>,
    browser_ui: Res<drawer::BrowserUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut focus: ResMut<InputFocus>,
    geometry: Geometry,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut colors: Query<&mut BackgroundColor>,
    mut borders: Query<&mut BorderColor>,
    mut controls: Query<(Entity, &mut Control)>,
    parents: Query<&ChildOf>,
    mut scrolls: Query<&mut ScrollPosition>,
) {
    let phase = if editor.pending.is_none() && !editor.hdri_library_open {
        context.phase
    } else {
        Phase::Closed
    };
    for (entity, shown) in [
        (interface.overlay, phase != Phase::Closed),
        (interface.menu, phase == Phase::Menu),
        (
            interface.dialog,
            matches!(phase, Phase::Folder | Phase::Image),
        ),
        (interface.size.root, phase == Phase::Image),
        (interface.error, !context.error.is_empty()),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if let Ok(mut color) = colors.get_mut(interface.overlay) {
        color.set_if_neq(BackgroundColor(
            if matches!(phase, Phase::Folder | Phase::Image) {
                Color::srgba(0.0, 0.0, 0.0, 0.65)
            } else {
                Color::NONE
            },
        ));
    }
    let location = if context.parent.as_os_str().is_empty() {
        "Ассеты".into()
    } else {
        format!("Ассеты / {}", context.parent.display())
    };
    for (entity, value) in [
        (interface.menu_title, location.clone()),
        (interface.parent, format!("В папке: {location}")),
        (
            interface.dialog_title,
            if phase == Phase::Image {
                "Создать изображение"
            } else {
                "Создать папку"
            }
            .into(),
        ),
        (
            interface.name_label,
            if phase == Phase::Image {
                "Название изображения"
            } else {
                "Название папки"
            }
            .into(),
        ),
        (interface.error, context.error.clone()),
    ] {
        if let Ok(mut label) = labels.get_mut(entity) {
            label.set_if_neq(Text(value));
        }
    }
    if let Ok((_, mut control)) = controls.get_mut(interface.submit) {
        control.action = if phase == Phase::Image {
            Action::BrowserCreateImage
        } else {
            Action::BrowserCreateFolder
        };
    }
    if phase != interface.phase && matches!(phase, Phase::Folder | Phase::Image) {
        commands.entity(interface.dialog).insert(AccessibleLabel(
            if phase == Phase::Image {
                "Создание изображения"
            } else {
                "Создание папки"
            }
            .into(),
        ));
        commands.entity(interface.name).insert(AccessibleLabel(
            if phase == Phase::Image {
                "Название нового изображения"
            } else {
                "Название новой папки"
            }
            .into(),
        ));
    }
    if let Ok(mut menu) = nodes.get_mut(interface.menu) {
        menu.left = px(context
            .anchor
            .x
            .clamp(8.0, (window.width() - 264.0).max(8.0)));
        menu.top = px(context
            .anchor
            .y
            .clamp(8.0, (window.height() - 180.0).max(8.0)));
    }
    for entity in [interface.name, interface.size.width, interface.size.height] {
        if let Ok(mut border) = borders.get_mut(entity) {
            border.set_if_neq(BorderColor::all(if !context.error.is_empty() {
                ERROR
            } else if focus.get() == Some(entity) {
                FOCUS
            } else {
                EDGE
            }));
        }
    }
    if phase != Phase::Closed {
        if (phase != interface.phase
            || !focus.get().is_some_and(|entity| {
                parents
                    .iter_ancestors(entity)
                    .any(|parent| parent == interface.overlay)
            }))
            && window.focused
        {
            focus.set(
                if phase == Phase::Menu {
                    interface.new_folder
                } else if phase == Phase::Image {
                    interface.size.width
                } else {
                    interface.name
                },
                FocusCause::Auto,
            );
        }
    } else if interface.phase != Phase::Closed
        && editor.pending.is_none()
        && !editor.hdri_library_open
        && context.focus_after.is_none()
    {
        if let Some(previous) = context
            .previous_focus
            .take()
            .filter(|&entity| geometry.contains(entity))
        {
            focus.set(previous, FocusCause::Auto);
        } else {
            focus.clear();
        }
    }
    interface.phase = phase;
    if phase == Phase::Closed
        && editor.pending.is_none()
        && !editor.hdri_library_open
        && let Some(path) = context.focus_after.as_ref()
    {
        let entity = controls.iter().find_map(|(entity, control)| {
            let matched = match control.action {
                Action::Browse(index) => browser
                    .folders
                    .get(index)
                    .is_some_and(|folder| folder.path == *path),
                Action::ViewImage(index) => browser
                    .files
                    .get(index)
                    .is_some_and(|file| file.path == *path),
                _ => false,
            };
            (matched
                && parents
                    .iter_ancestors(entity)
                    .any(|parent| parent == browser_ui.grid))
            .then_some(entity)
        });
        if let Some(entity) = entity
            && let Some(rect) = bounds(&geometry, entity)
            && let Some(grid) = bounds(&geometry, browser_ui.grid)
        {
            if let Ok(mut scroll) = scrolls.get_mut(browser_ui.grid) {
                let delta = if rect.max.y > grid.max.y - 8.0 {
                    rect.max.y - grid.max.y + 8.0
                } else if rect.min.y < grid.min.y + 8.0 {
                    rect.min.y - grid.min.y - 8.0
                } else {
                    0.0
                };
                scroll.y = (scroll.y + delta / window.scale_factor()).max(0.0);
            }
            focus.set(entity, FocusCause::Auto);
            context.focus_after = None;
            context.previous_focus = None;
        }
    }
    let target = context
        .target
        .filter(|_| !context.hovered.is_empty() && phase == Phase::Closed && editor.drawer);
    for entity in [interface.indicator, interface.drop_hint] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if target.is_some() {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    if let Some((folder, rect)) = target {
        if let Some(panel) = bounds(&geometry, browser_ui.panel) {
            let rect = rect.intersect(panel);
            let scale = window.scale_factor();
            if let Ok(mut node) = nodes.get_mut(interface.indicator) {
                node.left = px(rect.min.x / scale);
                node.top = px(rect.min.y / scale);
                node.width = px(rect.width() / scale);
                node.height = px(rect.height() / scale);
            }
            if let Ok(mut node) = nodes.get_mut(interface.drop_hint) {
                node.top = px(panel.min.y / scale + 8.0);
            }
        }
        if let Ok(mut label) = labels.get_mut(interface.drop_label) {
            label.set_if_neq(Text(format!(
                "Добавить изображения в «{}»",
                browser.folders[folder].label
            )));
        }
    }
}

#[cfg(test)]
#[path = "browser_context_tests.rs"]
mod tests;
