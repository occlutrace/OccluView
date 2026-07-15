# Contributing

Small, focused changes are easiest to review.

Before opening a pull request, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

For behavior changes, add or update tests. For visible changes, include a short
note in `CHANGELOG.md`.

For a release, bump the workspace version, update `CHANGELOG.md`, and tag the
same version as `vX.Y.Z`.

Keep public copy plain and specific. Avoid broad claims, internal process notes,
and unrelated cleanup in the same change.
