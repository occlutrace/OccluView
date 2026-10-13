#!/usr/bin/env bash
# Optional maintainer-run acceptance against a private, manifest-described
# corpus. This script is deliberately not called by release-check or CI.
#
# Usage:
#   OCCLUVIEW_PRIVATE_MANIFEST=/private/acceptance.json \
#   OCCLUVIEW_PRIVATE_CORPUS=/private/corpus \
#     scripts/validate-release-private.sh
#
# The manifest contract is documented in
# crates/occluview-align/tests/private-acceptance.md.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$repo_root"

manifest_input="${OCCLUVIEW_PRIVATE_MANIFEST:-}"
corpus_input="${OCCLUVIEW_PRIVATE_CORPUS:-}"
if [[ -z "$manifest_input" || -z "$corpus_input" ]]; then
  echo "private acceptance requires OCCLUVIEW_PRIVATE_MANIFEST and OCCLUVIEW_PRIVATE_CORPUS" >&2
  exit 2
fi
if [[ ! -f "$manifest_input" ]]; then
  echo "private acceptance manifest is unavailable" >&2
  exit 2
fi
if [[ ! -d "$corpus_input" ]]; then
  echo "private acceptance corpus is unavailable" >&2
  exit 2
fi

manifest="$(realpath -e -- "$manifest_input" 2>/dev/null)" || {
  echo "private acceptance manifest is unavailable" >&2
  exit 2
}
corpus="$(realpath -e -- "$corpus_input" 2>/dev/null)" || {
  echo "private acceptance corpus is unavailable" >&2
  exit 2
}

target="crates/occluview-align/tests/private_acceptance.rs"
if [[ ! -f "$target" ]]; then
  echo "private acceptance test target is missing" >&2
  exit 2
fi

profile="${OCCLUVIEW_PRIVATE_PROFILE:-debug}"
case "$profile" in
  debug|release) ;;
  *)
    echo "OCCLUVIEW_PRIVATE_PROFILE must be debug or release" >&2
    exit 2
    ;;
esac

features="${OCCLUVIEW_PRIVATE_FEATURES:-}"
feature_args=()
if [[ -n "$features" ]]; then
  if [[ "$features" != "search-probe" ]]; then
    echo "private acceptance supports only the search-probe feature" >&2
    exit 2
  fi
  feature_args=(--features "$features")
fi

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
run_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
revision="$(git rev-parse HEAD)"
receipt="${OCCLUVIEW_RECEIPT:-dist/private-acceptance-${revision}-${run_stamp}.json}"
if [[ -e "$receipt" ]]; then
  echo "private acceptance receipt path already exists; choose a new path" >&2
  exit 2
fi

scratch="$(mktemp -d -t occluview-private-acceptance-XXXXXX)"
result_path="$scratch/results.json"
log_path="$scratch/test.log"
receipt_tmp=""
cleanup() {
  [[ -z "$receipt_tmp" ]] || rm -f -- "$receipt_tmp"
  rm -rf -- "$scratch"
}
trap cleanup EXIT

tree_snapshot() {
  python3 - <<'PYTHON'
import hashlib
import subprocess

diff = subprocess.run(
    ["git", "diff", "--binary", "HEAD"],
    check=True,
    stdout=subprocess.PIPE,
).stdout
status = subprocess.run(
    ["git", "status", "--porcelain=v1", "--untracked-files=all"],
    check=True,
    stdout=subprocess.PIPE,
).stdout
untracked = subprocess.run(
    ["git", "ls-files", "--others", "--exclude-standard", "-z"],
    check=True,
    stdout=subprocess.PIPE,
).stdout.split(b"\0")
digest = hashlib.sha256()
digest.update(b"occluview-worktree-diff-v1\0")
digest.update(len(diff).to_bytes(8, "big"))
digest.update(diff)
for raw_path in sorted(path for path in untracked if path):
    with open(raw_path, "rb") as source:
        content = source.read()
    digest.update(len(raw_path).to_bytes(8, "big"))
    digest.update(raw_path)
    digest.update(len(content).to_bytes(8, "big"))
    digest.update(content)
print(("clean" if not status else "dirty") + " " + digest.hexdigest())
PYTHON
}

