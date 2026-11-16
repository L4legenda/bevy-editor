//! Linear BT.709 RGBA16F cubemaps with lossless, pure-Rust Zstandard encoding.
use crate::{CubeMip, invalid};
use std::{fs, io, path::Path};
fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn u64le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn encode(mips: &[CubeMip]) -> io::Result<Vec<u8>> {
    let Some(base) = mips.first() else {
        return Err(invalid("Empty cubemap"));
    };
    if !base.size.is_power_of_two() || mips.len() > base.size.ilog2() as usize + 1 {
        return Err(invalid("Invalid mip chain"));
    }
    for (level, mip) in mips.iter().enumerate() {
        if mip.size != base.size >> level
            || mip.pixels.len() != 6 * mip.size as usize * mip.size as usize
            || mip.pixels.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(invalid("Invalid cubemap texels"));
        }
    }
    let dfd_offset = 80 + 24 * mips.len() as u32;
    let mut out = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
    for value in [
        97,
        2,
        base.size,
        base.size,
        0,
        0,
        6,
        mips.len() as u32,
        2,
        dfd_offset,
        92,
        0,
        0,
    ] {
        u32le(&mut out, value);
    }
    u64le(&mut out, 0);
    u64le(&mut out, 0);
    out.resize(dfd_offset as usize, 0);
    // Basic DFD, unchanged from the original baker (KHR_DF_MODEL_RGBSDA,
    // linear transfer, BT.709 primaries, signed floating point channel samples).
    u32le(&mut out, 92);
    u32le(&mut out, 0);
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&88u16.to_le_bytes());
    out.extend_from_slice(&[1, 1, 1, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0]);
    for (channel, name) in [0u8, 1, 2, 15].into_iter().enumerate() {
        out.extend_from_slice(&((channel * 16) as u16).to_le_bytes());
        out.extend_from_slice(&[15, name | 0xC0, 0, 0, 0, 0]);
        u32le(&mut out, 0xBF800000);
        u32le(&mut out, 0x3F800000);
    }
    for level in (0..mips.len()).rev() {
        let mip = &mips[level];
        let mut raw = Vec::with_capacity(mip.pixels.len() * 8);
        for pixel in &mip.pixels {
            for value in [pixel[0], pixel[1], pixel[2], 1.] {
                raw.extend_from_slice(
                    &half::f16::from_f64(value.clamp(0., 60000.))
                        .to_bits()
                        .to_le_bytes(),
                );
            }
        }
        let compressed = ruzstd::encoding::compress_to_vec(
            raw.as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        out.resize(out.len().next_multiple_of(8), 0);
        let offset = out.len() as u64;
        for (i, value) in [offset, compressed.len() as u64, raw.len() as u64]
            .into_iter()
            .enumerate()
        {
            let start = 80 + level * 24 + i * 8;
            out[start..start + 8].copy_from_slice(&value.to_le_bytes());
        }
        out.extend(compressed);
    }
    Ok(out)
}
pub fn write(path: &Path, mips: &[CubeMip]) -> io::Result<()> {
    fs::write(path, encode(mips)?)
}
