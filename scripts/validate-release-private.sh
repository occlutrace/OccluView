#!/usr/bin/env bash
# Validate a release against the private scan corpus.
#
# The align acceptance tests need real scans, and real scans are patient data:
# they live outside the repository and CI never has them. The tests therefore
# skip when OCCLUVIEW_ALIGN_FIXTURES is unset, which is honest and useless at
# release time — a green CI run proves the maths on synthetic domes, not that a
# real arch comes home inside 0.05 mm.
#
# This script is the missing gate. It fails when the corpus is absent instead of
# skipping, runs the acceptance tests with the corpus required, and writes a
# receipt that says which corpus was used and what it proved. The receipt names
# no scan: the corpus is identified by a fingerprint over hashed file names, so
# it can be kept next to a release without carrying patient data.
#
# Usage:
#   OCCLUVIEW_ALIGN_FIXTURES=/path/to/stl/corpus scripts/validate-release-private.sh
#
# Environment:
#   OCCLUVIEW_ALIGN_FIXTURES  directory of binary STL scans (required)
#   OCCLUVIEW_RECEIPT         receipt path (default dist/private-acceptance.json)
#   OCCLUVIEW_SKIP_GATE=1     exit 2 without running anything (for a rehearsal
#                             that must not claim clinical coverage)
set -euo pipefail

cd "$(dirname "$0")/.."

receipt="${OCCLUVIEW_RECEIPT:-dist/private-acceptance.json}"

if [[ "${OCCLUVIEW_SKIP_GATE:-0}" != "0" ]]; then
  echo "validate-release-private: skipped on request; this run proves nothing about real scans" >&2
  exit 2
fi

corpus="${OCCLUVIEW_ALIGN_FIXTURES:-}"
if [[ -z "$corpus" ]]; then
  cat >&2 <<'MESSAGE'
validate-release-private: OCCLUVIEW_ALIGN_FIXTURES is not set.

This gate exists because the align acceptance criteria (0.05 mm residual, 85%
measured, 90% inside the clinical band) are only checked against real scans, and
real scans are not in the repository. Point the variable at a directory of
binary STL scans and run it again:

  OCCLUVIEW_ALIGN_FIXTURES=/path/to/corpus scripts/validate-release-private.sh

A release that skips this step ships an alignment solver whose clinical
behaviour nothing has verified.
MESSAGE
  exit 2
fi

if [[ ! -d "$corpus" ]]; then
  echo "validate-release-private: $corpus is not a directory" >&2
  exit 2
fi

scans=()
while IFS= read -r -d '' file; do
  scans+=("$file")
done < <(find "$corpus" -maxdepth 1 -type f -iname '*.stl' -print0 | sort -z)
if [[ ${#scans[@]} -eq 0 ]]; then
  echo "validate-release-private: $corpus holds no .stl files" >&2
  exit 2
fi

echo "validate-release-private: ${#scans[@]} scan(s) in the corpus"

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
log="$(mktemp -t occluview-private-acceptance-XXXXXX.log)"
trap 'rm -f "$log"' EXIT

status=0
OCCLUVIEW_ALIGN_FIXTURES="$corpus" \
OCCLUVIEW_ALIGN_FIXTURES_REQUIRED=1 \
  cargo test --locked -p occluview-align --test real_scans -- --nocapture --test-threads=1 \
  >"$log" 2>&1 || status=$?

if [[ $status -ne 0 ]]; then
  echo "validate-release-private: the acceptance tests failed" >&2
  tail -n 40 "$log" >&2
  exit "$status"
fi

# Only the corpus tests are this gate's business: the same binary holds two
# tests that skip on pair fixtures (OCCLUVIEW_ALIGN_PREP_PAIR, ..._OWNER_PAIR)
# which this script does not own, and their "skipped:" lines must not turn a
# passing acceptance run into a failure.
if grep -q "set OCCLUVIEW_ALIGN_FIXTURES" "$log"; then
  echo "validate-release-private: a corpus test skipped even though the corpus is present:" >&2
  grep -n "set OCCLUVIEW_ALIGN_FIXTURES" "$log" >&2
  exit 1
fi

if ! grep -q "test result: ok" "$log"; then
  echo "validate-release-private: no test result line; the run did not complete" >&2
  tail -n 40 "$log" >&2
  exit 1
fi

revision="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
  tree_state="dirty"
else
  tree_state="clean"
fi

mkdir -p "$(dirname "$receipt")"
OCCLUVIEW_RECEIPT_PATH="$receipt" \
OCCLUVIEW_RECEIPT_CORPUS="$corpus" \
OCCLUVIEW_RECEIPT_STARTED="$started_at" \
OCCLUVIEW_RECEIPT_REVISION="$revision" \
OCCLUVIEW_RECEIPT_TREE="$tree_state" \
OCCLUVIEW_RECEIPT_LOG="$log" \
python3 - "${scans[@]}" <<'PYTHON'
"""Write the receipt: what was proved, against which corpus, at which revision.

The corpus is described by a fingerprint over hashed file names and sizes. The
receipt travels with a release, and a release is public: it must not name a
patient's scan or say where the maintainer keeps them.
"""

import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys

files = [pathlib.Path(argument) for argument in sys.argv[1:]]
digest = hashlib.sha256()
for path in sorted(files, key=lambda path: path.name):
    name = hashlib.sha256(path.name.encode("utf-8", "replace")).hexdigest()
    digest.update(f"{name} {path.stat().st_size}\n".encode("utf-8"))

thresholds = {}
test_names = []
log = pathlib.Path(os.environ["OCCLUVIEW_RECEIPT_LOG"]).read_text(
    encoding="utf-8", errors="replace"
)
for match in re.finditer(r"^test ([a-z0-9_]+) \.\.\. ok$", log, flags=re.MULTILINE):
    test_names.append(match.group(1))
for name, value in (
    ("max_residual_mm", r"MAX_RESIDUAL_MM: f64 = ([0-9.]+)"),
    ("min_measured_share", r"MIN_MEASURED_SHARE: f64 = ([0-9.]+)"),
    ("min_within_tolerance", r"MIN_WITHIN_TOLERANCE: f64 = ([0-9.]+)"),
    ("tolerance_mm", r"TOLERANCE_MM: f64 = ([0-9.]+)"),
):
    source = pathlib.Path("crates/occluview-align/tests/real_scans.rs").read_text(
        encoding="utf-8"
    )
    found = re.search(value, source)
    thresholds[name] = float(found.group(1)) if found else None

rustc = subprocess.run(
    ["rustc", "--version"], capture_output=True, text=True, check=False
).stdout.strip()

receipt = {
    "schema": 1,
    "kind": "occluview-private-acceptance",
    "generated_at": os.environ["OCCLUVIEW_RECEIPT_STARTED"],
    "git_revision": os.environ["OCCLUVIEW_RECEIPT_REVISION"],
    "worktree": os.environ["OCCLUVIEW_RECEIPT_TREE"],
    "rustc": rustc,
    "corpus": {
        "scans": len(files),
        "fingerprint": digest.hexdigest(),
    },
    "thresholds": thresholds,
    "tests": sorted(test_names),
    "verdict": "passed",
}
path = pathlib.Path(os.environ["OCCLUVIEW_RECEIPT_PATH"])
path.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
print(f"validate-release-private: {len(test_names)} test(s) passed against {len(files)} scan(s)")
print(f"validate-release-private: receipt at {path}")
PYTHON
