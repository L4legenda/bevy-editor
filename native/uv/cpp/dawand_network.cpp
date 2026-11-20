#include "dawand_network.h"
#include <cstring>
#include <nlohmann/json.hpp>
#include <stdexcept>
#include <string>

extern "C" const unsigned char *bevy_uv_dawand_weights(size_t *size);

namespace dawand {
class Network {
  nlohmann::json header;
  std::vector<float> data;
  Eigen::Map<const Features> tensor(const std::string &name) const {
    const auto &desc = header.at(name);
    const auto &shape = desc.at("shape");
    int rows = shape.size() == 2 ? shape.at(0).get<int>() : 1;
    int cols = shape.back().get<int>();
    auto offsets = desc.at("data_offsets").get<std::array<size_t, 2>>();
    if (rows <= 0 || cols <= 0 || offsets[1] > data.size() * sizeof(float) ||
        offsets[0] % sizeof(float) != 0 || offsets[0] > offsets[1] ||
        offsets[1] - offsets[0] != size_t(rows) * cols * sizeof(float))
      throw std::runtime_error("Invalid DA Wand tensor");
    return {data.data() + offsets[0] / sizeof(float), rows, cols};
  }

public:
  Network() {
    size_t length;
    const auto *model = bevy_uv_dawand_weights(&length);
    uint64_t bytes;
    if (length < sizeof(bytes))
      throw std::runtime_error("Invalid DA Wand model");
    std::memcpy(&bytes, model, sizeof(bytes));
    if (bytes > length - sizeof(bytes))
      throw std::runtime_error("Invalid DA Wand header");
    const auto *body = model + sizeof(bytes) + bytes;
    header = nlohmann::json::parse(model + sizeof(bytes), body);
    length -= sizeof(bytes) + bytes;
    if (length % sizeof(float))
      throw std::runtime_error("Invalid DA Wand weights");
    data.resize(length / sizeof(float));
    std::memcpy(data.data(), body, length);
  }
  Features linear(const std::string &name, const Features &input) const {
    auto weight = tensor(name + ".weight");
    auto bias = tensor(name + ".bias");
    if (weight.cols() != input.cols() || bias.cols() != weight.rows())
      throw std::runtime_error("Incompatible DA Wand graph");
    Features output(input.rows(), weight.rows());
    output.noalias() = input * weight.transpose();
    output.rowwise() += bias.row(0);
    return output;
  }
};

static void activate(Features &x) { x = x.cwiseMax(0.01f * x); }
static void layer_norm(Features &x) {
  float mean = x.mean(),
        stddev = std::sqrt((x.array() - mean).square().mean()) + 1e-8f;
  if (stddev > 1e-7f)
    x = ((x.array() - mean) / stddev).matrix();
}

static Features mesh_conv(const Network &net, const std::string &name,
                          const Features &x, const std::vector<Ring> &rings) {
  int c = x.cols();
  Features image(x.rows(), 5 * c);
  for (int i = 0; i < x.rows(); ++i) {
    const auto &r = rings[i];
    auto row = image.row(i);
    row.head(c) = x.row(i);
    row.segment(c, c) = x.row(r[0]) + x.row(r[2]);
    row.segment(2 * c, c) = x.row(r[1]) + x.row(r[3]);
    row.segment(3 * c, c) = (x.row(r[0]) - x.row(r[2])).cwiseAbs();
    row.tail(c) = (x.row(r[1]) - x.row(r[3])).cwiseAbs();
  }
  return net.linear(name + ".conv", image);
}

static Features residual(const Network &net, const std::string &name,
                         const Features &x, const std::vector<Ring> &rings) {
  auto first = mesh_conv(net, name + ".conv0", x, rings);
  auto result = mesh_conv(net, name + ".conv1", first, rings);
  result += first;
  activate(result);
  return result;
}

static Features block(const Network &net, const std::string &name,
                      const Features &input, const std::vector<Ring> &rings,
                      bool last) {
  Features x = residual(net, name + ".conv1", input, rings);
  layer_norm(x);
  activate(x);
  for (int i = 0; i < 3; ++i) {
    auto next = residual(net, name + ".conv2." + std::to_string(i), x, rings);
    layer_norm(next);
    next += x;
    if (!last || i != 2)
      activate(next);
    x = std::move(next);
  }
  return x;
}

Eigen::VectorXf predict(Features x, const std::vector<Ring> &rings,
                        const std::vector<Triangle> &face_edges) {
  static const Network net;
  // Per-mesh dataset standardization followed by the checkpoint's
  // InstanceNormPad.
  for (int c = 0; c < x.cols(); ++c) {
    auto col = x.col(c);
    float mean = col.mean(),
          sigma = std::sqrt((col.array() - mean).square().mean());
    col = (col.array() - (mean > 1e-5f ? mean : 0.f)) /
          (sigma > 1e-5f ? sigma : 1.f);
  }
  Eigen::RowVectorXf means = x.colwise().mean();
  Features centered = x.rowwise() - means;
  Eigen::RowVectorXf stddev =
      centered.array().square().colwise().mean().sqrt() + 1e-5f;
  if ((stddev.array() > 5e-5f).all())
    x = centered.array().rowwise() / stddev.array();
  std::vector<Features> skips;
  for (int i = 0; i < 6; ++i) {
    x = block(net, "encoder.convs." + std::to_string(i), x, rings, false);
    if (i > 0 && i < 5)
      skips.push_back(x);
  }
  for (int i = 0; i < 5; ++i) {
    bool last = i == 4;
    std::string name =
        last ? "decoder.final_conv" : "decoder.up_convs." + std::to_string(i);
    auto up = residual(net, name + ".up_conv", x, rings);
    if (!last) {
      const auto &skip = skips[3 - i];
      Features joined(up.rows(), up.cols() + skip.cols());
      joined << up, skip;
      up = std::move(joined);
    }
    x = block(net, name, up, rings, last);
  }
  Features faces(face_edges.size(), x.cols());
  for (size_t i = 0; i < face_edges.size(); ++i) {
    auto f = face_edges[i];
    faces.row(i) = (x.row(f[0]) + x.row(f[1]) + x.row(f[2])) / 3.f;
  }
  for (int i = 0; i <= 8; i += 2) {
    faces = net.linear("classification.mlp." + std::to_string(i), faces);
    if (i < 8)
      faces = faces.cwiseMax(0.f);
  }
  if (!faces.allFinite())
    throw std::runtime_error("Invalid DA Wand prediction");
  return (1.f + (-faces.col(0).array()).exp()).inverse().matrix();
}
} // namespace dawand
