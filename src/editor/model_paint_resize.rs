//! Resample the editable layers of one texture set on the paint worker.
use super::{Surface, solid};
use image::{
    ImageBuffer, Rgba,
    imageops::{FilterType, resize},
};
use std::{collections::HashMap, sync::Arc};

pub(super) fn apply(surface: &mut Surface, size: u32) {
    let previous = surface.size;
    let mut cached = HashMap::new();
    let mut pixels = |input: &Arc<Vec<u8>>, alpha: bool| {
        cached
            .entry((Arc::as_ptr(input), alpha))
            .or_insert_with(|| resample(input, previous, size, alpha))
            .clone()
    };
    for base in &mut surface.base {
        *base = pixels(base, false);
    }
    for layer in &mut surface.stack.layers {
        for channel in layer.channels.iter_mut().flatten() {
            *channel = pixels(channel, true);
        }
        if let Some(mask) = &mut layer.mask {
            let mut resized = mask.with_pixels(pixels(&mask.pixels, false));
            resized.context = None;
            *mask = Arc::new(resized);
        }
    }
    surface.size = size;
}

fn resample(input: &[u8], previous: u32, size: u32, alpha: bool) -> Arc<Vec<u8>> {
    let first: [u8; 4] = input[..4].try_into().unwrap();
    if input.chunks_exact(4).all(|pixel| pixel == first) {
        return solid(size, first);
    }
    let filter = FilterType::Triangle;
    if !alpha {
        let image = ImageBuffer::<Rgba<u8>, _>::from_raw(previous, previous, input).unwrap();
        return Arc::new(resize(&image, size, size, filter).into_raw());
    }
    // Preserve straight-alpha paint colors across transparent texels.
    let mut premultiplied = Vec::with_capacity(input.len());
    for p in input.chunks_exact(4) {
        let a = u16::from(p[3]);
        premultiplied.extend([
            u16::from(p[0]) * a,
            u16::from(p[1]) * a,
            u16::from(p[2]) * a,
            a * 255,
        ]);
    }
    let image = ImageBuffer::<Rgba<u16>, _>::from_raw(previous, previous, premultiplied).unwrap();
    let resized = resize(&image, size, size, filter);
    let mut result = Vec::with_capacity(size as usize * size as usize * 4);
    for p in resized.pixels() {
        let a = u32::from(p[3]);
        let color = |value| {
            if a == 0 {
                0
            } else {
                ((u32::from(value) * 255 + a / 2) / a).min(255) as u8
            }
        };
        result.extend([
            color(p[0]),
            color(p[1]),
            color(p[2]),
            ((a + 127) / 255) as u8,
        ]);
    }
    Arc::new(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model_paint_data::{Layer, LayerKind, Stack, mask::Mask};

    #[test]
    fn paint_resolution_preserves_layer_color_mask_and_shared_pixels() {
        let mut layer = Layer::new("Paint".into(), LayerKind::Paint);
        let red = Arc::new([255, 0, 0, 255, 0, 0, 0, 0].repeat(2));
        layer.channels[0] = Some(red);
        layer.mask = Some(Arc::new(Mask::new(solid(2, [128; 4]), vec![])));
        let mut surface = Surface {
            size: 2,
            base: std::array::from_fn(|_| solid(2, [9; 4])),
            stack: Stack {
                layers: vec![layer.clone(), layer],
                selected: 1,
            },
            material: serde_json::Value::Null,
            baked: None,
        };
        surface.resize(1);
        let first = &surface.stack.layers[0];
        let second = &surface.stack.layers[1];
        assert_eq!(
            first.channels[0].as_deref().unwrap().as_slice(),
            [255, 0, 0, 128]
        );
        assert_eq!(first.mask.as_ref().unwrap().pixels.as_slice(), [128; 4]);
        assert!(Arc::ptr_eq(
            first.channels[0].as_ref().unwrap(),
            second.channels[0].as_ref().unwrap()
        ));
        assert_eq!(surface.stack.selected, 1);
        assert_eq!(surface.base[0].as_slice(), [9; 4]);
    }
}
