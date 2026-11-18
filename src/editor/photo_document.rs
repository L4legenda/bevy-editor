//! The embedded PhotoCraft document, commands and native persistence.
use image::{ImageDecoder, ImageFormat};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::{Rect, Size};
#[cfg(test)]
use photocraft_paint::BrushSettings;
use photocraft_paint::StrokePoint;
use photocraft_raster::{Rgba8Image, Surface};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

pub(super) const MAX_PIXELS: u64 = 16_777_216;
pub(super) const MAX_LAYERS: usize = 4096;
const HISTORY: usize = 32;

#[path = "photo_layers.rs"]
mod layers;
#[path = "photo_libraries.rs"]
mod libraries;
#[path = "photo_selection.rs"]
mod selection;
pub(super) use libraries::LayerLibraries;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SelectionTool {
    #[default]
    Rectangle,
    Ellipse,
    Lasso,
    Polygon,
    Wand,
    Quick,
    Object,
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PhotoTool {
    Hand,
    Move,
    #[default]
    Brush,
    Pencil,
    Eraser,
    Fill,
    Eyedropper,
    Select(SelectionTool),
    Transform,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    document: Document,
    active: usize,
    selected: Vec<LayerId>,
    mask_target: bool,
    filter_target: bool,
    revision: u64,
}

pub(super) struct LayerMove {
    before: Snapshot,
    evicted: Option<Snapshot>,
    redo: Vec<Snapshot>,
    applied: bool,
}

pub(super) struct PhotoDocument {
    pub document: Document,
    pub active: usize,
    pub selected: Vec<LayerId>,
    pub mask_target: bool,
    pub filter_target: bool,
    pub mask_view: Option<photocraft_engine::commands::mask_view_cmds::LayerMaskView>,
    pub view_revision: u64,
    tool_colors: ([f32; 4], [f32; 4]),
    engine: Option<std::sync::Mutex<Box<photocraft_engine::Session>>>,
    parents: Vec<layers::Parent>,
    pub revision: u64,
    pub saved_revision: u64,
    next_revision: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    // None invalidates the whole preview; a rectangle limits interactive redraws.
    damage: Option<Rect>,
}

impl PhotoDocument {
    pub fn load(source: &Path) -> Result<Self, String> {
        source_format(source)?;
        let native = sidecar(source);
        let document = if native.exists() {
            let limits = photocraft_format::LoadOptions {
                max_canvas_pixels: MAX_PIXELS,
                max_canvas_side: 32768,
                max_total_bytes: 512 << 20,
                max_blob_bytes: 128 << 20,
                ..Default::default()
            };
            let document = photocraft_format::load_path_with(&native, &limits)
                .map_err(|e| format!("Не удалось открыть {}: {e}", native.display()))?;
            validate(&document)?;
            if image::ImageFormat::from_path(&document.name).is_ok()
                && !document
                    .name
                    .eq_ignore_ascii_case(&source.file_name().unwrap_or_default().to_string_lossy())
            {
                return Err(format!(
                    "Этот .pcraft относится к «{}». Изображения с одинаковым именем и разными расширениями разместите в разных папках.",
                    document.name
                ));
            }
            document
        } else {
            let mut reader = image::ImageReader::open(source)
                .map_err(|e| e.to_string())?
                .with_guessed_format()
                .map_err(|e| e.to_string())?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(256 << 20);
            reader.limits(limits);
            let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
            let (width, height) = decoder.dimensions();
            if u64::from(width) * u64::from(height) > MAX_PIXELS {
                return Err("Редактирование доступно для изображений до 16 Мп.".into());
            }
            if decoder.color_type().bits_per_pixel() / decoder.color_type().channel_count() as u16
                > 8
            {
                return Err(
                    "Базовый редактор пока поддерживает изображения с 8 битами на канал.".into(),
                );
            }
            let orientation = decoder.orientation().map_err(|e| e.to_string())?;
            let mut image =
                image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
            image.apply_orientation(orientation);
            let image = image.into_rgba8();
            let size = Size::new(image.width(), image.height());
            let mut surface = Surface::new(PixelFormat::RGBA8);
            surface.write_interleaved(Rect::from_size(size), image.as_raw());
            let mut doc = Document::new(
                source.file_name().unwrap_or_default().to_string_lossy(),
                size,
                ColorMode::Rgb,
                SampleType::U8,
            );
            doc.layers.push(Layer::new(
                "Исходное изображение",
                LayerContent::Raster(surface),
            ));
            validate(&doc)?;
            // Never replace an existing native project when entering edit mode.
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default())
                .map_err(|e| e.to_string())?;
            photocraft_format::write_noreplace(&native, &bytes).map_err(|e| e.to_string())?;
            doc
        };
        let active = document.walk().len().saturating_sub(1);
        let selected = document
            .walk()
            .into_iter()
            .nth(active)
            .map(|(_, _, l)| l.id)
            .into_iter()
            .collect();
        Ok(Self {
            document,
            active,
            selected,
            mask_target: false,
            filter_target: false,
            mask_view: None,
            view_revision: 0,
            tool_colors: ([0.0, 0.0, 0.0, 1.0], [1.0; 4]),
            engine: None,
            parents: Vec::new(),
            revision: 1,
            saved_revision: 1,
            next_revision: 2,
            undo: Vec::new(),
            redo: Vec::new(),
            damage: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.revision != self.saved_revision || self.parents.iter().any(|p| p.dirty())
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn size(&self) -> Size {
        self.document.size
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            document: self.document.clone(),
            active: self.active,
            selected: self.selected.clone(),
            mask_target: self.mask_target,
            filter_target: self.filter_target,
            revision: self.revision,
        }
    }
    pub fn checkpoint(&mut self) {
        self.checkpoint_from(self.snapshot());
    }
    pub fn checkpoint_from(&mut self, snapshot: Snapshot) {
        self.undo.push(snapshot);
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn changed(&mut self) {
        self.damage = None;
        self.revision = self.next_revision;
        self.next_revision += 1;
    }
    pub fn take_damage(&mut self) -> Option<Rect> {
        std::mem::replace(&mut self.damage, Some(Rect::EMPTY))
    }
    fn changed_region(&mut self, damage: Rect) {
        if self.document.walk().iter().any(|(_, _, l)| {
            !l.effects.items.is_empty()
                || l.mask.as_ref().is_some_and(|m| m.feather > 0.0)
                || l.vector_mask.is_some()
                || matches!(l.content, LayerContent::Adjustment(_))
        }) {
            self.changed();
            return;
        }
        let damage = damage.intersect(&Rect::from_size(self.size()));
        if damage.is_empty() {
            return;
        }
        if let Some(previous) = &mut self.damage {
            *previous = previous.union(&damage);
        }
        self.revision = self.next_revision;
        self.next_revision += 1;
    }
    fn restore(&mut self, snapshot: Snapshot) {
        self.document = snapshot.document;
        self.active = snapshot.active;
        self.selected = snapshot.selected;
        self.mask_target = snapshot.mask_target;
        self.filter_target = snapshot.filter_target;
        self.revision = snapshot.revision;
        self.damage = None;
        if self.mask_view.is_some_and(|view| {
            self.active_id() != Some(view.layer)
                || self.document.layer(view.layer).is_none_or(|layer| {
                    photocraft_engine::commands::mask_view_cmds::viewed_mask(layer, view.filter)
                        .is_none()
                })
        }) {
            self.mask_view = None;
            self.view_revision += 1;
        }
    }
    pub fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(snapshot);
        }
    }
    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(snapshot);
        }
    }
    pub fn add_layer(&mut self) {
        let _ = self.layer_command("layer.new.layer", serde_json::json!({}));
    }
    #[cfg(test)]
    pub fn duplicate_layer(&mut self) {
        let _ = self.layer_command("layer.duplicate", serde_json::json!({}));
    }
    pub fn delete_layer(&mut self) {
        let _ = self.layer_command("layer.delete", serde_json::json!({}));
    }
    #[cfg(test)]
    pub fn reorder(&mut self, up: bool) {
        let _ = self.layer_command(
            if up {
                "layer.arrange.bringForward"
            } else {
                "layer.arrange.sendBackward"
            },
            serde_json::json!({}),
        );
    }
    pub fn visibility(&mut self, index: usize) {
        if let Some(layer) = self.layer(index) {
            let params = serde_json::json!({"layer": layer.id.0, "visible": !layer.visible});
            let _ = self.layer_command("layer.setProps", params);
        }
    }
    #[cfg(test)]
    pub fn opacity(&mut self, value: f32) {
        if value.is_finite() {
            let _ = self.layer_command(
                "layer.setProps",
                serde_json::json!({"opacity": value.clamp(0.0, 1.0)}),
            );
        }
    }
    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() && self.active_layer().is_some_and(|l| l.name != name) {
            let _ = self.layer_command(
                "layer.setProps",
                serde_json::json!({"name": name.chars().take(100).collect::<String>()}),
            );
        }
    }
    pub fn paintable(&self) -> bool {
        let Some(id) = self.active_id() else {
            return false;
        };
        let Some(path) = self.document.path_of(id) else {
            return false;
        };
        let locks = self.document.effective_locks(id);
        self.document.shown_at(&path)
            && !locks.all
            && (self.mask_target || !locks.pixels)
            && (self.mask_target
                || self
                    .active_layer()
                    .is_some_and(|l| matches!(l.content, LayerContent::Raster(_))))
            && self.paint_surface().is_some()
    }
    #[cfg(test)]
    pub fn paint(&mut self, original: &Surface, brush: &BrushSettings, points: &[StrokePoint]) {
        let bounds = Rect::from_size(self.size());
        if let Some(surface) = self.paint_surface_mut() {
            *surface = original.clone();
            photocraft_paint::render_stroke_clipped(
                surface,
                brush,
                points,
                None,
                false,
                1.0,
                Some(bounds),
            );
            self.changed();
        }
    }
    #[cfg(test)]
    pub fn move_layer(&mut self, original: &Surface, dx: i32, dy: i32) {
        let bounds = Rect::from_size(self.size());
        if let Some(surface) = self.paint_surface_mut() {
            *surface = original.translated(dx, dy, bounds);
            self.changed();
        }
    }
    pub fn paint_live(
        &mut self,
        original: &Surface,
        renderer: &mut photocraft_paint::StrokeRenderer,
        points: &[StrokePoint],
        tail: &mut Rect,
        finish: bool,
    ) -> Result<(), String> {
        renderer.push(points);
        if finish {
            renderer.finish();
        }
        let lock_transparency = !self.mask_target
            && self
                .active_id()
                .is_some_and(|id| self.document.effective_locks(id).transparency);
        let selection = self.document.selection.clone();
        let filter = self.mask_target && self.filter_target;
        let before = filter.then(|| self.document.clone());
        let mut changed = None;
        if let Some(surface) = self.paint_surface_mut() {
            // PhotoCraft previews the unfinished interpolation tail, then replaces it on
            // the next batch. A click and the newest pointer position are visible immediately.
            let (damage, next_tail) = renderer.composite_live(
                *tail,
                original,
                surface,
                selection.as_ref(),
                lock_transparency,
            );
            *tail = next_tail;
            changed = Some(damage);
        }
        if let Some(damage) = changed.filter(|d| !d.is_empty()) {
            if filter
                && let Some(id) = self.active_id()
                && let Err(error) =
                    photocraft_engine::commands::smart_cmds::refresh_after_filter_mask_edit(
                        &mut self.document,
                        id,
                    )
            {
                if let Some(before) = before {
                    self.document = before;
                }
                return Err(error.to_string());
            }
            if filter {
                // Feathering and filters can affect pixels outside the brush's damage rectangle.
                self.changed();
            } else {
                self.changed_region(damage);
            }
        }
        Ok(())
    }
    pub fn fill(&mut self, x: i32, y: i32, color: [f32; 4]) {
        if !self.paintable()
            || x < 0
            || y < 0
            || x >= self.size().width as i32
            || y >= self.size().height as i32
        {
            return;
        }
        let _=self.layer_command("paint.bucket",serde_json::json!({"x":x,"y":y,"color":color,"tolerance":0,"antiAlias":false,"target":if self.mask_target && self.filter_target{"filterMask"}else if self.mask_target{"mask"}else{"pixels"}}));
    }
    #[cfg(test)]
    pub fn composite(&self) -> Rgba8Image {
        composite(&self.document)
    }
    pub fn composite_region(&self, rect: Rect) -> Rgba8Image {
        composite_region(&self.document, rect)
    }
}

