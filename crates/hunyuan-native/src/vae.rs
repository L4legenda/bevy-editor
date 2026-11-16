//! ShapeVAE decoder with cached cross-attention K/V and adaptive grid evaluation.
use crate::tensor::{Weights, attention, check_cancel, mlp};
use anyhow::{Result, ensure};
use candle_core::{DType, Tensor};
use std::sync::atomic::AtomicBool;

pub fn decode(w: &Weights, latent: &Tensor, cancelled: &AtomicBool) -> Result<Tensor> {
    let mut x = w.linear("post_kl", latent)?;
    for i in 0..16 {
        check_cancel(cancelled)?;
        let p = format!("transformer.resblocks.{i}");
        let qkv = w
            .linear(
                &format!("{p}.attn.c_qkv"),
                &w.norm(&format!("{p}.ln_1"), &x, 1e-6)?,
            )?
            .reshape((1, 512, 16, 192))?;
        // VAE interleaves Q/K/V per head; DiT uses three contiguous width-sized slices.
        let parts = qkv.chunk(3, 3)?;
        let q = w
            .norm(&format!("{p}.attn.attention.q_norm"), &parts[0], 1e-6)?
            .transpose(1, 2)?
            .contiguous()?;
        let k = w
            .norm(&format!("{p}.attn.attention.k_norm"), &parts[1], 1e-6)?
            .transpose(1, 2)?
            .contiguous()?;
        let v = parts[2].transpose(1, 2)?.contiguous()?;
        x = (&x + w.linear(&format!("{p}.attn.c_proj"), &attention(&q, &k, &v)?)?)?;
        x = (&x
            + mlp(
                w,
                &format!("{p}.mlp"),
                &w.norm(&format!("{p}.ln_2"), &x, 1e-6)?,
            )?)?;
    }
    Ok(x)
}

struct Field<'a> {
    weights: &'a Weights,
    k: Tensor,
    v: Tensor,
}
impl<'a> Field<'a> {
    fn new(weights: &'a Weights, latent: &Tensor) -> Result<Self> {
        let p = "geo_decoder.cross_attn_decoder";
        let kv = weights
            .linear(
                &format!("{p}.attn.c_kv"),
                &weights.norm(&format!("{p}.ln_2"), latent, 1e-6)?,
            )?
            .reshape((1, 512, 16, 128))?;
        let parts = kv.chunk(2, 3)?;
        let k = weights
            .norm(&format!("{p}.attn.attention.k_norm"), &parts[0], 1e-6)?
            .transpose(1, 2)?
            .contiguous()?;
        let v = parts[1].transpose(1, 2)?.contiguous()?;
        Ok(Self { weights, k, v })
    }
    fn query(&self, coords: &[[f32; 3]]) -> Result<Vec<f32>> {
        let mut embedding = Vec::with_capacity(coords.len() * 51);
        for coord in coords {
            embedding.extend_from_slice(coord);
            for trig in 0..2 {
                for axis in coord {
                    for freq in 0..8 {
                        let v = axis * (1u32 << freq) as f32;
                        embedding.push(if trig == 0 { v.sin() } else { v.cos() });
                    }
                }
            }
        }
        let w = self.weights;
        let p = "geo_decoder.cross_attn_decoder";
        let embedding = Tensor::from_vec(embedding, (1, coords.len(), 51), self.k.device())?
            .to_dtype(DType::F16)?;
        let mut x = w.linear("geo_decoder.query_proj", &embedding)?;
        let q = w.linear(
            &format!("{p}.attn.c_q"),
            &w.norm(&format!("{p}.ln_1"), &x, 1e-6)?,
        )?;
        let q = q.reshape((1, coords.len(), 16, 64))?;
        let q = w
            .norm(&format!("{p}.attn.attention.q_norm"), &q, 1e-6)?
            .transpose(1, 2)?
            .contiguous()?;
        x = (&x
            + w.linear(
                &format!("{p}.attn.c_proj"),
                &attention(&q, &self.k, &self.v)?,
            )?)?;
        x = (&x
            + mlp(
                w,
                &format!("{p}.mlp"),
                &w.norm(&format!("{p}.ln_3"), &x, 1e-6)?,
            )?)?;
        let logits = w.linear(
            "geo_decoder.output_proj",
            &w.norm("geo_decoder.ln_post", &x, 1e-5)?,
        )?;
        // The checkpoint predicts positive occupancy inside; Surface Nets uses negative inside.
        Ok(logits
            .flatten_all()?
            .to_dtype(DType::F32)?
            .to_vec1::<f32>()?
            .into_iter()
            .map(|v| -v)
            .collect())
    }
}

