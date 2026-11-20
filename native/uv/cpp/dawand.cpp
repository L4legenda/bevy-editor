#include "dawand_network.h"
#include <Eigen/Eigenvalues>
#include <Eigen/QR>
#include <Eigen/SparseCholesky>
#include <algorithm>
#include <cmath>
#include <deque>
#include <map>
#include <nlohmann/json.hpp>
#include <random>
#include <stdexcept>

namespace {
using dawand::Triangle;
using Edge = std::array<int, 2>;
using Sparse = Eigen::SparseMatrix<double>;
using Matrix = Eigen::MatrixXd;
using Json = nlohmann::json;
using Point = std::array<double, 3>;
Edge edge(int a, int b) { return {std::min(a, b), std::max(a, b)}; }

// Bound both spectral work and inference memory on high-resolution assets.
// Keep the original triangle IDs for transferring the prediction back to the
// document.
std::vector<int> neighborhood(const std::vector<Triangle> &faces, int seed) {
  std::map<Edge, std::array<int, 2>> incident;
  for (int i = 0; i < int(faces.size()); ++i) {
    for (int k = 0; k < 3; ++k) {
      auto key = edge(faces[i][k], faces[i][(k + 1) % 3]);
      auto [it, added] = incident.emplace(key, std::array<int, 2>{i, -1});
      if (!added)
        it->second[1] =
            it->second[1] == -1 ? i : -2; // A nonmanifold fan is a boundary.
    }
  }
  std::vector<int> ids{seed};
  std::vector<bool> seen(faces.size());
  seen[seed] = true;
  constexpr size_t max_faces = 8000;
  for (size_t j = 0; j < ids.size() && ids.size() < max_faces; ++j) {
    const auto &f = faces[ids[j]];
    for (int k = 0; k < 3 && ids.size() < max_faces; ++k) {
      auto adj = incident.at(edge(f[k], f[(k + 1) % 3]));
      if (adj[1] < 0)
        continue;
      int next = adj[0] == ids[j] ? adj[1] : adj[0];
      if (!seen[next]) {
        seen[next] = true;
        ids.push_back(next);
      }
    }
  }
  return ids;
}

// Heat-kernel signature of the barycentric-mass cotangent Laplacian.
// Rayleigh–Ritz block inverse iteration keeps large meshes sparse.
Matrix signatures(const Matrix &v, const std::vector<Triangle> &faces) {
  const int n = v.rows(), modes = std::min(n, 128);
  Eigen::VectorXd mass = Eigen::VectorXd::Zero(n);
  std::vector<Eigen::Triplet<double>> coefficients;
  for (const auto &f : faces) {
    Eigen::Vector3d a = v.row(f[1]) - v.row(f[0]),
                    b = v.row(f[2]) - v.row(f[0]);
    double twice_area = a.cross(b).norm();
    if (twice_area <= 0.)
      throw std::runtime_error("DA Wand: вырожденная грань");
    for (int j = 0; j < 3; ++j) {
      int i = f[j], u = f[(j + 1) % 3], w = f[(j + 2) % 3];
      mass[i] += twice_area / 6.;
      double cot = 0.5 * (v.row(u) - v.row(i)).dot(v.row(w) - v.row(i)) /
                   std::max(twice_area, 1e-10);
      coefficients.emplace_back(u, w, -cot);
      coefficients.emplace_back(w, u, -cot);
      coefficients.emplace_back(u, u, cot);
      coefficients.emplace_back(w, w, cot);
    }
  }
  mass.array() += 1e-8 * mass.mean();
  const Eigen::VectorXd inv = mass.array().rsqrt();
  Sparse lap(n, n);
  lap.setFromTriplets(coefficients.begin(), coefficients.end());
  lap.diagonal().array() += 1e-8;
  Sparse normalized = inv.asDiagonal() * lap * inv.asDiagonal();
  Matrix basis;
  Eigen::VectorXd values;
  if (n <= 192) {
    Eigen::SelfAdjointEigenSolver<Matrix> solver{Matrix(normalized)};
    if (solver.info() != Eigen::Success)
      throw std::runtime_error("DA Wand: сбой спектрального анализа");
    basis = solver.eigenvectors().leftCols(modes);
    values = solver.eigenvalues().head(modes);
  } else {
    int width = std::min(n, modes + 32);
    Sparse shifted = normalized;
    shifted.diagonal().array() += 1e-5;
    Eigen::SimplicialLDLT<Sparse> inverse(shifted);
    if (inverse.info() != Eigen::Success)
      throw std::runtime_error("DA Wand: сбой факторизации поверхности");
    std::mt19937 rng(42);
    std::normal_distribution<double> normal;
    Matrix q = Matrix::NullaryExpr(n, width, [&]() { return normal(rng); });
    Matrix eye = Matrix::Identity(n, width);
    for (int i = 0; i < 7; ++i) {
      Matrix solved = inverse.solve(q);
      q = solved.householderQr().householderQ() * eye;
    }
    Eigen::SelfAdjointEigenSolver<Matrix> reduced(q.transpose() *
                                                  (normalized * q));
    if (reduced.info() != Eigen::Success)
      throw std::runtime_error("DA Wand: сбой спектрального анализа");
    basis = q * reduced.eigenvectors().leftCols(modes);
    values = reduced.eigenvalues().head(modes);
  }
  basis = inv.asDiagonal() * basis;
  Matrix decay(modes, 16);
  for (int t = 0; t < 16; ++t)
    decay.col(t) =
        (-std::pow(10., -2. + 2. * t / 15.) * values.cwiseMax(0.).array())
            .exp();
  return basis.array().square().matrix() * decay;
}
} // namespace

