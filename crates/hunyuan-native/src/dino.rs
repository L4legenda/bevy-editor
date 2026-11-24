//! DINOv2 ViT-g/14 (40 blocks, SwiGLU), translated from the upstream architecture.
use crate::tensor::{Weights, attention, check_cancel, heads};
use anyhow::{Result, ensure};
use candle_core::{DType, Device, Tensor};
use candle_nn::{LayerNorm, Module};
use image::{
    GenericImageView, ImageReader, RgbImage,
    imageops::{self, FilterType},
};
use std::{path::Path, sync::atomic::AtomicBool};

pub fn prepare(path: &Path, device: &Device) -> Result<Tensor> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    use image::ImageDecoder;
    let orientation = decoder.orientation()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let (width, height) = decoded.dimensions();
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 16_000_000,
        "Слишком большое изображение"
    );
    let rgba = decoded.to_rgba8();
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for (x, y, pixel) in rgba.enumerate_pixels() {
        if pixel[3] > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    ensure!(x0 <= x1 && y0 <= y1, "Изображение полностью прозрачное");
    let crop = imageops::crop_imm(&rgba, x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)).to_image();
    // Preserve the reference processor's 15% border and white alpha composite.
    // Compose at conditioning resolution, never at max(source_width, source_height)^2:
    // a long narrow image must not cause an unbounded square allocation.
    let size = 512;
    let scale = (size as f32 * 0.85).floor() / crop.width().max(crop.height()) as f32;
    let cw = (crop.width() as f32 * scale).max(1.) as u32;
    let ch = (crop.height() as f32 * scale).max(1.) as u32;
    let resized = imageops::resize(&crop, cw, ch, FilterType::Triangle);
    let mut canvas = RgbImage::from_pixel(size, size, image::Rgb([255, 255, 255]));
    let (ox, oy) = ((size - cw) / 2, (size - ch) / 2);
    for (x, y, p) in resized.enumerate_pixels() {
        let a = p[3] as f32 / 255.;
        canvas.put_pixel(
            x + ox,
            y + oy,
            image::Rgb(std::array::from_fn(|i| {
                (p[i] as f32 * a + 255. * (1. - a)).round() as u8
            })),
        );
    }
    let image = imageops::resize(&canvas, 512, 512, FilterType::CatmullRom);
    let image = imageops::resize(&image, 518, 518, FilterType::Triangle);
    let mut values = vec![0f32; 3 * 518 * 518];
    for (x, y, p) in image.enumerate_pixels() {
        for c in 0..3 {
            values[c * 518 * 518 + y as usize * 518 + x as usize] =
                (p[c] as f32 / 255. - [0.485, 0.456, 0.406][c]) / [0.229, 0.224, 0.225][c];
        }
    }
    Ok(Tensor::from_vec(values, (1, 3, 518, 518), device)?.to_dtype(DType::F16)?)
}

pub fn encode(
    weights: &Weights,
    image: &Tensor,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f32, &str),
) -> Result<Tensor> {
    const PREFIX: &str = "conditioner.main_image_encoder.model";
    let device = image.device();
    let embeddings = weights.stage(&format!("{PREFIX}.embeddings."), device, cancelled)?;
    let patch = image
        .conv2d(
            embeddings.get("patch_embeddings.projection.weight")?,
            0,
            14,
            1,
            1,
        )?
        .broadcast_add(
            &embeddings
                .get("patch_embeddings.projection.bias")?
                .reshape((1, 1536, 1, 1))?,
        )?;
    let patch = patch.flatten(2, 3)?.transpose(1, 2)?.contiguous()?;
    let mut x = Tensor::cat(&[embeddings.get("cls_token")?, &patch], 1)?
        .broadcast_add(embeddings.get("position_embeddings")?)?;
    drop(embeddings);
    // DINO is the largest stage. Reuse VRAM for its blocks instead of holding
    // all 40 sets of weights on the device while encoding a single image.
    for i in 0..40 {
        check_cancel(cancelled)?;
        let w = weights.stage(&format!("{PREFIX}.encoder.layer.{i}."), device, cancelled)?;
        let xn = w.norm("norm1", &x, 1e-6)?;
        let q = heads(&w.linear("attention.attention.query", &xn)?, 24)?;
        let k = heads(&w.linear("attention.attention.key", &xn)?, 24)?;
        let v = heads(&w.linear("attention.attention.value", &xn)?, 24)?;
        let a = w
            .linear("attention.output.dense", &attention(&q, &k, &v)?)?
            .broadcast_mul(w.get("layer_scale1.lambda1")?)?;
        x = (&x + a)?;
        let mlp = w.linear("mlp.weights_in", &w.norm("norm2", &x, 1e-6)?)?;
        let halves = mlp.chunk(2, 2)?;
        let mlp = (candle_nn::ops::silu(&halves[0])? * &halves[1])?;
        x = (&x
            + w.linear("mlp.weights_out", &mlp)?
                .broadcast_mul(w.get("layer_scale2.lambda1")?)?)?;
        progress((i + 1) as f32 / 40., "Анализ изображения");
    }
    let norm = weights.stage(&format!("{PREFIX}.layernorm."), device, cancelled)?;
    Ok(
        LayerNorm::new(norm.get("weight")?.clone(), norm.get("bias")?.clone(), 1e-6)
            .forward(&x.contiguous()?)?,
    )
}