pub(super) fn sidecar(source: &Path) -> PathBuf {
    source.with_extension("pcraft")
}

pub(super) fn source_format(source: &Path) -> Result<ImageFormat, String> {
    let format = ImageFormat::from_path(source)
        .map_err(|_| "Редактирование доступно для PNG, JPEG, WebP, BMP, TGA и ICO.".to_owned())?;
    if matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::WebP
            | ImageFormat::Bmp
            | ImageFormat::Tga
            | ImageFormat::Ico
    ) {
        Ok(format)
    } else {
        Err("Этот формат пока доступен в режиме просмотра. Для редактирования используйте PNG, JPEG, WebP, BMP, TGA или ICO.".into())
    }
}

fn validate(doc: &Document) -> Result<(), String> {
    if doc.size.is_empty()
        || doc.size.width > 32768
        || doc.size.height > 32768
        || doc.size.area() > MAX_PIXELS
        || doc.layer_count() > MAX_LAYERS
    {
        return Err("Проект превышает ограничения редактора: 16 Мп, 4096 слоёв.".into());
    }

    Ok(())
}

pub(super) fn composite(doc: &Document) -> Rgba8Image {
    composite_region(doc, Rect::from_size(doc.size))
}

fn composite_region(doc: &Document, rect: Rect) -> Rgba8Image {
    let rect = rect.intersect(&Rect::from_size(doc.size));
    if rect.is_empty() {
        return Rgba8Image::new(0, 0);
    }
    photocraft_compose::render(doc, rect).to_rgba8()
}

