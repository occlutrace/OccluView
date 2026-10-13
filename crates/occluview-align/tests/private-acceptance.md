# Private Align acceptance manifest

This is an optional maintainer-run check against a private corpus. The test is
ignored in ordinary Cargo runs, and `scripts/release-check.sh` does not invoke
it. A successful receipt records only that the named cases met this manifest;
it is not a clinical certification or a substitute for operator review.

Run it with an explicit manifest and corpus directory:

```sh
OCCLUVIEW_PRIVATE_MANIFEST=/private/acceptance.json \
OCCLUVIEW_PRIVATE_CORPUS=/private/corpus \
  scripts/validate-release-private.sh
```

The JSON document has `schema_version: 1` and a non-empty `cases` array. Each
case has a stable, non-identifying ID of the form `case_` followed by twelve
lowercase hexadecimal characters. Generate IDs independently of scan names,
patient identifiers, dates, or other case metadata. IDs must be unique.

Each case declares `moving` and `fixed` assets. An asset has a corpus-relative
`file` and an explicit positive `scale_to_mm`. Paths cannot be absolute, use
parent traversal, contain symlinks, or resolve outside the corpus. The scale is
an explicit corpus-owner decision; the harness never infers units from the
scene size or changes coordinates automatically. The production format reader
parses each file from the same owned byte buffer whose SHA-256 contributes to
the receipt.

Every case declares `search` settings: `normal_policy` (`match`, `opposed`, or
`unsigned`), `profile` (`standard` or `local`), `top_k` (1 through 5),
`reference_regions` (`all_eligible` for this target), and a positive
`wall_limit_ms`. The target does not expose ROI masks, so `reference_roi` is
rejected instead of silently running without one. These values are explicit
per case; the manifest has no acceptance defaults.

`expectation.type` is one of:

- `pose`: provide a known `truth` pose (`rotation_xyzw` and `translation_mm`),
  transform tolerances, at least three independently authored
  `control_landmarks`, a maximum landmark error, and `quality` bounds for
  minimum confidence, minimum overlap, maximum RMS, and maximum p95. These
  quality checks apply to the selected first-ranked candidate; the receipt
  also records each returned candidate's stable family/proposal identity,
  confidence, and reasons. The landmarks are held out: they are checked after
  the search and never passed into Align as constraints. Truth, landmarks, and
  bounds must come from an independent reference for that case. Choose bounds
  for the intended use; do not copy solver-internal confidence cutoffs or
  invent clinical limits.
- `not_verified`: provide a confidence ceiling below `verified` and the exact
  expected evidence reason for the selected first-ranked candidate. Every
  returned candidate must stay at or below the confidence ceiling; a verified
  alternative anywhere in the candidate list fails the case. This checks a
  counterexample where the solver must not claim verification. It does not
  mean the parser rejects the file or that the application has
  accepted/rejected an operator action.

Confidence values are `weak`, `ambiguous`, `probable`, and `verified`. A
`not_verified` case names one manifest `required_reason` from
`insufficient_support`, `unobservable_motion`, `close_rival`,
`policy_conflict`, `suspected_units_or_deformation`, `missing_normals`,
`uniqueness_not_established`, and `invalid_topology`. A returned candidate's
evidence can additionally carry `unvalidated_input`, `budget_exhausted`,
`invalid_seed`, or `landmark_rejected`; those explain why a candidate was not
selected and cannot be demanded by a manifest. The manifest parser accepts only
the eight reasons a case can require; the receipt records every returned
candidate's reasons from the full result vocabulary.

The manifest is strict: unknown fields, duplicate IDs, invalid bounds,
unsupported schema, absent inputs, zero cases, incomplete search, missing
measurements needed by a pose case, or a corpus change during execution fail
closed. A receipt is written only after every manifest case ran and passed.

The receipt includes the exact Git SHA, clean/dirty state and a digest of the
working-tree diff, Rust/Cargo versions, build profile/features, timestamps,
manifest SHA-256, content-byte corpus SHA-256, executed case IDs, and measured
numeric results. It contains no asset paths or patient names. Hashes identify
bytes; they do not anonymize a corpus that someone already possesses. Keep the
manifest and any mapping from random IDs to case files private.
