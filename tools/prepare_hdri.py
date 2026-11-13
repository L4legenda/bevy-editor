"""Bake Poly Haven equirectangular HDRIs into Bevy's diffuse/specular cubemaps.

Offline dependency: pip install numpy zstandard
Usage: python tools/prepare_hdri.py --source PATH_TO_HDRI_DIRECTORY
Only the resulting CC0 maps are shipped; no Material Painter code is used.
"""
import argparse
import math
from pathlib import Path
import struct

import numpy as np
import zstandard


SOURCES = (
    "studio_small_09", "brown_photostudio_02", "christmas_photo_studio_07",
    "empty_warehouse_01", "lebombo", "kloofendal_48d_partly_cloudy_puresky",
    "venice_sunset", "industrial_sunset_02_puresky", "phalzer_forest_01",
    "moonless_golf",
)


def read_hdr(path):
    """Read Radiance RGBE scanlines, preserving scene-linear HDR values."""
    with path.open("rb") as stream:
        assert stream.readline().strip() in (b"#?RADIANCE", b"#?RGBE")
        while stream.readline().strip():
            pass
        axes = stream.readline().split()
        assert axes[0] == b"-Y" and axes[2] == b"+X", axes
        height, width = int(axes[1]), int(axes[3])
        rgbe = np.empty((height, width, 4), dtype=np.uint8)
        for row in rgbe:
            header = stream.read(4)
            assert header[:2] == b"\x02\x02" and int.from_bytes(header[2:], "big") == width
            for channel in range(4):
                offset = 0
                while offset < width:
                    length = stream.read(1)[0]
                    if length > 128:
                        count = length - 128
                        row[offset:offset + count, channel] = stream.read(1)[0]
                    else:
                        count = length
                        assert count > 0
                        row[offset:offset + count, channel] = np.frombuffer(stream.read(count), dtype=np.uint8)
                    offset += count
                assert offset == width
    return np.ldexp(rgbe[..., :3].astype(np.float32), rgbe[..., 3:4].astype(np.int32) - 136)


def normalize(values):
    return values / np.maximum(np.linalg.norm(values, axis=-1, keepdims=True), 1e-20)


def cube_directions(size):
    axis = (np.arange(size, dtype=np.float32) + 0.5) * (2.0 / size) - 1.0
    x, y = np.meshgrid(axis, axis)
    one = np.ones_like(x)
    # KTX/Vulkan cube faces: +X, -X, +Y, -Y, +Z, -Z.
    faces = ((one, -y, -x), (-one, -y, x), (x, one, y),
             (x, -one, -y), (x, -y, one), (-x, -y, -one))
    return normalize(np.stack([np.stack(face, axis=-1) for face in faces]))


def sample_panorama(image, directions):
    height, width = image.shape[:2]
    u = np.arctan2(directions[..., 2], directions[..., 0]) / (2 * math.pi) + 0.5
    v = np.arccos(np.clip(directions[..., 1], -1, 1)) / math.pi
    x, y = u * width - 0.5, np.clip(v * height - 0.5, 0, height - 1)
    x0, y0 = np.floor(x).astype(np.int32), np.floor(y).astype(np.int32)
    tx, ty = (x - x0)[..., None], (y - y0)[..., None]
    top = image[y0, x0 % width] * (1 - tx) + image[y0, (x0 + 1) % width] * tx
    bottom = image[np.minimum(y0 + 1, height - 1), x0 % width] * (1 - tx) + image[np.minimum(y0 + 1, height - 1), (x0 + 1) % width] * tx
    return top * (1 - ty) + bottom * ty


def sh_basis(directions):
    x, y, z = np.moveaxis(directions, -1, 0)
    return np.stack((np.full_like(x, 0.2820947918),
                     0.4886025119 * x, 0.4886025119 * y, 0.4886025119 * z,
                     1.0925484306 * x * y, 1.0925484306 * y * z,
                     0.3153915653 * (3 * z * z - 1), 1.0925484306 * x * z,
                     0.5462742153 * (x * x - y * y)), axis=-1)


def diffuse_cube(image, size=32):
    height, width = image.shape[:2]
    phi = ((np.arange(width) + 0.5) / width - 0.5) * 2 * math.pi
    theta = (np.arange(height) + 0.5) / height * math.pi
    phi, theta = np.meshgrid(phi, theta)
    directions = np.stack((np.sin(theta) * np.cos(phi), np.cos(theta), np.sin(theta) * np.sin(phi)), axis=-1)
    area = np.sin(theta) * (2 * math.pi / width) * (math.pi / height)
    coefficients = np.einsum("yxc,yxk,yx->kc", image, sh_basis(directions), area, optimize=True)
    # Cosine convolution divided by pi: Bevy expects irradiance/pi.
    bands = np.array([1, 2/3, 2/3, 2/3, 1/4, 1/4, 1/4, 1/4, 1/4])
    return np.maximum(sh_basis(cube_directions(size)) @ (coefficients * bands[:, None]), 0)


