//! Image document tabs. Source decoding stays off the editor's main thread.
use super::*;
use crate::editor::{
    browser::FileKind,
    image_previews::{self, DecodedImage, ImagePreviews},
};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    window::CursorIcon,
};
use std::{collections::HashMap, path::PathBuf};

#[path = "image_canvas.rs"]
mod canvas_material;
#[path = "image_edit.rs"]
pub(in crate::editor) mod edit;
pub(in crate::editor) use canvas_material::{CanvasMaterial, ImageCanvasPlugin};
#[path = "image_paint_input.rs"]
mod paint_input;
#[path = "image_panel_resize.rs"]
mod panel_resize;
pub(in crate::editor) use panel_resize::resize as resize_panel;
#[path = "image_cursor.rs"]
mod tool_cursor;
pub(in crate::editor) use tool_cursor::{cleanup as cleanup_cursor, cursor};

const MAX_TEXTURE: u32 = 4096;
const ZOOM_STEP: f32 = 1.25;

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum ImageAction {
    Close,
    Previous,
    Next,
    Fit,
    ActualSize,
    ZoomIn,
    ZoomOut,
    Background,
    Retry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Zoom {
    #[default]
    Fit,
    Scale(f32),
}

impl Zoom {
    fn scale(self, original: Vec2, viewport: Vec2) -> f32 {
        match self {
            Self::Fit => ((viewport - Vec2::splat(40.0)).max(Vec2::ONE) / original.max(Vec2::ONE))
                .min_element()
                .min(1.0),
            Self::Scale(value) => value,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Backdrop {
    #[default]
    Checker,
    Dark,
    Light,
}

struct Job {
    tab: usize,
    path: PathBuf,
    scan: u64,
    task: Task<Result<DecodedImage, String>>,
}

#[derive(Clone, Copy)]
struct SavedView {
    zoom: Zoom,
    pan: Vec2,
    dimensions: Option<UVec2>,
}

#[derive(Resource, Default)]
pub(in crate::editor) struct ImageViewer {
    pub edit: edit::State,
    panel: panel_resize::Panel,
    tab: Option<usize>,
    path: Option<PathBuf>,
    needs_load: bool,
    views: HashMap<usize, SavedView>,
    scan: u64,
    image: Option<Handle<Image>>,
    dimensions: Option<UVec2>,
    decoded_size: Option<UVec2>,
    pending: Option<Job>,
    error: Option<String>,
    zoom: Zoom,
    pan: Vec2,
    gesture: Option<(Vec2, Vec2)>,
    last_click: Option<(f64, Vec2)>,
    background: Backdrop,
    scale_factor: Option<f32>,
    cursor_owned: bool,
    previous_cursor: Option<CursorIcon>,
    last_cursor: Option<CursorIcon>,
    brush_cursor: Option<tool_cursor::Tip>,
    brush_cursor_position: Option<Vec2>,
    native_brush_cursor: bool,
    pipette_preview: Option<tool_cursor::PipetteSample>,
}

impl ImageViewer {
    pub(super) fn resizing_panel(&self) -> bool {
        self.panel.capturing()
    }

    pub(super) fn is_fit(&self) -> bool {
        self.zoom == Zoom::Fit
    }

    pub(super) fn is_actual_size(&self) -> bool {
        self.zoom == Zoom::Scale(1.0)
    }

    fn source_size(&self, dimensions: UVec2) -> Vec2 {
        dimensions.as_vec2() / self.scale_factor.unwrap_or(1.0)
    }

    fn image_rect(&self, dimensions: UVec2, viewport: Rect) -> Rect {
        let original = self.source_size(dimensions);
        let size = original * self.zoom.scale(original, viewport.size());
        let density = self.scale_factor.unwrap_or(1.0);
        let offset = (((viewport.size() - size) * 0.5 + self.pan) * density).round() / density;
        Rect::from_corners(viewport.min + offset, viewport.min + offset + size)
    }

    fn reset(
        &mut self,
        tab: usize,
        path: PathBuf,
        scan: u64,
        preview: Option<&image_previews::ImagePreview>,
    ) {
        let saved = self.views.get(&tab).copied();
        self.tab = Some(tab);
        self.path = Some(path);
        self.scan = scan;
        self.needs_load = true;
        self.image = None;
        self.dimensions = saved
            .and_then(|view| view.dimensions)
            .or_else(|| preview.and_then(|preview| preview.dimensions));
        self.decoded_size = None;
        self.error = None;
        self.zoom = saved.map_or(Zoom::Fit, |view| view.zoom);
        self.pan = saved.map_or(Vec2::ZERO, |view| view.pan);
        self.gesture = None;
        self.last_click = None;
    }

    fn zoom_at(&mut self, factor: f32, anchor: Vec2, viewport: Vec2) {
        let Some(original) = self.dimensions else {
            return;
        };
        let previous = self.zoom.scale(self.source_size(original), viewport);
        let scale = (previous * factor).clamp(0.001, 32.0);
        self.pan = anchor + (self.pan - anchor) * (scale / previous);
        self.zoom = Zoom::Scale(scale);
    }
}

#[derive(Component)]
pub(in crate::editor) struct ImageCanvas;

#[derive(Component)]
pub(in crate::editor) struct CanvasControl;

#[derive(Resource)]
pub(in crate::editor) struct ViewerUi {
    overlay: Entity,
    picture: Entity,
    checker: Entity,
    canvas: Entity,
    background: Entity,
    metadata: Entity,
    help: Entity,
    notice: Entity,
    notice_text: Entity,
    retry: Entity,
}

pub(super) fn checker_texture(images: &mut Assets<Image>) -> Handle<Image> {
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let value = if (x / 16 + y / 16) % 2 == 0 { 27 } else { 35 };
            rgba.extend_from_slice(&[value, value, value + 2, 255]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::nearest()
    });
    images.add(image)
}

pub(super) fn checker(commands: &mut Commands, parent: Entity, image: &Handle<Image>) -> Entity {
    commands
        .spawn((
            ImageNode {
                image: image.clone(),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: 1.0,
                },
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}

pub(super) fn setup(
    commands: &mut Commands,
    viewport: Entity,
    root: Entity,
    font: &Handle<Font>,
    checker_image: &Handle<Image>,
) {
    let overlay = surface(
        commands,
        viewport,
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(0),
            bottom: px(0),
            left: px(0),
            right: px(0),
            ..default()
        },
        PANEL,
    );
    commands
        .entity(overlay)
        .insert((ZIndex(6), AccessibleLabel("Вкладка изображения".into())));
    let panel = surface(
        commands,
        overlay,
        Node {
            width: percent(100),
            height: percent(100),
            min_width: px(0),
            overflow: Overflow::clip(),
            ..column(0.0)
        },
        PANEL,
    );
    let toolbar = surface(
        commands,
        panel,
        Node {
            width: percent(100),
            height: px(44),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(12)),
            border: UiRect::bottom(px(1)),
            ..row(8.0)
        },
        CHROME,
    );
    let editing_controls = node(
        commands,
        toolbar,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            ..row(12.0)
        },
    );
    let background_button = button(
        commands,
        toolbar,
        font,
        "Фон: шахматный",
        Action::ImageView(ImageAction::Background),
        Look::Regular,
    );
    let background = commands.entity(background_button).id();
    commands
        .entity(background)
        .insert(AccessibleLabel("Сменить фон прозрачности · B".into()));

    let body = node(
        commands,
        panel,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            align_items: AlignItems::Stretch,
            ..row(0.0)
        },
    );
    let canvas = node(
        commands,
        body,
        Node {
            flex_basis: px(0),
            flex_grow: 1.0,
            min_height: px(0),
            min_width: px(0),
            overflow: Overflow::clip(),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    );
    commands.entity(canvas).insert((
        ImageCanvas,
        BackgroundColor(WELL),
        AccessibleLabel("Изображение: колесо — масштаб, перетаскивание — панорама".into()),
    ));
    let checker = checker(commands, canvas, checker_image);
    edit::setup(commands, editing_controls, body, canvas, root, font);
    let picture = commands
        .spawn((
            MaterialNode(canvas_material::HANDLE),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(canvas),
        ))
        .id();
    let notice = surface(
        commands,
        canvas,
        Node {
            max_width: percent(85),
            padding: UiRect::all(px(18)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(9)),
            align_items: AlignItems::Center,
            ..column(12.0)
        },
        PANEL,
    );
    commands.entity(notice).insert(ZIndex(1));
    let notice_text = text(commands, notice, font, "Загрузка изображения…", 14.0, TEXT);
    commands.entity(notice_text).insert(TextLayout::default());
    let retry = button(
        commands,
        notice,
        font,
        "Повторить загрузку",
        Action::ImageView(ImageAction::Retry),
        Look::Regular,
    );
    let footer = surface(
        commands,
        panel,
        Node {
            width: percent(100),
            height: px(38),
            flex_shrink: 0.0,
            border: UiRect::top(px(1)),
            padding: UiRect::horizontal(px(14)),
            overflow: Overflow::clip(),
            ..row(12.0)
        },
        CHROME,
    );
    let details = node(
        commands,
        footer,
        Node {
            max_width: percent(42),
            min_width: px(0),
            overflow: Overflow::clip(),
            ..row(0.0)
        },
    );
    let metadata = text(commands, details, font, "", 11.0, MUTED);
    let help = node(
        commands,
        footer,
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            overflow: Overflow::clip(),
            justify_content: JustifyContent::End,
            ..row(0.0)
        },
    );
    let help_text = text(
        commands,
        help,
        font,
        "← → — листать · Колесо — масштаб · Ctrl+W — закрыть вкладку",
        11.0,
        MUTED,
    );
    commands.insert_resource(ViewerUi {
        overlay,
        picture,
        checker,
        canvas,
        background,
        metadata,
        help: help_text,
        notice,
        notice_text,
        retry,
    });
}

pub(super) fn shortcuts(keys: &ButtonInput<KeyCode>, actions: &mut MessageWriter<Action>) {
    for (key, action) in [
        (KeyCode::ArrowLeft, ImageAction::Previous),
        (KeyCode::ArrowRight, ImageAction::Next),
        (KeyCode::Equal, ImageAction::ZoomIn),
        (KeyCode::NumpadAdd, ImageAction::ZoomIn),
        (KeyCode::Minus, ImageAction::ZoomOut),
        (KeyCode::NumpadSubtract, ImageAction::ZoomOut),
        (KeyCode::KeyF, ImageAction::Fit),
        (KeyCode::Digit0, ImageAction::Fit),
        (KeyCode::Digit1, ImageAction::ActualSize),
        (KeyCode::KeyB, ImageAction::Background),
    ] {
        if keys.just_pressed(key) {
            actions.write(Action::ImageView(action));
        }
    }
    if keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]) && keys.just_pressed(KeyCode::KeyW)
    {
        actions.write(Action::ImageView(ImageAction::Close));
    }
}

