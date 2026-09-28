#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

release_link='href="https://github.com/occlutrace/OccluView/releases/latest"'
badge_endpoint='https://img.shields.io/github/v/release/occlutrace/OccluView?'
link_count="$(grep -Fc "$release_link" README.md || true)"

if [[ "$link_count" != 1 ]]; then
  echo "README.md must link to the latest published release exactly once." >&2
  exit 1
fi

badge_line="$(grep -F "$release_link" README.md)"
if [[ "$badge_line" != *"$badge_endpoint"* ]]; then
  echo "README.md latest-release link must use the published-release version badge." >&2
  exit 1
fi

if grep -Eq 'img\.shields\.io/badge/latest%20release-v[0-9]' README.md; then
  echo "README.md latest-release badge must not pin a version." >&2
  exit 1
fi

echo "README latest-release badge follows the published release."
