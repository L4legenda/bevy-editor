//! WGSL node source editor and asynchronous GPU validation.
use super::*;
use crate::editor::ui::control_hints;
use bevy::{
    input::{ButtonState, keyboard::KeyboardInput},
    input_focus::{FocusCause, FocusLost, FocusedInput, InputFocus},
    picking::events::PointerClick,
    render::renderer::{RenderDevice, RenderQueue},
    tasks::IoTaskPool,
    text::{FontCx, LayoutCx, TextEdit},
};
use bevy_scene_format::{Revision, atomic_write};
use material_painter_graph::{Graph, Library, Runtime, def, shading};
use std::{
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};

use node_library::SOURCE_LIMIT;
#[path = "model_paint_shader_parameters.rs"]
mod parameters;

const DRAFT_ID: &str = "user/draft";
const INPUTS: [(&str, &str); 5] = [
    ("noise", "Шум"),
    ("gradient", "Градиент"),
    ("checker", "Шахматная сетка"),
    ("cells", "Ячейки"),
    ("tiles", "Плитки"),
];

const SNIPPETS: [(&str, &str, &str); 14] = [
    (
        "@param float",
        "Дробный параметр",
        "// @param amount: float(0, 1) = 0.5\n",
    ),
    (
        "@param int",
        "Целочисленный параметр",
        "// @param count: int(1, 16) = 4\n",
    ),
    (
        "@param color",
        "Параметр цвета",
        "// @param tint: color = 1, 1, 1\n",
    ),
    (
        "@param enum",
        "Список вариантов",
        "// @param mode: enum(First, Second) = First\n",
    ),
    (
        "@param bool",
        "Переключатель",
        "// @param flip: bool = false\n",
    ),
    (
        "@input gray",
        "Вход для маски",
        "// @input mask: gray = 1\n",
    ),
    (
        "@input color",
        "Вход для цвета",
        "// @input input: color = 0.5\n",
    ),
    (
        "perlin",
        "Бесшовный градиентный шум",
        "perlin(uv, 8.0, 0.0)",
    ),
    ("fbm2", "Фрактальный шум", "fbm2(uv, 6.0, 5, 0.5, 0.0)"),
    (
        "value_noise",
        "Мягкий бесшовный шум",
        "value_noise(uv, 8.0, 0.0)",
    ),
    ("cells", "Клеточный шум Ворли", "cells(uv, 8.0, 1.0, 0.0)"),
    (
        "rotate2",
        "Поворот UV в радианах",
        "rotate2(uv - 0.5, 0.5) + 0.5",
    ),
    ("px()", "Размер одного текселя в UV", "px()"),
    ("luma", "Яркость цвета", "luma(c.rgb)"),
];

const TEMPLATES: [(&str, &str); 5] = [
    (
        "Фильтр",
        r#"// @name My Filter
// @category Custom
// @output color
// @input input: color = 0.5
// @param tint: color = 0.35, 0.75, 1
// @param amount: float(0, 1) = 0.7

fn node(uv: vec2<f32>) -> vec4<f32> {
    let color = in_input(uv);
    let tinted = params.tint * luma(color.rgb) * 1.6;
    return vec4<f32>(mix(color.rgb, tinted, params.amount), color.a);
}
"#,
    ),
    (
        "Генератор",
        r#"// @name My Pattern
// @category Custom
// @output gray
// @param scale: int(1, 64) = 6
// @param rings: float(1, 20) = 6
// @param seed: int(0, 1000) = 0

fn node(uv: vec2<f32>) -> vec4<f32> {
    let c = cells(uv, f32(params.scale), 1.0, f32(params.seed));
    let n = fbm2(uv, f32(params.scale) * 2.0, 4, 0.5, f32(params.seed) + 7.0);
    return vec4<f32>(0.5 + 0.5 * sin((c.x + n * 0.3) * params.rings * TAU));
}
"#,
    ),
    (
        "Смешивание",
        r#"// @name My Blend
// @category Custom
// @output color
// @input top: color = 1
// @input bottom: color = 0
// @input mask: gray = 0.5
// @param opacity: float(0, 1) = 1

fn node(uv: vec2<f32>) -> vec4<f32> {
    let amount = clamp(in_mask(uv) * params.opacity, 0.0, 1.0);
    return mix(in_bottom(uv), in_top(uv), amount);
}
"#,
    ),
    (
        "Маска по карте",
        r#"// @name My Mask
// @category Custom
// @output gray
// @param map: meshmap
// @param threshold: float(0, 1) = 0.5
// @param softness: float(0, 0.5) = 0.1
// @param invert: bool = false

fn node(uv: vec2<f32>) -> vec4<f32> {
    let m = smoothstep(params.threshold - params.softness, params.threshold + params.softness, map(uv));
    return vec4<f32>(select(m, 1.0 - m, params.invert));
}
"#,
    ),
    (
        "Шейдинг",
        r#"// @name My Shading
// @category Custom
// @shading
// @output color
// @param color: color = 0.35, 0.75, 1
// @param strength: float(0, 4) = 1

fn node(c: Shade) -> vec4<f32> {
    return vec4<f32>(params.color * params.strength, 1.0);
}
"#,
    ),
];

