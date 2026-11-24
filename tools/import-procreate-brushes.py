"""Prepare the editor's brush catalog from a user-supplied library (requires Pillow)."""
import argparse
import io
import json
from pathlib import PurePosixPath
import plistlib
from statistics import median
import zipfile

from PIL import Image, ImageChops, ImageStat


def stroke_sample(preview):
    """Recover a repeatable cross-section from the painted part of a QuickLook stroke.

    The archive names the component tips and grains, but doesn't contain their pixels.
    Remove the preview gesture's bend and taper while retaining the combined coverage,
    including gaps between bristles, broken edges and translucent interiors.
    """
    ink = ImageChops.multiply(preview.getchannel("A"), preview.convert("L"))
    x0, _, x1, _ = ink.getbbox()
    # Exclude the touchdown and lift, whose pressure belongs to the preview gesture.
    start = x0 + (x1 - x0) // 6
    end = x1 - (x1 - x0) // 6
    half_window = max(2, (end - start) // 32)
    columns = ink.transpose(Image.Transpose.TRANSPOSE).tobytes()
    upper, lower = [], []
    for x in range(start, end):
        col = columns[x * ink.height:(x + 1) * ink.height]
        # Ignore isolated specks when estimating the path, keep them in the image.
        cutoff = max(1, int(max(col) * 0.045))
        rows = [y for y, v in enumerate(col) if v >= cutoff]
        upper.append(rows[0] if rows else None)
        lower.append(rows[-1] + 1 if rows else None)

    def bounds(i):
        lo, hi = max(0, i - half_window), min(len(upper), i + half_window + 1)
        tops = [v for v in upper[lo:hi] if v is not None]
        bottoms = [v for v in lower[lo:hi] if v is not None]
        top, bottom = (median(tops), median(bottoms)) if tops else (0, ink.height)
        pad = max(1.5, (bottom - top) * 0.09)
        return top - pad, bottom + pad

    edges = list(range(0, end - start, 4)) + [end - start]
    spans = [bounds(i) for i in edges]
    height = max(2, round(median(b - t for t, b in spans)))
    mesh = []
    for left, right, (t0, b0), (t1, b1) in zip(edges, edges[1:], spans, spans[1:]):
        x, xx = start + left, start + right
        mesh.append(((left, 0, right, height), (x, t0, x, b0, xx, b1, xx, t1)))
    sample = ink.transform((end - start, height), Image.Transform.MESH, mesh, Image.Resampling.BILINEAR)
    sample.thumbnail((720, 192), Image.Resampling.LANCZOS)
    return sample, max(1, round(height / 5))


def paper_tile(preview):
    """Find the repeat of a fixed pattern in a preview that covers the whole frame."""
    mask = ImageChops.multiply(preview.getchannel("A"), preview.convert("L"))
    x, y = mask.width // 3, mask.height // 3
    w, h = mask.width // 6, mask.height // 5
    ref = mask.crop((x, y, x + w, y + h))
    contrast = ImageStat.Stat(ref).stddev[0]
    periods = []
    for vertical in (False, True):
        room = mask.height - y - h if vertical else mask.width - x - w
        scores = []
        for shift in range(6, min(room, mask.height // 2) + 1):
            xx, yy = (x, y + shift) if vertical else (x + shift, y)
            patch = mask.crop((xx, yy, xx + w, yy + h))
            difference = ImageStat.Stat(ImageChops.difference(ref, patch)).mean[0]
            scores.append((difference, shift))
        error, period = min(scores)
        if error > contrast * 0.22:
            return None
        periods.append(period)
    pw, ph = periods
    tile = mask.crop((x, y, x + pw, y + ph))
    return tile.resize((max(1, round(pw / 5)), max(1, round(ph / 5))), Image.Resampling.LANCZOS)


def png(image):
    data = io.BytesIO()
    image.save(data, format="PNG", optimize=True)
    return data.getvalue()


def import_library(source, destination):
    entries = []
    with zipfile.ZipFile(source) as library, zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as output:
        for filename in library.namelist():
            path = PurePosixPath(filename)
            if path.suffix != ".brush":
                continue
            with zipfile.ZipFile(io.BytesIO(library.read(filename))) as brush:
                archive = plistlib.loads(brush.read("Brush.archive"))
                objects = archive["$objects"]
                root = objects[archive["$top"]["root"].data]

                def value(key, fallback):
                    item = root.get(key, fallback)
                    return objects[item.data] if isinstance(item, plistlib.UID) else item

                preview = Image.open(io.BytesIO(brush.read("QuickLook/Thumbnail.png"))).convert("RGBA")
                sample, diameter = stroke_sample(preview)
                x0, y0, x1, y1 = preview.getchannel("A").getbbox()
                canvas_grain = False
                if (value("textureApplication", 0) == 1 and x1 - x0 > preview.width * 0.96
                        and y1 - y0 > preview.height * 0.9):
                    tile = paper_tile(preview)
                    if tile is not None:
                        canvas_grain, sample = True, tile
                # The transverse opacity profile also tells us how gently a dab should end.
                profile = sample.resize((1, sample.height), Image.Resampling.BOX).tobytes()
                peak = max(profile)
                dense_fraction = sum(profile) / (len(profile) * max(peak, 1))
                cap_hardness = max(0.0, min(1.0, 2.0 * dense_fraction - 0.8))
                index = len(entries)
                entries.append({
                    "name": value("name", path.stem),
                    "category": path.parent.name,
                    "sample": f"stroke/{index}.png",
                    "diameter": diameter,
                    "canvas_grain": canvas_grain,
                    "cap_hardness": round(cap_hardness, 3),
                    "pressure_size": value("dynamicsPressureSize", 0.0),
                    "pressure_opacity": value("dynamicsPressureOpacity", 0.0),
                })
                output.writestr(entries[-1]["sample"], png(sample))
                preview.thumbnail((480, 140), Image.Resampling.LANCZOS)
                output.writestr(f"{index}.png", png(preview))
        output.writestr("catalog.json", json.dumps(entries, ensure_ascii=False, allow_nan=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("library")
    parser.add_argument("output")
    args = parser.parse_args()
    import_library(args.library, args.output)
