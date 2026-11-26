//! Lossless snapshots of paint stacks. Shared pixel planes are compressed once,
//! off the editor thread, while layer metadata remains immediately available.
use super::{Layer, Mask, Stack};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

type Pixels = Arc<Vec<u8>>;

const MIN_PACKED_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Label {
    #[default]
    LayerProperties,
    AddLayer,
    AddMaterial,
    Duplicate,
    Remove,
    Clear,
    Group,
    Move,
    Rename,
    Mask,
    Filter,
    Graph,
    FillProperties,
    Stroke,
    Erase,
    Fill,
    Path,
    MaskStroke,
    SurfaceSelection,
}

impl Label {
    pub(super) fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::LayerProperties => ("layer_properties", "Настройки слоя"),
            Self::AddLayer => ("add_layer", "Добавление слоя"),
            Self::AddMaterial => ("add_material", "Добавление материала"),
            Self::Duplicate => ("duplicate_layer", "Копирование слоя"),
            Self::Remove => ("remove_layer", "Удаление слоя"),
            Self::Clear => ("clear_layer", "Очистка слоя"),
            Self::Group => ("group_layers", "Группировка слоёв"),
            Self::Move => ("move_layer", "Перемещение слоя"),
            Self::Rename => ("rename_layer", "Переименование слоя"),
            Self::Mask => ("mask", "Изменение маски"),
            Self::Filter => ("mask_filter", "Изменение фильтра маски"),
            Self::Graph => ("material_graph", "Изменение графа"),
            Self::FillProperties => ("fill_properties", "Настройки заливки"),
            Self::Stroke => ("stroke", "Мазок кисти"),
            Self::Erase => ("erase", "Мазок ластиком"),
            Self::Fill => ("fill", "Заливка области"),
            Self::Path => ("path", "Мазок по контуру"),
            Self::MaskStroke => ("mask_stroke", "Мазок по маске"),
            Self::SurfaceSelection => ("surface_selection", "Маска из выделения поверхности"),
        }
    }
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub order: u64,
    pub label: Label,
    stack: Stack,
    pixels: Vec<LayerPixels>,
}

#[cfg(test)]
impl Snapshot {
    pub fn layer_count(&self) -> usize {
        self.stack.layers.len()
    }
}

#[derive(Clone)]
struct LayerPixels {
    channels: [Option<Arc<Plane>>; crate::editor::model_paint_data::CHANNEL_COUNT],
    mask: Option<(Arc<Mask>, Arc<Plane>)>,
}

#[derive(Default)]
pub(super) struct Cache {
    planes: HashMap<usize, PlaneRef>,
    captures: u8,
    bytes: Arc<AtomicUsize>,
}

struct PlaneRef {
    pixels: Weak<Vec<u8>>,
    plane: Weak<Plane>,
}

impl Cache {
    pub fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn capture(&mut self, mut stack: Stack) -> Snapshot {
        if self.captures.is_multiple_of(32) {
            self.planes.retain(|_, entry| {
                entry.pixels.strong_count() != 0 && entry.plane.strong_count() != 0
            });
        }
        self.captures = self.captures.wrapping_add(1);
        let pixels = stack
            .layers
            .iter_mut()
            .map(|layer| {
                let channels = std::mem::take(&mut layer.channels)
                    .map(|pixels| pixels.map(|pixels| self.plane(pixels)));
                let mask = layer.mask.take().map(|mask| {
                    let mut settings = (*mask).clone();
                    let pixels = self.plane(std::mem::take(&mut settings.pixels));
                    (Arc::new(settings), pixels)
                });
                LayerPixels { channels, mask }
            })
            .collect();
        Snapshot { order: 0, label: Label::default(), stack, pixels }
    }

    pub fn restore(&mut self, snapshot: Snapshot) -> Stack {
        let Snapshot { mut stack, pixels, .. } = snapshot;
        for (layer, pixels) in stack.layers.iter_mut().zip(pixels) {
            self.restore_layer(layer, pixels);
        }
        stack
    }

    fn restore_layer(&mut self, layer: &mut Layer, pixels: LayerPixels) {
        layer.channels = pixels
            .channels
            .map(|plane| plane.map(|plane| self.raw(&plane)));
        layer.mask = pixels
            .mask
            .map(|(settings, plane)| Arc::new(settings.with_pixels(self.raw(&plane))));
    }

