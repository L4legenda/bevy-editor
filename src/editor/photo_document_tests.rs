use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[path = "photo_selection_tests.rs"]
mod selection;

#[test]
fn shared_layer_libraries_preserve_pattern_pixels_styles_and_effect_defaults() {
    let fixture = Fixture::new();
    let mut source = PhotoDocument::load(&fixture.0).unwrap();
    let revision = source.revision;
    source
        .layer_command("edit.definePattern", json!({"name":"Мой узор"}))
        .unwrap();
    source
        .layer_command(
            "layer.layerStyle.makeDefault",
            json!({"kind":"dropShadow","params":{"opacity":37.25,"size":19}}),
        )
        .unwrap();
    assert_eq!(
        source.revision, revision,
        "Library edits must not dirty the image"
    );
    source
        .layer_command(
            "layer.layerStyle.stroke",
            json!({"size":3,"color":"#ff0000"}),
        )
        .unwrap();
    source
        .layer_command("style.presets.new", json!({"name":"Моя обводка"}))
        .unwrap();
    let libraries = source.libraries();
    let path = fixture.0.with_file_name("layer-libraries.pcraft");
    libraries.save(&path).unwrap();
    let restored = LayerLibraries::load(&path).unwrap();
    let pattern = restored
        .patterns
        .items
        .iter()
        .find(|p| p.name == "Мой узор")
        .unwrap();
    let original = libraries
        .patterns
        .items
        .iter()
        .find(|p| p.id == pattern.id)
        .unwrap();
    assert_eq!(pattern.surface, original.surface);
    assert_eq!((pattern.width, pattern.height), (48, 32));
    let target_fixture = Fixture::new();
    let mut target = PhotoDocument::load(&target_fixture.0).unwrap();
    target.set_libraries(&restored);
    assert_eq!(target.effect_defaults("dropShadow")["opacity"], 37.25);
    assert_eq!(target.effect_defaults("dropShadow")["size"], 19);
    assert!(
        target
            .library_options("pattern", "layer.newFillLayer.pattern")
            .unwrap()
            .contains(&json!("Мой узор"))
    );
    target
        .layer_command("style.presets.apply", json!({"preset":"Моя обводка"}))
        .unwrap();
    assert_eq!(
        target.active_layer().unwrap().effects.items,
        source.active_layer().unwrap().effects.items
    );
    target
        .layer_command("layer.newFillLayer.pattern", json!({"pattern":"Мой узор"}))
        .unwrap();
    assert_eq!(
        target
            .document
            .patterns
            .iter()
            .find(|p| p.id == pattern.id)
            .unwrap()
            .surface,
        pattern.surface
    );
    assert_eq!(target.composite().pixels[3], 255);
}

#[test]
fn nested_layers_masks_and_effects_share_paint_history_and_roundtrip() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.add_layer();
    doc.fill(0, 0, [1.0, 0.0, 0.0, 1.0]);
    let red = doc.active_id().unwrap();
    doc.layer_command("layer.layerMask.hideAll", json!({}))
        .unwrap();
    assert_eq!(&doc.composite().pixels[..4], &[30, 60, 90, 255]);
    doc.mask_target = true;
    let original = doc.paint_surface().unwrap().clone();
    doc.checkpoint();
    doc.paint(
        &original,
        &BrushSettings {
            size: 12.0,
            color: [1.0; 4],
            ..Default::default()
        },
        &[StrokePoint::new(20.0, 16.0, 1.0)],
    );
    assert_eq!(
        &doc.composite().pixels[(16 * 48 + 20) * 4..(16 * 48 + 20) * 4 + 4],
        &[255, 0, 0, 255]
    );
    doc.undo();
    assert_eq!(&doc.composite().pixels[..4], &[30, 60, 90, 255]);
    doc.redo();
    doc.add_layer();
    let top = doc.active;
    doc.select(1, false, false);
    doc.select(top, true, false);
    doc.layer_command("layer.groupLayers", json!({})).unwrap();
    assert_eq!(doc.document.layers.len(), 2);
    assert_eq!(doc.active_layer().unwrap().children().unwrap().len(), 2);
    assert!(doc.document.layer(red).unwrap().mask.is_some());
    doc.layer_command(
        "layer.layerStyle.stroke",
        json!({"size":2,"color":"#ffffff"}),
    )
    .unwrap();
    let expected = doc.composite().pixels;
    save(&fixture.0, &doc.document).unwrap();
    let reopened = PhotoDocument::load(&fixture.0).unwrap();
    assert_eq!(reopened.document.layer_count(), 4);
    assert_eq!(reopened.composite().pixels, expected);
    doc.undo();
    assert!(doc.active_layer().unwrap().effects.items.is_empty());
    doc.undo();
    assert_eq!(doc.document.layers.len(), 3);
}

