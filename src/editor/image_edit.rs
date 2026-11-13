//! PhotoCraft integration for image tabs: commands, background I/O and gestures.
use super::*;
use crate::editor::{
    PendingAction,
    photo_document::{self, PhotoDocument, PhotoTool},
};
use photocraft_paint::{BrushSettings, StrokePoint};
use photocraft_raster::Surface;

#[path = "image_edit_ui.rs"]
mod interface;
pub(in crate::editor) use interface::{color_pointer, refresh, setup};

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum EditAction {
    Menu,
    View,
    Edit,
    Tool(PhotoTool),
    DrawingTool,
    PaintMenu,
    Save,
    Undo,
    Redo,
    AddLayer,
    DuplicateLayer,
    DeleteLayer,
    LayerUp,
    LayerDown,
    SelectLayer(usize),
    Visible(usize),
    Opacity(f32, bool),
    Size(f32),
    Hardness(f32),
    BrushOpacity(f32),
    Hue(f32),
    SaturationValue(f32, f32),
    Color([f32; 4]),
    ColorPanel,
}

struct Session {
    pub doc: PhotoDocument,
    pub editing: bool,
    rendered: Option<u64>,
}
struct Opening {
    id: usize,
    path: PathBuf,
    task: Task<Result<PhotoDocument, String>>,
}
struct Saving {
    id: usize,
    revision: u64,
    task: Task<Result<(), String>>,
}
enum Gesture {
    Paint {
        original: Surface,
        renderer: photocraft_paint::StrokeRenderer,
        last: Option<Vec2>,
    },
    Move {
        original: Surface,
        origin: Vec2,
    },
}

#[derive(Default)]
pub(in crate::editor) struct State {
    sessions: HashMap<usize, Session>,
    opening: Option<Opening>,
    saving: Option<Saving>,
    gesture: Option<(usize, Gesture)>,
    opacity_drag: Option<usize>,
    pub menu: bool,
    pub paint_menu: bool,
    pub paint_menu_dismissed: bool,
    pub drawing_tool: PhotoTool,
    brush_size: f32,
    pencil_size: f32,
    pub tool: PhotoTool,
    pub size: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub hsv: Option<bevy::color::Hsva>,
    pub color: Option<[f32; 4]>,
    pub error: Option<String>,
    pub color_expanded: Option<bool>,
    pub compact: bool,
}

