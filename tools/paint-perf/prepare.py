"""Create an identical high-poly fixture and a disposable Material Painter benchmark.

Run from the editor repository: python tools/paint-perf/prepare.py G:/material-painter
The source repository is read only; all generated files live under target/paint-perf.
"""
import json
import math
from pathlib import Path
import shutil
import struct
import sys

root = Path(__file__).resolve().parents[2]
out = root / "target/paint-perf"
out.mkdir(parents=True, exist_ok=True)
nx, ny = 512, 256
vertices = []
for y in range(ny + 1):
    v = y / ny
    theta = math.pi * v
    for x in range(nx + 1):
        u = x / nx
        phi = 2 * math.pi * u
        n = (math.sin(theta) * math.sin(phi), math.cos(theta), math.sin(theta) * math.cos(phi))
        r = 0.90 + 0.055 * math.sin(phi * 13) * math.sin(theta * 17) ** 2
        vertices.append((*[c * r for c in n], *n, u, v))
with (out / "complex.bin").open("wb") as f:
    for y in range(ny):
        for x in range(nx):
            a = y * (nx + 1) + x
            b, c, d = a + 1, a + nx + 2, a + nx + 1
            for i in (a, b, c, a, c, d):
                f.write(struct.pack("<8f", *vertices[i]))
(out / "fixture.json").write_text(json.dumps(dict(triangles=nx * ny * 2, view=[1024, 768], fov=58, distance=4, radius=35, stamps_per_frame=6, frames=180, texture_sizes=[1024, 2048], layers=8), indent=2))
source = Path(sys.argv[1])
dest = out / "material-painter"
dest.mkdir(exist_ok=True)
for name in ("Cargo.toml", "Cargo.lock", "build.rs"):
    shutil.copy2(source / name, dest / name)
for name in ("src", "assets", "vendor"):
    shutil.copytree(source / name, dest / name, dirs_exist_ok=True)
manifest = dest / "Cargo.toml"
with manifest.open("a") as f:
    f.write('\n[profile.test]\nopt-level = 1\ndebug = 0\n')
shutil.copy2(root / "tools/paint-perf/material_painter_bench.rs", dest / "src/paint_comparison.rs")
with (dest / "src/main.rs").open("a") as f:
    f.write('\n#[cfg(test)]\nmod paint_comparison;\n')
print(dest)
