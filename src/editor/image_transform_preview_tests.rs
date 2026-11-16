use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

#[test]
fn live_transform_inverse_maps_perspective_mirrors_and_rejects_degenerate_frames() {
    let size = Vec2::new(2048.0, 1024.0);
    for quad in [
        frame_quad([30.0, -20.0, 1500.0, 800.0]),
        [
            Vec2::new(300.0, 100.0),
            Vec2::new(1600.0, 250.0),
            Vec2::new(1300.0, 900.0),
            Vec2::new(400.0, 700.0),
        ],
        [
            Vec2::new(1400.0, 100.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(100.0, 800.0),
            Vec2::new(1400.0, 800.0),
        ],
    ] {
        let rows = inverse(quad, size).unwrap();
        for (p, expected) in quad
            .into_iter()
            .zip([Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y])
        {
            let p = (p / size).extend(1.0);
            let uv = Vec2::new(rows[0].truncate().dot(p), rows[1].truncate().dot(p))
                / rows[2].truncate().dot(p);
            assert!(uv.abs_diff_eq(expected, 0.00001), "{uv:?} != {expected:?}");
        }
    }
    assert!(inverse([Vec2::ONE; 4], size).is_none());
    assert!(inverse(frame_quad([0.0, 0.0, 10.0, 10.0]), Vec2::ZERO).is_none());
}

fn raster(rect: Rect, color: [u8; 4]) -> Layer {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.write_interleaved(rect, &color.repeat((rect.width() * rect.height()) as usize));
    Layer::new("test", LayerContent::Raster(surface))
}
fn pixel(image: &Rgba8Image, x: usize, y: usize) -> [u8; 4] {
    let i = (y * image.width as usize + x) * 4;
    image.pixels[i..i + 4].try_into().unwrap()
}

#[test]
fn live_transform_planes_keep_stationary_pixels_and_occluding_layers() {
    let mut doc = Document::new("test", Size::new(24, 16), ColorMode::Rgb, SampleType::U8);
    let bottom = raster(doc.bounds(), [240, 240, 240, 255]);
    let middle = raster(Rect::new(2, 2, 20, 14), [40, 100, 220, 255]);
    let top = raster(Rect::new(12, 4, 23, 10), [240, 70, 30, 255]);
    let id = middle.id;
    doc.layers = vec![bottom, middle, top];
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.write_interleaved(Rect::new(4, 3, 10, 11), &[255; 48]);
    doc.selection = Some(selection);
    let planes = prepare(Source {
        doc: doc.clone(),
        moving: vec![id],
        whole: vec![],
        frame: [4.0, 3.0, 10.0, 11.0],
    })
    .unwrap();
    assert_eq!(pixel(&planes.under, 5, 5), [240, 240, 240, 255]);
    assert_eq!(pixel(&planes.under, 2, 5), [40, 100, 220, 255]);
    assert_eq!(pixel(&planes.moving, 2, 2), [40, 100, 220, 255]);
    assert_eq!(pixel(&planes.above, 14, 6), [240, 70, 30, 255]);
    assert_eq!(pixel(&planes.above, 5, 5)[3], 0);

    let mut flat = vec![];
    let mut moving = vec![];
    let mut whole = vec![];
    doc.layers[1].blend = BlendMode::Multiply;
    assert!(
        flatten(
            &doc.layers,
            &[id],
            false,
            true,
            &mut flat,
            &mut moving,
            &mut whole
        )
        .is_none(),
        "Blends dependent on underlying pixels require native compositing"
    );
}

#[test]
fn live_transform_groups_inherit_visibility_and_ignore_fragment_selection() {
    let child = raster(Rect::new(2, 2, 6, 6), [40, 100, 220, 255]);
    let child_id = child.id;
    let mut group = Layer::group("group", vec![child]);
    group.visible = false;
    let mut flat = vec![];
    let mut moving = vec![];
    let mut whole = vec![];
    flatten(
        std::slice::from_ref(&group),
        &[group.id],
        false,
        true,
        &mut flat,
        &mut moving,
        &mut whole,
    )
    .unwrap();
    assert_eq!(moving, vec![child_id]);
    assert_eq!(whole, vec![child_id]);
    assert!(!flat[0].visible);
}
