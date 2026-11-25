//! Library of seamlessly tileable, procedural grayscale textures (height /
//! roughness / mask images for fill layers).
//!
//! Every texture is evaluated in continuous `[0,1)²` UV space with periodic
//! lattice noise, so the result tiles exactly and looks the same at any
//! resolution (a 96 px thumbnail shows the same pattern as a 1024 px render).
//! Height convention: 255 = raised, 0 = recessed, ~128 = base.
//!
//! The noise / voronoi / sampling helpers are `pub(crate)` so the brush tip
//! library can share them.

use rayon::prelude::*;
use std::f32::consts::{PI, TAU};

/// A named procedural texture.
pub struct ProcTexture {
    pub name: &'static str,
    /// UI grouping: "Pattern", "Surface", "Fabric" or "Metal".
    pub category: &'static str,
    generate: fn(u32) -> Vec<f32>,
}

impl ProcTexture {
    /// Renders the texture as `size × size` row-major 8-bit grayscale.
    /// Deterministic and seamlessly tileable.
    pub fn render(&self, size: u32) -> Vec<u8> {
        to_u8((self.generate)(size.max(1)))
    }
}

const fn tex(name: &'static str, category: &'static str, generate: fn(u32) -> Vec<f32>) -> ProcTexture {
    ProcTexture { name, category, generate }
}

pub const TEXTURES: &[ProcTexture] = &[
    tex("Tiles Square", "Pattern", tiles_square),
    tex("Tiles Hexagon", "Pattern", tiles_hexagon),
    tex("Bricks", "Pattern", bricks),
    tex("Bricks Herringbone", "Pattern", herringbone),
    tex("Planks", "Surface", planks),
    tex("Cobblestone", "Surface", cobblestone),
    tex("Diamond Plate", "Metal", diamond_plate),
    tex("Perforated Holes", "Metal", perforated),
    tex("Carbon Fiber", "Fabric", carbon_fiber),
    tex("Knurling", "Metal", knurling),
    tex("Fabric Weave", "Fabric", fabric_weave),
    tex("Canvas", "Fabric", canvas),
    tex("Scales", "Pattern", scales),
    tex("Corrugated", "Metal", corrugated),
    tex("Stripes", "Pattern", stripes),
    tex("Checker", "Pattern", checker),
    tex("Dots Grid", "Pattern", dots_grid),
    tex("Hex Mesh", "Metal", hex_mesh),
    tex("Noise Fine", "Surface", noise_fine),
    tex("Noise Coarse", "Surface", noise_coarse),
    tex("Cracks", "Surface", cracks),
    tex("Grunge", "Surface", grunge),
    tex("Scratches", "Surface", scratches),
    tex("Leather Grain", "Surface", leather),
    tex("Wood Grain", "Surface", wood_grain),
];

// ---------------------------------------------------------------------------
// Shared math / noise helpers
// ---------------------------------------------------------------------------

pub(crate) fn to_u8(v: Vec<f32>) -> Vec<u8> {
    v.into_iter().map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
}

#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Anti-aliased coverage of a signed distance (`d > 0` inside) for a pixel
/// footprint `w`.
#[inline]
pub(crate) fn aa(d: f32, w: f32) -> f32 {
    (d / w + 0.5).clamp(0.0, 1.0)
}

/// Rounded bevel profile: 0 at the edge (`d <= 0`), easing to 1 at `d >= w`.
/// Never sharper than one pixel (`px`), so edges stay anti-aliased.
#[inline]
pub(crate) fn bevel(d: f32, w: f32, px: f32) -> f32 {
    let t = ((d + 0.5 * px) / (w.max(0.0) + px)).clamp(0.0, 1.0);
    t * (2.0 - t)
}

#[inline]
fn mix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

#[inline]
pub(crate) fn hash(x: i32, y: i32, seed: u32) -> u32 {
    mix32(mix32(mix32(seed.wrapping_add(0x9e37_79b9)) ^ x as u32) ^ (y as u32).wrapping_mul(0x2545_f491))
}

/// Uniform random in `[0,1)` for a lattice point.
#[inline]
pub(crate) fn rand01(x: i32, y: i32, seed: u32) -> f32 {
    (hash(x, y, seed) >> 8) as f32 * (1.0 / 16_777_216.0)
}

const GRAD: [(f32, f32); 16] = [
    (1.0, 0.0),
    (0.92388, 0.38268),
    (0.70711, 0.70711),
    (0.38268, 0.92388),
    (0.0, 1.0),
    (-0.38268, 0.92388),
    (-0.70711, 0.70711),
    (-0.92388, 0.38268),
    (-1.0, 0.0),
    (-0.92388, -0.38268),
    (-0.70711, -0.70711),
    (-0.38268, -0.92388),
    (0.0, -1.0),
    (0.38268, -0.92388),
    (0.70711, -0.70711),
    (0.92388, -0.38268),
];

