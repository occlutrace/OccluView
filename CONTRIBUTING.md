# Contributing

Small, focused changes are easiest to review.

## Before opening a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked
```

CI also runs Windows checks, dependency policy checks, parser fuzz smoke tests,
and a Debian package build. The minimum supported Rust version is pinned in
`rust-toolchain.toml`.

The render tests use a software rasteriser (Lavapipe on Linux, WARP on
Windows), so `cargo test` needs no GPU. They are slower than the rest; that is
expected.

## Repository hygiene

Keep process material outside the repository. Do not commit prompts, exported
agent sessions, handoffs, audit dumps, scratch plans, generated process
documents, agent instruction files such as `AGENTS.md`, or files whose only
purpose is to direct an AI worker. Add a file only when it is product
documentation or an explicit build, release, security, or contribution
contract.

Comments and doc comments describe current behaviour: an invariant, ownership
rule, format contract, or reproducible measurement. Do not use them as a work
diary or to preserve review history, speculation, obsolete behaviour, or
unverified performance claims. The pull request diff must be checked for these
artifacts; formatting, tests, and CI do not enforce this rule.

## Tests

For behaviour changes, add or update tests. Prefer behavioural assertions over
source-text checks. Keep performance thresholds tied to a reproducible
measurement.

The workspace test list is the audit baseline. Refresh it before deleting or
adding a large group of tests:

```bash
cargo test --workspace --all-targets --locked -- --list
```

Keep tests that prove observable geometry, state transitions, worker ordering,
render output, packaging, or a reproducible performance budget. A small
source-contract test is acceptable only when the runtime path is unavailable
to the test harness and the assertion protects a concrete operator or release
contract; it must inspect a narrow seam and fail closed if that seam moves.
Remove cosmetic wording pins, deleted-feature negative checks, and tests that
only duplicate the implementation's current string layout. Report-only
inventory counts are preferred to arbitrary repository-wide test caps.

## Commits

Use conventional commits (`fix(scope): ...`) with an imperative subject. Keep
each commit focused and describe the engineering reason for the change. Keep
comments and commit bodies factual: explain an invariant, boundary, or user
visible contract, and omit process narration, filler, and claims not backed by
the implementation or its checks.

For visible changes, add a note to `CHANGELOG.md` under the version being
prepared. Do not open a new version section: the release job publishes the
section matching the tag, and an untagged section publishes nothing.

## Releases

Bump the workspace version, update `CHANGELOG.md`, and tag `vX.Y.Z`. The release
workflow builds and verifies the distributable packages. Signing-key rotation is
described in `SECURITY.md`.
