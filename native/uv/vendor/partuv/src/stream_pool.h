#pragma once
using cudaStream_t = void*;
struct StreamPool {
 static void asyncInitializeStreams(int) { throw std::runtime_error("CUDA is unavailable in CPU mode"); }
 static cudaStream_t getStream(int) { throw std::runtime_error("CUDA is unavailable in CPU mode"); }
};
