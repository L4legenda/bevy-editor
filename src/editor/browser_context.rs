//! Native file drops and the asset browser's folder-creation menu.
use super::*;
#[path = "browser_delete.rs"]
mod deletion;
#[path = "browser_duplicate.rs"]
mod duplication;
#[path = "browser_explorer.rs"]
mod explorer;
#[path = "browser_image.rs"]
mod image_creation;
#[path = "browser_primitives.rs"]
mod primitive_creation;
#[path = "browser_primitive_dialog.rs"]
mod primitive_dialog;
#[path = "browser_primitive_preview.rs"]
mod primitive_preview;
#[path = "browser_rename.rs"]
mod rename;
#[path = "browser_scene.rs"]
mod scene_creation;
use crate::editor::{
    browser_files,
    model_primitives::{Parameters, Primitive},
    thumbnails::Thumbnails,
};
use bevy::{
    input_focus::FocusCause,
    tasks::{IoTaskPool, Task, block_on, poll_once},
    text::TextEdit,
    ui::CalculatedClip,
    window::{FileDragAndDrop, RequestRedraw},
    winit::WinitWindows,
};
pub(in crate::editor) use deletion::BrowserDeletionPlugin;
pub(in crate::editor) use primitive_creation::BrowserPrimitivePlugin;
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
    Scene,
    Rename,
    Primitive(Primitive),
}

impl Phase {
    fn dialog(self) -> bool {
        matches!(
            self,
            Self::Folder | Self::Image | Self::Scene | Self::Rename | Self::Primitive(_)
        )
    }
    fn primitive(self) -> Option<Primitive> {
        if let Self::Primitive(kind) = self {
            Some(kind)
        } else {
            None
        }
    }
}

