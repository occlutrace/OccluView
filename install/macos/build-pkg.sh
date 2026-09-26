#!/usr/bin/env bash
set -euo pipefail

build_app=1
for arg in "$@"; do
  case "$arg" in
    --no-build)
      build_app=0
      ;;
    *)
      echo "usage: $0 [--no-build]" >&2
      exit 2
      ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo "build-pkg.sh requires a native Apple Silicon Mac" >&2
  exit 1
fi

cargo_target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$cargo_target_dir" != /* ]]; then
  cargo_target_dir="$repo_root/$cargo_target_dir"
fi
app_bundle="$cargo_target_dir/macos/OccluView.app"
if (( build_app )); then
  bash install/macos/build-app.sh
fi
if [[ ! -d "$app_bundle" ]]; then
  echo "missing $app_bundle; run build-app.sh first" >&2
  exit 1
fi

version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app_bundle/Contents/Info.plist")"
output_dir="$cargo_target_dir/macos"
mkdir -p "$output_dir"
output="$output_dir/OccluView-$version-aarch64.pkg"
pkgbuild --component "$app_bundle" \
  --install-location /Applications \
  --identifier ai.occlutrace.occluview \
  --version "$version" \
  --ownership recommended \
  "$output"
payload="$(pkgutil --payload-files "$output")"
printf '%s\n' "$payload" | grep -Fq './OccluView.app/Contents/MacOS/occluview'
printf 'Created unsigned Apple Silicon installer package: %s\n' "$output"
