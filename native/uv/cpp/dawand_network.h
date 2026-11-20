// Inference graph of the published DA Wand MeshCNN checkpoint.
#pragma once
#include <Eigen/Core>
#include <array>
#include <vector>

namespace dawand {
using Features =
    Eigen::Matrix<float, Eigen::Dynamic, Eigen::Dynamic, Eigen::RowMajor>;
using Triangle = std::array<int, 3>;
using Ring = std::array<int, 4>;
Eigen::VectorXf predict(Features features, const std::vector<Ring> &rings,
                        const std::vector<Triangle> &face_edges);
} // namespace dawand
