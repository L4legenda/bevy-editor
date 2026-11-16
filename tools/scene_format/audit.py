"""Read-only TSCN inventory and isolated real-Godot tests/benchmarks.

The source project is never passed to Godot. Original scripts are never run.
Static inventory is deliberately not presented as an engine parser or validator.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time


HEADER = re.compile(r'^\[(\w+)(?:\s|\])', re.M)
EXTERNAL = re.compile(r'^\[ext_resource\s+([^\n]+)\]', re.M)
ATTRIBUTE = re.compile(r'(\w+)="((?:[^"\\]|\\.)*)"')


def inventory(project):
    scenes = []
    hashes = {}
    for folder in ('Testing', 'components', 'assets'):
        for path in sorted((project / folder).rglob('*.tscn')):
            if path.is_symlink() or '.godot' in path.parts:
                continue
            data = path.read_bytes()
            source = data.decode('utf-8-sig')
            relative = path.relative_to(project).as_posix()
            hashes[relative] = hashlib.sha256(data).hexdigest()
            counts = Counter(HEADER.findall(source))
            dependencies = []
            for match in EXTERNAL.finditer(source):
                attrs = dict(ATTRIBUTE.findall(match.group(1)))
                target = attrs.get('path', '')
                dependencies.append({**attrs, 'path_exists': (project / target[6:]).is_file() if target.startswith('res://') else None})
            animations = list(re.finditer(r'^\[sub_resource type="Animation"[^\n]*\]\n', source, re.M))
            animation_bytes = 0
            for animation in animations:
                next_header = re.search(r'^\[', source[animation.end():], re.M)
                end = animation.end() + next_header.start() if next_header else len(source)
                animation_bytes += len(source[animation.start():end].encode('utf-8'))
            scenes.append({'file': relative, 'bytes': len(data), 'nodes': counts['node'], 'external_resources': counts['ext_resource'], 'local_resources': counts['sub_resource'], 'connections': counts['connection'], 'animations': len(animations), 'animation_bytes': animation_bytes, 'dependencies': dependencies})
    return {'files': len(scenes), 'bytes': sum(s['bytes'] for s in scenes), 'nodes': sum(s['nodes'] for s in scenes), 'external_resources': sum(s['external_resources'] for s in scenes), 'local_resources': sum(s['local_resources'] for s in scenes), 'scenes': scenes, 'source_sha256': hashes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--godot', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=Path('target/scene-format-audit'))
    args = parser.parse_args()
    project = args.project.resolve(strict=True)
    output = args.output.resolve()
    if output == project or output.is_relative_to(project):
        parser.error('The output must be outside the source project')
    output.mkdir(parents=True, exist_ok=True)
    before = inventory(project)
    lab = output / 'godot-lab'
    lab.mkdir(exist_ok=True)
    (lab / 'project.godot').write_text('config_version=5\n\n[application]\nconfig/name="Scene Format Lab"\n\n[rendering]\nrenderer/rendering_method="gl_compatibility"\n', encoding='utf-8')
    for filename in ('godot_lab.gd', 'probe_node.gd'):
        shutil.copyfile(Path(__file__).with_name(filename), lab / filename)
    started = time.perf_counter()
    run = subprocess.run([str(args.godot.resolve(strict=True)), '--headless', '--path', str(lab), '--script', 'res://godot_lab.gd'], capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=180)
    (output / 'godot.log').write_text(run.stdout + '\n' + run.stderr, encoding='utf-8')
    after = inventory(project)
    report = {'platform': platform.platform(), 'elapsed_seconds': time.perf_counter() - started, 'godot_exit_code': run.returncode, 'source_unchanged': before['source_sha256'] == after['source_sha256'], 'inventory': before, 'godot': json.loads((lab / 'results.json').read_text(encoding='utf-8')) if (lab / 'results.json').exists() else None}
    (output / 'audit.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({key: value for key, value in report.items() if key not in ('inventory', 'godot')}))
    print('Inventory:', before['files'], 'scenes,', before['nodes'], 'node declarations')
    if run.returncode or report['godot'] is None:
        print((run.stdout + '\n' + run.stderr)[-12000:])
    if not report['source_unchanged'] or run.returncode or report['godot'] is None:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
