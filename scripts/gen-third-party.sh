#!/usr/bin/env bash
set -euo pipefail
# Regenerates THIRD-PARTY-NOTICES.md from Cargo.lock. CI regenerates and
# fails on drift, so run this after any dependency change.
# cargo-about 0.8.4 keeps local and CI notice generation byte-comparable with
# THIRD-PARTY-NOTICES.md.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo about generate --workspace --all-features --locked --fail \
  about.hbs -o THIRD-PARTY-NOTICES.md

# Normalize generated whitespace so CI produces a stable, patch-clean file.
sed -i -e 's/\r$//' -e 's/[[:space:]]\+$//' THIRD-PARTY-NOTICES.md
normalized_file="$(mktemp)"
trap 'rm -f "$normalized_file"' EXIT
awk '
  NF {
    if (pending_blank && printed) print ""
    print
    pending_blank = 0
    printed = 1
    next
  }
  { pending_blank = 1 }
' THIRD-PARTY-NOTICES.md > "$normalized_file"
mv "$normalized_file" THIRD-PARTY-NOTICES.md
trap - EXIT

# The generation is only correct when the bundled fonts' notice-retention
# licenses made it in and no first-party crate attributed itself.
grep -q "SIL OPEN FONT LICENSE" THIRD-PARTY-NOTICES.md || {
  echo "OFL font license text missing from THIRD-PARTY-NOTICES.md" >&2
  exit 1
}
grep -q "UBUNTU FONT LICENCE" THIRD-PARTY-NOTICES.md || {
  echo "Ubuntu font licence text missing from THIRD-PARTY-NOTICES.md" >&2
  exit 1
}
if grep -q "^- occluview" THIRD-PARTY-NOTICES.md; then
  echo "first-party crate leaked into THIRD-PARTY-NOTICES.md" >&2
  exit 1
fi