pub(super) fn save(source: &Path, document: &Document) -> Result<(), String> {
    if !source.is_file() {
        return Err(
            "Исходное изображение перемещено или удалено. Восстановите его перед сохранением."
                .into(),
        );
    }
    validate(document)?;
    let format = source_format(source)?;
    let image = composite(document);
    let preview = image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
        .ok_or("Некорректные пиксели изображения")?;
    let mut encoded = Cursor::new(Vec::new());
    let dynamic = image::DynamicImage::ImageRgba8(preview);
    if matches!(format, ImageFormat::Jpeg) {
        // JPEG has no alpha: flatten transparent pixels onto white.
        let mut rgb = dynamic.to_rgb8();
        for (rgb, rgba) in rgb.pixels_mut().zip(image.pixels.chunks_exact(4)) {
            for (channel, &value) in rgb.0.iter_mut().zip(rgba) {
                *channel = ((u32::from(value) * u32::from(rgba[3])
                    + 255 * (255 - u32::from(rgba[3]))
                    + 127)
                    / 255) as u8;
            }
        }
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())?;
    } else {
        dynamic
            .write_to(&mut encoded, format)
            .map_err(|e| e.to_string())?;
    }
    let mut native_document = document.clone();
    native_document.name = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let native = photocraft_format::save_to_bytes(
        &native_document,
        &photocraft_format::SaveOptions {
            full_composite: Some(image),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    // Encode both outputs before replacing either file. A failed second replace
    // restores the previous sidecar; the source remains untouched on that failure.
    let native_path = sidecar(source);
    let previous = std::fs::read(&native_path).map_err(|e| e.to_string())?;
    photocraft_format::atomic_write(&native_path, &native).map_err(|e| e.to_string())?;
    if let Err(error) = photocraft_format::atomic_write(source, encoded.get_ref()) {
        if let Err(rollback) = photocraft_format::atomic_write(&native_path, &previous) {
            return Err(format!(
                "Не удалось обновить изображение: {error}. Слои сохранены в .pcraft; восстановление предыдущей версии: {rollback}"
            ));
        }
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "photo_document_tests.rs"]
mod tests;
