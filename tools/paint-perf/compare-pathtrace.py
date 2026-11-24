"""Compare matched scene-linear path-tracing captures and synchronized frame times."""

import argparse
import json
import math
import statistics
import struct
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def hdr(path, size):
    pixels = [pixel[:3] for pixel in struct.iter_unpack("<eeee", path.read_bytes())]
    if len(pixels) != math.prod(size) or not all(
        math.isfinite(channel) for pixel in pixels for channel in pixel
    ):
        raise ValueError(f"Malformed HDR capture: {path}")
    return pixels


def luminance(pixel):
    r, g, b = pixel
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def region(pixels, size, background):
    mask = Image.new("L", size)
    mask.putdata([
        255 if max(abs(value - base) for value, base in zip(pixel, background)) > 0.05 else 0
        for pixel in pixels
    ])
    mask = mask.filter(ImageFilter.MinFilter(5))
    return mask.get_flattened_data() if hasattr(mask, "get_flattened_data") else mask.getdata()


def neutral(pixel):
    low = min(pixel)
    shift = low - 6.25 * low * low if low < 0.08 else 0.04
    color = [value - shift for value in pixel]
    peak = max(color)
    if peak >= 0.76:
        new_peak = 1.0 - 0.24**2 / (peak + 0.24 - 0.76)
        fade = 1.0 - 1.0 / (1.0 + 0.15 * (peak - new_peak))
        color = [(value * new_peak / peak) * (1.0 - fade) + new_peak * fade for value in color]
    return tuple(round(255 * srgb(value)) for value in color)


def srgb(value):
    value = min(1.0, max(0.0, value))
    return 12.92 * value if value <= 0.0031308 else 1.055 * value ** (1.0 / 2.4) - 0.055


def summarize(directory):
    config = load_json(directory / "input.json")
    size = tuple(config["size"])
    counts = config["checkpoints"]
    names = ("bevy", "mp")
    labels = ("Bevy Editor", "Material Painter")
    reports = [load_json(directory / f"{name}.json") for name in names]
    if reports[0]["adapter"] != reports[1]["adapter"]:
        raise ValueError("Use the same GPU and graphics backend for both renders")
    for report in reports:
        if [entry["samples"] for entry in report["checkpoints"]] != counts or len(report["frame_ms"]) != counts[-1]:
            raise ValueError("Capture and timing sample counts differ")

    references = [hdr(directory / f"{name}-{counts[-1]}.rgba16", size) for name in names]
    masks = [region(pixels, size, config["background"]) for pixels in references]
    inside = [i for i, (a, b) in enumerate(zip(*masks)) if a and b]
    if len(inside) < 1000:
        raise ValueError("Insufficient common model coverage")
    refs_luma = [[luminance(pixels[i]) for i in inside] for pixels in references]
    rgb_mae = statistics.fmean(
        abs(a - b) for i in inside for a, b in zip(references[0][i], references[1][i])
    )
    view = Image.new("RGB", (size[0] * 2, (size[1] + 26) * len(counts)), "#14171c")
    draw = ImageDraw.Draw(view)
    results = []
    for column, (name, label, report, luma) in enumerate(zip(names, labels, reports, refs_luma)):
        frames = report["frame_ms"]
        by_samples = []
        for row, count in enumerate(counts):
            pixels = hdr(directory / f"{name}-{count}.rgba16", size)
            rmse = math.sqrt(statistics.fmean(
                (luminance(pixels[i]) - reference) ** 2 for i, reference in zip(inside, luma)
            ))
            by_samples.append({
                "samples": count,
                "luminance_rmse_against_final_frame": round(rmse, 6),
                "synchronized_total_ms": round(report["checkpoints"][row]["total_ms"], 2),
            })
            image = Image.new("RGB", size)
            image.putdata([neutral(pixel) for pixel in pixels])
            x, y = column * size[0], row * (size[1] + 26)
            draw.text((x + 8, y + 6), f"{label}  |  {count} spp", fill="#f4f6f8")
            view.paste(image, (x, y + 26))
        results.append({
            "app": label, "median_frame_ms": round(statistics.median(frames), 3),
            "p95_frame_ms": round(statistics.quantiles(frames, n=20, method="inclusive")[-1], 3),
            "checkpoints": by_samples,
        })
    summary = {
        "adapter": reports[0]["adapter"], "size": size, "triangles": config["triangles"],
        "bounces": config["bounces"], "rgb_mae_linear_at_final_sample": round(rgb_mae, 6),
        "common_interior_pixels": len(inside), "apps": results,
    }
    view.save(directory / "comparison.png")
    output = json.dumps(summary, indent=2)
    (directory / "comparison.json").write_text(output + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    summarize(parser.parse_args().directory)
