//! PhotoCraft integration for image tabs: commands, background I/O and gestures.
use super::gradient::{self, Draft, Settings};
use super::*;
use crate::editor::{
    PendingAction,
    photo_document::{self, PhotoDocument, PhotoTool},
};
use photocraft_paint::{BrushSettings, StrokePoint};
use photocraft_raster::Surface;

#[path = "image_edit_ui.rs"]
pub(super) mod interface;
pub(in crate::editor) use interface::{color_pointer, refresh, setup};
#[path = "image_color.rs"]
pub(in crate::editor) mod color;
#[path = "image_color_picker.rs"]
pub(in crate::editor) mod picker_ui;
pub(in crate::editor) use color::{ColorComponent, ColorTarget};
pub(in crate::editor) use picker_ui::ColorPickerScroll;
pub(in crate::editor) use picker_ui::{input as color_input, refresh as refresh_color_picker};
#[path = "image_brush_library.rs"]
mod brush_library;
#[path = "image_brush_size.rs"]
mod brush_size;
#[path = "image_color_drop.rs"]
pub(in crate::editor) mod color_drop;
pub(super) use brush_size::pointer as resize_brush;
#[path = "image_layers.rs"]
pub(in crate::editor) mod layers;
#[path = "image_selection.rs"]
pub(in crate::editor) mod selection;
#[path = "image_selection_ui.rs"]
pub(in crate::editor) mod selection_ui;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum EraserMode {
    #[default]
    Brush,
    Pixel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum PixelShape {
    #[default]
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::editor) enum ToolMenu {
    #[default]
    Paint,
    Eraser,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum EditAction {
    Menu,
    View,
    Edit,
    Tool(PhotoTool),
    ModelTool(Tool),
    ModelPanel,
    EraserMode(EraserMode),
    DrawingTool,
    PixelShape(PixelShape),
    PaintMenu,
    EraserMenu,
    Save,
    Undo,
    Redo,
    CancelGesture,
    GradientRadial(bool),
    GradientTransparent,
    GradientReverse,
    AddLayer,
    DeleteLayer,
    SelectLayer(usize),
    Visible(usize),
    Opacity(photocraft_doc::LayerId, f32, bool),
    Layers(layers::LayerAction),
    Size(f32),
    SizeMenu,
    BrushCategory(usize),
    BrushPreset(Option<usize>),
    Hardness(f32),
    BrushOpacity(f32),
    #[cfg(test)]
    Color([f32; 4]),
    ColorPicker(ColorTarget),
    ColorConfirm,
    ColorCancel,
    ColorComponent(ColorComponent, f32),
    ColorDraft([f32; 4]),
    ColorSv(f32, f32),
    SwapColors,
    ResetColors,
    AddSwatch,
    Selection(selection::SelectionAction),
}

struct Session {
    pub doc: PhotoDocument,
    pub editing: bool,
    opacity_preview: Option<OpacityPreview>,
    rendered: Option<(u64, u64)>,
    prepared: Option<(u64, u64, photocraft_raster::Rgba8Image)>,
}
/// The main thread keeps the document authoritative. A gesture owns one worker and
/// one reduced document; queued slider positions collapse into the newest revision.
struct OpacityPreview {
    layer: photocraft_doc::LayerId,
    revision: u64,
    view: u64,
    proxy: Option<photocraft_doc::Document>,
    task: Option<Task<OpacityFrame>>,
    displayed: Option<u64>,
}

struct OpacityFrame {
    revision: u64,
    exact: bool,
    proxy: Option<photocraft_doc::Document>,
    image: photocraft_raster::Rgba8Image,
}

impl OpacityPreview {
    fn poll(&mut self, doc: &PhotoDocument, dragging: bool) -> Option<OpacityFrame> {
        let mut frame = self
            .task
            .as_mut()
            .and_then(|task| block_on(poll_once(task)));
        if let Some(result) = &mut frame {
            self.task = None;
            self.proxy = result.proxy.take();
            if !dragging && (!result.exact || result.revision != self.revision) {
                frame = None;
            } else {
                self.displayed = Some(result.revision);
            }
        }
        let complete = frame
            .as_ref()
            .is_some_and(|f| f.exact && f.revision == self.revision);
        if self.task.is_none() && !complete && (!dragging || self.displayed != Some(self.revision))
        {
            let revision = self.revision;
            let layer = self.layer;
            let opacity = doc.document.layer(layer)?.opacity;
            let mask_view = doc.mask_view;
            let cached = dragging.then(|| self.proxy.take()).flatten();
            let document = cached.is_none().then(|| doc.document.clone());
            self.task = Some(bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
                let (mut source, reduced) = if let Some(proxy) = cached {
                    (proxy, true)
                } else {
                    let source = document.unwrap();
                    let factor = source.size.width.max(source.size.height).div_ceil(768);
                    if dragging && factor > 1 && photocraft_compose::proxy::proxy_faithful(&source)
                    {
                        (
                            photocraft_compose::proxy::proxy_document(&source, factor),
                            true,
                        )
                    } else {
                        (source, false)
                    }
                };
                if let Some(layer) = source.layer_mut(layer) {
                    layer.opacity = opacity;
                }
                let image = photo_document::render_preview(
                    &source,
                    mask_view,
                    photocraft_geom::Rect::from_size(source.size),
                );
                OpacityFrame {
                    revision,
                    exact: !reduced,
                    image,
                    proxy: reduced.then_some(source),
                }
            }));
        }
        frame
    }
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
    PickColor {
        last: Option<IVec2>,
    },
    Paint {
        original: Surface,
        checkpoint: Option<photo_document::Snapshot>,
        renderer: photocraft_paint::StrokeRenderer,
        last: Option<Vec2>,
        last_pressure: f32,
        tail: photocraft_geom::Rect,
    },
    Move {
        original: photo_document::LayerMove,
        origin: Vec2,
    },
    Gradient(Draft),
}

#[derive(Default)]
pub(in crate::editor) struct State {
    pub(in crate::editor) model_tool: Option<Tool>,
    pub(super) model_panel_closed: bool,
    pub selection: selection::SelectionState,
    pub layers: layers::LayerState,
    sessions: HashMap<usize, Session>,
    opening: Option<Opening>,
    saving: Option<Saving>,
    gesture: Option<(usize, Gesture)>,
    opacity_drag: Option<(usize, photocraft_doc::LayerId)>,
    pub size_adjust: brush_size::Adjustment,
    pub menu: bool,
    pub paint_menu: bool,
    pub tool_menu: ToolMenu,
    pub paint_menu_dismissed: bool,
    pub mode_menu_dismissed: bool,
    pub size_menu: bool,
    pub size_menu_dismissed: bool,
    brush_category: usize,
    brush_preset: Option<usize>,
    preset_sizes: HashMap<Option<usize>, f32>,
    pub drawing_tool: PhotoTool,
    brush_size: f32,
    pencil_size: f32,
    eraser_size: f32,
    pixel_eraser_size: f32,
    pub eraser_mode: EraserMode,
    pub pencil_shape: PixelShape,
    pub pixel_eraser_shape: PixelShape,
    pub tool: PhotoTool,
    pub(super) gradient: Settings,
    pub size: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub hsv: Option<bevy::color::Hsva>,
    pub color: Option<[f32; 4]>,
    pub error: Option<String>,
    pub background: Option<[f32; 4]>,
    pub color_picker: Option<color::Picker>,
    pub color_dragging: bool,
    pub swatches: Vec<[f32; 4]>,
}