before_tree="$(tree_snapshot)"
before_revision="$(git rev-parse HEAD)"
if [[ "$before_revision" != "$revision" ]]; then
  echo "source revision changed during private acceptance setup; no receipt was written" >&2
  exit 1
fi
rustc_version="$(rustc --version --verbose 2>/dev/null)" || {
  echo "pinned rustc is unavailable" >&2
  exit 2
}
cargo_version="$(cargo --version 2>/dev/null)" || {
  echo "Cargo is unavailable" >&2
  exit 2
}

cargo_args=(test --locked --no-default-features -p occluview-align)
if [[ "$profile" == "release" ]]; then
  cargo_args+=(--release)
fi
cargo_args+=("${feature_args[@]}" --test private_acceptance private_acceptance_manifest -- --ignored --exact --nocapture --test-threads=1)

status=0
OCCLUVIEW_PRIVATE_MANIFEST="$manifest" \
OCCLUVIEW_PRIVATE_CORPUS="$corpus" \
OCCLUVIEW_PRIVATE_RESULT="$result_path" \
  cargo "${cargo_args[@]}" >"$log_path" 2>&1 || status=$?
if [[ $status -ne 0 ]]; then
  echo "private acceptance did not complete successfully; no receipt was written" >&2
  tail -n 40 "$log_path" >&2
  if [[ -s "$result_path" ]]; then
    python3 - "$result_path" <<'PYTHON' >&2
import json
import pathlib
import sys
try:
    result = json.loads(pathlib.Path(sys.argv[1]).read_bytes())
except (OSError, ValueError):
    raise SystemExit(0)
print("structured results for cases actually invoked:")
for case in result.get("cases", []):
    print(json.dumps({key: case.get(key) for key in (
        "case_id", "passed", "completion", "confidence", "reasons",
        "candidate_family", "candidate_proposal", "candidate_evidence",
        "rotation_error_deg", "translation_error_mm",
        "maximum_control_landmark_error_mm", "overlap_smaller",
        "rms_mm", "p95_mm", "examined_poses", "iterations", "elapsed_ms",
    )}, allow_nan=False))
PYTHON
  fi
  exit "$status"
fi
if [[ ! -s "$result_path" ]]; then
  echo "private acceptance target produced no structured results; no receipt was written" >&2
  exit 1
fi

finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
after_tree="$(tree_snapshot)"
after_revision="$(git rev-parse HEAD)"
if [[ "$before_tree" != "$after_tree" || "$revision" != "$after_revision" ]]; then
  echo "source tree changed during private acceptance; no receipt was written" >&2
  exit 1
fi

mkdir -p -- "$(dirname "$receipt")"
receipt_tmp="${receipt}.tmp.$$"
if [[ -e "$receipt_tmp" ]]; then
  echo "private acceptance temporary receipt path already exists" >&2
  exit 2
fi

PRIVATE_MANIFEST="$manifest" \
PRIVATE_CORPUS="$corpus" \
PRIVATE_RESULT="$result_path" \
PRIVATE_RECEIPT="$receipt_tmp" \
PRIVATE_STARTED="$started_at" \
PRIVATE_FINISHED="$finished_at" \
PRIVATE_REVISION="$revision" \
PRIVATE_TREE="$after_tree" \
PRIVATE_RUSTC="$rustc_version" \
PRIVATE_CARGO="$cargo_version" \
PRIVATE_PROFILE="$profile" \
PRIVATE_FEATURES="$features" \
python3 - <<'PYTHON'
"""Validate the target's structured output and write a path-free receipt."""

import hashlib
import json
import math
import os
import pathlib
import re
import stat