    fn plane(&mut self, pixels: Pixels) -> Arc<Plane> {
        let id = Arc::as_ptr(&pixels) as usize;
        if let Some(plane) = self.planes.get(&id).and_then(|entry| entry.plane.upgrade()) {
            return plane;
        }
        let byte_len = pixels.len();
        // Hold a weak reference to prevent reusing this address as a key while
        // the cache entry exists, even after the pixel allocation is freed.
        let source = Arc::downgrade(&pixels);
        let plane = Arc::new(Plane::new(pixels, self.bytes.clone()));
        self.planes.insert(
            id,
            PlaneRef {
                pixels: source,
                plane: Arc::downgrade(&plane),
            },
        );
        if byte_len >= MIN_PACKED_BYTES
            && let Some(worker) = worker()
        {
            let _ = worker.send(Arc::downgrade(&plane));
        }
        plane
    }

    fn raw(&mut self, plane: &Arc<Plane>) -> Pixels {
        let pixels = plane.raw();
        self.planes.insert(
            Arc::as_ptr(&pixels) as usize,
            PlaneRef {
                pixels: Arc::downgrade(&pixels),
                plane: Arc::downgrade(plane),
            },
        );
        pixels
    }
}

struct Plane {
    storage: Mutex<Storage>,
    total_bytes: Arc<AtomicUsize>,
}

enum Storage {
    Raw(Pixels),
    Packed {
        bytes: Box<[u8]>,
        len: usize,
        raw: Weak<Vec<u8>>,
    },
}

impl Drop for Plane {
    fn drop(&mut self) {
        let bytes = match self.storage.get_mut().unwrap() {
            Storage::Raw(raw) => raw.len(),
            Storage::Packed { bytes, .. } => bytes.len(),
        };
        self.total_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }
}

impl Plane {
    fn new(pixels: Pixels, total_bytes: Arc<AtomicUsize>) -> Self {
        total_bytes.fetch_add(pixels.len(), Ordering::Relaxed);
        Self {
            storage: Mutex::new(Storage::Raw(pixels)),
            total_bytes,
        }
    }

    fn pack(&self) {
        let raw = match &*self.storage.lock().unwrap() {
            Storage::Raw(raw) => raw.clone(),
            Storage::Packed { .. } => return,
        };
        let bytes = lz4_flex::block::compress(&raw);
        if bytes.len() < raw.len() {
            let bytes = bytes.into_boxed_slice();
            let saved_bytes = raw.len() - bytes.len();
            let mut storage = self.storage.lock().unwrap();
            *storage = Storage::Packed {
                bytes,
                len: raw.len(),
                raw: Arc::downgrade(&raw),
            };
            drop(raw);
            self.total_bytes.fetch_sub(saved_bytes, Ordering::Relaxed);
        }
    }

    fn raw(&self) -> Pixels {
        match &mut *self.storage.lock().unwrap() {
            Storage::Raw(raw) => raw.clone(),
            Storage::Packed { bytes, len, raw } => raw.upgrade().unwrap_or_else(|| {
                // The private block and its length are produced together by pack().
                let pixels = Arc::new(
                    lz4_flex::block::decompress(bytes, *len).expect("valid paint history block"),
                );
                *raw = Arc::downgrade(&pixels);
                pixels
            }),
        }
    }
}

