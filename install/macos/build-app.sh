#!/usr/bin/env bash
set -euo pipefail

build_release=1
for arg in "$@"; do
  case "$arg" in
    --no-build)
      build_release=0
      ;;
    *)
      echo "usage: $0 [--no-build]" >&2
      exit 2
      ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
if [[ -f "$HOME/.cargo/env" ]]; then
  # Rustup writes this initializer; CI-provisioned toolchains may already be on PATH.
  source "$HOME/.cargo/env"
fi

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo "build-app.sh requires a native Apple Silicon Mac" >&2
  exit 1
fi
if (( build_release )) && ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is unavailable; install the repository-pinned Rust toolchain first" >&2
  exit 127
fi

version="$(awk '
  $0 == "[workspace.package]" { in_section = 1; next }
  /^\[/ && in_section { exit }
  in_section && $1 == "version" {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' Cargo.toml)"
if [[ -z "$version" ]]; then
  echo "could not read workspace version from Cargo.toml" >&2
  exit 1
fi

cargo_target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$cargo_target_dir" != /* ]]; then
  cargo_target_dir="$repo_root/$cargo_target_dir"
fi
profile_dir="$cargo_target_dir/aarch64-apple-darwin/release-unwind"
app_binary="$profile_dir/occluview"
cli_binary="$profile_dir/occluview-cli"
app_bundle="$cargo_target_dir/macos/OccluView.app"

if (( build_release )); then
  # Same rule as install/linux/build-deb.sh: release packaging passes the key
  # from its secret store, and encrypted HPS/.dcm files open only in a build
  # that embeds it. The empty-array form keeps /bin/bash 3.2 under `set -u`.
  feature_args=()
  if [[ -n "${OCCLUVIEW_HPS_EMBEDDED_KEY:-}" ]]; then
    echo "Private HPS key embedding enabled for this build."
    feature_args=(--features occluview-formats/private-hps-key)
  fi
  MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked \
    --target aarch64-apple-darwin --profile release-unwind \
    -p occluview-app -p occluview-cli ${feature_args[@]+"${feature_args[@]}"}
fi

for binary in "$app_binary" "$cli_binary"; do
  if [[ ! -x "$binary" ]]; then
    echo "missing executable $binary; build the Apple Silicon release first" >&2
    exit 1
  fi
done

rm -rf "$app_bundle"
contents="$app_bundle/Contents"
resources="$contents/Resources"
mkdir -p "$contents/MacOS" "$contents/Helpers" "$resources/Legal"
install -m 0755 "$app_binary" "$contents/MacOS/occluview"
install -m 0755 "$cli_binary" "$contents/Helpers/occluview-cli"
cp install/macos/README.txt "$resources/README.txt"
sed "s/@VERSION@/$version/g" install/macos/Info.plist.in > "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"
sips -s format icns install/assets/icons/hicolor/512x512/apps/occluview.png \
  --out "$resources/occluview.icns" >/dev/null

for notice in LICENSE NOTICE THIRD-PARTY-NOTICES.md THIRD-PARTY-NOTICES-NATIVE.md; do
  cp "$notice" "$resources/Legal/$notice"
done
plutil -lint "$contents/Info.plist"
printf 'Created unsigned Apple Silicon app bundle: %s\n' "$app_bundle"
printf 'Bundle version: %s; minimum macOS: 14.0\n' "$version"
