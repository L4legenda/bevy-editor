//! Image-to-3D workspace with progressive disclosure and visible task activity.
use super::*;
use crate::editor::{
    generation::{Quality, Workspace},
    image_previews,
};
use bevy::{
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::FileDragAndDrop,
};
use std::path::PathBuf;

#[path = "generation_install_ui.rs"]
mod installation;
#[path = "generation_progress.rs"]
mod progress;
#[path = "generation_run_ui.rs"]
mod run;
#[path = "generation_waiting.rs"]
mod waiting;

#[cfg(test)]
#[path = "generation_ui_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum GenerationAction {
    Open,
    Toggle,
    Close,
    Pick,
    Dismiss,
    Choose(usize, u64),
    LoadPath,
    Clear,
    Quality(Quality),
    Advanced,
    RandomSeed,
    Simplify,
    Obj,
    Generate,
    StartModel,
    StopModel,
    DownloadModel,
    CancelModelDownload,
    CheckModel,
    CancelGeneration,
    ImportResult,
    OpenResult,
    FitResult,
    ToggleWireframe,
    Section(Section),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Section {
    Path,
    Output,
    System,
    Advanced,
    Details,
    Installation,
}
#[derive(Component)]
struct FoldIcon(Section);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Field {
    Path,
    Name,
    Steps,
    Guidance,
    Resolution,
    Chunks,
    Seed,
    Faces,
}

#[derive(Component)]
pub(in crate::editor) struct SourceDrop {
    pub enabled: bool,
}

pub(in crate::editor) fn source_file(browser: &Browser, entry: browser::Entry) -> Option<usize> {
    let browser::Entry::Image(file) = entry else {
        return None;
    };
    browser
        .files
        .get(file)
        .filter(|file| Workspace::accepts(&file.path))
        .map(|_| file)
}

#[derive(Resource)]
struct GenerationUi {
    root: Entity,
    advanced: Entity,
    path_settings: Entity,
    output_settings: Entity,
    system_settings: Entity,
    source_progress: Entity,
    source_fill: Entity,
    faces: Entity,
    seed: Entity,
    picture: Entity,
    empty: Entity,
    source_label: Entity,
    error: Entity,
    picker: Entity,
    choices: Entity,
    picker_revision: Option<u64>,
    font: Handle<Font>,
    path_input: Entity,
    advanced_open: bool,
    path_open: bool,
    output_open: bool,
    system_open: bool,
    picking: bool,
    requested: Option<PathBuf>,
    loaded: Option<PathBuf>,
    task: Option<Task<Result<image_previews::DecodedImage, String>>>,
    texture: Option<Handle<Image>>,
    dimensions: Option<UVec2>,
    problem: String,
}

pub(in crate::editor) struct GenerationPlugin;
impl Plugin for GenerationPlugin {
    fn build(&self, app: &mut App) {
        run::register(app);
        waiting::register(app);
        app.init_resource::<installation::State>()
            .add_systems(Update, drops.before(crate::editor::handle_actions))
            .add_systems(
                Update,
                actions
                    .after(crate::editor::handle_actions)
                    .before(super::refresh),
            )
            .add_systems(
                Update,
                installation::update
                    .after(crate::editor::handle_actions)
                    .before(super::refresh),
            )
            .add_systems(
                Update,
                run::update
                    .after(installation::update)
                    .after(actions)
                    .before(super::refresh),
            )
            .add_systems(
                Update,
                (refresh, installation::refresh, run::refresh)
                    .chain()
                    .after(super::refresh),
            );
        app.add_systems(
            Update,
            progress::animate
                .after(refresh)
                .after(installation::refresh)
                .after(run::refresh),
        );
    }
}

