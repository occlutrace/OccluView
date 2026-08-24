#!/usr/bin/env bash
# Run one fuzz target against the tracked seeds plus the accumulating corpus.
#
# Usage: scripts/run-fuzz.sh <target> <seconds> <max-len>
#
# Two things this wrapper exists to get right:
#
#   * cargo-fuzz resolves `<cwd>/fuzz/Cargo.toml`, so it must be invoked from
#     the repository root. Running it with `working-directory: fuzz` makes it
#     look for `fuzz/fuzz/Cargo.toml` and fail before building anything.
#   * libFuzzer writes into the FIRST corpus directory and only reads the rest.
#     `fuzz/corpus/<target>` is the writable, cached one; `fuzz/seeds/<target>`
#     is tracked in git and must stay untouched.
#   * The toolchain is named explicitly. cargo-fuzz needs nightly, and this
#     repository pins 1.86.0 in `rust-toolchain.toml`, which wins over whatever
#     a CI step set as the default.
set -euo pipefail

target="${1:?usage: run-fuzz.sh <target> <seconds> <max-len>}"
seconds="${2:?usage: run-fuzz.sh <target> <seconds> <max-len>}"
max_len="${3:?usage: run-fuzz.sh <target> <seconds> <max-len>}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

seeds="fuzz/seeds/$target"
corpus="fuzz/corpus/$target"
dictionary="fuzz/dictionaries/formats.dict"

if [[ ! -d "$seeds" ]]; then
  echo "no seed directory for target '$target' at $seeds" >&2
  exit 1
fi
mkdir -p "$corpus"

cargo "+${OCCLUVIEW_FUZZ_TOOLCHAIN:-nightly}" fuzz run "$target" "$corpus" "$seeds" -- \
  "-max_total_time=$seconds" \
  "-max_len=$max_len" \
  "-dict=$dictionary"
