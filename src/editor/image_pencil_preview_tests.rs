use super::*;
use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

#[test]
fn pencil_preview_cells_match_native_dabs_for_both_shapes_and_pixel_parities() {
    for size in [1, 2, 3, 4, 7, 9, 24, 256] {
        for square in [false, true] {
            let mask = Mask::new(size, square).unwrap();
            let brush = BrushSettings {
                size: size as f32,
                square,
                aliased: true,
                hardness: 1.0,
                ..Default::default()
            };
            let mut target = Surface::new(PixelFormat::RGBA8);
            let point = StrokePoint::new(10.3, -4.2, 1.0);
            photocraft_paint::render_stroke(&mut target, &brush, &[point], None, false, 1.0);
            let (cx, cy) = photocraft_paint::grid_center(point.x, point.y, size as f32);
            let offset = Vec2::new(cx, cy) - mask.center;
            let mut count = 0;
            for y in mask.rect.y0..mask.rect.y1 {
                for x in mask.rect.x0..mask.rect.x1 {
                    let i = ((y - mask.rect.y0) as u32 * mask.rect.width()
                        + (x - mask.rect.x0) as u32) as usize;
                    let alpha = target.rgba(x + offset.x as i32, y + offset.y as i32)[3];
                    assert_eq!(
                        mask.pixels[i] > 0,
                        alpha > 0.0,
                        "size={size}, square={square}, {x},{y}"
                    );
                    assert!(
                        alpha == 0.0 || alpha == 1.0,
                        "Pencil pixels must not be smoothed"
                    );
                    count += usize::from(mask.pixels[i] > 0);
                }
            }
            if square {
                assert_eq!(count, size as usize * size as usize);
            }
            if !square && size >= 4 {
                assert!(count < size as usize * size as usize);
            }
        }
    }
}

#[test]
fn pencil_preview_caches_coverage_without_allocating_images_on_cursor_movement() {
    let mut images = Assets::<Image>::default();
    let mut cache = Cache::default();
    assert!(cache.prepare(9, false, &mut images));
    let handle = cache.image.clone();
    let round = images.get(handle.as_ref().unwrap()).unwrap().data.clone();
    for _ in 0..30 {
        assert!(cache.prepare(9, false, &mut images));
    }
    assert_eq!(images.len(), 1);
    assert_eq!(cache.image, handle);
    assert!(cache.prepare(9, true, &mut images));
    assert_eq!(cache.image, handle);
    assert_eq!(images.len(), 1);
    assert_ne!(images.get(handle.as_ref().unwrap()).unwrap().data, round);
}