pub(super) fn available(editor: &Editor, action: GenerationAction) -> bool {
    if matches!(action, GenerationAction::StopModel) {
        return editor.generation.engine_active;
    }
    if editor.pending.is_some() || editor.browser_popup || editor.hdri_library_open {
        return false;
    }
    match action {
        GenerationAction::Open | GenerationAction::Toggle => true,
        GenerationAction::Close => editor.generation.opened,
        GenerationAction::Generate => {
            editor.generation.active
                && editor.generation.backend_ready
                && editor.generation.source_ready
                && editor.generation.config_valid
                && !editor.generation.running
        }
        GenerationAction::CancelGeneration => editor.generation.running,
        GenerationAction::StartModel => {
            editor.generation.active && !editor.generation.engine_active
        }
        GenerationAction::StopModel => editor.generation.engine_active,
        GenerationAction::ImportResult => {
            editor.generation.has_result
                && !editor.generation.running
                && !editor.generation.result_imported
        }
        GenerationAction::OpenResult => editor.generation.result_imported,
        GenerationAction::FitResult => editor.generation.has_result,
        GenerationAction::ToggleWireframe => {
            editor.generation.active && editor.generation.has_result && !editor.generation.running
        }
        GenerationAction::CheckModel => editor.generation.active && !editor.generation.running,
        _ => editor.generation.active,
    }
}

pub(super) fn active(editor: &Editor, action: GenerationAction) -> bool {
    let draft = &editor.generation;
    match action {
        GenerationAction::Open | GenerationAction::Toggle => draft.active,
        GenerationAction::Quality(q) => {
            let (steps, resolution) = match q {
                Quality::Draft => (20, 256),
                Quality::Standard => (30, 380),
                Quality::Detailed => (50, 512),
            };
            draft.steps == steps && draft.resolution == resolution
        }
        GenerationAction::RandomSeed => draft.random_seed,
        GenerationAction::Simplify => draft.simplify,
        GenerationAction::Obj => draft.obj,
        GenerationAction::ToggleWireframe => draft.wireframe,
        _ => false,
    }
}

pub(super) fn shortcuts(
    keys: &ButtonInput<KeyCode>,
    editing: bool,
    actions: &mut MessageWriter<Action>,
) {
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if command && keys.just_pressed(KeyCode::KeyW) {
        actions.write(Action::Generation(GenerationAction::Close));
    } else if keys.just_pressed(KeyCode::Escape) {
        actions.write(Action::Generation(GenerationAction::Dismiss));
    } else if !editing && command && keys.just_pressed(KeyCode::Space) {
        actions.write(Action::Drawer);
    }
}

pub(super) fn footer(commands: &mut Commands, parent: Entity, handle: &Handle<Font>) {
    let item = control(
        commands,
        parent,
        Action::Generation(GenerationAction::Toggle),
        Look::Regular,
        Node {
            height: percent(100),
            padding: UiRect::horizontal(px(10)),
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(5)),
            ..row(7.0)
        },
    );
    commands
        .entity(item)
        .insert(AccessibleLabel("Переключить 3D генерацию".into()));
    icons::spawn(commands, item, icons::Glyph::Box, 16.0, FOCUS);
    text(commands, item, handle, "3D генерация", 13.0, TEXT);
}

pub(super) fn service_footer(commands: &mut Commands, parent: Entity, handle: &Handle<Font>) {
    run::service_footer(commands, parent, handle);
}

fn action(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    label: &str,
    value: GenerationAction,
    primary: bool,
) -> Entity {
    button(
        commands,
        parent,
        handle,
        label,
        Action::Generation(value),
        if primary {
            Look::Primary
        } else {
            Look::Regular
        },
    )
}

fn heading(commands: &mut Commands, parent: Entity, handle: &Handle<Font>, caption: &str) {
    text(commands, parent, handle, caption, 14.0, TEXT);
}

fn disclosure(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    caption: &str,
    section: Section,
) -> Entity {
    let item = action(
        commands,
        parent,
        handle,
        caption,
        if section == Section::Advanced {
            GenerationAction::Advanced
        } else {
            GenerationAction::Section(section)
        },
        false,
    );
    let icon = icons::spawn(commands, item, icons::Glyph::ChevronRight, 12.0, MUTED);
    commands
        .entity(icon)
        .insert((FoldIcon(section), UiTransform::default()));
    commands.entity(item).insert_children(0, &[icon]);
    item
}