/// Periodic gradient noise in roughly `[-1,1]`. The lattice wraps every
/// `px` cells in x and `py` cells in y.
pub(crate) fn pnoise(x: f32, y: f32, px: i32, py: i32, seed: u32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let ix = x0 as i32;
    let iy = y0 as i32;
    let ix0 = ix.rem_euclid(px);
    let ix1 = (ix + 1).rem_euclid(px);
    let iy0 = iy.rem_euclid(py);
    let iy1 = (iy + 1).rem_euclid(py);
    let g = |cx: i32, cy: i32, dx: f32, dy: f32| {
        let (gx, gy) = GRAD[(hash(cx, cy, seed) & 15) as usize];
        gx * dx + gy * dy
    };
    let n00 = g(ix0, iy0, fx, fy);
    let n10 = g(ix1, iy0, fx - 1.0, fy);
    let n01 = g(ix0, iy1, fx, fy - 1.0);
    let n11 = g(ix1, iy1, fx - 1.0, fy - 1.0);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let u = fade(fx);
    let v = fade(fy);
    lerp(lerp(n00, n10, u), lerp(n01, n11, u), v) * 1.41
}

/// Tileable noise over UV space with `fx × fy` lattice cells across the tile.
#[inline]
pub(crate) fn tnoise(u: f32, v: f32, fx: i32, fy: i32, seed: u32) -> f32 {
    pnoise(u * fx as f32, v * fy as f32, fx, fy, seed)
}

/// Tileable fBm over UV space (roughly `[-1,1]`).
pub(crate) fn tfbm(u: f32, v: f32, fx: i32, fy: i32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let (mut fx, mut fy) = (fx, fy);
    for o in 0..octaves {
        sum += amp * tnoise(u, v, fx, fy, seed.wrapping_add(o * 1013));
        norm += amp;
        amp *= 0.5;
        fx *= 2;
        fy *= 2;
    }
    sum / norm * 1.5
}

/// Result of a voronoi lookup.
pub(crate) struct Cell {
    /// Distance to the nearest feature point (cell units).
    pub f1: f32,
    /// Exact distance to the nearest cell border (cell units).
    pub border: f32,
    /// Stable per-cell id (wrapped, so it tiles).
    pub id: u32,
}

/// Periodic voronoi with `n × n` cells per period; `(x, y)` in cell units.
/// Border distance uses the two-pass method so lines have uniform width.
pub(crate) fn voronoi(x: f32, y: f32, n: i32, jitter: f32, seed: u32) -> Cell {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = x - ix as f32;
    let fy = y - iy as f32;
    let point = |cx: i32, cy: i32| -> (f32, f32) {
        let h = hash(cx.rem_euclid(n), cy.rem_euclid(n), seed);
        let jx = ((h & 0xffff) as f32 / 65535.0 - 0.5) * jitter + 0.5;
        let jy = ((h >> 16) as f32 / 65535.0 - 0.5) * jitter + 0.5;
        (jx, jy)
    };
    let mut md = f32::MAX;
    let mut mr = (0.0, 0.0);
    let mut mg = (0, 0);
    for j in -1..=1 {
        for i in -1..=1 {
            let (ox, oy) = point(ix + i, iy + j);
            let rx = i as f32 + ox - fx;
            let ry = j as f32 + oy - fy;
            let d = rx * rx + ry * ry;
            if d < md {
                md = d;
                mr = (rx, ry);
                mg = (i, j);
            }
        }
    }
    let mut border = f32::MAX;
    for j in -2..=2 {
        for i in -2..=2 {
            let gi = mg.0 + i;
            let gj = mg.1 + j;
            if i == 0 && j == 0 {
                continue;
            }
            let (ox, oy) = point(ix + gi, iy + gj);
            let rx = gi as f32 + ox - fx;
            let ry = gj as f32 + oy - fy;
            let dx = rx - mr.0;
            let dy = ry - mr.1;
            let l2 = dx * dx + dy * dy;
            if l2 > 1e-8 {
                let d = ((mr.0 + rx) * 0.5 * dx + (mr.1 + ry) * 0.5 * dy) / l2.sqrt();
                border = border.min(d);
            }
        }
    }
    Cell {
        f1: md.sqrt(),
        border,
        id: hash((ix + mg.0).rem_euclid(n), (iy + mg.1).rem_euclid(n), seed ^ 0x5bd1_e995),
    }
}

#[inline]
fn id01(id: u32) -> f32 {
    (mix32(id) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Evaluates `f(u, v)` at every pixel center (rows in parallel).
pub(crate) fn par_pixels(size: u32, f: impl Fn(f32, f32) -> f32 + Sync) -> Vec<f32> {
    let s = size as usize;
    let inv = 1.0 / size as f32;
    let mut out = vec![0.0f32; s * s];
    out.par_chunks_mut(s).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) * inv;
        for (x, o) in row.iter_mut().enumerate() {
            *o = f((x as f32 + 0.5) * inv, v);
        }
    });
    out
}

