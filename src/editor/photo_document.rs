//! The embedded PhotoCraft document, commands and native persistence.
use image::{ImageDecoder, ImageFormat};
use photocraft_color::{ColorMode, PixelFormat, SampleType, blend};
use photocraft_doc::{Document, Layer, LayerContent};
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
const MAX_LAYERS: usize = 64;
const HISTORY: usize = 32;

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
}

#[derive(Clone)]
struct Snapshot {
    document: Document,
    active: usize,
    revision: u64,
}

pub(super) struct PhotoDocument {
    pub document: Document,
    pub active: usize,
    pub revision: u64,
    pub saved_revision: u64,
    next_revision: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
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
        let active = document.layers.len().saturating_sub(1);
        Ok(Self {
            document,
            active,
            revision: 1,
            saved_revision: 1,
            next_revision: 2,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    pub fn dirty(&self) -> bool {
        self.revision != self.saved_revision
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
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            document: self.document.clone(),
            active: self.active,
            revision: self.revision,
        }
    }
    pub fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn changed(&mut self) {
        self.revision = self.next_revision;
        self.next_revision += 1;
    }
    fn restore(&mut self, snapshot: Snapshot) {
        self.document = snapshot.document;
        self.active = snapshot.active;
        self.revision = snapshot.revision;
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
        if self.document.layers.len() >= MAX_LAYERS {
            return;
        }
        self.checkpoint();
        let position = (self.active + 1).min(self.document.layers.len());
        self.document.layers.insert(
            position,
            Layer::raster(
                format!("Слой {}", self.document.layers.len() + 1),
                PixelFormat::RGBA8,
            ),
        );
        self.active = position;
        self.changed();
    }
    pub fn duplicate_layer(&mut self) {
        if self.document.layers.len() >= MAX_LAYERS {
            return;
        }
        if let Some(layer) = self.document.layers.get(self.active) {
            let mut copy = layer.duplicate();
            copy.name = format!("{} — копия", copy.name);
            self.checkpoint();
            self.active += 1;
            self.document.layers.insert(self.active, copy);
            self.changed();
        }
    }
    pub fn delete_layer(&mut self) {
        if self.document.layers.len() <= 1 {
            return;
        }
        self.checkpoint();
        self.document.layers.remove(self.active);
        self.active = self.active.min(self.document.layers.len() - 1);
        self.changed();
    }
    pub fn reorder(&mut self, up: bool) {
        let next = if up {
            self.active.checked_add(1)
        } else {
            self.active.checked_sub(1)
        };
        if let Some(next) = next.filter(|&n| n < self.document.layers.len()) {
            self.checkpoint();
            self.document.layers.swap(self.active, next);
            self.active = next;
            self.changed();
        }
    }
    pub fn visibility(&mut self, index: usize) {
        if index >= self.document.layers.len() {
            return;
        }
        self.checkpoint();
        self.document.layers[index].visible ^= true;
        self.changed();
    }
    #[cfg(test)]
    pub fn opacity(&mut self, value: f32) {
        if !value.is_finite() {
            return;
        }
        self.checkpoint();
        self.document.layers[self.active].opacity = value.clamp(0.0, 1.0);
        self.changed();
    }
    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || self.document.layers[self.active].name == name {
            return;
        }
        self.checkpoint();
        self.document.layers[self.active].name = name.chars().take(100).collect();
        self.changed();
    }
    pub fn paintable(&self) -> bool {
        self.document
            .layers
            .get(self.active)
            .is_some_and(|l| l.visible && !l.locks.all && !l.locks.pixels && l.surface().is_some())
    }
    #[cfg(test)]
    pub fn paint(&mut self, original: &Surface, brush: &BrushSettings, points: &[StrokePoint]) {
        let bounds = Rect::from_size(self.size());
        if let Some(surface) = self
            .document
            .layers
            .get_mut(self.active)
            .and_then(Layer::surface_mut)
        {
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
    pub fn move_layer(&mut self, original: &Surface, dx: i32, dy: i32) {
        let bounds = Rect::from_size(self.size());
        if let Some(surface) = self
            .document
            .layers
            .get_mut(self.active)
            .and_then(Layer::surface_mut)
        {
            *surface = original.translated(dx, dy, bounds);
            self.changed();
        }
    }
    pub fn paint_live(
        &mut self,
        original: &Surface,
        renderer: &mut photocraft_paint::StrokeRenderer,
        point: Option<StrokePoint>,
        finish: bool,
    ) {
        if let Some(point) = point {
            renderer.push(&[point]);
        }
        if finish {
            renderer.finish();
        }
        if let Some(surface) = self
            .document
            .layers
            .get_mut(self.active)
            .and_then(Layer::surface_mut)
        {
            renderer.composite(original, surface, None, false, false);
            self.changed();
        }
    }
    pub fn fill(&mut self, x: i32, y: i32, color: [f32; 4]) {
        if !self.paintable() {
            return;
        }
        let size = self.size();
        if x < 0 || y < 0 || x >= size.width as i32 || y >= size.height as i32 {
            return;
        }
        let Some(surface) = self.document.layers[self.active].surface() else {
            return;
        };
        let old = surface.rgba(x, y);
        if old
            .iter()
            .zip(color)
            .all(|(a, b)| (*a - b).abs() < 1.0 / 255.0)
        {
            return;
        }
        self.checkpoint();
        let Some(surface) = self.document.layers[self.active].surface_mut() else {
            return;
        };
        let mut visited = vec![false; size.area() as usize];
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            let index = y as usize * size.width as usize + x as usize;
            if visited[index] {
                continue;
            }
            visited[index] = true;
            if surface
                .rgba(x, y)
                .iter()
                .zip(old)
                .any(|(a, b)| (*a - b).abs() > 1.0 / 255.0)
            {
                continue;
            }
            surface.write_pixel(x, y, &color);
            if x > 0 {
                stack.push((x - 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if x + 1 < size.width as i32 {
                stack.push((x + 1, y));
            }
            if y + 1 < size.height as i32 {
                stack.push((x, y + 1));
            }
        }
        self.changed();
    }
    pub fn composite(&self) -> Rgba8Image {
        composite(&self.document)
    }
}

pub(super) fn sidecar(source: &Path) -> PathBuf {
    source.with_extension("pcraft")
}

fn source_format(source: &Path) -> Result<ImageFormat, String> {
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
        || doc.layers.is_empty()
        || doc.layers.len() > MAX_LAYERS
    {
        return Err("Проект превышает ограничения базового редактора: 16 Мп, 64 слоя.".into());
    }
    if doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::U8
        || doc.layers.iter().any(|l| {
            !matches!(l.content, LayerContent::Raster(_))
                || l.mask.is_some()
                || l.vector_mask.is_some()
                || !l.effects.items.is_empty()
                || l.clipped
                || l.frame.is_some()
        })
    {
        return Err("Этот проект использует возможности полной версии PhotoCraft. Базовый редактор поддерживает растровые RGB-слои без масок и эффектов.".into());
    }
    Ok(())
}

pub(super) fn composite(doc: &Document) -> Rgba8Image {
    let mut image = Rgba8Image::new(doc.size.width, doc.size.height);
    // Gather a row per layer instead of locking a raster tile for every pixel.
    let width = doc.size.width as usize;
    let mut accum = vec![[0.0; 4]; width];
    let mut source = vec![[0.0; 4]; width];
    for (y, row) in image.pixels.chunks_exact_mut(width * 4).enumerate() {
        accum.fill([0.0; 4]);
        for layer in doc.layers.iter().filter(|l| l.visible) {
            if let Some(surface) = layer.surface() {
                surface.read_rgba_into(
                    Rect::new(0, y as i32, width as i32, y as i32 + 1),
                    &mut source,
                );
                for (dst, src) in accum.iter_mut().zip(&source) {
                    *dst = blend::composite(
                        layer.blend,
                        *dst,
                        *src,
                        layer.opacity * layer.fill_opacity,
                    );
                }
            }
        }
        for (pixel, rgba) in row.chunks_exact_mut(4).zip(&accum) {
            for (channel, value) in pixel.iter_mut().zip(rgba) {
                *channel = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    image
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
    let native = photocraft_format::save_to_bytes(
        document,
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