manifest_path = pathlib.Path(os.environ["PRIVATE_MANIFEST"])
try:
    corpus_root = pathlib.Path(os.environ["PRIVATE_CORPUS"]).resolve(strict=True)
except (OSError, RuntimeError):
    raise SystemExit("private acceptance corpus changed during receipt validation; no receipt was written") from None
try:
    manifest_bytes = manifest_path.read_bytes()
except OSError:
    raise SystemExit("private acceptance manifest changed during receipt validation; no receipt was written") from None
manifest_sha = hashlib.sha256(manifest_bytes).digest()
manifest = json.loads(manifest_bytes)
if manifest.get("schema_version") != 1 or not isinstance(manifest.get("cases"), list):
    raise SystemExit("private acceptance manifest schema is invalid; no receipt was written")
cases = manifest["cases"]
if not cases:
    raise SystemExit("private acceptance manifest has no cases; no receipt was written")

case_ids = [case.get("id") for case in cases]
if any(not isinstance(case_id, str) or not re.fullmatch(r"case_[0-9a-f]{12}", case_id) for case_id in case_ids):
    raise SystemExit("private acceptance manifest contains an invalid case ID; no receipt was written")
if len(set(case_ids)) != len(case_ids):
    raise SystemExit("private acceptance manifest contains duplicate case IDs; no receipt was written")

def update_field(digest, value):
    digest.update(len(value).to_bytes(8, "big"))
    digest.update(value)

def resolve_asset(relative):
    candidate = corpus_root
    try:
        for component in relative.parts:
            candidate = candidate / component
            if stat.S_ISLNK(candidate.lstat().st_mode):
                raise ValueError
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(corpus_root)
        if not resolved.is_file():
            raise ValueError
        return candidate, resolved
    except (OSError, RuntimeError, ValueError):
        raise SystemExit(
            "private acceptance asset is unavailable or uses a symlink; no receipt was written"
        ) from None

def hash_checked_asset(relative):
    candidate, resolved = resolve_asset(relative)
    try:
        before = resolved.stat()
        digest = hashlib.sha256()
        read_size = 0
        with resolved.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                read_size += len(chunk)
                if read_size > before.st_size:
                    raise OSError("asset grew during receipt validation")
                digest.update(chunk)
        candidate_after, resolved_after = resolve_asset(relative)
        after = resolved_after.stat()
    except (OSError, RuntimeError):
        raise SystemExit("private acceptance asset changed during receipt validation; no receipt was written") from None
    before_identity = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    after_identity = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
    if (candidate_after != candidate or resolved_after != resolved
            or before_identity != after_identity or read_size != before.st_size):
        raise SystemExit("private acceptance asset changed during receipt validation; no receipt was written")
    return digest.digest()

corpus_digest = hashlib.sha256()
update_field(corpus_digest, b"occluview-private-acceptance-corpus-v1")
update_field(corpus_digest, manifest_sha)
for case in cases:
    case_id = case["id"]
    for role in ("moving", "fixed"):
        asset = case[role]
        if not isinstance(asset.get("file"), str):
            raise SystemExit("private acceptance manifest has an invalid asset path; no receipt was written")
        relative = pathlib.PurePosixPath(asset["file"])
        if relative.is_absolute() or not relative.parts or any(part in ("", ".", "..") for part in relative.parts):
            raise SystemExit("private acceptance manifest has an unsafe asset path; no receipt was written")
        if "\\" in asset["file"] or ":" in asset["file"]:
            raise SystemExit("private acceptance manifest has an unsafe asset path; no receipt was written")
        file_sha = hash_checked_asset(relative)
        update_field(corpus_digest, case_id.encode("utf-8"))
        update_field(corpus_digest, asset["file"].encode("utf-8"))
        update_field(corpus_digest, file_sha)

