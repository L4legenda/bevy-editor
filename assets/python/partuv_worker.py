"""Local PartUV Geometric adapter. Only UVs return to the editor, never geometry."""
import argparse
import collections
import json
import os
from pathlib import Path
import tempfile
import traceback

os.environ.setdefault("OMP_NUM_THREADS", "6")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "6")


def event(phase, fraction):
    print("BEVY_UV:" + json.dumps({"phase": phase, "fraction": fraction}), flush=True)


def pack_rigid(uv, faces, request):
    """Pack charts, then apply one rigid transform to every corner of each chart.

    xatlas may leave corners of sliver/degenerate UV triangles untransformed.
    Recovering a chart transform prevents those corners from tearing the surface.
    A rectangular chart fallback handles charts that xatlas splits internally.
    """
    import numpy as np
    import xatlas
    parent = list(range(len(faces)))

    def root(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    edges = {}
    for i, face in enumerate(faces):
        for a, b in zip(face, np.roll(face, -1)):
            key = tuple(sorted((int(a), int(b))))
            if key in edges:
                parent[root(i)] = root(edges[key])
            else:
                edges[key] = i
    groups = collections.defaultdict(list)
    for i in range(len(faces)):
        groups[root(i)].append(i)
    charts = []
    for indices in groups.values():
        indices = np.asarray(indices)
        unique, inverse = np.unique(faces[indices], return_inverse=True)
        local = inverse.reshape(-1, 3).astype(np.uint32)
        coordinates = np.asarray(uv[unique], dtype=np.float32)
        tri = coordinates[local].astype(np.float64)
        ab, ac = tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]
        areas = np.abs(ab[:, 0] * ac[:, 1] - ab[:, 1] * ac[:, 0])
        valid = areas > max(float(areas.max()) * 1e-10, 1e-24)
        charts.append((indices, local, coordinates, local[valid]))

    def attempt(rectangles):
        atlas = xatlas.Atlas()
        sources = []
        for _, _, coordinates, valid in charts:
            if rectangles or not len(valid):
                low = coordinates.min(axis=0)
                extent = max(float(np.ptp(coordinates, axis=0).max()), 1e-4)
                high = np.maximum(coordinates.max(axis=0), low + extent * 1e-4)
                source = np.array([low, [high[0], low[1]], high, [low[0], high[1]]], dtype=np.float32)
                triangles = np.array([[0, 1, 2], [0, 2, 3]], dtype=np.uint32)
            else:
                source, triangles = coordinates, valid
            atlas.add_uv_mesh(source, triangles)
            sources.append((source, triangles))
        options = xatlas.PackOptions()
        options.resolution = int(request["resolution"])
        options.padding = int(request["padding"])
        atlas.generate(pack_options=options)
        # xatlas may choose a rectangular backing image. The editor's target is
        # square: normalize both axes by the same size to preserve texel shape.
        side = max(atlas.width, atlas.height)
        normalization = np.array([atlas.width / side, atlas.height / side])
        output = np.empty((len(faces), 3, 2), dtype=np.float64)
        total_area = 0.0
        for i, ((indices, local, coordinates, _), (source, triangles)) in enumerate(zip(charts, sources)):
            mapping, packed_faces, packed_uv = atlas[i]
            if not np.array_equal(np.asarray(mapping)[packed_faces], triangles):
                raise ValueError("Упаковщик изменил порядок треугольников")
            before = source[triangles].astype(np.float64)
            after = np.asarray(packed_uv)[packed_faces].astype(np.float64) * normalization
            ab, ac = before[:, 1] - before[:, 0], before[:, 2] - before[:, 0]
            seed = np.argmax(np.abs(ab[:, 0] * ac[:, 1] - ab[:, 1] * ac[:, 0]))
            a, b = before[seed], after[seed]
            matrix = np.linalg.solve(a[1:] - a[0], b[1:] - b[0])
            origin = a[0]
            destination = b[0]
            if rectangles:
                # Pixel rounding can scale the two rectangle axes differently.
                # Fit a rigid chart inside that reserved rectangle instead.
                left, singular, right = np.linalg.svd(matrix)
                origin = (source.min(axis=0).astype(np.float64) + source.max(axis=0)) * .5
                destination = (origin - a[0]) @ matrix + b[0]
                matrix = (left @ right) * singular.min()
            gram = matrix @ matrix.T
            scale = float(np.trace(gram) / 2)
            predicted = (before - origin) @ matrix + destination
            if not np.isfinite(matrix).all() or scale <= 0 or not np.allclose(gram, np.eye(2) * scale, rtol=.003, atol=scale * .003):
                raise ValueError("Упаковщик исказил форму UV-острова")
            # Degenerate corners may be unset in xatlas; valid triangles must agree.
            ab, ac = before[:, 1] - before[:, 0], before[:, 2] - before[:, 0]
            area = np.abs(ab[:, 0] * ac[:, 1] - ab[:, 1] * ac[:, 0])
            stable = area > max(float(area.max()) * 1e-6, 1e-20)
            if not rectangles and not np.allclose(predicted[stable], after[stable], atol=3e-5, rtol=0):
                raise ValueError("Упаковщик разделил непрерывный UV-остров")
            result = ((coordinates.astype(np.float64) - origin) @ matrix + destination)[local]
            if result.min() < -1e-5 or result.max() > 1.00001:
                raise ValueError("Остров вышел за пределы атласа")
            output[indices] = result
            ab, ac = result[:, 1] - result[:, 0], result[:, 2] - result[:, 0]
            total_area += float(np.abs(ab[:, 0] * ac[:, 1] - ab[:, 1] * ac[:, 0]).sum() / 2)
        if total_area > 1.0001:
            raise ValueError("Острова перекрываются после упаковки")
        return output, len(charts), float(atlas.utilization) * atlas.width * atlas.height / (side * side)

    try:
        return attempt(False)
    except (ValueError, np.linalg.LinAlgError):
        event("Упаковка сложных UV-островов", .85)
        return attempt(True)