fn gallery(editor: &Editor, browser: &Browser) -> Vec<usize> {
    let Some(path) = editor
        .viewing_image
        .and_then(|id| editor.image_tab(id))
        .map(|tab| &tab.path)
    else {
        return Vec::new();
    };
    let visible = browser.image_files(&editor.project.catalog);
    if visible
        .iter()
        .any(|&file| browser.files[file].path == *path)
    {
        return visible;
    }
    let mut files: Vec<_> = browser
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.kind == FileKind::Image && file.path.parent() == path.parent())
        .map(|(index, _)| index)
        .collect();
    files.sort_by_key(|&index| browser.files[index].filename.to_lowercase());
    files
}

pub(in crate::editor) fn neighbor(
    editor: &Editor,
    browser: &Browser,
    forward: bool,
) -> Option<usize> {
    let path = &editor.image_tab(editor.viewing_image?)?.path;
    let images = gallery(editor, browser);
    let position = images
        .iter()
        .position(|&image| browser.files[image].path == *path)?;
    let next = if forward {
        position.checked_add(1)?
    } else {
        position.checked_sub(1)?
    };
    images.get(next).copied()
}

fn canvas_rect(
    canvas: &Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    window: &Window,
) -> Rect {
    canvas
        .single()
        .ok()
        .filter(|(node, _)| !node.is_empty())
        .map_or_else(
            || {
                Rect::from_corners(
                    Vec2::new(0.0, IMAGE_TOP + 44.0),
                    Vec2::new(
                        window.width(),
                        (window.height() - FOOT - 38.0).max(IMAGE_TOP + 45.0),
                    ),
                )
            },
            |(node, transform)| {
                let center = transform.affine().translation * node.inverse_scale_factor();
                let half = node.size() * node.inverse_scale_factor() * 0.5;
                Rect::from_corners(center - half, center + half)
            },
        )
}

