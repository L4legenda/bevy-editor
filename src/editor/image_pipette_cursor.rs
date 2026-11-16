//! Native eyedropper bitmap, shared with the toolbar's vector icon.
use bevy::{
    asset::RenderAssetUsages,
    prelude::Image,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub(super) fn image(density: f32) -> Result<(Image, (u16, u16)), String> {
    // Reuse the toolbar glyph. A black outer stroke keeps the white pipette
    // legible on light images, with the sampling hotspot at its lower-left tip.
    let source = include_str!("../../assets/icons/lucide/pipette.svg");
    let outer = source
        .replace("currentColor", "#000000")
        .replace("stroke-width=\"2\"", "stroke-width=\"4\"");
    let inner = source.replace("currentColor", "#ffffff");
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 28 28"><g transform="translate(2 2)">{outer}{inner}</g></svg>"#
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .map_err(|error| error.to_string())?;
    let side = (28.0 * density).round().max(1.0) as u32;
    let scale = side as f32 / 28.0;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side).ok_or("Invalid cursor size")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    for pixel in pixmap.pixels() {
        let pixel = pixel.demultiply();
        pixels.extend_from_slice(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
    }
    Ok((
        Image::new(
            Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        ),
        ((4.0 * scale).round() as u16, (24.0 * scale).round() as u16),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eyedropper_bitmap_points_at_the_sample_and_contrasts_at_each_dpi() {
        let output = std::path::PathBuf::from(option_env!("CARGO_MANIFEST_DIR").unwrap_or("."))
            .join("target/photo-eyedropper-qa");
        std::fs::create_dir_all(&output).unwrap();
        for density in [1.0, 1.5, 2.0] {
            let (image, hotspot) = image(density).unwrap();
            assert_eq!(hotspot, ((4.0 * density) as u16, (24.0 * density) as u16));
            assert_eq!(image.width(), (28.0 * density) as u32);
            let pixels = image.data.as_ref().unwrap();
            let hot =
                (usize::from(hotspot.1) * image.width() as usize + usize::from(hotspot.0)) * 4;
            assert!(
                pixels[hot + 3] > 200,
                "The sampling point must land on the pipette tip"
            );
            for background in [0.0, 128.0, 255.0] {
                assert!(
                    pixels
                        .chunks_exact(4)
                        .filter(|p| {
                            let alpha = f32::from(p[3]) / 255.0;
                            (f32::from(p[0]) * alpha + background * (1.0 - alpha) - background)
                                .abs()
                                > 80.0
                        })
                        .count()
                        > 30,
                    "The cursor must be visible on {background} at {density}x"
                );
            }
            image::RgbaImage::from_raw(image.width(), image.height(), pixels.clone())
                .unwrap()
                .save(output.join(format!("pipette-{density}x.png")))
                .unwrap();
        }
    }
}
