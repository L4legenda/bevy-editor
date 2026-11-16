# lscm-src/CMakeLists.txt

# Note: We don't need to redefine the basic project settings since they're inherited
# from the parent CMakeLists.txt

# Define LSCM source files

# We don't need cmake_minimum_required or project() calls here since they're handled by the root CMakeLists.txt

# Define the source files for the triangle intersection library
set(TRIANGLE_SOURCES
    triangleHelper.cpp
)

add_library(triangle_intersection STATIC ${TRIANGLE_SOURCES})

target_include_directories(triangle_intersection
    PUBLIC
        ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(triangle_intersection
    PUBLIC
        CGAL::CGAL
    PRIVATE
        OpenMP::OpenMP_CXX
)