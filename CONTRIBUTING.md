# Contributing

Small, focused changes are easiest to review.

## Manual verification

The following commands are available for an explicitly requested verification
run. They are not automatic pull-request or release prerequisites.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked
```

The `Manual verification` GitHub Actions workflow is started on demand; it does
not run automatically for pull requests or pushes and is not a release gate.
It includes Windows checks, dependency policy checks, parser fuzz smoke tests,
and a Debian package build. The minimum supported Rust version is pinned in
`rust-toolchain.toml`.

The render tests use a software rasteriser (Lavapipe on Linux, WARP on
Windows), so `cargo test` needs no GPU. They are slower than the rest; that is
expected.

## Workspace

The workspace contains these crates:

- `occluview-geometry` — Shared geometry constants, pure math, and triangle-surface queries; the bottom leaf, importing no other workspace crate.
- `occluview-i18n` — Fluent catalogs and locale resolution shared by the app and the shell.
- `occluview-update` — Signed update-manifest checks; the only crate carrying HTTP and signature verification.
- `occluview-sculpt` — Surface sculpting kernel with its own `parallel` feature.
- `occluview-core` — Pure data model: math, units, mesh, scene and camera, with no I/O, GPU or platform code.
- `occluview-mesh-edit` — Product-neutral mesh-editing kernel plus the adapter to core meshes; the `robust` module owns the optional native CSG fallback (feature `robust-csg`).
- `occluview-align` — Scan registration and signed deviation metrology.
- `occluview-contact` — Occlusal contact metrology and the clinical colour law.
- `occluview-formats` — Import and export readers and writers for the supported scan formats; the `hps` module owns the HPS parser (unencrypted inputs only).
- `occluview-render` — wgpu renderer for the live viewer and the offscreen thumbnail path.
- `occluview-thumbnail` — Platform-neutral thumbnail loading, rendering and fallback.
- `occluview-shell` — Windows COM shell extension: Explorer thumbnails and the preview pane.
- `occluview-app` — Desktop viewer binary (`occluview`), built with egui and wgpu.
- `occluview-cli` — Headless `occluview-cli` helper.

New crate names follow `occluview-<noun>`; do not introduce names ending in `-kernel`, `-math`, `-core`, `-index`, `-csg`, `-query`, or `-session`.
An optional part of a crate belongs in a module behind a Cargo feature, not in a new crate.
A new crate needs a reason and an entry in the workspace list above. Crate
boundaries are reviewed from the manifests; no repository-contract test
automatically enforces this list.

## Repository hygiene

Keep process material outside the repository. Do not commit working notes,
plans, review logs, generated reports, or configuration for a personal
workflow. Add a file only when it is product documentation or an explicit
build, release, security, or contribution contract.

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
that states the invariant that makes the call sound. Treat these as review
requirements; the current workspace does not automatically check every crate
gate or `SAFETY:` comment.

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

GPU tests without an available adapter report
`SKIPPED (no GPU adapter): <test name>` and return. Setting
`OCCLUVIEW_REQUIRE_GPU_TESTS=1` makes the shared test helper panic instead, so
a CI lane that promises GPU coverage cannot pass vacuously.

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

## Releases

Bump the workspace version and tag `vX.Y.Z`. The release workflow builds and
signs the distributable packages and publishes a short download list. Manual
verification workflows are separate from packaging and are not release gates.
Signing-key rotation is
described in `SECURITY.md`.

Every crate is `publish = false`, so the workspace carries no semver obligation
to crates.io. The one compatibility-committed artifact is the update feed:
installed copies fetch `latest.json` from the latest release and verify it
against the key compiled into the binary, so the manifest's field names and
platform keys are a stable contract. Add a field when a release needs one —
older clients ignore what they do not know — but do not rename, remove, or
repurpose one without bumping the `schema` the reader checks.
`scripts/write-latest-json.py` writes the manifest and must agree with the
platform keys in `crates/occluview-update`; the installers it names are the
Windows MSI and Linux `.deb`. A notarized macOS `.pkg` is included only when
that package is explicitly requested.

Regenerate a lockfile in the same commit as any manifest change it resolves:
`Cargo.lock` for the workspace and `fuzz/Cargo.lock` for the fuzz crate. CI
builds with `--locked`, so a manifest edit committed with a stale lockfile stops
the build instead of silently re-resolving it, and neither lockfile drifts from
the manifests it reads.

Alignment quality needs review on real scans in the application. A successful
package build does not establish clinical accuracy or operator acceptance.
Patient scans stay out of the public repository.

Every change to published runtime bytes requires a new patch version and tag;
published releases are not replaced in place. The package workflow runs for a
tag push or a manual run started from that tag, and requires the tag to point at
the exact source commit being packaged. It stages and checks the release assets
as an unpublished draft before publishing. A failed upload remains a draft
until that same source can be retried.

The MSI builds `occluview_shell.dll` from the revision in
`install/shell-pin.json` while the viewer builds from the tagged tree. Explorer
loads this DLL in its own process, so its pin names a revision exercised through
thumbnails, interactive preview, file association, and uninstall on Windows.
`scripts/report-shell-pin.sh` records that revision and any linked-crate changes
in `dist/occluview-shell-revision.json`. Move the pin when a new shell revision
has been reviewed and used in Explorer; include parser, rendering, and COM
fixes rather than leaving them behind in the viewer.