#[derive(Clone)]
enum Command {
    Source(Arc<str>, Option<String>),
    Input(usize),
    Open,
    Save,
    SaveCurrent,
    Revert,
    ToggleSnippets,
    Insert(&'static str),
    Publish,
    Update,
    Remove,
    CancelRemove,
    ConfirmRemove,
    ResetLayout,
    ToggleTiles,
    Parameter(parameters::Edit),
}

enum FileResult {
    Opened(String, PathBuf),
    Saved(PathBuf),
    SavedDraft(PathBuf, Arc<str>),
    SavedCurrent(PathBuf, Arc<str>),
    PreviewImage {
        source: String,
        name: String,
        bitmap: texture_fill::Bitmap,
    },
    Published(PathBuf, String),
    Updated(Installed),
    Removed(String, PathBuf),
}

#[derive(Clone)]
struct Installed {
    id: String,
    expected: Arc<str>,
}
#[derive(Clone)]
struct Request {
    source: String,
    input: usize,
    maps: Option<Arc<bake::Maps>>,
    surface: Option<Arc<material_graph::surface::Source>>,
    uses_surface: bool,
    parameters: Arc<parameters::Values>,
}

enum ResultView {
    Texture(image::RgbaImage),
    Shading(shader_preview::ShadingSource),
}

#[derive(Resource)]
pub(in crate::editor::ui::model_viewer) struct Maker {
    root: Entity,
    source: Entity,
    input_title: Entity,
    texture_panel: Entity,
    library: Entity,
    library_revision: Option<usize>,
    font: Handle<Font>,
    remove_button: Entity,
    revert_button: Entity,
    snippets: Entity,
    snippets_visible: bool,
    selection_before_blur: Option<Range<usize>>,
    remove_confirmation: Entity,
    preview: Entity,
    tiles: [Entity; 4],
    tile_toggle: Entity,
    tiled: bool,
    error_line: Option<usize>,
    status: Entity,
    file_status: Entity,
    command: Option<Command>,
    selected: Option<Installed>,
    opened: Option<PathBuf>,
    baseline: Arc<str>,
    pending_save: Option<String>,
    confirming_removal: bool,
    input: usize,
    parameter_panel: Entity,
    parameters: parameters::State,
    file_task: Option<Task<Result<Option<FileResult>, String>>>,
    preview_task: Option<(u64, Task<Result<ResultView, String>>)>,
    request: Option<Request>,
    validated: Option<String>,
    draft: Option<(u64, shader_preview::ShadingSource)>,
    revision: u64,
    due: Option<f64>,
    #[cfg(test)]
    pub(in crate::editor::ui::model_viewer::paint) node_directory: Option<PathBuf>,
}

impl Maker {
    pub(in crate::editor::ui::model_viewer::paint) fn draft(
        &self,
    ) -> Option<(u64, &shader_preview::ShadingSource)> {
        self.draft
            .as_ref()
            .map(|(revision, draft)| (*revision, draft))
    }

    pub(in crate::editor::ui::model_viewer) fn shows_model(&self) -> bool {
        self.draft.is_some()
    }

    pub(super) fn preview_pane(&self) -> Entity {
        self.texture_panel
    }

