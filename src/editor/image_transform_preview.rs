//! Cache the stationary/moving raster planes once; a drag only changes a homography.
use super::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use photocraft_doc::{BlendMode, Document, Layer, LayerContent, LayerId};
use photocraft_raster::Rgba8Image;
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(1);

pub(in crate::editor) type PrepareLayer =
    Box<dyn FnOnce(&mut photocraft_raster::Surface) + Send + Sync>;

pub(super) struct Planes {
    pub under: Rgba8Image,
    pub moving: Rgba8Image,
    pub above: Rgba8Image,
    selection: Option<image::GrayImage>,
}
#[derive(Clone)]
struct Source {
    doc: Document,
    moving: Vec<LayerId>,
    whole: Vec<LayerId>,
    frame: [f32; 4],
}
pub(in crate::editor) struct Live {
    pub serial: u64,
    pub revision: u64,
    pub direct: bool,
    opacity: f32,
    ready: Option<Planes>,
    source: Source,
    task: Option<Task<Result<Planes, String>>>,
    failed: bool,
    captured: bool,
    commit: Option<Commit>,
}
type Output = Result<(Document, Rgba8Image, bool), String>;
struct Commit {
    quad: [Vec2; 4],
    nearest: bool,
    revision: u64,
    task: Task<Output>,
}

// Flatten only associative normal-over stacks. Unsupported blends/masks/effects
// keep the native path rather than showing an inaccurate preview.
fn flatten(
    layers: &[Layer],
    selected: &[LayerId],
    inherited: bool,
    visible: bool,
    flat: &mut Vec<Layer>,
    moving: &mut Vec<LayerId>,
    whole: &mut Vec<LayerId>,
) -> Option<()> {
    for layer in layers {
        if layer.mask.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.frame.is_some()
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || layer.video.is_some()
        {
            return None;
        }
        let chosen = inherited || selected.contains(&layer.id);
        match &layer.content {
            LayerContent::Group(group)
                if matches!(layer.blend, BlendMode::Normal | BlendMode::PassThrough)
                    && layer.opacity == 1.0
                    && layer.fill_opacity == 1.0
                    && group.artboard.is_none() =>
            {
                flatten(
                    &group.children,
                    selected,
                    chosen,
                    visible && layer.visible,
                    flat,
                    moving,
                    whole,
                )?;
            }
            LayerContent::Raster(_) if layer.blend == BlendMode::Normal => {
                let mut copy = layer.clone();
                copy.visible &= visible;
                if chosen {
                    moving.push(layer.id);
                    if inherited {
                        whole.push(layer.id);
                    }
                }
                flat.push(copy);
            }
            _ => return None,
        }
    }
    Some(())
}
impl Live {
    pub fn new(
        doc: &crate::editor::photo_document::PhotoDocument,
        frame: [f32; 4],
    ) -> Option<Self> {
        let selected = if doc.selected.is_empty() {
            doc.active_id().into_iter().collect()
        } else {
            doc.selected.clone()
        };
        let targets =
            photocraft_engine::commands::transform_cmds::box_targets(&doc.document, &selected);
        if targets.iter().any(|id| !selected.contains(id)) {
            return None;
        }
        Self::for_layers(doc, frame, &selected, None)
    }

    pub(in crate::editor) fn for_model(
        doc: &PhotoDocument,
        frame: [f32; 4],
        layer: LayerId,
        extend: Option<PrepareLayer>,
    ) -> Option<Self> {
        if doc.document.selection.is_some() {
            return None;
        }
        Self::for_layers(doc, frame, &[layer], extend)
    }

