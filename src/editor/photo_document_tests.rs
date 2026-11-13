use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bevy-photo-doc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("photo.png");
        image::RgbaImage::from_pixel(48, 32, image::Rgba([30, 60, 90, 255]))
            .save(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

#[test]
fn native_sidecar_round_trips_layers_and_save_updates_source_pixels() {
    let fixture = Fixture::new();
    let original = std::fs::read(&fixture.0).unwrap();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    assert!(sidecar(&fixture.0).exists());
    assert_eq!(std::fs::read(&fixture.0).unwrap(), original);
    doc.add_layer();
    let surface = doc.document.layers[doc.active].surface().unwrap().clone();
    doc.paint(
        &surface,
        &BrushSettings {
            size: 12.0,
            color: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
        &[StrokePoint::new(20.0, 16.0, 1.0)],
    );
    assert!(doc.dirty());
    save(&fixture.0, &doc.document).unwrap();
    let reopened = PhotoDocument::load(&fixture.0).unwrap();
    assert_eq!(reopened.document.layers.len(), 2);
    let native = photocraft_format::load_path(&sidecar(&fixture.0)).unwrap();
    assert_eq!(native.layers.len(), 2);
    let pixels = image::open(&fixture.0).unwrap().into_rgba8();
    assert_eq!(pixels.get_pixel(20, 16).0, [255, 0, 0, 255]);
    assert_eq!(pixels.as_raw(), &reopened.composite().pixels);
}

#[test]
fn layer_commands_and_undo_restore_pixels_order_and_saved_state() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.add_layer();
    doc.fill(2, 2, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(doc.composite().pixels[..4], [0, 255, 0, 255]);
    doc.visibility(1);
    assert_eq!(doc.composite().pixels[..4], [30, 60, 90, 255]);
    doc.undo();
    assert_eq!(doc.composite().pixels[..4], [0, 255, 0, 255]);
    doc.opacity(0.5);
    let color = doc.composite().pixels[..4].to_vec();
    assert!(color[0] > 10 && color[1] > 150);
    doc.undo();
    doc.reorder(false);
    assert_eq!(doc.composite().pixels[..4], [30, 60, 90, 255]);
    doc.undo();
    doc.rename("Краска");
    doc.duplicate_layer();
    assert_eq!(doc.document.layers.len(), 3);
    doc.delete_layer();
    assert_eq!(doc.document.layers.len(), 2);
    while doc.can_undo() {
        doc.undo();
    }
    assert!(!doc.dirty());
    assert_eq!(doc.document.layers.len(), 1);
    assert!(doc.can_redo());
}

#[test]
fn corrupt_sidecar_never_overwrites_source_or_native_file() {
    let fixture = Fixture::new();
    let original = std::fs::read(&fixture.0).unwrap();
    std::fs::write(sidecar(&fixture.0), b"broken").unwrap();
    assert!(PhotoDocument::load(&fixture.0).is_err());
    assert_eq!(std::fs::read(&fixture.0).unwrap(), original);
    assert_eq!(std::fs::read(sidecar(&fixture.0)).unwrap(), b"broken");
}

#[test]
fn saving_keeps_the_source_format_for_every_supported_raster_type() {
    let fixture = Fixture::new();
    for extension in ["png", "jpg", "webp", "bmp", "tga", "ico"] {
        let source = fixture
            .0
            .with_file_name(format!("raster-{extension}.{extension}"));
        if extension == "jpg" {
            image::RgbImage::from_pixel(24, 24, image::Rgb([20, 40, 60]))
                .save(&source)
                .unwrap();
        } else {
            image::RgbaImage::from_pixel(24, 24, image::Rgba([20, 40, 60, 255]))
                .save(&source)
                .unwrap();
        }
        let mut doc = PhotoDocument::load(&source).unwrap();
        doc.fill(2, 2, [0.0, 1.0, 0.0, 1.0]);
        save(&source, &doc.document).unwrap();
        let pixels = image::open(&source).unwrap().into_rgb8();
        let color = pixels.get_pixel(2, 2);
        assert!(color[1] > 240 && color[0] < 15, "{extension}: {color:?}");
        assert_eq!(
            PhotoDocument::load(&source).unwrap().document.layers.len(),
            1
        );
    }
}

#[test]
fn images_with_the_same_stem_never_reuse_another_images_native_project() {
    let fixture = Fixture::new();
    PhotoDocument::load(&fixture.0).unwrap();
    let jpeg = fixture.0.with_extension("jpg");
    image::RgbImage::from_pixel(48, 32, image::Rgb([255, 0, 0]))
        .save(&jpeg)
        .unwrap();
    let bytes = std::fs::read(&jpeg).unwrap();
    let native = std::fs::read(sidecar(&jpeg)).unwrap();
    assert!(PhotoDocument::load(&jpeg).is_err());
    assert_eq!(std::fs::read(&jpeg).unwrap(), bytes);
    assert_eq!(std::fs::read(sidecar(&jpeg)).unwrap(), native);
}

#[test]
fn eraser_and_layer_move_preserve_the_original_layer_and_dimensions() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    let source = doc.document.layers[0].surface().unwrap().clone();
    doc.checkpoint();
    doc.paint(
        &source,
        &BrushSettings {
            size: 10.0,
            erase: true,
            ..Default::default()
        },
        &[StrokePoint::new(12.0, 12.0, 1.0)],
    );
    assert_eq!(doc.composite().pixels[(12 * 48 + 12) * 4 + 3], 0);
    doc.undo();
    assert!(!doc.dirty());
    doc.checkpoint();
    doc.move_layer(&source, 4, 0);
    assert_eq!(doc.composite().pixels[3], 0);
    assert_eq!(doc.size(), Size::new(48, 32));
}