    pub(super) fn error_line(&self) -> Option<usize> {
        self.error_line
    }
}

fn control(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: Command,
) -> Entity {
    let button = graph_canvas::local_button(commands, parent, font, label);
    commands
        .entity(button)
        .observe(move |event: On<PointerClick>, mut maker: ResMut<Maker>| {
            if event.button == PointerButton::Primary {
                maker.command = Some(action.clone());
            }
        });
    button
}

pub(super) fn setup(
    commands: &mut Commands,
    overlay: Entity,
    font: &Handle<Font>,
    fonts: &mut Assets<Font>,
) {
    let root = surface(
        commands,
        overlay,
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            left: px(44),
            right: px(0),
            top: px(118),
            bottom: px(SHELF_HEIGHT),
            padding: UiRect::all(px(8)),
            ..column(6.)
        },
        WELL,
    );
    commands.entity(root).insert(ZIndex(20));
    let title = node(
        commands,
        root,
        Node {
            flex_shrink: 0.,
            flex_wrap: FlexWrap::Wrap,
            ..row(5.)
        },
    );
    text(commands, title, font, "Редактор узлов WGSL", 14., TEXT);
    control(commands, title, font, "Открыть .wgsl…", Command::Open);
    let save = control(
        commands,
        title,
        font,
        "Сохранить правки",
        Command::SaveCurrent,
    );
    control_hints::attach(commands, save, "Ctrl+S");
    control(commands, title, font, "Сохранить копию…", Command::Save);
    let revert_button = control(commands, title, font, "Вернуть исходник", Command::Revert);
    control(
        commands,
        title,
        font,
        "Вставить фрагмент",
        Command::ToggleSnippets,
    );
    control(
        commands,
        title,
        font,
        "Сбросить раскладку WGSL",
        Command::ResetLayout,
    );
    control(
        commands,
        title,
        font,
        "Добавить в библиотеку",
        Command::Publish,
    );
    let remove_button = control(commands, title, font, "Убрать из палитры…", Command::Remove);
    for entity in [remove_button, revert_button] {
        commands
            .entity(entity)
            .entry::<Node>()
            .and_modify(|mut node| node.display = Display::None);
    }
    let snippets = node(
        commands,
        root,
        Node {
            display: Display::None,
            flex_shrink: 0.,
            flex_wrap: FlexWrap::Wrap,
            ..row(4.)
        },
    );
    for (label, hint, snippet) in SNIPPETS {
        let button = control(commands, snippets, font, label, Command::Insert(snippet));
        control_hints::attach(commands, button, hint);
    }
    let remove_confirmation = node(
        commands,
        root,
        Node {
            display: Display::None,
            flex_wrap: FlexWrap::Wrap,
            ..row(5.)
        },
    );
    text(
        commands,
        remove_confirmation,
        font,
        "Убрать узел из палитры?",
        12.,
        TEXT,
    );
    control(
        commands,
        remove_confirmation,
        font,
        "Убрать узел",
        Command::ConfirmRemove,
    );
    control(
        commands,
        remove_confirmation,
        font,
        "Отмена",
        Command::CancelRemove,
    );
    let file_status = text(commands, root, font, "", 11., MUTED);
    let body = node(
        commands,
        root,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            ..column(0.)
        },
    );
    let library = node(
        commands,
        body,
        Node {
            width: percent(100),
            min_width: px(0),
            min_height: px(0),
            flex_grow: 1.,
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(6)),
            ..column(4.)
        },
    );
    commands.entity(library).insert(Scroller);
    let editor = surface(
        commands,
        body,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_width: px(0),
            min_height: px(0),
            overflow: Overflow::clip(),
            padding: UiRect::all(px(8)),
            ..row(6.)
        },
        Color::srgb(0.047, 0.056, 0.072),
    );
    let face = fonts.add(Font::from_bytes(
        include_bytes!("../../vendor/photocraft/assets/fonts/JetBrainsMono-Regular.ttf").to_vec(),
    ));
    let source = shader_code::setup(commands, editor, TEMPLATES[0].1, face);
    commands.entity(source).observe(
        |event: On<FocusLost>, fields: Query<&EditableText>, mut maker: ResMut<Maker>| {
            maker.selection_before_blur = fields
                .get(event.entity)
                .ok()
                .map(|field| field.editor().raw_selection().text_range());
        },
    );
    commands.entity(source).observe(
        |mut event: On<FocusedInput<KeyboardInput>>,
         keys: Res<ButtonInput<KeyCode>>,
         mut maker: ResMut<Maker>| {
            let input = &event.input;
            if !input.repeat
                && input.state == ButtonState::Pressed
                && input.key_code == KeyCode::KeyS
                && keys.any_pressed([
                    KeyCode::ControlLeft,
                    KeyCode::ControlRight,
                    KeyCode::SuperLeft,
                    KeyCode::SuperRight,
                ])
            {
                maker.command = Some(Command::SaveCurrent);
                event.propagate(false);
            }
        },
    );

    let right = node(
        commands,
        body,
        Node {
            flex_grow: 1.,
            width: percent(100),
            min_width: px(0),
            min_height: px(0),
            ..column(6.)
        },
    );
    let input_title = text(commands, right, font, "", 13., TEXT);
    let tile_toggle = control(commands, right, font, "Плитка 2×2", Command::ToggleTiles);
    let preview = commands
        .spawn((
            ChildOf(right),
            Node {
                display: Display::None,
                width: percent(100),
                max_width: px(256),
                aspect_ratio: Some(1.),
                flex_shrink: 0.,
                ..default()
            },
            ImageNode::default(),
            AccessibleLabel("Предпросмотр узла WGSL".into()),
        ))
        .id();
    let tiles = std::array::from_fn(|i| {
        commands
            .spawn((
                ImageNode::default(),
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    left: percent(((i % 2) * 50) as f32),
                    top: percent(((i / 2) * 50) as f32),
                    width: percent(50),
                    height: percent(50),
                    ..default()
                },
                ChildOf(preview),
                Pickable::IGNORE,
            ))
            .id()
    });
    let parameter_panel = node(
        commands,
        right,
        Node {
            display: Display::None,
            width: percent(100.),
            min_height: px(0.),
            max_height: percent(42.),
            overflow: Overflow::scroll_y(),
            ..column(5.)
        },
    );
    commands.entity(parameter_panel).insert(Scroller);
    let status = text(commands, root, font, "", 12., TEXT);
    commands
        .entity(status)
        .insert(Scroller)
        .entry::<Node>()
        .and_modify(|mut node| {
            node.max_height = px(104);
            node.overflow = Overflow::scroll_y();
        });
    commands.insert_resource(Maker {
        root,
        source,
        input_title,
        texture_panel: right,
        library,
        library_revision: None,
        font: font.clone(),
        remove_button,
        revert_button,
        snippets,
        snippets_visible: false,
        selection_before_blur: None,
        remove_confirmation,
        preview,
        tiles,
        tile_toggle,
        tiled: false,
        error_line: None,
        status,
        file_status,
        command: None,
        selected: None,
        opened: None,
        baseline: TEMPLATES[0].1.into(),
        pending_save: None,
        confirming_removal: false,
        input: 0,
        parameter_panel,
        parameters: parameters::State::default(),
        file_task: None,
        preview_task: None,
        request: None,
        validated: None,
        draft: None,
        revision: 0,
        due: None,
        #[cfg(test)]
        node_directory: None,
    });
    sidebar::setup_shader(commands, body, library, editor, right);
}

