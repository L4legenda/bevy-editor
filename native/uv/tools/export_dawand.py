"""Export the linear weights used by the pinned DA Wand inference graph to safetensors."""
import argparse
import hashlib
import io
import json
import math
import pickle
import struct
import zipfile
from collections import OrderedDict
from pathlib import Path

SOURCE_SHA256 = "9d5cf2fad115b2afa1fa5c0ad51eb81d5619549b534fe2505f51e332950a17d2"


class Checkpoint(pickle.Unpickler):
    def find_class(self, module, name):
        if (module, name) == ("collections", "OrderedDict"):
            return OrderedDict
        if (module, name) == ("torch", "FloatStorage"):
            return "F32"
        if (module, name) == ("torch._utils", "_rebuild_tensor_v2"):
            return lambda storage, offset, shape, stride, *_: (storage, offset, shape, stride)
        raise ValueError(f"Unexpected checkpoint type {module}.{name}")

    def persistent_load(self, identifier):
        kind, dtype, key, _, _ = identifier
        if kind != "storage" or dtype != "F32":
            raise ValueError("Expected float32 storage")
        return key


def export(source, destination):
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError("Expected DAWand-Blender checkpoint at 7d0ee5b54e4e")
    header, data = {}, bytearray()
    with zipfile.ZipFile(source) as archive:
        meta = next(p for p in archive.namelist() if p.endswith("/data.pkl"))
        root = meta.removesuffix("data.pkl")
        tensors = Checkpoint(io.BytesIO(archive.read(meta))).load()
        for name, (key, offset, shape, stride) in tensors.items():
            if ".ogconv." in name:  # Training checkpoint also stores an unused Conv2d.
                continue
            assert tuple(stride) == tuple(math.prod(shape[i + 1:]) for i in range(len(shape)))
            block = archive.read(f"{root}data/{key}")[4 * offset:4 * (offset + math.prod(shape))]
            assert len(block) == 4 * math.prod(shape)
            header[name] = dict(dtype="F32", shape=shape, data_offsets=[len(data), len(data)+len(block)])
            data.extend(block)
    meta = json.dumps(header, separators=(",", ":")).encode()
    meta += b" " * (-len(meta) % 8)
    destination.write_bytes(struct.pack("<Q", len(meta)) + meta + data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkpoint", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    export(args.checkpoint, args.output)
