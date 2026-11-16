"""Regression for UV slivers: packing must preserve every chart's shape."""
import unittest
import numpy as np
from partuv_worker import pack_rigid


class PackingTest(unittest.TestCase):
    def test_sliver_and_collapsed_corners_follow_the_whole_chart(self):
        uv = np.array([[0, 0], [1, 0], [1, 1], [0, 1], [1.5, 1e-10],
                       [0, 0], [0.4, 0], [0.4, 0.2], [0, 0.2]], dtype=np.float32)
        faces = np.array([[0, 1, 2], [0, 2, 3], [0, 4, 1],
                          [5, 6, 7], [5, 7, 8]], dtype=np.uint32)
        result, count, _ = pack_rigid(uv, faces, {"resolution": 1024, "padding": 8})
        self.assertEqual(count, 2)
        self.assertTrue(np.isfinite(result).all())
        self.assertGreaterEqual(result.min(), -1e-5)
        self.assertLessEqual(result.max(), 1.00001)
        for group in [slice(0, 3), slice(3, 5)]:
            before = uv[faces[group]].astype(np.float64).reshape(-1, 2)
            after = result[group].reshape(-1, 2)
            distances = np.linalg.norm(before[:, None] - before[None, :], axis=-1)
            packed = np.linalg.norm(after[:, None] - after[None, :], axis=-1)
            ratios = packed[distances > 1e-6] / distances[distances > 1e-6]
            self.assertTrue(np.allclose(ratios, np.median(ratios), rtol=.001))


if __name__ == "__main__":
    unittest.main()