nlohmann::json suggest_island(const nlohmann::json &request,
                              void (*progress)(int)) {
  auto all = request.at("triangles").get<std::vector<Triangle>>();
  auto points = request.at("positions").get<std::vector<Point>>();
  int seed = request.at("seed");
  const auto ids = neighborhood(all, seed);
  std::vector<Triangle> faces;
  std::map<int, int> vertex_ids;
  for (int id : ids) {
    Triangle f = all[id];
    for (int &v : f)
      v = vertex_ids.emplace(v, int(vertex_ids.size())).first->second;
    faces.push_back(f);
  }
  Matrix vertices(vertex_ids.size(), 3);
  for (auto [source, i] : vertex_ids)
    vertices.row(i) =
        Eigen::Map<const Eigen::RowVector3d>(points[source].data());
  vertices.rowwise() -= vertices.colwise().mean().eval();
  double extent = vertices.rowwise().norm().maxCoeff();
  if (!(extent > 0.))
    throw std::runtime_error("DA Wand: поверхность пуста");
  vertices /= extent;
  const auto hks = signatures(vertices, faces);
  struct LocalEdge {
    Edge vertices;
    std::array<int, 2> faces{-1, -1};
    dawand::Ring ring{};
  };
  std::map<Edge, int> edge_ids;
  std::vector<LocalEdge> edges;
  std::vector<Triangle> face_edges(faces.size());
  std::vector<Eigen::Vector3d> normals;
  for (size_t i = 0; i < faces.size(); ++i) {
    auto f = faces[i];
    Eigen::Vector3d a = vertices.row(f[1]) - vertices.row(f[0]),
                    b = vertices.row(f[2]) - vertices.row(f[0]);
    normals.push_back(a.cross(b).normalized());
    for (int k = 0; k < 3; ++k) {
      auto key = edge(f[k], f[(k + 1) % 3]);
      auto [it, added] = edge_ids.emplace(key, int(edges.size()));
      if (added)
        edges.push_back({key});
      face_edges[i][k] = it->second;
    }
  }
  for (int i = 0; i < int(faces.size()); ++i) {
    auto f = face_edges[i];
    for (int k = 0; k < 3; ++k) {
      auto &e = edges[f[k]];
      int side = e.faces[0] >= 0;
      if (e.faces[side] >= 0)
        throw std::runtime_error(
            "DA Wand: неманифолдное ребро в выбранной области");
      e.faces[side] = i;
      e.ring[2 * side] = f[(k + 1) % 3];
      e.ring[2 * side + 1] = f[(k + 2) % 3];
    }
  }
  dawand::Features features(edges.size(), 18);
  std::vector<dawand::Ring> rings;
  for (size_t i = 0; i < edges.size(); ++i) {
    auto &e = edges[i];
    int a = e.faces[0], b = e.faces[1];
    // Reflect the incident triangle at an open edge for a valid four-neighbor
    // stencil.
    if (b < 0) {
      b = a;
      e.ring[2] = e.ring[1];
      e.ring[3] = e.ring[0];
    }
    rings.push_back(e.ring);
    features(i, 0) =
        float(std::acos(-1.) -
              std::acos(std::clamp(normals[a].dot(normals[b]), -1., 1.)));
    features(i, 1) = (a == 0 || b == 0) ? 1.f : 0.f;
    features.row(i).tail(16) =
        ((hks.row(e.vertices[0]) + hks.row(e.vertices[1])) * .5).cast<float>();
  }
  if (progress)
    progress(60);
  auto scores = dawand::predict(std::move(features), rings, face_edges);
  // A suggestion is always connected to the clicked face, including on noisy
  // meshes.
  std::vector<int> patch{0};
  std::vector<bool> chosen(faces.size());
  chosen[0] = true;
  for (size_t i = 0; i < patch.size(); ++i) {
    for (int edge_id : face_edges[patch[i]]) {
      for (int next : edges[edge_id].faces) {
        if (next >= 0 && !chosen[next] && scores[next] >= 0.5f) {
          chosen[next] = true;
          patch.push_back(next);
        }
      }
    }
  }
  for (int &id : patch)
    id = ids[id];
  std::sort(patch.begin(), patch.end());
  return Json{{"suggestion", patch}};
}