fn worker() -> Option<&'static mpsc::Sender<Weak<Plane>>> {
    static WORKER: OnceLock<Option<mpsc::Sender<Weak<Plane>>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (send, receive) = mpsc::channel::<Weak<Plane>>();
            std::thread::Builder::new()
                .name("paint-history".into())
                .spawn(move || {
                    for plane in receive {
                        if let Some(plane) = plane.upgrade() {
                            plane.pack();
                        }
                    }
                })
                .ok()?;
            Some(send)
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_paint_data::LayerKind;
    use std::time::{Duration, Instant};

    fn packed_len(plane: &Plane) -> usize {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Storage::Packed { bytes, .. } = &*plane.storage.lock().unwrap() {
                return bytes.len();
            }
            assert!(
                Instant::now() < deadline,
                "paint history worker did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn paint_history_restores_shared_channels_and_mask_metadata() {
        let mut cache = Cache::default();
        let len = 128 * 128 * 4;
        let mut raw = vec![0; len];
        raw[240..244].copy_from_slice(&[17, 24, 31, 255]);
        let raw = Arc::new(raw);
        let mut layer = Layer::new("Мазок".into(), LayerKind::Paint);
        layer.channels[0] = Some(raw.clone());
        layer.channels[1] = Some(raw.clone());
        let mut mask = Mask::new(raw.clone(), Vec::new());
        mask.set_geometry_coverage(vec![51; len / 4]);
        mask.use_affine();
        layer.mask = Some(Arc::new(mask));
        let stack = Stack {
            layers: vec![layer],
            selected: 0,
        };
        let first = cache.capture(stack.clone());
        let other = cache.capture(stack.clone());
        let color = first.pixels[0].channels[0].as_ref().unwrap();
        let roughness = other.pixels[0].channels[1].as_ref().unwrap();
        assert!(Arc::ptr_eq(color, roughness));
        let color_bytes = packed_len(color);
        let mask_bytes = packed_len(&first.pixels[0].mask.as_ref().unwrap().1);
        assert!(color_bytes < len / 8);
        assert!(mask_bytes < len / 8);
        assert_eq!(cache.bytes(), color_bytes + mask_bytes);
        let restored = cache.restore(first);
        assert_eq!(restored, stack);
        assert!(Arc::ptr_eq(
            restored.layers[0].channels[0].as_ref().unwrap(),
            &raw
        ));
        drop(restored);
        drop(stack);
        let original = Arc::downgrade(&raw);
        drop(raw);
        assert!(
            original.upgrade().is_none(),
            "history must release the uncompressed plane"
        );
        let restored = cache.restore(other);
        let layer = &restored.layers[0];
        assert_eq!(
            &layer.channels[0].as_ref().unwrap()[240..244],
            &[17, 24, 31, 255]
        );
        assert!(Arc::ptr_eq(
            layer.channels[0].as_ref().unwrap(),
            layer.channels[1].as_ref().unwrap()
        ));
        let mask = layer.mask.as_ref().unwrap();
        assert!(mask.affine);
        assert_eq!(mask.geometry_coverage().unwrap(), &vec![51; len / 4]);
        assert_eq!(&mask.pixels[240..244], &[17, 0, 0, 255]);
    }

    #[test]
    fn paint_history_keeps_pixels_raw_when_packing_would_grow_them() {
        let mut seed = 1729u32;
        let pixels: Pixels = Arc::new(
            (0..MIN_PACKED_BYTES)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect(),
        );
        let bytes = Arc::new(AtomicUsize::new(0));
        let plane = Plane::new(pixels.clone(), bytes.clone());
        plane.pack();
        assert!(Arc::ptr_eq(&plane.raw(), &pixels));
        assert!(matches!(*plane.storage.lock().unwrap(), Storage::Raw(_)));
        assert_eq!(bytes.load(Ordering::Relaxed), pixels.len());
        drop(plane);
        assert_eq!(bytes.load(Ordering::Relaxed), 0);
    }

    #[test]
    #[ignore = "compares the raw and packed history sizes of two 4K strokes"]
    fn paint_history_4k_releases_original_planes() {
        let mut cache = Cache::default();
        let len = 4096 * 4096 * 4;
        let mut snapshots = Vec::new();
        for offset in [1024, 2048] {
            let pixels = Arc::new({
                let mut pixels = vec![0; len];
                pixels[offset..offset + 4].copy_from_slice(&[211, 99, 68, 255]);
                pixels
            });
            let original = Arc::downgrade(&pixels);
            let mut layer = Layer::new("Краска".into(), LayerKind::Paint);
            layer.channels[0] = Some(pixels);
            let snapshot = cache.capture(Stack {
                layers: vec![layer],
                selected: 0,
            });
            snapshots.push((snapshot, original, offset));
        }
        let packed: usize = snapshots
            .iter()
            .map(|(snapshot, _, _)| packed_len(snapshot.pixels[0].channels[0].as_ref().unwrap()))
            .sum();
        println!(
            "4K paint history: {} raw bytes → {packed} packed bytes",
            len * 2
        );
        assert!(packed < len / 4);
        for (snapshot, original, offset) in snapshots {
            assert!(original.upgrade().is_none());
            let restored = cache.restore(snapshot);
            let pixels = restored.layers[0].channels[0].as_ref().unwrap();
            assert_eq!(pixels.len(), len);
            assert_eq!(&pixels[offset..offset + 4], &[211, 99, 68, 255]);
        }
    }
}