/// Signed distance to a rounded box centred at the origin (negative inside).
#[inline]
pub(crate) fn sd_round_box(x: f32, y: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = x.abs() - hx + r;
    let qy = y.abs() - hy + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// Distance from `p` to segment `a`–`b`, plus the segment parameter.
#[inline]
pub(crate) fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> (f32, f32) {
    let (dx, dy) = (bx - ax, by - ay);
    let l2 = (dx * dx + dy * dy).max(1e-12);
    let t = (((px - ax) * dx + (py - ay) * dy) / l2).clamp(0.0, 1.0);
    let (ex, ey) = (px - ax - dx * t, py - ay - dy * t);
    ((ex * ex + ey * ey).sqrt(), t)
}

/// Pointy-top hexagonal tiling with unit circumradius.
/// Returns (local offset from the hex centre, centre position).
#[inline]
fn hex_cell(x: f32, y: f32) -> ((f32, f32), (f32, f32)) {
    const SX: f32 = 1.732_050_8;
    const SY: f32 = 3.0;
    let m = |a: f32, p: f32| a - p * (a / p).floor();
    let a = (m(x, SX) - SX * 0.5, m(y, SY) - SY * 0.5);
    let b = (m(x - SX * 0.5, SX) - SX * 0.5, m(y - SY * 0.5, SY) - SY * 0.5);
    let q = if a.0 * a.0 + a.1 * a.1 < b.0 * b.0 + b.1 * b.1 { a } else { b };
    (q, (x - q.0, y - q.1))
}

/// Inside distance to the edge of a pointy-top unit hexagon (apothem √3/2).
#[inline]
fn hex_inside(q: (f32, f32)) -> f32 {
    let ax = q.0.abs();
    let ay = q.1.abs();
    0.866_025_4 - ax.max(0.5 * ax + 0.866_025_4 * ay)
}

/// Wrapping f32 canvas for splatting line features into a tileable image.
struct WrapCanvas {
    s: usize,
    data: Vec<f32>,
}

impl WrapCanvas {
    fn new(size: u32) -> Self {
        let s = size as usize;
        Self { s, data: vec![0.0; s * s] }
    }

    /// Tapered line from `a` to `b` (UV units), width `w` at the middle,
    /// max-blended with intensity `k`. Thin lines keep their energy by
    /// widening to a pixel and fading instead.
    fn scratch(&mut self, a: (f32, f32), b: (f32, f32), w: f32, k: f32) {
        let s = self.s as f32;
        let px = 1.0 / s;
        let we = w.max(px * 0.75);
        let k = k * (w / we);
        let x0 = ((a.0.min(b.0) - we) * s - 2.0).floor() as i32;
        let x1 = ((a.0.max(b.0) + we) * s + 2.0).ceil() as i32;
        let y0 = ((a.1.min(b.1) - we) * s - 2.0).floor() as i32;
        let y1 = ((a.1.max(b.1) + we) * s + 2.0).ceil() as i32;
        for py in y0..=y1 {
            let v = (py as f32 + 0.5) * px;
            let row = py.rem_euclid(self.s as i32) as usize * self.s;
            for pxl in x0..=x1 {
                let u = (pxl as f32 + 0.5) * px;
                let (d, t) = seg_dist(u, v, a.0, a.1, b.0, b.1);
                let taper = (4.0 * t * (1.0 - t)).sqrt();
                let c = aa(we * 0.5 * taper - d, px) * k * taper.sqrt();
                let idx = row + pxl.rem_euclid(self.s as i32) as usize;
                if c > self.data[idx] {
                    self.data[idx] = c;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern
// ---------------------------------------------------------------------------

fn tiles_square(size: u32) -> Vec<f32> {
    let n = 4.0;
    let pl = n / size as f32;
    par_pixels(size, |u, v| {
        let (x, y) = (u * n, v * n);
        let (cx, cy) = (x.floor(), y.floor());
        let sd = sd_round_box(x - cx - 0.5, y - cy - 0.5, 0.47, 0.47, 0.05);
        let r = rand01(cx as i32, cy as i32, 11);
        let top = 0.74 + (r - 0.5) * 0.1 + tfbm(u, v, 12, 12, 3, 7) * 0.025;
        let grout = 0.16 + tfbm(u, v, 48, 48, 2, 3) * 0.04;
        lerp(grout, top, bevel(-sd, 0.06, pl))
    })
}

fn tiles_hexagon(size: u32) -> Vec<f32> {
    // 7 hex columns × 4 double-rows: the √3·7 : 12 aspect is within 1% of square.
    let (ax, ay) = (7.0 * 1.732_050_8, 12.0);
    let pl = ay / size as f32;
    par_pixels(size, |u, v| {
        let (q, c) = hex_cell(u * ax, v * ay);
        let d = hex_inside(q) - 0.06;
        let ci = (c.0 / 0.866_025_4).round() as i32;
        let cj = (c.1 / 1.5).round() as i32;
        let r = rand01(ci.rem_euclid(14), cj.rem_euclid(8), 21);
        let top = 0.74 + (r - 0.5) * 0.1 + tfbm(u, v, 12, 12, 3, 9) * 0.025;
        let grout = 0.16 + tfbm(u, v, 48, 48, 2, 4) * 0.04;
        lerp(grout, top, bevel(d, 0.14, pl))
    })
}

fn bricks(size: u32) -> Vec<f32> {
    let px = 1.0 / size as f32;
    par_pixels(size, |u, v| {
        let y = v * 8.0;
        let j = y.floor();
        let xs = u * 4.0 + if (j as i32) & 1 == 1 { 0.5 } else { 0.0 };
        let i = xs.floor();
        let (lx, ly) = ((xs - i - 0.5) / 4.0, (y - j - 0.5) / 8.0);
        let rough = tfbm(u, v, 24, 24, 3, 31) * 0.0025;
        let sd = sd_round_box(lx, ly, 0.125 - 0.006, 0.0625 - 0.006, 0.004) + rough;
        let id = ((i as i32).rem_euclid(4), j as i32);
        let r = rand01(id.0, id.1, 33);
        let pits = smoothstep(0.55, 0.8, tfbm(u, v, 40, 40, 2, 37) * 0.5 + 0.5) * 0.12;
        let top = 0.7 + (r - 0.5) * 0.14 + tfbm(u, v, 16, 16, 4, 35) * 0.05 - pits;
        let mortar = 0.2 + tfbm(u, v, 96, 96, 2, 39) * 0.05;
        lerp(mortar, top, bevel(-sd, 0.012, px))
    })
}

fn herringbone(size: u32) -> Vec<f32> {
    // Bricks are 2×1 units; the pattern repeats every 4 units, the tile holds 2×2 periods.
    let n = 8.0;
    let pl = n / size as f32;
    par_pixels(size, |u, v| {
        let (x, y) = (u * n, v * n);
        let s0 = ((x + y) * 0.5).floor() as i32;
        let t0 = ((x - y) * 0.25).floor() as i32;
        let mut best = -1.0f32;
        let mut best_id = 0u32;
        for s in s0 - 1..=s0 + 2 {
            for t in t0 - 1..=t0 + 1 {
                let axp = (s + 2 * t) as f32;
                let ayp = (s - 2 * t) as f32;
                // horizontal brick [ax, ax+2] × [ay, ay+1], vertical [ax+2, ax+3] × [ay-1, ay+1]
                let rects = [(axp, ayp, axp + 2.0, ayp + 1.0, 0u32), (axp + 2.0, ayp - 1.0, axp + 3.0, ayp + 1.0, 1u32)];
                for (x0, y0, x1, y1, kind) in rects {
                    let d = (x - x0).min(x1 - x).min(y - y0).min(y1 - y);
                    if d > best {
                        best = d;
                        let wx = (axp as i32).rem_euclid(8);
                        let wy = (ayp as i32).rem_euclid(8);
                        best_id = hash(wx, wy, 41 + kind);
                    }
                }
            }
        }
        let r = id01(best_id);
        let rough = tfbm(u, v, 24, 24, 3, 43) * 0.012;
        let d = best - 0.05 + rough;
        let top = 0.7 + (r - 0.5) * 0.14 + tfbm(u, v, 16, 16, 4, 45) * 0.05;
        let mortar = 0.2 + tfbm(u, v, 96, 96, 2, 47) * 0.05;
        lerp(mortar, top, bevel(d, 0.07, pl))
    })
}

fn scales(size: u32) -> Vec<f32> {
    // Rows every 0.5 units, staggered; lower rows overlap the ones below (fish scales).
    let n = 8.0;
    let pl = n / size as f32;
    let radius = 0.62;
    par_pixels(size, |u, v| {
        let (x, y) = (u * n, v * n);
        let jr = (y / 0.5).floor() as i32;
        let mut hit: Option<(f32, i32, i32)> = None;
        for j in (jr - 1..=jr + 2).rev() {
            let cy = j as f32 * 0.5;
            let off = if j.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
            let i = (x - off).round();
            let d = ((x - i - off).powi(2) + (y - cy).powi(2)).sqrt();
            if d < radius {
                hit = Some((d, i as i32, j));
            }
        }
        let Some((d, i, j)) = hit else { return 0.1 };
        let r = rand01(i.rem_euclid(8), j.rem_euclid(16), 51);
        let t = d / radius;
        let body = 0.25 + 0.65 * smoothstep(0.0, 1.0, t).powf(0.8);
        let rim = bevel(radius - d, 0.06, pl);
        let ridges = 0.03 * (t * 28.0).sin() * t;
        (body + ridges + (r - 0.5) * 0.08) * rim + (1.0 - rim) * 0.1
    })
}

fn stripes(size: u32) -> Vec<f32> {
    let k = 8.0;
    let pl = k * 2.0f32.sqrt() / size as f32;
    par_pixels(size, |u, v| {
        let t = ((u + v) * k).fract();
        let d = 0.25 - (t - 0.5).abs(); // >0 on the raised band
        let w = 0.06;
        0.2 + 0.6 * smoothstep(-w - pl * 0.5, w + pl * 0.5, d)
    })
}

fn checker(size: u32) -> Vec<f32> {
    let k = 8.0;
    let pl = k / size as f32;
    par_pixels(size, |u, v| {
        // anti-aliased square wave per axis; the product is the checker
        let wave = |t: f32| {
            let sign = if (t.floor() as i32) & 1 == 0 { 1.0 } else { -1.0 };
            let f = t.fract();
            sign * (2.0 * aa(f.min(1.0 - f), pl) - 1.0)
        };
        0.5 + 0.35 * wave(u * k) * wave(v * k)
    })
}

fn dots_grid(size: u32) -> Vec<f32> {
    let k = 10.0;
    let pl = k / size as f32;
    par_pixels(size, |u, v| {
        let dx = (u * k).fract() - 0.5;
        let dy = (v * k).fract() - 0.5;
        let r = 0.3;
        let d = (dx * dx + dy * dy).sqrt();
        let dome = (1.0 - (d / r).powi(2)).max(0.0).sqrt();
        let cov = aa(r - d, pl);
        0.35 + 0.6 * dome * cov
    })
}

// ---------------------------------------------------------------------------
// Metal
// ---------------------------------------------------------------------------

fn diamond_plate(size: u32) -> Vec<f32> {
    let n = 6.0;
    let pl = n / size as f32;
    par_pixels(size, |u, v| {
        let (x, y) = (u * n, v * n);
        let (i, j) = (x.floor() as i32, y.floor() as i32);
        let (qx, qy) = (x.fract() - 0.5, y.fract() - 0.5);
        let flip = if (i + j) & 1 == 0 { 1.0 } else { -1.0 };
        let a = (qx + flip * qy) * std::f32::consts::FRAC_1_SQRT_2;
        let b = (qx - flip * qy) * std::f32::consts::FRAC_1_SQRT_2;
        // lens: half-length L, half-width W at the centre, parabolic sides
        let (l, w) = (0.4, 0.085);
        let t = (a / l).clamp(-1.0, 1.0);
        let half = w * (1.0 - t * t);
        let sd = (b.abs() - half).max(a.abs() - l) * 0.85;
        let plate = 0.4 + tfbm(u, v, 32, 32, 3, 61) * 0.02;
        let ridge = bevel(-sd, 0.035, pl);
        let crown = 0.04 * (1.0 - (b.abs() / w.max(1e-3)).min(1.0));
        lerp(plate, 0.86 + crown, ridge)
    })
}

fn perforated(size: u32) -> Vec<f32> {
    let n = 8.0;
    let pl = n / size as f32;
    par_pixels(size, |u, v| {
        let (x, y) = (u * n, v * n);
        let jr = y.floor() as i32;
        let mut dmin = f32::MAX;
        for j in jr - 1..=jr + 1 {
            let off = if j.rem_euclid(2) == 1 { 0.5 } else { 0.0 };
            let cx = (x - off - 0.5).round() + off + 0.5;
            let cy = j as f32 + 0.5;
            dmin = dmin.min(((x - cx).powi(2) + (y - cy).powi(2)).sqrt());
        }
        let r = 0.3;
        let plate = 0.72 + tfbm(u, v, 24, 24, 3, 71) * 0.02;
        lerp(0.02, plate, bevel(dmin - r, 0.05, pl))
    })
}

fn knurling(size: u32) -> Vec<f32> {
    let k = 14.0;
    par_pixels(size, |u, v| {
        let tri = |t: f32| 1.0 - 2.0 * (t.fract() - 0.5).abs();
        let a = tri((u + v) * k);
        let b = tri((u - v) * k + 100.0);
        let h = (a.min(b) * 1.12).min(1.0);
        let wear = tfbm(u, v, 16, 16, 3, 81) * 0.04;
        0.08 + 0.85 * h * (1.0 - 0.15 * (1.0 - h)) + wear * h
    })
}

fn corrugated(size: u32) -> Vec<f32> {
    let k = 10.0;
    par_pixels(size, |u, v| {
        let w = 0.5 + 0.5 * (u * k * TAU).cos();
        let dent = tfbm(u, v, 4, 4, 4, 91) * 0.03;
        0.12 + 0.76 * w + dent
    })
}

fn hex_mesh(size: u32) -> Vec<f32> {
    let (ax, ay) = (7.0 * 1.732_050_8, 12.0);
    let pl = ay / size as f32;
    par_pixels(size, |u, v| {
        let (q, _) = hex_cell(u * ax, v * ay);
        let d = hex_inside(q) - 0.13; // >0 inside the hole
        let frame = 0.85 + tfbm(u, v, 24, 24, 2, 101) * 0.02;
        lerp(frame, 0.02, bevel(d, 0.1, pl))
    })
}

// ---------------------------------------------------------------------------
// Fabric
// ---------------------------------------------------------------------------

/// Plain weave: `n` threads per tile, `irregular` adds thickness variation.
fn weave(u: f32, v: f32, n: f32, irregular: f32, seed: u32) -> f32 {
    let (x, y) = (u * n, v * n);
    let (i, j) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x.fract(), y.fract());
    // per-thread thickness along its length (periodic 1D noise)
    let thick_h = 0.82 + irregular * pnoise(x * 0.35, j as f32 * 7.3, (n * 0.35) as i32, 1 << 16, seed);
    let thick_v = 0.82 + irregular * pnoise(y * 0.35, i as f32 * 7.3, (n * 0.35) as i32, 1 << 16, seed + 1);
    let bump = |f: f32, th: f32| {
        let d = (f - 0.5).abs() * 2.0 / th.clamp(0.4, 1.0);
        (1.0 - d * d).max(0.0).sqrt()
    };
    let h_over = ((i + j).rem_euclid(2)) == 0;
    let arc = |f: f32| (f * PI).sin();
    // horizontal thread runs along x: across = fy, along = fx
    let twist_h = 0.9 + 0.1 * ((fx * 2.0 + fy) * TAU).sin();
    let twist_v = 0.9 + 0.1 * ((fy * 2.0 + fx) * TAU).sin();
    let hh = bump(fy, thick_h) * twist_h;
    let hv = bump(fx, thick_v) * twist_v;
    let (top, under) = if h_over { (hh * (0.6 + 0.4 * arc(fx)), hv * 0.45) } else { (hv * (0.6 + 0.4 * arc(fy)), hh * 0.45) };
    top.max(under)
}

fn fabric_weave(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| {
        let w = weave(u, v, 32.0, 0.08, 111);
        let fuzz = tfbm(u, v, 64, 64, 2, 113) * 0.04;
        0.08 + 0.85 * w + fuzz
    })
}

fn canvas(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| {
        let w = weave(u, v, 20.0, 0.22, 121);
        let slub = tfbm(u, v, 8, 8, 3, 123) * 0.06;
        let fuzz = tfbm(u, v, 96, 96, 2, 125) * 0.05;
        0.1 + 0.8 * w + slub + fuzz
    })
}

fn carbon_fiber(size: u32) -> Vec<f32> {
    // 2×2 twill: each tow passes over two cells, shifting by one per row.
    let n = 16.0;
    par_pixels(size, |u, v| {
        let (x, y) = (u * n, v * n);
        let (i, j) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x.fract(), y.fract());
        let phase = (i + j).rem_euclid(4);
        let bump = |f: f32| {
            let d = (f - 0.5).abs() * 2.0 / 0.92;
            (1.0 - d * d).max(0.0).powf(0.35)
        };
        let fibers = |f: f32| 0.88 + 0.12 * (f * TAU * 7.0).cos().abs();
        let (across, run_t, along_f) = if phase < 2 {
            (fy, (phase as f32 + fx) / 2.0, fy)
        } else {
            (fx, ((phase - 2) as f32 + fy) / 2.0, fx)
        };
        let run = (run_t * PI).sin().powf(0.4);
        0.1 + 0.85 * bump(across) * (0.55 + 0.45 * run) * fibers(along_f)
    })
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

fn planks(size: u32) -> Vec<f32> {
    let rows = 6.0;
    let px = 1.0 / size as f32;
    par_pixels(size, |u, v| {
        let y = v * rows;
        let j = y.floor() as i32;
        let fy = y.fract();
        let o = rand01(j, 0, 131);
        let split = 0.5 + (rand01(j, 1, 131) - 0.5) * 0.3;
        let xa = (u - o).rem_euclid(1.0);
        let board = if xa < split { 0 } else { 1 };
        let dx = xa.min((xa - split).abs()).min(1.0 - xa);
        let dy = (0.5 - (fy - 0.5).abs()) / rows;
        let d = dx.min(dy) - 0.0025;
        let r = rand01(j, board, 133);
        // grain: stretched periodic noise per board, rings warped by it
        let shift = r * 13.0;
        let warp = tfbm(u + shift, v, 2, 12, 3, 135);
        let rings = 0.5 + 0.5 * ((fy * 2.2 + warp * 0.9 + r * 5.0) * TAU * 1.5).sin();
        let pores = tfbm(u + shift, v, 6, 96, 2, 137) * 0.5 + 0.5;
        let top = 0.66 + (r - 0.5) * 0.08 + rings * 0.07 - pores * 0.06;
        lerp(0.12, top, bevel(d, 0.006, px))
    })
}

fn cobblestone(size: u32) -> Vec<f32> {
    let n = 6;
    let pl = n as f32 / size as f32;
    par_pixels(size, |u, v| {
        let wu = u + tfbm(u, v, 6, 6, 3, 141) * 0.012;
        let wv = v + tfbm(u, v, 6, 6, 3, 143) * 0.012;
        let c = voronoi(wu * n as f32, wv * n as f32, n, 0.85, 145);
        let r = id01(c.id);
        let gap = 0.045;
        let t = smoothstep(gap, gap + 0.32, c.border);
        let dome = 1.0 - (1.0 - t) * (1.0 - t);
        let top = 0.62 + r * 0.22 + tfbm(u, v, 24, 24, 4, 147) * 0.05 - c.f1 * 0.06;
        let stone = top * (0.35 + 0.65 * dome);
        let gravel = 0.1 + tfbm(u, v, 64, 64, 2, 149) * 0.05;
        lerp(gravel, stone, aa(c.border - gap, pl))
    })
}

fn noise_fine(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| 0.5 + 0.42 * tfbm(u, v, 48, 48, 3, 151))
}

fn noise_coarse(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| 0.5 + 0.45 * tfbm(u, v, 4, 4, 6, 161))
}

fn cracks(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| {
        let wu = u + tfbm(u, v, 5, 5, 4, 171) * 0.03;
        let wv = v + tfbm(u, v, 5, 5, 4, 173) * 0.03;
        let base = 0.62 + tfbm(u, v, 16, 16, 4, 175) * 0.06;
        let mut h = base;
        // main cracks
        let n1 = 5;
        let c1 = voronoi(wu * n1 as f32, wv * n1 as f32, n1, 0.9, 177);
        let pl1 = n1 as f32 / size as f32;
        let w1 = (0.012 + 0.01 * tnoise(u, v, 10, 10, 179)).max(0.0);
        let keep1 = smoothstep(-0.45, -0.1, tnoise(u, v, 7, 7, 181));
        let k1 = aa(w1 - c1.border, pl1) * keep1;
        let chip1 = (-c1.border / 0.06).exp() * keep1 * 0.18;
        // fine secondary cracks
        let n2 = 13;
        let c2 = voronoi(wu * n2 as f32, wv * n2 as f32, n2, 0.9, 183);
        let pl2 = n2 as f32 / size as f32;
        let w2 = 0.012 + 0.012 * tnoise(u, v, 20, 20, 185);
        let keep2 = smoothstep(0.0, 0.35, tnoise(u, v, 9, 9, 187));
        let k2 = aa(w2 - c2.border, pl2) * keep2;
        h -= chip1;
        h = lerp(h, 0.3, k2 * 0.8);
        lerp(h, 0.04, k1)
    })
}