fn tools(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    text(commands, parent, font, "Шаблоны узлов", 12., TEXT);
    for (label, source) in TEMPLATES {
        control(
            commands,
            parent,
            font,
            label,
            Command::Source(source.into(), None),
        );
    }
    text(commands, parent, font, "Вход для предпросмотра", 12., TEXT);
    for (index, (_, label)) in INPUTS.into_iter().enumerate() {
        control(commands, parent, font, label, Command::Input(index));
    }
    text(commands, parent, font, "Исходники узлов", 12., TEXT);
    let (library, archived) = node_library::palette();
    for def in library
        .defs
        .iter()
        .filter(|def| node_library::visible(&def.id, &archived))
    {
        control(
            commands,
            parent,
            font,
            &def.name,
            Command::Source(
                def.source.as_str().into(),
                def.id.starts_with("user/").then(|| def.id.clone()),
            ),
        );
    }
}

fn text_value(labels: &mut Query<&mut Text>, entity: Entity, value: &str) {
    if let Ok(mut label) = labels.get_mut(entity)
        && label.0 != value
    {
        label.0 = value.to_owned();
    }
}

fn open_source(path: &Path) -> Result<String, String> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wgsl"))
    {
        return Err("Выберите файл .wgsl".into());
    }
    let bytes = material_graph::file::read_limited(path, SOURCE_LIMIT)?;
    String::from_utf8(bytes).map_err(|_| "Ожидался текст WGSL в UTF-8".into())
}

fn save_source(directory: &Path, source: &str) -> Result<PathBuf, String> {
    if source.len() > SOURCE_LIMIT {
        return Err("Исходник WGSL слишком большой".into());
    }
    let definition = def::parse(DRAFT_ID, source).ok();
    let name = definition
        .as_ref()
        .map_or("WGSL", |definition| definition.name.as_str());
    presets::publish_named(directory, "Узел", name, "wgsl", |file| {
        file.write_all(source.as_bytes())
            .map_err(|error| error.to_string())
    })
}

fn save_open_source(path: &Path, expected: &str, source: &str) -> Result<(), String> {
    if source.len() > SOURCE_LIMIT {
        return Err("Исходник WGSL слишком большой".into());
    }
    atomic_write(path, source.as_bytes(), Revision::of(expected.as_bytes()))
        .map(|_| ())
        .map_err(|error| format!("Не удалось сохранить {} · {error}", path.display()))
}

fn file(action: Command, directory: PathBuf, source: String) -> Result<Option<FileResult>, String> {
    match action {
        Command::Open => {
            let Some(path) = crate::editor::asset_monitor::choose_file_path(&directory)
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            Ok(Some(FileResult::Opened(open_source(&path)?, path)))
        }
        Command::Save | Command::SaveCurrent => {
            if source.len() > SOURCE_LIMIT {
                return Err("Исходник WGSL слишком большой".into());
            }
            let Some(directory) = crate::project_manager::choose_folder("Папка для исходника WGSL")
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            let path = save_source(&directory, &source)?;
            Ok(Some(if matches!(action, Command::SaveCurrent) {
                FileResult::SavedDraft(path, source.into())
            } else {
                FileResult::Saved(path)
            }))
        }
        Command::Publish => Ok(Some(FileResult::Published(
            node_library::publish(&directory, &source)?,
            source,
        ))),
        _ => unreachable!(),
    }
}

