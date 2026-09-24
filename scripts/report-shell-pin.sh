#!/usr/bin/env bash
# Report which shell source the next release will ship.
#
# The release MSI builds occluview_shell.dll from a pinned revision while the
# viewer builds from the current tree. That is deliberate (Explorer loads the
# DLL in its own process), and it is exactly the kind of deliberate exception
# that turns into a silent divergence: the pin lives in a build script, and the
# released artifacts say nothing about it.
#
# This script makes the pin visible. It writes the revision, its tag, and every
# commit that has touched a crate the shell links since that revision, so the
# release records what the shipped DLL is and how far behind it is.
#
# Usage:
#   scripts/report-shell-pin.sh [output.json]
#
# Without an argument it prints the report to stdout.
set -euo pipefail

cd "$(dirname "$0")/.."

pin_file="install/shell-pin.json"
if [[ ! -f "$pin_file" ]]; then
  echo "report-shell-pin: $pin_file is missing" >&2
  exit 1
fi

# Read every field the script needs through ONE Python call, NUL-delimited.
#
# Line-based reads are fragile here: on a Windows runner Python's text mode
# writes CRLF, and command substitution drops the newline but keeps the
# carriage return. A revision left as `<sha>\r` makes `git cat-file -e` fail and
# aborts the script; a crate left as `occluview-shell\r` is a pathspec matching
# nothing, so the delta silently collapses to whichever names happened to
# survive and the reviewer is shown less work than there is. A NUL delimiter
# cannot appear in any of these values, so the field boundaries survive any
# line-ending convention. Read into an array so the values keep their own
# characters, and strip a stray CR defensively.
mapfile -d '' -t pin_fields < <(
  python3 -c '
import json, sys
pin = json.load(open(sys.argv[1]))
print(pin["revision"], end="\0")
print(pin.get("tag", ""), end="\0")
for crate in pin["shell_crates"]:
    print(crate, end="\0")
' "$pin_file"
)

revision="${pin_fields[0]%$'\r'}"
tag="${pin_fields[1]%$'\r'}"

if ! git cat-file -e "${revision}^{commit}" 2>/dev/null; then
  echo "report-shell-pin: the pinned revision $revision is not in this clone." >&2
  echo "Fetch full history (git fetch --unshallow or fetch-depth: 0) before releasing." >&2
  exit 1
fi

paths=()
for crate in "${pin_fields[@]:2}"; do
  # Belt and braces: the NUL framing already keeps the names clean, but a stray
  # carriage return here would silently drop the crate from the delta.
  crate="${crate%$'\r'}"
  [[ -n "$crate" ]] && paths+=("crates/$crate")
done

# Commits that touched a crate the shell links. This is the list a backport has
# to consider, and the number a reviewer should see before approving a release.
delta_file="$(mktemp -t occluview-shell-delta-XXXXXX)"
trap 'rm -f "$delta_file"' EXIT
git log --no-merges --pretty=format:'%H %ad %s' --date=short \
  "${revision}..HEAD" -- "${paths[@]}" > "$delta_file" || true
# `git log --pretty=format:` separates rather than terminates, so the last
# line carries no newline and `wc -l` would report one commit too few.
commits="$(git rev-list --no-merges --count "${revision}..HEAD" -- "${paths[@]}" 2>/dev/null || echo 0)"

head_revision="$(git rev-parse HEAD)"
output="${1:-}"

REPORT_REVISION="$revision" \
REPORT_TAG="$tag" \
REPORT_HEAD="$head_revision" \
REPORT_COMMITS="$commits" \
REPORT_DELTA="$delta_file" \
REPORT_PIN_FILE="$pin_file" \
python3 - "$output" <<'PYTHON'
"""Emit the shell-pin report as JSON."""

import json
import os
import pathlib
import subprocess
import sys

delta = pathlib.Path(os.environ["REPORT_DELTA"]).read_text(encoding="utf-8").splitlines()
pin = json.loads(pathlib.Path(os.environ["REPORT_PIN_FILE"]).read_text(encoding="utf-8"))
revision = os.environ["REPORT_REVISION"]
tag = os.environ["REPORT_TAG"]

reachable = (
    subprocess.run(
        ["git", "merge-base", "--is-ancestor", revision, "HEAD"],
        capture_output=True,
        check=False,
    ).returncode
    == 0
)

# The local clone may have been rewritten (a filter-branch run does that), which
# makes a released commit look unreachable while it is still the tag the world
# downloads. The remote's tags are the authoritative answer, so ask them.
remote_tag = None
remote = subprocess.run(
    ["git", "ls-remote", "--tags", "origin"],
    capture_output=True,
    text=True,
    check=False,
    timeout=60,
)
if remote.returncode == 0:
    for line in remote.stdout.splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[0] == revision and fields[1].endswith("^{}"):
            remote_tag = fields[1][len("refs/tags/") : -len("^{}")]
            break

report = {
    "schema": 1,
    "kind": "occluview-shell-pin",
    "revision": revision,
    "tag": tag or None,
    "viewer_revision": os.environ["REPORT_HEAD"],
    "ancestor_of_viewer": reachable,
    "remote_tag": remote_tag,
    "shell_crates": pin["shell_crates"],
    "commits_behind": int(os.environ["REPORT_COMMITS"]),
    "commits": delta,
    "why": pin["why"],
    "retire_when": pin["retire_when"],
    "backport": pin["backport"],
    "summary": (
        f"Explorer shell built from {revision[:12]}"
        + (f" ({tag})" if tag else "")
        + f"; {os.environ['REPORT_COMMITS']} commit(s) touching its crates since then"
    ),
}

text = json.dumps(report, indent=2) + "\n"
if sys.argv[1]:
    path = pathlib.Path(sys.argv[1])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    print(report["summary"])
    print(f"report-shell-pin: {path}")
else:
    sys.stdout.write(text)
PYTHON
