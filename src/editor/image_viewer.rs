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
    window::{CursorIcon, SystemCursorIcon},
};
use std::{collections::HashMap, path::PathBuf};

#[path = "image_edit.rs"]
pub(in crate::editor) mod edit;

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
}

impl ImageViewer {
    pub(super) fn is_fit(&self) -> bool {
        self.zoom == Zoom::Fit
    }

    pub(super) fn is_actual_size(&self) -> bool {
        self.zoom == Zoom::Scale(1.0)
    }

    fn source_size(&self, dimensions: UVec2) -> Vec2 {
        dimensions.as_vec2() / self.scale_factor.unwrap_or(1.0)
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

    fn clamp_pan(&mut self, viewport: Vec2) {
        let original = self.source_size(self.dimensions.unwrap_or(UVec2::ONE));
        let extent =
            ((original * self.zoom.scale(original, viewport) - viewport) * 0.5).max(Vec2::ZERO);
        self.pan = self.pan.clamp(-extent, extent);
    }

    fn zoom_at(&mut self, factor: f32, anchor: Vec2, viewport: Vec2) {
        let Some(original) = self.dimensions else {
            return;
        };
        let previous = self.zoom.scale(self.source_size(original), viewport);
        let scale = (previous * factor).clamp(0.001, 32.0);
        self.pan = anchor + (self.pan - anchor) * (scale / previous);
        self.zoom = Zoom::Scale(scale);
        self.clamp_pan(viewport);
    }
}

#[derive(Component)]
pub(in crate::editor) struct ImageCanvas;

#[derive(Resource)]
pub(in crate::editor) struct ViewerUi {
    overlay: Entity,
    picture: Entity,
    checker: Entity,
    canvas: Entity,
    counter: Entity,
    zoom: Entity,
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
    let navigation = group(commands, toolbar);
    icons::button(
        commands,
        navigation,
        icons::Glyph::ChevronLeft,
        "Предыдущее изображение · ←",
        Action::ImageView(ImageAction::Previous),
        Look::Regular,
    );
    let counter = text(commands, navigation, font, "", 12.0, MUTED);
    commands.entity(counter).insert(Node {
        min_width: px(48),
        justify_content: JustifyContent::Center,
        ..default()
    });
    icons::button(
        commands,
        navigation,
        icons::Glyph::ChevronRight,
        "Следующее изображение · →",
        Action::ImageView(ImageAction::Next),
        Look::Regular,
    );
    button(
        commands,
        toolbar,
        font,
        "Вписать",
        Action::ImageView(ImageAction::Fit),
        Look::Regular,
    );
    button(
        commands,
        toolbar,
        font,
        "1:1",
        Action::ImageView(ImageAction::ActualSize),
        Look::Regular,
    );
    let zoom_controls = group(commands, toolbar);
    let minus = button(
        commands,
        zoom_controls,
        font,
        "−",
        Action::ImageView(ImageAction::ZoomOut),
        Look::Regular,
    );
    commands
        .entity(minus)
        .insert(AccessibleLabel("Уменьшить · −".into()));
    let zoom = text(commands, zoom_controls, font, "100%", 12.0, TEXT);
    commands.entity(zoom).insert(Node {
        min_width: px(52),
        ..default()
    });
    icons::button(
        commands,
        zoom_controls,
        icons::Glyph::Plus,
        "Увеличить · +",
        Action::ImageView(ImageAction::ZoomIn),
        Look::Regular,
    );
    node(
        commands,
        toolbar,
        Node {
            flex_grow: 1.0,
            ..default()
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
    edit::setup(commands, toolbar, body, canvas, font);
    let picture = commands
        .spawn((
            ImageNode::default(),
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
        counter,
        zoom,
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
    drawer_height: f32,
) -> Rect {
    canvas
        .single()
        .ok()
        .filter(|(node, _)| !node.is_empty())
        .map_or_else(
            || {
                Rect::from_corners(
                    Vec2::new(0.0, TOP + 44.0),
                    Vec2::new(
                        window.width(),
                        (window.height() - FOOT - 38.0 - drawer_height).max(TOP + 45.0),
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

pub(in crate::editor) fn update(
    mut messages: MessageReader<Action>,
    mut editor: ResMut<Editor>,
    mut browser: ResMut<Browser>,
    mut viewer: ResMut<ImageViewer>,
    previews: Res<ImagePreviews>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    mut focus: ResMut<InputFocus>,
    mut browser_ui: Option<ResMut<drawer::BrowserUi>>,
    drawer: Option<Res<DrawerMotion>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let rect = canvas_rect(
        &canvas,
        &window,
        drawer.as_ref().map_or(0.0, |drawer| drawer.visible_height),
    );
    let viewport = rect.size().max(Vec2::ONE);
    viewer.scale_factor = Some(window.scale_factor());
    viewer.edit.compact = window.height() < 740.0;
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
                    interface.selected = Some(file);
                    interface.selected_model = None;
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
    let Some((tab, path)) = active else { return };
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
    edit::sync(&mut viewer, &editor, &mut images);
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
    let edit_pointer = edit::pointer(
        &mut viewer,
        &mut editor,
        rect,
        window.cursor_position(),
        &mouse,
        &keys,
        &mut focus,
    );
    if let Some(cursor) = window.cursor_position() {
        if rect.contains(cursor)
            && scroll.delta.y != 0.0
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
            && rect.contains(cursor)
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
    viewer.clamp_pan(viewport);
}

pub(in crate::editor) fn cursor(
    mut commands: Commands,
    editor: Res<Editor>,
    mut viewer: ResMut<ImageViewer>,
    window: Single<(Entity, &Window, Option<&CursorIcon>), With<PrimaryWindow>>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    drawer: Option<Res<DrawerMotion>>,
) {
    let (entity, window, current) = *window;
    let rect = canvas_rect(
        &canvas,
        window,
        drawer.as_ref().map_or(0.0, |drawer| drawer.visible_height),
    );
    let can_pan = viewer.dimensions.is_some_and(|size| {
        let original = viewer.source_size(size);
        (original * viewer.zoom.scale(original, rect.size()) - rect.size()).max_element() > 1.0
    });
    let desired = (editor.viewing_image.is_some()
        && editor.pending.is_none()
        && !editor.browser_popup
        && !editor.hdri_library_open
        && window.focused
        && (can_pan || viewer.edit.editing(editor.viewing_image))
        && viewer.error.is_none()
        && window
            .cursor_position()
            .is_some_and(|point| rect.contains(point)))
    .then(|| {
        CursorIcon::from(
            if viewer.edit.editing(editor.viewing_image)
                && viewer.edit.tool != crate::editor::photo_document::PhotoTool::Hand
            {
                SystemCursorIcon::Crosshair
            } else if viewer.gesture.is_some() {
                SystemCursorIcon::Grabbing
            } else {
                SystemCursorIcon::Grab
            },
        )
    });
    if let Some(desired) = desired {
        if !viewer.cursor_owned {
            viewer.previous_cursor = current.cloned();
            viewer.cursor_owned = true;
        }
        if current != Some(&desired) {
            commands.entity(entity).insert(desired);
        }
    } else if viewer.cursor_owned {
        viewer.cursor_owned = false;
        if let Some(previous) = viewer.previous_cursor.take() {
            commands.entity(entity).insert(previous);
        } else {
            commands.entity(entity).remove::<CursorIcon>();
        }
    }
}

pub(in crate::editor) fn refresh(
    editor: Res<Editor>,
    browser: Res<Browser>,
    viewer: Res<ImageViewer>,
    previews: Res<ImagePreviews>,
    interface: Res<ViewerUi>,
    drawer: Res<DrawerMotion>,
    canvas: Query<(&ComputedNode, &UiGlobalTransform), With<ImageCanvas>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node>,
    mut labels: Query<&mut Text>,
    mut pictures: Query<&mut ImageNode>,
    mut colors: Query<&mut BackgroundColor>,
    children: Query<&Children>,
) {
    let shown = editor.viewing_image.is_some();
    if let Ok(mut node) = nodes.get_mut(interface.overlay) {
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        if node.bottom != px(drawer.visible_height) {
            node.bottom = px(drawer.visible_height);
        }
    }
    let Some(tab) = editor.viewing_image.and_then(|id| editor.image_tab(id)) else {
        if let Ok(mut picture) = pictures.get_mut(interface.picture) {
            picture.image = Handle::default();
        }
        return;
    };
    let file = browser.files.iter().position(|file| file.path == tab.path);
    let source = file.and_then(|file| browser.files.get(file));
    let rect = canvas_rect(&canvas, &window, drawer.visible_height);
    let preview = file.and_then(|file| previews.get(file));
    let dimensions = viewer
        .dimensions
        .or_else(|| preview.and_then(|preview| preview.dimensions));
    let handle = viewer
        .image
        .as_ref()
        .or_else(|| preview.and_then(|preview| preview.image.as_ref()));
    let ready = handle.is_some() && viewer.error.is_none();
    if let Ok(mut image) = pictures.get_mut(interface.picture) {
        image.image = handle.cloned().unwrap_or_default();
    }
    if let Ok(mut picture) = nodes.get_mut(interface.picture) {
        picture.display = if ready { Display::Flex } else { Display::None };
        if let Some(dimensions) = dimensions {
            let original = viewer.source_size(dimensions);
            let size = original * viewer.zoom.scale(original, rect.size());
            let offset = (((rect.size() - size) * 0.5 + viewer.pan) * window.scale_factor())
                .round()
                / window.scale_factor();
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
    let files = gallery(&editor, &browser);
    let position = files
        .iter()
        .position(|&file| browser.files[file].path == tab.path)
        .map(|position| (position + 1).to_string())
        .unwrap_or_else(|| "—".into());
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
    let percent = dimensions.map_or(100.0, |size| {
        viewer.zoom.scale(viewer.source_size(size), rect.size()) * 100.0
    });
    for (entity, value) in [
        (
            interface.help,
            if viewer.edit.editing(editor.viewing_image) {
                "Пробел — панорама · Ctrl+Z — отменить · Ctrl+S — сохранить".into()
            } else {
                "← → — листать · Колесо — масштаб · Ctrl+W — закрыть вкладку".into()
            },
        ),
        (interface.counter, format!("{position} / {}", files.len())),
        (
            interface.zoom,
            if percent < 10.0 {
                format!("{percent:.1}%")
            } else {
                format!("{percent:.0}%")
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