enum Work {
    Duplicate {
        path: PathBuf,
    },
    Scene {
        name: String,
    },
    Primitive {
        kind: Primitive,
        name: String,
        parameters: Parameters,
    },
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
            Self::Duplicate { path } => match duplication::duplicate(&root, &path) {
                Ok(path) => Outcome {
                    status: format!(
                        "Создана копия · {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    report: browser_files::ImportReport {
                        imported: vec![path],
                        ..default()
                    },
                },
                Err(error) => Outcome {
                    status: format!("Не удалось дублировать файл · {error}"),
                    report: browser_files::ImportReport {
                        errors: vec![error],
                        ..default()
                    },
                },
            },
            Self::Scene { name } => match scene_creation::create(&root, &parent, &name) {
                Ok(path) => Outcome {
                    status: format!("Создана сцена · {}", path.display()),
                    report: browser_files::ImportReport {
                        imported: vec![path],
                        ..default()
                    },
                },
                Err(error) => Outcome {
                    status: format!("Не удалось создать сцену · {error}"),
                    report: browser_files::ImportReport {
                        errors: vec![error],
                        ..default()
                    },
                },
            },
            Self::Primitive {
                kind,
                name,
                parameters,
            } => match primitive_creation::create(&root, &parent, kind, &name, &parameters) {
                Ok(path) => Outcome {
                    status: format!("Создана 3D-модель · {}", path.display()),
                    report: browser_files::ImportReport {
                        imported: vec![path],
                        ..default()
                    },
                },
                Err(error) => Outcome {
                    status: format!("Не удалось создать 3D-модель · {error}"),
                    report: browser_files::ImportReport {
                        errors: vec![error],
                        ..default()
                    },
                },
            },
            Self::Import(paths) => {
                let report = browser_files::import_files(&root, &parent, paths);
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

#[derive(Clone, Copy)]
struct MenuEntry {
    entity: Entity,
    entry: browser::Entry,
    revision: u64,
    action: Action,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct BrowserContext {
    phase: Phase,
    parent: PathBuf,
    anchor: Vec2,
    error: String,
    previous_focus: Option<Entity>,
    menu_entry: Option<MenuEntry>,
    rename_target: Option<rename::Target>,
    focus_after: Option<PathBuf>,
    suppress_release: bool,
    hovered: HashSet<PathBuf>,
    target: Option<(usize, Rect)>,
    queue: VecDeque<Batch>,
    pending: Option<Import>,
    completed: Option<Completed>,
}

impl BrowserContext {
    pub(super) fn is_menu(&self) -> bool {
        self.phase == Phase::Menu
    }
    fn file_menu(&self) -> bool {
        self.is_menu()
            && self
                .menu_entry
                .is_some_and(|entry| !matches!(entry.entry, browser::Entry::Folder(_)))
    }
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
    creation: [Entity; 5],
    open: Entity,
    instance: Entity,
    open_icon: Entity,
    open_label: Entity,
    reveal: Entity,
    rename: Entity,
    duplicate: Entity,
    shown_entry: Option<Entity>,
    dialogues: primitive_creation::MenuUi,
    primitives: primitive_creation::MenuUi,
    dialog: Entity,
    dialog_title: Entity,
    name_label: Entity,
    extension: Entity,
    size: image_creation::SizeUi,
    parameters: primitive_dialog::ParametersUi,
    submit: Entity,
    submit_label: Entity,
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
    let open = control(
        commands,
        menu,
        Action::BrowserOpenEntry,
        Look::Regular,
        Node {
            width: percent(100),
            height: px(34),
            padding: UiRect::axes(px(9), px(5)),
            border: UiRect::ZERO,
            ..row(6.0)
        },
    );
    let open_icon = icons::themed(commands, open, icons::Glyph::File, 16.0);
    let open_label = text(commands, open, font_handle, "Открыть", 13.0, TEXT);
    let instance = button(
        commands,
        menu,
        font_handle,
        "Добавить экземпляр в сцену",
        Action::BrowserInstanceScene,
        Look::Regular,
    );
    let reveal = control(
        commands,
        menu,
        Action::BrowserRevealEntry,
        Look::Regular,
        Node {
            width: percent(100),
            height: px(34),
            padding: UiRect::axes(px(9), px(5)),
            ..row(6.0)
        },
    );
    commands
        .entity(reveal)
        .insert(AccessibleLabel("Показать в проводнике".into()));
    icons::themed(commands, reveal, icons::Glyph::FolderOpen, 16.0);
    text(
        commands,
        reveal,
        font_handle,
        "Показать в проводнике",
        13.0,
        TEXT,
    );
    let rename = control(
        commands,
        menu,
        Action::BrowserRenameFile,
        Look::Regular,
        Node {
            width: percent(100),
            height: px(34),
            padding: UiRect::axes(px(9), px(5)),
            ..row(6.0)
        },
    );
    commands
        .entity(rename)
        .insert(AccessibleLabel("Переименовать файл (F2)".into()));
    icons::themed(commands, rename, icons::Glyph::Pencil, 16.0);
    text(commands, rename, font_handle, "Переименовать", 13.0, TEXT);
    let shortcut = text(commands, rename, font_handle, "F2", 11.0, MUTED);
    commands.entity(shortcut).insert(Node {
        margin: UiRect::left(Val::Auto),
        ..default()
    });
    let duplicate = control(
        commands,
        menu,
        Action::BrowserDuplicateFile,
        Look::Regular,
        Node {
            width: percent(100),
            height: px(34),
            padding: UiRect::axes(px(9), px(5)),
            ..row(6.0)
        },
    );
    commands
        .entity(duplicate)
        .insert(AccessibleLabel("Дублировать файл".into()));
    icons::themed(commands, duplicate, icons::Glyph::Copy, 16.0);
    text(commands, duplicate, font_handle, "Дублировать", 13.0, TEXT);
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
            node.border = UiRect::all(px(0));
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

    let new_scene = button(
        commands,
        menu,
        font_handle,
        "Создать сцену",
        Action::BrowserNewScene,
        Look::Regular,
    );
    commands
        .entity(new_scene)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.width = percent(100);
            node.height = px(34);
            node.border = UiRect::all(px(0));
        });
    let icon = icons::themed(commands, new_scene, icons::Glyph::Layers, 16.0);
    commands.entity(new_scene).insert_children(0, &[icon]);
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
            node.border = UiRect::all(px(0));
        });
    let icon = icons::themed(commands, new_image, icons::Glyph::Image, 16.0);
    commands.entity(new_image).insert_children(0, &[icon]);
    let dialogues = primitive_creation::setup_dialogues(commands, menu, font_handle);
    let primitives = primitive_creation::setup(commands, menu, font_handle);

    let dialog = surface(
        commands,
        overlay,
        Node {
            width: px(440),
            max_width: percent(90),
            max_height: percent(92),
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(22)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            ..column(13.0)
        },
        PANEL,
    );
    commands
        .entity(dialog)
        .insert((AccessibleLabel("Создание папки".into()), Scroller));
    let dialog_title = text(commands, dialog, font_handle, "Создать папку", 21.0, TEXT);
    let parent = text(commands, dialog, font_handle, "", 12.0, MUTED);
    commands.entity(parent).insert((
        TextLayout::default(),
        Node {
            max_width: percent(100),
            ..default()
        },
    ));
    let name_group = node(
        commands,
        dialog,
        Node {
            width: percent(100),
            ..column(6.0)
        },
    );
    let name_label = text(
        commands,
        name_group,
        font_handle,
        "Название папки",
        12.0,
        MUTED,
    );
    let name_row = node(
        commands,
        name_group,
        Node {
            width: percent(100),
            ..row(8.0)
        },
    );
    let name = commands
        .spawn((
            TextInput,
            EditableText::new(""),
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel("Название новой папки".into()),
            font(font_handle, 14.0),
            TextColor(TEXT),
            TextLayout::no_wrap(),
            TabIndex(0),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_width: px(0),
                height: px(36),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(9), px(7)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(name_row),
        ))
        .id();
    let extension = text(commands, name_row, font_handle, ".png", 13.0, MUTED);
    let size = image_creation::setup(commands, dialog, font_handle);
    let parameters = primitive_dialog::setup(commands, dialog, font_handle);
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
    let cancel = button(
        commands,
        actions,
        font_handle,
        "Отмена",
        Action::BrowserCancelPopup,
        Look::Regular,
    );
    let submit = control(
        commands,
        actions,
        Action::BrowserCreateFolder,
        Look::Primary,
        Node {
            padding: UiRect::horizontal(px(9)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..row(4.0)
        },
    );
    commands
        .entity(submit)
        .insert(AccessibleLabel("Создать".into()));
    let submit_label = text(commands, submit, font_handle, "Создать", 13.0, TEXT);
    for entity in [cancel, submit] {
        commands
            .entity(entity)
            .entry::<Node>()
            .and_modify(|mut node| {
                node.min_width = px(88);
                node.height = px(34);
                node.justify_content = JustifyContent::Center;
            });
    }

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
        creation: [
            new_folder,
            new_scene,
            new_image,
            dialogues.trigger,
            primitives.trigger,
        ],
        open,
        instance,
        open_icon,
        open_label,
        reveal,
        rename,
        duplicate,
        shown_entry: None,
        dialogues,
        primitives,
        dialog,
        dialog_title,
        name_label,
        extension,
        size,
        parameters,
        submit,
        submit_label,
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

type EntryCards<'w, 's> = Query<
    'w,
    's,
    (
        &'static Control,
        Option<&'static drawer::FileCard>,
        Option<&'static drawer::DropFolder>,
    ),
    With<AssetOpen>,
>;

fn entry_at(
    browser: &Browser,
    geometry: &Geometry,
    cards: &EntryCards,
    point: Vec2,
) -> Option<MenuEntry> {
    geometry
        .iter()
        .filter_map(|(entity, node, _, _)| {
            let (control, card, folder) = cards.get(entity).ok()?;
            if !hit(geometry, entity, point) {
                return None;
            }
            let entry = match (card, folder) {
                (Some(card), _) if card.1 == browser.scan_revision => card.0,
                (None, Some(folder)) if folder.1 == browser.scan_revision => {
                    browser::Entry::Folder(folder.0)
                }
                _ => return None,
            };
            Some((
                node.size().element_product(),
                MenuEntry {
                    entity,
                    entry,
                    revision: browser.scan_revision,
                    action: control.action,
                },
            ))
        })
        .min_by(|(a, _), (b, _)| a.total_cmp(b))
        .map(|(_, entry)| entry)
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
    viewer: Option<Res<ImageViewer>>,
    browser: Res<Browser>,
    mut context: ResMut<BrowserContext>,
    interfaces: (Res<ContextUi>, ResMut<drawer::BrowserUi>),
    motion: Res<DrawerMotion>,
    window: Single<(Entity, &Window), With<PrimaryWindow>>,
    (mouse, mut keys, focus): (
        Res<ButtonInput<MouseButton>>,
        ResMut<ButtonInput<KeyCode>>,
        Res<InputFocus>,
    ),
    drag: Res<AssetDrag>,
    mut drops: MessageReader<FileDragAndDrop>,
    geometry: Geometry,
    (folders, cards): (Query<&drawer::DropFolder>, EntryCards),
    inputs: Query<(), With<TextInput>>,
    windows: Option<NonSend<WinitWindows>>,
    (mut redraw, mut actions): (MessageWriter<RequestRedraw>, MessageWriter<Action>),
) {
    let (entity, window) = *window;
    let (interface, mut browser_ui) = interfaces;
    context.suppress_release &=
        mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left);
    let allowed = editor.drawer
        && !viewer
            .as_ref()
            .is_some_and(|viewer| viewer.edit.color_picker.is_some())
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
        if context.phase == Phase::Closed
            && keys.just_pressed(KeyCode::F2)
            && !focus.get().is_some_and(|entity| inputs.contains(entity))
        {
            let focused = focus
                .get()
                .and_then(|entity| cards.get(entity).ok())
                .and_then(|(_, card, _)| {
                    card.filter(|card| card.1 == browser.scan_revision)
                        .map(|card| card.0)
                });
            let inside = window.cursor_position().is_some_and(|point| {
                hit(&geometry, browser_ui.panel, point * window.scale_factor())
            });
            if focused.is_some() || inside {
                keys.clear_just_pressed(KeyCode::F2);
                if let Some(entry) = focused {
                    browser_ui.select(Some(entry));
                }
                actions.write(Action::BrowserRenameFile);
            }
        }
        if mouse.just_pressed(MouseButton::Right)
            && !context.phase.dialog()
            && let Some(point) = window
                .cursor_position()
                .map(|point| point * window.scale_factor())
            && hit(&geometry, browser_ui.panel, point)
            && !(context.is_menu()
                && [
                    interface.menu,
                    interface.primitives.root,
                    interface.dialogues.root,
                ]
                .into_iter()
                .any(|entity| hit(&geometry, entity, point)))
            && !geometry
                .iter()
                .any(|(entity, _, _, _)| inputs.contains(entity) && hit(&geometry, entity, point))
        {
            let folder = destination(&browser, &geometry, &folders, point)
                .map_or(browser.current, |(folder, _)| folder);
            context.menu_entry = entry_at(&browser, &geometry, &cards, point);
            browser_ui.select(context.menu_entry.map(|entry| entry.entry));
            context.previous_focus = context.menu_entry.map(|entry| entry.entity);
            context.parent = browser.folders[folder].path.clone();
            context.anchor = point / window.scale_factor();
            context.phase = Phase::Menu;
        }
        if context.phase == Phase::Menu
            && mouse.just_pressed(MouseButton::Left)
            && window.cursor_position().is_none_or(|point| {
                let point = point * window.scale_factor();
                !hit(&geometry, interface.menu, point)
                    && !hit(&geometry, interface.primitives.root, point)
                    && !hit(&geometry, interface.dialogues.root, point)
            })
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
                    "Перетащите файлы в папку или свободное место файлового менеджера".into();
            }
            context.hovered.clear();
            context.target = None;
        }
    }
    if received {
        if !allowed || context.phase != Phase::Closed {
            editor.status = "Закройте открытое меню или просмотр и перетащите файлы в папку файлового менеджера".into();
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
            Some(Phase::Scene) => {
                actions.write(Action::BrowserCreateScene);
            }
            Some(Phase::Rename) => {
                actions.write(Action::BrowserConfirmRename);
            }
            Some(Phase::Primitive(_)) => {
                actions.write(Action::BrowserConfirmPrimitive);
            }
            _ => {}
        }
    }
}