fn field(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    caption: &str,
    kind: Field,
    value: &str,
) -> Entity {
    text(commands, parent, handle, caption, 11.0, MUTED);
    let mut editable = EditableText::new(value);
    editable.allow_newlines = false;
    commands
        .spawn((
            TextInput,
            editable,
            kind,
            crate::ui_theme::text_cursor_style(),
            AccessibleLabel(caption.into()),
            font(handle, 13.0),
            TextColor(TEXT),
            TabIndex(0),
            TextLayout::no_wrap(),
            BackgroundColor(WELL),
            BorderColor::all(EDGE),
            Node {
                width: percent(100),
                height: px(32),
                flex_shrink: 0.0,
                padding: UiRect::axes(px(9), px(6)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

fn card(commands: &mut Commands, parent: Entity) -> Entity {
    let entity = surface(
        commands,
        parent,
        Node {
            width: percent(100),
            padding: UiRect::all(px(14)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            flex_shrink: 0.0,
            ..column(9.0)
        },
        PANEL,
    );
    commands.entity(entity).insert(BorderColor::all(EDGE));
    entity
}

pub(super) fn setup(
    commands: &mut Commands,
    parent: Entity,
    handle: &Handle<Font>,
    checker: &Handle<Image>,
) {
    let root = surface(
        commands,
        parent,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(IMAGE_TOP),
            bottom: px(FOOT),
            left: px(0),
            right: px(0),
            padding: UiRect::all(px(20)),
            ..column(16.0)
        },
        CHROME,
    );
    commands.entity(root).insert((
        ZIndex(6),
        AccessibleLabel("Рабочая область 3D генерации".into()),
        TabGroup::new(0),
    ));
    let title = node(
        commands,
        root,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            flex_shrink: 0.0,
            ..row(12.0)
        },
    );
    let title_copy = node(commands, title, column(4.0));
    text(commands, title_copy, handle, "3D генерация", 23.0, TEXT);
    text(commands, title_copy, handle, "Hunyuan3D-2mini", 12.0, MUTED);
    let badge = surface(
        commands,
        title,
        Node {
            padding: UiRect::axes(px(12), px(8)),
            border_radius: BorderRadius::all(px(7)),
            ..default()
        },
        WELL,
    );
    let model_badge = text(commands, badge, handle, "Проверка модели", 12.0, FOCUS);

    let body = node(
        commands,
        root,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            align_items: AlignItems::Stretch,
            ..row(18.0)
        },
    );
    let left = node(
        commands,
        body,
        Node {
            width: px(342),
            min_width: px(290),
            max_width: percent(46),
            height: percent(100),
            flex_shrink: 0.0,
            ..column(12.0)
        },
    );
    let scroll = node(
        commands,
        left,
        Node {
            width: percent(100),
            min_height: px(0),
            flex_grow: 1.0,
            overflow: Overflow::scroll_y(),
            padding: UiRect::right(px(5)),
            ..column(12.0)
        },
    );
    commands.entity(scroll).insert(Scroller);
    let source = card(commands, scroll);
    heading(commands, source, handle, "Исходное изображение");
    let preview = surface(
        commands,
        source,
        Node {
            width: percent(100),
            height: px(148),
            flex_shrink: 0.0,
            overflow: Overflow::clip(),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(7)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        WELL,
    );
    commands.entity(preview).insert((
        SourceDrop { enabled: false },
        BorderColor::all(EDGE),
        AccessibleLabel("Исходное изображение для 3D генерации".into()),
        ImageNode {
            image: checker.clone(),
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
    ));
    let picture = commands
        .spawn((
            ImageNode::default(),
            Node {
                display: Display::None,
                width: px(140),
                height: px(140),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(preview),
        ))
        .id();
    let empty = node(
        commands,
        preview,
        Node {
            align_items: AlignItems::Center,
            ..column(8.0)
        },
    );
    icons::spawn(commands, empty, icons::Glyph::Image, 28.0, MUTED);
    text(
        commands,
        empty,
        handle,
        "Перетащите изображение",
        12.0,
        MUTED,
    );
    let (source_progress, source_fill) = progress::bar(commands, preview, 4.0);
    commands.entity(source_progress).insert(Node {
        display: Display::None,
        position_type: PositionType::Absolute,
        left: px(12),
        right: px(12),
        bottom: px(8),
        height: px(4),
        overflow: Overflow::clip(),
        border_radius: BorderRadius::MAX,
        ..default()
    });
    let source_label = text(commands, source, handle, "PNG · JPG · WebP", 11.0, MUTED);
    let source_actions = node(commands, source, row(6.0));
    action(
        commands,
        source_actions,
        handle,
        "Выбрать из проекта",
        GenerationAction::Pick,
        false,
    );
    action(
        commands,
        source_actions,
        handle,
        "Убрать",
        GenerationAction::Clear,
        false,
    );
    disclosure(commands, source_actions, handle, "Путь…", Section::Path);
    let path_settings = node(
        commands,
        source,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(6.0)
        },
    );
    let path_input = field(
        commands,
        path_settings,
        handle,
        "Или путь к файлу на компьютере",
        Field::Path,
        "",
    );
    action(
        commands,
        path_settings,
        handle,
        "Открыть изображение",
        GenerationAction::LoadPath,
        false,
    );

    let quality = card(commands, scroll);
    heading(commands, quality, handle, "Качество");
    let profiles = node(
        commands,
        quality,
        Node {
            flex_wrap: FlexWrap::Wrap,
            ..row(4.0)
        },
    );
    for (label, value) in [
        ("Черновик", Quality::Draft),
        ("Стандарт", Quality::Standard),
        ("Детально", Quality::Detailed),
    ] {
        action(
            commands,
            profiles,
            handle,
            label,
            GenerationAction::Quality(value),
            false,
        );
    }
    let advanced_button = disclosure(
        commands,
        quality,
        handle,
        "Дополнительно",
        Section::Advanced,
    );
    commands
        .entity(advanced_button)
        .insert(AccessibleLabel("Расширенные параметры".into()));
    let advanced = node(
        commands,
        quality,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(6.0)
        },
    );
    field(
        commands,
        advanced,
        handle,
        "Шаги · 1–100",
        Field::Steps,
        "30",
    );
    field(
        commands,
        advanced,
        handle,
        "CFG · 1–20",
        Field::Guidance,
        "5",
    );
    field(
        commands,
        advanced,
        handle,
        "Разрешение поверхности · 16–512",
        Field::Resolution,
        "380",
    );
    field(
        commands,
        advanced,
        handle,
        "Размер блока · 1 000–5 000 000",
        Field::Chunks,
        "20000",
    );
    action(
        commands,
        advanced,
        handle,
        "Случайный seed",
        GenerationAction::RandomSeed,
        false,
    );
    let seed = node(
        commands,
        advanced,
        Node {
            width: percent(100),
            ..column(5.0)
        },
    );
    field(commands, seed, handle, "Seed", Field::Seed, "12345");

    let output = card(commands, scroll);
    field(
        commands,
        output,
        handle,
        "Название модели",
        Field::Name,
        "Новая модель",
    );
    disclosure(commands, output, handle, "Экспорт и сетка", Section::Output);
    let output_settings = node(
        commands,
        output,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(7.0)
        },
    );
    action(
        commands,
        output_settings,
        handle,
        "Дополнительно сохранить OBJ",
        GenerationAction::Obj,
        false,
    );
    action(
        commands,
        output_settings,
        handle,
        "Оптимизировать сетку",
        GenerationAction::Simplify,
        false,
    );
    let faces = node(
        commands,
        output_settings,
        Node {
            width: percent(100),
            ..column(5.0)
        },
    );
    field(
        commands,
        faces,
        handle,
        "Цель · треугольники 100–1 000 000",
        Field::Faces,
        "40000",
    );
    let system = node(
        commands,
        scroll,
        Node {
            width: percent(100),
            ..column(6.0)
        },
    );
    disclosure(commands, system, handle, "Система", Section::System);
    let system_settings = node(
        commands,
        system,
        Node {
            display: Display::None,
            width: percent(100),
            ..column(6.0)
        },
    );
    let launch = card(commands, left);
    let error = text(commands, launch, handle, "", 11.0, ERROR);
    let generate = action(
        commands,
        launch,
        handle,
        "Сгенерировать 3D модель",
        GenerationAction::Generate,
        true,
    );
    commands.entity(generate).insert((
        InteractionDisabled,
        Node {
            width: percent(100),
            height: px(38),
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(px(7)),
            ..row(5.0)
        },
    ));
    let launch_hint = text(
        commands,
        launch,
        handle,
        "Запуск станет доступен после подключения модели.",
        11.0,
        MUTED,
    );

    let result = surface(
        commands,
        body,
        Node {
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            height: percent(100),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            padding: UiRect::all(px(24)),
            ..column(12.0)
        },
        WELL,
    );
    commands.entity(result).insert(BorderColor::all(EDGE));
    let result_heading = node(
        commands,
        result,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..row(10.0)
        },
    );
    text(
        commands,
        result_heading,
        handle,
        "Предпросмотр модели",
        14.0,
        TEXT,
    );
    let placeholder = node(
        commands,
        result,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..column(16.0)
        },
    );
    waiting::illustration(commands, placeholder);
    text(
        commands,
        placeholder,
        handle,
        "Здесь появится модель",
        19.0,
        TEXT,
    );
    let description = text(
        commands,
        placeholder,
        handle,
        "Выберите изображение и нажмите «Сгенерировать».",
        13.0,
        MUTED,
    );
    commands
        .entity(description)
        .insert(TextLayout::justify(Justify::Center));

    let picker = surface(
        commands,
        root,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::all(px(30)),
            ..default()
        },
        Color::srgba(0.0, 0.0, 0.0, 0.7),
    );
    commands.entity(picker).insert(ZIndex(50));
    let dialog = surface(
        commands,
        picker,
        Node {
            width: px(600),
            max_width: percent(95),
            max_height: percent(90),
            padding: UiRect::all(px(20)),
            border_radius: BorderRadius::all(px(12)),
            ..column(12.0)
        },
        PANEL,
    );
    let picker_header = node(
        commands,
        dialog,
        Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            ..row(8.0)
        },
    );
    heading(commands, picker_header, handle, "Изображения проекта");
    action(
        commands,
        picker_header,
        handle,
        "Закрыть",
        GenerationAction::Dismiss,
        false,
    );
    let choices = node(
        commands,
        dialog,
        Node {
            width: percent(100),
            max_height: px(460),
            overflow: Overflow::scroll_y(),
            ..column(4.0)
        },
    );
    commands.entity(choices).insert(Scroller);
    action(
        commands,
        system_settings,
        handle,
        "Проверить среду",
        GenerationAction::CheckModel,
        false,
    );
    run::setup(
        commands,
        launch,
        result,
        placeholder,
        handle,
        launch_hint,
        result_heading,
    );
    installation::setup(commands, root, handle, body, model_badge);
    run::service_setup(commands, root, body, handle);
    commands.insert_resource(GenerationUi {
        root,
        advanced,
        path_settings,
        output_settings,
        system_settings,
        source_progress,
        source_fill,
        faces,
        seed,
        picture,
        empty,
        source_label,
        error,
        picker,
        choices,
        picker_revision: None,
        font: handle.clone(),
        path_input,
        advanced_open: false,
        path_open: false,
        output_open: false,
        system_open: false,
        picking: false,
        requested: None,
        loaded: None,
        task: None,
        texture: None,
        dimensions: None,
        problem: String::new(),
    });
}