fn over_controls(
    controls: &Query<(&ComputedNode, &UiGlobalTransform), With<CanvasControl>>,
    point: Vec2,
) -> bool {
    controls.iter().any(|(node, transform)| {
        !node.is_empty() && node.contains_point(*transform, point / node.inverse_scale_factor())
    })
}

pub(in crate::editor) fn update(
    mut messages: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    mut viewer: ResMut<ImageViewer>,
    previews: Res<ImagePreviews>,
    (mut images, mut uploads): (
        ResMut<Assets<Image>>,
        Option<ResMut<crate::editor::image_texture::TextureUploads>>,
    ),
    window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    (canvas, controls): (
        Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
        Query<(&ComputedNode, &UiGlobalTransform), With<CanvasControl>>,
    ),
    mut focus: ResMut<InputFocus>,
    mut browser_ui: Option<ResMut<drawer::BrowserUi>>,
    drawer: Option<Res<DrawerMotion>>,
    (keys, mut strokes, mut events): (
        Res<ButtonInput<KeyCode>>,
        Local<paint_input::Samples>,
        MessageReader<bevy::window::WindowEvent>,
    ),
    inputs: Query<(), With<TextInput>>,
) {
    let (window_entity, mut window) = window.into_inner();
    let mut samples = strokes.collect(
        events.read(),
        window_entity,
        window.cursor_position(),
        &mouse,
    );
    let pressed_controls = samples.iter().any(|sample| {
        matches!(sample, paint_input::Sample::Down(point) if over_controls(&controls, *point))
    });
    // Floating controls share the canvas rectangle, but must never start a
    // stroke. Keep motion/release samples for gestures already captured there.
    samples.retain(|sample| {
        !matches!(sample, paint_input::Sample::Down(point) if over_controls(&controls, *point))
    });
    let hovering_controls = window
        .cursor_position()
        .is_some_and(|point| over_controls(&controls, point));
    let rect = canvas_rect(&canvas, &window);
    let input_rect = drawer
        .as_ref()
        .map_or(rect, |drawer| drawer.input_rect(rect, window.height()));
    let viewport = rect.size().max(Vec2::ONE);
    viewer.scale_factor = Some(window.scale_factor());
    if let Some(tab) = viewer.tab {
        let view = SavedView {
            zoom: viewer.zoom,
            pan: viewer.pan,
            dimensions: viewer.dimensions,
        };
        viewer.views.insert(tab, view);
    }
    let actions: Vec<_> = messages.read().copied().collect();
    for action in &actions {
        let enabled =
            editor.pending.is_none() && !editor.browser_popup && !editor.hdri_library_open;
        match *action {
            Action::ViewImage(file)
                if enabled
                    && browser
                        .files
                        .get(file)
                        .is_some_and(|file| file.kind == FileKind::Image) =>
            {
                editor.open_image_tab(browser.files[file].path.clone());
                if let Some(interface) = browser_ui.as_mut() {
                    interface.select(Some(browser::Entry::Image(file)));
                }
                focus.clear();
            }
            Action::OpenImageTab(id) if enabled => {
                editor.activate_image_tab(id);
                focus.clear();
            }
            Action::CloseImageTab(id) if enabled => {
                edit::request_close(&mut editor, id);
            }
            Action::ImageView(action) if enabled && editor.viewing_image.is_some() => {
                match action {
                    ImageAction::Close => {
                        let id = editor.viewing_image.unwrap();
                        edit::request_close(&mut editor, id);
                        focus.clear();
                    }
                    ImageAction::Previous | ImageAction::Next => {
                        if let Some(file) =
                            neighbor(&editor, &browser, matches!(action, ImageAction::Next))
                        {
                            editor.open_image_tab(browser.files[file].path.clone());
                        }
                    }
                    ImageAction::Fit | ImageAction::ActualSize => {
                        viewer.zoom = if matches!(action, ImageAction::Fit) {
                            Zoom::Fit
                        } else {
                            Zoom::Scale(1.0)
                        };
                        viewer.pan = Vec2::ZERO;
                    }
                    ImageAction::ZoomIn => viewer.zoom_at(ZOOM_STEP, Vec2::ZERO, viewport),
                    ImageAction::ZoomOut => viewer.zoom_at(1.0 / ZOOM_STEP, Vec2::ZERO, viewport),
                    ImageAction::Background => {
                        viewer.background = match viewer.background {
                            Backdrop::Checker => Backdrop::Dark,
                            Backdrop::Dark => Backdrop::Light,
                            Backdrop::Light => Backdrop::Checker,
                        }
                    }
                    ImageAction::Retry => {
                        viewer.error = None;
                        viewer.needs_load = true;
                    }
                }
            }
            _ => {}
        }
    }
    edit::update(&actions, &mut editor, &mut viewer, &mut browser);
    viewer.views.retain(|id, _| editor.image_tab(*id).is_some());
    let active = editor
        .viewing_image
        .and_then(|id| editor.image_tab(id).map(|tab| (id, tab.path.clone())));
    let file = active
        .as_ref()
        .and_then(|(_, path)| browser.files.iter().position(|file| file.path == *path));
    if let Some((id, path)) = &active {
        if viewer.tab != Some(*id) || viewer.path.as_ref() != Some(path) {
            viewer.reset(
                *id,
                path.clone(),
                browser.scan_revision,
                file.and_then(|file| previews.get(file)),
            );
        } else if viewer.scan != browser.scan_revision {
            viewer.scan = browser.scan_revision;
            viewer.needs_load = true;
            viewer.error = None;
        }
    } else {
        viewer.tab = None;
        viewer.path = None;
        viewer.image = None;
        viewer.dimensions = None;
        viewer.error = None;
        viewer.gesture = None;
    }
    // A single decoder remains bounded even when rapidly switching document tabs.
    let finished = viewer.pending.as_mut().and_then(|job| {
        block_on(poll_once(&mut job.task))
            .map(|result| (job.tab, job.path.clone(), job.scan, result))
    });
    if let Some((tab, path, scan, result)) = finished {
        viewer.pending = None;
        if viewer.tab == Some(tab)
            && viewer.path.as_ref() == Some(&path)
            && scan == viewer.scan
            && !viewer.edit.has_document(tab)
        {
            match result {
                Ok(decoded) => {
                    viewer.dimensions = Some(decoded.original);
                    viewer.decoded_size = Some(decoded.size);
                    viewer.image = Some(image_previews::texture(decoded, &mut images));
                }
                Err(error) => {
                    viewer.image = None;
                    viewer.error = Some(error);
                }
            }
        }
    }
    let Some((tab, path)) = active else {
        viewer.edit.size_adjust = default();
        return;
    };
    if viewer.needs_load && viewer.pending.is_none() && !viewer.edit.has_document(tab) {
        viewer.needs_load = false;
        let absolute = editor.root.join(&path);
        viewer.pending = Some(Job {
            tab,
            path,
            scan: viewer.scan,
            task: AsyncComputeTaskPool::get().spawn(async move {
                image_previews::decode_image(&absolute, UVec2::splat(MAX_TEXTURE))
            }),
        });
    }
    edit::sync(&mut viewer, &editor, &mut images, uploads.as_deref_mut());
    let adjusting_size = (viewer.edit.resizing_brush()
        || (!hovering_controls && !pressed_controls && !viewer.edit.mode_menu_dismissed))
        && edit::resize_brush(
            &mut viewer,
            &editor,
            rect,
            input_rect,
            &mut window,
            &mouse,
            &keys,
            focus.get().is_some_and(|entity| inputs.contains(entity)),
        );
    if editor.pending.is_some()
        || editor.browser_popup
        || editor.hdri_library_open
        || viewer.error.is_some()
    {
        viewer.gesture = None;
        return;
    }
    if !window.focused {
        viewer.gesture = None;
        viewer.last_click = None;
        return;
    }
    if adjusting_size {
        edit::sync(&mut viewer, &editor, &mut images, uploads.as_deref_mut());
        return;
    }
    let edit_pointer = edit::pointer(
        &mut viewer,
        &mut editor,
        rect,
        input_rect,
        &mouse,
        &keys,
        &mut focus,
        &samples,
    );
    // Present this batch, including its live interpolation tail, in the frame that received it.
    edit::sync(&mut viewer, &editor, &mut images, uploads.as_deref_mut());
    if viewer.panel.blocks_canvas() || viewer.edit.color_picker.is_some() || viewer.edit.size_menu {
        viewer.gesture = None;
        viewer.last_click = None;
        return;
    }
    if let Some(cursor) = window.cursor_position() {
        if !input_rect.contains(cursor) {
            viewer.gesture = None;
            viewer.last_click = None;
        }
        if input_rect.contains(cursor)
            && scroll.delta.y != 0.0
            && !hovering_controls
            && !viewer.edit.menu
            && !viewer.edit.paint_menu
        {
            let delta = if scroll.unit == MouseScrollUnit::Line {
                scroll.delta.y
            } else {
                scroll.delta.y / 100.0
            };
            viewer.zoom_at(
                ZOOM_STEP.powf(delta.clamp(-4.0, 4.0)),
                cursor - rect.center(),
                viewport,
            );
        }
        if !edit_pointer
            && input_rect.contains(cursor)
            && !hovering_controls
            && !pressed_controls
            && !viewer.edit.mode_menu_dismissed
            && !viewer.edit.menu
            && mouse.any_just_pressed([MouseButton::Left, MouseButton::Middle])
        {
            focus.clear();
            viewer.gesture = Some((cursor, viewer.pan));
        }
        if let Some((origin, pan)) = viewer.gesture {
            if mouse.any_pressed([MouseButton::Left, MouseButton::Middle])
                || mouse.any_just_released([MouseButton::Left, MouseButton::Middle])
            {
                viewer.pan = pan + cursor - origin;
            }
            if mouse.any_just_released([MouseButton::Left, MouseButton::Middle]) {
                if origin.distance(cursor) < 4.0 {
                    let now = time.elapsed_secs_f64();
                    if viewer.last_click.is_some_and(|(last, point)| {
                        now - last < 0.32 && point.distance(cursor) < 5.0
                    }) {
                        viewer.zoom = if viewer.zoom == Zoom::Fit {
                            Zoom::Scale(1.0)
                        } else {
                            Zoom::Fit
                        };
                        viewer.pan = Vec2::ZERO;
                        viewer.last_click = None;
                    } else {
                        viewer.last_click = Some((now, cursor));
                    }
                } else {
                    viewer.last_click = None;
                }
            }
        }
    }
    if !mouse.any_pressed([MouseButton::Left, MouseButton::Middle]) {
        viewer.gesture = None;
    }
}

