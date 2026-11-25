//! Brush alpha images for the Material Paint shelf and GPU stamps.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const SIDE: u32 = 256;
const COLUMNS: u32 = 6;
const ATLAS: &[u8] = include_bytes!("../../assets/paint/brush-tips.png");
const SPLATTER_CELL: u32 = 3;
const GRAINS: &[u8] = include_bytes!("../../assets/paint/brush-grains.png");
const GRAIN_LABELS: [&str; 25] = [
    "Плитка",
    "Соты",
    "Кирпич",
    "Кирпичная ёлочка",
    "Доски",
    "Булыжник",
    "Рифлёный лист",
    "Перфорация",
    "Карбон",
    "Накатка",
    "Плетение",
    "Холст",
    "Чешуя",
    "Гофра",
    "Полосы",
    "Шахматная клетка",
    "Точки",
    "Решётка",
    "Мелкий шум",
    "Крупный шум",
    "Трещины",
    "Потёртости",
    "Царапины",
    "Кожа",
    "Дерево",
];
const LABELS: [&str; 24] = [
    "Квадрат",
    "Спрей",
    "Потёртости",
    "Брызги",
    "Мел",
    "Царапины",
    "Щетина",
    "Трещины",
    "Точки",
    "Ткань",
    "Звезда",
    "Лист",
    "Купол",
    "Заклёпка",
    "Шестигранный болт",
    "Крестовой винт",
    "Шлицевой винт",
    "Прорезь",
    "Стежок",
    "Двойной стежок",
    "Крестик",
    "Косой стежок",
    "Кольцо",
    "Шайба",
];

pub(super) struct Tip {
    pub name: String,
    pub image: Handle<Image>,
}

type Import = Result<Option<(String, image::GrayImage)>, String>;

#[derive(Default)]
pub(super) struct Library {
    pub entries: Vec<Tip>,
    pub grains: Vec<Tip>,
    import: Option<(usize, Task<Import>)>,
}

fn tip_image(gray: &image::GrayImage) -> Image {
    let rgba = image::RgbaImage::from_fn(gray.width(), gray.height(), |x, y| {
        let v = gray.get_pixel(x, y)[0];
        image::Rgba([v, v, v, 255])
    });
    Image::from_dynamic(
        image::DynamicImage::ImageRgba8(rgba),
        false,
        RenderAssetUsages::default(),
    )
}

fn atlas_cell(atlas: &image::GrayImage, columns: u32, index: u32) -> image::GrayImage {
    image::imageops::crop_imm(
        atlas,
        index % columns * SIDE,
        index / columns * SIDE,
        SIDE,
        SIDE,
    )
    .to_image()
}

pub(super) fn splatter_alpha() -> &'static image::GrayImage {
    static SPLATTER: OnceLock<image::GrayImage> = OnceLock::new();
    SPLATTER.get_or_init(|| {
        let atlas = image::load_from_memory_with_format(ATLAS, image::ImageFormat::Png)
            .expect("bundled brush textures")
            .into_luma8();
        atlas_cell(&atlas, COLUMNS, SPLATTER_CELL)
    })
}

impl Library {
    #[cfg(feature = "mcp")]
    pub(super) fn names(&self) -> Vec<&str> {
        if self.entries.is_empty() {
            LABELS.to_vec()
        } else {
            self.entries.iter().map(|tip| tip.name.as_str()).collect()
        }
    }

    pub fn ensure(&mut self, images: &mut Assets<Image>) {
        Self::fill(&mut self.entries, images, ATLAS, COLUMNS, &LABELS);
    }
    pub fn ensure_grains(&mut self, images: &mut Assets<Image>) {
        Self::fill(&mut self.grains, images, GRAINS, 5, &GRAIN_LABELS);
    }
    fn fill(
        entries: &mut Vec<Tip>,
        images: &mut Assets<Image>,
        png: &[u8],
        columns: u32,
        labels: &[&str],
    ) {
        if !entries.is_empty() {
            return;
        }
        let atlas = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .expect("bundled brush textures")
            .into_luma8();
        for (i, &name) in labels.iter().enumerate() {
            let gray = atlas_cell(&atlas, columns, i as u32);
            entries.push(Tip {
                name: name.into(),
                image: images.add(tip_image(&gray)),
            });
        }
    }

    pub fn start(&mut self, index: usize, root: PathBuf) {
        if self.import.is_some() {
            return;
        }
        self.import = Some((
            index,
            IoTaskPool::get().spawn(async move {
                let path = crate::editor::asset_monitor::choose_file_path(&root)
                    .map_err(|error| error.to_string())?;
                path.map(|path| {
                    let label = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    load(&path).map(|gray| (label, gray))
                })
                .transpose()
            }),
        ));
    }

    pub fn finish(
        &mut self,
        images: &mut Assets<Image>,
    ) -> Option<Result<Option<(usize, usize)>, String>> {
        let (index, task) = self.import.as_mut()?;
        let result = block_on(poll_once(task))?;
        let index = *index;
        self.import = None;
        Some(result.map(|entry| {
            entry.map(|(name, gray)| {
                self.ensure(images);
                let tip = self.entries.len();
                self.entries.push(Tip {
                    name,
                    image: images.add(tip_image(&gray)),
                });
                (index, tip)
            })
        }))
    }
}

pub(super) fn load(path: &Path) -> Result<image::GrayImage, String> {
    use image::{GenericImageView, ImageReader, Limits};
    let mut reader = ImageReader::open(path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|error| error.to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    let (width, height) = decoded.dimensions();
    let rgba = decoded.into_rgba8();
    let alpha = rgba.pixels().any(|p| p[3] != 255);
    let gray = image::GrayImage::from_fn(width, height, |x, y| {
        let p = rgba.get_pixel(x, y);
        if alpha {
            image::Luma([p[3]])
        } else {
            use image::Pixel;
            p.to_luma()
        }
    });
    let scale = SIDE as f32 / width.max(height) as f32;
    let w = (width as f32 * scale).round().max(1.) as u32;
    let h = (height as f32 * scale).round().max(1.) as u32;
    let gray = image::imageops::resize(&gray, w, h, image::imageops::FilterType::Triangle);
    let mut output = image::GrayImage::new(SIDE, SIDE);
    image::imageops::replace(
        &mut output,
        &gray,
        i64::from((SIDE - w) / 2),
        i64::from((SIDE - h) / 2),
    );
    Ok(output)
}