pub(in crate::editor) fn update(
    mut commands: Commands,
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
    redirects: Option<Res<crate::asset_paths::AssetRedirects>>,
    (model_edit, image_viewer): (Res<ModelEditingState>, Option<Res<ImageViewer>>),
) {
    let allowed = editor.drawer && editor.pending.is_none() && !editor.hdri_library_open;
    if context.phase.dialog()
        && let Ok(input) = inputs.get(interface.name)
    {
        let signature = context.phase.primitive().map_or_else(
            || interface.size.signature(&inputs),
            |kind| interface.parameters.signature(kind, &inputs),
        );
        let name = format!("{}:{signature}", input.value());
        if *last_name != name {
            context.error.clear();
            *last_name = name;
        }
    }
    for action in actions.read() {
        match *action {
            Action::BrowserDuplicateFile if allowed && context.is_menu() => {
                let Some(entry) = context
                    .menu_entry
                    .filter(|entry| entry.revision == browser.scan_revision)
                else {
                    continue;
                };
                match rename::Target::new(&editor, &browser, entry.entry) {
                    Ok(target) => {
                        let saving_model =
                            editor
                                .project
                                .catalog
                                .iter()
                                .enumerate()
                                .any(|(index, model)| {
                                    std::path::Path::new(&model.asset) == target.path
                                        && model_edit.saving_model(index)
                                });
                        let saving_image = image_viewer.as_ref().is_some_and(|viewer| {
                            editor
                                .image_tabs
                                .iter()
                                .any(|tab| tab.path == target.path && viewer.edit.file_busy(tab.id))
                        });
                        if saving_model || saving_image {
                            editor.status =
                                "Дождитесь загрузки или сохранения файла и повторите дублирование"
                                    .into();
                        } else {
                            context.queue.push_back(Batch {
                                parent: target
                                    .path
                                    .parent()
                                    .unwrap_or(std::path::Path::new(""))
                                    .to_owned(),
                                revision: browser.revision,
                                work: Work::Duplicate { path: target.path },
                            });
                        }
                    }
                    Err(error) => editor.status = error,
                }
                context.phase = Phase::Closed;
            }
            Action::BrowserRenameFile
                if allowed && matches!(context.phase, Phase::Menu | Phase::Closed) =>
            {
                let entry = if context.is_menu() {
                    context
                        .menu_entry
                        .filter(|entry| entry.revision == browser.scan_revision)
                        .map(|entry| entry.entry)
                } else {
                    browser_ui.selected_entry
                };
                let Some(entry) = entry else {
                    continue;
                };
                match rename::Target::new(&editor, &browser, entry) {
                    Ok(target) => {
                        if context.phase == Phase::Closed {
                            context.previous_focus = focus.get();
                        }
                        context.parent = target
                            .path
                            .parent()
                            .unwrap_or(std::path::Path::new(""))
                            .to_owned();
                        if let Ok(mut input) = inputs.get_mut(interface.name) {
                            input.editor_mut().set_text(&target.stem());
                            input.queue_edit(TextEdit::SelectAll);
                        }
                        context.rename_target = Some(target);
                        context.phase = Phase::Rename;
                        context.error.clear();
                    }
                    Err(error) => editor.status = error,
                }
            }
            Action::BrowserConfirmRename if allowed && context.phase == Phase::Rename => {
                let Some(target) = context.rename_target.as_ref() else {
                    continue;
                };
                if editor
                    .project
                    .catalog
                    .iter()
                    .enumerate()
                    .any(|(index, model)| {
                        std::path::Path::new(&model.asset) == target.path
                            && model_edit.saving_model(index)
                    })
                {
                    context.error = "Дождитесь сохранения модели и повторите переименование".into();
                    continue;
                }
                if image_viewer.as_ref().is_some_and(|viewer| {
                    editor
                        .image_tabs
                        .iter()
                        .any(|tab| tab.path == target.path && viewer.edit.file_busy(tab.id))
                }) {
                    context.error =
                        "Дождитесь загрузки или сохранения изображения и повторите переименование"
                            .into();
                    continue;
                }
                let name = inputs
                    .get(interface.name)
                    .map(|input| input.value().to_string())
                    .unwrap_or_default();
                let scene_asset = editor
                    .has_scene()
                    .then(|| editor.definition().asset.clone());
                match target.apply(&name, &mut editor, &mut browser) {
                    Ok(path) => {
                        editor.status = match redirects.as_ref().map(|map| map.reload()).transpose()
                        {
                            Ok(_) => format!(
                                "Файл переименован · {}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            Err(error) => {
                                format!("Файл переименован, ошибка обновления ссылок · {error}")
                            }
                        };
                        if scene_asset.is_some_and(|asset| asset != editor.definition().asset) {
                            editor.restore_selection = match editor.selection {
                                Some(Selection::Imported(index)) => {
                                    editor.imported.get(index).map(|node| node.name.clone())
                                }
                                _ => None,
                            };
                            editor.reload = true;
                        }
                        browser_ui.select(rename::entry_at_path(&editor, &browser, &path));
                        context.focus_after = Some(path);
                        context.phase = Phase::Closed;
                        context.rename_target = None;
                        context.error.clear();
                    }
                    Err(error) => {
                        context.error = error;
                        focus.set(interface.name, FocusCause::Auto);
                    }
                }
            }
            Action::BrowserRevealEntry if allowed && context.is_menu() => {
                if let Some(entry) = context
                    .menu_entry
                    .filter(|entry| entry.revision == browser.scan_revision)
                {
                    let result =
                        crate::editor::file_moves::entry_path(&editor, &browser, entry.entry)
                            .and_then(|path| explorer::open(&editor.root, &path));
                    match result {
                        Ok(()) => browser.error = None,
                        Err(error) => {
                            let message = format!("Не удалось открыть проводник · {error}");
                            editor.status = message.clone();
                            browser.error = Some(message);
                        }
                    }
                }
                context.phase = Phase::Closed;
            }
            Action::BrowserInstanceScene if allowed && context.is_menu() => {
                if editor.has_scene()
                    && editor.ready
                    && editor.definition().is_native()
                    && let Some(entry) = context
                        .menu_entry
                        .filter(|e| e.revision == browser.scan_revision)
                    && let browser::Entry::Scene { room, .. } = entry.entry
                {
                    commands.write_message(Action::Hierarchy(
                        crate::editor::scene_hierarchy::HierarchyAction::Insert(room),
                    ));
                }
                context.phase = Phase::Closed;
                context.suppress_release = false;
            }
            Action::BrowserOpenEntry if allowed && context.is_menu() => {
                if let Some(entry) = context
                    .menu_entry
                    .filter(|entry| entry.revision == browser.scan_revision)
                {
                    commands.write_message(entry.action);
                }
                context.phase = Phase::Closed;
            }
            Action::BrowserCreatePrimitive(primitive)
                if allowed && matches!(context.phase, Phase::Menu | Phase::Closed) =>
            {
                if context.phase == Phase::Closed {
                    context.parent = browser.folders[browser.current].path.clone();
                    context.previous_focus = focus.get();
                }
                context.phase = Phase::Primitive(primitive);
                context.error.clear();
                let name =
                    primitive_creation::suggested_name(&editor.root, &context.parent, primitive);
                if let Ok(mut input) = inputs.get_mut(interface.name) {
                    input.editor_mut().set_text(&name);
                    input.queue_edit(TextEdit::SelectAll);
                }
                interface
                    .parameters
                    .reset(primitive, &mut inputs, &mut commands);
            }
            Action::BrowserConfirmPrimitive if allowed => {
                let Some(kind) = context.phase.primitive() else {
                    continue;
                };
                let name = inputs
                    .get(interface.name)
                    .map(|input| input.value().to_string())
                    .unwrap_or_default();
                let result = primitive_creation::stem(&name).and_then(|name| {
                    interface
                        .parameters
                        .read(kind, &inputs.as_readonly())
                        .map(|parameters| (name, parameters))
                });
                match result {
                    Ok((name, parameters)) => {
                        if let Some(folder) = browser
                            .folders
                            .iter()
                            .position(|folder| folder.path == context.parent)
                        {
                            browser.navigate(folder);
                        }
                        browser.set_search(String::new());
                        let batch = Batch {
                            parent: context.parent.clone(),
                            revision: browser.revision,
                            work: Work::Primitive {
                                kind,
                                name,
                                parameters,
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
            Action::BrowserNewFolder | Action::BrowserNewImage | Action::BrowserNewScene
                if allowed =>
            {
                if context.phase == Phase::Closed {
                    context.parent = browser.folders[browser.current].path.clone();
                    context.previous_focus = focus.get();
                }
                let image = matches!(action, Action::BrowserNewImage);
                let scene = matches!(action, Action::BrowserNewScene);
                context.phase = if image {
                    Phase::Image
                } else if scene {
                    Phase::Scene
                } else {
                    Phase::Folder
                };
                context.error.clear();
                if let Ok(mut input) = inputs.get_mut(interface.name) {
                    let name = if image {
                        browser_files::suggested_image_name(&editor.root, &context.parent)
                    } else if scene {
                        scene_creation::suggested_name(&editor.root, &context.parent)
                    } else {
                        browser_files::suggested_name(&editor.root, &context.parent)
                    };
                    input.editor_mut().set_text(&name);
                    input.queue_edit(TextEdit::SelectAll);
                }
                if image {
                    interface.size.set(&mut inputs, 1024, 1024);
                }
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
            Action::BrowserCreateScene if allowed && context.phase == Phase::Scene => {
                let name = inputs
                    .get(interface.name)
                    .map(|input| input.value().to_string())
                    .unwrap_or_default();
                match scene_creation::stem(&name) {
                    Ok(_) => {
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
                            work: Work::Scene { name },
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
            Action::Narrative(NarrativeAction::Create(_)) => {
                context.phase = Phase::Closed;
                context.error.clear();
            }
            Action::BrowserCancelPopup => {
                context.phase = Phase::Closed;
                context.rename_target = None;
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
                        browser_ui.select(None);
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
                browser_ui.selected_model = editor
                    .project
                    .catalog
                    .iter()
                    .position(|model| std::path::Path::new(&model.asset) == path);
                browser_ui.selected_entry = browser_ui
                    .selected_model
                    .map(browser::Entry::Model)
                    .or_else(|| {
                        browser_ui.selected.map(|file| {
                            if browser.files[file].kind == browser::FileKind::Image {
                                browser::Entry::Image(file)
                            } else if browser.files[file].kind == browser::FileKind::Scene {
                                editor
                                    .project
                                    .scenes
                                    .iter()
                                    .position(|scene| {
                                        std::path::Path::new(&scene.file)
                                            == browser.files[file].path.as_path()
                                    })
                                    .map_or(browser::Entry::File(file), |room| {
                                        browser::Entry::Scene { file, room }
                                    })
                            } else {
                                browser::Entry::File(file)
                            }
                        })
                    });
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
        let status = match &batch.work {
            Work::Duplicate { .. } => "Создание копии файла…",
            Work::Primitive { .. } => "Создание 3D-модели…",
            Work::Image { .. } => "Создание изображения…",
            Work::Scene { .. } => "Создание сцены…",
            Work::Import(_) => "Добавление файлов…",
        };
        context.pending = Some(Import {
            parent: batch.parent,
            revision: batch.revision,
            task: IoTaskPool::get().spawn(async move { batch.work.run(root, parent) }),
        });
        editor.status = status.into();
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
    (inputs, parents): (Query<&EditableText>, Query<&ChildOf>),
    (mut scrolls, mut numbers): (Query<&mut ScrollPosition>, Query<&mut Number>),
) {
    let phase = if editor.pending.is_none() && !editor.hdri_library_open {
        context.phase
    } else {
        Phase::Closed
    };
    if context.is_menu()
        && context.menu_entry.is_some_and(|entry| {
            entry.revision != browser.scan_revision || !geometry.contains(entry.entity)
        })
    {
        context.phase = Phase::Closed;
        context.menu_entry = None;
    }
    let phase = if context.phase == Phase::Closed {
        Phase::Closed
    } else {
        phase
    };
    let file_menu = context.file_menu();
    let has_entry = context.menu_entry.is_some();
    for entity in interface.creation {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if file_menu {
                Display::None
            } else {
                Display::Flex
            };
        }
    }
    for entity in [interface.open, interface.reveal] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if has_entry {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    for entity in [interface.rename, interface.duplicate] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if file_menu {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    if let Ok(mut node) = nodes.get_mut(interface.instance) {
        node.display=if editor.has_scene() && editor.ready && !editor.asset_viewer_open() && editor.definition().is_native()
            && context.menu_entry.is_some_and(|e|matches!(e.entry,browser::Entry::Scene {room,..} if room!=editor.room && editor.project.scenes.get(room).is_some_and(|s|s.is_native()))) {Display::Flex}else{Display::None};
    }
    let (open_label, glyph) = match context.menu_entry.map(|entry| (entry.entry, entry.action)) {
        Some((browser::Entry::Folder(_), _)) => ("Открыть папку", icons::Glyph::FolderOpen),
        Some((browser::Entry::Model(_), _)) => ("Открыть", icons::Glyph::Box),
        Some((browser::Entry::Image(_), _)) => ("Открыть", icons::Glyph::Image),
        Some((browser::Entry::Scene { .. }, _)) => ("Открыть", icons::Glyph::Layers),
        Some((_, Action::BrowseFile(_))) => ("Сведения", icons::Glyph::File),
        _ => ("Открыть", icons::Glyph::File),
    };
    icons::set_glyph(&mut commands, interface.open_icon, glyph);
    commands
        .entity(interface.open)
        .insert(AccessibleLabel(open_label.into()));
    if let Ok(mut label) = labels.get_mut(interface.open_label) {
        label.set_if_neq(Text(open_label.into()));
    }
    let rename_extension = context
        .rename_target
        .as_ref()
        .and_then(|target| target.extension());
    let extension = match phase {
        Phase::Image => Some(".png"),
        Phase::Scene => Some(".scene"),
        Phase::Primitive(_) => Some(".glb"),
        Phase::Rename => rename_extension.as_deref(),
        _ => None,
    };
    interface.parameters.refresh(
        phase.primitive(),
        &mut nodes,
        &mut labels,
        &inputs,
        &mut numbers,
    );
    interface.size.limits(&inputs, &mut numbers);
    if let Ok(mut dialog) = nodes.get_mut(interface.dialog) {
        dialog.width = px(if phase.primitive().is_some() {
            760
        } else {
            440
        });
    }
    for (entity, shown) in [
        (interface.overlay, phase != Phase::Closed),
        (interface.menu, phase == Phase::Menu),
        (interface.dialog, phase.dialog()),
        (interface.size.root, phase == Phase::Image),
        (interface.parameters.root, phase.primitive().is_some()),
        (
            interface.parent,
            matches!(
                phase,
                Phase::Folder | Phase::Scene | Phase::Rename | Phase::Primitive(_)
            ),
        ),
        (
            interface.extension,
            extension.is_some()
                && !inputs.get(interface.name).is_ok_and(|input| {
                    input
                        .value()
                        .to_string()
                        .to_ascii_lowercase()
                        .ends_with(extension.unwrap_or_default())
                }),
        ),
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
        color.set_if_neq(BackgroundColor(if phase.dialog() {
            Color::srgba(0.0, 0.0, 0.0, 0.65)
        } else {
            Color::NONE
        }));
    }
    let location = if context.parent.as_os_str().is_empty() {
        "Ассеты".into()
    } else {
        format!("Ассеты / {}", context.parent.display())
    };
    let menu_title = if file_menu {
        context
            .menu_entry
            .and_then(|entry| match entry.entry {
                browser::Entry::Model(index) => editor
                    .project
                    .catalog
                    .get(index)
                    .and_then(|model| std::path::Path::new(&model.asset).file_name())
                    .map(|name| name.to_string_lossy().into_owned()),
                browser::Entry::Scene { file, .. }
                | browser::Entry::Image(file)
                | browser::Entry::File(file) => {
                    browser.files.get(file).map(|file| file.filename.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| location.clone())
    } else {
        location.clone()
    };
    for (entity, value) in [
        (interface.menu_title, menu_title),
        (interface.parent, format!("В папке: {location}")),
        (interface.extension, extension.unwrap_or_default().into()),
        (
            interface.dialog_title,
            match phase {
                Phase::Image => "Новое изображение".into(),
                Phase::Scene => "Создать сцену".into(),
                Phase::Primitive(kind) => format!("Новая фигура · {}", kind.label()),
                Phase::Rename => "Переименовать файл".into(),
                _ => "Создать папку".into(),
            },
        ),
        (
            interface.name_label,
            if phase == Phase::Rename {
                "Новое имя файла"
            } else if phase == Phase::Image {
                "Имя"
            } else if phase == Phase::Scene {
                "Название сцены"
            } else if phase.primitive().is_some() {
                "Название модели"
            } else {
                "Название папки"
            }
            .into(),
        ),
        (interface.error, context.error.clone()),
        (
            interface.submit_label,
            if phase == Phase::Rename {
                "Переименовать"
            } else {
                "Создать"
            }
            .into(),
        ),
    ] {
        if let Ok(mut label) = labels.get_mut(entity) {
            label.set_if_neq(Text(value));
        }
    }
    if let Ok((_, mut control)) = controls.get_mut(interface.submit) {
        control.action = if phase == Phase::Rename {
            Action::BrowserConfirmRename
        } else if phase == Phase::Image {
            Action::BrowserCreateImage
        } else if phase == Phase::Scene {
            Action::BrowserCreateScene
        } else if phase.primitive().is_some() {
            Action::BrowserConfirmPrimitive
        } else {
            Action::BrowserCreateFolder
        };
    }
    commands.entity(interface.submit).insert(AccessibleLabel(
        if phase == Phase::Rename {
            "Переименовать"
        } else {
            "Создать"
        }
        .into(),
    ));
    if phase != interface.phase && phase.dialog() {
        if let Ok(mut scroll) = scrolls.get_mut(interface.dialog) {
            scroll.0 = Vec2::ZERO;
        }
        commands.entity(interface.dialog).insert(AccessibleLabel(
            if phase == Phase::Rename {
                "Переименование файла"
            } else if phase == Phase::Image {
                "Создание изображения"
            } else if phase == Phase::Scene {
                "Создание сцены"
            } else if phase.primitive().is_some() {
                "Создание 3D-фигуры"
            } else {
                "Создание папки"
            }
            .into(),
        ));
        commands.entity(interface.name).insert(AccessibleLabel(
            if phase == Phase::Rename {
                "Новое имя файла"
            } else if phase == Phase::Image {
                "Название нового изображения"
            } else if phase == Phase::Scene {
                "Название новой сцены"
            } else if phase.primitive().is_some() {
                "Название новой 3D-модели"
            } else {
                "Название новой папки"
            }
            .into(),
        ));
    }
    if let Ok(mut menu) = nodes.get_mut(interface.menu) {
        let menu_height = bounds(&geometry, interface.menu)
            .map_or(250.0, |rect| rect.height() / window.scale_factor());
        menu.left = px(context
            .anchor
            .x
            .clamp(8.0, (window.width() - 264.0).max(8.0)));
        menu.top = px(context
            .anchor
            .y
            .clamp(8.0, (window.height() - menu_height - 8.0).max(8.0)));
    }
    for entity in [interface.name, interface.size.width, interface.size.height]
        .into_iter()
        .chain(interface.parameters.inputs())
    {
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
            || (phase == Phase::Menu
                && interface.shown_entry != context.menu_entry.map(|entry| entry.entity))
            || !focus.get().is_some_and(|entity| {
                parents
                    .iter_ancestors(entity)
                    .any(|parent| parent == interface.overlay)
            }))
            && window.focused
        {
            focus.set(
                if phase == Phase::Menu {
                    if has_entry {
                        interface.open
                    } else {
                        interface.new_folder
                    }
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
    interface.shown_entry = context.menu_entry.map(|entry| entry.entity);
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
                Action::Open(room) => editor
                    .project
                    .scenes
                    .get(room)
                    .is_some_and(|scene| std::path::Path::new(&scene.file) == path),
                Action::ViewModel(index) => editor
                    .project
                    .catalog
                    .get(index)
                    .is_some_and(|model| std::path::Path::new(&model.asset) == path),
                Action::BrowseFile(index) => browser
                    .files
                    .get(index)
                    .is_some_and(|file| file.path == *path),
                Action::Narrative(NarrativeAction::File(index)) => browser
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
                "Добавить файлы в «{}»",
                browser.folders[folder].label
            )));
        }
    }
}

#[cfg(test)]
#[path = "browser_context_tests.rs"]
mod tests;