def source_mips(image):
    result = [image]
    while min(image.shape[:2]) >= 2:
        h, w = image.shape[:2]
        image = image[:h // 2 * 2, :w // 2 * 2].reshape(h // 2, 2, w // 2, 2, 3).mean(axis=(1, 3))
        result.append(image)
    return result


def filtered_sample(mips, directions, lod):
    lod = np.clip(lod, 0, len(mips) - 1)
    lower = lod.astype(np.int32)
    result = np.zeros((*directions.shape[:-1], 3), dtype=np.float32)
    for index, image in enumerate(mips):
        for upper in (False, True):
            mask = (np.minimum(lower + int(upper), len(mips) - 1) == index)
            if mask.any():
                fraction = (lod - lower)[mask]
                weight = fraction if upper else 1 - fraction
                result[mask] += sample_panorama(image, directions[mask]) * weight[:, None]
    return result


def specular_mips(image, size, samples):
    result = [sample_panorama(image, cube_directions(size))]
    sources = source_mips(image)
    level_count = int(math.log2(size)) + 1
    bits = np.arange(samples, dtype=np.uint32)
    inverse = np.zeros(samples, dtype=np.float32)
    for bit in range(32):
        inverse += ((bits >> bit) & 1) * (2.0 ** (-bit - 1))
    azimuth = inverse * 2 * math.pi
    xi = (np.arange(samples, dtype=np.float32) + 0.5) / samples
    for level in range(1, level_count):
        roughness = level / (level_count - 1)
        alpha2 = roughness ** 4
        cos_h = np.sqrt((1 - xi) / (1 + (alpha2 - 1) * xi))
        sin_h = np.sqrt(np.maximum(1 - cos_h ** 2, 0))
        local_h = np.stack((sin_h * np.cos(azimuth), sin_h * np.sin(azimuth), cos_h), axis=-1)
        weights = np.maximum(2 * cos_h ** 2 - 1, 0)
        pdf = alpha2 / (math.pi * (cos_h ** 2 * (alpha2 - 1) + 1) ** 2) / 4
        sample_area = 1 / (samples * pdf)
        normals = cube_directions(size >> level)
        flat = normals.reshape(-1, 3)
        output = np.empty_like(flat)
        for start in range(0, len(flat), 512):
            normal = flat[start:start + 512]
            up = np.zeros_like(normal)
            up[:, 1] = 1
            up[np.abs(normal[:, 1]) > 0.99] = (1, 0, 0)
            tangent = normalize(np.cross(up, normal))
            bitangent = np.cross(normal, tangent)
            half = (tangent[:, None] * local_h[None, :, 0:1]
                    + bitangent[:, None] * local_h[None, :, 1:2]
                    + normal[:, None] * local_h[None, :, 2:3])
            light = 2 * cos_h[None, :, None] * half - normal[:, None]
            texel_area = (2 * math.pi ** 2 / (image.shape[0] * image.shape[1])) * np.maximum(np.sqrt(np.maximum(1 - light[..., 1] ** 2, 0)), 1e-4)
            lod = 0.5 * np.log2(sample_area[None, :] / texel_area)
            color = filtered_sample(sources, light, lod)
            output[start:start + len(normal)] = (color * weights[None, :, None]).sum(axis=1) / weights.sum()
        result.append(output.reshape(normals.shape))
    return result


def write_ktx2(path, mips):
    # Khronos basic DFD: linear BT.709 RGBA16 signed floating point.
    basic = struct.pack("<IHH4B4B8B", 0, 2, 88, 1, 1, 1, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0)
    samples = b"".join(struct.pack("<HBB4BII", channel * 16, 15, name | 0xC0, 0, 0, 0, 0, 0xBF800000, 0x3F800000)
                       for channel, name in enumerate((0, 1, 2, 15)))
    dfd = struct.pack("<I", 92) + basic + samples
    size = mips[0].shape[1]
    dfd_offset = 80 + 24 * len(mips)
    header = b"\xabKTX 20\xbb\r\n\x1a\n" + struct.pack("<13I2Q", 97, 2, size, size, 0, 0, 6, len(mips), 2, dfd_offset, len(dfd), 0, 0, 0, 0)
    data = bytearray(header + bytes(24 * len(mips)) + dfd)
    compressor = zstandard.ZstdCompressor(level=9)
    for level in reversed(range(len(mips))):
        rgb = np.clip(mips[level], 0, 60000)
        rgba = np.concatenate((rgb, np.ones((*rgb.shape[:-1], 1))), axis=-1)
        raw = rgba.astype("<f2").tobytes()
        compressed = compressor.compress(raw)
        data.extend(bytes((-len(data)) % 8))
        struct.pack_into("<3Q", data, 80 + level * 24, len(data), len(compressed), len(raw))
        data.extend(compressed)
    path.write_bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[1] / "assets/hdri")
    parser.add_argument("--size", type=int, default=256)
    parser.add_argument("--samples", type=int, default=256)
    args = parser.parse_args()
    assert args.size > 0 and args.size & (args.size - 1) == 0
    assert args.samples > 0
    args.output.mkdir(parents=True, exist_ok=True)
    for name in SOURCES:
        print(f"Baking {name}", flush=True)
        image = read_hdr(args.source / f"{name}_1k.hdr")
        write_ktx2(args.output / f"{name}.diffuse.ktx2", [diffuse_cube(image)])
        write_ktx2(args.output / f"{name}.specular.ktx2", specular_mips(image, args.size, args.samples))
    print("Finished all ten environments.", flush=True)


if __name__ == "__main__":
    main()