fn select(editor: &mut Editor, ui: &mut GenerationUi, path: PathBuf) {
    if !Workspace::accepts(&path) {
        ui.problem = "Поддерживаются PNG, JPG и WebP".into();
        return;
    }
    editor.generation.source = Some(path);
    editor.generation.source_ready = false;
    editor.drawer = false;
    // Also retry decoding when the same source file has been edited externally.
    ui.requested = None;
    ui.problem.clear();
    ui.picking = false;
}

fn actions(
    mut events: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    browser: Res<Browser>,
    mut ui: ResMut<GenerationUi>,
    mut focus: ResMut<InputFocus>,
    inputs: Query<&EditableText>,
) {
    for event in events.read() {
        let Action::Generation(action) = *event else {
            continue;
        };
        if !available(&editor, action) {
            continue;
        }
        match action {
            GenerationAction::Open | GenerationAction::Toggle | GenerationAction::Close => {
                ui.picking = false;
                focus.clear();
            }
            GenerationAction::Pick => {
                ui.picking = true;
                ui.picker_revision = None;
                focus.clear();
            }
            GenerationAction::Dismiss => {
                ui.picking = false;
                editor.drawer = false;
                focus.clear();
            }
            GenerationAction::Choose(index, revision) => {
                if revision == browser.scan_revision
                    && let Some(file) = browser.files.get(index)
                {
                    let path = editor.root.join(&file.path);
                    select(&mut editor, &mut ui, path);
                }
            }
            GenerationAction::LoadPath => {
                if let Ok(input) = inputs.get(ui.path_input) {
                    let path = input
                        .value()
                        .to_string()
                        .trim()
                        .trim_matches('"')
                        .to_owned();
                    if path.is_empty() {
                        ui.problem = "Укажите путь к изображению".into();
                    } else {
                        let path = PathBuf::from(path);
                        let path = if path.is_absolute() {
                            path
                        } else {
                            editor.root.join(path)
                        };
                        select(&mut editor, &mut ui, path);
                    }
                }
            }
            GenerationAction::Clear => {
                editor.generation.source = None;
                editor.generation.source_ready = false;
                ui.problem.clear();
            }
            GenerationAction::Quality(value) => {
                editor.generation.set_quality(value);
                ui.problem.clear();
            }
            GenerationAction::Advanced => ui.advanced_open = !ui.advanced_open,
            GenerationAction::Section(section) => match section {
                Section::Path => ui.path_open = !ui.path_open,
                Section::Output => ui.output_open = !ui.output_open,
                Section::System => ui.system_open = !ui.system_open,
                Section::Advanced => ui.advanced_open = !ui.advanced_open,
                _ => {}
            },
            GenerationAction::RandomSeed => {
                editor.generation.random_seed = !editor.generation.random_seed
            }
            GenerationAction::Simplify => editor.generation.simplify = !editor.generation.simplify,
            GenerationAction::Obj => editor.generation.obj = !editor.generation.obj,
            GenerationAction::ToggleWireframe => {
                editor.generation.wireframe = !editor.generation.wireframe;
            }
            GenerationAction::Generate
            | GenerationAction::StartModel
            | GenerationAction::StopModel
            | GenerationAction::DownloadModel
            | GenerationAction::CancelModelDownload
            | GenerationAction::CheckModel
            | GenerationAction::CancelGeneration
            | GenerationAction::ImportResult
            | GenerationAction::OpenResult
            | GenerationAction::FitResult => {}
        }
    }
}

