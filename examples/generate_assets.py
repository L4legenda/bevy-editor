"""Regenerate the small, self-authored GLB example assets. Python stdlib only."""
import json
import math
from pathlib import Path
import struct

ROOT = Path(__file__).parent


def glb(path, color):
    positions, normals, indices = [], [], []
    # Counterclockwise faces with separate vertices for flat normals.
    for normal, corners in [
        ((0, 0, 1), [(-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]),
        ((0, 0, -1), [(1, -1, -1), (-1, -1, -1), (-1, 1, -1), (1, 1, -1)]),
        ((1, 0, 0), [(1, -1, 1), (1, -1, -1), (1, 1, -1), (1, 1, 1)]),
        ((-1, 0, 0), [(-1, -1, -1), (-1, -1, 1), (-1, 1, 1), (-1, 1, -1)]),
        ((0, 1, 0), [(-1, 1, 1), (1, 1, 1), (1, 1, -1), (-1, 1, -1)]),
        ((0, -1, 0), [(-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1)]),
    ]:
        offset = len(positions) // 3
        positions.extend(value * 0.5 for point in corners for value in point)
        normals.extend(normal * 4)
        indices.extend(offset + index for index in (0, 1, 2, 0, 2, 3))
    binary = struct.pack('<72f72f36H', *positions, *normals, *indices)
    attributes = {'POSITION': 0, 'NORMAL': 1}
    data = {
        'asset': {'version': '2.0', 'generator': 'bevy-editor example'},
        'scene': 0, 'scenes': [{'nodes': [0]}],
        'nodes': [{'name': 'Example block', 'mesh': 0, 'translation': [0, 0.5, 0]}],
        'meshes': [{'primitives': [{'attributes': attributes, 'indices': 2, 'material': 0}]}],
        'materials': [{'pbrMetallicRoughness': {'baseColorFactor': color, 'metallicFactor': 0, 'roughnessFactor': 0.65}}],
        'buffers': [{'byteLength': len(binary)}],
        'bufferViews': [
            {'buffer': 0, 'byteOffset': 0, 'byteLength': 288, 'target': 34962},
            {'buffer': 0, 'byteOffset': 288, 'byteLength': 288, 'target': 34962},
            {'buffer': 0, 'byteOffset': 576, 'byteLength': 72, 'target': 34963},
        ],
        'accessors': [
            {'bufferView': 0, 'componentType': 5126, 'count': 24, 'type': 'VEC3', 'min': [-0.5] * 3, 'max': [0.5] * 3},
            {'bufferView': 1, 'componentType': 5126, 'count': 24, 'type': 'VEC3'},
            {'bufferView': 2, 'componentType': 5123, 'count': 36, 'type': 'SCALAR'},
        ],
    }
    encoded = json.dumps(data, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(struct.pack('<III', 0x46546C67, 2, 28 + len(encoded) + len(binary))
                     + struct.pack('<II', len(encoded), 0x4E4F534A) + encoded
                     + struct.pack('<II', len(binary), 0x004E4942) + binary)


glb(ROOT / 'assets/models/block.glb', [0.72, 0.36, 0.13, 1])
glb(ROOT / 'assets/models/blue_block.glb', [0.12, 0.4, 0.72, 1])
yaw, pitch = math.atan2(6, 7), -math.atan2(4, math.hypot(6, 7))
rotation = [math.cos(yaw / 2) * math.sin(pitch / 2), math.sin(yaw / 2) * math.cos(pitch / 2),
            -math.sin(yaw / 2) * math.sin(pitch / 2), math.cos(yaw / 2) * math.cos(pitch / 2)]
camera = {'id': 'main', 'label': 'Главная камера', 'placement': {
    'pose': {'position': [6, 4, 7], 'rotation': rotation, 'scale': [1, 1, 1]}, 'lens': 45}}
definitions = [('Мастерская', 'workshop', 'block', [[-2, 0, 0], [0, 0, 0], [2, 0, 0]]),
               ('Малая сцена', 'small_room', 'blue_block', [[0, 0, 0]])]
for label, name, model, positions in definitions:
    scene = {'scene': {'label': label, 'cameras': [camera]}, 'models': [
        {'id': index + 1, 'asset': f'models/{model}.glb',
         'pose': {'position': position, 'rotation': [0, 0, 0, 1], 'scale': [1, 1, 1]}}
        for index, position in enumerate(positions)
    ]}
    path = ROOT / f'assets/scenes/{name}.scene'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(scene, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
project = {'title': 'Bevy Editor', 'asset_root': 'assets', 'scenes': [
    {'file': f'scenes/{name}.scene'} for _, name, _, _ in definitions
], 'catalog': [
    {'label': label, 'asset': f'models/{name}.glb', 'category': 'Примитивы', 'solid': True}
    for label, name in [('Блок', 'block'), ('Синий блок', 'blue_block')]
]}
(ROOT / 'project.json').write_text(json.dumps(project, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
