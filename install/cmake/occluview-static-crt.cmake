# Keep CMake-built native dependencies on the same static MSVC runtime as the
# Rust installer payload. This file is loaded before the dependency's first
# project() call through CMAKE_TOOLCHAIN_FILE.
#
# cargo-xwin supplies its compiler toolchain separately; include it first so
# this overlay changes only runtime linkage, not compiler discovery.
if(DEFINED ENV{OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE}
   AND NOT "$ENV{OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE}" STREQUAL "")
  include("$ENV{OCCLUVIEW_BASE_CMAKE_TOOLCHAIN_FILE}")
endif()

# Do not gate this on MSVC: toolchain files are evaluated before CMake has
# enabled a language, so the MSVC variable may not be available yet. CMake
# ignores this setting for targets outside the MSVC ABI.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded" CACHE STRING
    "Use the static MSVC runtime for OccluView native dependencies" FORCE)