    fn for_layers(
        doc: &PhotoDocument,
        frame: [f32; 4],
        selected: &[LayerId],
        extend: Option<PrepareLayer>,
    ) -> Option<Self> {
        if doc.mask_target
            || doc.mask_view.is_some()
            || doc.document.mode != photocraft_doc::ColorMode::Rgb
            || doc.document.depth != photocraft_doc::SampleType::U8
        {
            return None;
        }
        crate::editor::photo_document::PhotoDocument::check_transform(
            frame_quad(frame).map(|p| p.to_array()),
        )
        .ok()?;
        let mut source = Source {
            doc: doc.document.clone(),
            moving: Vec::new(),
            whole: Vec::new(),
            frame,
        };
        let mut flat = Vec::new();
        flatten(
            &doc.document.layers,
            selected,
            false,
            true,
            &mut flat,
            &mut source.moving,
            &mut source.whole,
        )?;
        let indices = flat
            .iter()
            .enumerate()
            .filter(|(_, l)| source.moving.contains(&l.id))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if indices.is_empty() || indices.windows(2).any(|p| p[1] != p[0] + 1) {
            return None;
        }
        if source.moving.len() > 1
            && source.doc.selection.is_some()
            && source.whole.len() != source.moving.len()
        {
            return None;
        }
        // A selected fragment rejoins its original layer before that layer's
        // opacity is applied. Baking opacity separately into both pieces would
        // make overlapping translucent fragments too opaque in the live view.
        if source.doc.selection.is_some()
            && flat.iter().any(|l| {
                source.moving.contains(&l.id)
                    && !source.whole.contains(&l.id)
                    && (l.opacity != 1.0 || l.fill_opacity != 1.0)
            })
        {
            return None;
        }
        let first = &flat[indices[0]];
        let opacity = if first.visible {
            first.opacity * first.fill_opacity
        } else {
            0.0
        };
        source.doc.layers = flat;
        let direct = extend.is_none()
            && source.doc.layers.len() == 1
            && source.doc.selection.is_none()
            && frame[0] >= 0.0
            && frame[1] >= 0.0
            && frame[2] <= doc.size().width as f32
            && frame[3] <= doc.size().height as f32;
        let mut live = Self {
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
            revision: doc.revision,
            direct,
            opacity,
            ready: None,
            source,
            task: None,
            failed: false,
            captured: false,
            commit: None,
        };
        if !direct {
            live.start(extend);
        }
        Some(live)
    }
    fn start(&mut self, extend: Option<PrepareLayer>) {
        let mut source = self.source.clone();
        self.task = Some(AsyncComputeTaskPool::get().spawn(async move {
            if let Some(extend) = extend {
                let layer = source
                    .doc
                    .layer_mut(source.moving[0])
                    .and_then(Layer::surface_mut)
                    .ok_or("Missing model layer")?;
                extend(layer);
            }
            prepare(source)
        }));
    }
    pub fn poll(&mut self, revision: u64) {
        // A command can change the document before the UI captures the original
        // canvas texture. In that case prepare from the retained source instead.
        if self.direct
            && !self.captured
            && revision != self.revision
            && self.task.is_none()
            && self.ready.is_none()
            && !self.failed
        {
            self.start(None);
        }
        if let Some(result) = self.task.as_mut().and_then(|t| block_on(poll_once(t))) {
            self.task = None;
            match result {
                Ok(p) => self.ready = Some(p),
                Err(_) => self.failed = true,
            }
        }
    }
    pub(in crate::editor) fn ready_for_display(&self) -> bool {
        self.captured || self.ready.is_some()
    }
    pub fn usable(&self) -> bool {
        !self.failed
    }
    pub fn needs_job(frame: [f32; 4], quad: [Vec2; 4]) -> bool {
        let lo = quad.into_iter().fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let hi = quad
            .into_iter()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
        (frame[2] - frame[0]) * (frame[3] - frame[1]) > 262_144.0
            || (hi.x - lo.x) * (hi.y - lo.y) > 262_144.0
    }
    pub fn commit(
        &mut self,
        doc: &PhotoDocument,
        original: &LayerMove,
        frame: [f32; 4],
        quad: [Vec2; 4],
        nearest: bool,
    ) {
        if self.commit.is_none() {
            let work = doc.transform_job(original, frame, quad.map(|p| p.to_array()), nearest);
            self.commit = Some(Commit {
                quad,
                nearest,
                revision: doc.revision,
                task: AsyncComputeTaskPool::get().spawn(async move { work() }),
            });
        }
    }
    pub fn poll_commit(&mut self) -> Option<([Vec2; 4], bool, u64, Output)> {
        let result = self
            .commit
            .as_mut()
            .and_then(|job| block_on(poll_once(&mut job.task)))?;
        let job = self.commit.take().unwrap();
        Some((job.quad, job.nearest, job.revision, result))
    }
    pub fn cancel_commit(&mut self) {
        self.commit = None;
    }
    pub fn disable(&mut self) {
        self.failed = true;
        self.cancel_commit();
    }
}

