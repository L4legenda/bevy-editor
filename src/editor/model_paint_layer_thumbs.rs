//! Downsample committed layer content for the layer list without GPU readbacks.
use super::*;

const SIZE: usize = 64;

#[derive(Clone)]
pub(super) enum Source {
    Color(Arc<Vec<u8>>),
    Mask(Arc<Mask>),
}

impl Source {
    pub(super) fn content(layer: &Layer) -> Option<Self> {
        [0, 5, 1, 2, 3, 4, 6, 7]
            .into_iter()
            .find_map(|channel| layer.channel(channel).cloned())
            .map(Self::Color)
    }

    fn key(&self) -> Key {
        match self {
            Self::Color(pixels) => Key::Color(Arc::as_ptr(pixels) as usize),
            Self::Mask(mask) => Key::Mask(Arc::as_ptr(mask) as usize),
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Key {
    Color(usize),
    Mask(usize),
}

struct Entry {
    source: Source,
    image: Handle<Image>,
    ready: bool,
}

pub(super) struct Cache {
    entries: HashMap<Key, Entry>,
    requested: Vec<Key>,
    pending: Option<(Source, Task<Vec<u8>>)>,
    pub(super) checker: Handle<Image>,
}

impl Cache {
    pub(super) fn new(images: &mut Assets<Image>) -> Self {
        let checker = (0..SIZE * SIZE)
            .flat_map(|i| {
                let value = if (i % SIZE / 8 + i / SIZE / 8) % 2 == 0 {
                    39
                } else {
                    59
                };
                [value, value, value, 255]
            })
            .collect();
        Self {
            checker: images.add(image(checker)),
            entries: HashMap::new(),
            requested: Vec::new(),
            pending: None,
        }
    }

    pub(super) fn begin_list(&mut self) {
        self.requested.clear();
    }

    pub(super) fn thumbnail(&mut self, source: Source, images: &Assets<Image>) -> Handle<Image> {
        let key = source.key();
        self.requested.push(key);
        self.entries
            .entry(key)
            .or_insert_with(|| Entry {
                source,
                image: images.reserve_handle(),
                ready: false,
            })
            .image
            .clone()
    }

    pub(super) fn end_list(&mut self) {
        let used: std::collections::HashSet<_> = self.requested.iter().copied().collect();
        self.entries.retain(|key, _| used.contains(key));
        if self
            .pending
            .as_ref()
            .is_some_and(|(source, _)| !used.contains(&source.key()))
        {
            self.pending = None;
        }
    }

    pub(super) fn update(&mut self, images: &mut Assets<Image>, idle: bool) {
        if let Some((source, task)) = &mut self.pending
            && let Some(pixels) = block_on(poll_once(task))
        {
            if let Some(entry) = self.entries.get_mut(&source.key()) {
                entry.ready = images.insert(entry.image.id(), image(pixels)).is_ok();
            }
            self.pending = None;
        }
        if idle
            && self.pending.is_none()
            && let Some(entry) = self
                .requested
                .iter()
                .filter_map(|key| self.entries.get(key))
                .find(|entry| !entry.ready)
        {
            let source = entry.source.clone();
            let work = source.clone();
            self.pending = Some((
                source,
                AsyncComputeTaskPool::get().spawn(async move { render(&work) }),
            ));
        }
    }
}

fn image(pixels: Vec<u8>) -> Image {
    Image::from_dynamic(
        image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(SIZE as u32, SIZE as u32, pixels).unwrap(),
        ),
        true,
        RenderAssetUsages::default(),
    )
}

fn render(source: &Source) -> Vec<u8> {
    let (pixels, mask) = match source {
        Source::Color(pixels) => (pixels.as_slice(), false),
        Source::Mask(mask) => (mask.effective(), true),
    };
    let side = (pixels.len() / 4).isqrt();
    let mut out = vec![0; SIZE * SIZE * 4];
    if side == 0 {
        return out;
    }
    let extent = |index: usize| {
        let start = index * side / SIZE;
        start..((index + 1) * side / SIZE).max(start + 1)
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut rgb = [0u64; 3];
            let mut alpha = 0u64;
            let (xs, ys) = (extent(x), extent(y));
            let count = (xs.len() * ys.len()) as u64;
            for row in ys {
                let start = (row * side + xs.start) * 4;
                let end = (row * side + xs.end) * 4;
                for pixel in pixels[start..end].chunks_exact(4) {
                    if mask {
                        rgb[0] += u64::from(pixel[0]);
                    } else {
                        let weight = u64::from(pixel[3]);
                        alpha += weight;
                        for (sum, value) in rgb.iter_mut().zip(pixel) {
                            *sum += u64::from(*value) * weight;
                        }
                    }
                }
            }
            let offset = (y * SIZE + x) * 4;
            if mask {
                let value = ((rgb[0] + count / 2) / count) as u8;
                out[offset..offset + 4].copy_from_slice(&[value, value, value, 255]);
            } else if alpha != 0 {
                let [r, g, b] = rgb.map(|sum| ((sum + alpha / 2) / alpha) as u8);
                out[offset..offset + 4].copy_from_slice(&[
                    r,
                    g,
                    b,
                    ((alpha + count / 2) / count) as u8,
                ]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_preserve_thin_color_and_show_the_effective_mask() {
        let mut color = vec![0; 128 * 128 * 4];
        color[4..8].copy_from_slice(&[210, 65, 12, 255]);
        let painted = render(&Source::Color(Arc::new(color)));
        assert_eq!(&painted[..8], &[210, 65, 12, 64, 0, 0, 0, 0]);

        let raw = crate::editor::model_paint_data::solid(2, [25, 25, 25, 255]);
        let mask = Mask::new(raw, vec![Filter::new(FilterKind::Invert)]);
        let preview = render(&Source::Mask(Arc::new(mask)));
        assert_eq!(&preview[..4], &[230, 230, 230, 255]);
    }
}