#[test]
fn smart_contents_return_to_parent_as_one_undoable_edit() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.layer_command("layer.smartObjects.convertToSmartObject", json!({}))
        .unwrap();
    let parent = doc.document.id;
    let before = doc.composite().pixels;
    doc.layer_command("layer.smartObjects.editContents", json!({}))
        .unwrap();
    assert!(doc.editing_smart());
    assert_ne!(doc.document.id, parent);
    doc.fill(2, 2, [1.0, 0.0, 0.0, 1.0]);
    doc.finish_smart().unwrap();
    assert!(!doc.editing_smart());
    assert_eq!(doc.document.id, parent);
    assert_ne!(doc.composite().pixels, before);
    doc.undo();
    assert_eq!(doc.composite().pixels, before);
    doc.redo();
    assert_eq!(&doc.composite().pixels[..4], &[255, 0, 0, 255]);
}

#[test]
fn moving_a_group_keeps_mask_alignment_and_one_history_step() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.layer_command("layer.groupLayers", json!({})).unwrap();
    let before = doc.composite().pixels;
    let mut original = doc.begin_move();
    doc.translate_live(&mut original, 4, 0).unwrap();
    doc.translate_live(&mut original, 8, 0).unwrap();
    assert_eq!(doc.composite().pixels[3], 0);
    doc.undo();
    assert_eq!(doc.composite().pixels, before);
    assert!(doc.active_layer().unwrap().is_group());
}

#[test]
fn moving_back_to_origin_and_cancel_preserve_full_undo_and_redo_history() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    for i in 0..HISTORY {
        doc.layer_command(
            "layer.renameLayer",
            json!({"name": format!("Название {i}")}),
        )
        .unwrap();
    }
    let revisions = doc.undo.iter().map(|s| s.revision).collect::<Vec<_>>();
    let before = doc.composite().pixels;
    let mut movement = doc.begin_move();
    doc.translate_live(&mut movement, 4, 0).unwrap();
    doc.translate_live(&mut movement, 0, 0).unwrap();
    assert_eq!(
        doc.undo.iter().map(|s| s.revision).collect::<Vec<_>>(),
        revisions
    );
    doc.translate_live(&mut movement, 8, 0).unwrap();
    doc.translate_live(&mut movement, 0, 0).unwrap();
    doc.translate_live(&mut movement, 3, 0).unwrap();
    doc.cancel_move(&mut movement);
    assert_eq!(
        doc.undo.iter().map(|s| s.revision).collect::<Vec<_>>(),
        revisions
    );
    assert_eq!(doc.composite().pixels, before);
    doc.undo();
    let redo = doc.redo.len();
    let revision = doc.revision;
    let mut movement = doc.begin_move();
    doc.translate_live(&mut movement, 7, 0).unwrap();
    doc.cancel_move(&mut movement);
    assert_eq!(doc.revision, revision);
    assert_eq!(doc.redo.len(), redo);
    doc.redo();
    assert_eq!(
        doc.document.layers[0].name,
        format!("Название {}", HISTORY - 1)
    );
}

#[test]
fn mask_preview_is_view_state_and_never_changes_exported_pixels() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.layer_command("layer.layerMask.hideAll", json!({}))
        .unwrap();
    doc.saved_revision = doc.revision;
    let before = doc.composite().pixels;
    let revision = doc.revision;
    doc.layer_command("view.layerMask", json!({"mode":"gray"}))
        .unwrap();
    assert!(!doc.dirty());
    assert_eq!(doc.revision, revision);
    assert!(doc.mask_target);
    assert_eq!(&doc.preview().pixels[..4], &[0, 0, 0, 255]);
    assert_eq!(doc.composite().pixels, before);
    save(&fixture.0, &doc.document).unwrap();
    assert_eq!(
        image::open(&fixture.0).unwrap().into_rgba8().as_raw(),
        &before
    );
    doc.layer_command("view.layerMask", json!({"mode":"off"}))
        .unwrap();
    assert_eq!(doc.preview().pixels, before);
    doc.layer_command("view.layerMask", json!({"mode":"gray"}))
        .unwrap();
    doc.select(doc.active, false, false);
    assert!(!doc.mask_target);
    doc.layer_command("layer.layerMask.edit", json!({"density":75}))
        .unwrap();
    assert!(
        doc.mask_view.is_none(),
        "A later command must not restore a dismissed mask preview"
    );
    assert!(!doc.mask_target);
}