pub(in crate::editor) fn refresh(
    editor: Res<Editor>,
    browser: Res<Browser>,
    viewer: Res<ImageViewer>,
    previews: Res<ImagePreviews>,
    interface: Res<ViewerUi>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut materials: ResMut<Assets<CanvasMaterial>>,
    mut colors: Query<&mut BackgroundColor>,
    children: Query<&Children>,
) {
    let shown = editor.viewing_image.is_some();
    if let Ok(mut node) = nodes.get_mut(interface.background) {
        node.display = if viewer.edit.editing(editor.viewing_image) && window.width() < 1000.0 {
            Display::None
        } else {
            Display::Flex
        };
    }
    if let Ok(mut node) = nodes.get_mut(interface.overlay) {
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        node.bottom = px(0);
    }
    let Some(tab) = editor.viewing_image.and_then(|id| editor.image_tab(id)) else {
        if let Some(mut material) = materials.get_mut(&canvas_material::HANDLE)
            && material.image.is_some()
        {
            material.image = None;
        }
        return;
    };
    let file = browser.files.iter().position(|file| file.path == tab.path);
    let source = file.and_then(|file| browser.files.get(file));
    let rect = canvas_rect(&canvas, &window);
    let preview = file.and_then(|file| previews.get(file));
    let dimensions = viewer
        .dimensions
        .or_else(|| preview.and_then(|preview| preview.dimensions));
    let handle = viewer
        .image
        .as_ref()
        .or_else(|| preview.and_then(|preview| preview.image.as_ref()));
    let ready = handle.is_some() && viewer.error.is_none();
    if let Some(mut material) = materials.get_mut(&canvas_material::HANDLE) {
        let image = handle.cloned();
        let parameters = dimensions.map_or(Vec4::ZERO, |size| {
            Vec4::new(
                size.x as f32,
                size.y as f32,
                if viewer.image.is_some() && viewer.decoded_size == Some(size) {
                    1.0
                } else {
                    0.0
                },
                viewer.zoom.scale(viewer.source_size(size), rect.size()),
            )
        });
        if material.image != image || material.parameters != parameters {
            material.image = image;
            material.parameters = parameters;
        }
    }
    if let Ok(mut picture) = nodes.get_mut(interface.picture) {
        picture.display = if ready { Display::Flex } else { Display::None };
        if let Some(dimensions) = dimensions {
            let image = viewer.image_rect(dimensions, rect);
            let size = image.size();
            let offset = image.min - rect.min;
            picture.width = px(size.x);
            picture.height = px(size.y);
            picture.left = px(offset.x);
            picture.top = px(offset.y);
        }
    }
    if let Ok(mut checker) = nodes.get_mut(interface.checker) {
        checker.display = if viewer.background == Backdrop::Checker {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut color) = colors.get_mut(interface.canvas) {
        color.set_if_neq(BackgroundColor(if viewer.background == Backdrop::Light {
            Color::srgb(0.88, 0.88, 0.89)
        } else {
            WELL
        }));
    }
    let size = dimensions.map_or(String::new(), |size| {
        format!(" · {} × {} px", size.x, size.y)
    });
    let limited = viewer
        .decoded_size
        .zip(dimensions)
        .is_some_and(|(decoded, original)| decoded != original);
    let extension = tab
        .path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_uppercase();
    let bytes = source.map_or(String::new(), |source| {
        format!(" · {}", drawer::file_size(source.bytes))
    });
    let metadata = format!(
        "{} · {extension}{size}{bytes}{}",
        tab.path.display(),
        if limited {
            " · Превью до 4096 px"
        } else {
            ""
        }
    );
    for (entity, value) in [
        (
            interface.help,
            if viewer.edit.editing(editor.viewing_image) {
                "Пробел — панорама · Ctrl+Z — отменить · Ctrl+S — сохранить".into()
            } else {
                "← → — листать · Колесо — масштаб · Ctrl+W — закрыть вкладку".into()
            },
        ),
        (interface.metadata, metadata),
        (
            interface.notice_text,
            if viewer.error.is_some() {
                "Не удалось открыть изображение.\nПроверьте, что файл не повреждён и его формат поддерживается.".into()
            } else {
                "Загрузка изображения…".into()
            },
        ),
    ] {
        if let Ok(mut label) = labels.get_mut(entity) {
            label.set_if_neq(Text(value));
        }
    }
    if let Ok(children) = children.get(interface.background) {
        for child in children.iter() {
            if let Ok(mut label) = labels.get_mut(child) {
                label.set_if_neq(Text(format!(
                    "Фон: {}",
                    match viewer.background {
                        Backdrop::Checker => "шахматный",
                        Backdrop::Dark => "тёмный",
                        Backdrop::Light => "светлый",
                    }
                )));
            }
        }
    }
    if let Ok(mut notice) = nodes.get_mut(interface.notice) {
        notice.display = if viewer.image.is_none() {
            Display::Flex
        } else {
            Display::None
        };
    }
    if let Ok(mut retry) = nodes.get_mut(interface.retry) {
        retry.display = if viewer.error.is_some() {
            Display::Flex
        } else {
            Display::None
        };
    }
}

#[cfg(test)]
#[path = "image_viewer_tests.rs"]
mod tests;
