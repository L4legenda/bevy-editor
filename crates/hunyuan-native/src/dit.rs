//! Hunyuan3DDiT mini: dual stream + single stream flow transformer.
use crate::tensor::{Weights, attention, check_cancel, heads, norm};
use anyhow::Result;
use candle_core::{DType, Tensor};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::StandardNormal;
use std::sync::atomic::AtomicBool;

fn modulation(w: &Weights, p: &str, v: &Tensor, n: usize) -> candle_core::Result<Vec<Tensor>> {
    w.linear(p, &candle_nn::ops::silu(v)?)?
        .unsqueeze(1)?
        .chunk(n, 2)
}
fn modulate(x: &Tensor, m: &[Tensor]) -> candle_core::Result<Tensor> {
    norm(x)?.broadcast_mul(&(&m[1] + 1.)?)?.broadcast_add(&m[0])
}
fn qkv(w: &Weights, p: &str, x: &Tensor) -> candle_core::Result<(Tensor, Tensor, Tensor)> {
    let parts = x.chunk(3, 2)?;
    Ok((
        w.rms(&format!("{p}.query_norm"), &heads(&parts[0], 16)?)?,
        w.rms(&format!("{p}.key_norm"), &heads(&parts[1], 16)?)?,
        heads(&parts[2], 16)?,
    ))
}
fn double(
    w: &Weights,
    p: &str,
    img: Tensor,
    txt: Tensor,
    vec: &Tensor,
) -> candle_core::Result<(Tensor, Tensor)> {
    let im = modulation(w, &format!("{p}.img_mod.lin"), vec, 6)?;
    let tm = modulation(w, &format!("{p}.txt_mod.lin"), vec, 6)?;
    let (iq, ik, iv) = qkv(
        w,
        &format!("{p}.img_attn.norm"),
        &w.linear(&format!("{p}.img_attn.qkv"), &modulate(&img, &im)?)?,
    )?;
    let (tq, tk, tv) = qkv(
        w,
        &format!("{p}.txt_attn.norm"),
        &w.linear(&format!("{p}.txt_attn.qkv"), &modulate(&txt, &tm)?)?,
    )?;
    let a = attention(
        &Tensor::cat(&[tq, iq], 2)?,
        &Tensor::cat(&[tk, ik], 2)?,
        &Tensor::cat(&[tv, iv], 2)?,
    )?;
    let nt = txt.dim(1)?;
    let img = (&img
        + w.linear(
            &format!("{p}.img_attn.proj"),
            &a.narrow(1, nt, img.dim(1)?)?,
        )?
        .broadcast_mul(&im[2])?)?;
    let txt = (&txt
        + w.linear(&format!("{p}.txt_attn.proj"), &a.narrow(1, 0, nt)?)?
            .broadcast_mul(&tm[2])?)?;
    let imlp = w.linear(
        &format!("{p}.img_mlp.2"),
        &w.linear(&format!("{p}.img_mlp.0"), &modulate(&img, &im[3..])?)?
            .gelu()?,
    )?;
    let tmlp = w.linear(
        &format!("{p}.txt_mlp.2"),
        &w.linear(&format!("{p}.txt_mlp.0"), &modulate(&txt, &tm[3..])?)?
            .gelu()?,
    )?;
    Ok((
        (&img + imlp.broadcast_mul(&im[5])?)?,
        (&txt + tmlp.broadcast_mul(&tm[5])?)?,
    ))
}
fn forward(
    w: &Weights,
    latents: &Tensor,
    cond: &Tensor,
    time: f32,
    cancelled: &AtomicBool,
) -> Result<Tensor> {
    let b = latents.dim(0)?;
    let mut embedding = Vec::with_capacity(b * 256);
    for _ in 0..b {
        for trigonometric in 0..2 {
            for i in 0..128 {
                let a = time * 1000. * (-1000f32.ln() * i as f32 / 128.).exp();
                embedding.push(if trigonometric == 0 { a.cos() } else { a.sin() });
            }
        }
    }
    let t = Tensor::from_vec(embedding, (b, 256), latents.device())?.to_dtype(latents.dtype())?;
    let vec = w.linear(
        "time_in.out_layer",
        &candle_nn::ops::silu(&w.linear("time_in.in_layer", &t)?)?,
    )?;
    let mut img = w.linear("latent_in", latents)?;
    let mut txt = w.linear("cond_in", cond)?;
    for i in 0..8 {
        check_cancel(cancelled)?;
        (img, txt) = double(w, &format!("double_blocks.{i}"), img, txt, &vec)?;
    }
    let nt = txt.dim(1)?;
    let mut x = Tensor::cat(&[txt, img], 1)?;
    for i in 0..16 {
        check_cancel(cancelled)?;
        let p = format!("single_blocks.{i}");
        let m = modulation(w, &format!("{p}.modulation.lin"), &vec, 3)?;
        let y = w.linear(&format!("{p}.linear1"), &modulate(&x, &m)?)?;
        let (q, k, v) = qkv(w, &format!("{p}.norm"), &y.narrow(2, 0, 3072)?)?;
        let a = attention(&q, &k, &v)?;
        let y = Tensor::cat(&[a, y.narrow(2, 3072, 4096)?.gelu()?], 2)?;
        x = (&x
            + w.linear(&format!("{p}.linear2"), &y)?
                .broadcast_mul(&m[2])?)?;
    }
    let x = x.narrow(1, nt, 512)?;
    let m = modulation(w, "final_layer.adaLN_modulation.1", &vec, 2)?;
    Ok(w.linear("final_layer.linear", &modulate(&x, &m)?)?)
}
pub fn sample(
    w: &Weights,
    cond: &Tensor,
    steps: u32,
    guidance: f32,
    seed: u64,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f32, &str),
) -> Result<Tensor> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let noise: Vec<f32> = (0..512 * 64).map(|_| rng.sample(StandardNormal)).collect();
    let mut latent = Tensor::from_vec(noise, (1, 512, 64), cond.device())?.to_dtype(DType::F16)?;
    let cond = Tensor::cat(&[cond, &Tensor::zeros_like(cond)?], 0)?;
    // Upstream flow goes from sigma=0 (noise) to sigma=1 (shape), inclusive.
    for i in 0..steps {
        check_cancel(cancelled)?;
        let t = if steps > 1 {
            i as f32 / (steps - 1) as f32
        } else {
            0.
        };
        let next = if i + 1 < steps {
            (i + 1) as f32 / (steps - 1).max(1) as f32
        } else {
            1.
        };
        let prediction = forward(
            w,
            &Tensor::cat(&[&latent, &latent], 0)?,
            &cond,
            t,
            cancelled,
        )?
        .to_dtype(DType::F32)?;
        let pair = prediction.chunk(2, 0)?;
        let prediction = (&pair[1] + ((&pair[0] - &pair[1])? * guidance as f64)?)?;
        latent = (latent.to_dtype(DType::F32)? + (prediction * (next - t) as f64)?)?
            .to_dtype(DType::F16)?;
        progress((i + 1) as f32 / steps as f32, "Генерация формы");
    }
    Ok((latent / 1.0188137142395404)?)
}