#[test]
fn every_layer_blend_and_effect_family_survives_native_save() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    doc.add_layer();
    doc.fill(0, 0, [0.9, 0.4, 0.1, 0.8]);
    for blend in photocraft_color::BlendMode::layer_modes() {
        doc.layer_command("layer.setProps", json!({"blend":blend.label()}))
            .unwrap();
        assert_eq!(doc.composite().pixels.len(), 48 * 32 * 4);
    }
    for kind in [
        "dropShadow",
        "innerShadow",
        "outerGlow",
        "innerGlow",
        "stroke",
        "colorOverlay",
        "gradientOverlay",
        "patternOverlay",
        "bevelEmboss",
        "satin",
    ] {
        doc.layer_command(&format!("layer.layerStyle.{kind}"), json!({}))
            .unwrap();
    }
    assert_eq!(doc.active_layer().unwrap().effects.items.len(), 10);
    save(&fixture.0, &doc.document).unwrap();
    let reopened = PhotoDocument::load(&fixture.0).unwrap();
    assert_eq!(
        reopened.document.layers.last().unwrap().effects.items.len(),
        10
    );
    assert_eq!(reopened.composite().pixels, doc.composite().pixels);
}

#[test]
fn smart_filter_mask_paint_refreshes_cache_and_undo_restores_the_filtered_image() {
    let fixture = Fixture::new();
    let mut doc = PhotoDocument::load(&fixture.0).unwrap();
    let unfiltered = doc.composite().pixels;
    doc.layer_command("filter.other.highPass", json!({"radius":2}))
        .unwrap();
    let filtered = doc.composite().pixels;
    assert_ne!(filtered, unfiltered);
    doc.ensure_filter_mask().unwrap();
    doc.layer_command("view.layerMask", json!({"mask":"filter","mode":"gray"}))
        .unwrap();
    doc.layer_command("view.layerMask", json!({"mask":"filter","mode":"off"}))
        .unwrap();
    assert!(
        doc.mask_target && doc.filter_target,
        "Dismissing grayscale preview keeps the filter mask selected for painting"
    );
    let original = doc
        .paint_surface()
        .expect("Smart filters have an editable white mask")
        .clone();
    let brush = BrushSettings {
        size: 16.0,
        hardness: 1.0,
        color: [0.0, 0.0, 0.0, 1.0],
        ..Default::default()
    };
    let mut renderer = photocraft_paint::StrokeRenderer::new(&brush, Some(original.format()), 1.0)
        .with_clip(doc.document.bounds());
    doc.checkpoint();
    doc.take_damage();
    let mut tail = Rect::EMPTY;
    doc.paint_live(
        &original,
        &mut renderer,
        &[StrokePoint::new(20.0, 16.0, 1.0)],
        &mut tail,
        true,
    )
    .unwrap();
    let center = (16 * 48 + 20) * 4;
    assert_eq!(
        &doc.composite().pixels[center..center + 4],
        &unfiltered[center..center + 4]
    );
    assert!(
        doc.take_damage().is_none(),
        "A changed filter mask invalidates the full filtered cache"
    );
    doc.undo();
    assert_eq!(doc.composite().pixels, filtered);
    doc.redo();
    assert_eq!(
        &doc.composite().pixels[center..center + 4],
        &unfiltered[center..center + 4]
    );
    doc.layer_command(
        "editor.filterMaskOptions",
        json!({"density":50,"feather":2}),
    )
    .unwrap();
    let LayerContent::Smart(smart) = &doc.active_layer().unwrap().content else {
        panic!("smart layer")
    };
    assert_eq!(smart.filter_mask.as_ref().unwrap().density, 0.5);
    assert_eq!(smart.filter_mask.as_ref().unwrap().feather, 2.0);
    doc.undo();
    assert_eq!(
        &doc.composite().pixels[center..center + 4],
        &unfiltered[center..center + 4]
    );
}

#[test]
fn cross_document_paste_keeps_nested_masks_and_remaps_link_groups() {
    let first = Fixture::new();
    let second = Fixture::new();
    let mut source = PhotoDocument::load(&first.0).unwrap();
    source.add_layer();
    source
        .layer_command("layer.layerMask.hideAll", json!({}))
        .unwrap();
    source.select(0, false, false);
    source.select(1, true, false);
    source.layer_command("layer.linkLayers", json!({})).unwrap();
    source
        .layer_command("layer.groupLayers", json!({}))
        .unwrap();
    let copied = source.copy_layers();
    assert_eq!(copied.len(), 1);
    let mut target = PhotoDocument::load(&second.0).unwrap();
    target.paste_layers(&copied).unwrap();
    let group = target.active_layer().unwrap();
    let children = group.children().unwrap();
    assert_ne!(group.id, copied[0].id);
    assert_eq!(children[0].link_group, children[1].link_group);
    assert!(children[1].mask.is_some());
    target.undo();
    assert_eq!(target.document.layer_count(), 1);
}
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
    doc.layer_command("layer.renameLayer", json!({"name": "Краска"}))
        .unwrap();
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