fn drops(
    mut events: MessageReader<FileDragAndDrop>,
    mut editor: ResMut<Editor>,
    mut ui: ResMut<GenerationUi>,
) {
    for event in events.read() {
        if editor.generation.active
            && !editor.drawer
            && editor.pending.is_none()
            && !editor.browser_popup
            && let FileDragAndDrop::DroppedFile { path_buf, .. } = event
        {
            select(&mut editor, &mut ui, path_buf.clone());
        }
    }
}

fn apply_field(draft: &mut Workspace, kind: Field, value: &str) -> bool {
    let integer = value.trim().parse::<u32>();
    match kind {
        Field::Path => true,
        Field::Name => {
            if crate::editor::browser_files::validate_name(value.trim()).is_err() {
                false
            } else {
                draft.name = value.into();
                true
            }
        }
        Field::Seed => value.trim().parse::<u64>().map(|v| draft.seed = v).is_ok(),
        Field::Guidance => value
            .trim()
            .replace(',', ".")
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && (1.0..=20.0).contains(v))
            .map(|v| draft.guidance = v)
            .is_some(),
        _ => {
            let (min, max) = match kind {
                Field::Steps => (1, 100),
                Field::Resolution => (16, 512),
                Field::Chunks => (1000, 5_000_000),
                Field::Faces => (100, 1_000_000),
                _ => unreachable!(),
            };
            let Ok(value) = integer else {
                return false;
            };
            if !(min..=max).contains(&value) {
                return false;
            }
            match kind {
                Field::Steps => draft.steps = value,
                Field::Resolution => draft.resolution = value,
                Field::Chunks => draft.chunks = value,
                Field::Faces => draft.target_faces = value,
                _ => {}
            }
            true
        }
    }
}

