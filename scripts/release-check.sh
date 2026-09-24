#!/usr/bin/env bash
# The local half of a release: the checks CI cannot run.
#
# CI has no patient scans and no Explorer session, so two things a release
# depends on can only be checked on the maintainer's machine. This script runs
# both and refuses to continue when either is missing, so "I forgot" is not one
# of the failure modes.
#
# Usage:
#   OCCLUVIEW_ALIGN_FIXTURES=/path/to/corpus scripts/release-check.sh
#
# Environment:
#   OCCLUVIEW_ALIGN_FIXTURES  directory of binary STL scans (required)
#   OCCLUVIEW_SKIP_GATE=1     exit 2 without checking anything
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OCCLUVIEW_SKIP_GATE:-0}" != "0" ]]; then
  echo "release-check: skipped on request; nothing about this release was verified" >&2
  exit 2
fi

version="$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
  | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
if [[ -z "$version" ]]; then
  echo "release-check: Cargo.toml has no workspace version" >&2
  exit 1
fi
tag="v$version"
echo "release-check: preparing $tag"

# 1. The changelog section the release job publishes. A missing section means
#    the release publishes nothing, which the workflow only discovers on the
#    runner.
if ! grep -q "^## $version " CHANGELOG.md; then
  echo "release-check: CHANGELOG.md has no '## $version' section" >&2
  exit 1
fi
echo "release-check: changelog section present"

# 2. The private acceptance run. Fails when the corpus is absent; there is no
#    skip path, because a release that skipped this ships an alignment solver
#    whose clinical behaviour nothing verified.
bash scripts/validate-release-private.sh

# 3. Which shell source the MSI will ship, and how far behind it is.
bash scripts/report-shell-pin.sh dist/occluview-shell-revision.json

echo
echo "release-check: local gates passed for $tag"
echo "  dist/private-acceptance.json        the corpus this was validated against"
echo "  dist/occluview-shell-revision.json  the Explorer shell this ships"
echo
echo "CI still has to pass, and the tag is still yours to push."