def unwrap(request):
    import numpy as np
    import trimesh
    import partuv
    from partuv.geometric import build_face_tree
    from partuv.preprocess_utils.manifold import fix_mesh_trimesh
    from partuv.output import validate_provenance

    vertices = np.asarray(request["positions"], dtype=np.float64)
    faces = np.asarray(request["triangles"], dtype=np.int32)
    if vertices.ndim != 2 or vertices.shape[1] != 3 or faces.ndim != 2 or faces.shape[1] != 3:
        raise ValueError("Некорректная треугольная сетка")
    if not len(faces) or not np.isfinite(vertices).all():
        raise ValueError("Модель не содержит корректной поверхности")
    if faces.min() < 0 or faces.max() >= len(vertices):
        raise ValueError("Повреждены индексы поверхности")
    # CPU mode: no CUDA, PyTorch, checkpoints or AI segmentation are required.
    config = """verbose: false
save_stuff: false
pipeline:
  threshold: 1.25
  parallelDepth: 6
  component_maxDepth: 10
  num_omp_threads: 6
  num_cuda_streams: 1
unwrap:
  method: abf
  abf_iters: 5
  agg_parts: 10
  pamo: false
  usePamoFaceThreshold: 1000
"""
    existing = request.get("existing_uv")
    if existing is not None:
        uv = np.asarray(existing, dtype=np.float32).reshape(-1, 2)
        if len(uv) != len(faces) * 3 or not np.isfinite(uv).all():
            raise ValueError("Некорректная исходная UV-карта")
        # Weld only identical geometry+UV corners, retaining every existing seam.
        keys = np.column_stack([faces.reshape(-1), uv])
        _, first, inverse = np.unique(keys, axis=0, return_index=True, return_inverse=True)
        v = vertices[faces.reshape(-1)[first]]
        f = inverse.reshape(-1, 3).astype(np.uint32)
        uv = uv[first]
    else:
        event("Подготовка поверхности", .08)
        mesh = fix_mesh_trimesh(trimesh.Trimesh(vertices, faces, process=False))
        if len(mesh.faces) != len(faces):
            raise ValueError("Подготовка изменила число треугольников; результат отклонён")
        event("Поиск частей модели", .20)
        tree = build_face_tree(mesh)
        source_v = np.asarray(mesh.vertices, dtype=np.float64)
        source_f = np.asarray(mesh.faces, dtype=np.int32)
        event("Развёртка поверхности", .40)
        with tempfile.TemporaryDirectory(prefix="bevy-partuv-") as temporary:
            path = Path(temporary) / "config.yaml"
            path.write_text(config, encoding="utf-8")
            parts = partuv.pipeline_numpy(source_v, source_f, tree, str(path), float(request["threshold"]), visual=False)
        combined = parts.to_components()
        v = np.asarray(combined.V)
        f = np.asarray(combined.F, dtype=np.uint32)
        uv = np.asarray(combined.UV, dtype=np.float32)[:, :2]
        provenance = np.asarray(combined.source_vid)
        validate_provenance(v, f, uv, provenance, source_v, source_f)
    event("Упаковка UV-островов", .80)
    result_uv, chart_count, utilization = pack_rigid(uv, f, request)
    event("Проверка результата", .95)
    # Match by exact positions, preserving the editor's original corner order,
    # materials, normals and polygon topology even after non-manifold fan splits.
    def key(points):
        return tuple(sorted(tuple(p) for p in points))
    original = collections.defaultdict(collections.deque)
    for index, face in enumerate(faces):
        original[key(vertices[face])].append(index)
    output = np.empty((len(faces), 3, 2), dtype=np.float64)
    for face, coordinates in zip(f, result_uv):
        points = v[face]
        queue = original.get(key(points))
        if not queue:
            raise ValueError("Движок изменил геометрию; результат отклонён")
        index = queue.popleft()
        for corner, point in enumerate(vertices[faces[index]]):
            matches = np.flatnonzero(np.all(points == point, axis=1))
            if len(matches) != 1:
                raise ValueError("Найдены вырожденные треугольники")
            output[index, corner] = coordinates[matches[0]]
    if any(original.values()) or not np.isfinite(output).all() or output.min() < -1e-5 or output.max() > 1.00001:
        raise ValueError("UV-карта не прошла проверку")
    return {"uv": output.tolist(), "charts": chart_count, "utilization": utilization}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        result = unwrap(json.loads(args.input.read_text(encoding="utf-8")))
        temporary = args.output.with_suffix(".tmp")
        temporary.write_text(json.dumps(result, allow_nan=False, separators=(",", ":")), encoding="utf-8")
        temporary.replace(args.output)
        event("Готово", 1.0)
        return 0
    except Exception as error:
        traceback.print_exc()
        event(str(error), 0)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
