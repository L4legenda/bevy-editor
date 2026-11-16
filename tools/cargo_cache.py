"""Run Cargo and prune only previously observed, now superseded artifact files.

Unknown files, build-script directories and Cargo fingerprints are never removed.
Cargo reports both newly compiled and fresh artifacts in its JSON messages.
"""
import json
import os
from pathlib import Path
import subprocess
import sys


def local_file(root, name):
    path = Path(name)
    if not path.is_absolute():
        path = root / path
    # Resolve every candidate separately, including parent links/junctions.
    resolved = path.resolve()
    if resolved == root or not resolved.is_relative_to(root):
        return None
    if path.is_symlink() or not resolved.is_file():
        return None
    return resolved


def load_state(root):
    path = root / '.bevy-editor-cache.json'
    if not path.exists():
        return {'version': 1, 'snapshots': {}, 'known': []}
    state = json.loads(path.read_text(encoding='utf-8'))
    if state.get('version') != 1 or not isinstance(state['snapshots'], dict):
        raise ValueError('Unrecognised cache index; cleanup skipped')
    if not all(isinstance(files, list) and all(isinstance(p, str) for p in files)
               for files in state['snapshots'].values()):
        raise ValueError('Invalid cache snapshots; cleanup skipped')
    if not isinstance(state['known'], list) or not all(isinstance(p, str) for p in state['known']):
        raise ValueError('Invalid cache file index; cleanup skipped')
    return state


def prune(root, state, limit_bytes):
    active = set().union(*map(set, state['snapshots'].values()))
    # Never use file age: an unchanged dependency may still be in active use.
    candidates = set(state['known']) - active
    total = sum(p.stat().st_size for p in root.rglob('*') if p.is_file())
    removed = 0
    for name in sorted(candidates):
        if total <= limit_bytes:
            break
        path = local_file(root, name)
        if path is None:
            continue
        try:
            size = path.stat().st_size
            path.unlink()
        except OSError as error:
            print(f'Cache: keeping {name}: {error}', file=sys.stderr)
            continue
        total -= size
        removed += size
    state['known'] = sorted(name for name in state['known'] if local_file(root, name))
    if removed:
        print(f'Cache: removed {removed / 1024**2:.1f} MiB of superseded artifacts.')
    if total > limit_bytes:
        print('Cache: size remains above the threshold; current and untracked files were kept.')


def snapshot_key(args):
    # Keep separate caches for build/run, tests, check/clippy, and each profile/target.
    # Feature/package selections replace the previous snapshot of that category.
    group = {'run': 'build', 'clippy': 'check'}.get(args[0], args[0])
    profile, target = 'dev', 'host'
    for index, arg in enumerate(args):
        if arg == '--':
            break
        if arg == '--release':
            profile = 'release'
        for option in ('--profile', '--target'):
            value = None
            if arg == option and index + 1 < len(args):
                value = args[index + 1]
            elif arg.startswith(option + '='):
                value = arg.split('=', 1)[1]
            if value is not None:
                if option == '--profile':
                    profile = value
                else:
                    target = value
    return json.dumps([group, profile, target])


def run(cargo, root, limit_gb, args):
    if root.is_symlink() or (root.exists() and getattr(root, 'is_junction', lambda: False)()):
        raise ValueError('Refusing to maintain a linked target directory')
    root = root.resolve()
    env = dict(os.environ, CARGO_TARGET_DIR=str(root))
    # Explicit formatting overrides would make the inventory incomplete.
    cargo_options = args[:args.index('--')] if '--' in args else args
    if any(a.startswith('--message-format') for a in cargo_options):
        raise ValueError('This wrapper manages --message-format itself')
    command = [cargo, args[0], '--message-format=json-render-diagnostics', *args[1:]]
    artifacts = set()
    finished = False
    process = subprocess.Popen(command, env=env, stdout=subprocess.PIPE,
                               text=True, encoding='utf-8', errors='replace')
    try:
        for line in process.stdout:
            message = None
            if not finished and line.startswith('{'):
                try:
                    message = json.loads(line)
                except ValueError:
                    pass
            if not isinstance(message, dict) or 'reason' not in message:
                print(line, end='', flush=True)
                continue
            if message['reason'] == 'compiler-artifact':
                for name in message.get('filenames', []):
                    path = local_file(root, name)
                    if path:
                        artifacts.add(path.relative_to(root).as_posix())
            elif message['reason'] == 'build-finished':
                finished = bool(message.get('success'))
            elif message['reason'] == 'compiler-message':
                rendered = message.get('message', {}).get('rendered')
                if rendered:
                    print(rendered, end='', file=sys.stderr, flush=True)
        result = process.wait()
    except BaseException:
        process.terminate()
        process.wait()
        raise
    if result == 0 and finished and artifacts:
        try:
            state = load_state(root)
            state['snapshots'][snapshot_key(args)] = sorted(artifacts)
            state['known'] = sorted(set(state['known']) | artifacts)
            prune(root, state, limit_gb * 1024**3)
            temporary = root / '.bevy-editor-cache.json.tmp'
            temporary.write_text(json.dumps(state, indent=2), encoding='utf-8')
            temporary.replace(root / '.bevy-editor-cache.json')
        except (OSError, ValueError, KeyError, TypeError) as error:
            # Maintenance must never turn a successful build into a failed one.
            print(f'Cache maintenance skipped: {error}', file=sys.stderr)
    return result


if __name__ == '__main__':
    sys.exit(run(sys.argv[1], Path(sys.argv[2]), float(sys.argv[3]), sys.argv[4:]))