#[test]
fn live_paint_shows_the_finished_prefix_before_release_without_opacity_build_up() {
    for (erase, aliased) in [(false, false), (true, false), (false, true)] {
        let fixture = Fixture::new();
        let mut doc = PhotoDocument::load(&fixture.0).unwrap();
        let original = doc.document.layers[0].surface().unwrap().clone();
        let brush = BrushSettings {
            size: 9.0,
            hardness: 0.4,
            opacity: 0.35,
            color: [1.0, 0.15, 0.1, 1.0],
            erase,
            aliased,
            ..Default::default()
        };
        let bounds = Rect::from_size(doc.size());
        let mut renderer =
            photocraft_paint::StrokeRenderer::new(&brush, Some(original.format()), 1.0)
                .with_clip(bounds);
        let mut tail = Rect::EMPTY;
        let mut points = Vec::new();
        doc.checkpoint();
        doc.take_damage();
        for (x, y) in [(8.0, 15.0), (20.0, 8.0), (33.0, 25.0), (44.0, 14.0)] {
            let point = StrokePoint::new(x, y, 1.0);
            points.push(point);
            doc.paint_live(&original, &mut renderer, &[point], &mut tail, false)
                .unwrap();
            let mut expected = original.clone();
            photocraft_paint::render_stroke_clipped(
                &mut expected,
                &brush,
                &points,
                None,
                false,
                1.0,
                Some(bounds),
            );
            assert_eq!(
                doc.document.layers[0]
                    .surface()
                    .unwrap()
                    .to_interleaved(bounds),
                expected.to_interleaved(bounds),
                "The current pointer position must be visible while held: erase={erase}, aliased={aliased}, points={}",
                points.len()
            );
            let damage = doc
                .take_damage()
                .expect("A stroke updates a region, not the whole document");
            assert!(!damage.is_empty());
            let region = doc.composite_region(damage);
            let full = doc.composite();
            for (row, pixels) in region
                .pixels
                .chunks_exact(region.width as usize * 4)
                .enumerate()
            {
                let start =
                    ((damage.y0 as usize + row) * full.width as usize + damage.x0 as usize) * 4;
                assert_eq!(pixels, &full.pixels[start..start + pixels.len()]);
            }
        }
        let before_release = doc.composite().pixels;
        doc.paint_live(&original, &mut renderer, &[], &mut tail, true)
            .unwrap();
        assert_eq!(
            doc.composite().pixels,
            before_release,
            "Release must not suddenly add the missing tail"
        );
        assert!(tail.is_empty());
        doc.undo();
        assert!(!doc.dirty());
        assert!(
            doc.take_damage().is_none(),
            "Undo invalidates the full preview"
        );
        assert_eq!(
            doc.document.layers[0]
                .surface()
                .unwrap()
                .to_interleaved(bounds),
            original.to_interleaved(bounds)
        );
    }
}

#[test]
#[ignore = "times the previous full-canvas compositor and damage-only compositor on 1 and 16 MP canvases"]
fn paint_damage_compositing_benchmark() {
    use std::{hint::black_box, time::Instant};
    for size in [1024, 4096] {
        let mut document = Document::new(
            "latency",
            Size::new(size, size),
            ColorMode::Rgb,
            SampleType::U8,
        );
        document.layers.push(Layer::new(
            "Background",
            LayerContent::Raster(Surface::with_default(
                PixelFormat::RGBA8,
                &[0.9, 0.9, 0.9, 1.0],
            )),
        ));
        document.layers.push(Layer::new(
            "Paint",
            LayerContent::Raster(Surface::new(PixelFormat::RGBA8)),
        ));
        let rect = Rect::new(400, 400, 496, 496);
        let mut whole = Vec::new();
        let mut damage = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            black_box(composite(&document));
            whole.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            black_box(super::composite_region(&document, rect));
            damage.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        whole.sort_by(f64::total_cmp);
        damage.sort_by(f64::total_cmp);
        println!(
            "PAINT_COMPOSITE {size}x{size}, 2 layers, 96x96 damage, n=7 median: full={:.3} ms, region={:.3} ms, {:.1}x; upload {} -> {} bytes",
            whole[3],
            damage[3],
            whole[3] / damage[3],
            size as u64 * size as u64 * 4,
            rect.width() * rect.height() * 4
        );
    }
}