fn bitmap(doc: &Document, rect: photocraft_geom::Rect) -> Rgba8Image {
    if !doc.layers.iter().any(|l| l.visible) {
        return Rgba8Image::new(1, 1);
    }
    let side = rect.width().max(rect.height());
    if side > 4096 {
        photocraft_compose::render_reduced_rect(
            doc,
            rect,
            (rect.width() * 4096 / side).max(1),
            (rect.height() * 4096 / side).max(1),
        )
        .to_rgba8()
    } else {
        photocraft_compose::render(doc, rect).to_rgba8()
    }
}
fn prepare(source: Source) -> Result<Planes, String> {
    let mut under = source.doc.clone();
    let mut moving = source.doc.clone();
    let mut above = source.doc.clone();
    let first = source
        .doc
        .layers
        .iter()
        .position(|l| source.moving.contains(&l.id))
        .unwrap();
    let last = source
        .doc
        .layers
        .iter()
        .rposition(|l| source.moving.contains(&l.id))
        .unwrap();
    under.layers.truncate(last + 1);
    above.layers.drain(..last + 1);
    moving.layers = source.doc.layers[first..=last].to_vec();
    for layer in &mut under.layers {
        if !source.moving.contains(&layer.id) {
            continue;
        }
        if let Some(selection) = source
            .doc
            .selection
            .as_ref()
            .filter(|_| !source.whole.contains(&layer.id))
        {
            let (part, rest) = photocraft_engine::commands::transform_cmds::split_selected(
                layer.surface().unwrap(),
                selection,
            )
            .map_err(|e| e.to_string())?;
            layer.content = LayerContent::Raster(rest);
            moving
                .layers
                .iter_mut()
                .find(|l| l.id == layer.id)
                .unwrap()
                .content = LayerContent::Raster(part);
        } else {
            layer.visible = false;
        }
    }
    let f = source.frame;
    let rect = photocraft_geom::Rect::new(f[0] as i32, f[1] as i32, f[2] as i32, f[3] as i32);
    Ok(Planes {
        under: bitmap(&under, under.bounds()),
        moving: bitmap(&moving, rect),
        above: bitmap(&above, above.bounds()),
        selection: source.doc.selection.as_ref().and_then(|s| {
            let data = s
                .read_region(source.doc.bounds())
                .into_iter()
                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            let mask =
                image::GrayImage::from_raw(source.doc.size.width, source.doc.size.height, data)?;
            Some(if mask.width().max(mask.height()) > 4096 {
                image::DynamicImage::ImageLuma8(mask)
                    .thumbnail(4096, 4096)
                    .into_luma8()
            } else {
                mask
            })
        }),
    })
}

/// Map document UVs back to the unmodified source rectangle. Solving in f64
/// avoids jitter at large zoom; only the final three rows are GPU uniforms.
pub(super) fn inverse(quad: [Vec2; 4], size: Vec2) -> Option<[Vec4; 3]> {
    if !size.is_finite() || size.min_element() <= 0.0 || quad.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let mut a = [[0.0f64; 9]; 8];
    for (i, (p, uv)) in quad
        .into_iter()
        .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        .enumerate()
    {
        let x = (p.x / size.x) as f64;
        let y = (p.y / size.y) as f64;
        let [u, v] = uv;
        a[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for col in 0..8 {
        let pivot = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        let d = a[col][col];
        for v in &mut a[col][col..] {
            *v /= d;
        }
        for row in 0..8 {
            if row != col {
                let factor = a[row][col];
                for k in col..9 {
                    a[row][k] -= factor * a[col][k];
                }
            }
        }
    }
    let x = a.map(|r| r[8] as f32);
    Some([
        Vec4::new(x[0], x[1], x[2], 0.0),
        Vec4::new(x[3], x[4], x[5], 0.0),
        Vec4::new(x[6], x[7], 1.0, 0.0),
    ])
}

#[cfg(test)]
#[path = "image_transform_preview_tests.rs"]
mod tests;

pub(in crate::editor) struct Placement<'a> {
    pub live: &'a mut Live,
    pub frame: [f32; 4],
    pub quad: [Vec2; 4],
    pub deferred: bool,
    pub camera: Option<Handle<Image>>,
}
impl Transform {
    pub(in crate::editor) fn placement(&mut self) -> Option<Placement<'_>> {
        Some(Placement {
            live: self.live.as_mut()?,
            frame: self.frame,
            quad: self.quad,
            deferred: self.deferred,
            camera: None,
        })
    }
}

