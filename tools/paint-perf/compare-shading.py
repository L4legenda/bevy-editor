"""Summarize matched offscreen renders using pixels inside both cube silhouettes."""

import argparse
import json
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageOps


MODES = ("Unlit linear", "Unlit", "PBR", "PBR + Rim Light", "Toon", "Clay")


def pixels(image):
    return image.get_flattened_data() if hasattr(image, "get_flattened_data") else image.getdata()


def coverage(path):
    image = Image.open(path).convert("RGB")
    mask = Image.new("L", image.size)
    mask.putdata([255 if g > r + 80 and g > b + 80 else 0
                  for r, g, b in pixels(image)])
    return mask


def summary(directory):
    adapters = [(directory / f"{name}-adapter.txt").read_text().strip()
                for name in ("bevy", "material-painter")]
    if adapters[0] != adapters[1]:
        raise ValueError(f"Renderer adapters differ: {adapters}")
    bevy_mask = coverage(directory / "bevy-Coverage.png")
    mp_mask = coverage(directory / "mp-Coverage.png")
    if bevy_mask.size != mp_mask.size:
        raise ValueError("The render sizes differ")
    common = ImageChops.darker(bevy_mask, mp_mask)
    union = ImageChops.lighter(bevy_mask, mp_mask)
    iou = common.histogram()[255] / union.histogram()[255]
    interior = [i for i, alpha in enumerate(pixels(common.filter(ImageFilter.MinFilter(5)))) if alpha]
    if not interior:
        raise ValueError("No common interior")

    columns, row_height = 350, 237
    sheet = Image.new("RGB", (columns * 2, row_height * len(MODES)), "#14171c")
    draw = ImageDraw.Draw(sheet)
    results = []
    for row, mode in enumerate(MODES):
        bevy, mp = [Image.open(directory / f"{app}-{mode}.png").convert("RGB")
                    for app in ("bevy", "mp")]
        a, b = list(pixels(bevy)), list(pixels(mp))
        deltas = sorted(abs(a[i][channel] - b[i][channel]) for i in interior for channel in range(3))
        count = len(interior)
        results.append({
            "mode": mode,
            "bevy_rgb": [round(sum(a[i][channel] for i in interior) / count, 2) for channel in range(3)],
            "material_painter_rgb": [round(sum(b[i][channel] for i in interior) / count, 2) for channel in range(3)],
            "mean_absolute_error": round(sum(deltas) / len(deltas), 3),
            "p95_absolute_error": deltas[int((len(deltas) - 1) * 0.95)],
        })
        for col, (label, image) in enumerate(zip(("Bevy Editor", "Material Painter"), (bevy, mp))):
            thumb = ImageOps.contain(image, (columns - 12, row_height - 30))
            x, y = col * columns, row * row_height
            draw.text((x + 7, y + 6), f"{label} | {mode}", fill="#f4f6f8")
            sheet.paste(thumb, (x + (columns - thumb.width) // 2, y + 24))
    sheet.save(directory / "comparison.png")
    report = {
        "adapter": adapters[0],
        "image_size": list(bevy_mask.size),
        "silhouette_iou": round(iou, 5),
        "interior_pixels": len(interior),
        "rgb_range": [0, 255],
        "modes": results,
    }
    output = json.dumps(report, indent=2)
    (directory / "comparison.json").write_text(output + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    summary(parser.parse_args().directory)
