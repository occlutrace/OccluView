#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -eq 0 ]]; then
  echo "usage: $0 path/to/occluview_VERSION_ARCH.deb [...]" >&2
  exit 2
fi

tmp_dirs=()
cleanup() {
  for dir in "${tmp_dirs[@]}"; do
    rm -rf "$dir"
  done
}
trap cleanup EXIT

require_path() {
  local root="$1"
  local rel="$2"
  if [[ ! -e "$root/$rel" ]]; then
    echo "missing package path: $rel" >&2
    exit 1
  fi
}

require_executable() {
  local root="$1"
  local rel="$2"
  require_path "$root" "$rel"
  if [[ ! -x "$root/$rel" ]]; then
    echo "package path is not executable: $rel" >&2
    exit 1
  fi
}

check_ldd() {
  local bin="$1"
  if ! command -v ldd >/dev/null 2>&1; then
    echo "ldd not found; skipping shared-library resolution check for $bin"
    return
  fi

  local out="$2"
  ldd "$bin" > "$out"
  if grep -F "not found" "$out" >/dev/null; then
    cat "$out" >&2
    echo "unresolved shared library dependency in $bin" >&2
    exit 1
  fi
}

for deb in "$@"; do
  if [[ ! -f "$deb" ]]; then
    echo "Debian package not found: $deb" >&2
    exit 1
  fi

  tmp="$(mktemp -d)"
  tmp_dirs+=("$tmp")
  control="$tmp/control"
  root="$tmp/root"
  mkdir -p "$control" "$root"

  dpkg-deb --info "$deb" > "$tmp/info.txt"
  dpkg-deb --contents "$deb" > "$tmp/contents.txt"
  dpkg-deb --control "$deb" "$control"
  dpkg-deb -x "$deb" "$root"

  grep -F "Package: occluview" "$control/control" >/dev/null
  grep -F "Maintainer: Dental Cloud Technologies" "$control/control" >/dev/null
  grep -F "Architecture:" "$control/control" >/dev/null
  grep -F "Version:" "$control/control" >/dev/null
  grep -F "Depends:" "$control/control" >/dev/null

  sh -n "$control/postinst"
  sh -n "$control/postrm"
  grep -F "update-desktop-database" "$control/postinst" >/dev/null
  grep -F "update-mime-database" "$control/postinst" >/dev/null
  grep -F "gtk-update-icon-cache" "$control/postinst" >/dev/null
  grep -F "update-desktop-database" "$control/postrm" >/dev/null
  grep -F "update-mime-database" "$control/postrm" >/dev/null
  grep -F "gtk-update-icon-cache" "$control/postrm" >/dev/null

  require_executable "$root" "usr/bin/occluview"
  require_executable "$root" "usr/bin/occluview-cli"
  require_path "$root" "usr/share/applications/ai.occlutrace.OccluView.desktop"
  require_path "$root" "usr/share/metainfo/ai.occlutrace.OccluView.metainfo.xml"
  require_path "$root" "usr/share/mime/packages/occluview-mime.xml"
  require_path "$root" "usr/share/thumbnailers/ai.occlutrace.OccluView.thumbnailer"
  require_path "$root" "usr/share/icons/hicolor/512x512/apps/occluview.png"
  require_path "$root" "usr/share/doc/occluview/README.md"
  require_path "$root" "usr/share/doc/occluview/copyright"
  require_path "$root" "usr/share/doc/occluview/changelog.gz"

  gzip -t "$root/usr/share/doc/occluview/changelog.gz"
  grep -F "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/" "$root/usr/share/doc/occluview/copyright" >/dev/null
  grep -F "/usr/share/common-licenses/Apache-2.0" "$root/usr/share/doc/occluview/copyright" >/dev/null

  grep -F "Exec=occluview %F" "$root/usr/share/applications/ai.occlutrace.OccluView.desktop" >/dev/null
  grep -F "MimeType=model/stl;model/obj;model/gltf-binary;" "$root/usr/share/applications/ai.occlutrace.OccluView.desktop" >/dev/null
  grep -F "Exec=occluview-cli thumbnail %i -o %o --size %s" "$root/usr/share/thumbnailers/ai.occlutrace.OccluView.thumbnailer" >/dev/null
  grep -F "MimeType=model/stl;model/obj;model/gltf-binary;" "$root/usr/share/thumbnailers/ai.occlutrace.OccluView.thumbnailer" >/dev/null

  if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$root/usr/share/applications/ai.occlutrace.OccluView.desktop"
  else
    echo "desktop-file-validate not found; skipping desktop entry validation"
  fi

  if command -v appstreamcli >/dev/null 2>&1; then
    appstreamcli validate --no-net "$root/usr/share/metainfo/ai.occlutrace.OccluView.metainfo.xml"
  else
    echo "appstreamcli not found; skipping AppStream validation"
  fi

  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout \
      "$root/usr/share/mime/packages/occluview-mime.xml" \
      "$root/usr/share/metainfo/ai.occlutrace.OccluView.metainfo.xml"
  else
    echo "xmllint not found; skipping XML validation"
  fi

  check_ldd "$root/usr/bin/occluview" "$tmp/occluview.ldd"
  check_ldd "$root/usr/bin/occluview-cli" "$tmp/occluview-cli.ldd"

  if command -v lintian >/dev/null 2>&1; then
    lintian --fail-on error "$deb"
  else
    echo "lintian not found; skipping Debian policy validation"
  fi

  echo "Debian package check passed: $deb"
done
