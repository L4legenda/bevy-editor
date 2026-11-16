#pragma once

// Native fixed Geometric/ABF configuration; UI threshold is passed separately.
#include <string>
#include <iostream>
#include <vector>

class ConfigManager {
public:
    // Public configuration parameters.
    std::string meshPath;

    bool log_traverse_csv;

    bool pipelineOverlaps;
    double      pipelineThreshold;
    // std::vector<std::string> pipelineSimpleMethods;
    // std::vector<std::string> pipelineFullMethods;

    // New pipeline flags.
    bool checkSelfIntersection;
    bool checkNon2Manifold;

    std::string unwrapMethod;
    int         unwrapAbfIters;
    int         unwrapAggParts;   // NEW: Default set to 10
    bool        unwrapPamo;
    int         unwrapUsePamoFaceThreshold; // use pamo if the face count is greater than this threshold (default: 1000)

    // Global verbose flag.
    bool verbose;

    int componentMaxDepth;
    int parallelDepth;
    // New save_stuff flag.
    bool saveStuff;

    // Threading configuration.
    int num_omp_threads;
    int num_cuda_streams;

    // New output path.
    std::string outputPath;

    // Default constructor with default values.
    ConfigManager()
        : meshPath("default_mesh_path"),
          pipelineOverlaps(true),
          pipelineThreshold(1.25),
          checkSelfIntersection(false),
          checkNon2Manifold(false),
          unwrapMethod("abf"),
          unwrapAbfIters(10),
          unwrapPamo(false),
          unwrapAggParts(10),
          verbose(false),
          componentMaxDepth(10),
          parallelDepth(10),
          saveStuff(false),
          log_traverse_csv(false),
          num_omp_threads(8),
          num_cuda_streams(10),
          unwrapUsePamoFaceThreshold(1000)
    {}

    // Singleton accessor: created on first use.
    static ConfigManager& instance() {
        static ConfigManager instance;
        return instance;
    }

    // Load configuration from a YAML file (call once at startup).
    void loadFromFile(const std::string& filename) {
 unwrapMethod="abf"; unwrapAbfIters=5; unwrapPamo=false; unwrapAggParts=10;
 pipelineOverlaps=true; checkSelfIntersection=false; checkNon2Manifold=false;
 verbose=false; saveStuff=false; log_traverse_csv=false; componentMaxDepth=10;
 parallelDepth=6; num_omp_threads=6; num_cuda_streams=1;
 }

    void setMeshPath(const std::string& path) {
        meshPath = path;
        outputPath = path.substr(0, path.find_last_of('/')) + "/output/";
    }
    // Function to print all configuration parameters.
    void printConfigs() const {
        std::cout << "\n[CONFIG] meshPath: " << meshPath << "\n";
        
        if (log_traverse_csv)
            std::cout << "[CONFIG] log_traverse_csv: " << log_traverse_csv << "\n";
        
        std::cout << "[CONFIG] pipeline.overlaps: " << std::boolalpha << pipelineOverlaps << "\n";
        std::cout << "[CONFIG] pipeline.threshold: " << pipelineThreshold << "\n";

        // std::cout << "[CONFIG] pipeline.simple_methods: ";
        // for (const auto& method : pipelineSimpleMethods) {
        //     std::cout << method << " ";
        // }
        // std::cout << "\n";

        // std::cout << "[CONFIG] pipeline.full_methods: ";
        // for (const auto& method : pipelineFullMethods) {
        //     std::cout << method << " ";
        // }
        // std::cout << "\n";

        std::cout << "[CONFIG] pipeline.checkSelfIntersection: " << std::boolalpha << checkSelfIntersection << "\n";
        std::cout << "[CONFIG] pipeline.checkNon2Manifold: " << std::boolalpha << checkNon2Manifold << "\n";

        std::cout << "[CONFIG] unwrap.method: " << unwrapMethod << "\n";
        std::cout << "[CONFIG] unwrap.abf_iters: " << unwrapAbfIters << "\n";
        std::cout << "[CONFIG] unwrap.pamo: " << std::boolalpha << unwrapPamo << "\n";
        std::cout << "[CONFIG] unwrap.usePamoFaceThreshold: " << unwrapUsePamoFaceThreshold << "\n";
        std::cout << "[CONFIG] unwrap.agg_parts: " << unwrapAggParts << "\n";  // NEW: Print unwrapAggParts

        std::cout << "[CONFIG] verbose: " << std::boolalpha << verbose << "\n";
        std::cout << "[CONFIG] save_stuff: " << std::boolalpha << saveStuff << "\n";
        std::cout << "[CONFIG] component_maxDepth: " << componentMaxDepth << "\n"; 
        std::cout << "[CONFIG] parallelDepth: " << parallelDepth << "\n";
        std::cout << "[CONFIG] output_path: " << outputPath << "\n";
        std::cout << "[CONFIG] num_omp_threads: " << num_omp_threads << "\n";
        std::cout << "[CONFIG] num_cuda_streams: " << num_cuda_streams << "\n";
    }

private:
    // Private constructor enforces the singleton pattern.
    // ConfigManager() = default;
};

// Global inline variables for easy access (requires C++17 or later).
inline std::string& CONFIG_meshPath             = ConfigManager::instance().meshPath;
inline bool&        CONFIG_log_traverse_csv     = ConfigManager::instance().log_traverse_csv;

inline bool&        CONFIG_pipelineOverlaps     = ConfigManager::instance().pipelineOverlaps;
inline double&      CONFIG_pipelineThreshold    = ConfigManager::instance().pipelineThreshold;
inline bool&        CONFIG_checkSelfIntersection = ConfigManager::instance().checkSelfIntersection;
inline bool&        CONFIG_checkNon2Manifold     = ConfigManager::instance().checkNon2Manifold;
inline std::string& CONFIG_unwrapMethod         = ConfigManager::instance().unwrapMethod;
inline int&         CONFIG_unwrapAbfIters       = ConfigManager::instance().unwrapAbfIters;
inline bool&        CONFIG_unwrapPamo           = ConfigManager::instance().unwrapPamo;
inline int&         CONFIG_unwrapUsePamoFaceThreshold = ConfigManager::instance().unwrapUsePamoFaceThreshold;
// End of Selection

inline int&         CONFIG_unwrapAggParts       = ConfigManager::instance().unwrapAggParts;  // NEW: Global inline variable
inline bool&        CONFIG_verbose              = ConfigManager::instance().verbose;
inline bool&        CONFIG_saveStuff            = ConfigManager::instance().saveStuff;
// inline std::vector<std::string>& CONFIG_pipelineSimpleMethods = ConfigManager::instance().pipelineSimpleMethods;
// inline std::vector<std::string>& CONFIG_pipelineFullMethods   = ConfigManager::instance().pipelineFullMethods;
inline std::string& CONFIG_outputPath           = ConfigManager::instance().outputPath;
inline int&         CONFIG_componentMaxDepth    = ConfigManager::instance().componentMaxDepth;
inline int&         CONFIG_parallelDepth        = ConfigManager::instance().parallelDepth;
inline int&         CONFIG_num_omp_threads      = ConfigManager::instance().num_omp_threads;
inline int&         CONFIG_num_cuda_streams     = ConfigManager::instance().num_cuda_streams;