//! Hunyuan3D-2mini inference primitives. The checkpoint is kept in host RAM;
//! only the current stage occupies CUDA memory (also works on an 8 GB GPU).
use anyhow::{Context, Result, ensure};
use candle_core::{D, DType, Device, Tensor};
use candle_nn::{LayerNorm, Linear, Module};
use std::{
    collections::HashMap,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct Weights(pub HashMap<String, Tensor>);
impl Weights {
    pub fn load(path: &Path, cancelled: &AtomicBool, progress: &dyn Fn(f32, &str)) -> Result<Self> {
        // The file is read-only and owned by the model cache for the worker lifetime.
        let file = unsafe { candle_core::safetensors::MmapedSafetensors::new(path) }?;
        let mut names: Vec<_> = file
            .tensors()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|n| {
                n.starts_with("model.")
                    || n.starts_with("conditioner.main_image_encoder.model.")
                    || n.starts_with("vae.post_kl.")
                    || n.starts_with("vae.transformer.")
                    || n.starts_with("vae.geo_decoder.")
            })
            .collect();
        names.sort();
        let count = names.len();
        let mut weights = HashMap::with_capacity(count);
        for (i, name) in names.into_iter().enumerate() {
            check_cancel(cancelled)?;
            let t = file
                .load(&name, &Device::Cpu)
                .with_context(|| name.clone())?;
            weights.insert(name, t);
            if i % 16 == 0 {
                progress(i as f32 / count as f32, "Загрузка весов в память");
            }
        }
        let weights = Self(weights);
        weights.validate()?;
        progress(1., "Модель загружена");
        Ok(weights)
    }
    fn validate(&self) -> Result<()> {
        let shape = |name: &str, expected: &[usize]| -> Result<()> {
            let tensor = self
                .get(name)
                .with_context(|| "Неполный checkpoint Hunyuan3D-2mini")?;
            ensure!(
                tensor.dims() == expected,
                "Неподдерживаемый checkpoint: {name}, ожидается {expected:?}, получено {:?}",
                tensor.dims()
            );
            Ok(())
        };
        let dino = "conditioner.main_image_encoder.model";
        shape(
            &format!("{dino}.embeddings.patch_embeddings.projection.weight"),
            &[1536, 3, 14, 14],
        )?;
        shape(
            &format!("{dino}.embeddings.position_embeddings"),
            &[1, 1370, 1536],
        )?;
        for i in 0..40 {
            let p = format!("{dino}.encoder.layer.{i}");
            for name in ["query", "key", "value"] {
                shape(
                    &format!("{p}.attention.attention.{name}.weight"),
                    &[1536, 1536],
                )?;
            }
            shape(&format!("{p}.mlp.weights_in.weight"), &[8192, 1536])?;
            shape(&format!("{p}.mlp.weights_out.weight"), &[1536, 4096])?;
        }
        shape("model.latent_in.weight", &[1024, 64])?;
        shape("model.cond_in.weight", &[1024, 1536])?;
        for i in 0..8 {
            for stream in ["img", "txt"] {
                let p = format!("model.double_blocks.{i}.{stream}");
                shape(&format!("{p}_attn.qkv.weight"), &[3072, 1024])?;
                shape(&format!("{p}_mod.lin.weight"), &[6144, 1024])?;
            }
        }
        for i in 0..16 {
            shape(
                &format!("model.single_blocks.{i}.linear1.weight"),
                &[7168, 1024],
            )?;
            shape(
                &format!("model.single_blocks.{i}.linear2.weight"),
                &[1024, 5120],
            )?;
            shape(
                &format!("vae.transformer.resblocks.{i}.attn.c_qkv.weight"),
                &[3072, 1024],
            )?;
        }
        shape("model.final_layer.linear.weight", &[64, 1024])?;
        shape("vae.post_kl.weight", &[1024, 64])?;
        shape("vae.geo_decoder.query_proj.weight", &[1024, 51])?;
        shape("vae.geo_decoder.output_proj.weight", &[1, 1024])?;
        Ok(())
    }
    pub fn stage(&self, prefix: &str, device: &Device, cancelled: &AtomicBool) -> Result<Self> {
        let mut out = HashMap::new();
        for (name, tensor) in &self.0 {
            if let Some(local) = name.strip_prefix(prefix) {
                check_cancel(cancelled)?;
                out.insert(
                    local.to_owned(),
                    tensor.to_device(device)?.to_dtype(DType::F16)?,
                );
            }
        }
        Ok(Self(out))
    }
    pub fn get(&self, name: &str) -> candle_core::Result<&Tensor> {
        self.0
            .get(name)
            .ok_or_else(|| candle_core::Error::Msg(format!("Missing checkpoint tensor: {name}")))
    }
    pub fn linear(&self, prefix: &str, x: &Tensor) -> candle_core::Result<Tensor> {
        let w = self.get(&format!("{prefix}.weight"))?.clone();
        let b = self.0.get(&format!("{prefix}.bias")).cloned();
        Linear::new(w, b).forward(x)
    }
    pub fn norm(&self, prefix: &str, x: &Tensor, eps: f64) -> candle_core::Result<Tensor> {
        LayerNorm::new(
            self.get(&format!("{prefix}.weight"))?.clone(),
            self.get(&format!("{prefix}.bias"))?.clone(),
            eps,
        )
        .forward(&x.contiguous()?)
    }
    pub fn rms(&self, prefix: &str, x: &Tensor) -> candle_core::Result<Tensor> {
        let xf = x.to_dtype(DType::F32)?;
        let rms = (xf.sqr()?.mean_keepdim(D::Minus1)? + 1e-6)?.sqrt()?;
        xf.broadcast_div(&rms)?
            .to_dtype(x.dtype())?
            .broadcast_mul(self.get(&format!("{prefix}.scale"))?)
    }
}
pub fn check_cancel(cancelled: &AtomicBool) -> Result<()> {
    ensure!(!cancelled.load(Ordering::Relaxed), "Генерация отменена");
    Ok(())
}
pub fn norm(x: &Tensor) -> candle_core::Result<Tensor> {
    let width = x.dim(D::Minus1)?;
    LayerNorm::new(
        Tensor::ones(width, x.dtype(), x.device())?,
        Tensor::zeros(width, x.dtype(), x.device())?,
        1e-6,
    )
    .forward(&x.contiguous()?)
}
pub fn heads(x: &Tensor, n: usize) -> candle_core::Result<Tensor> {
    let (b, l, w) = x.dims3()?;
    x.reshape((b, l, n, w / n))?.transpose(1, 2)?.contiguous()
}
pub fn unheads(x: &Tensor) -> candle_core::Result<Tensor> {
    let (b, h, l, d) = x.dims4()?;
    x.transpose(1, 2)?.contiguous()?.reshape((b, l, h * d))
}
/// Query chunks bound peak memory independently of image/grid resolution.
/// Accumulate softmax in f32, as PyTorch SDPA's fp16 implementation does.
pub fn attention(q: &Tensor, k: &Tensor, v: &Tensor) -> candle_core::Result<Tensor> {
    let (_, _, length, width) = q.dims4()?;
    let kt = k.t()?.contiguous()?;
    let v = v.contiguous()?;
    let mut chunks = Vec::with_capacity(length.div_ceil(128));
    for start in (0..length).step_by(128) {
        let query = q
            .narrow(2, start, (length - start).min(128))?
            .contiguous()?;
        let scores = (query.matmul(&kt)? / (width as f64).sqrt())?.to_dtype(DType::F32)?;
        let probability = candle_nn::ops::softmax_last_dim(&scores)?.to_dtype(q.dtype())?;
        chunks.push(probability.matmul(&v)?);
    }
    unheads(&Tensor::cat(&chunks, 2)?)
}
pub fn mlp(w: &Weights, prefix: &str, x: &Tensor) -> candle_core::Result<Tensor> {
    w.linear(
        &format!("{prefix}.c_proj"),
        &w.linear(&format!("{prefix}.c_fc"), x)?.gelu_erf()?,
    )
}
