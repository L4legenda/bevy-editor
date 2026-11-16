import tempfile
import unittest
from pathlib import Path

from cargo_cache import local_file, prune, snapshot_key


class CargoCacheTests(unittest.TestCase):
    def test_only_known_superseded_files_are_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for name in ('old.rlib', 'current.rlib', 'tests.exe', 'unknown.zip'):
                (root / name).write_bytes(b'x' * 128)
            state = {'known': ['old.rlib', 'current.rlib', 'tests.exe'],
                     'snapshots': {'build': ['current.rlib'], 'test': ['tests.exe']}}
            prune(root, state, 1)
            self.assertFalse((root / 'old.rlib').exists())
            for name in ('current.rlib', 'tests.exe', 'unknown.zip'):
                self.assertTrue((root / name).exists())

    def test_below_threshold_keeps_superseded_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'old.rlib').write_bytes(b'x')
            state = {'known': ['old.rlib'], 'snapshots': {}}
            prune(root, state, 1024)
            self.assertTrue((root / 'old.rlib').exists())

    def test_files_outside_target_are_never_candidates(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory).resolve()
            root = parent / 'target'
            root.mkdir()
            outside = parent / 'keep.txt'
            outside.write_text('keep')
            self.assertIsNone(local_file(root, '../keep.txt'))
            self.assertIsNone(local_file(root, str(outside)))
            prune(root, {'known': ['../keep.txt'], 'snapshots': {}}, 0)
            self.assertTrue(outside.exists())

    def test_cache_categories_and_game_arguments(self):
        self.assertEqual(snapshot_key(['build']), snapshot_key(['run', '--', '--release']))
        self.assertEqual(snapshot_key(['check']), snapshot_key(['clippy']))
        self.assertNotEqual(snapshot_key(['build']), snapshot_key(['test']))
        self.assertNotEqual(snapshot_key(['build']), snapshot_key(['build', '--release']))
        self.assertNotEqual(snapshot_key(['build']), snapshot_key(['build', '--target=x86_64-pc-windows-msvc']))


if __name__ == '__main__':
    unittest.main()