impl State {
    pub fn editing(&self, id: Option<usize>) -> bool {
        id.and_then(|id| self.sessions.get(&id))
            .is_some_and(|s| s.editing)
    }
    pub fn has_document(&self, id: usize) -> bool {
        self.sessions.contains_key(&id)
    }
    pub fn active(&self, action: EditAction, id: Option<usize>) -> bool {
        match action {
            EditAction::View => !self.editing(id),
            EditAction::Edit => self.editing(id),
            EditAction::Tool(tool) => self.tool == tool,
            EditAction::DrawingTool => matches!(self.tool, PhotoTool::Brush | PhotoTool::Pencil),
            EditAction::SelectLayer(index) => {
                self.document(id).is_some_and(|doc| doc.active == index)
            }
            _ => false,
        }
    }
    pub fn document(&self, id: Option<usize>) -> Option<&PhotoDocument> {
        Some(&self.sessions.get(&id?)?.doc)
    }
    pub fn available(&self, action: EditAction, id: Option<usize>) -> bool {
        if self.busy() {
            return false;
        }
        let doc = self.document(id);
        match action {
            EditAction::Menu | EditAction::View | EditAction::Edit => true,
            EditAction::Save => doc.is_some_and(PhotoDocument::dirty),
            EditAction::Undo => doc.is_some_and(PhotoDocument::can_undo),
            EditAction::Redo => doc.is_some_and(PhotoDocument::can_redo),
            EditAction::DeleteLayer => doc.is_some_and(|d| d.document.layers.len() > 1),
            EditAction::LayerUp => doc.is_some_and(|d| d.active + 1 < d.document.layers.len()),
            EditAction::LayerDown => doc.is_some_and(|d| d.active > 0),
            EditAction::AddLayer | EditAction::DuplicateLayer => {
                doc.is_some_and(|d| d.document.layers.len() < 64)
            }
            _ => self.editing(id),
        }
    }
    pub fn busy(&self) -> bool {
        self.opening.is_some() || self.saving.is_some()
    }
    pub fn color(&self) -> [f32; 4] {
        self.color.unwrap_or([0.0, 0.0, 0.0, 1.0])
    }
    fn select_tool(&mut self, tool: PhotoTool) {
        if self.tool == PhotoTool::Pencil {
            self.pencil_size = self.size;
        } else {
            self.brush_size = self.size;
        }
        self.size = if tool == PhotoTool::Pencil {
            self.pencil_size.max(1.0).round()
        } else {
            self.brush_size.max(1.0)
        };
        self.tool = tool;
        if matches!(tool, PhotoTool::Brush | PhotoTool::Pencil) {
            self.drawing_tool = tool;
        }
        self.paint_menu = false;
        self.gesture = None;
        for session in self.sessions.values_mut() {
            session.rendered = None;
        }
    }
    fn brush(&self) -> BrushSettings {
        let pencil = self.tool == PhotoTool::Pencil;
        BrushSettings {
            size: if pencil {
                self.size.round().max(1.0)
            } else {
                self.size
            },
            hardness: if pencil { 1.0 } else { self.hardness },
            opacity: self.opacity,
            color: self.color(),
            erase: self.tool == PhotoTool::Eraser,
            aliased: pencil,
            ..Default::default()
        }
    }
    fn set_color(&mut self, color: [f32; 4]) {
        self.color = Some(color);
        self.hsv = Some(bevy::color::Hsva::from(bevy::color::Srgba::new(
            color[0], color[1], color[2], color[3],
        )));
    }
    fn save(&mut self, id: usize, editor: &mut Editor) {
        if self.saving.is_some() {
            return;
        }
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        let Some(tab) = editor.image_tab(id) else {
            return;
        };
        let source = editor.root.join(&tab.path);
        let document = session.doc.document.clone();
        self.saving = Some(Saving {
            id,
            revision: session.doc.revision,
            task: AsyncComputeTaskPool::get()
                .spawn(async move { photo_document::save(&source, &document) }),
        });
        editor.status = "Сохранение слоёв .pcraft и изображения…".into();
    }
}

pub(super) fn request_close(editor: &mut Editor, id: usize) {
    if editor.image_tab(id).is_some_and(|tab| tab.dirty) {
        editor.activate_image_tab(id);
        editor.pending = Some(PendingAction::CloseImageTab(id));
        editor.status = "Есть несохранённые изменения изображения".into();
    } else {
        editor.close_image_tab(id);
    }
}

