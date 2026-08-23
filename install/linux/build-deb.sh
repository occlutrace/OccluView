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

if [[ "$build_release" -eq 1 ]]; then
  feature_args=()
  if [[ -n "${OCCLUVIEW_HPS_EMBEDDED_KEY:-}" ]]; then
    echo "Private HPS key embedding enabled for this build."
    feature_args=(--features occluview-formats/private-hps-key)
  fi
  cargo build --release -p occluview-app -p occluview-cli "${feature_args[@]}"
fi

arch="$(dpkg --print-architecture 2>/dev/null || true)"
if [[ -z "$arch" ]]; then
  case "$(uname -m)" in
    x86_64) arch=amd64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) arch="$(uname -m)" ;;
  esac
fi

pkg_root="target/deb/occluview_${version}_${arch}"
rm -rf "$pkg_root"
mkdir -p \
  "$pkg_root/DEBIAN" \
  "$pkg_root/usr/bin" \
  "$pkg_root/usr/share/applications" \
  "$pkg_root/usr/share/icons/hicolor/512x512/apps" \
  "$pkg_root/usr/share/icons/hicolor/scalable/mimetypes" \
  "$pkg_root/usr/share/metainfo" \
  "$pkg_root/usr/share/mime/packages" \
  "$pkg_root/usr/share/thumbnailers" \
  "$pkg_root/usr/share/doc/occluview"

install -m 0755 target/release/occluview "$pkg_root/usr/bin/occluview"
install -m 0755 target/release/occluview-cli "$pkg_root/usr/bin/occluview-cli"
install -m 0644 assets/occluview-logo.png \
  "$pkg_root/usr/share/icons/hicolor/512x512/apps/occluview.png"
# The file-type icon the Windows installer registers per extension, installed
# under the freedesktop icon name of each type it belongs to. Without it a scan
# that has no thumbnail yet -- or whose thumbnailer the desktop never runs --
# is drawn with the generic unknown-file glyph.
for mime_icon in \
  model-stl \
  model-obj \
  model-gltf-binary \
  application-x-ply \
  application-x-occluview-hps
do
  install -m 0644 install/assets/file-icons/occluview-3d.svg \
    "$pkg_root/usr/share/icons/hicolor/scalable/mimetypes/${mime_icon}.svg"
done

install -m 0644 install/linux/ai.occlutrace.OccluView.desktop \
  "$pkg_root/usr/share/applications/ai.occlutrace.OccluView.desktop"
install -m 0644 install/linux/ai.occlutrace.OccluView.metainfo.xml \
  "$pkg_root/usr/share/metainfo/ai.occlutrace.OccluView.metainfo.xml"
install -m 0644 install/linux/occluview-mime.xml \
  "$pkg_root/usr/share/mime/packages/occluview-mime.xml"
install -m 0644 install/linux/ai.occlutrace.OccluView.thumbnailer \
  "$pkg_root/usr/share/thumbnailers/ai.occlutrace.OccluView.thumbnailer"
install -m 0644 README.md "$pkg_root/usr/share/doc/occluview/README.md"
install -m 0644 install/linux/copyright "$pkg_root/usr/share/doc/occluview/copyright"
# The Apache NOTICE and the generated third-party attributions travel with
# the statically linked binary; the copyright file points at them.
install -m 0644 NOTICE "$pkg_root/usr/share/doc/occluview/NOTICE"
install -m 0644 docs/USAGE.md \
  "$pkg_root/usr/share/doc/occluview/USAGE.md"
install -m 0644 THIRD-PARTY-NOTICES.md \
  "$pkg_root/usr/share/doc/occluview/THIRD-PARTY-NOTICES.md"
# The C++ geometry kernel and its own dependencies are statically linked and
# invisible to every Cargo tool, so their notices are carried by hand.
install -m 0644 THIRD-PARTY-NOTICES-NATIVE.md \
  "$pkg_root/usr/share/doc/occluview/THIRD-PARTY-NOTICES-NATIVE.md"
gzip -9 -n -c CHANGELOG.md > "$pkg_root/usr/share/doc/occluview/changelog.gz"

installed_size="$(du -sk "$pkg_root/usr" | awk '{ print $1 }')"
cat > "$pkg_root/DEBIAN/control" <<CONTROL
Package: occluview
Version: $version
Section: graphics
Priority: optional
Architecture: $arch
Maintainer: Dental Cloud Technologies <support@occlutrace.ai>
Installed-Size: $installed_size
Depends: libc6, libgcc-s1, libx11-6, libxcb1, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libwayland-client0, libwayland-cursor0, libwayland-egl1, libvulkan1, desktop-file-utils, shared-mime-info, hicolor-icon-theme
Recommends: xdg-desktop-portal
Homepage: https://occlutrace.ai
Description: fast 3D mesh viewer for dental scans
 OccluView opens STL, PLY, OBJ, GLB, and HPS mesh packages in a native
 desktop viewport and provides a thumbnail command for file managers.
CONTROL

cat > "$pkg_root/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database /usr/share/mime || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor || true
fi
exit 0
POSTINST

cat > "$pkg_root/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database /usr/share/mime || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor || true
fi
exit 0
POSTRM

chmod 0755 "$pkg_root/DEBIAN/postinst" "$pkg_root/DEBIAN/postrm"

out_dir="target/deb"
mkdir -p "$out_dir"
dpkg-deb --build --root-owner-group "$pkg_root" "$out_dir/occluview_${version}_${arch}.deb"
# Contract: the last line on stdout is the path of the package just built.
# Callers take it from here rather than globbing target/deb, which on a
# developer machine holds every earlier version as well.
echo "$out_dir/occluview_${version}_${arch}.deb"
