#!/usr/bin/env bash
set -euo pipefail

target="${OCCLUVIEW_WINDOWS_TARGET:-x86_64-pc-windows-msvc}"
profile="${OCCLUVIEW_PROFILE:-release}"

# Explorer hosts the shell DLL in dllhost.exe, so release builds use the
# unwinding profile to keep a panic from terminating the host process.
case "$profile" in
  release)
    app_profile_args=(--release)
    shell_profile_args=(--profile release-unwind)
    profile_dir="release"
    shell_profile_dir="release-unwind"
    ;;
  debug)
    app_profile_args=()
    shell_profile_args=()
    profile_dir="debug"
    shell_profile_dir="debug"
    ;;
  *)
    echo "OCCLUVIEW_PROFILE must be 'release' or 'debug'." >&2
    exit 2
    ;;
esac

if ! command -v cargo-xwin >/dev/null 2>&1 && ! cargo xwin --version >/dev/null 2>&1; then
  echo "cargo-xwin is required for Linux -> Windows MSVC builds." >&2
  echo "Install it with: cargo install cargo-xwin --locked" >&2
  exit 127
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# cargo-xwin exposes the CMake toolchain under a target-qualified variable.
# Cargo-native build scripts understand that convention, but manifold-csg-sys
# invokes CMake directly and therefore needs the ordinary variable as well.
target_env_suffix="${target//-/_}"
cmake_toolchain_var="CMAKE_TOOLCHAIN_FILE_${target_env_suffix}"
xwin_environment="$(cargo xwin env --target "$target")"
cmake_toolchain="$({
  printf '%s\n' "$xwin_environment" |
    sed -n "s/^export ${cmake_toolchain_var}=\"\(.*\)\";$/\1/p"
} | head -n 1)"
if [[ -z "$cmake_toolchain" || ! -f "$cmake_toolchain" ]]; then
  echo "cargo-xwin did not provide a usable CMake toolchain for $target." >&2
  exit 1
fi
export CMAKE_TOOLCHAIN_FILE="$cmake_toolchain"

# Remove generated Manifold build directories whose compiler cache is not
# configured for clang-cl; valid caches remain reusable.
if [[ -d "$repo_root/target/$target" ]]; then
  while IFS= read -r -d '' cache; do
    if ! grep -Eq '^CMAKE_CXX_COMPILER:(FILEPATH|STRING)=.*clang-cl' "$cache"; then
      stale_build_dir="${cache%/CMakeCache.txt}"
      printf 'Removing stale host-compiler CMake cache: %s\n' "$stale_build_dir"
      rm -rf -- "$stale_build_dir"
    fi
  done < <(
    find "$repo_root/target/$target" \
      -path '*/build/manifold-csg-sys-*/out/build/CMakeCache.txt' \
      -print0
  )
fi

if [[ "$profile" == "release" ]]; then
  sep=$'\x1f'
  remap_flag="--remap-path-prefix=$repo_root=occluview"
  if [[ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]]; then
    export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS}${sep}${remap_flag}"
  else
    export CARGO_ENCODED_RUSTFLAGS="$remap_flag"
  fi
fi

feature_args=()
if [[ -n "${OCCLUVIEW_HPS_EMBEDDED_KEY:-}" ]]; then
  feature_args=(--features occluview-formats/private-hps-key)
fi

cargo xwin build \
  -p occluview-app \
  --target "$target" \
  "${app_profile_args[@]}" \
  "${feature_args[@]}"

cargo xwin build \
  -p occluview-shell \
  --target "$target" \
  "${shell_profile_args[@]}" \
  "${feature_args[@]}"

build_dir="$repo_root/target/$target/$profile_dir"
shell_build_dir="$repo_root/target/$target/$shell_profile_dir"
required=(
  "$build_dir/occluview.exe"
  "$shell_build_dir/occluview_shell.dll"
)

for path in "${required[@]}"; do
  if [[ ! -f "$path" ]]; then
    echo "Missing expected Windows artifact: $path" >&2
    exit 1
  fi
done

printf 'Windows MSVC artifacts built in %s\n' "$build_dir"
