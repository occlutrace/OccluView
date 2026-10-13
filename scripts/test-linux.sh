#!/bin/bash
# Run the workspace test suite on this Linux box under the same software GPU
# the CI test job pins.
#
# Why this wrapper exists: `cargo test -p occluview-render` with the ambient
# `DISPLAY=:99` reaches an X server it cannot authorize and the wgpu process
# dies with SIGSEGV (signal 11), which reads like a real test failure. The CI
# `test` job installs Lavapipe and exports VK_ICD_FILENAMES + WGPU_BACKEND;
# repeating that here makes the GPU suites actually execute instead of dying
# or early-returning. See .github/workflows/ci.yml "Install Lavapipe Vulkan
# adapter".
#
# Usage: scripts/test-linux.sh <cargo test args...>
#   scripts/test-linux.sh -p occluview-render --lib
#   scripts/test-linux.sh -p occluview-app --lib
#
# The portable default invokes Cargo directly. A host may opt into its local
# scheduler with OCCLUVIEW_HEAVY_WRAPPER=/path/to/heavy.sh; that wrapper must
# pass Cargo's exit status through unchanged.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

icd="${OCCLUVIEW_VK_ICD:-}"
if [ -z "$icd" ]; then
  icd="$(find /usr/share/vulkan/icd.d -name '*lvp*.json' -print -quit 2>/dev/null || true)"
fi
if [ -z "$icd" ] || [ ! -f "$icd" ]; then
  echo "test-linux: no Lavapipe ICD found under /usr/share/vulkan/icd.d." >&2
  echo "test-linux: install mesa-vulkan-drivers or set OCCLUVIEW_VK_ICD." >&2
  exit 2
fi

# A writable XDG_RUNTIME_DIR is required by the Vulkan loader even when no
# display is used; DISPLAY/WAYLAND_DISPLAY would otherwise re-enter the
# broken :99 X path.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/xdg-test-linux-$$}"
mkdir -p "$XDG_RUNTIME_DIR"
export VK_ICD_FILENAMES="$icd"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
# A test that early-returns when no adapter is present asserts nothing. With a
# guaranteed software adapter here, make GPU-dependent tests fail loudly rather
# than pass vacuously, so "green" means they actually ran.
export OCCLUVIEW_REQUIRE_GPU_TESTS=1
unset DISPLAY WAYLAND_DISPLAY

if [[ -n "${OCCLUVIEW_HEAVY_WRAPPER:-}" ]]; then
  if [[ ! -x "$OCCLUVIEW_HEAVY_WRAPPER" ]]; then
    echo "test-linux: OCCLUVIEW_HEAVY_WRAPPER is not executable: $OCCLUVIEW_HEAVY_WRAPPER" >&2
    exit 2
  fi
  exec "$OCCLUVIEW_HEAVY_WRAPPER" cargo test --locked "$@"
fi

exec cargo test --locked "$@"
