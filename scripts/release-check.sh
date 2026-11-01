#!/usr/bin/env bash
# Local release metadata report. This does not run tests, require patient scans,
# or authorize publication. Private Align acceptance is an independent,
# optional maintainer action through validate-release-private.sh.
#
# Usage:
#   scripts/release-check.sh
set -euo pipefail

cd "$(dirname "$0")/.."

version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
  | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
if [[ -z "$version" ]]; then
  echo "release-check: Cargo.toml has no workspace version" >&2
  exit 1
fi
tag="v$version"
echo "release-check: reporting local metadata for $tag"

bash scripts/report-shell-pin.sh dist/occluview-shell-revision.json

echo
echo "release-check: metadata report written for $tag"
echo "  dist/occluview-shell-revision.json  the Explorer shell source and delta"
echo "Private Align acceptance remains optional and is not part of this report."
