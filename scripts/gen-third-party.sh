#!/usr/bin/env bash
set -euo pipefail
# Regenerates THIRD-PARTY-NOTICES.md from Cargo.lock. CI regenerates and
# fails on drift, so run this after any dependency change.
# Requires: cargo install cargo-about --version 0.8.4 --locked
# (0.9.x needs rustc 1.88 and gates its binary behind a `cli` feature, so it
# cannot be installed by the toolchain this repository pins.)
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo about generate --workspace --all-features --locked --fail \
  about.hbs -o THIRD-PARTY-NOTICES.md

# Normalize generated whitespace so CI produces a stable, patch-clean file.
sed -i -e 's/\r$//' -e 's/[[:space:]]\+$//' THIRD-PARTY-NOTICES.md

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