impl State {
    pub(in crate::editor) fn file_busy(&self, id: usize) -> bool {
        self.opening.as_ref().is_some_and(|work| work.id == id)
            || self.saving.as_ref().is_some_and(|work| work.id == id)
    }
    pub fn sizeable(&self) -> bool {
        matches!(
            self.tool,
            PhotoTool::Brush | PhotoTool::Pencil | PhotoTool::Eraser
        )
    }
    pub fn resizing_brush(&self) -> bool {
        self.size_adjust.active()
    }
    pub fn eyedropper(&self, keys: &ButtonInput<KeyCode>, id: Option<usize>) -> bool {
        // A press keeps its tool until release, even if Alt changes mid-stroke.
        if let Some((gesture_id, gesture)) = &self.gesture
            && Some(*gesture_id) == id
        {
            return matches!(gesture, Gesture::PickColor { .. });
        }
        self.tool == PhotoTool::Eyedropper
            || ((self.sizeable() || matches!(self.tool, PhotoTool::Fill | PhotoTool::Gradient))
                && keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]))
    }
    fn set_size(&mut self, size: f32) {
        if size.is_finite() {
            self.size = size.clamp(1.0, 256.0);
            if self.pixel_tool() {
                self.size = self.size.round();
            }
        }
    }
    pub fn editing(&self, id: Option<usize>) -> bool {
        id.and_then(|id| self.sessions.get(&id))
            .is_some_and(|s| s.editing)
    }
    pub fn has_document(&self, id: usize) -> bool {
        self.sessions.contains_key(&id)
    }
    pub fn active(&self, action: EditAction, id: Option<usize>) -> bool {
        if self.model_tool.is_some()
            && matches!(
                action,
                EditAction::Tool(_) | EditAction::DrawingTool | EditAction::EraserMode(_)
            )
            && self.model_reference(id).is_some()
        {
            return false;
        }
        match action {
            EditAction::ModelTool(tool) => self.model_tool.unwrap_or(Tool::Universal) == tool,
            EditAction::Layers(layers::LayerAction::BalanceTone(tone)) => self.layers.balance_tone == tone,
            EditAction::GradientRadial(radial) => self.gradient.radial == radial,
            EditAction::GradientTransparent => self.gradient.transparent,
            EditAction::GradientReverse => self.gradient.reverse,
            EditAction::Selection(action) => self.selection.active(action, self.tool),
            EditAction::View => !self.editing(id),
            EditAction::Edit => self.editing(id),
            EditAction::Tool(tool) => self.tool == tool,
            EditAction::EraserMode(mode) => {
                self.tool == PhotoTool::Eraser && self.eraser_mode == mode
            }
            EditAction::DrawingTool => matches!(self.tool, PhotoTool::Brush | PhotoTool::Pencil),
            EditAction::PixelShape(shape) => self.pixel_shape() == shape,
            EditAction::PaintMenu => self.paint_menu && self.tool_menu == ToolMenu::Paint,
            EditAction::EraserMenu => self.paint_menu && self.tool_menu == ToolMenu::Eraser,
            EditAction::SizeMenu => self.size_menu,
            EditAction::BrushCategory(category) => self.brush_category == category,
            EditAction::BrushPreset(preset) => self.brush_preset == preset,
            EditAction::ColorPicker(target) => self
                .color_picker
                .as_ref()
                .is_some_and(|p| p.target == target),
            EditAction::SelectLayer(index) => self
                .document(id)
                .is_some_and(|doc| doc.active == index && !doc.mask_target),
            EditAction::Layers(layers::LayerAction::Mask(index)) => self
                .document(id)
                .is_some_and(|doc| doc.active == index && doc.mask_target && !doc.filter_target),
            EditAction::Layers(layers::LayerAction::FilterMask(index)) => self
                .document(id)
                .is_some_and(|doc| doc.active == index && doc.mask_target && doc.filter_target),
            EditAction::Layers(layers::LayerAction::PanelSection(section)) => {
                self.layers.section_open(section)
                    || matches!(section, layers::PanelSection::Search) && self.layers.has_filter()
            }
            EditAction::Layers(layers::LayerAction::FieldChoice(index,choice)) => self.layers.form.as_ref()
                .and_then(|f|f.fields.get(index)).is_some_and(|f| matches!(&f.kind,layers::FieldKind::Choice(values) if values.get(choice).is_some_and(|v|v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string())==f.text))),
            EditAction::Layers(layers::LayerAction::Preview) => self.layers.preview_enabled,
            EditAction::Layers(layers::LayerAction::Blend(layer)) => self.layers.blend_menu == Some(layer),
            EditAction::Layers(layers::LayerAction::SetBlend(layer, blend)) => self.document(id).and_then(|doc| doc.document.layer(layer)).is_some_and(|l|l.blend==blend),
            EditAction::Layers(layers::LayerAction::ToggleLock(layer)) => self
                .document(id)
                .and_then(|doc| doc.document.layer(photocraft_doc::LayerId(layer)))
                .is_some_and(|layer| layer.locks.all),
            EditAction::Layers(layers::LayerAction::Lock(key)) => self
                .document(id)
                .and_then(PhotoDocument::active_layer)
                .is_some_and(|layer| match key {
                    "all" => layer.locks.all,
                    "transparency" => layer.locks.transparency,
                    "pixels" => layer.locks.pixels,
                    "position" => layer.locks.position,
                    _ => false,
                }),
            _ => false,
        }
    }
    pub(super) fn gradient_draft(&self, id: usize) -> Option<Draft> {
        match self.gesture {
            Some((tab, Gesture::Gradient(draft))) if tab == id => Some(draft),
            _ => None,
        }
    }
    pub(super) fn cancel_gradient(&mut self) {
        if matches!(self.gesture, Some((_, Gesture::Gradient(_)))) {
            self.gesture = None;
        }
    }
    pub(in crate::editor) fn gradient_dragging(&self) -> bool {
        matches!(self.gesture, Some((_, Gesture::Gradient(_))))
    }
    pub(super) fn dragging_layer_opacity(&self, id: Option<usize>) -> bool {
        self.opacity_drag.is_some_and(|(tab, _)| Some(tab) == id)
    }
    pub fn document(&self, id: Option<usize>) -> Option<&PhotoDocument> {
        Some(&self.sessions.get(&id?)?.doc)
    }
    pub(super) fn display_model_preview(
        &mut self,
        id: usize,
        image: photocraft_raster::Rgba8Image,
    ) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.prepared = Some((session.doc.revision, session.doc.view_revision, image));
            session.rendered = None;
        }
    }
    pub(super) fn clear_model_preview(&mut self, id: usize) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.prepared = None;
            session.rendered = None;
        }
    }
    pub(in crate::editor) fn model_reference(
        &self,
        id: Option<usize>,
    ) -> Option<crate::editor::image_reference::ModelReference> {
        let photo = self.document(id)?;
        let layer = photo.active_layer()?;
        if photo.mask_target || photo.filter_target || layer.locks.position || layer.locks.all {
            return None;
        }
        layer
            .xmp
            .as_deref()
            .and_then(crate::editor::image_reference::ModelReference::from_xmp)
    }
    pub(in crate::editor) fn insert_reference(
        &mut self,
        id: usize,
        document: photocraft_doc::DocId,
        size: UVec2,
        name: String,
        surface: &Surface,
        reference: &crate::editor::image_reference::ModelReference,
        replace: Option<crate::editor::image_reference::ReferenceTarget>,
    ) -> Result<bool, String> {
        if self.file_busy(id)
            || self.gesture.as_ref().is_some_and(|(tab, _)| *tab == id)
            || self.opacity_drag.is_some_and(|(tab, _)| tab == id)
            || self
                .selection
                .transform
                .as_ref()
                .is_some_and(|t| t.id == id)
            || self.layers.preview.is_some()
            || self.layers.fill_drag
        {
            return Ok(false);
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return Err("документ закрыт".into());
        };
        let current = session.doc.size();
        if document != session.doc.document.id || size != UVec2::new(current.width, current.height)
        {
            return Err("холст изменился во время рендера".into());
        }
        if let Some(target) = replace {
            session
                .doc
                .replace_reference(target, surface.clone(), reference)?;
        } else {
            session
                .doc
                .add_reference(name, surface.clone(), reference)?;
            self.model_tool = Some(Tool::Move);
        }
        Ok(true)
    }
    pub fn available(&self, action: EditAction, id: Option<usize>) -> bool {
        if self.busy() {
            return false;
        }
        if let Some(picker) = &self.color_picker {
            return match action {
                EditAction::ColorConfirm => picker.error.is_none(),
                EditAction::ColorCancel
                | EditAction::ColorComponent(..)
                | EditAction::ColorDraft(_)
                | EditAction::ColorSv(..) => true,
                EditAction::AddSwatch => {
                    self.swatches.len() < 24
                        && !self.swatches.iter().any(|c| color::hex(*c) == picker.hex())
                }
                _ => false,
            };
        }
        let doc = self.document(id);
        match action {
            EditAction::ModelTool(_) | EditAction::ModelPanel => {
                self.editing(id) && self.model_reference(id).is_some()
            }
            EditAction::Selection(selection::SelectionAction::FlipSelection(_)) => {
                self.editing(id)
                    && matches!(self.tool, PhotoTool::Select(_))
                    && self.selection.transform.is_none()
                    && doc.is_some_and(selection::can_flip_selection)
            }
            EditAction::Selection(selection::SelectionAction::Apply) => {
                self.selection.transform.is_some()
            }
            EditAction::Selection(selection::SelectionAction::Cancel) => {
                self.selection.transform.is_some() || self.selection.draft.is_some()
            }
            EditAction::Selection(selection::SelectionAction::Command(command))
                if matches!(command, "select.deselect" | "select.inverse" | "edit.clear") =>
            {
                self.editing(id) && doc.is_some_and(|d| d.document.selection.is_some())
            }
            EditAction::Selection(_) => self.editing(id),
            EditAction::Menu | EditAction::View | EditAction::Edit => true,
            EditAction::Save => {
                doc.is_some_and(PhotoDocument::dirty)
                    || self
                        .selection
                        .transform
                        .as_ref()
                        .is_some_and(|t| Some(t.id) == id && t.deferred)
            }
            EditAction::Undo => doc.is_some_and(PhotoDocument::can_undo),
            EditAction::Redo => doc.is_some_and(PhotoDocument::can_redo),
            EditAction::DeleteLayer => doc.is_some_and(|d| d.active_layer().is_some()),
            EditAction::AddLayer => {
                doc.is_some_and(|d| d.document.layer_count() < photo_document::MAX_LAYERS)
            }
            EditAction::EraserMode(_) => self.editing(id),
            EditAction::SizeMenu => self.editing(id) && self.sizeable(),
            EditAction::Layers(layers::LayerAction::PanelSection(section)) => {
                self.editing(id)
                    && (matches!(section, layers::PanelSection::Search)
                        || doc.is_some_and(|doc| doc.active_layer().is_some()))
            }
            EditAction::Layers(action) => {
                self.editing(id) && doc.is_some_and(|doc| layers::ux::enabled(action, doc))
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
    pub fn background(&self) -> [f32; 4] {
        self.background.unwrap_or([1.0; 4])
    }
    pub fn pixel_tool(&self) -> bool {
        self.tool == PhotoTool::Pencil
            || (self.tool == PhotoTool::Eraser && self.eraser_mode == EraserMode::Pixel)
    }
    pub fn pixel_square(&self) -> bool {
        self.pixel_tool()
            && (self.size.round().max(1.0) <= 3.0 || self.pixel_shape() == PixelShape::Square)
    }
    pub fn pixel_shape(&self) -> PixelShape {
        if self.tool == PhotoTool::Eraser {
            self.pixel_eraser_shape
        } else {
            self.pencil_shape
        }
    }
    pub fn snap_point(&self, point: Vec2) -> Vec2 {
        if self.pixel_tool() {
            let (x, y) = photocraft_paint::grid_center(point.x as f64, point.y as f64, self.size);
            Vec2::new(x, y)
        } else {
            point
        }
    }
    fn remember_size(&mut self) {
        match self.tool {
            PhotoTool::Pencil => self.pencil_size = self.size,
            PhotoTool::Eraser if self.eraser_mode == EraserMode::Pixel => {
                self.pixel_eraser_size = self.size
            }
            PhotoTool::Eraser => self.eraser_size = self.size,
            PhotoTool::Brush => self.brush_size = self.size,
            _ => {}
        }
    }
    fn restore_size(&mut self) {
        self.size = match self.tool {
            PhotoTool::Pencil => self.pencil_size.max(1.0).round(),
            PhotoTool::Eraser if self.eraser_mode == EraserMode::Pixel => {
                self.pixel_eraser_size.max(1.0).round()
            }
            PhotoTool::Eraser => self.eraser_size.max(1.0),
            _ => self.brush_size.max(1.0),
        };
    }
    fn select_eraser_mode(&mut self, mode: EraserMode) {
        if self.eraser_mode == mode {
            return;
        }
        self.remember_size();
        self.eraser_mode = mode;
        self.restore_size();
        self.gesture = None;
    }
    fn select_tool(&mut self, tool: PhotoTool) {
        self.model_tool = None;
        if self.tool != tool {
            if !selection::finish_pending(self) {
                return;
            }
            self.selection.transform = None;
            self.selection.draft = None;
            self.selection.menu = 0;
        }
        self.remember_size();
        self.tool = tool;
        self.restore_size();
        if matches!(tool, PhotoTool::Brush | PhotoTool::Pencil) {
            self.drawing_tool = tool;
        }
        self.paint_menu = false;
        self.gesture = None;
    }
    fn brush(&self) -> BrushSettings {
        let pencil = self.pixel_tool();
        let mut brush = BrushSettings {
            size: if pencil {
                self.size.round().max(1.0)
            } else {
                self.size
            },
            hardness: if pencil { 1.0 } else { self.hardness },
            opacity: self.opacity,
            color: self.color(),
            background: self.background(),
            erase: self.tool == PhotoTool::Eraser,
            aliased: pencil,
            pressure_size: !pencil,
            square: pencil && self.pixel_shape() == PixelShape::Square,
            ..Default::default()
        };
        if self.tool == PhotoTool::Brush
            && let Some(preset) = self
                .brush_preset
                .and_then(|index| brush_library::library().presets.get(index))
        {
            preset.apply(&mut brush);
        }
        brush
    }
    fn set_color(&mut self, color: [f32; 4]) {
        self.color = Some(color);
        self.hsv = Some(bevy::color::Hsva::from(bevy::color::Srgba::new(
            color[0], color[1], color[2], color[3],
        )));
    }
    pub(super) fn save(&mut self, id: usize, editor: &mut Editor) {
        if !selection::finish_pending(self) {
            return;
        }
        layers::finish_preview(self, true);
        self.layers.form = None;
        self.selection.transform = None;
        self.selection.draft = None;
        if self.tool == PhotoTool::Transform {
            self.select_tool(PhotoTool::Move);
        }
        if self.saving.is_some() {
            return;
        }
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        while session.doc.editing_smart() {
            if let Err(error) = session.doc.finish_smart() {
                self.error = Some(error);
                return;
            }
        }
        let Some(tab) = editor.image_tab(id) else {
            return;
        };
        let source = editor.root.join(&tab.path);
        let mut document = session.doc.document.clone();
        let root = editor.root.clone();
        self.saving = Some(Saving {
            id,
            revision: session.doc.revision,
            task: AsyncComputeTaskPool::get().spawn(async move {
                let index = crate::asset_registry::index(&root).map_err(|e| e.to_string())?;
                crate::editor::image_reference::ModelReference::bind_layers(
                    &mut document.layers,
                    &index,
                )?;
                photo_document::save(&source, &document)?;
                crate::asset_registry::scan::record_save(&root, &source)
                    .map_err(|e| format!("Изображение сохранено, ошибка обновления реестра: {e}"))
            }),
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
    if state
        .selection
        .transform
        .as_ref()
        .is_some_and(|t| Some(t.id) != editor.viewing_image)
    {
        if state
            .selection
            .transform
            .as_ref()
            .is_some_and(|t| !state.sessions.contains_key(&t.id))
            || selection::finish_pending(state)
        {
            state.selection.transform = None;
        }
    }
    if state
        .selection
        .draft
        .as_ref()
        .is_some_and(|d| Some(d.id) != editor.viewing_image)
    {
        state.selection.draft = None;
    }
    if state.size == 0.0 {
        state.size = 24.0;
        state.brush_size = 24.0;
        state.pencil_size = 1.0;
        state.eraser_size = 24.0;
        state.pixel_eraser_size = 1.0;
        state.hardness = 1.0;
        state.opacity = 1.0;
        state.swatches = color::PRESETS.to_vec();
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
                            opacity_preview: None,
                            doc,
                            editing: true,
                            rendered: None,
                            prepared: None,
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
    layers::poll_mask_import(state, editor);
    for &action in actions {
        if !matches!(action, Action::ImageEdit(EditAction::Opacity(..))) {
            state.opacity_drag = None;
        }
        if !matches!(
            action,
            Action::ImageEdit(
                EditAction::SizeMenu
                    | EditAction::BrushCategory(_)
                    | EditAction::BrushPreset(_)
                    | EditAction::Size(_)
                    | EditAction::Hardness(_)
                    | EditAction::BrushOpacity(_)
                    | EditAction::PixelShape(_)
            )
        ) {
            state.size_menu = false;
        }
        if matches!(
            action,
            Action::Generation(super::GenerationAction::Open)
                | Action::Open(_)
                | Action::OpenImageTab(_)
                | Action::ViewImage(_)
                | Action::EditBrowserImage(..)
                | Action::ViewModel(_)
                | Action::CloseImageTab(_)
        ) {
            if let Some(old) = viewer.tab.and_then(|id| state.sessions.get_mut(&id))
                && old.opacity_preview.take().is_some()
            {
                old.rendered = None;
            }
            layers::finish_preview(state, false);
            state.menu = false;
            state.color_picker = None;
            state.layers.form = None;
            state.layers.menu = false;
            state.layers.context_menu_y = None;
            state.layers.blend_menu = None;
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
        // A layer edit or leaving Edit mode must first retain a pending GPU drag.
        if matches!(
            action,
            EditAction::View
                | EditAction::Layers(_)
                | EditAction::AddLayer
                | EditAction::DeleteLayer
                | EditAction::SelectLayer(_)
                | EditAction::Visible(_)
                | EditAction::Opacity(..)
        ) && state.selection.transform.is_some()
        {
            if !selection::finish_pending(state) {
                continue;
            }
            state.selection.transform = None;
            if state.tool == PhotoTool::Transform {
                state.select_tool(PhotoTool::Move);
            }
        }
        match action {
            EditAction::Undo if state.layers.preview.is_some() => {
                layers::finish_preview(state, false);
                state.layers.form = None;
                state.layers.ui_revision += 1;
            }
            EditAction::Selection(action) => selection::dispatch(state, id, action),
            EditAction::Undo if state.selection.transform.is_some() => {
                selection::dispatch(state, id, selection::SelectionAction::Cancel)
            }
            EditAction::Menu => {
                state.menu ^= true;
                state.paint_menu = false;
                viewer.gesture = None;
                viewer.last_click = None;
            }
            EditAction::View => {
                layers::finish_preview(state, false);
                state.selection.transform = None;
                state.selection.draft = None;
                state.selection.menu = 0;
                if state.tool == PhotoTool::Transform {
                    state.select_tool(PhotoTool::Move);
                }
                state.color_picker = None;
                state.layers.form = None;
                state.layers.menu = false;
                state.layers.context_menu_y = None;
                state.layers.blend_menu = None;
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
                    let asset_root = editor.root.clone();
                    state.opening = Some(Opening {
                        id,
                        path,
                        task: AsyncComputeTaskPool::get().spawn(async move {
                            PhotoDocument::load_indexed(&source, Some(&asset_root))
                        }),
                    });
                    editor.status = "Открытие редактора PhotoCraft…".into();
                }
            }
            EditAction::Save => state.save(id, editor),
            EditAction::CancelGesture => {
                if let Some((gesture_id, Gesture::Move { mut original, .. })) = state.gesture.take()
                    && let Some(session) = state.sessions.get_mut(&gesture_id)
                {
                    session.doc.cancel_move(&mut original);
                }
            }
            EditAction::GradientRadial(radial) => state.gradient.radial = radial,
            EditAction::GradientTransparent => {
                state.gradient.transparent = !state.gradient.transparent
            }
            EditAction::GradientReverse => state.gradient.reverse = !state.gradient.reverse,
            EditAction::Layers(action) => {
                state.selection.transform = None;
                state.selection.draft = None;
                if state.tool == PhotoTool::Transform {
                    state.select_tool(PhotoTool::Move);
                }
                layers::dispatch(state, id, action);
                viewer.gesture = None;
            }
            EditAction::Tool(tool) => {
                state.select_tool(tool);
            }
            EditAction::ModelPanel => state.model_panel_closed ^= true,
            EditAction::ModelTool(tool) => {
                if state.model_reference(Some(id)).is_some() {
                    state.model_tool = (tool != Tool::Universal).then_some(tool);
                    state.selection.transform = None;
                    state.selection.draft = None;
                    viewer.gesture = None;
                }
            }
            EditAction::EraserMode(mode) => {
                if state.tool != PhotoTool::Eraser {
                    state.select_tool(PhotoTool::Eraser);
                }
                state.select_eraser_mode(mode);
                state.paint_menu = false;
            }
            EditAction::DrawingTool => state.select_tool(state.drawing_tool),
            EditAction::PixelShape(shape) => {
                if state.tool == PhotoTool::Eraser {
                    state.pixel_eraser_shape = shape;
                } else {
                    state.pencil_shape = shape;
                }
                state.gesture = None;
            }
            EditAction::PaintMenu | EditAction::EraserMenu => {
                let menu = if matches!(action, EditAction::EraserMenu) {
                    ToolMenu::Eraser
                } else {
                    ToolMenu::Paint
                };
                state.paint_menu = !state.paint_menu || state.tool_menu != menu;
                state.tool_menu = menu;
                state.menu = false;
                state.gesture = None;
                viewer.gesture = None;
                viewer.last_click = None;
            }
            EditAction::Size(value) => {
                state.set_size(value);
            }
            EditAction::SizeMenu => {
                state.size_menu ^= true;
                state.menu = false;
                state.paint_menu = false;
                state.gesture = None;
                viewer.gesture = None;
                viewer.last_click = None;
            }
            EditAction::Hardness(value) => state.hardness = value.clamp(0.0, 1.0),
            EditAction::BrushCategory(category) => {
                if state.tool == PhotoTool::Brush
                    && state.size_menu
                    && category < brush_library::library().categories.len()
                {
                    state.brush_category = category;
                }
            }
            EditAction::BrushPreset(preset) => {
                if state.tool == PhotoTool::Brush
                    && state.size_menu
                    && preset.is_none_or(|index| index < brush_library::library().presets.len())
                {
                    if state.brush_preset != preset {
                        state.preset_sizes.insert(state.brush_preset, state.size);
                        let diameter = state
                            .preset_sizes
                            .get(&preset)
                            .copied()
                            .or_else(|| {
                                preset.map(|index| brush_library::library().presets[index].diameter)
                            })
                            .unwrap_or(24.0);
                        state.set_size(diameter);
                        state.brush_preset = preset;
                    }
                    state.size_menu = false;
                    state.size_menu_dismissed = true;
                }
            }
            EditAction::BrushOpacity(value) => state.opacity = value.clamp(0.01, 1.0),
            #[cfg(test)]
            EditAction::Color(color) => {
                if let Some(picker) = &mut state.color_picker {
                    picker.set_color(color);
                } else {
                    state.set_color(color);
                }
            }
            EditAction::ColorPicker(target) => {
                state.color_picker = Some(color::Picker::new(
                    target,
                    match target {
                        ColorTarget::Foreground => state.color(),
                        ColorTarget::Background => state.background(),
                        ColorTarget::LayerField(index) => state
                            .layers
                            .form
                            .as_ref()
                            .and_then(|f| f.fields.get(index))
                            .and_then(|f| layers::parse_color(&f.text))
                            .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                    },
                ));
                state.menu = false;
                state.paint_menu = false;
                state.gesture = None;
                viewer.gesture = None;
                viewer.last_click = None;
            }
            EditAction::ColorConfirm => {
                if state
                    .color_picker
                    .as_ref()
                    .is_some_and(|p| p.error.is_none())
                    && let Some(picker) = state.color_picker.take()
                {
                    match picker.target {
                        ColorTarget::Foreground => {
                            state.set_color(picker.draft);
                            state.hsv = Some(picker.hsv);
                        }
                        ColorTarget::Background => state.background = Some(picker.draft),
                        ColorTarget::LayerField(index) => {
                            if let Some(field) = state
                                .layers
                                .form
                                .as_mut()
                                .and_then(|f| f.fields.get_mut(index))
                            {
                                let mut chosen = picker.draft;
                                chosen[3] = layers::parse_color(&field.text).map_or(1.0, |c| c[3]);
                                field.text = layers::color_hex(chosen);
                                field.enabled = true;
                                state.layers.ui_revision = state.layers.ui_revision.wrapping_add(1);
                            }
                        }
                    }
                }
            }
            EditAction::ColorCancel => state.color_picker = None,
            EditAction::ColorComponent(channel, value) => {
                if let Some(picker) = &mut state.color_picker {
                    picker.set_component(channel, value);
                }
            }
            EditAction::ColorDraft(value) => {
                if let Some(picker) = &mut state.color_picker {
                    picker.set_color(value);
                }
            }
            EditAction::ColorSv(s, v) => {
                if let Some(picker) = &mut state.color_picker {
                    picker.sv(s, v);
                }
            }
            EditAction::SwapColors => {
                let fg = state.color();
                state.set_color(state.background());
                state.background = Some(fg);
            }
            EditAction::ResetColors => {
                state.set_color([0.0, 0.0, 0.0, 1.0]);
                state.background = Some([1.0; 4]);
            }
            EditAction::AddSwatch => {
                if let Some(picker) = &state.color_picker
                    && state.swatches.len() < 24
                    && !state
                        .swatches
                        .iter()
                        .any(|c| color::hex(*c) == picker.hex())
                {
                    state.swatches.push(picker.draft);
                }
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
                    EditAction::DeleteLayer => session.doc.delete_layer(),
                    EditAction::Visible(index) => session.doc.visibility(index),
                    EditAction::Opacity(layer, value, final_value) => {
                        if state.layers.blend_menu != Some(layer)
                            || session.doc.document.layer(layer).is_none()
                        {
                            continue;
                        }
                        let opacity = value.clamp(0.0, 1.0);
                        if !opacity.is_finite() {
                            continue;
                        }
                        if session.doc.document.layer(layer).unwrap().opacity != opacity {
                            if state.opacity_drag != Some((id, layer)) {
                                session.doc.checkpoint();
                                state.opacity_drag = Some((id, layer));
                                if session.doc.size().area() >= 512 * 512 {
                                    session.opacity_preview = Some(OpacityPreview {
                                        layer,
                                        revision: session.doc.revision,
                                        view: session.doc.view_revision,
                                        proxy: None,
                                        task: None,
                                        displayed: None,
                                    });
                                }
                            }
                            session.doc.document.layer_mut(layer).unwrap().opacity = opacity;
                            session.doc.changed();
                            if let Some(preview) = &mut session.opacity_preview {
                                preview.revision = session.doc.revision;
                            }
                        }
                        if final_value {
                            state.opacity_drag = None;
                        }
                    }
                    EditAction::SelectLayer(index) => session.doc.select(index, false, false),
                    _ => {}
                }
            }
            _ => {}
        }
        if matches!(
            action,
            EditAction::SelectLayer(_) | EditAction::Layers(layers::LayerAction::Select(..))
        ) {
            state.model_tool = state.model_reference(Some(id)).map(|_| Tool::Move);
        }
    }
    state
        .sessions
        .retain(|id, _| editor.image_tab(*id).is_some());
    if let Some(id) = editor.viewing_image {
        layers::preview(state, id);
    }
    for tab in &mut editor.image_tabs {
        tab.dirty = state.sessions.get(&tab.id).is_some_and(|s| s.doc.dirty())
            || state
                .selection
                .transform
                .as_ref()
                .is_some_and(|t| t.id == tab.id && t.deferred);
    }
}

pub(super) fn sync(
    viewer: &mut ImageViewer,
    editor: &Editor,
    images: &mut Assets<Image>,
    mut uploads: Option<&mut crate::editor::image_texture::TextureUploads>,
) {
    selection::poll(&mut viewer.edit);
    let state = &mut viewer.edit;
    if let Some(id) = editor.viewing_image
        && let Some(session) = state.sessions.get_mut(&id)
    {
        let mut opacity_frame = None;
        if let Some(preview) = &mut session.opacity_preview {
            let dragging = state.opacity_drag == Some((id, preview.layer));
            if preview.revision != session.doc.revision
                || preview.view != session.doc.view_revision
                || (!dragging && session.rendered == Some((preview.revision, preview.view)))
            {
                session.opacity_preview = None;
            } else {
                let Some(frame) = preview.poll(&session.doc, dragging) else {
                    return;
                };
                let settled = frame.exact && frame.revision == session.doc.revision;
                if settled && !dragging {
                    session.opacity_preview = None;
                }
                opacity_frame = Some((frame.image, settled));
            }
        }
        if opacity_frame.is_some()
            || session.rendered != Some((session.doc.revision, session.doc.view_revision))
            || viewer.tab != Some(id)
            || viewer.image.is_none()
        {
            let source_size = session.doc.size();
            let size = UVec2::new(source_size.width, source_size.height);
            let initialized = session.rendered.is_some() && viewer.tab == Some(id);
            let damage = session.doc.take_damage();
            if initialized
                && opacity_frame.is_none()
                && size.max_element() <= MAX_TEXTURE
                && let (Some(rect), Some(handle), Some(uploads)) =
                    (damage, viewer.image.as_ref(), uploads.as_deref_mut())
                && let Some(image) = images.get_mut_untracked(handle)
                && image.width() == size.x
                && image.height() == size.y
                && let Some(data) = image.data.as_mut()
                && data.len() == (size.x * size.y * 4) as usize
            {
                if !rect.is_empty() {
                    let region = session.doc.preview_region(rect);
                    let stride = size.x as usize * 4;
                    let row_len = region.width as usize * 4;
                    for (y, row) in region.pixels.chunks_exact(row_len).enumerate() {
                        let start = (rect.y0 as usize + y) * stride + rect.x0 as usize * 4;
                        data[start..start + row_len].copy_from_slice(row);
                    }
                    uploads.patch(
                        handle.id(),
                        UVec2::new(rect.x0 as u32, rect.y0 as u32),
                        UVec2::new(region.width, region.height),
                        region.pixels,
                    );
                }
                session.rendered = Some((session.doc.revision, session.doc.view_revision));
                return;
            }
            let exact = opacity_frame.as_ref().is_none_or(|(_, exact)| *exact);
            let composite = opacity_frame
                .map(|(image, _)| image)
                .or_else(|| {
                    session
                        .prepared
                        .take()
                        .filter(|(revision, view, _)| {
                            (*revision, *view) == (session.doc.revision, session.doc.view_revision)
                        })
                        .map(|(_, _, image)| image)
                })
                .unwrap_or_else(|| session.doc.preview());
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
            // The canvas shader selects sharp pixels by zoom, independently of the active tool.
            texture.sampler = ImageSampler::linear();
            if let Some(mut image) = viewer
                .image
                .as_ref()
                .and_then(|handle| images.get_mut(handle))
            {
                *image = texture;
            } else {
                viewer.image = Some(images.add(texture));
            }
            if let (Some(uploads), Some(handle)) = (uploads, viewer.image.as_ref()) {
                uploads.reset(handle.id());
            }
            viewer.dimensions = Some(UVec2::new(source_size.width, source_size.height));
            viewer.decoded_size = Some(decoded_size);
            viewer.error = None;
            session.rendered = exact.then_some((session.doc.revision, session.doc.view_revision));
        }
    }
}

pub(super) fn pointer(
    viewer: &mut ImageViewer,
    editor: &mut Editor,
    rect: Rect,
    input_rect: Rect,
    mouse: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    focus: &mut InputFocus,
    samples: &[super::paint_input::Sample],
) -> bool {
    if viewer.edit.selection.menu != 0 || viewer.edit.selection.dismissed {
        viewer.edit.gesture = None;
        return true;
    }
    if selection::pointer(
        viewer, editor, rect, input_rect, mouse, keys, focus, samples,
    ) {
        return true;
    }
    if viewer.panel.blocks_canvas()
        || viewer.edit.color_picker.is_some()
        || viewer.edit.layers.menu
        || viewer.edit.layers.form.is_some()
    {
        viewer.edit.gesture = None;
        return true;
    }
    let Some(id) = editor.viewing_image else {
        return false;
    };
    if !viewer.edit.editing(Some(id)) {
        viewer.edit.gesture = None;
        return false;
    }
    if viewer.edit.busy()
        || viewer.edit.menu
        || viewer.edit.mode_menu_dismissed
        || viewer.edit.paint_menu
        || viewer.edit.paint_menu_dismissed
        || viewer.edit.size_menu
        || viewer.edit.size_menu_dismissed
    {
        viewer.edit.gesture = None;
        return true;
    }
    if matches!(viewer.edit.gesture, Some((_, Gesture::Gradient(_))))
        && (keys.just_pressed(KeyCode::Escape)
            || keys.pressed(KeyCode::Space)
            || mouse.pressed(MouseButton::Middle))
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
    let Some(dimensions) = viewer.dimensions else {
        return true;
    };
    // Input and the visible pixel grid use the same pixel-aligned document origin.
    let image = viewer.image_rect(dimensions, rect);
    let size = image.size();
    let image_min = image.min;
    let state = &mut viewer.edit;
    let mut points = Vec::new();
    let mut position = None;
    let mut pressure = 1.0;
    for &sample in samples {
        use super::paint_input::Sample;
        let (cursor, down, up) = match sample {
            Sample::Pressure(value) => {
                pressure = value;
                continue;
            }
            Sample::Down(point) => (point, true, false),
            Sample::Move(point) => (point, false, false),
            Sample::Up(point) => (point, false, true),
        };
        // Only a workspace press can start painting. Once captured, keep the
        // complete trajectory over panels and outside the window until release.
        let captured = matches!(
            state.gesture,
            Some((gesture_id, Gesture::Paint { .. } | Gesture::PickColor { .. } | Gesture::Gradient(_))) if gesture_id == id
        );
        if !input_rect.contains(cursor) && (down || !captured) {
            flush(state, id, &points, position, true);
            state.gesture = None;
            points.clear();
            position = None;
            continue;
        }
        let point = (cursor - image_min) / size * dimensions.as_vec2();
        if !point.is_finite() {
            continue;
        }
        if down {
            flush(state, id, &points, position, true);
            points.clear();
            state.gesture = None;
            let picking = state.eyedropper(keys, Some(id));
            // Painting owns a press anywhere in the workspace, including the
            // pasteboard. Point tools still need an actual document pixel.
            if (picking || !state.sizeable())
                && (!point.cmpge(Vec2::ZERO).all() || !point.cmplt(dimensions.as_vec2()).all())
            {
                continue;
            }
            focus.clear();
            let color = state.color();
            let gradient_colors = state.gradient.colors(color, state.background());
            let mut brush = state.brush();
            let Some(session) = state.sessions.get_mut(&id) else {
                continue;
            };
            if session.doc.mask_target && brush.erase {
                brush.erase = false;
                brush.color = [0.0, 0.0, 0.0, 1.0];
            }
            if picking {
                state.gesture = Some((id, Gesture::PickColor { last: None }));
            } else if state.tool == PhotoTool::Move && session.doc.movable() {
                state.gesture = Some((
                    id,
                    Gesture::Move {
                        original: session.doc.begin_move(),
                        origin: point,
                    },
                ));
            } else if !session.doc.paintable() {
                editor.status = "Выберите видимый незаблокированный слой".into();
            } else if state.tool == PhotoTool::Fill {
                if let Err(error) = session.doc.fill(point.x as i32, point.y as i32, color) {
                    state.error = Some(error);
                }
            } else if state.tool == PhotoTool::Gradient {
                state.gesture = Some((
                    id,
                    Gesture::Gradient(Draft {
                        from: point,
                        to: point,
                        colors: gradient_colors,
                        radial: state.gradient.radial,
                        min_length: 4.0 * dimensions.x as f32 / size.x,
                        revision: session.doc.revision,
                        view_revision: session.doc.view_revision,
                        layer: session.doc.active_id(),
                        target: session.doc.paint_target(),
                    }),
                ));
            } else if let Some(original) = session.doc.paint_surface().cloned() {
                let gesture = {
                    let renderer =
                        photocraft_paint::StrokeRenderer::new(&brush, Some(original.format()), 1.0)
                            .with_clip(photocraft_geom::Rect::from_size(session.doc.size()));
                    Gesture::Paint {
                        original,
                        checkpoint: Some(session.doc.snapshot()),
                        renderer,
                        last: None,
                        last_pressure: -1.0,
                        tail: photocraft_geom::Rect::EMPTY,
                    }
                };
                state.gesture = Some((id, gesture));
            }
        }
        sample_color(state, id, point, dimensions);
        if let Some((tab, Gesture::Gradient(draft))) = &mut state.gesture
            && *tab == id
        {
            draft.to = gradient::snap(
                draft.from,
                point,
                keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            );
        }
        let point = state.snap_point(point);
        position = Some(point);
        if let Some((
            gesture_id,
            Gesture::Paint {
                last,
                last_pressure,
                ..
            },
        )) = &mut state.gesture
            && *gesture_id == id
            && (last.is_none_or(|last| last.distance_squared(point) > 0.01)
                || *last_pressure != pressure)
        {
            points.push(StrokePoint::new(point.x as f64, point.y as f64, pressure));
            *last = Some(point);
            *last_pressure = pressure;
        }
        if up {
            if let Some(draft) = state.gradient_draft(id)
                && let Some(session) = state.sessions.get_mut(&id)
                && draft.valid_for(&session.doc)
                && let Err(error) = session.doc.layer_command("paint.gradient", draft.params())
            {
                state.error = Some(error);
            }
            flush(state, id, &points, position, true);
            state.gesture = None;
            points.clear();
            position = None;
        }
    }
    let finished = !mouse.pressed(MouseButton::Left);
    // A lost pointer capture cancels the draft. Only an actual Up applies it.
    if finished && matches!(state.gesture, Some((_, Gesture::Gradient(_)))) {
        state.gesture = None;
    }
    flush(state, id, &points, position, finished);
    if finished {
        state.gesture = None;
    }
    if let Some(tab) = editor.image_tabs.iter_mut().find(|tab| tab.id == id) {
        tab.dirty = state
            .sessions
            .get(&id)
            .is_some_and(|session| session.doc.dirty());
    }
    true
}

fn sample_color(state: &mut State, id: usize, point: Vec2, dimensions: UVec2) {
    if !point.cmpge(Vec2::ZERO).all() || !point.cmplt(dimensions.as_vec2()).all() {
        return;
    }
    let pixel = point.floor().as_ivec2();
    if let Some((gesture_id, Gesture::PickColor { last })) = &mut state.gesture
        && *gesture_id == id
        && *last != Some(pixel)
        && let Some(session) = state.sessions.get(&id)
    {
        *last = Some(pixel);
        let composite = session.doc.composite_region(photocraft_geom::Rect::new(
            pixel.x,
            pixel.y,
            pixel.x + 1,
            pixel.y + 1,
        ));
        if let Some(pixel) = composite.pixels.get(..4) {
            state.set_color([
                pixel[0] as f32 / 255.0,
                pixel[1] as f32 / 255.0,
                pixel[2] as f32 / 255.0,
                1.0,
            ]);
        }
    }
}

fn flush(
    state: &mut State,
    id: usize,
    points: &[StrokePoint],
    position: Option<Vec2>,
    finish: bool,
) {
    let mut failure = None;
    if let Some((gesture_id, gesture)) = &mut state.gesture
        && *gesture_id == id
        && let Some(session) = state.sessions.get_mut(&id)
    {
        match gesture {
            Gesture::Paint {
                original,
                checkpoint,
                renderer,
                tail,
                ..
            } if !points.is_empty() || finish => {
                let revision = session.doc.revision;
                if let Err(error) = session
                    .doc
                    .paint_live(original, renderer, points, tail, finish)
                {
                    failure = Some(error);
                }
                // Capture the original document at Down, but only create an undo
                // entry once this gesture actually reaches the visible image.
                if session.doc.revision != revision
                    && let Some(before) = checkpoint.take()
                {
                    session.doc.checkpoint_from(before);
                }
            }
            Gesture::Move { original, origin } => {
                if let Some(position) = position {
                    let delta = (position - *origin)
                        .round()
                        .clamp(Vec2::splat(-32768.0), Vec2::splat(32768.0));
                    if let Err(error) =
                        session
                            .doc
                            .translate_live(original, delta.x as i32, delta.y as i32)
                    {
                        failure = Some(error);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(error) = failure {
        state.error = Some(error);
        state.gesture = None;
    }
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
    if selection::shortcuts(keys, viewer, actions) {
        return true;
    }
    if keys.just_pressed(KeyCode::Escape)
        && matches!(
            viewer.edit.gesture,
            Some((_, Gesture::Move { .. } | Gesture::Gradient(_)))
        )
    {
        actions.write(Action::ImageEdit(EditAction::CancelGesture));
        return true;
    }
    if viewer.edit.layers.menu || viewer.edit.layers.form.is_some() {
        if keys.just_pressed(KeyCode::Escape) {
            actions.write(Action::ImageEdit(EditAction::Layers(
                layers::LayerAction::Cancel,
            )));
        }
        return true;
    }
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    if command {
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
        let layer_command = if keys.just_pressed(KeyCode::KeyG) {
            Some(if shift {
                "layer.ungroupLayers"
            } else if alt {
                "layer.createClippingMask"
            } else {
                "layer.groupLayers"
            })
        } else if keys.just_pressed(KeyCode::KeyJ) {
            Some("layer.duplicate")
        } else if keys.just_pressed(KeyCode::KeyE) {
            Some(if shift {
                "layer.mergeVisible"
            } else {
                "layer.mergeLayers"
            })
        } else if shift && keys.just_pressed(KeyCode::KeyN) {
            Some("layer.new.layer")
        } else {
            None
        };
        if let Some(command) = layer_command {
            actions.write(Action::ImageEdit(EditAction::Layers(
                layers::LayerAction::Command(command),
            )));
            return true;
        }
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
        if viewer.edit.model_reference(viewer.tab).is_some() {
            for (key, tool) in [
                (KeyCode::KeyQ, Tool::Universal),
                (KeyCode::KeyG, Tool::Move),
                (KeyCode::KeyR, Tool::Rotate),
                (KeyCode::KeyS, Tool::Scale),
            ] {
                if keys.just_pressed(key) {
                    actions.write(Action::ImageEdit(EditAction::ModelTool(tool)));
                    return true;
                }
            }
        }
        if keys.just_pressed(KeyCode::Delete) {
            actions.write(Action::ImageEdit(EditAction::DeleteLayer));
            return true;
        }
        if keys.just_pressed(KeyCode::F2) {
            actions.write(Action::ImageEdit(EditAction::Layers(
                layers::LayerAction::Command("layer.renameLayer"),
            )));
            return true;
        }
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
            (KeyCode::KeyU, PhotoTool::Gradient),
            (KeyCode::KeyI, PhotoTool::Eyedropper),
            (KeyCode::KeyV, PhotoTool::Move),
            (KeyCode::KeyH, PhotoTool::Hand),
        ] {
            if keys.just_pressed(key) {
                actions.write(Action::ImageEdit(EditAction::Tool(tool)));
                return true;
            }
        }
        for (key, action) in [
            (KeyCode::KeyX, EditAction::SwapColors),
            (KeyCode::KeyD, EditAction::ResetColors),
        ] {
            if keys.just_pressed(key) {
                actions.write(Action::ImageEdit(action));
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

    #[test]
    fn pixel_eraser_removes_whole_cells_without_soft_edges() {
        let mut state = State {
            tool: PhotoTool::Eraser,
            eraser_mode: EraserMode::Pixel,
            size: 1.0,
            hardness: 0.1,
            opacity: 1.0,
            ..Default::default()
        };
        let opaque = || {
            let mut surface = Surface::new(PixelFormat::RGBA8);
            surface.fill_rect(photocraft_geom::Rect::new(0, 0, 20, 16), &[1.0; 4]);
            surface
        };
        for size in [1.0, 2.0, 3.0, 7.0] {
            state.size = size;
            let brush = state.brush();
            assert!(brush.aliased && brush.erase);
            assert_eq!(brush.hardness, 1.0);
            assert_eq!(brush.flow, 1.0);
            let points = [Vec2::new(2.1, 6.2), Vec2::new(14.7, 6.8)]
                .map(|point| state.snap_point(point))
                .map(|p| StrokePoint::new(p.x as f64, p.y as f64, 1.0));
            let mut pixels = opaque();
            photocraft_paint::render_stroke(&mut pixels, &brush, &points, None, false, 1.0);
            for y in 0..16 {
                for x in 0..20 {
                    let alpha = pixels.rgba(x, y)[3];
                    assert!(
                        alpha == 0.0 || alpha == 1.0,
                        "Pixel eraser size {size} smoothed {x},{y}: {alpha}"
                    );
                    if size == 1.0 {
                        assert_eq!(
                            alpha,
                            if y == 6 && (2..=14).contains(&x) {
                                0.0
                            } else {
                                1.0
                            },
                            "One-pixel eraser touched the wrong cell {x},{y}"
                        );
                    }
                }
            }
        }
        state.eraser_mode = EraserMode::Brush;
        state.size = 7.0;
        let mut soft = opaque();
        photocraft_paint::render_stroke(
            &mut soft,
            &state.brush(),
            &[StrokePoint::new(7.3, 7.7, 1.0)],
            None,
            false,
            1.0,
        );
        assert!(
            (0..16).any(|y| (0..20).any(|x| {
                let alpha = soft.rgba(x, y)[3];
                alpha > 0.0 && alpha < 1.0
            })),
            "Normal eraser must keep its soft edge"
        );
        state.eraser_mode = EraserMode::Pixel;
        state.opacity = 0.5;
        assert_eq!(
            state.brush().opacity,
            0.5,
            "Respect explicitly chosen opacity, as PhotoCraft does"
        );
        assert_eq!(
            state.hardness, 0.1,
            "Pixel mode must not overwrite the soft eraser settings"
        );
    }

    #[test]
    fn eraser_modes_remember_sizes_and_snap_even_and_odd_tips() {
        let mut state = State {
            size: 24.0,
            pencil_size: 1.0,
            eraser_size: 12.0,
            pixel_eraser_size: 1.0,
            ..Default::default()
        };
        state.select_tool(PhotoTool::Eraser);
        assert_eq!(state.size, 12.0);
        state.select_eraser_mode(EraserMode::Pixel);
        assert_eq!(state.size, 1.0);
        assert!(state.pixel_square());
        assert_eq!(state.snap_point(Vec2::new(2.1, 6.8)), Vec2::new(2.5, 6.5));
        state.size = 2.0;
        assert_eq!(state.snap_point(Vec2::new(2.1, 6.8)), Vec2::new(2.0, 7.0));
        state.size = 7.0;
        assert!(!state.pixel_square());
        state.select_eraser_mode(EraserMode::Brush);
        assert_eq!(state.size, 12.0);
        state.size = 16.0;
        state.select_eraser_mode(EraserMode::Pixel);
        assert_eq!(state.size, 7.0);
        state.select_tool(PhotoTool::Brush);
        assert_eq!(state.size, 24.0);
        state.select_tool(PhotoTool::Pencil);
        assert_eq!(state.size, 1.0);
        state.select_tool(PhotoTool::Eraser);
        assert_eq!(state.eraser_mode, EraserMode::Pixel);
        assert_eq!(state.size, 7.0);
        state.select_eraser_mode(EraserMode::Brush);
        assert_eq!(state.size, 16.0);
        assert_eq!(state.snap_point(Vec2::new(2.1, 6.8)), Vec2::new(2.1, 6.8));
    }
}

#[cfg(test)]
mod opacity_preview_tests {
    use super::*;

    #[test]
    fn release_during_a_proxy_render_publishes_the_latest_exact_pixels() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opacity.png");
        ::image::RgbaImage::from_pixel(1536, 64, ::image::Rgba([63u8, 107, 203, 211]))
            .save(&path)
            .unwrap();
        let mut doc = PhotoDocument::load(&path).unwrap();
        let layer = doc.active_id().unwrap();
        let mut work = OpacityPreview {
            layer,
            revision: doc.revision,
            view: doc.view_revision,
            task: None,
            proxy: None,
            displayed: None,
        };
        assert!(work.poll(&doc, true).is_none());
        doc.document.layer_mut(layer).unwrap().opacity = 0.35;
        doc.changed();
        work.revision = doc.revision;
        let timeout = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let frame = loop {
            if let Some(frame) = work.poll(&doc, false) {
                break frame;
            }
            assert!(std::time::Instant::now() < timeout);
            std::thread::yield_now();
        };
        assert!(frame.exact);
        assert_eq!(frame.revision, doc.revision);
        assert_eq!(frame.image.width, 1536);
        assert_eq!(frame.image.pixels, doc.preview().pixels);
    }
}