fn render(
    request: Request,
    device: &RenderDevice,
    queue: &RenderQueue,
) -> Result<ResultView, String> {
    if request.source.len() > SOURCE_LIMIT {
        return Err("Исходник WGSL слишком большой".into());
    }
    let definition = def::parse(DRAFT_ID, &request.source)?;
    let shading = definition.shading;
    let mut graph = Graph::new("Предпросмотр узла");
    graph.res = 256;
    let input = INPUTS[request.input].0;
    let sink = if shading {
        shading::OUTPUT
    } else {
        material_painter_graph::OUTPUT_DEF
    };
    let builtin = material_graph::library();
    let mut library = Library {
        defs: vec![
            builtin.find(sink).unwrap().clone(),
            builtin.find(input).unwrap().clone(),
        ],
        errors: Vec::new(),
        user_dir: None,
    };
    let node = graph.add(DRAFT_ID, [0., 0.]);
    request
        .parameters
        .apply(graph.node_mut(node).unwrap(), &definition);
    if !shading && !definition.inputs.is_empty() {
        let feed = graph.add(input, [-240., 0.]);
        for port in &definition.inputs {
            graph.connect(feed, node, &port.name);
        }
    }
    library.defs.push(definition);
    let output = graph.add(sink, [240., 0.]);
    graph.connect(node, output, if shading { "color" } else { "input" });
    if shading {
        let source = shader_preview::ShadingSource::compile(&graph, &library)?;
        source.validate_node(device, node)?;
        return Ok(ResultView::Shading(source));
    }
    let used = material_graph::mesh_maps::used_with(&graph, &library);
    let maps = material_graph::mesh_maps::Textures::new(
        &used,
        request.maps.as_deref(),
        graph.res,
        device,
        queue,
    );
    let raw = device.wgpu_device();
    let mut runtime = Runtime::new(raw, queue, material_graph::assets());
    material_graph::file::bind_images(
        &mut graph,
        request.parameters.images(),
        &mut runtime,
        device,
        queue,
    )?;
    let surface = request.surface.as_ref().map(|source| {
        material_graph::surface::Textures::new(
            source,
            request.maps.as_deref(),
            graph.res,
            device,
            queue,
        )
    });
    runtime.evaluate(
        raw,
        queue,
        &graph,
        &library,
        &|id| maps.get(id),
        surface
            .as_ref()
            .map(material_graph::surface::Textures::inputs)
            .as_ref(),
        0,
        true,
    );
    if let Some(error) = runtime.errors.get(&node) {
        return Err(library.find(DRAFT_ID).unwrap().map_error(error));
    }
    if let Some((_, error)) = runtime.errors.iter().next() {
        return Err(error.clone());
    }
    let result = runtime.read_outputs(raw, queue, &graph, &library)?[0]
        .take()
        .ok_or("Узел не вернул текстуру")?;
    Ok(ResultView::Texture(result))
}

