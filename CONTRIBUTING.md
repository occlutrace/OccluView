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
purpose is to direct an automated tool. Add a file only when it is product
documentation or an explicit build, release, security, or contribution
contract.

Comments and doc comments describe current behaviour: an invariant, ownership
rule, format contract, or reproducible measurement. Do not use them as a work
diary or to preserve review history, speculation, obsolete behaviour, or
unverified performance claims. The pull request diff must be checked for these
artifacts; formatting, tests, and CI do not enforce this rule.

## Unsafe code

Every crate is `#![forbid(unsafe_code)]`. A crate that needs FFI or another
`unsafe` boundary opts out with `#![deny(unsafe_code)]` and a module-level
`#![allow(unsafe_code)]` in the boundary module only, with a comment naming the
boundary. Every `unsafe` block carries a `SAFETY:` comment on the line above it
that states the invariant that makes the call sound. The repository-contract
tests enforce the crate gate and the `SAFETY:` comment.

## Logging

Tracing events feed the operator's console and the in-memory ring buffer that a
crash report writes under `crashes/`. That ring is attached to public issues and
support bundles, so every recorded event is treated as public.

Choose the lowest level that fits:

- `error!` — the operation failed and someone has to act. One event per failure,
  naming the operation that failed.
- `warn!` — the operation continued with a degraded result, or an optional
  channel (a dialog, a notification, an icon) was unavailable.
- `info!` — a state transition that reconstructs a session: startup, scene load,
  diagnostics. Not a per-frame or per-item event.
- `debug!` — development detail behind `RUST_LOG`. It still reaches the ring, so
  the rules below apply.
- `trace!` — unused; prefer `debug!`.

Forbidden fields. Never log a path, a file name, a patient name, or any other
case identifier, whether as an event field or inside the message. A file path is
never a log field: record the shape of the session instead — a file count, a
format extension, a `path_count` — never the location. An error logged with
`%error` must already be scrubbed of paths by its constructor. `CrashLogLayer`
masks a field whose name is a known path field or whose rendered value contains
a path separator, but that mask is a backstop, not permission to log the value.

Subscribers live in the binaries. A library crate that does not already depend
on `tracing` must not add it to emit a breadcrumb; return a value or an error
and let the calling binary decide whether the outcome deserves a line.

## Errors

A fallible operation returns a typed error enum owned by the crate that first
names the failure; a caller that only forwards it returns the same type instead
of re-wrapping it. Application code carries the failure as `anyhow::Error` and
downcasts to the typed enum at the UI boundary, where the operator-facing
sentence is produced. Every `Display` message is such a sentence: it names what
failed and what the operator can do, never an internal crate, module, or
variant identifier, and never a path, file name, or case identifier. A file
path belongs in the caller's `anyhow` context, not in the enum. Each public
error enum has a test that constructs every variant and asserts its rendered
message is non-empty and free of internal crate names, so a variant cannot be
added without a case there.

## Tests

For behaviour changes, add or update tests. Prefer behavioural assertions over
source-text checks. Keep performance thresholds tied to a reproducible
measurement.

The workspace test list is the baseline. Refresh it before deleting or
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

For visible changes, add a short note to `CHANGELOG.md` under `## Unreleased`,
creating that section above the newest version when it is missing. A release
publishes only the section matching its version tag; draft notes stay out of
the published release until they move into a versioned section.

## Releases

Bump the workspace version, update `CHANGELOG.md`, and tag `vX.Y.Z`. The release
workflow builds and verifies the distributable packages. Signing-key rotation is
described in `SECURITY.md`.

Two checks are local and cannot run in CI, because CI has no patient scans and
no Explorer session. Both fail rather than skip: a gate that passes without
checking anything is worse than no gate.

```sh
OCCLUVIEW_ALIGN_FIXTURES=/path/to/corpus scripts/release-check.sh
```

1. **The private scan corpus.** Alignment acceptance is defined on real scans
   (0.05 mm residual, 85% measured, 90% inside the clinical band), and real
   scans are patient data that stay off GitHub. The gate writes
   `dist/private-acceptance.json`: which corpus was used (by fingerprint, never
   by name), which revision, and what it proved. Without
   `OCCLUVIEW_ALIGN_FIXTURES` it exits 2 and the release stops.

2. **The shipped Explorer shell revision.** The MSI builds
   `occluview_shell.dll` from the revision in `install/shell-pin.json` while the
   viewer builds from the tagged tree. The pin is deliberate (Explorer loads the
   DLL in its own process), so the release states it in
   `dist/occluview-shell-revision.json`: the revision, its tag, and every commit
   since that touched a crate the shell links.

   Read that list before tagging. A fix that matters to Explorer — a parser, a
   thumbnail, or a COM defect — has to be cherry-picked onto the pinned
   revision, and the pin moves to that new commit. The pin never moves to a
   revision that has not been exercised in Explorer, and it retires when the
   shell built from the current tree has been through the thumbnail,
   preview-pane, file-association and uninstall paths on a real machine.
