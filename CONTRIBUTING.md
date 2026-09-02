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

## Tests

For behaviour changes, add or update tests. Prefer behavioural assertions over
source-text checks. Keep performance thresholds tied to a reproducible
measurement.

## Commits

Use conventional commits (`fix(scope): ...`) with an imperative subject. Keep
each commit focused and describe the engineering reason for the change.

For visible changes, add a note to `CHANGELOG.md` under the version being
prepared. Do not open a new version section: the release job publishes the
section matching the tag, and an untagged section publishes nothing.

## Releases

Bump the workspace version, update `CHANGELOG.md`, and tag `vX.Y.Z`. The release
workflow builds and verifies the distributable packages. Signing-key rotation is
described in `SECURITY.md`.