#[derive(Default)]
pub(in crate::editor) struct Textures {
    serial: u64,
    handles: Vec<Handle<Image>>,
    full_source: bool,
    selection: Option<Handle<Image>>,
}
impl Textures {
    pub(in crate::editor) fn sync(
        &mut self,
        transform: Option<Placement>,
        source: Option<&Handle<Image>>,
        dimensions: Option<UVec2>,
        revision: Option<u64>,
        images: &mut Assets<Image>,
        material: &mut super::super::super::CanvasMaterial,
        nearest: bool,
    ) {
        material.transform_style = Vec4::ZERO;
        let Some(t) = transform else {
            self.clear(images, material);
            return;
        };
        let live = t.live;
        if self.serial != live.serial {
            self.clear(images, material);
            if live.direct
                && revision == Some(live.revision)
                && let Some(source) = source.and_then(|h| images.get(h)).cloned()
            {
                self.handles = vec![
                    images.add(image_previews::texture_image(DecodedImage {
                        size: UVec2::ONE,
                        original: UVec2::ONE,
                        rgba: vec![0; 4],
                    })),
                    images.add(source),
                ];
                self.full_source = true;
            } else if let Some(planes) = live.ready.take() {
                if let Some(mask) = planes.selection {
                    let mut texture = Image::new(
                        bevy::render::render_resource::Extent3d {
                            width: mask.width(),
                            height: mask.height(),
                            depth_or_array_layers: 1,
                        },
                        bevy::render::render_resource::TextureDimension::D2,
                        mask.into_raw(),
                        bevy::render::render_resource::TextureFormat::R8Unorm,
                        bevy::asset::RenderAssetUsages::MAIN_WORLD
                            | bevy::asset::RenderAssetUsages::RENDER_WORLD,
                    );
                    texture.sampler = bevy::image::ImageSampler::nearest();
                    self.selection = Some(images.add(texture));
                }
                self.handles = [planes.under, planes.moving, planes.above]
                    .into_iter()
                    .map(|p| {
                        let size = UVec2::new(p.width, p.height);
                        images.add(image_previews::texture_image(DecodedImage {
                            size,
                            original: size,
                            rgba: p.pixels,
                        }))
                    })
                    .collect();
            } else {
                return;
            }
            self.serial = live.serial;
            live.captured = true;
        }
        if !t.deferred {
            return;
        }
        let Some(size) = dimensions.map(|s| s.as_vec2()) else {
            return;
        };
        let Some(rows) = inverse(t.quad, size) else {
            return;
        };
        material.transform_under = Some(self.handles[0].clone());
        material.transform_selection = self.selection.clone();
        let camera = t.camera.is_some();
        material.transform_moving = t.camera.or_else(|| Some(self.handles[1].clone()));
        material.transform_above = Some(self.handles[if self.full_source { 0 } else { 2 }].clone());
        material.transform_rows = rows;
        material.transform_source = if self.full_source && !camera {
            Vec4::new(
                t.frame[0] / size.x,
                t.frame[1] / size.y,
                (t.frame[2] - t.frame[0]) / size.x,
                (t.frame[3] - t.frame[1]) / size.y,
            )
        } else {
            Vec4::new(0.0, 0.0, 1.0, 1.0)
        };
        material.transform_style = Vec4::new(
            1.0,
            if nearest { 1.0 } else { 0.0 },
            if camera { live.opacity } else { 1.0 },
            0.0,
        );
        material.transform_frame = Vec4::new(
            t.frame[0] / size.x,
            t.frame[1] / size.y,
            (t.frame[2] - t.frame[0]) / size.x,
            (t.frame[3] - t.frame[1]) / size.y,
        );
    }
    fn clear(
        &mut self,
        images: &mut Assets<Image>,
        material: &mut super::super::super::CanvasMaterial,
    ) {
        material.transform_under = None;
        material.transform_moving = None;
        material.transform_above = None;
        material.transform_selection = None;
        if let Some(h) = self.selection.take() {
            images.remove(h.id());
        }
        for h in self.handles.drain(..) {
            images.remove(h.id());
        }
        self.serial = 0;
        self.full_source = false;
    }
}