fn refresh(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    browser: Res<Browser>,
    mut ui: ResMut<GenerationUi>,
    focus: Res<InputFocus>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut fields: Query<(Entity, &Field, &mut EditableText, &mut BorderColor)>,
    mut pictures: Query<&mut ImageNode, Without<SourceDrop>>,
    mut images: ResMut<Assets<Image>>,
    mut folds: Query<(&FoldIcon, &mut UiTransform)>,
    drag: Res<AssetDrag>,
    mut sources: Query<
        (
            &mut SourceDrop,
            &mut BorderColor,
            &ComputedNode,
            &mut ImageNode,
        ),
        Without<Field>,
    >,
) {
    let draft = &mut editor.generation;
    for (FoldIcon(section), mut icon) in &mut folds {
        let opened = match section {
            Section::Path => ui.path_open,
            Section::Output => ui.output_open,
            Section::System => ui.system_open,
            Section::Advanced => ui.advanced_open,
            _ => continue,
        };
        icon.set_if_neq(UiTransform::from_rotation(Rot2::radians(if opened {
            std::f32::consts::FRAC_PI_2
        } else {
            0.0
        })));
    }
    for (mut source, mut border, computed, mut checker) in &mut sources {
        // The tiled-image renderer ignores corner radii. Repeating UVs on the
        // regular image renderer preserves both rounded corners and cell size.
        if !computed.is_empty() {
            let rect = Some(Rect::from_corners(
                Vec2::ZERO,
                computed.content_box().size() * computed.inverse_scale_factor(),
            ));
            if checker.rect != rect {
                checker.rect = rect;
            }
        }
        source.enabled = draft.active && !ui.picking;
        border.set_if_neq(BorderColor::all(
            if source.enabled
                && drag.active()
                && drag.target == Some(drawer::DropTarget::GenerationSource)
            {
                GREEN
            } else {
                EDGE
            },
        ));
    }
    for (entity, shown) in [
        (ui.root, draft.active),
        (ui.advanced, ui.advanced_open),
        (ui.path_settings, ui.path_open),
        (ui.output_settings, ui.output_open),
        (ui.system_settings, ui.system_open),
        (ui.faces, draft.simplify),
        (ui.seed, !draft.random_seed),
        (ui.picker, ui.picking && draft.active),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if shown { Display::Flex } else { Display::None };
            if node.display != display {
                node.display = display;
            }
        }
    }
    if !draft.active {
        progress::set(&mut commands, ui.source_fill, progress::Value::Hidden);
        ui.picking = false;
        return;
    }
    let mut invalid = false;
    for (entity, kind, mut input, mut border) in &mut fields {
        if focus.get() == Some(entity) {
            let valid = apply_field(draft, *kind, &input.value().to_string());
            *border = BorderColor::all(if valid { FOCUS } else { ERROR });
            invalid |= !valid;
        } else {
            let value = match kind {
                Field::Path => draft
                    .source
                    .as_ref()
                    .map(|v| v.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                Field::Name => draft.name.clone(),
                Field::Steps => draft.steps.to_string(),
                Field::Guidance => draft.guidance.to_string(),
                Field::Resolution => draft.resolution.to_string(),
                Field::Chunks => draft.chunks.to_string(),
                Field::Seed => draft.seed.to_string(),
                Field::Faces => draft.target_faces.to_string(),
            };
            // Leave an unsubmitted source path intact until the user clicks Open.
            if *kind != Field::Path && input.value() != value.as_str() {
                input.editor.set_text(&value);
            }
            *border = BorderColor::all(EDGE);
        }
    }
    draft.config_valid = !invalid;
    if ui.requested != draft.source {
        ui.task = None;
        ui.requested = draft.source.clone();
        ui.loaded = None;
        draft.source_ready = false;
        ui.dimensions = None;
        if let Some(texture) = ui.texture.take() {
            images.remove(texture.id());
        }
        ui.problem.clear();
        if let Some(path) = draft.source.clone() {
            if let Ok((_, _, mut input, _)) = fields.get_mut(ui.path_input) {
                input.editor.set_text(&path.to_string_lossy());
            }
            ui.task = Some(
                AsyncComputeTaskPool::get()
                    .spawn(async move { image_previews::decode_image(&path, UVec2::splat(512)) }),
            );
        }
    }
    if let Some(result) = ui.task.as_mut().and_then(|task| block_on(poll_once(task))) {
        ui.task = None;
        match result {
            Ok(decoded) => {
                ui.dimensions = Some(decoded.original);
                ui.texture = Some(image_previews::texture(decoded, &mut images));
                ui.loaded = ui.requested.clone();
                draft.source_ready = true;
            }
            Err(error) => {
                ui.problem = format!("Не удалось открыть изображение · {error}");
            }
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.picture) {
        node.display = if ui.texture.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(size) = ui.dimensions {
            let fit = (280.0 / size.x as f32).min(140.0 / size.y as f32);
            node.width = px(size.x as f32 * fit);
            node.height = px(size.y as f32 * fit);
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.empty) {
        node.display = if ui.texture.is_none() && ui.task.is_none() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut node) = nodes.get_mut(ui.source_progress) {
        node.display = if ui.task.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
    progress::set(
        &mut commands,
        ui.source_fill,
        if ui.task.is_some() {
            progress::Value::Busy
        } else {
            progress::Value::Hidden
        },
    );
    if let Ok(mut picture) = pictures.get_mut(ui.picture) {
        picture.image = ui.texture.clone().unwrap_or_default();
    }
    let caption = match (&draft.source, ui.dimensions) {
        (Some(path), Some(size)) => format!(
            "{} · {} × {}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            size.x,
            size.y
        ),
        (Some(_), None) if ui.task.is_some() => "Загрузка изображения…".into(),
        _ => "PNG · JPG · WebP".into(),
    };
    if let Ok(mut label) = labels.get_mut(ui.source_label) {
        if label.0 != caption {
            label.0 = caption;
        }
    }
    let problem = if invalid {
        "Проверьте значение в выделенном поле"
    } else {
        &ui.problem
    };
    if let Ok(mut label) = labels.get_mut(ui.error) {
        if label.0 != problem {
            label.0 = problem.into();
        }
    }
    if let Ok(mut node) = nodes.get_mut(ui.error) {
        node.display = if problem.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
    }
    if ui.picking && ui.picker_revision != Some(browser.scan_revision) {
        commands.entity(ui.choices).despawn_children();
        ui.picker_revision = Some(browser.scan_revision);
        let mut count = 0;
        for (index, file) in browser
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| Workspace::accepts(&f.path))
        {
            let item = action(
                &mut commands,
                ui.choices,
                &ui.font,
                &file.path.to_string_lossy(),
                GenerationAction::Choose(index, browser.scan_revision),
                false,
            );
            commands.entity(item).insert(Node {
                width: percent(100),
                min_height: px(34),
                height: px(34),
                padding: UiRect::horizontal(px(10)),
                overflow: Overflow::clip(),
                ..row(6.0)
            });
            count += 1;
        }
        if count == 0 {
            text(
                &mut commands,
                ui.choices,
                &ui.font,
                "В проекте пока нет PNG, JPG или WebP.\nПеретащите файл из Проводника или укажите путь.",
                13.0,
                MUTED,
            );
        }
    }
}