fn grunge(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| {
        let stains = smoothstep(-0.25, 0.55, tfbm(u, v, 3, 3, 6, 191));
        let blotch = smoothstep(0.2, 0.6, tfbm(u + 0.3, v, 8, 8, 4, 193));
        let spots = smoothstep(0.5, 0.8, tfbm(u, v, 40, 40, 2, 195));
        let fine = tfbm(u, v, 64, 64, 2, 197) * 0.06;
        0.28 + stains * 0.42 + blotch * 0.18 - spots * 0.2 + fine
    })
}

fn scratches(size: u32) -> Vec<f32> {
    let mut c = WrapCanvas::new(size);
    let seed = 201;
    for i in 0..260 {
        let r = |k: i32| rand01(i, k, seed);
        let (cx, cy) = (r(0), r(1));
        // two dominant directions plus random ones, like handling wear
        let ang = if r(2) < 0.7 { 0.35 + (r(3) - 0.5) * 0.5 } else { r(3) * PI };
        let len = 0.03 + 0.22 * r(4).powi(2);
        let (dx, dy) = (ang.cos() * len * 0.5, ang.sin() * len * 0.5);
        let w = 0.0008 + 0.0022 * r(5).powi(2);
        let k = 0.35 + 0.65 * r(6);
        c.scratch((cx - dx, cy - dy), (cx + dx, cy + dy), w, k);
    }
    let base = par_pixels(size, |u, v| 0.55 + tfbm(u, v, 32, 32, 2, 203) * 0.02);
    base.into_iter().zip(c.data).map(|(b, s)| b - s * 0.5).collect()
}