result = json.loads(pathlib.Path(os.environ["PRIVATE_RESULT"]).read_bytes())
if result.get("schema_version") != 1 or result.get("all_passed") is not True:
    raise SystemExit("private acceptance result is incomplete or failed; no receipt was written")
if result.get("manifest_sha256") != manifest_sha.hex():
    raise SystemExit("private acceptance manifest changed during execution; no receipt was written")
if result.get("corpus_sha256") != corpus_digest.hexdigest():
    raise SystemExit("private acceptance corpus changed during execution; no receipt was written")
actual = result.get("cases")
if (
    not isinstance(actual, list)
    or any(not isinstance(item, dict) for item in actual)
    or [item.get("case_id") for item in actual] != case_ids
):
    raise SystemExit("private acceptance cases are missing, duplicated, or out of order; no receipt was written")

confidence_rank = {"weak": 0, "ambiguous": 1, "probable": 2, "verified": 3}
known_reasons = {
    "insufficient_support",
    "unobservable_motion",
    "close_rival",
    "policy_conflict",
    "suspected_units_or_deformation",
    "missing_normals",
    "unvalidated_input",
    "budget_exhausted",
    "invalid_seed",
    "landmark_rejected",
    "uniqueness_not_established",
    "invalid_topology",
}
numeric_fields = (
    "rotation_error_deg",
    "translation_error_mm",
    "maximum_control_landmark_error_mm",
    "overlap_smaller",
    "rms_mm",
    "p95_mm",
    "elapsed_ms",
)
receipt_cases = []
for planned, observed in zip(cases, actual, strict=True):
    if (
        observed.get("passed") is not True
        or observed.get("completion") != "complete"
        or observed.get("input_check_complete") is not True
    ):
        raise SystemExit("private acceptance contains an incomplete case; no receipt was written")
    confidence = observed.get("confidence")
    if not isinstance(confidence, str) or confidence not in confidence_rank:
        raise SystemExit("private acceptance contains an unknown confidence; no receipt was written")
    candidates = observed.get("candidate_evidence")
    if not isinstance(candidates, list) or not candidates:
        raise SystemExit("private acceptance candidate evidence is missing; no receipt was written")
    for candidate in candidates:
        if not isinstance(candidate, dict):
            raise SystemExit("private acceptance candidate evidence is malformed; no receipt was written")
        if (
            type(candidate.get("family")) is not int
            or not 0 <= candidate["family"] <= 255
            or type(candidate.get("proposal")) is not int
            or not 0 <= candidate["proposal"] <= 0xFFFFFFFF
            or not isinstance(candidate.get("confidence"), str)
            or candidate["confidence"] not in confidence_rank
            or not isinstance(candidate.get("reasons"), list)
            or any(
                not isinstance(reason, str) or reason not in known_reasons
                for reason in candidate["reasons"]
            )
        ):
            raise SystemExit("private acceptance candidate evidence is malformed; no receipt was written")
    selected = candidates[0]
    if (
        type(observed.get("candidate_family")) is not int
        or type(observed.get("candidate_proposal")) is not int
        or observed["candidate_family"] != selected["family"]
        or observed["candidate_proposal"] != selected["proposal"]
        or confidence != selected["confidence"]
        or observed.get("reasons") != selected["reasons"]
    ):
        raise SystemExit("private acceptance selected candidate does not match its evidence; no receipt was written")
    for field in numeric_fields:
        value = observed.get(field)
        if value is not None and (type(value) not in (int, float) or not math.isfinite(value)):
            raise SystemExit("private acceptance contains a non-finite result; no receipt was written")
    expectation = planned.get("expectation", {})
    kind = expectation.get("type")
    if kind == "pose":
        tolerance = expectation.get("transform_tolerance", {})
        quality = expectation.get("quality", {})
        checks = (
            observed.get("rotation_error_deg") is not None
            and observed["rotation_error_deg"] <= tolerance.get("rotation_deg", -1)
            and observed.get("translation_error_mm") is not None
            and observed["translation_error_mm"] <= tolerance.get("translation_mm", -1)
            and observed.get("maximum_control_landmark_error_mm") is not None
            and observed["maximum_control_landmark_error_mm"] <= expectation.get("maximum_control_landmark_error_mm", -1)
            and observed.get("overlap_smaller") is not None
            and observed["overlap_smaller"] >= quality.get("minimum_overlap_smaller", 2)
            and observed.get("rms_mm") is not None
            and observed["rms_mm"] <= quality.get("maximum_rms_mm", -1)
            and observed.get("p95_mm") is not None
            and observed["p95_mm"] <= quality.get("maximum_p95_mm", -1)
            and confidence_rank[confidence] >= confidence_rank.get(quality.get("minimum_confidence"), 99)
        )
        if not checks:
            raise SystemExit("private acceptance pose result no longer satisfies its manifest; no receipt was written")
    elif kind == "not_verified":
        maximum = expectation.get("maximum_confidence")
        required_reason = expectation.get("required_reason")
        if (
            not isinstance(maximum, str)
            or maximum not in confidence_rank
            or maximum == "verified"
            or any(
                confidence_rank[candidate["confidence"]] > confidence_rank[maximum]
                for candidate in candidates
            )
        ):
            raise SystemExit(
                "private acceptance rejection result does not meet the all-candidate confidence cap; no receipt was written"
            )
        if not isinstance(required_reason, str) or required_reason not in selected["reasons"]:
            raise SystemExit("private acceptance rejection reason is missing; no receipt was written")
    else:
        raise SystemExit("private acceptance manifest has an unknown expectation; no receipt was written")
    receipt_cases.append({"case_id": observed["case_id"], **{field: observed.get(field) for field in (
        "completion",
        "input_check_complete",
        "confidence",
        "reasons",
        "candidate_family",
        "candidate_proposal",
        "candidate_evidence",
        *numeric_fields,
        "examined_poses",
        "iterations",
    )}})

