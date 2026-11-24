use super::*;
use photocraft_color::PixelFormat;
use photocraft_paint::render_stroke;
use std::collections::HashSet;

fn draw(brush: &BrushSettings, points: &[StrokePoint]) -> Surface {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    render_stroke(&mut surface, brush, points, None, false, 1.0);
    surface
}

fn brush(index: usize, size: f32) -> BrushSettings {
    State {
        tool: PhotoTool::Brush,
        brush_preset: Some(index),
        size,
        opacity: 1.0,
        ..default()
    }
    .brush()
}

#[test]
fn imported_strokes_paint_distinct_materials() {
    let presets = &library().presets;
    assert_eq!(presets.len(), 180);
    let proof = std::env::var_os("BEVY_BRUSH_PROOF_DIR").map(std::path::PathBuf::from);
    let mut signatures = HashSet::new();
    for (index, preset) in presets.iter().enumerate() {
        let settings = brush(index, preset.diameter);
        assert!(
            settings.stroke_sample.is_some() || settings.texture.pattern.bitmap().is_some(),
            "missing sample for {}",
            preset.name
        );
        let points: Vec<_> = (0..=64)
            .map(|i| {
                let t = f64::from(i) / 64.0;
                let wave = (t * std::f64::consts::TAU).sin();
                let pressure = 0.2 + 0.8 * (t * std::f64::consts::PI).sin() as f32;
                StrokePoint::new(36.0 + t * 212.0, 56.0 + wave * 11.0, pressure)
            })
            .collect();
        let original = Surface::new(PixelFormat::RGBA8);
        let mut surface = original.clone();
        let mut renderer = photocraft_paint::StrokeRenderer::new(&settings, Some(surface.format()), 1.0);
        let mut tail = photocraft_geom::Rect::EMPTY;
        for chunk in points.chunks(12) {
            renderer.push(chunk);
            (_, tail) = renderer.composite_live(tail, &original, &mut surface, None, false);
        }
        renderer.finish();
        renderer.composite_live(tail, &original, &mut surface, None, false);
        let swatch = image::GrayImage::from_fn(288, 112, |x, y| {
            image::Luma([(surface.rgba(x as i32, y as i32)[3] * 255.0).round() as u8])
        });
        assert!(
            swatch.as_raw().iter().any(|v| *v > 8),
            "{} produced no ink",
            preset.name
        );
        if let Some(dir) = &proof {
            std::fs::create_dir_all(dir).unwrap();
            swatch.save(dir.join(format!("{index}.png"))).unwrap();
        }
        assert!(
            signatures.insert(swatch.into_raw()),
            "{} repeated a stroke",
            preset.name
        );
    }
}

#[test]
fn parallel_brush_marks_follow_the_path_and_survive_dab_overlap() {
    let index = library()
        .presets
        .iter()
        .position(|p| p.name == "Acacia")
        .unwrap();
    let settings = brush(index, 54.0);
    for vertical in [false, true] {
        let point = |a, b| {
            if vertical {
                StrokePoint::new(b, a, 1.0)
            } else {
                StrokePoint::new(a, b, 1.0)
            }
        };
        let points = [point(40.0, 50.0), point(180.0, 50.0)];
        let surface = draw(&settings, &points);
        let mut bands = 0;
        let mut last = false;
        for cross in 10..90 {
            let (x, y) = if vertical { (cross, 110) } else { (110, cross) };
            let ink = surface.rgba(x, y)[3] > 0.35;
            bands += usize::from(ink && !last);
            last = ink;
        }
        assert_eq!(
            bands, 6,
            "lost a bristle or filled a gap (vertical={vertical})"
        );
    }
}

#[test]
fn fixed_paper_pattern_aligns_across_intersecting_strokes() {
    let index = library()
        .presets
        .iter()
        .position(|p| p.name == "Homestead")
        .unwrap();
    let settings = brush(index, 60.0);
    assert!(settings.texture.enabled);
    let horizontal = draw(
        &settings,
        &[
            StrokePoint::new(16.0, 70.0, 1.0),
            StrokePoint::new(124.0, 70.0, 1.0),
        ],
    );
    let vertical = draw(
        &settings,
        &[
            StrokePoint::new(70.0, 16.0, 1.0),
            StrokePoint::new(70.0, 124.0, 1.0),
        ],
    );
    let mut peak = 0.0f32;
    for y in 60..80 {
        for x in 60..80 {
            let a = horizontal.rgba(x, y)[3];
            peak = peak.max(a);
            assert!((a - vertical.rgba(x, y)[3]).abs() <= 1.0 / 255.0);
        }
    }
    assert!(peak > 0.05);
}

#[test]
fn imported_pressure_preserves_each_presets_response() {
    let index = library()
        .presets
        .iter()
        .position(|p| p.name == "Bruny")
        .unwrap();
    let settings = brush(index, 18.0);
    let ink = |pressure| {
        let surface = draw(
            &settings,
            &[
                StrokePoint::new(12.0, 20.0, pressure),
                StrokePoint::new(90.0, 20.0, pressure),
            ],
        );
        (8..33).map(|y| surface.rgba(48, y)[3]).sum::<f32>()
    };
    assert!(ink(1.0) > ink(0.25) * 2.0);
    let standard = State {
        tool: PhotoTool::Brush,
        ..default()
    }
    .brush();
    assert!(standard.stroke_sample.is_none());
}
