// Native PartUV CPU + xatlas adapter. No Python ABI.
#include "pipeline.h"
#include <Eigen/SVD>
#include <array>
#include <cstring>
#include <deque>
#include <memory>
#include <nlohmann/json.hpp>
#include <numeric>
#include <xatlas.h>
using Json = nlohmann::json;
using UV = std::array<float, 2>;
using Tri = std::array<uint32_t, 3>;
using FaceUV = std::array<std::array<double, 2>, 3>;
using Progress = void (*)(int);
Json suggest_island(const Json &, Progress);
struct Chart {
  std::vector<int> ids;
  std::vector<UV> uv;
  std::vector<Tri> faces, valid;
};
struct Packing {
  std::vector<FaceUV> uv;
  size_t charts;
  double utilization;
  double area = 0.;
};
struct Union {
  std::vector<int> p;
  Union(int n) : p(n) { std::iota(p.begin(), p.end(), 0); }
  int root(int i) {
    while (p[i] != i) {
      p[i] = p[p[i]];
      i = p[i];
    }
    return i;
  }
  void join(int a, int b) { p[root(a)] = root(b); }
};
static Eigen::Vector2d point(const UV &p) { return {p[0], p[1]}; }
static double area(const Eigen::Vector2d &a, const Eigen::Vector2d &b,
                   const Eigen::Vector2d &c) {
  auto d = (b - a).eval();
  auto e = (c - a).eval();
  return std::abs(d.x() * e.y() - d.y() * e.x());
}
static Packing pack(const std::vector<UV> &uv, const std::vector<Tri> &faces,
                    int resolution, int padding, bool refine, Progress progress) {
  if (progress)
    progress(80);
  Union u((int)faces.size());
  std::map<std::pair<uint32_t, uint32_t>, int> edges;
  for (int i = 0; i < (int)faces.size(); ++i)
    for (int c = 0; c < 3; ++c) {
      auto e = std::minmax(faces[i][c], faces[i][(c + 1) % 3]);
      auto k = std::make_pair(e.first, e.second);
      auto old = edges.find(k);
      if (old != edges.end())
        u.join(i, old->second);
      else
        edges[k] = i;
    }
  std::vector<Chart> charts;
  std::map<int, size_t> group;
  for (int i = 0; i < (int)faces.size(); ++i) {
    int r = u.root(i);
    if (!group.count(r)) {
      group[r] = charts.size();
      charts.emplace_back();
    }
    charts[group[r]].ids.push_back(i);
  }
  for (auto &ch : charts) {
    std::set<uint32_t> unique;
    for (int i : ch.ids)
      unique.insert(faces[i].begin(), faces[i].end());
    std::map<uint32_t, uint32_t> local;
    for (uint32_t id : unique) {
      local[id] = (uint32_t)ch.uv.size();
      ch.uv.push_back(uv[id]);
    }
    // PartUV may place tiny charts far from the origin. Recenter before float
    // packing so their rotation does not lose precision to a large translation.
    UV offset = ch.uv.front();
    for (auto &p : ch.uv)
      for (int k = 0; k < 2; ++k)
        offset[k] = std::min(offset[k], p[k]);
    for (auto &p : ch.uv)
      for (int k = 0; k < 2; ++k)
        p[k] -= offset[k];
    double largest = 0.;
    std::vector<double> areas;
    for (int i : ch.ids) {
      Tri f;
      for (int k = 0; k < 3; ++k)
        f[k] = local[faces[i][k]];
      ch.faces.push_back(f);
      double a =
          area(point(ch.uv[f[0]]), point(ch.uv[f[1]]), point(ch.uv[f[2]]));
      areas.push_back(a);
      largest = std::max(largest, a);
    }
    for (size_t i = 0; i < ch.faces.size(); ++i)
      if (areas[i] > std::max(largest * 1e-10, 1e-24))
        ch.valid.push_back(ch.faces[i]);
  }
  auto attempt = [&](bool rectangles, int orientation) {
    std::unique_ptr<xatlas::Atlas, void (*)(xatlas::Atlas *)> atlas(
        xatlas::Create(), xatlas::Destroy);
    std::vector<std::vector<UV>> sources;
    std::vector<std::vector<Tri>> triangles;
    for (auto &ch : charts) {
      std::vector<UV> src = ch.uv;
      std::vector<Tri> fs = ch.faces;
      if (rectangles || ch.valid.empty()) {
        Eigen::Vector2f low = Eigen::Vector2f::Constant(INFINITY),
                        high = Eigen::Vector2f::Constant(-INFINITY);
        for (auto &q : ch.uv) {
          low = low.cwiseMin(Eigen::Vector2f(q[0], q[1]));
          high = high.cwiseMax(Eigen::Vector2f(q[0], q[1]));
        }
        float extent = std::max((high - low).maxCoeff(), 1e-4f);
        high = high.cwiseMax((low.array() + extent * 1e-4f).matrix());
        src = {{low.x(), low.y()},
               {high.x(), low.y()},
               {high.x(), high.y()},
               {low.x(), high.y()}};
        fs = {Tri{0, 1, 2}, Tri{0, 2, 3}};
      }
      xatlas::UvMeshDecl decl;
      decl.preserveInputChart = true;
      decl.vertexCount = (uint32_t)src.size();
      decl.vertexUvData = src.data();
      decl.vertexStride = sizeof(UV);
      decl.indexData = fs.data();
      decl.indexCount = (uint32_t)fs.size() * 3;
      decl.indexFormat = xatlas::IndexFormat::UInt32;
      if (xatlas::AddUvMesh(atlas.get(), decl) != xatlas::AddMeshError::Success)
        throw std::runtime_error("xatlas rejected UV mesh");
      sources.push_back(std::move(src));
      triangles.push_back(std::move(fs));
    }
    xatlas::PackOptions options;
    options.preserveChartShape = true;
    options.resolution = resolution;
    options.padding = padding;
    // Exhaustive pixel search on concave masks can take minutes per island.
    // Try original and hull-aligned orientations with bounded random placement.
    options.bruteForce = orientation == 3;
    options.blockAlign = orientation == 3;
    options.rotateChartsToAxis = orientation == 0 || orientation == 3;
    options.rotateAlternateCharts = orientation == 2;
    xatlas::Generate(atlas.get(), xatlas::ChartOptions{}, options);
    if (atlas->atlasCount != 1 || atlas->meshCount != charts.size())
      throw std::runtime_error("Packing requires exactly one atlas");
    double side = std::max(atlas->width, atlas->height);
    if (side <= 0)
      throw std::runtime_error("Empty packed atlas");
    Packing out;
    out.uv.resize(faces.size());
    out.charts = charts.size();
    out.utilization =
        atlas->utilization[0] * atlas->width * atlas->height / (side * side);
    double sum_area = 0.;
    for (size_t i = 0; i < charts.size(); ++i) {
      auto &ch = charts[i];
      auto &src = sources[i];
      auto &fs = triangles[i];
      auto &mesh = atlas->meshes[i];
      if (mesh.indexCount != fs.size() * 3)
        throw std::runtime_error("Packing changed face count");
      if (mesh.chartCount != 1 || mesh.vertexCount != src.size())
        throw std::runtime_error("Packing split continuous chart");
      for (size_t f = 0; f < fs.size(); ++f)
        for (int c = 0; c < 3; ++c)
          if (mesh.vertexArray[mesh.indexArray[f * 3 + c]].xref != fs[f][c])
            throw std::runtime_error("Packing changed face order");
      // Fit the rotation using all chart vertices. A single triangle can be
      // almost collinear, even when it has the largest area in a tiny island.
      Eigen::Vector2d origin = Eigen::Vector2d::Zero(),
                      destination = Eigen::Vector2d::Zero();
      for (size_t v = 0; v < src.size(); ++v) {
        origin += point(src[v]);
        destination += Eigen::Vector2d(mesh.vertexArray[v].uv[0] / side,
                                       mesh.vertexArray[v].uv[1] / side);
      }
      origin /= double(src.size());
      destination /= double(src.size());
      Eigen::Matrix2d covariance = Eigen::Matrix2d::Zero();
      for (size_t v = 0; v < src.size(); ++v) {
        Eigen::Vector2d q(mesh.vertexArray[v].uv[0] / side,
                          mesh.vertexArray[v].uv[1] / side);
        covariance += (q - destination) * (point(src[v]) - origin).transpose();
      }
      Eigen::JacobiSVD<Eigen::Matrix2d> rigid(
          covariance, Eigen::ComputeFullU | Eigen::ComputeFullV);
      // One common scale preserves relative texel density, including slivers.
      Eigen::Matrix2d matrix = rigid.matrixU() * rigid.matrixV().transpose() *
                               (atlas->texelsPerUnit / side);
      for (size_t v = 0; v < src.size(); ++v) {
        Eigen::Vector2d q(mesh.vertexArray[v].uv[0] / side,
                          mesh.vertexArray[v].uv[1] / side);
        const double residual =
            (matrix * (point(src[v]) - origin) + destination - q).norm();
        if (!matrix.allFinite() || residual > std::max(3e-6, 0.05 / side))
          throw std::runtime_error("Packing changed island shape at chart " +
                                   std::to_string(i) + ": " +
                                   std::to_string(residual));
      }
      for (size_t f = 0; f < ch.faces.size(); ++f) {
        std::array<Eigen::Vector2d, 3> ps;
        for (int c = 0; c < 3; ++c) {
          ps[c] =
              matrix * (point(ch.uv[ch.faces[f][c]]) - origin) + destination;
          if (!ps[c].allFinite() || ps[c].minCoeff() < -.00001 ||
              ps[c].maxCoeff() > 1.00001)
            throw std::runtime_error("Packed UV outside atlas");
          out.uv[ch.ids[f]][c] = {ps[c].x(), ps[c].y()};
        }
        sum_area += area(ps[0], ps[1], ps[2]) * .5;
      }
    }
    if (sum_area > 1.0001)
      throw std::runtime_error("Packed UV area exceeds atlas");
    out.area = sum_area;
    return out;
  };
  bool rectangles = false;
  Packing best;
  try {
    best = attempt(false, false);
  } catch (const std::exception &e) {
    // Convex-hull axis fitting is unstable on subpixel islands. Preserve the
    // actual contours and try their existing axes before reserving rectangles.
    try {
      best = attempt(false, 1);
    } catch (const std::exception &) {
      std::cerr << "Contour packing fallback: " << e.what() << std::endl;
      rectangles = true;
      if (progress)
        progress(85);
      best = attempt(true, 1);
    }
  }
  // Search an additional placement for ordinary game assets. Always retain the
  // better valid atlas, so the extra search cannot degrade the default result.
  if (refine && charts.size() <= 256 && faces.size() <= 100000) {
    if (progress)
      progress(87);
    for (int orientation : {1, 2, 3}) {
      try {
        auto candidate = attempt(rectangles, orientation);
        if (candidate.area > best.area)
          best = std::move(candidate);
      } catch (const std::exception &) {
      }
    }
  }
  return best;
}
using Point = std::array<double, 3>;
using TriangleKey = std::array<Point, 3>;
static Json repack(const Json &request, Progress progress) {
  auto faces = request.at("triangles").get<std::vector<Tri>>();
  std::vector<UV> uv;
  std::vector<Tri> uvfaces;
  auto existing = request.at("existing_uv").get<std::vector<FaceUV>>();
  if (existing.size() != faces.size())
    throw std::runtime_error("Incomplete existing UV");
  using Key = std::tuple<uint32_t, float, float>;
  std::map<Key, uint32_t> unique;
  for (size_t i = 0; i < faces.size(); ++i)
    for (int c = 0; c < 3; ++c)
      unique[{faces[i][c], float(existing[i][c][0]),
              float(existing[i][c][1])}] = 0;
  for (auto &kv : unique) {
    kv.second = (uint32_t)uv.size();
    uv.push_back({std::get<1>(kv.first), std::get<2>(kv.first)});
  }
  for (size_t i = 0; i < faces.size(); ++i) {
    Tri f;
    for (int c = 0; c < 3; ++c)
      f[c] = unique.at(
          {faces[i][c], float(existing[i][c][0]), float(existing[i][c][1])});
    uvfaces.push_back(f);
  }
  for (auto &p : uv)
    if (!std::isfinite(p[0]) || !std::isfinite(p[1]))
      throw std::runtime_error("Non-finite source UV");
  auto packed = pack(uv, uvfaces, request.at("resolution"),
                     request.at("padding"), true, progress);
  return Json{{"uv", packed.uv},
              {"charts", packed.charts},
              {"utilization", packed.utilization}};
}
// Keep the editor triangle/corner order while assigning each independently
// solved region its own UV indices. Packing all regions together preserves
// texel density.
static void unwrap_part(const Json &part, double threshold,
                        bool balance_density, std::vector<UV> &uv,
                        std::vector<Tri> &output_faces) {
  auto vertices = part.at("positions").get<std::vector<Point>>();
  auto faces = part.at("triangles").get<std::vector<Tri>>();
  auto ids = part.at("ids").get<std::vector<size_t>>();
  uint32_t offset = static_cast<uint32_t>(uv.size());
  if (faces.size() == 1) {
    auto position = [&](int k) {
      return Eigen::Map<const Eigen::Vector3d>(vertices[faces[0][k]].data());
    };
    Eigen::Vector3d u = position(1) - position(0),
                    v = position(2) - position(0);
    if (u.norm() == 0 || u.cross(v).norm() == 0)
      throw std::runtime_error("Degenerate source triangle");
    uv.push_back({0, 0});
    uv.push_back({float(u.norm()), 0});
    uv.push_back(
        {float(u.dot(v) / u.norm()), float(u.cross(v).norm() / u.norm())});
    output_faces[ids.at(0)] = {offset, offset + 1, offset + 2};
    return;
  }
  Eigen::MatrixXd V(vertices.size(), 3);
  Eigen::MatrixXi F(faces.size(), 3);
  for (size_t i = 0; i < vertices.size(); ++i)
    for (int c = 0; c < 3; ++c)
      V(i, c) = vertices[i][c];
  for (size_t i = 0; i < faces.size(); ++i)
    for (int c = 0; c < 3; ++c)
      F(i, c) = faces[i][c];
  std::vector<NodeRecord> nodes;
  for (auto &v : part.at("tree"))
    nodes.push_back({v[0].get<int>(), v[1].get<int>(), v[2].get<int>()});
  std::vector<UVParts> parts;
  auto result =
      pipeline(V, F, nodes, "", threshold, false, parts).to_components();
  if (result.F.rows() != (int)faces.size() ||
      result.UV.rows() != result.V.rows())
    throw std::runtime_error("PartUV changed mesh coverage");
  double scale = 1.;
  if (balance_density) {
    double surface = 0., flat = 0.;
    for (int i = 0; i < result.F.rows(); ++i) {
      auto a = result.F(i, 0), b = result.F(i, 1), c = result.F(i, 2);
      Eigen::Vector3d u = result.V.row(b) - result.V.row(a),
                      v = result.V.row(c) - result.V.row(a);
      Eigen::Vector2d x = result.UV.row(b) - result.UV.row(a),
                      y = result.UV.row(c) - result.UV.row(a);
      surface += u.cross(v).norm();
      flat += std::abs(x.x() * y.y() - x.y() * y.x());
    }
    if (flat > 0.)
      scale = std::sqrt(surface / flat);
  }
  for (int i = 0; i < result.UV.rows(); ++i) {
    UV coord{float(scale * result.UV(i, 0)), float(scale * result.UV(i, 1))};
    if (!std::isfinite(coord[0]) || !std::isfinite(coord[1]))
      throw std::runtime_error("Non-finite PartUV result");
    uv.push_back(coord);
  }
  std::map<TriangleKey, std::deque<size_t>> original;
  for (size_t i = 0; i < faces.size(); ++i) {
    TriangleKey key = {vertices[faces[i][0]], vertices[faces[i][1]],
                       vertices[faces[i][2]]};
    std::sort(key.begin(), key.end());
    original[key].push_back(i);
  }

  for (size_t i = 0; i < faces.size(); ++i) {
    TriangleKey points;
    for (int c = 0; c < 3; ++c)
      for (int k = 0; k < 3; ++k)
        points[c][k] = result.V(result.F(i, c), k);
    auto key = points;
    std::sort(key.begin(), key.end());
    auto found = original.find(key);
    if (found == original.end() || found->second.empty())
      throw std::runtime_error("PartUV changed geometry");
    auto fi = found->second.front();
    found->second.pop_front();
    for (int c = 0; c < 3; ++c) {
      int match = -1;
      for (int k = 0; k < 3; ++k)
        if (points[k] == vertices[faces[fi][c]]) {
          if (match >= 0)
            throw std::runtime_error("Degenerate source triangle");
          match = k;
        }
      if (match < 0)
        throw std::runtime_error("PartUV changed corner");
      output_faces[ids[fi]][c] = offset + result.F(i, match);
    }
  }
  for (auto &kv : original)
    if (!kv.second.empty())
      throw std::runtime_error("PartUV omitted triangles");
}
static Json preview(const Json &request, Progress progress) {
  auto vertices =
      request.at("positions").get<std::vector<std::array<float, 3>>>();
  auto faces = request.at("triangles").get<std::vector<Tri>>();
  auto regions = request.at("groups").get<std::vector<uint32_t>>();
  std::unique_ptr<xatlas::Atlas, decltype(&xatlas::Destroy)> atlas(
      xatlas::Create(), xatlas::Destroy);
  xatlas::MeshDecl mesh;
  mesh.vertexPositionData = vertices.data();
  mesh.vertexCount = static_cast<uint32_t>(vertices.size());
  mesh.vertexPositionStride = sizeof(vertices.front());
  mesh.indexData = faces.data();
  mesh.indexCount = static_cast<uint32_t>(faces.size() * 3);
  mesh.indexFormat = xatlas::IndexFormat::UInt32;
  mesh.faceMaterialData = regions.data();
  if (xatlas::AddMesh(atlas.get(), mesh) != xatlas::AddMeshError::Success)
    throw std::runtime_error("xatlas rejected surface");
  xatlas::ComputeCharts(atlas.get());
  if (progress)
    progress(80);
  xatlas::PackOptions settings;
  settings.resolution = request.at("resolution");
  settings.padding = request.at("padding");
  settings.preserveChartShape = true;
  xatlas::PackCharts(atlas.get(), settings);
  if (atlas->meshCount != 1 || atlas->atlasCount != 1 ||
      atlas->meshes[0].indexCount != mesh.indexCount)
    throw std::runtime_error("Incomplete xatlas preview");
  const auto &out = atlas->meshes[0];
  double side = std::max(atlas->width, atlas->height);
  std::vector<FaceUV> ordered(faces.size());
  for (size_t i = 0; i < faces.size(); ++i)
    for (size_t c = 0; c < 3; ++c) {
      const auto &v = out.vertexArray[out.indexArray[i * 3 + c]];
      if (v.xref != faces[i][c] || v.atlasIndex != 0)
        throw std::runtime_error("xatlas changed source corners");
      ordered[i][c] = {v.uv[0] / side, v.uv[1] / side};
    }
  return Json{{"uv", ordered},
              {"charts", atlas->chartCount},
              {"utilization", atlas->utilization[0] * atlas->width *
                                  atlas->height / (side * side)}};
}
static Json process(const Json &request, Progress progress) {
  if (request.contains("existing_uv"))
    return repack(request, progress);
  if (request.at("method") == "preview")
    return preview(request, progress);
  if (request.at("method") == "suggest")
    return suggest_island(request, progress);
  std::vector<UV> uv;
  std::vector<Tri> faces(request.at("count").get<size_t>());
  const auto &parts = request.at("parts");
  for (const auto &part : parts)
    unwrap_part(part, request.at("threshold"), parts.size() > 1, uv, faces);
  auto packed = pack(uv, faces, request.at("resolution"), request.at("padding"),
                     false, progress);
  return Json{{"uv", packed.uv},
              {"charts", packed.charts},
              {"utilization", packed.utilization}};
}
extern "C" {
struct NativeBuffer {
  char *data;
  size_t length;
  int error;
};
NativeBuffer bevy_uv_process(const char *input, size_t length,
                             Progress progress) noexcept {
  std::string data;
  int error = 0;
  try {
    data = process(Json::parse(input, input + length), progress).dump();
  } catch (const std::exception &e) {
    data = e.what();
    error = 1;
  } catch (...) {
    data = "Unknown native UV failure";
    error = 1;
  }
  char *buffer = static_cast<char *>(std::malloc(data.size() + 1));
  if (!buffer)
    return {nullptr, 0, 1};
  std::memcpy(buffer, data.c_str(), data.size() + 1);
  return {buffer, data.size(), error};
}
void bevy_uv_free(NativeBuffer b) noexcept { std::free(b.data); }
}