pub(in crate::editor::ui::model_viewer) fn refresh(
    mut commands: Commands,
    mut maker: ResMut<Maker>,
    editor: Res<Editor>,
    editing: Res<editing::State>,
    paint: Res<PaintState>,
    baked: Res<bake::State>,
    time: Res<Time<Real>>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
    mut fields: Query<&mut EditableText>,
    mut labels: Query<&mut Text>,
    mut nodes: Query<&mut Node>,
    mut preview: Query<&mut ImageNode>,
    mut images: ResMut<Assets<Image>>,
    workspace: (
        Res<sidebar::State>,
        ResMut<sidebar::ShaderDock>,
        ResMut<InputFocus>,
        ResMut<FontCx>,
        ResMut<LayoutCx>,
    ),
    mut backgrounds: Query<&mut BackgroundColor>,
) {
    let (shelf, mut dock, mut focus, mut fonts, mut layouts) = workspace;
    let active = editor
        .viewing_model
        .filter(|_| {
            editor.pending.is_none()
                && editor.viewing_image.is_none()
                && !editor.browser_popup
                && !editor.hdri_library_open
        })
        .and_then(|model| {
            let session = paint.sessions.get(&model)?;
            let doc = editing.documents.get(&model)?;
            (doc.mode == Mode::Paint && session.panel == Panel::Shader)
                .then_some((model, session, doc))
        });
    if let Ok(mut root) = nodes.get_mut(maker.root) {
        root.display = if active.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        root.bottom = px(if shelf.shelf_in_sidebar() {
            0.
        } else {
            SHELF_HEIGHT
        });
    }
    let Some((model, session, doc)) = active else {
        return;
    };
    let revision = node_library::revision();
    if maker.library_revision != Some(revision) {
        commands.entity(maker.library).despawn_related::<Children>();
        tools(&mut commands, maker.library, &maker.font);
        maker.library_revision = Some(revision);
    }
    let Ok(mut source) = fields.get_mut(maker.source) else {
        return;
    };
    if let Some(result) = maker
        .file_task
        .as_mut()
        .and_then(|task| block_on(poll_once(task)))
    {
        maker.file_task = None;
        let message = match result {
            Ok(Some(FileResult::Opened(code, path))) => {
                maker.selected = None;
                maker.baseline = Arc::from(code.as_str());
                maker.opened = Some(path.clone());
                maker.pending_save = None;
                maker.selection_before_blur = None;
                maker.confirming_removal = false;
                maker.parameters.new_source();
                source.editor_mut().set_text(&code);
                format!("Открыт · {}", path.display())
            }
            Ok(Some(FileResult::Saved(path))) => format!("Сохранено · {}", path.display()),
            Ok(Some(FileResult::SavedDraft(path, code))) => {
                if source.value() == code.as_ref() {
                    maker.opened = Some(path.clone());
                    maker.baseline = code;
                }
                format!("Сохранено · {}", path.display())
            }
            Ok(Some(FileResult::SavedCurrent(path, code))) => {
                if maker.opened.as_ref() == Some(&path) {
                    maker.baseline = code;
                }
                format!("Правки сохранены · {}", path.display())
            }
            Ok(Some(FileResult::PreviewImage {
                source: requested,
                name,
                bitmap,
            })) => {
                if source.value() == requested.as_str() {
                    match maker.parameters.image(name, bitmap) {
                        Ok(()) => String::new(),
                        Err(error) => error,
                    }
                } else {
                    "Исходник изменился · выберите изображение снова".into()
                }
            }
            Ok(Some(FileResult::Published(path, code))) => {
                if source.value() == code.as_str() {
                    maker.opened = None;
                    maker.baseline = Arc::from(code.as_str());
                    maker.pending_save = None;
                    maker.selected = node_library::node_id(&path).ok().map(|id| Installed {
                        id,
                        expected: code.into(),
                    });
                }
                format!("Узел добавлен · {}", path.display())
            }
            Ok(Some(FileResult::Updated(updated))) => {
                if maker
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.id == updated.id)
                {
                    maker.baseline = updated.expected.clone();
                    maker.selected = Some(updated);
                }
                "Правки узла сохранены".into()
            }
            Ok(Some(FileResult::Removed(id, path))) => {
                if maker
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.id == id)
                {
                    maker.selected = None;
                    maker.pending_save = None;
                }
                format!("Узел убран из палитры · {}", path.display())
            }
            Ok(None) => String::new(),
            Err(error) => error,
        };
        text_value(&mut labels, maker.file_status, &message);
    }
    if maker
        .pending_save
        .as_deref()
        .is_some_and(|code| source.value() != code)
    {
        maker.pending_save = None;
        text_value(
            &mut labels,
            maker.file_status,
            "Исходник изменился · сохраните правки ещё раз",
        );
    }
    if maker.command.is_none()
        && maker.file_task.is_none()
        && maker.pending_save.is_some()
        && maker
            .request
            .as_ref()
            .is_some_and(|request| source.value() == request.source.as_str())
        && maker.preview_task.is_none()
        && maker.due.is_none()
    {
        maker.pending_save = None;
        maker.command = Some(Command::Update);
    }
    if let Some(command) = maker.command.take() {
        match command {
            Command::Source(code, id) => {
                source.editor_mut().set_text(&code);
                maker.baseline = code.clone();
                maker.selected = id.map(|id| Installed { id, expected: code });
                maker.opened = None;
                maker.pending_save = None;
                maker.selection_before_blur = None;
                maker.confirming_removal = false;
                maker.parameters.new_source();
                text_value(&mut labels, maker.file_status, "");
            }
            Command::Input(index) => maker.input = index,
            Command::Parameter(parameters::Edit::PickImage(name)) if maker.file_task.is_none() => {
                let root = editor.project.asset_root.clone();
                let source = source.value().to_string();
                maker.file_task = Some(IoTaskPool::get().spawn(async move {
                    let Some(path) = crate::editor::asset_monitor::choose_file_path(&root)
                        .map_err(|error| error.to_string())?
                    else {
                        return Ok(None);
                    };
                    let bitmap = material_graph::file::read_bitmap(&path)?;
                    Ok(Some(FileResult::PreviewImage {
                        source,
                        name,
                        bitmap,
                    }))
                }));
            }
            Command::Parameter(edit) if !matches!(edit, parameters::Edit::PickImage(_)) => {
                let numeric = matches!(edit, parameters::Edit::Number(..));
                match maker.parameters.edit(edit) {
                    Ok(()) => {
                        if numeric {
                            focus.clear();
                        }
                        text_value(&mut labels, maker.file_status, "");
                    }
                    Err(error) => text_value(&mut labels, maker.file_status, &error),
                }
            }
            Command::SaveCurrent if maker.file_task.is_none() => {
                if source.value() == maker.baseline.as_ref()
                    && (maker.selected.is_some() || maker.opened.is_some())
                {
                    text_value(&mut labels, maker.file_status, "Нет новых правок");
                } else if let Some(path) = maker.opened.clone() {
                    let code: Arc<str> = source.value().to_string().into();
                    let expected = maker.baseline.clone();
                    maker.file_task = Some(IoTaskPool::get().spawn(async move {
                        save_open_source(&path, &expected, &code)?;
                        Ok(Some(FileResult::SavedCurrent(path, code)))
                    }));
                } else if maker.selected.is_some() {
                    maker.pending_save = Some(source.value().to_string());
                    text_value(
                        &mut labels,
                        maker.file_status,
                        "Ожидание проверки перед сохранением…",
                    );
                } else {
                    let root = editor.project.asset_root.clone();
                    let code = source.value().to_string();
                    maker.file_task = Some(
                        IoTaskPool::get()
                            .spawn(async move { file(Command::SaveCurrent, root, code) }),
                    );
                }
            }
            Command::Revert if maker.file_task.is_none() => {
                maker.pending_save = None;
                maker.selection_before_blur = None;
                source.editor_mut().set_text(&maker.baseline);
                text_value(&mut labels, maker.file_status, "Исходник восстановлен");
                focus.set(maker.source, FocusCause::Auto);
            }
            Command::ToggleSnippets => maker.snippets_visible = !maker.snippets_visible,
            Command::Insert(snippet) => {
                if let Some(range) = maker.selection_before_blur.take()
                    && focus.get() != Some(maker.source)
                {
                    source
                        .editor_mut()
                        .driver(&mut fonts, &mut layouts.0)
                        .select_byte_range(range.start, range.end);
                }
                source.queue_edit(TextEdit::Insert(snippet.into()));
                maker.snippets_visible = false;
                focus.set(maker.source, FocusCause::Auto);
            }
            Command::Open | Command::Save if maker.file_task.is_none() => {
                let root = editor.project.asset_root.clone();
                let code = source.value().to_string();
                maker.file_task =
                    Some(IoTaskPool::get().spawn(async move { file(command, root, code) }));
            }
            Command::Publish | Command::Update | Command::ConfirmRemove
                if maker.file_task.is_none() =>
            {
                maker.confirming_removal = false;
                let directory = node_library::directory();
                #[cfg(test)]
                let directory = maker.node_directory.clone().or(directory);
                if !matches!(command, Command::ConfirmRemove)
                    && !maker
                        .validated
                        .as_ref()
                        .is_some_and(|code| source.value() == code.as_str())
                {
                    text_value(
                        &mut labels,
                        maker.file_status,
                        "Исправьте ошибки и дождитесь проверки исходника",
                    );
                } else if let Some(directory) = directory {
                    let code = source.value().to_string();
                    let selected = maker.selected.clone();
                    if matches!(command, Command::Publish) || selected.is_some() {
                        text_value(&mut labels, maker.file_status, "Операция с узлом…");
                        maker.file_task = Some(IoTaskPool::get().spawn(async move {
                            match (command, selected) {
                                (Command::Update, Some(mut installed)) => {
                                    node_library::update(
                                        &directory,
                                        &installed.id,
                                        &installed.expected,
                                        &code,
                                    )?;
                                    installed.expected = code.into();
                                    Ok(Some(FileResult::Updated(installed)))
                                }
                                (Command::ConfirmRemove, Some(installed)) => {
                                    let path = node_library::remove(
                                        &directory,
                                        &installed.id,
                                        &installed.expected,
                                    )?;
                                    Ok(Some(FileResult::Removed(installed.id, path)))
                                }
                                (command, _) => file(command, directory, code),
                            }
                        }));
                    }
                } else {
                    text_value(
                        &mut labels,
                        maker.file_status,
                        "Папка пользовательских узлов недоступна",
                    );
                }
            }
            Command::Remove if maker.file_task.is_none() => {
                maker.confirming_removal = maker.selected.is_some()
            }
            Command::CancelRemove => maker.confirming_removal = false,
            Command::ResetLayout => dock.reset(),
            Command::ToggleTiles => maker.tiled = !maker.tiled,
            _ => {}
        }
    }
    for (entity, visible) in [
        (maker.remove_button, maker.selected.is_some()),
        (
            maker.revert_button,
            source.value() != maker.baseline.as_ref(),
        ),
        (maker.snippets, maker.snippets_visible),
    ] {
        if let Ok(mut node) = nodes.get_mut(entity) {
            node.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    if let Ok(mut node) = nodes.get_mut(maker.remove_confirmation) {
        node.display = if maker.confirming_removal {
            Display::Flex
        } else {
            Display::None
        };
    }
    let code = source.value();
    let maps = session
        .binding
        .and_then(|set| baked.maps(model, doc, set, session.texture_size));
    let previous = maker.request.as_ref();
    let changed = previous.is_none_or(|request| code != request.source.as_str());
    let new_code = changed.then(|| code.to_string());
    if let Some(source) = &new_code {
        maker.parameters.schema(source);
    }
    maker.parameters.maps(&doc.custom_bakes);
    if std::mem::take(&mut maker.parameters.dirty) {
        commands
            .entity(maker.parameter_panel)
            .despawn_related::<Children>();
        parameters::controls(
            &mut commands,
            maker.parameter_panel,
            &maker.font,
            &maker.parameters,
        );
        if let Ok(mut panel) = nodes.get_mut(maker.parameter_panel) {
            panel.display = if maker.parameters.visible() {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
    let previous = maker.request.as_ref();
    let uses_surface = new_code.as_ref().map_or_else(
        || previous.is_some_and(|request| request.uses_surface),
        |source| {
            source.lines().any(|line| {
                line.trim()
                    .strip_prefix("//")
                    .is_some_and(|tag| tag.trim() == "@surface")
            })
        },
    );
    let surface = if uses_surface {
        session.binding.and_then(|set| {
            previous
                .and_then(|request| request.surface.as_ref())
                .filter(|source| source.matches(doc, set))
                .or_else(|| {
                    session
                        .graph_surfaces
                        .get(&set)
                        .filter(|source| source.matches(doc, set))
                })
                .cloned()
                .or_else(|| material_graph::surface::Source::capture(doc, set).map(Arc::new))
        })
    } else {
        None
    };
    if changed
        || previous.is_some_and(|request| {
            request.input != maker.input
                || !Arc::ptr_eq(&request.parameters, &maker.parameters.values)
                || !material_graph::mesh_maps::same(request.maps.as_ref(), maps)
                || !material_graph::mesh_maps::same(request.surface.as_ref(), surface.as_ref())
        })
    {
        maker.request = Some(Request {
            source: new_code.unwrap_or_else(|| code.to_string()),
            input: maker.input,
            maps: maps.cloned(),
            surface,
            uses_surface,
            parameters: maker.parameters.values.clone(),
        });
        maker.revision += 1;
        maker.due = Some(time.elapsed_secs_f64() + 0.3);
        text_value(
            &mut labels,
            maker.input_title,
            &format!("Вход · {}", INPUTS[maker.input].1),
        );
        maker.error_line = None;
        text_value(&mut labels, maker.status, "Проверка WGSL…");
    }
    if let Some((revision, result)) = maker
        .preview_task
        .as_mut()
        .and_then(|(revision, task)| block_on(poll_once(task)).map(|result| (*revision, result)))
    {
        maker.preview_task = None;
        if revision == maker.revision {
            maker.validated = result
                .as_ref()
                .ok()
                .and_then(|_| maker.request.as_ref().map(|request| request.source.clone()));
            let message = match result {
                Ok(ResultView::Texture(result)) => {
                    maker.draft = None;
                    if let Ok(mut target) = preview.get_mut(maker.preview) {
                        target.image = images.add(Image::from_dynamic(
                            image::DynamicImage::ImageRgba8(result),
                            true,
                            RenderAssetUsages::default(),
                        ));
                    }
                    if let Ok(mut node) = nodes.get_mut(maker.preview) {
                        node.display = Display::Flex;
                    }
                    "Текстурный узел собран".to_owned()
                }
                Ok(ResultView::Shading(source)) => {
                    maker.draft = Some((revision, source));
                    if let Ok(mut node) = nodes.get_mut(maker.preview) {
                        node.display = Display::None;
                    }
                    "Узел шейдинга прошёл проверку WGSL".to_owned()
                }
                Err(error) => {
                    maker.error_line = shader_code::error_line(&error);
                    format!("Ошибка WGSL · {error}")
                }
            };
            text_value(&mut labels, maker.status, &message);
        }
    }
    let tiles_visible = maker.tiled && !maker.shows_model();
    let texture = preview.get_mut(maker.preview).ok().map(|mut target| {
        let tint = if tiles_visible {
            Color::NONE
        } else {
            Color::WHITE
        };
        if target.color != tint {
            target.color = tint;
        }
        target.image.clone()
    });
    for entity in maker.tiles {
        if let Ok(mut node) = nodes.get_mut(entity) {
            let display = if tiles_visible {
                Display::Flex
            } else {
                Display::None
            };
            if node.display != display {
                node.display = display;
            }
        }
        if tiles_visible
            && let Some(image) = &texture
            && let Ok(mut tile) = preview.get_mut(entity)
            && tile.image != *image
        {
            tile.image = image.clone();
        }
    }
    if let Ok(mut toggle) = nodes.get_mut(maker.tile_toggle) {
        let display = if maker.shows_model() {
            Display::None
        } else {
            Display::Flex
        };
        if toggle.display != display {
            toggle.display = display;
        }
    }
    if let Ok(mut color) = backgrounds.get_mut(maker.tile_toggle) {
        color.set_if_neq(BackgroundColor(if tiles_visible { ACTIVE } else { CHROME }));
    }
    if maker.preview_task.is_none() && maker.due.is_some_and(|due| time.elapsed_secs_f64() >= due) {
        maker.due = None;
        if let (Some(device), Some(queue), Some(request)) = (device, queue, maker.request.as_ref())
        {
            let request = request.clone();
            let (device, queue) = (device.clone(), queue.clone());
            maker.preview_task = Some((
                maker.revision,
                AsyncComputeTaskPool::get().spawn(async move { render(request, &device, &queue) }),
            ));
        } else {
            text_value(&mut labels, maker.status, "Предпросмотр требует GPU");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgsl_source_files_preserve_unicode_and_drafts() {
        let folder =
            std::env::temp_dir().join(format!("bevy-wgsl-{}", bevy::asset::uuid::Uuid::new_v4(),));
        std::fs::create_dir(&folder).unwrap();
        let draft = "// @name Черновик\nfn node(\n";
        let path = save_source(&folder, draft).unwrap();
        assert_eq!(open_source(&path).unwrap(), draft);
        let another = save_source(&folder, draft).unwrap();
        assert_ne!(path, another);
        let modified = "// @name Черновик\nfn node() {\n";
        save_open_source(&path, draft, modified).unwrap();
        assert_eq!(open_source(&path).unwrap(), modified);
        assert!(save_open_source(&path, draft, "conflicting write").is_err());
        assert_eq!(open_source(&path).unwrap(), modified);
        assert_eq!(open_source(&another).unwrap(), draft);
        for (_, source) in TEMPLATES {
            def::parse(DRAFT_ID, source).unwrap();
        }
        std::fs::remove_dir_all(folder).unwrap();
    }
}