fn index(n: usize, x: usize, y: usize, z: usize) -> usize {
    (z * n + y) * n + x
}
fn interpolate(previous: &[f32], old: usize, cell: [usize; 3], f: [f32; 3]) -> f32 {
    let n = old + 1;
    let mut v = 0.;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let weight = if dx == 0 { 1. - f[0] } else { f[0] }
                    * if dy == 0 { 1. - f[1] } else { f[1] }
                    * if dz == 0 { 1. - f[2] } else { f[2] };
                v += previous[index(n, cell[0] + dx, cell[1] + dy, cell[2] + dz)] * weight;
            }
        }
    }
    v
}
pub fn grid(
    w: &Weights,
    latent: &Tensor,
    resolution: u32,
    chunks: u32,
    cancelled: &AtomicBool,
    progress: &dyn Fn(f32, &str),
) -> Result<Vec<f32>> {
    let field = Field::new(w, latent)?;
    let final_res = resolution as usize;
    let mut levels = vec![final_res.min(32)];
    while *levels.last().unwrap() < final_res {
        levels.push((levels.last().unwrap() * 2).min(final_res));
    }
    let mut previous = Vec::<f32>::new();
    let mut old = 0;
    let chunk = (chunks as usize).clamp(128, 1024);
    for (level, &res) in levels.iter().enumerate() {
        let n = res + 1;
        let total = n * n * n;
        let mut values = vec![0f32; total];
        let mut active = vec![true; old * old * old];
        if old > 0 {
            active.fill(false);
            // Evaluate a one-cell halo around every coarse zero crossing. This keeps thin
            // surfaces while avoiding expensive neural evaluation of known empty volume.
            for z in 0..old {
                for y in 0..old {
                    for x in 0..old {
                        let mut lo = f32::INFINITY;
                        let mut hi = f32::NEG_INFINITY;
                        for dz in 0..2 {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    let v = previous[index(old + 1, x + dx, y + dy, z + dz)];
                                    lo = lo.min(v);
                                    hi = hi.max(v);
                                }
                            }
                        }
                        if lo <= 0. && hi >= 0. || lo.abs().min(hi.abs()) < 0.15 {
                            for az in z.saturating_sub(1)..=(z + 1).min(old - 1) {
                                for ay in y.saturating_sub(1)..=(y + 1).min(old - 1) {
                                    for ax in x.saturating_sub(1)..=(x + 1).min(old - 1) {
                                        active[index(old, ax, ay, az)] = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut selected = Vec::new();
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let i = index(n, x, y, z);
                    if old == 0 {
                        selected.push(i);
                        continue;
                    }
                    let coords = [x, y, z].map(|v| v as f32 * old as f32 / res as f32);
                    let cell = coords.map(|v| (v.floor() as usize).min(old - 1));
                    let f = std::array::from_fn(|a| coords[a] - cell[a] as f32);
                    if active[index(old, cell[0], cell[1], cell[2])] {
                        selected.push(i);
                    } else {
                        values[i] = interpolate(&previous, old, cell, f);
                    }
                }
            }
        }
        for (batch, indices) in selected.chunks(chunk).enumerate() {
            check_cancel(cancelled)?;
            let points: Vec<_> = indices
                .iter()
                .map(|&i| {
                    [i % n, i / n % n, i / (n * n)].map(|v| v as f32 / res as f32 * 2.02 - 1.01)
                })
                .collect();
            let logits = field.query(&points)?;
            ensure!(
                logits.iter().all(|v| v.is_finite()),
                "Не удалось вычислить поверхность: нечисловые значения"
            );
            for (&i, v) in indices.iter().zip(logits) {
                values[i] = v;
            }
            if batch % 8 == 0 {
                progress(
                    (level as f32 + (batch * chunk) as f32 / selected.len().max(1) as f32)
                        / levels.len() as f32,
                    "Построение поверхности",
                );
            }
        }
        previous = values;
        old = res;
    }
    progress(1., "Поверхность готова");
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grid_interpolation_preserves_linear_fields() {
        let mut v = Vec::new();
        for z in 0..3 {
            for y in 0..3 {
                for x in 0..3 {
                    v.push(x as f32 + 2. * y as f32 - 3. * z as f32);
                }
            }
        }
        assert!(
            (interpolate(&v, 2, [1, 0, 1], [0.25, 0.5, 0.75]) - (1.25 + 1. - 5.25)).abs() < 1e-6
        );
    }
}
