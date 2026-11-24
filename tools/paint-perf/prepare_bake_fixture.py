"""Build identical low- and high-poly inputs for the two bake benchmarks."""
import math
import struct


def point(phi, theta):
    radius = 0.28 + 0.035 * math.cos(13 * phi) * math.sin(7 * theta)
    ring = 0.70 + radius * math.cos(theta)
    return ring * math.cos(phi), radius * math.sin(theta), ring * math.sin(phi)


def vertex(u, v):
    phi, theta = math.tau * u, math.tau * v
    step = 1e-4
    px, py, pz = (a - b for a, b in zip(point(phi + step, theta), point(phi - step, theta)))
    tx, ty, tz = (a - b for a, b in zip(point(phi, theta + step), point(phi, theta - step)))
    normal = (ty * pz - tz * py, tz * px - tx * pz, tx * py - ty * px)
    length = math.hypot(*normal)
    return (*point(phi, theta), *(value / length for value in normal), 0.05 + 0.90 * u, 0.05 + 0.90 * v)


def triangles(width, height):
    for y in range(height):
        for x in range(width):
            a = y * (width + 1) + x
            b, c, d = a + 1, a + width + 2, a + width + 1
            yield a, d, b
            yield b, d, c


def prepare(out):
    width, height = 128, 64
    vertices = [struct.pack("<8f", *vertex(x / width, y / height))
                for y in range(height + 1) for x in range(width + 1)]
    with (out / "bake-low.bin").open("wb") as file:
        for face in triangles(width, height):
            for index in face:
                file.write(vertices[index])

    width, height = 512, 256
    vertices = [vertex(x / width, y / height)
                for y in range(height + 1) for x in range(width + 1)]
    with (out / "bake-high.obj").open("w", encoding="ascii") as file:
        file.write("o Relief_high\n")
        for x, y, z, nx, ny, nz, _, _ in vertices:
            file.write(f"v {x:.9g} {y:.9g} {z:.9g}\nvn {nx:.9g} {ny:.9g} {nz:.9g}\n")
        for face in triangles(width, height):
            file.write("f " + " ".join(f"{i+1}//{i+1}" for i in face) + "\n")
