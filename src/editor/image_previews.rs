//! Decode only images being browsed, uploading small previews instead of source textures.

use std::{collections::HashMap, path::Path};

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};

use super::{
    Editor,
    browser::{Browser, Entry},
};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 160;

pub(super) struct ImagePreview {
    /// Failed previews have no texture, so the UI can keep its file placeholder.
    pub image: Option<Handle<Image>>,
    /// Source dimensions, before fitting the image into the thumbnail.
    pub dimensions: Option<UVec2>,
    pub error: Option<String>,
}

#[derive(Resource, Default)]
pub(super) struct ImagePreviews {
    scan_revision: Option<u64>,
    browser_revision: Option<u64>,
    visible_files: Vec<usize>,
    completed: HashMap<usize, ImagePreview>,
    pending: Option<Job>,
}

impl ImagePreviews {
    /// `None` means the file has not finished decoding yet.
    pub fn get(&self, file: usize) -> Option<&ImagePreview> {
        self.completed.get(&file)
    }
}

struct Job {
    file: usize,
    scan_revision: u64,
    task: Task<Result<DecodedThumbnail, String>>,
}

struct DecodedThumbnail {
    rgba: Vec<u8>,
    size: UVec2,
    original: UVec2,
}

fn decode(path: &Path) -> Result<DecodedThumbnail, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    // One worker owns the full-resolution pixels temporarily; only its small
    // RGBA result crosses back to the main world or becomes a GPU texture.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let source = reader.decode().map_err(|error| error.to_string())?;
    let original = UVec2::new(source.width(), source.height());
    let thumbnail = if original.x > WIDTH || original.y > HEIGHT {
        source.thumbnail(WIDTH, HEIGHT)
    } else {
        source
    };
    let rgba = thumbnail.into_rgba8();
    Ok(DecodedThumbnail {
        size: UVec2::new(rgba.width(), rgba.height()),
        rgba: rgba.into_raw(),
        original,
    })
}

pub(super) fn update(
    editor: Res<Editor>,
    browser: Res<Browser>,
    mut previews: ResMut<ImagePreviews>,
    mut images: ResMut<Assets<Image>>,
) {
    if previews.scan_revision != Some(browser.scan_revision) {
        previews.completed.clear();
        previews.scan_revision = Some(browser.scan_revision);
        previews.browser_revision = None;
        // A synchronous codec may still be running inside the worker. Let it
        // finish before starting another job, then reject its obsolete index.
    }

    let finished = previews.pending.as_mut().and_then(|job| {
        block_on(poll_once(&mut job.task)).map(|result| (job.file, job.scan_revision, result))
    });
    if let Some((file, scan_revision, result)) = finished {
        previews.pending = None;
        if scan_revision == browser.scan_revision {
            let preview = match result {
                Ok(decoded) => ImagePreview {
                    image: Some(images.add(Image::new(
                        Extent3d {
                            width: decoded.size.x,
                            height: decoded.size.y,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        decoded.rgba,
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
                    ))),
                    dimensions: Some(decoded.original),
                    error: None,
                },
                Err(error) => ImagePreview {
                    image: None,
                    dimensions: None,
                    error: Some(error),
                },
            };
            previews.completed.insert(file, preview);
        }
    }

    if !editor.drawer || previews.pending.is_some() {
        return;
    }
    if previews.browser_revision != Some(browser.revision) {
        previews.visible_files = browser
            .entries(&editor.project.catalog)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Image(file) => Some(file),
                _ => None,
            })
            .collect();
        previews.browser_revision = Some(browser.revision);
    }
    let next = previews
        .visible_files
        .iter()
        .copied()
        .find(|file| !previews.completed.contains_key(file));
    let Some(file) = next else { return };
    let path = editor.root.join(&browser.files[file].path);
    previews.pending = Some(Job {
        file,
        scan_revision: browser.scan_revision,
        task: AsyncComputeTaskPool::get().spawn(async move { decode(&path) }),
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn decode_png(width: u32, height: u32) -> DecodedThumbnail {
        let path = std::env::temp_dir().join(format!(
            "bevy-editor-image-preview-{}-{}.png",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        ));
        image::RgbaImage::from_pixel(width, height, image::Rgba([10, 30, 50, 128]))
            .save(&path)
            .unwrap();
        let decoded = decode(&path);
        let _ = std::fs::remove_file(path);
        decoded.unwrap()
    }

    #[test]
    fn previews_fit_landscape_and_portrait_images_and_keep_original_dimensions() {
        let landscape = decode_png(1000, 250);
        assert_eq!(landscape.original, UVec2::new(1000, 250));
        assert_eq!(landscape.size, UVec2::new(256, 64));
        assert_eq!(landscape.rgba.len(), 256 * 64 * 4);

        let portrait = decode_png(250, 1000);
        assert_eq!(portrait.original, UVec2::new(250, 1000));
        assert_eq!(portrait.size, UVec2::new(40, 160));
    }

    #[test]
    fn small_images_keep_their_pixels_and_alpha_without_upscaling() {
        let preview = decode_png(8, 4);
        assert_eq!(preview.original, UVec2::new(8, 4));
        assert_eq!(preview.size, preview.original);
        assert_eq!(&preview.rgba[..4], &[10, 30, 50, 128]);
    }

    #[test]
    fn invalid_image_data_returns_an_error_without_allocating_a_texture() {
        let path = std::env::temp_dir().join(format!(
            "bevy-editor-image-preview-{}-invalid-{}.png",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode(&path).is_err());
        let _ = std::fs::remove_file(path);
    }
}