tree_state, tree_digest = os.environ["PRIVATE_TREE"].split(" ", 1)
receipt = {
    "schema_version": 1,
    "kind": "occluview-private-align-acceptance",
    "generated_at_utc": os.environ["PRIVATE_FINISHED"],
    "started_at_utc": os.environ["PRIVATE_STARTED"],
    "source": {
        "git_sha": os.environ["PRIVATE_REVISION"],
        "worktree": tree_state,
        "worktree_diff_sha256": tree_digest,
    },
    "toolchain": {
        "rustc_version_verbose": os.environ["PRIVATE_RUSTC"],
        "cargo_version": os.environ["PRIVATE_CARGO"],
    },
    "build": {
        "profile": os.environ["PRIVATE_PROFILE"],
        "default_features": False,
        "features": os.environ["PRIVATE_FEATURES"].split(",") if os.environ["PRIVATE_FEATURES"] else [],
        "test_target": "private_acceptance::private_acceptance_manifest",
    },
    "manifest_sha256": manifest_sha.hex(),
    "corpus_content_sha256": corpus_digest.hexdigest(),
    "executed_case_ids": case_ids,
    "cases": receipt_cases,
    "verdict": "passed",
}
pathlib.Path(os.environ["PRIVATE_RECEIPT"]).write_text(
    json.dumps(receipt, indent=2, allow_nan=False) + "\n",
    encoding="utf-8",
)
PYTHON

python3 - "$receipt_tmp" "$receipt" <<'PYTHON'
import os
import sys
os.link(sys.argv[1], sys.argv[2])
os.unlink(sys.argv[1])
PYTHON
receipt_tmp=""
case_count="$(python3 - "$receipt" <<'PYTHON'
import json
import sys
print(len(json.load(open(sys.argv[1], encoding="utf-8"))["executed_case_ids"]))
PYTHON
)"
echo "private Align acceptance passed for $case_count manifest case(s)"
echo "path-free receipt written"
