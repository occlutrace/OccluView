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

# cargo-about traverses license groups in hash order. Canonicalize the overview,
# crate lists, and full license sections so generation stays stable across hosts.
python3 - <<'PY'
from pathlib import Path

notice_path = Path("THIRD-PARTY-NOTICES.md")
lines = notice_path.read_text(encoding="utf-8").splitlines()

overview_heading = lines.index("## Licenses used")
overview_start = overview_heading + 1
while overview_start < len(lines) and not lines[overview_start]:
    overview_start += 1
overview_end = overview_start
while overview_end < len(lines) and lines[overview_end].startswith("- "):
    overview_end += 1
lines[overview_start:overview_end] = sorted(lines[overview_start:overview_end])

license_heading = lines.index("## License texts")
section_starts = []
inside_fence = False
for index in range(license_heading + 1, len(lines)):
    if lines[index] == "```":
        inside_fence = not inside_fence
    elif not inside_fence and lines[index].startswith("### "):
        section_starts.append(index)

prefix = lines[:section_starts[0]]
sections = []
for position, start in enumerate(section_starts):
    end = section_starts[position + 1] if position + 1 < len(section_starts) else len(lines)
    section = lines[start:end]

    used_by = section.index("Used by:")
    crate_start = used_by + 1
    while crate_start < len(section) and not section[crate_start]:
        crate_start += 1
    crate_end = crate_start
    while crate_end < len(section) and section[crate_end].startswith("- "):
        crate_end += 1
    section[crate_start:crate_end] = sorted(section[crate_start:crate_end])
    sections.append("\n".join(section).strip())

canonical = "\n\n".join(["\n".join(prefix).rstrip(), *sorted(sections)]) + "\n"
notice_path.write_text(canonical, encoding="utf-8")
PY

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