pub(super) fn update(
    actions: &[Action],
    editor: &mut Editor,
    viewer: &mut ImageViewer,
    browser: &mut Browser,
) {
    let state = &mut viewer.edit;
    if state.size == 0.0 {
        state.size = 24.0;
        state.brush_size = 24.0;
        state.pencil_size = 1.0;
        state.hardness = 1.0;
        state.opacity = 1.0;
    }
    if let Some((id, path, result)) = state.opening.as_mut().and_then(|job| {
        block_on(poll_once(&mut job.task)).map(|result| (job.id, job.path.clone(), result))
    }) {
        state.opening = None;
        if editor
            .image_tab(id)
            .is_some_and(|tab| editor.root.join(&tab.path) == path)
        {
            match result {
                Ok(doc) => {
                    state.sessions.insert(
                        id,
                        Session {
                            doc,
                            editing: true,
                            rendered: None,
                        },
                    );
                    state.error = None;
                    editor.status =
                        "Редактирование PhotoCraft · Ctrl+S — сохранить слои и изображение".into();
                    browser.refresh(&mut editor.project);
                }
                Err(error) => {
                    state.error = Some(error.clone());
                    editor.status = format!("Не удалось открыть редактор · {error}");
                }
            }
        }
    }
    if let Some((id, revision, result)) = state
        .saving
        .as_mut()
        .and_then(|job| block_on(poll_once(&mut job.task)).map(|r| (job.id, job.revision, r)))
    {
        state.saving = None;
        match result {
            Ok(()) => {
                state.error = None;
                if let Some(session) = state.sessions.get_mut(&id) {
                    session.doc.saved_revision = revision;
                }
                editor.status = "Сохранены слои .pcraft и исходное изображение".into();
                if let Some(tab) = editor.image_tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.dirty = state.sessions.get(&id).is_some_and(|s| s.doc.dirty());
                }
                browser.refresh(&mut editor.project);
                match editor.pending {
                    Some(PendingAction::CloseImageTab(closing)) if closing == id => {
                        editor.pending = None;
                        editor.close_image_tab(id);
                    }
                    Some(PendingAction::CloseImageEditor(closing)) if closing == id => {
                        editor.pending = None;
                        editor.continue_exit();
                    }
                    _ => {}
                }
            }
            Err(error) => {
                state.error = Some(error.clone());
                editor.status = format!("Не удалось сохранить изображение · {error}");
            }
        }
    }
    for &action in actions {
        if matches!(
            action,
            Action::Open(_)
                | Action::OpenImageTab(_)
                | Action::ViewImage(_)
                | Action::ViewModel(_)
                | Action::CloseImageTab(_)
        ) {
            state.menu = false;
        }
        if let Some(PendingAction::CloseImageTab(id) | PendingAction::CloseImageEditor(id)) =
            editor.pending
        {
            if state.saving.is_some() {
                continue;
            }
            match action {
                Action::SaveAndContinue => state.save(id, editor),
                Action::DiscardAndContinue => {
                    let exiting =
                        matches!(editor.pending, Some(PendingAction::CloseImageEditor(_)));
                    state.sessions.remove(&id);
                    editor.pending = None;
                    editor.close_image_tab(id);
                    if exiting {
                        editor.continue_exit();
                    }
                }
                Action::CancelPending => {
                    editor.pending = None;
                    editor.exit_queue.clear();
                    editor.status = "Продолжайте редактирование изображения".into();
                }
                _ => {}
            }
            continue;
        }
        if editor.pending.is_some() || editor.browser_popup || editor.hdri_library_open {
            continue;
        }
        let Action::ImageEdit(action) = action else {
            continue;
        };
        let Some(id) = editor.viewing_image else {
            continue;
        };
        match action {
            EditAction::Menu => {
                state.menu ^= true;
                state.paint_menu = false;
                viewer.gesture = None;
                viewer.last_click = None;
            }
            EditAction::View => {
                if let Some(session) = state.sessions.get_mut(&id) {
                    session.editing = false;
                }
                state.menu = false;
                state.paint_menu = false;
                state.error = None;
                state.gesture = None;
            }
            EditAction::Edit => {
                state.menu = false;
                if let Some(session) = state.sessions.get_mut(&id) {
                    session.editing = true;
                } else if state.opening.is_none()
                    && let Some(tab) = editor.image_tab(id)
                {
                    let path = editor.root.join(&tab.path);
                    let source = path.clone();
                    state.error = None;
                    state.opening = Some(Opening {
                        id,
                        path,
                        task: AsyncComputeTaskPool::get()
                            .spawn(async move { PhotoDocument::load(&source) }),
                    });
                    editor.status = "Открытие редактора PhotoCraft…".into();
                }
            }
            EditAction::Save => state.save(id, editor),
            EditAction::Tool(tool) => {
                state.select_tool(tool);
            }
            EditAction::DrawingTool => state.select_tool(state.drawing_tool),
            EditAction::PaintMenu => {
                state.paint_menu ^= true;
                state.menu = false;
                state.gesture = None;
                viewer.gesture = None;
                viewer.last_click = None;
            }
            EditAction::Size(value) => {
                state.size = value.clamp(1.0, 256.0);
                if state.tool == PhotoTool::Pencil {
                    state.size = state.size.round();
                }
            }
            EditAction::Hardness(value) => state.hardness = value.clamp(0.0, 1.0),
            EditAction::BrushOpacity(value) => state.opacity = value.clamp(0.01, 1.0),
            EditAction::Hue(value) => {
                let mut hsv = state.hsv.unwrap_or_else(|| {
                    bevy::color::Hsva::from(bevy::color::Srgba::new(
                        state.color()[0],
                        state.color()[1],
                        state.color()[2],
                        1.0,
                    ))
                });
                hsv.hue = value;
                let rgb = bevy::color::Srgba::from(hsv);
                state.set_color([rgb.red, rgb.green, rgb.blue, 1.0]);
            }
            EditAction::SaturationValue(s, v) => {
                let mut hsv = state.hsv.unwrap_or_else(|| {
                    bevy::color::Hsva::from(bevy::color::Srgba::new(
                        state.color()[0],
                        state.color()[1],
                        state.color()[2],
                        1.0,
                    ))
                });
                hsv.saturation = s;
                hsv.value = v;
                let rgb = bevy::color::Srgba::from(hsv);
                state.set_color([rgb.red, rgb.green, rgb.blue, 1.0]);
            }
            EditAction::Color(color) => state.set_color(color),
            EditAction::ColorPanel => {
                state.color_expanded = Some(!state.color_expanded.unwrap_or(!state.compact))
            }
            _ if state.saving.is_none() => {
                let Some(session) = state.sessions.get_mut(&id).filter(|s| s.editing) else {
                    continue;
                };
                state.gesture = None;
                match action {
                    EditAction::Undo => session.doc.undo(),
                    EditAction::Redo => session.doc.redo(),
                    EditAction::AddLayer => session.doc.add_layer(),
                    EditAction::DuplicateLayer => session.doc.duplicate_layer(),
                    EditAction::DeleteLayer => session.doc.delete_layer(),
                    EditAction::LayerUp => session.doc.reorder(true),
                    EditAction::LayerDown => session.doc.reorder(false),
                    EditAction::Visible(index) => session.doc.visibility(index),
                    EditAction::Opacity(value, final_value) => {
                        if state.opacity_drag != Some(id) {
                            session.doc.checkpoint();
                            state.opacity_drag = Some(id);
                        }
                        session.doc.document.layers[session.doc.active].opacity =
                            value.clamp(0.0, 1.0);
                        session.doc.changed();
                        if final_value {
                            state.opacity_drag = None;
                        }
                    }
                    EditAction::SelectLayer(index) if index < session.doc.document.layers.len() => {
                        session.doc.active = index
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    state
        .sessions
        .retain(|id, _| editor.image_tab(*id).is_some());
    for tab in &mut editor.image_tabs {
        tab.dirty = state.sessions.get(&tab.id).is_some_and(|s| s.doc.dirty());
    }
}

pub(super) fn sync(viewer: &mut ImageViewer, editor: &Editor, images: &mut Assets<Image>) {
    let state = &mut viewer.edit;
    if let Some(id) = editor.viewing_image
        && let Some(session) = state.sessions.get_mut(&id)
    {
        if session.rendered != Some(session.doc.revision)
            || viewer.tab != Some(id)
            || viewer.image.is_none()
        {
            let composite = session.doc.composite();
            let size = UVec2::new(composite.width, composite.height);
            let decoded = if size.max_element() > MAX_TEXTURE {
                let rgba = image::RgbaImage::from_raw(size.x, size.y, composite.pixels)
                    .map(image::DynamicImage::ImageRgba8)
                    .map(|image| image.thumbnail(MAX_TEXTURE, MAX_TEXTURE).into_rgba8());
                let Some(rgba) = rgba else { return };
                DecodedImage {
                    size: UVec2::new(rgba.width(), rgba.height()),
                    rgba: rgba.into_raw(),
                    original: size,
                }
            } else {
                DecodedImage {
                    rgba: composite.pixels,
                    size,
                    original: size,
                }
            };
            let decoded_size = decoded.size;
            let mut texture = image_previews::texture_image(decoded);
            if state.tool == PhotoTool::Pencil {
                texture.sampler = ImageSampler::nearest();
            }
            if let Some(mut image) = viewer
                .image
                .as_ref()
                .and_then(|handle| images.get_mut(handle))
            {
                *image = texture;
            } else {
                viewer.image = Some(images.add(texture));
            }
            viewer.dimensions = Some(size);
            viewer.decoded_size = Some(decoded_size);
            viewer.error = None;
            session.rendered = Some(session.doc.revision);
        }
    }
}

pub(super) fn pointer(
    viewer: &mut ImageViewer,
    editor: &mut Editor,
    rect: Rect,
    cursor: Option<Vec2>,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    focus: &mut InputFocus,
) -> bool {
    let Some(id) = editor.viewing_image else {
        return false;
    };
    if !viewer.edit.editing(Some(id)) {
        viewer.edit.gesture = None;
        return false;
    }
    if viewer.edit.busy()
        || viewer.edit.menu
        || viewer.edit.paint_menu
        || viewer.edit.paint_menu_dismissed
    {
        viewer.edit.gesture = None;
        return true;
    }
    if viewer.edit.tool == PhotoTool::Hand
        || keys.pressed(KeyCode::Space)
        || mouse.pressed(MouseButton::Middle)
    {
        return false;
    }
    let Some(cursor) = cursor else { return true };
    let Some(dimensions) = viewer.dimensions else {
        return true;
    };
    let original_size = viewer.source_size(dimensions);
    let size = original_size * viewer.zoom.scale(original_size, rect.size());
    let image_min = rect.center() - size * 0.5 + viewer.pan;
    let point = (cursor - image_min) / size * dimensions.as_vec2();
    if mouse.just_pressed(MouseButton::Left)
        && rect.contains(cursor)
        && point.cmpge(Vec2::ZERO).all()
        && point.cmplt(dimensions.as_vec2()).all()
    {
        focus.clear();
        let state = &mut viewer.edit;
        let color = state.color();
        let brush = state.brush();
        let Some(session) = state.sessions.get_mut(&id) else {
            return true;
        };
        if state.tool == PhotoTool::Eyedropper {
            let composite = session.doc.composite();
            let index = (point.y as usize * composite.width as usize + point.x as usize) * 4;
            if let Some(pixel) = composite.pixels.get(index..index + 4) {
                state.set_color([
                    pixel[0] as f32 / 255.0,
                    pixel[1] as f32 / 255.0,
                    pixel[2] as f32 / 255.0,
                    1.0,
                ]);
            }
        } else if session.doc.paintable() {
            if state.tool == PhotoTool::Fill {
                session.doc.fill(point.x as i32, point.y as i32, color);
            } else if let Some(original) = session.doc.document.layers[session.doc.active]
                .surface()
                .cloned()
            {
                session.doc.checkpoint();
                state.gesture = Some((
                    id,
                    if state.tool == PhotoTool::Move {
                        Gesture::Move {
                            original,
                            origin: point,
                        }
                    } else {
                        let renderer = photocraft_paint::StrokeRenderer::new(
                            &brush,
                            Some(original.format()),
                            1.0,
                        )
                        .with_clip(photocraft_geom::Rect::from_size(session.doc.size()));
                        Gesture::Paint {
                            original,
                            renderer,
                            last: None,
                        }
                    },
                ));
            }
        } else {
            editor.status = "Выберите видимый незаблокированный слой".into();
        }
    }
    if let Some((gesture_id, gesture)) = &mut viewer.edit.gesture
        && *gesture_id == id
        && (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
        && let Some(session) = viewer.edit.sessions.get_mut(&id)
    {
        match gesture {
            Gesture::Paint {
                original,
                renderer,
                last,
            } => {
                let point = point.clamp(
                    Vec2::splat(-256.0),
                    dimensions.as_vec2() + Vec2::splat(256.0),
                );
                let changed = last.is_none_or(|last| last.distance_squared(point) > 0.01);
                let finished = mouse.just_released(MouseButton::Left);
                if changed || finished {
                    session.doc.paint_live(
                        original,
                        renderer,
                        changed.then(|| StrokePoint::new(point.x as f64, point.y as f64, 1.0)),
                        finished,
                    );
                    *last = Some(point);
                }
            }
            Gesture::Move { original, origin } => {
                let delta = (point - *origin)
                    .round()
                    .clamp(Vec2::splat(-32768.0), Vec2::splat(32768.0));
                session
                    .doc
                    .move_layer(original, delta.x as i32, delta.y as i32);
            }
        }
        if let Some(tab) = editor.image_tabs.iter_mut().find(|t| t.id == id) {
            tab.dirty = session.doc.dirty();
        }
    }
    if !mouse.pressed(MouseButton::Left) {
        viewer.edit.gesture = None;
    }
    true
}

pub(in crate::editor) fn shortcuts(
    keys: &ButtonInput<KeyCode>,
    viewer: Option<&ImageViewer>,
    actions: &mut MessageWriter<Action>,
) -> bool {
    let Some(viewer) = viewer else { return false };
    if !viewer.edit.editing(viewer.tab) {
        return false;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if command {
        if keys.just_pressed(KeyCode::KeyZ) {
            actions.write(Action::ImageEdit(
                if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                    EditAction::Redo
                } else {
                    EditAction::Undo
                },
            ));
            return true;
        }
        if keys.just_pressed(KeyCode::KeyY) {
            actions.write(Action::ImageEdit(EditAction::Redo));
            return true;
        }
    } else {
        if keys.just_pressed(KeyCode::KeyB) {
            let action = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                EditAction::Tool(if viewer.edit.drawing_tool == PhotoTool::Pencil {
                    PhotoTool::Brush
                } else {
                    PhotoTool::Pencil
                })
            } else {
                EditAction::DrawingTool
            };
            actions.write(Action::ImageEdit(action));
            return true;
        }
        for (key, tool) in [
            (KeyCode::KeyE, PhotoTool::Eraser),
            (KeyCode::KeyG, PhotoTool::Fill),
            (KeyCode::KeyI, PhotoTool::Eyedropper),
            (KeyCode::KeyV, PhotoTool::Move),
            (KeyCode::KeyH, PhotoTool::Hand),
        ] {
            if keys.just_pressed(key) {
                actions.write(Action::ImageEdit(EditAction::Tool(tool)));
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn pencil_is_pixel_aligned_and_opaque_while_brush_has_antialiased_edges() {
        let mut state = State {
            tool: PhotoTool::Pencil,
            size: 1.0,
            hardness: 0.1,
            opacity: 1.0,
            ..Default::default()
        };
        let mut pixels = Surface::new(PixelFormat::RGBA8);
        photocraft_paint::render_stroke(
            &mut pixels,
            &state.brush(),
            &[
                StrokePoint::new(2.2, 6.2, 1.0),
                StrokePoint::new(14.7, 6.2, 1.0),
            ],
            None,
            false,
            1.0,
        );
        for y in 0..16 {
            for x in 0..20 {
                let alpha = pixels.rgba(x, y)[3];
                assert!(
                    alpha == 0.0 || alpha == 1.0,
                    "Pencil smoothed pixel {x},{y}: {alpha}"
                );
                if y != 6 {
                    assert_eq!(alpha, 0.0);
                }
            }
        }
        for x in 2..=14 {
            assert_eq!(pixels.rgba(x, 6)[3], 1.0, "Gap in pencil line at {x}");
        }
        state.tool = PhotoTool::Brush;
        state.size = 7.0;
        state.hardness = 1.0;
        let mut brush = Surface::new(PixelFormat::RGBA8);
        photocraft_paint::render_stroke(
            &mut brush,
            &state.brush(),
            &[StrokePoint::new(7.3, 7.7, 1.0)],
            None,
            false,
            1.0,
        );
        assert!(
            (0..16).any(|y| (0..20).any(|x| {
                let a = brush.rgba(x, y)[3];
                a > 0.0 && a < 1.0
            })),
            "Brush must keep its antialiased edge"
        );
    }

    #[test]
    fn drawing_group_remembers_each_tool_and_its_size() {
        let mut state = State {
            size: 24.0,
            pencil_size: 1.0,
            ..Default::default()
        };
        state.select_tool(PhotoTool::Pencil);
        assert_eq!(state.size, 1.0);
        state.size = 3.0;
        state.select_tool(PhotoTool::Brush);
        assert_eq!(state.size, 24.0);
        state.select_tool(PhotoTool::Pencil);
        assert_eq!(state.size, 3.0);
        state.select_tool(PhotoTool::Hand);
        assert_eq!(state.drawing_tool, PhotoTool::Pencil);
        state.select_tool(state.drawing_tool);
        assert_eq!(state.tool, PhotoTool::Pencil);
        assert_eq!(state.size, 3.0);
    }
}
