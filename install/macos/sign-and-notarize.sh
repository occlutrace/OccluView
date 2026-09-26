#!/usr/bin/env bash
# Sign, notarize and staple the Apple Silicon release artifacts.
#
# Run after build-app.sh. Signs the bundled CLI and the app with Developer ID
# Application under the hardened runtime, rebuilds the disk image and the
# installer package from the signed app, signs the image with the same
# identity and the package with Developer ID Installer, then notarizes and
# staples both. Leaves OccluView-<version>-aarch64.dmg and .pkg in
# target/macos (or $CARGO_TARGET_DIR/macos).
#
# Environment:
#   OCCLUVIEW_MACOS_APP_IDENTITY        "Developer ID Application: <name> (<team>)"
#   OCCLUVIEW_MACOS_INSTALLER_IDENTITY  "Developer ID Installer: <name> (<team>)"
#   OCCLUVIEW_NOTARY_KEY_PATH           App Store Connect API key file (.p8)
#   OCCLUVIEW_NOTARY_KEY_ID             that key's ID
#   OCCLUVIEW_NOTARY_ISSUER             that key's issuer ID
# Both identities must be reachable through the keychain search list.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
if [[ "$(uname -s)" != Darwin ]]; then
  echo "sign-and-notarize.sh requires macOS" >&2
  exit 1
fi
for name in OCCLUVIEW_MACOS_APP_IDENTITY OCCLUVIEW_MACOS_INSTALLER_IDENTITY \
  OCCLUVIEW_NOTARY_KEY_PATH OCCLUVIEW_NOTARY_KEY_ID OCCLUVIEW_NOTARY_ISSUER; do
  if [[ -z "${!name:-}" ]]; then
    echo "$name is required" >&2
    exit 2
  fi
done

cargo_target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$cargo_target_dir" != /* ]]; then
  cargo_target_dir="$repo_root/$cargo_target_dir"
fi
output_dir="$cargo_target_dir/macos"
app="$output_dir/OccluView.app"
if [[ ! -d "$app" ]]; then
  echo "missing $app; run build-app.sh first" >&2
  exit 1
fi
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
dmg="$output_dir/OccluView-$version-aarch64.dmg"
pkg="$output_dir/OccluView-$version-aarch64.pkg"

# Nested code first: codesign does not re-sign what a bundle contains.
codesign --force --timestamp --options runtime \
  --sign "$OCCLUVIEW_MACOS_APP_IDENTITY" "$app/Contents/Helpers/occluview-cli"
codesign --force --timestamp --options runtime \
  --sign "$OCCLUVIEW_MACOS_APP_IDENTITY" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

bash install/macos/build-dmg.sh --no-build
codesign --force --timestamp --sign "$OCCLUVIEW_MACOS_APP_IDENTITY" "$dmg"

bash install/macos/build-pkg.sh --no-build
productsign --timestamp --sign "$OCCLUVIEW_MACOS_INSTALLER_IDENTITY" "$pkg" "$pkg.signed"
mv -f "$pkg.signed" "$pkg"
pkgutil --check-signature "$pkg"

notarize() {
  local artifact="$1" result status id
  result="$(xcrun notarytool submit "$artifact" \
    --key "$OCCLUVIEW_NOTARY_KEY_PATH" \
    --key-id "$OCCLUVIEW_NOTARY_KEY_ID" \
    --issuer "$OCCLUVIEW_NOTARY_ISSUER" \
    --wait --timeout 45m --output-format json)"
  status="$(plutil -extract status raw -o - - <<<"$result")"
  id="$(plutil -extract id raw -o - - <<<"$result")"
  if [[ "$status" != Accepted ]]; then
    echo "notarization of $(basename "$artifact") ended as '$status' (submission $id)" >&2
    xcrun notarytool log "$id" \
      --key "$OCCLUVIEW_NOTARY_KEY_PATH" \
      --key-id "$OCCLUVIEW_NOTARY_KEY_ID" \
      --issuer "$OCCLUVIEW_NOTARY_ISSUER" >&2 || true
    exit 1
  fi
  xcrun stapler staple "$artifact"
  xcrun stapler validate "$artifact"
}
notarize "$dmg"
notarize "$pkg"

spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
spctl --assess --type install --verbose=2 "$pkg"
printf 'Signed, notarized and stapled: %s\n' "$dmg" "$pkg"