fn leather(size: u32) -> Vec<f32> {
    let n = 26;
    let pl = n as f32 / size as f32;
    par_pixels(size, |u, v| {
        let wu = u + tfbm(u, v, 8, 8, 3, 211) * 0.008;
        let wv = v + tfbm(u, v, 8, 8, 3, 213) * 0.008;
        let c = voronoi(wu * n as f32, wv * n as f32, n, 0.95, 215);
        let t = smoothstep(0.0, 0.32 + pl, c.border);
        let pebble = t.powf(0.6) * (0.85 + 0.15 * id01(c.id));
        // long soft creases
        let cr = tfbm(u, v, 3, 3, 4, 217);
        let crease = (1.0 - (cr.abs() * 9.0).min(1.0)).powi(2);
        let fold = tfbm(u, v, 2, 2, 3, 219) * 0.06;
        0.2 + 0.62 * pebble - crease * 0.22 + fold
    })
}

fn wood_grain(size: u32) -> Vec<f32> {
    par_pixels(size, |u, v| {
        let warp = tfbm(u, v, 2, 3, 4, 221);
        let t = v * 9.0 + warp * 1.6 + tfbm(u, v, 1, 2, 2, 223) * 0.6;
        // `fract` keeps the sign; negative t (near v = 0) must wrap too.
        let f = t.rem_euclid(1.0);
        // gentle early wood, sharp late-wood band
        let ring = smoothstep(0.0, 0.75, f) * (1.0 - smoothstep(0.85, 1.0, f));
        let pores = tfbm(u, v, 4, 128, 2, 225) * 0.5 + 0.5;
        let streak = tfbm(u, v, 2, 48, 3, 227);
        0.35 + 0.35 * ring + streak * 0.08 - smoothstep(0.55, 0.85, pores) * 0.12
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col_diff(img: &[u8], s: usize, a: usize, b: usize) -> f32 {
        (0..s).map(|y| (img[y * s + a] as f32 - img[y * s + b] as f32).abs()).sum::<f32>() / s as f32
    }

    fn row_diff(img: &[u8], s: usize, a: usize, b: usize) -> f32 {
        (0..s).map(|x| (img[a * s + x] as f32 - img[b * s + x] as f32).abs()).sum::<f32>() / s as f32
    }

    #[test]
    fn textures_render_and_are_deterministic() {
        assert!(TEXTURES.len() >= 20);
        for t in TEXTURES {
            let a = t.render(64);
            assert_eq!(a.len(), 64 * 64, "{}", t.name);
            assert_eq!(a, t.render(64), "{} not deterministic", t.name);
            let (mn, mx) = (a.iter().min().unwrap(), a.iter().max().unwrap());
            assert!(mx - mn > 20, "{} is flat", t.name);
        }
    }

    #[test]
    fn textures_tile() {
        let s = 256;
        for t in TEXTURES {
            let img = t.render(s as u32);
            // The wrap may fall on a cell edge, so it is compared with the
            // largest jump between neighboring interior columns/rows.
            let interior_c = (1..s - 1).map(|x| col_diff(&img, s, x, x + 1)).fold(0.0f32, f32::max);
            let interior_r = (1..s - 1).map(|y| row_diff(&img, s, y, y + 1)).fold(0.0f32, f32::max);
            let seam_c = col_diff(&img, s, 0, s - 1);
            let seam_r = row_diff(&img, s, 0, s - 1);
            assert!(seam_c <= interior_c * 1.1 + 2.0, "{} column seam {seam_c} vs {interior_c}", t.name);
            assert!(seam_r <= interior_r * 1.1 + 2.0, "{} row seam {seam_r} vs {interior_r}", t.name);
        }
    }

    #[test]
    fn herringbone_covers_plane_once() {
        // every sample point lies inside exactly one brick
        for k in 0..400 {
            let x = rand01(k, 0, 9) * 8.0;
            let y = rand01(k, 1, 9) * 8.0;
            let s0 = ((x + y) * 0.5).floor() as i32;
            let t0 = ((x - y) * 0.25).floor() as i32;
            let mut count = 0;
            for s in s0 - 3..=s0 + 3 {
                for t in t0 - 3..=t0 + 3 {
                    let ax = (s + 2 * t) as f32;
                    let ay = (s - 2 * t) as f32;
                    if x > ax && x < ax + 2.0 && y > ay && y < ay + 1.0 {
                        count += 1;
                    }
                    if x > ax + 2.0 && x < ax + 3.0 && y > ay - 1.0 && y < ay + 1.0 {
                        count += 1;
                    }
                }
            }
            assert_eq!(count, 1, "point {x},{y}");
        }
    }

    #[test]
    #[ignore]
    fn timing_1024() {
        for t in TEXTURES {
            let start = std::time::Instant::now();
            let _ = t.render(1024);
            println!("{:<20} {:>7.1} ms", t.name, start.elapsed().as_secs_f64() * 1000.0);
        }
    }

    /// Writes a contact sheet of all textures plus 2×2 tiled seam checks.
    #[test]
    #[ignore]
    fn texture_contact_sheet() {
        let dir = std::env::var("SHEET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let cell = 192u32;
        let cols = 5u32;
        let rows = (TEXTURES.len() as u32).div_ceil(cols);
        let gap = 6;
        let mut sheet = image::GrayImage::from_pixel(cols * (cell + gap) + gap, rows * (cell + gap) + gap, image::Luma([40]));
        for (i, t) in TEXTURES.iter().enumerate() {
            let img = t.render(cell);
            let (ox, oy) = (gap + (i as u32 % cols) * (cell + gap), gap + (i as u32 / cols) * (cell + gap));
            for y in 0..cell {
                for x in 0..cell {
                    sheet.put_pixel(ox + x, oy + y, image::Luma([img[(y * cell + x) as usize]]));
                }
            }
        }
        sheet.save(dir.join("textures_sheet.png")).unwrap();
        // 2×2 tiling at 256 px per tile for seam inspection
        let s = 256u32;
        let cols = 5u32;
        let tile_cell = s; // each item shown as 2×2 of 128 px
        let half = s / 2;
        let rows = (TEXTURES.len() as u32).div_ceil(cols);
        let mut tiled = image::GrayImage::from_pixel(cols * (tile_cell + gap) + gap, rows * (tile_cell + gap) + gap, image::Luma([40]));
        for (i, t) in TEXTURES.iter().enumerate() {
            let img = t.render(half);
            let (ox, oy) = (gap + (i as u32 % cols) * (tile_cell + gap), gap + (i as u32 / cols) * (tile_cell + gap));
            for y in 0..s {
                for x in 0..s {
                    let p = img[((y % half) * half + (x % half)) as usize];
                    tiled.put_pixel(ox + x, oy + y, image::Luma([p]));
                }
            }
        }
        tiled.save(dir.join("textures_tiled.png")).unwrap();
        // one large render for detail inspection
        for name in ["Bricks", "Carbon Fiber", "Leather Grain", "Wood Grain"] {
            let t = TEXTURES.iter().find(|t| t.name == name).unwrap();
            let img = t.render(512);
            image::GrayImage::from_raw(512, 512, img).unwrap().save(dir.join(format!("tex_{}.png", name.replace(' ', "_")))).unwrap();
        }
    }
}
