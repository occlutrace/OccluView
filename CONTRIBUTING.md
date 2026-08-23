# Contributing

Small, focused changes are easiest to review.

## Before opening a pull request

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked
```

CI runs the same four with `--locked`, plus clippy for
`x86_64-pc-windows-gnu`, a cargo-deny run over licences and advisories, a
60-second fuzz smoke over the parsers, and a Debian package build. The minimum
supported Rust version is 1.86, pinned in `rust-toolchain.toml`.

The render tests use a software rasteriser (Lavapipe on Linux, WARP on
Windows), so `cargo test` needs no GPU. They are slower than the rest; that is
expected.

## What the test suite enforces beyond behaviour

These are checked by tests, not by review, so a change that breaks one fails
before anybody reads it:

- **800 physical lines per `.rs` file.** `clippy.toml` records the number and
  `rust_source_files_stay_within_the_physical_line_budget` asserts it. Split by
  responsibility rather than by line count.
- **Every source file is reachable through a `mod` declaration.** An orphaned
  file compiles nowhere and its tests never run; one was found holding 489
  lines that had never been compiled.
- **No second copy of a shared constant.** Where the layering genuinely forbids
  an import — `occlu-mesh-edit` and `occluview-hps` are leaves and cannot
  depend on `occluview-core` — the copy is allowed, documented as a copy, and
  kept equal by a test. Anywhere else, import it.
- **No absolute path from one machine** in any source file: no `/home/<name>`,
  no `C:\Users\<name>`, no per-run scratch directory. Fixtures use the names in
  `FIXTURE_HOME_NAMES`.
- **No scan path in anything that gets logged.** The crash report is a file
  operators are asked to attach to an issue, and a scan path names a patient.
  Log how many files and of which kinds.
- **The documents match the build.** `docs/USAGE.md` is checked against the
  keys the viewer actually binds, `CHANGELOG.md` against the workspace version,
  the packaging scripts against the workflows.

## Tests

For behaviour changes, add or update tests. Prefer a test that exercises the
behaviour; where only the source can be checked — a workflow file, a Windows
path that cannot run here — say so in the test, and make sure the assertion
would fail if the thing it names were broken. A guard that matches its own
source text is worse than no guard, because it reads as coverage.

Perf claims belong with their measurement: the number, the machine, and what
was being measured, written where the constant lives.

## Commits

Conventional commits (`fix(scope): …`), imperative subject, and a body that
says what was wrong rather than what was changed. A `commit-msg` hook rejects
messages that name people or products, narrate a request or the process of
writing the change, or carry Cyrillic text. `git commit --no-verify` overrides
it; the hook exists to keep the public history readable, not to fight you.

For visible changes, add a note to `CHANGELOG.md` under the version being
prepared. Do not open a new version section: the release job publishes the
section matching the tag, and an untagged section publishes nothing.

## Releases

Bump the workspace version, update `CHANGELOG.md`, tag `vX.Y.Z`. The release
workflow builds the MSI, the portable ZIP and the Debian package, signs them
with minisign, attaches SBOMs, and attests provenance. `docs/RUNBOOK-keys.md`
covers key rotation.
